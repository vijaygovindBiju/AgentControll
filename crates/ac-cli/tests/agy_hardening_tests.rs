//! Antigravity (`agy`) hardening tests.
//!
//! Uses a fake `agy` binary (shell script) that records the HOME it runs in,
//! which account token it sees, its arguments and selected env vars. This
//! proves which credentials the *actual process* used.
//!
//! Tests that touch process-wide environment (HOME, XDG_CONFIG_HOME,
//! ANTIGRAVITY_BIN) are serialized with a global lock.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use base64::Engine;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::broadcast,
};

use ac_cli::{
    client::DaemonClient,
    launcher::{self, parse_selection, resolve_account, AccountMatch, Choice, Selector},
};
use ac_core::{
    account_manager::{AccountManager, AccountManagerHandle, AccountStore},
    adapter::{AdapterFactory, CompositeAdapterFactory},
    agy_auth::{self, AgyAuthError, AgyToken, BrowserLogin, OAuthClient},
    event_store::EventStore,
    interaction_hub::{InteractionHub, InteractionHubHandle, InteractionStore},
    ipc::IpcServer,
    policy_engine::{PolicyEngine, PolicyStore},
    project_registry::{ProjectRegistry, ProjectRegistryHandle, ProjectStore},
    session::manager::{SessionManager, SessionManagerHandle},
    types::{Account, AgentEvent, Id, SessionContext, SessionState},
};

// ── Environment fixture ───────────────────────────────────────────────────────

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct Env {
    _guard: MutexGuard<'static, ()>,
    tmp: TempDir,
}

const FAKE_AGY: &str = r#"#!/bin/sh
tok="$HOME/.gemini/antigravity-cli/antigravity-oauth-token"
marker=$(grep -o 'MARK_[A-Z]*' "$tok" 2>/dev/null | head -1)
printf 'HOME=%s MARK=%s ARGC=%s GEMINI=%s ACCT=%s ARGS=%s PWD=%s\n' "$HOME" "${marker:-NONE}" "$#" "${GEMINI_API_KEY:-unset}" "$AGENTCONTROL_ACCOUNT_ID" "$(for a in "$@"; do printf '%s,' "$a"; done)" "$(pwd)" >> "$AC_TEST_LOG"
exec sleep 30
"#;

impl Env {
    fn new() -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        // The machine's "default" agy login — must never be used.
        let default_agy = home.join(".gemini/antigravity-cli");
        std::fs::create_dir_all(&default_agy).unwrap();
        std::fs::write(
            default_agy.join("antigravity-oauth-token"),
            native_token("MARK_DEFAULT", "default@x.com", "2099-01-01T00:00:00Z").to_string(),
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_test"), "keep-me").unwrap();
        std::fs::write(home.join(".gitconfig"), "[user]\n").unwrap();

        let bin = tmp.path().join("fake-agy");
        std::fs::write(&bin, FAKE_AGY).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", tmp.path().join("config"));
        std::env::set_var("ANTIGRAVITY_BIN", &bin);
        std::env::set_var("AC_TEST_LOG", tmp.path().join("agy.log"));
        std::env::set_var("GEMINI_API_KEY", "AMBIENT-SECRET");
        Env { _guard: guard, tmp }
    }

