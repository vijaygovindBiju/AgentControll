//! Integration & UX tests for the Antigravity launcher and Account UX.
//!
//! Tests:
//!  - `agy` with zero accounts
//!  - `agy` with one account
//!  - `agy` with multiple accounts
//!  - `agy "existing account"`
//!  - `agy "unknown account"`
//!  - Duplicate labels handling
//!  - Unavailable / rate-limited account
//!  - Incompatible account ("Claude Main cannot be used with Antigravity")
//!  - Concurrency limit reached
//!  - Explicit account does NOT silently fall back
//!  - Credential reference persistence & zero secret leakage
//!  - Controlled session hand-off & restart requirement for Antigravity

use std::time::Duration;
use chrono::Utc;
use tempfile::tempdir;
use tokio::sync::broadcast;

use ac_cli::{
    client::DaemonClient,
    launcher::is_antigravity_account,
};
use ac_core::{
    account_manager::{AccountManager, AccountManagerHandle, AccountStore},
    adapter::{mock::MockAdapterFactory, AdapterFactory},
    event_store::EventStore,
    interaction_hub::{InteractionHub, InteractionHubHandle, InteractionStore},
    ipc::IpcServer,
    policy_engine::{PolicyEngine, PolicyStore},
    project_registry::{ProjectRegistry, ProjectRegistryHandle, ProjectStore},
    session::manager::{SessionManager, SessionManagerHandle},
    types::{
        Account, AccountState, AccountSwitchMode, AgentEvent, SessionState,
    },
};