    fn log_lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.tmp.path().join("agy.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    async fn wait_for_lines(&self, n: usize) -> Vec<String> {
        for _ in 0..100 {
            let l = self.log_lines();
            if l.len() >= n {
                return l;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        panic!(
            "fake agy did not start {n} time(s); log: {:?}",
            self.log_lines()
        );
    }

    fn home(&self) -> PathBuf {
        self.tmp.path().join("home")
    }
}

fn jwt(email: &str) -> String {
    let p = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(format!("{{\"email\":\"{email}\"}}"));
    format!("hdr.{p}.sig")
}

fn native_token(marker: &str, email: &str, expiry: &str) -> serde_json::Value {
    serde_json::json!({
        "token": {
            "access_token": format!("{marker}-access-SECRET"),
            "token_type": "Bearer",
            "refresh_token": format!("{marker}-refresh-SECRET"),
            "expiry": expiry,
        },
        "auth_method": "consumer",
        "id_token": jwt(email),
    })
}

fn make_cred(label: &str, marker: &str, email: &str) -> String {
    let tok = AgyToken::from_native(native_token(marker, email, "2099-01-01T00:00:00Z")).unwrap();
    agy_auth::save_new_credential(label, &tok, "browser_login").unwrap()
}

fn agy_account(label: &str, cred_ref: &str) -> Account {
    Account::new(
        label.into(),
        "agy".into(),
        vec!["agy".into(), "antigravity".into()],
        cred_ref.into(),
        2,
        vec![],
    )
}

fn write_legacy(id: &str, token: Option<&str>) -> String {
    let dir = agy_auth::credentials_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let body = serde_json::json!({"id": id, "provider": "agy", "label": "College Google", "auth_mode": "oauth2_token", "token": token, "created_at": "2026-01-01T00:00:00Z"});
    std::fs::write(dir.join(format!("agy_{id}.json")), body.to_string()).unwrap();
    format!("ref:agy:{id}")
}

fn parse_log(line: &str) -> std::collections::HashMap<String, String> {
    line.split_whitespace()
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
}

// ── Daemon harness ────────────────────────────────────────────────────────────

struct Daemon {
    client: DaemonClient,
    accounts: AccountManagerHandle,
    _sessions: SessionManagerHandle,
}

async fn start_daemon(dir: &Path, factory: Box<dyn AdapterFactory + Send>) -> Daemon {
    let sock = dir.join(format!("ac-{}.sock", ulid::Ulid::new()));
    let store = EventStore::open(&dir.join("events.db")).unwrap();
    let (event_tx, _) = broadcast::channel::<AgentEvent>(256);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(64);
    let policy_engine =
        PolicyEngine::new(PolicyStore::open(&dir.join("policies.db")).unwrap()).unwrap();
    let hub = InteractionHubHandle::new(
        InteractionHub::new(
            InteractionStore::open(&dir.join("interactions.db")).unwrap(),
            policy_engine,
        )
        .unwrap(),
    );
    let accounts = AccountManagerHandle::new(
        AccountManager::new(AccountStore::open(&dir.join("accounts.db")).unwrap()).unwrap(),
    );
    let projects = ProjectRegistryHandle::new(
        ProjectRegistry::new(ProjectStore::open(&dir.join("projects.db")).unwrap()).unwrap(),
    );

    let sm = SessionManager::new(store, cmd_rx, event_tx.clone(), 3, factory)
        .with_account_manager_handle(accounts.clone())
        .with_project_registry_handle(projects.clone())
        .with_interaction_hub(hub.clone());
    let sessions = SessionManagerHandle::new(cmd_tx);
    tokio::spawn(sm.run());
    let server = IpcServer::bind(&sock, sessions.clone(), event_tx)
        .unwrap()
        .with_account_manager(accounts.clone())
        .with_project_registry(projects)
        .with_interaction_hub(hub);
    tokio::spawn(server.run());
    let client = DaemonClient::new(sock);
    for _ in 0..50 {
        if client.is_running().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Daemon {
        client,
        accounts,
        _sessions: sessions,
    }
}

fn register(d: &Daemon, a: Account) -> Id {
    d.accounts.0.lock().unwrap().register(a).unwrap()
}

// ── Per-account execution & isolation ─────────────────────────────────────────

#[tokio::test]
async fn three_accounts_launch_with_their_own_isolated_credentials() {
    let env = Env::new();
    let mut factory = CompositeAdapterFactory::new();
    let mut handles = vec![];
    let mut expected = vec![];
    for (label, marker) in [
        ("Personal Google", "MARK_A"),
        ("College Google", "MARK_B"),
        ("Work Google", "MARK_C"),
    ] {
        let cref = make_cred(label, marker, &format!("{}@x.com", marker.to_lowercase()));
        let aid = Id::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let ctx = SessionContext::new(
            Id::new(),
            "[interactive] Antigravity coding session".into(),
            "agy".into(),
        )
        .with_account(aid.clone(), cref);
        handles.push((factory.create(ctx, tx).unwrap(), _rx));
        expected.push((aid, marker));
        env.wait_for_lines(expected.len()).await;
    }

    let lines = env.log_lines();
    assert_eq!(lines.len(), 3);
    for (line, (aid, marker)) in lines.iter().zip(&expected) {
        let kv = parse_log(line);
        let profile = agy_auth::profile_dir(aid).unwrap();
        assert_eq!(
            kv["HOME"],
            profile.to_string_lossy(),
            "agy must run in the account's own profile"
        );
        assert_eq!(
            kv["MARK"], *marker,
            "agy must see the selected account's token"
        );
        assert_ne!(
            kv["MARK"], "MARK_DEFAULT",
            "default ~/.gemini login must never be used"
        );
        assert_eq!(
            kv["ARGC"], "0",
            "interactive session must not inject a fake prompt"
        );
        assert_eq!(
            kv["GEMINI"], "unset",
            "ambient API keys must not leak into agy"
        );
        assert_eq!(kv["ACCT"], aid.0);
        let tok = profile.join(".gemini/antigravity-cli/antigravity-oauth-token");
        assert!(
            !tok.symlink_metadata().unwrap().file_type().is_symlink(),
            "token must be a private file"
        );
        // Git/SSH config stays available; real ~/.gemini is not linked wholesale.
        assert!(profile.join(".ssh").exists() && profile.join(".gitconfig").exists());
        assert!(!profile
            .join(".gemini")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
    }
    // No two profiles share authentication state.
    let toks: Vec<String> = expected
        .iter()
        .map(|(a, _)| {
            std::fs::read_to_string(
                agy_auth::profile_dir(a)
                    .unwrap()
                    .join(".gemini/antigravity-cli/antigravity-oauth-token"),
            )
            .unwrap()
        })
        .collect();
    assert!(toks[0] != toks[1] && toks[1] != toks[2] && toks[0] != toks[2]);
    drop(handles);
}

#[tokio::test]
async fn missing_account_or_bad_credentials_never_fall_back_to_default_login() {
    let env = Env::new();
    let mut factory = CompositeAdapterFactory::new();
    let launch = |f: &mut CompositeAdapterFactory, ctx: SessionContext| {
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        f.create(ctx, tx)
            .err()
            .expect("launch must fail")
            .to_string()
    };
    let base = || SessionContext::new(Id::new(), "[interactive] s".into(), "agy".into());

    let e = launch(&mut factory, base());
    assert!(e.contains("No Antigravity account is selected"), "{e}");

    let e = launch(
        &mut factory,
        base().with_account(Id::new(), "ref:agy:01NOTEXISTING"),
    );
    assert!(e.contains("No Antigravity credential is saved"), "{e}");

    let e = launch(
        &mut factory,
        base().with_account(Id::new(), write_legacy("01EMPTY", Some(""))),
    );
    assert!(
        e.contains("cannot be started") && e.contains("No Antigravity credential"),
        "{e}"
    );

    let e = launch(
        &mut factory,
        base().with_account(
            Id::new(),
            write_legacy("01CODE", Some("4/0AVGzR1A-SECRETCODE")),
        ),
    );
    assert!(e.contains("one-time login code"), "{e}");
    assert!(!e.contains("SECRETCODE"), "errors must not echo the code");

    let e = launch(
        &mut factory,
        base().with_account(
            Id::new(),
            write_legacy("01JUNK", Some("some-random-SECRET")),
        ),
    );
    assert!(e.contains("not valid"), "{e}");
    assert!(!e.contains("SECRET"));

    std::fs::write(
        agy_auth::credentials_dir().join("agy_01MALFORMED.json"),
        "{not json",
    )
    .unwrap();
    let e = launch(
        &mut factory,
        base().with_account(Id::new(), "ref:agy:01MALFORMED"),
    );
    assert!(e.contains("malformed"), "{e}");

    let expired = serde_json::json!({"token": {"access_token": "X-SECRET", "expiry": "2000-01-01T00:00:00Z"}, "auth_method": "consumer"});
    let e = launch(
        &mut factory,
        base().with_account(
            Id::new(),
            write_legacy("01EXPIRED", Some(&expired.to_string())),
        ),
    );
    assert!(e.contains("expired") && !e.contains("X-SECRET"), "{e}");

    assert!(
        env.log_lines().is_empty(),
        "agy must never have been started"
    );
}

#[tokio::test]
async fn legacy_valid_credential_is_migrated_and_launches() {
    let env = Env::new();
    let native = native_token("MARK_L", "l@x.com", "2099-01-01T00:00:00Z").to_string();
    let cref = write_legacy("01LEGACY", Some(&native));
    let mut factory = CompositeAdapterFactory::new();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let ctx = SessionContext::new(Id::new(), "[interactive] s".into(), "agy".into())
        .with_account(Id::new(), cref.clone());
    let _h = factory.create(ctx, tx).unwrap();
    assert!(parse_log(&env.wait_for_lines(1).await[0])["MARK"] == "MARK_L");
    let migrated: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(agy_auth::credential_path(&cref).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(migrated["version"], 2);
    assert_eq!(migrated["credential_type"], "antigravity_oauth");
    assert_eq!(migrated["email"], "l@x.com");
}

#[tokio::test]
async fn refreshed_token_is_written_back_safely() {
    let _env = Env::new();
    let aid = Id::new();
    let cref = make_cred("Personal Google", "MARK_A", "a@x.com");
    agy_auth::prepare_profile(&aid, &cref).unwrap();
    let tok_path = agy_auth::profile_dir(&aid)
        .unwrap()
        .join(".gemini/antigravity-cli/antigravity-oauth-token");
    let stored = || std::fs::read_to_string(agy_auth::credential_path(&cref).unwrap()).unwrap();

    // agy refreshed: newer expiry, same identity → persisted.
    std::fs::write(
        &tok_path,
        native_token("MARK_REFRESHED", "a@x.com", "2099-06-01T00:00:00Z").to_string(),
    )
    .unwrap();
    assert!(agy_auth::sync_profile_back(&aid, &cref).unwrap());
    assert!(stored().contains("MARK_REFRESHED"));

    // Older token never overwrites a newer stored one.
    std::fs::write(
        &tok_path,
        native_token("MARK_OLD", "a@x.com", "2098-01-01T00:00:00Z").to_string(),
    )
    .unwrap();
    assert!(!agy_auth::sync_profile_back(&aid, &cref).unwrap());
    assert!(!stored().contains("MARK_OLD"));

    // A different Google identity is rejected.
    std::fs::write(
        &tok_path,
        native_token("MARK_EVIL", "other@x.com", "2100-01-01T00:00:00Z").to_string(),
    )
    .unwrap();
    assert!(matches!(
        agy_auth::sync_profile_back(&aid, &cref),
        Err(AgyAuthError::RefreshFailed { .. })
    ));
    assert!(!stored().contains("MARK_EVIL"));

    // Corrupt profile token → refresh failure reported; next launch restores stored token.
    std::fs::write(&tok_path, "garbage").unwrap();
    assert!(matches!(
        agy_auth::sync_profile_back(&aid, &cref),
        Err(AgyAuthError::RefreshFailed { .. })
    ));
    agy_auth::prepare_profile(&aid, &cref).unwrap();
    assert!(std::fs::read_to_string(&tok_path)
        .unwrap()
        .contains("MARK_REFRESHED"));
}

// ── Switching & restart through the daemon ────────────────────────────────────

#[tokio::test]
async fn switching_accounts_restarts_agy_with_target_credentials_only() {
    let env = Env::new();
    let d = start_daemon(env.tmp.path(), Box::new(CompositeAdapterFactory::new())).await;
    let a = register(
        &d,
        agy_account(
            "Personal Google",
            &make_cred("Personal Google", "MARK_A", "a@x.com"),
        ),
    );
    let b = register(
        &d,
        agy_account(
            "College Google",
            &make_cred("College Google", "MARK_B", "b@x.com"),
        ),
    );
    let c = register(
        &d,
        agy_account("Broken Google", &write_legacy("01BROKEN", Some("4/0Abc"))),
    );

    let sid = d
        .client
        .create_and_start_session("[interactive] s", "agy", None, Some(a.clone()))
        .await
        .unwrap();
    let l = env.wait_for_lines(1).await;
    assert_eq!(parse_log(&l[0])["MARK"], "MARK_A");

    let successor = d.client.switch_account(&sid, &b).await.unwrap();
    assert_ne!(successor, sid);
    let l = env.wait_for_lines(2).await;
    let kv = parse_log(&l[1]);
    assert_eq!(kv["MARK"], "MARK_B", "successor must use Account B");
    assert_eq!(
        kv["HOME"],
        agy_auth::profile_dir(&b).unwrap().to_string_lossy(),
        "successor must not inherit A's profile"
    );
    assert_eq!(kv["ACCT"], b.0);
    let pred = d.client.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(pred.state, SessionState::HandedOff);

    // Switching to an account with an unusable credential fails BEFORE the
    // running session is stopped and never launches anything else.
    let err = d
        .client
        .switch_account(&successor, &c)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("one-time login code"), "{err}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(env.log_lines().len(), 2);
    let still = d.client.get_session(&successor).await.unwrap().unwrap();
    assert!(
        !still.state.is_terminal(),
        "running session must survive a failed switch (state={})",
        still.state
    );
}

#[tokio::test]
async fn accounts_and_credentials_survive_daemon_restart() {
    let env = Env::new();
    let dir = env.tmp.path().join("daemon");
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = {
        let d = start_daemon(&dir, Box::new(CompositeAdapterFactory::new())).await;
        let a = register(
            &d,
            agy_account(
                "Personal Google",
                &make_cred("Personal Google", "MARK_A", "a@x.com"),
            ),
        );
        let b = register(
            &d,
            agy_account(
                "College Google",
                &make_cred("College Google", "MARK_B", "b@x.com"),
            ),
        );
        (a, b)
    };
    // "Restart": fresh daemon over the same database files.
    let d = start_daemon(&dir, Box::new(CompositeAdapterFactory::new())).await;
    let accounts = d.client.list_accounts().await.unwrap();
    assert!(accounts.iter().any(|x| x.id == a) && accounts.iter().any(|x| x.id == b));

    d.client
        .create_and_start_session("[interactive] s", "agy", None, Some(b.clone()))
        .await
        .unwrap();
    assert_eq!(parse_log(&env.wait_for_lines(1).await[0])["MARK"], "MARK_B");
    d.client
        .create_and_start_session("[interactive] s", "agy", None, Some(a.clone()))
        .await
        .unwrap();
    assert_eq!(parse_log(&env.wait_for_lines(2).await[1])["MARK"], "MARK_A");
}

// ── Removal ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn removal_is_blocked_while_active_then_cleans_up_only_owned_data() {
    let env = Env::new();
    let dir = env.tmp.path().join("daemon");
    std::fs::create_dir_all(&dir).unwrap();
    let d = start_daemon(&dir, Box::new(CompositeAdapterFactory::new())).await;
    let cref = make_cred("College Google", "MARK_B", "b@x.com");
    let b = register(&d, agy_account("College Google", &cref));
    let a = register(
        &d,
        agy_account(
            "Personal Google",
            &make_cred("Personal Google", "MARK_A", "a@x.com"),
        ),
    );

    let sid = d
        .client
        .create_and_start_session("[interactive] s", "agy", None, Some(b.clone()))
        .await
        .unwrap();
    env.wait_for_lines(1).await;

    let err = launcher::remove_account_command(&d.client, "College Google", true)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Cannot remove \"College Google\"") && err.contains("Active sessions"),
        "{err}"
    );
    assert!(agy_auth::credential_path(&cref).unwrap().exists());
    assert!(agy_auth::profile_dir(&b).unwrap().exists());

    d.client.stop_session(&sid, None).await.unwrap();
    launcher::remove_account_command(&d.client, "College Google", true)
        .await
        .unwrap();

    assert!(
        !agy_auth::credential_path(&cref).unwrap().exists(),
        "credential deleted"
    );
    assert!(
        !agy_auth::profile_dir(&b).unwrap().exists(),
        "profile deleted"
    );
    assert_eq!(
        std::fs::read_to_string(env.home().join(".ssh/id_test")).unwrap(),
        "keep-me",
        "user data untouched"
    );
    assert!(env.home().join(".gitconfig").exists());
    assert!(env
        .home()
        .join(".gemini/antigravity-cli/antigravity-oauth-token")
        .exists());
    let accounts = d.client.list_accounts().await.unwrap();
    assert!(accounts.iter().all(|x| x.id != b) && accounts.iter().any(|x| x.id == a));
    assert!(matches!(
        resolve_account(&accounts, "College Google"),
        AccountMatch::NotFound
    ));

    // Removal persists across restart.
    let d2 = start_daemon(&dir, Box::new(CompositeAdapterFactory::new())).await;
    assert!(d2
        .client
        .list_accounts()
        .await
        .unwrap()
        .iter()
        .all(|x| x.id != b));
}

#[tokio::test]
async fn invalid_or_ambiguous_names_never_remove_another_account() {
    let env = Env::new();
    let d = start_daemon(env.tmp.path(), Box::new(CompositeAdapterFactory::new())).await;
    let p1 = register(
        &d,
        agy_account(
            "Personal Google",
            &make_cred("Personal Google", "MARK_A", "a@x.com"),
        ),
    );
    let _p2 = register(
        &d,
        agy_account(
            "Personal Google",
            &make_cred("Personal Google", "MARK_B", "b@x.com"),
        ),
    );

    let err = launcher::remove_account_command(&d.client, "Wrong Account", true)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("does not exist"), "{err}");
    let err = launcher::remove_account_command(&d.client, "Personal Google", true)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Multiple accounts match") && err.contains("exact account ID"),
        "{err}"
    );
    let err = launcher::remove_account_command(&d.client, "1", true)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("does not exist"),
        "numeric input must not pick account #1: {err}"
    );
    assert_eq!(d.client.list_accounts().await.unwrap().len(), 2);

    launcher::remove_account_command(&d.client, &p1.0, true)
        .await
        .unwrap();
    let left = d.client.list_accounts().await.unwrap();
    assert_eq!(left.len(), 1);
    assert_ne!(left[0].id, p1);
}

#[tokio::test]
async fn daemon_refuses_to_register_agy_account_without_valid_credential() {
    let env = Env::new();
    let d = start_daemon(env.tmp.path(), Box::new(CompositeAdapterFactory::new())).await;
    for cref in [
        "ref:agy:01NOPE",
        &write_legacy("01C", Some("4/0Abc")),
        &write_legacy("01E", None),
    ] {
        let err = d
            .client
            .register_account("X", "agy", &["agy"], cref, 2, &[])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot be started"), "{err}");
    }
    assert!(d.client.list_accounts().await.unwrap().is_empty());
}

// ── Security ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn secrets_never_reach_sqlite_events_or_process_arguments() {
    let env = Env::new();
    let dir = env.tmp.path().join("daemon");
    std::fs::create_dir_all(&dir).unwrap();
    let d = start_daemon(&dir, Box::new(CompositeAdapterFactory::new())).await;
    let a = register(
        &d,
        agy_account(
            "Personal Google",
            &make_cred("Personal Google", "MARK_A", "a@x.com"),
        ),
    );
    let b = register(
        &d,
        agy_account(
            "College Google",
            &make_cred("College Google", "MARK_B", "b@x.com"),
        ),
    );
    let sid = d
        .client
        .create_and_start_session("[interactive] s", "agy", None, Some(a))
        .await
        .unwrap();
    env.wait_for_lines(1).await;
    d.client.switch_account(&sid, &b).await.unwrap();
    let lines = env.wait_for_lines(2).await;
    assert!(
        lines.iter().all(|l| parse_log(l)["ARGC"] == "0"),
        "no arguments (hence no secrets) passed to agy"
    );

    let events = d.client.query_events(&sid, None).await.unwrap();
    let events_json = serde_json::to_string(&events).unwrap();
    for db in ["events.db", "accounts.db"] {
        let mut bytes = std::fs::read(dir.join(db)).unwrap_or_default();
        bytes.extend(std::fs::read(dir.join(format!("{db}-wal"))).unwrap_or_default());
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("-SECRET"), "{db} must not contain tokens");
    }
    assert!(
        !events_json.contains("-SECRET"),
        "events must not contain tokens"
    );
}