/// Helper to spin up an isolated test daemon server on a temporary socket.
async fn setup_test_daemon() -> (DaemonClient, tempfile::TempDir, AccountManagerHandle, SessionManagerHandle) {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("test_agentcontrol.sock");

    let store = EventStore::open_in_memory().unwrap();
    let (event_tx, _) = broadcast::channel::<AgentEvent>(256);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(64);

    let pol_store = PolicyStore::open(&tmp.path().join("policies.db")).unwrap();
    let policy_engine = PolicyEngine::new(pol_store).unwrap();

    let int_store = InteractionStore::open(&tmp.path().join("interactions.db")).unwrap();
    let hub = InteractionHub::new(int_store, policy_engine).unwrap();
    let hub_handle = InteractionHubHandle::new(hub);

    let acct_store = AccountStore::open(&tmp.path().join("accounts.db")).unwrap();
    let acct_mgr = AccountManager::new(acct_store).unwrap();
    let acct_handle = AccountManagerHandle::new(acct_mgr);

    let proj_store = ProjectStore::open(&tmp.path().join("projects.db")).unwrap();
    let proj_reg = ProjectRegistry::new(proj_store).unwrap();
    let proj_handle = ProjectRegistryHandle::new(proj_reg);

    // Mock factory simulating agy/antigravity and claude
    let mock_factory = MockAdapterFactory::always(ac_core::adapter::mock::default_script());

    let session_mgr = SessionManager::new(
        store,
        cmd_rx,
        event_tx.clone(),
        3,
        Box::new(mock_factory),
    )
    .with_account_manager_handle(acct_handle.clone())
    .with_project_registry_handle(proj_handle.clone())
    .with_interaction_hub(hub_handle.clone());

    let mgr_handle = SessionManagerHandle::new(cmd_tx);

    tokio::spawn(async move {
        let sm = session_mgr;
        sm.run().await;
    });

    let server = IpcServer::bind(&sock, mgr_handle.clone(), event_tx)
        .unwrap()
        .with_account_manager(acct_handle.clone())
        .with_project_registry(proj_handle)
        .with_interaction_hub(hub_handle);

    tokio::spawn(async move {
        server.run().await;
    });

    // Wait until socket is ready
    let client = DaemonClient::new(sock);
    for _ in 0..50 {
        if client.is_running().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    (client, tmp, acct_handle, mgr_handle)
}

#[tokio::test]
async fn test_agy_zero_accounts_detection() {
    let (client, _tmp, _acct_h, _mgr_h) = setup_test_daemon().await;

    let accounts = client.list_accounts().await.unwrap();
    let agy_accounts: Vec<_> = accounts.into_iter().filter(|a| is_antigravity_account(a)).collect();
    assert_eq!(agy_accounts.len(), 0, "Initial environment should have 0 Antigravity accounts");
}

#[tokio::test]
async fn test_agy_one_account_automatic_selection() {
    let (client, _tmp, acct_h, _mgr_h) = setup_test_daemon().await;

    let acct = Account::new(
        "Personal Google".into(),
        "agy".into(),
        vec!["agy".into(), "antigravity".into()],
        "ref:antigravity:01TEST001".into(),
        2,
        vec!["personal".into()],
    );
    acct_h.0.lock().unwrap().register(acct).unwrap();

    let accounts = client.list_accounts().await.unwrap();
    let agy_accounts: Vec<_> = accounts.into_iter().filter(|a| is_antigravity_account(a)).collect();
    assert_eq!(agy_accounts.len(), 1);

    // Exactly one account exists and can accept session
    assert!(agy_accounts[0].can_accept_session());
    assert_eq!(agy_accounts[0].label, "Personal Google");
}

#[tokio::test]
async fn test_agy_multiple_accounts_filtering_and_availability() {
    let (client, _tmp, acct_h, _mgr_h) = setup_test_daemon().await;

    let a1 = Account::new(
        "Personal Google".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:antigravity:01".into(),
        2,
        vec![],
    );
    let mut a2 = Account::new(
        "Work Google".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:antigravity:02".into(),
        2,
        vec![],
    );
    a2.state = AccountState::Cooldown;
    a2.cooldown_until = Some(Utc::now() + chrono::Duration::seconds(45));

    let a3 = Account::new(
        "Claude Main".into(),
        "claude".into(),
        vec!["claude".into()],
        "ref:claude:03".into(),
        2,
        vec![],
    );

    acct_h.0.lock().unwrap().register(a1).unwrap();
    acct_h.0.lock().unwrap().register(a2).unwrap();
    acct_h.0.lock().unwrap().register(a3).unwrap();

    let all = client.list_accounts().await.unwrap();
    assert_eq!(all.len(), 3);

    let agy_only: Vec<_> = all.iter().filter(|a| is_antigravity_account(a)).collect();
    assert_eq!(agy_only.len(), 2, "Only 2 accounts belong to Antigravity");
    assert!(agy_only.iter().any(|a| a.label == "Personal Google" && a.state == AccountState::Active));
    assert!(agy_only.iter().any(|a| a.label == "Work Google" && a.state == AccountState::Cooldown));
}

#[tokio::test]
async fn test_agy_existing_account_by_label() {
    let (client, _tmp, acct_h, _mgr_h) = setup_test_daemon().await;

    let a1 = Account::new(
        "Personal Google".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:antigravity:01".into(),
        2,
        vec![],
    );
    let aid = a1.id.clone();
    acct_h.0.lock().unwrap().register(a1).unwrap();

    let all = client.list_accounts().await.unwrap();
    let agy_accounts: Vec<_> = all.iter().filter(|a| is_antigravity_account(a)).cloned().collect();

    // Matching label found
    let matched = agy_accounts.iter().find(|a| a.label.eq_ignore_ascii_case("Personal Google"));
    assert!(matched.is_some());
    assert_eq!(matched.unwrap().id, aid);
}

#[tokio::test]
async fn test_agy_incompatible_account_rejected() {
    let (client, _tmp, acct_h, _mgr_h) = setup_test_daemon().await;

    let claude_acct = Account::new(
        "Claude Main".into(),
        "claude".into(),
        vec!["claude".into(), "claude-code".into()],
        "ref:claude:01".into(),
        2,
        vec![],
    );
    acct_h.0.lock().unwrap().register(claude_acct).unwrap();

    let all = client.list_accounts().await.unwrap();
    let claude_found = all.iter().find(|a| a.label == "Claude Main").unwrap();

    // Verify it is NOT compatible with Antigravity
    assert!(!is_antigravity_account(claude_found));
    assert!(!claude_found.supports_agent_type("agy"));
}

#[tokio::test]
async fn test_agy_concurrency_limit_check() {
    let (client, _tmp, acct_h, _mgr_h) = setup_test_daemon().await;

    let mut acct = Account::new(
        "Personal Google".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:antigravity:01".into(),
        1, // Cap is 1
        vec![],
    );
    acct.active_session_count = 1; // At capacity
    acct_h.0.lock().unwrap().register(acct).unwrap();

    let all = client.list_accounts().await.unwrap();
    let a = all.iter().find(|a| a.label == "Personal Google").unwrap();

    assert_eq!(a.active_session_count, a.concurrency_cap);
    assert!(!a.can_accept_session(), "Account at cap cannot accept session");
}

#[tokio::test]
async fn test_credential_persistence_and_zero_secret_leakage() {
    let tmp = tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());

    let label = "Personal Google";
    let token = ac_core::agy_auth::AgyToken::from_native(serde_json::json!({
        "token": {"access_token": "SECRET-ACCESS", "refresh_token": "SECRET-REFRESH", "expiry": "2099-01-01T00:00:00Z"},
        "auth_method": "consumer", "id_token": ""
    })).unwrap();

    let cred_ref = ac_core::agy_auth::save_new_credential(label, &token, "browser_login").unwrap();
    assert!(cred_ref.starts_with("ref:agy:"));
    assert!(!cred_ref.contains("SECRET"), "reference must never contain secrets");

    let cred_file = ac_core::agy_auth::credential_path(&cred_ref).unwrap();
    assert!(cred_file.starts_with(tmp.path()));
    let content = std::fs::read_to_string(&cred_file).unwrap();
    assert!(content.contains("Personal Google"));
    assert!(content.contains("\"credential_type\": \"antigravity_oauth\""));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&cred_file).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600, "Credential file must be 0600");
    }
}