// ── OAuth browser login ───────────────────────────────────────────────────────

/// Minimal fake Google token endpoint. Returns the captured request body.
async fn fake_token_endpoint(
    status: &'static str,
    body: &'static str,
) -> (String, Arc<Mutex<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(String::new()));
    let cap = captured.clone();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = s.read(&mut chunk).await.unwrap();
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf).to_string();
            if let Some(i) = text.find("\r\n\r\n") {
                let len = text
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if buf.len() >= i + 4 + len || n == 0 {
                    *cap.lock().unwrap() = text;
                    break;
                }
            }
        }
        let resp = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        s.write_all(resp.as_bytes()).await.unwrap();
    });
    (url, captured)
}

/// Simulate the browser following the redirect back to the loopback listener.
async fn browser_redirect(login: &BrowserLogin, code: &str) {
    let state = login
        .authorization_url()
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string();
    let addr = login
        .redirect_uri()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();
    let req =
        format!("GET /oauth2callback?code={code}&state={state} HTTP/1.1\r\nHost: {addr}\r\n\r\n");
    tokio::spawn(async move {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let mut sink = Vec::new();
        let _ = s.read_to_end(&mut sink).await;
    });
}

#[tokio::test]
async fn browser_login_success_uses_same_port_and_keeps_secrets_in_body() {
    let body = r#"{"access_token":"AT-SECRET","refresh_token":"RT-SECRET","expires_in":3599,"token_type":"Bearer","id_token":"h.eyJlbWFpbCI6InVAeC5jb20ifQ.s"}"#;
    let (url, captured) = fake_token_endpoint("200 OK", body).await;
    let login = BrowserLogin::start_with(
        OAuthClient::new("cid", "CLIENT-SECRET"),
        "127.0.0.1:0",
        &url,
    )
    .await
    .unwrap();
    assert!(!login.authorization_url().contains("CLIENT-SECRET"));
    browser_redirect(&login, "4%2F0AUTHCODE").await;
    let code = login
        .wait_for_callback(Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(code, "4/0AUTHCODE");
    let tok = login.exchange(&code).await.unwrap();
    assert_eq!(tok.email().as_deref(), Some("u@x.com"));
    assert!(tok.has_refresh_token());

    let req = captured.lock().unwrap().clone();
    let (head, form) = req.split_once("\r\n\r\n").unwrap();
    assert!(
        head.starts_with("POST /token "),
        "secrets must not be in the URL: {head}"
    );
    let params: std::collections::HashMap<String, String> = url_params(form);
    assert_eq!(
        params["redirect_uri"],
        login.redirect_uri(),
        "exchange must use the callback's port"
    );
    assert_eq!(params["code"], "4/0AUTHCODE");
    assert_eq!(params["client_secret"], "CLIENT-SECRET");
    assert!(params["code_verifier"].len() >= 43);
}

fn url_params(form: &str) -> std::collections::HashMap<String, String> {
    form.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), urldecode(v)))
        .collect()
}

fn urldecode(s: &str) -> String {
    let s = s.replace('+', " ");
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

#[tokio::test]
async fn failed_exchanges_never_produce_a_credential() {
    let _env = Env::new();
    for (status, body, expect) in [
        (
            "400 Bad Request",
            r#"{"error":"invalid_grant","error_description":"Bad Request"}"#,
            "invalid_grant",
        ),
        ("200 OK", "<html>oops</html>", "malformed token response"),
        (
            "200 OK",
            r#"{"access_token":"AT-SECRET","expires_in":3599}"#,
            "no refresh token",
        ),
        ("500 Internal Server Error", r#"{}"#, "HTTP 500"),
    ] {
        let (url, _) = fake_token_endpoint(status, body).await;
        let login = BrowserLogin::start_with(
            OAuthClient::new("cid", "CLIENT-SECRET"),
            "127.0.0.1:0",
            &url,
        )
        .await
        .unwrap();
        let err = login.exchange("4/0CODE").await.unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, AgyAuthError::LoginFailed(_)), "{msg}");
        assert!(
            msg.contains("The account was not added") && msg.contains(expect),
            "{msg}"
        );
        assert!(!msg.contains("SECRET") && !msg.contains("4/0CODE"), "{msg}");
    }
    let saved = std::fs::read_dir(agy_auth::credentials_dir())
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(saved, 0, "no credential may be written on failed login");
}

#[tokio::test]
async fn callback_errors_and_timeouts_are_reported() {
    let login = BrowserLogin::start(OAuthClient::new("c", "s"))
        .await
        .unwrap();
    let addr = login
        .redirect_uri()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();
    tokio::spawn(async move {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /oauth2callback?error=access_denied HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
    });
    let err = login
        .wait_for_callback(Duration::from_secs(5))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Authorization was denied") && err.contains("not added"),
        "{err}"
    );

    let login = BrowserLogin::start(OAuthClient::new("c", "s"))
        .await
        .unwrap();
    let err = login
        .wait_for_callback(Duration::from_millis(100))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("timed out"), "{err}");
}

#[test]
fn oauth_client_is_not_configured_without_env_or_file() {
    let _env = Env::new();
    std::env::remove_var("AC_AGY_OAUTH_CLIENT_ID");
    std::env::remove_var("AC_AGY_OAUTH_CLIENT_SECRET");
    let err = OAuthClient::load().unwrap_err();
    assert!(matches!(err, AgyAuthError::OAuthNotConfigured(_)));
}

// ── Selection & selector UX ───────────────────────────────────────────────────

fn acct(label: &str) -> Account {
    agy_account(label, "ref:agy:01X")
}

#[test]
fn numeric_selection_never_defaults() {
    assert_eq!(parse_selection("2", 3), Ok(Some(2)));
    assert_eq!(parse_selection(" 1 ", 3), Ok(Some(1)));
    assert_eq!(parse_selection("q", 3), Ok(None));
    assert!(parse_selection("", 3).is_err());
    assert!(parse_selection("0", 3).is_err());
    assert!(parse_selection("4", 3).is_err());
    assert!(parse_selection("abc", 3).is_err());
    assert!(parse_selection("-1", 3).is_err());
}