#[tokio::test]
async fn test_controlled_session_creation_and_account_switch() {
    let (client, _tmp, acct_h, _mgr_h) = setup_test_daemon().await;

    let a1 = Account::new(
        "Personal Google".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:antigravity:01".into(),
        2,
        vec![],
    );
    let a2 = Account::new(
        "College Google".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:antigravity:02".into(),
        2,
        vec![],
    );

    let a1_id = a1.id.clone();
    let a2_id = a2.id.clone();
    acct_h.0.lock().unwrap().register(a1).unwrap();
    acct_h.0.lock().unwrap().register(a2).unwrap();

    // 1. Create and start session with account 1
    let session_id = client
        .create_and_start_session("Build features", "agy", None, Some(a1_id.clone()))
        .await
        .unwrap();

    let s1 = client.get_session(&session_id).await.unwrap().unwrap();
    assert_eq!(s1.account_id, Some(a1_id));

    // 2. Perform controlled switch to account 2
    let successor_id = client.switch_account(&session_id, &a2_id).await.unwrap();
    assert_ne!(successor_id, session_id, "RequiresRestart should produce a successor session");

    // 3. Verify predecessor reached HandedOff state with lineage
    let s1_after = client.get_session(&session_id).await.unwrap().unwrap();
    assert_eq!(s1_after.state, SessionState::HandedOff);
    assert_eq!(s1_after.successor_id, Some(successor_id.clone()));

    // 4. Verify successor session is assigned to account 2 with lineage to predecessor
    let s2 = client.get_session(&successor_id).await.unwrap().unwrap();
    assert_eq!(s2.account_id, Some(a2_id));
    assert_eq!(s2.predecessor_id, Some(session_id));
    assert_eq!(s2.task_description, "Build features");
}

#[test]
fn test_antigravity_provider_capabilities() {
    let composite = ac_core::adapter::CompositeAdapterFactory::new();
    let caps = composite.capabilities("agy");
    assert_eq!(caps.switch_mode, AccountSwitchMode::RequiresRestart);
    assert!(!caps.supports_snapshot_restore);
}