#[test]
fn account_resolution_is_exact() {
    let accounts = vec![
        acct("Personal Google"),
        acct("College Google"),
        acct("Personal Google"),
    ];
    assert!(
        matches!(resolve_account(&accounts, "college google"), AccountMatch::One(a) if a.label == "College Google")
    );
    assert!(
        matches!(resolve_account(&accounts, "Personal Google"), AccountMatch::Many(v) if v.len() == 2)
    );
    assert!(matches!(
        resolve_account(&accounts, "Wrong Account"),
        AccountMatch::NotFound
    ));
    assert!(matches!(
        resolve_account(&accounts, "Personal"),
        AccountMatch::NotFound
    ));
    assert!(matches!(
        resolve_account(&accounts, "1"),
        AccountMatch::NotFound
    ));
    let id = accounts[2].id.0.clone();
    assert!(matches!(resolve_account(&accounts, &id), AccountMatch::One(a) if a.id.0 == id));
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

#[test]
fn selector_keys_select_cancel_and_clamp() {
    let items = vec!["A".to_string(), "B".to_string()];
    let mut s = Selector::new("t", items.clone(), true);
    assert_eq!(s.handle_key(key(KeyCode::Up, KeyModifiers::NONE)), None);
    assert_eq!(s.selected, 0);
    s.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    s.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(s.selected, 1);
    assert_eq!(
        s.handle_key(key(KeyCode::Enter, KeyModifiers::NONE)),
        Some(Choice::Selected(1))
    );
    for k in [
        key(KeyCode::Esc, KeyModifiers::NONE),
        key(KeyCode::Char('q'), KeyModifiers::NONE),
        key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        key(KeyCode::Char('d'), KeyModifiers::CONTROL),
    ] {
        assert_eq!(
            Selector::new("t", items.clone(), true).handle_key(k),
            Some(Choice::Cancelled)
        );
    }
    assert_eq!(
        s.handle_key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
        None
    );
    assert_eq!(
        Selector::new("t", items.clone(), false)
            .handle_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
        None
    );
    let mut release = key(KeyCode::Enter, KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert_eq!(Selector::new("t", items, true).handle_key(release), None);
}

#[test]
fn selector_renders_with_carriage_returns_and_redraws_in_place() {
    let mut s = Selector::new(
        "Select Antigravity account",
        vec!["Personal Google".into(), "College Google".into()],
        true,
    );
    let mut first = Vec::new();
    s.render(&mut first).unwrap();
    let first = String::from_utf8(first).unwrap();
    assert!(
        !first.replace("\r\n", "").contains('\n'),
        "every newline must be \\r\\n in raw mode"
    );
    assert!(first.contains("> Personal Google"));

    s.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    let mut second = Vec::new();
    s.render(&mut second).unwrap();
    let second = String::from_utf8(second).unwrap();
    let lines = first.matches("\r\n").count();
    assert!(
        second.starts_with(&format!("\x1b[{lines}F")),
        "redraw must move back over the previous frame"
    );
    assert!(
        second.contains("\x1b[J"),
        "redraw must clear the previous frame"
    );
    assert!(second.contains("> College Google"));
    assert_eq!(
        second.matches("> ").count(),
        1,
        "no duplicated selection lines"
    );
}

// ── Zero-config OAuth client discovery & callback hardening ──────────────────

/// Fake token endpoint answering like Google for client-secret probes.
async fn fake_probe_endpoint(good_secret: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/token", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = s.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = if req.contains(&format!("client_secret={good_secret}")) {
                r#"{"error":"invalid_grant","error_description":"Malformed auth code."}"#
            } else {
                r#"{"error":"invalid_client","error_description":"Unauthorized"}"#
            };
            let resp = format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = s.write_all(resp.as_bytes()).await;
        }
    });
    url
}

#[tokio::test]
async fn oauth_client_is_discovered_from_agy_binary_and_verified() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("agy");
    let wrong = "GOCSPX-WRONGWRONGWRONGWRONGWRONGWRO";
    let right = "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";
    std::fs::write(&bin, format!("\x00junk{wrong}\x00more-bytes{right}\x00")).unwrap();
    let url = fake_probe_endpoint(right).await;
    let client = OAuthClient::discover_from_binary(&bin, &url).await.unwrap();
    assert_eq!(client.client_id, agy_auth::AGY_OAUTH_CLIENT_ID);
    assert!(
        !format!("{client:?}").contains("GOCSPX"),
        "Debug must not reveal the secret"
    );

    // A binary whose secrets are all rejected yields a clear, secret-free error.
    std::fs::write(&bin, format!("xx{wrong}xx")).unwrap();
    let err = OAuthClient::discover_from_binary(&bin, &url)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("requires OAuth configuration") && !err.contains("GOCSPX"),
        "{err}"
    );

    // No embedded configuration at all.
    std::fs::write(&bin, "no config here").unwrap();
    let err = OAuthClient::discover_from_binary(&bin, &url)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("does not contain a login configuration"),
        "{err}"
    );
}

#[tokio::test]
async fn callbacks_without_or_with_wrong_state_are_rejected_before_exchange() {
    let (url, captured) = fake_token_endpoint("200 OK", "{}").await;
    for query in ["code=abc", "code=abc&state=forged"] {
        let login = BrowserLogin::start_with(OAuthClient::new("c", "s"), "127.0.0.1:0", &url)
            .await
            .unwrap();
        let addr = login
            .redirect_uri()
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap()
            .to_string();
        let req = format!("GET /oauth2callback?{query} HTTP/1.1\r\n\r\n");
        tokio::spawn(async move {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            s.write_all(req.as_bytes()).await.unwrap();
        });
        let err = login
            .wait_for_callback(Duration::from_secs(5))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Invalid OAuth state"), "{err}");
    }
    assert!(
        captured.lock().unwrap().is_empty(),
        "the code must never be exchanged"
    );
}

#[tokio::test]
async fn finish_login_saves_only_after_successful_exchange() {
    let _env = Env::new();
    // Failure: nothing written.
    let (url, _) = fake_token_endpoint("400 Bad Request", r#"{"error":"invalid_grant"}"#).await;
    let login = BrowserLogin::start_with(OAuthClient::new("c", "s"), "127.0.0.1:0", &url)
        .await
        .unwrap();
    browser_redirect(&login, "4%2F0CODE").await;
    assert!(
        agy_auth::finish_browser_login(&login, "Personal Google", Duration::from_secs(5))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_dir(agy_auth::credentials_dir())
            .map(|d| d.count())
            .unwrap_or(0),
        0
    );
    drop(login);

    // Success: exactly one validated, typed credential for this label.
    let body = r#"{"access_token":"AT-SECRET","refresh_token":"RT-SECRET","expires_in":3599,"token_type":"Bearer","id_token":"h.eyJlbWFpbCI6InVAeC5jb20ifQ.s"}"#;
    let (url, _) = fake_token_endpoint("200 OK", body).await;
    let login = BrowserLogin::start_with(OAuthClient::new("c", "s"), "127.0.0.1:0", &url)
        .await
        .unwrap();
    browser_redirect(&login, "4%2F0CODE").await;
    let (cref, tok) =
        agy_auth::finish_browser_login(&login, "Personal Google", Duration::from_secs(5))
            .await
            .unwrap();
    let saved = agy_auth::validate_credential(&cref, "Personal Google").unwrap();
    assert_eq!(saved.label, "Personal Google");
    assert_eq!(saved.email.as_deref(), Some("u@x.com"));
    assert_eq!(tok.email().as_deref(), Some("u@x.com"));
    let raw = std::fs::read_to_string(agy_auth::credential_path(&cref).unwrap()).unwrap();
    assert!(
        !raw.contains("4/0CODE") && !raw.contains("code_verifier"),
        "code/verifier never stored"
    );
}

#[tokio::test]
async fn dropping_a_login_closes_its_callback_listener() {
    let login = BrowserLogin::start(OAuthClient::new("c", "s"))
        .await
        .unwrap();
    let addr = login
        .redirect_uri()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();
    drop(login);
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "listener must be closed after cancel"
    );
}

#[tokio::test]
async fn concurrent_logins_have_independent_state_and_ports() {
    let a = BrowserLogin::start(OAuthClient::new("c", "s"))
        .await
        .unwrap();
    let b = BrowserLogin::start(OAuthClient::new("c", "s"))
        .await
        .unwrap();
    assert_ne!(a.redirect_uri(), b.redirect_uri());
    let st = |l: &BrowserLogin| {
        l.authorization_url()
            .split("state=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string()
    };
    assert_ne!(st(&a), st(&b));
    // A's callback delivered to B is rejected (state belongs to A).
    let url_for_b = format!("{}?code=x&state={}", b.redirect_uri(), st(&a));
    assert!(b.code_from_pasted(&url_for_b).is_err());
}

// ── Per-session launch options ────────────────────────────────────────────────

fn launch_ctx(
    cref: &str,
    launch: Option<&ac_core::agy_launch::AgyLaunchOptions>,
) -> SessionContext {
    let mut ctx = SessionContext::new(Id::new(), "[interactive] s".into(), "agy".into())
        .with_account(Id::new(), cref.to_string());
    if let Some(l) = launch {
        ctx.workspace_path = l.working_dir.clone();
        ctx.agent_config = Some(serde_json::to_value(l).unwrap());
    }
    ctx
}

#[tokio::test]
async fn selected_permission_mode_model_and_directory_reach_the_agy_process() {
    use ac_core::agy_launch::{AgyExecutionMode, AgyLaunchOptions, AgyPermissionMode};
    let env = Env::new();
    let cref = make_cred("Personal Google", "MARK_A", "a@x.com");
    let work = env.tmp.path().join("projects/AgentDesk");
    std::fs::create_dir_all(&work).unwrap();
    let mut factory = CompositeAdapterFactory::new();
    let mut handles = Vec::new();

    // Default: no permission or execution flags at all, dangerous mode never implied.
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    handles.push((factory.create(launch_ctx(&cref, None), tx).unwrap(), rx));
    let l = parse_log(&env.wait_for_lines(1).await[0]);
    assert_eq!(l["ARGS"], "");
    assert_eq!(l["MARK"], "MARK_A", "existing account launches directly");

    let cases = [
        (
            AgyExecutionMode::Default,
            AgyPermissionMode::Normal,
            false,
            "",
        ),
        (
            AgyExecutionMode::AcceptEdits,
            AgyPermissionMode::Normal,
            false,
            "--mode=accept-edits,",
        ),
        (
            AgyExecutionMode::Plan,
            AgyPermissionMode::Normal,
            false,
            "--mode=plan,",
        ),
        (
            AgyExecutionMode::Default,
            AgyPermissionMode::DangerouslySkipPermissions,
            false,
            "--dangerously-skip-permissions,",
        ),
        (
            AgyExecutionMode::Default,
            AgyPermissionMode::Normal,
            true,
            "--sandbox,",
        ),
    ];
    for (i, (exec_mode, perm_mode, sandbox, expected)) in cases.iter().enumerate() {
        let opts = AgyLaunchOptions {
            execution_mode: *exec_mode,
            permission_mode: *perm_mode,
            sandbox: *sandbox,
            model: None,
            working_dir: Some(work.display().to_string()),
        };
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        handles.push((
            factory.create(launch_ctx(&cref, Some(&opts)), tx).unwrap(),
            rx,
        ));
        let l = parse_log(&env.wait_for_lines(i + 2).await[i + 1]);
        assert_eq!(&l["ARGS"], expected, "{exec_mode:?} {perm_mode:?}");
        assert_eq!(
            std::fs::canonicalize(&l["PWD"]).unwrap(),
            std::fs::canonicalize(&work).unwrap()
        );
        assert_eq!(l["GEMINI"], "unset");
    }

    let opts = AgyLaunchOptions {
        execution_mode: AgyExecutionMode::Plan,
        permission_mode: AgyPermissionMode::Normal,
        sandbox: false,
        model: Some("gemini-3.8-flash-medium".into()),
        working_dir: None,
    };
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    handles.push((
        factory.create(launch_ctx(&cref, Some(&opts)), tx).unwrap(),
        rx,
    ));
    let lines = env.wait_for_lines(cases.len() + 2).await;
    assert_eq!(
        parse_log(&lines[cases.len() + 1])["ARGS"],
        "--mode=plan,--model,gemini-3.8-flash-medium,"
    );
    assert!(
        lines.iter().all(|l| !l.contains("SECRET")),
        "no token in process args"
    );
}

#[tokio::test]
async fn launch_options_travel_through_the_daemon_to_agy_and_are_validated() {
    let env = Env::new();
    let d = start_daemon(env.tmp.path(), Box::new(CompositeAdapterFactory::new())).await;
    let aid = register(
        &d,
        agy_account(
            "Personal Google",
            &make_cred("Personal Google", "MARK_A", "a@x.com"),
        ),
    );
    let work = env.tmp.path().join("work dir");
    std::fs::create_dir_all(&work).unwrap();
    let launch = serde_json::json!({
        "execution_mode": "accept_edits",
        "permission_mode": "normal",
        "model": "gemini-3.8-flash-low",
        "working_dir": work.display().to_string(),
    });
    let create = |launch: serde_json::Value| {
        let client = &d.client;
        let aid = aid.clone();
        async move {
            client
                .send_command("session.create", serde_json::json!({"task_description": "[interactive] s", "agent_type": "agy", "account_id": aid, "launch": launch}))
                .await
                .unwrap()
        }
    };
    let resp = create(launch.clone()).await;
    let sid = Id::from(resp.result.unwrap()["session_id"].as_str().unwrap());
    let session = d.client.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(
        session.launch.as_ref().unwrap().execution_mode,
        ac_core::agy_launch::AgyExecutionMode::AcceptEdits
    );
    assert_eq!(
        session.launch.as_ref().unwrap().permission_mode,
        ac_core::agy_launch::AgyPermissionMode::Normal
    );
    let r = d
        .client
        .send_command("session.start", serde_json::json!({"session_id": sid}))
        .await
        .unwrap();
    assert!(r.error.is_none(), "{:?}", r.error);
    let line = parse_log(&env.wait_for_lines(1).await[0]).clone();
    let raw = &env.log_lines()[0];
    assert!(
        raw.contains("ARGS=--mode=accept-edits,--model,gemini-3.8-flash-low, "),
        "{raw}"
    );
    assert!(
        raw.ends_with(&format!(
            "PWD={}",
            std::fs::canonicalize(&work).unwrap().display()
        )),
        "{raw}"
    );
    assert_eq!(line["MARK"], "MARK_A");

    // Rejected: unknown mode, relative / missing directories, argument-like models.
    for bad in [
        serde_json::json!({"permission_mode": "yolo"}),
        serde_json::json!({"execution_mode": "yolo"}),
        serde_json::json!({"working_dir": "relative"}),
        serde_json::json!({"working_dir": "/definitely/not/here"}),
        serde_json::json!({"model": "--dangerously-skip-permissions"}),
    ] {
        let r = create(bad.clone()).await;
        assert_eq!(
            r.error.as_ref().map(|e| e.code.as_str()),
            Some("ValidationError"),
            "{bad}"
        );
    }
    assert_eq!(
        env.log_lines().len(),
        1,
        "rejected launches never start agy"
    );
}

// ── Login link handoff ────────────────────────────────────────────────────────

fn state_of(login: &BrowserLogin) -> String {
    login
        .authorization_url()
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn login_link_completes_from_a_pasted_redirect_after_rejecting_forgeries() {
    let _env = Env::new();
    let body = r#"{"access_token":"AT-SECRET","refresh_token":"RT-SECRET","expires_in":3599,"token_type":"Bearer","id_token":"h.eyJlbWFpbCI6InVAeC5jb20ifQ.s"}"#;
    let (url, captured) = fake_token_endpoint("200 OK", body).await;
    let login = BrowserLogin::start_with(OAuthClient::new("c", "s"), "127.0.0.1:0", &url)
        .await
        .unwrap();
    // The shareable link carries no secrets.
    let link = login.authorization_url().to_string();
    assert!(
        !link.contains("client_secret")
            && !link.contains("code_verifier")
            && !link.contains("SECRET")
    );
    assert!(link.contains("code_challenge_method=S256"));

    let (ptx, mut prx) = tokio::sync::mpsc::channel(4);
    ptx.send(format!("{}?code=4%2F0X&state=FORGED", login.redirect_uri()))
        .await
        .unwrap();
    ptx.send("not a url".into()).await.unwrap();
    ptx.send(format!(
        "{}?code=4%2F0GOOD&state={}",
        login.redirect_uri(),
        state_of(&login)
    ))
    .await
    .unwrap();
    let rejected = Arc::new(Mutex::new(Vec::<String>::new()));
    let rej = rejected.clone();
    let (cref, tok) = agy_auth::finish_login_with_handoff(
        &login,
        "Phone Login",
        Duration::from_secs(5),
        &mut prx,
        move |r| rej.lock().unwrap().push(r),
    )
    .await
    .unwrap();
    let rejected = rejected.lock().unwrap().clone();
    assert_eq!(rejected.len(), 2, "{rejected:?}");
    assert!(rejected[0].contains("Invalid OAuth state"));
    assert!(
        rejected.iter().all(|r| !r.contains("4/0")),
        "codes never echoed"
    );
    assert_eq!(tok.email().as_deref(), Some("u@x.com"));
    assert_eq!(
        agy_auth::validate_credential(&cref, "Phone Login")
            .unwrap()
            .label,
        "Phone Login"
    );
    let sent = captured.lock().unwrap().clone();
    assert!(
        sent.contains("code=4%2F0GOOD") && !sent.contains("0X"),
        "only the valid code is exchanged"
    );

    // One-time: the login is finished; the listener closes once it is dropped.
    let addr = login
        .redirect_uri()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();
    drop(login);
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn login_link_also_accepts_the_local_callback_and_expires() {
    let _env = Env::new();
    let body = r#"{"access_token":"AT-SECRET","refresh_token":"RT-SECRET","expires_in":3599,"token_type":"Bearer","id_token":"h.eyJlbWFpbCI6InVAeC5jb20ifQ.s"}"#;
    let (url, _) = fake_token_endpoint("200 OK", body).await;
    let login = BrowserLogin::start_with(OAuthClient::new("c", "s"), "127.0.0.1:0", &url)
        .await
        .unwrap();
    let (_ptx, mut prx) = tokio::sync::mpsc::channel::<String>(1);
    browser_redirect(&login, "4%2F0CODE").await;
    assert!(agy_auth::finish_login_with_handoff(
        &login,
        "Local",
        Duration::from_secs(5),
        &mut prx,
        |_| {}
    )
    .await
    .is_ok());

    // Expiry: nothing arrives → timed out, nothing saved.
    let before = std::fs::read_dir(agy_auth::credentials_dir())
        .unwrap()
        .count();
    let login = BrowserLogin::start_with(OAuthClient::new("c", "s"), "127.0.0.1:0", &url)
        .await
        .unwrap();
    let (ptx, mut prx) = tokio::sync::mpsc::channel::<String>(1);
    drop(ptx); // cancelled paste input still leaves the loopback path until expiry
    let err = agy_auth::finish_login_with_handoff(
        &login,
        "Late",
        Duration::from_millis(300),
        &mut prx,
        |_| {},
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("timed out"), "{err}");
    assert_eq!(
        std::fs::read_dir(agy_auth::credentials_dir())
            .unwrap()
            .count(),
        before
    );
}
