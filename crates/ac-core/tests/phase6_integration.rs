//! Phase 6 integration tests: Seamless Multi-Account Agent Switching.
//!
//! Covers:
//!  - Explicit and automatic deterministic account selection
//!  - Structured validation errors (AccountNotFound, IncompatibleAccount, AccountUnavailable, ConcurrencyLimitReached)
//!  - Switching an Idle session before start (atomic rebinding without restart)
//!  - Switching a running session with Controlled Hand-Off (snapshot, predecessor/successor linking, load management)
//!  - Unsupported switch mode rejection (UnsupportedCapability)
//!  - Hand-off target availability validation
//!  - Session snapshot serialization, versioning, and zero-secret leakage
//!  - IPC command round-trip for Phase 6 endpoints

use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

use ac_core::{
    account_manager::{AccountManager, AccountManagerHandle, AccountStore},
    adapter::mock::{MockAdapterFactory, MockScript, ScriptedEvent},
    event_store::EventStore,
    ipc::IpcServer,
    project_registry::{ProjectRegistry, ProjectRegistryHandle, ProjectStore},
    session::manager::{SessionManager, SessionManagerHandle},
    types::{
        Account, AccountSwitchMode, AdapterEvent, AgentEvent, ApiRequest, ApiResponse, Confidence,
        EventKind, Id, Project, ProviderCapabilities, SessionSnapshot, SessionState,
        WorkspacePolicy, SNAPSHOT_SCHEMA_VERSION,
    },
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_account(label: &str, provider: &str, agent_types: Vec<&str>, cap: u8) -> Account {
    Account::new(
        label.into(),
        provider.into(),
        agent_types.into_iter().map(String::from).collect(),
        format!("ref:{label}"),
        cap,
        vec!["test".into()],
    )
}

fn make_project(name: &str, policy: WorkspacePolicy) -> Project {
    Project::new(
        name.into(),
        format!("/tmp/ac_test_proj_{name}"),
        Some("mock".into()),
        vec![],
        policy,
    )
}

fn long_running_script() -> MockScript {
    vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            10_000,
            AdapterEvent::OutputChunk {
                text: "Working...".into(),
                confidence: Confidence::High,
            },
        ),
    ]
}

fn start_session_manager(
    adapter_caps: Option<ProviderCapabilities>,
    account_mgr: AccountManager,
    project_reg: ProjectRegistry,
) -> (SessionManagerHandle, broadcast::Receiver<AgentEvent>) {
    let mut factory = MockAdapterFactory::always(long_running_script());
    if let Some(caps) = adapter_caps {
        factory = factory.with_capabilities(caps);
    }

    let event_store = EventStore::open_in_memory().unwrap();
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (event_tx, event_rx) = broadcast::channel(512);

    let manager = SessionManager::new(event_store, cmd_rx, event_tx, 3, Box::new(factory))
        .with_account_manager(account_mgr)
        .with_project_registry(project_reg);

    tokio::spawn(async move { manager.run().await });
    (SessionManagerHandle::new(cmd_tx), event_rx)
}

// ── 1. Account Selection & Validation Tests ────────────────────────────────────

#[tokio::test]
async fn test_explicit_account_selection_success_and_validation() {
    let account_store = AccountStore::open_in_memory().unwrap();
    let mut acct_mgr = AccountManager::new(account_store).unwrap();

    let id_a = acct_mgr
        .register(make_account("acct-a", "anthropic", vec!["mock"], 2))
        .unwrap();
    let id_b = acct_mgr
        .register(make_account("acct-b", "anthropic", vec!["claude"], 2))
        .unwrap();
    let id_c = acct_mgr
        .register(make_account("acct-c", "openai", vec!["mock"], 1))
        .unwrap();

    // Disable id_b to test AccountUnavailable
    acct_mgr.disable(&id_b).unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = start_session_manager(None, acct_mgr, proj_reg);

    // 1. Explicit selection of Account A for a mock session succeeds
    let sid1 = mgr
        .create_with_context("Task 1".into(), "mock".into(), None, Some(id_a.clone()))
        .await
        .unwrap();

    let session1 = mgr.get(sid1.clone()).await.unwrap().unwrap();
    assert_eq!(session1.account_id, Some(id_a.clone()));

    // 2. Explicit selection of nonexistent account returns AccountNotFound
    let fake_id = Id::from("nonexistent-account-999");
    let err_not_found = mgr
        .create_with_context("Task 2".into(), "mock".into(), None, Some(fake_id))
        .await
        .unwrap_err();
    assert!(
        err_not_found.to_string().contains("AccountNotFound"),
        "expected AccountNotFound error, got: {err_not_found}"
    );

    // 3. Explicit selection of unavailable (disabled) account returns AccountUnavailable
    let err_unavail = mgr
        .create_with_context("Task B1".into(), "claude".into(), None, Some(id_b.clone()))
        .await
        .unwrap_err();
    assert!(
        err_unavail.to_string().contains("AccountUnavailable"),
        "expected AccountUnavailable error, got: {err_unavail}"
    );

    // 4. Concurrency limit reached
    // Account C has cap=1. Create first session with C:
    let _s_c1 = mgr
        .create_with_context("Task C1".into(), "mock".into(), None, Some(id_c.clone()))
        .await
        .unwrap();

    // Create second session with C -> fails with ConcurrencyLimitReached
    let err_cap = mgr
        .create_with_context("Task C2".into(), "mock".into(), None, Some(id_c.clone()))
        .await
        .unwrap_err();
    assert!(
        err_cap.to_string().contains("ConcurrencyLimitReached"),
        "expected ConcurrencyLimitReached error, got: {err_cap}"
    );
}

#[tokio::test]
async fn test_incompatible_account_rejected() {
    let account_store = AccountStore::open_in_memory().unwrap();
    let mut acct_mgr = AccountManager::new(account_store).unwrap();

    // Account only supports claude
    let id_claude = acct_mgr
        .register(make_account(
            "acct-claude-only",
            "anthropic",
            vec!["claude"],
            2,
        ))
        .unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = start_session_manager(None, acct_mgr, proj_reg);

    // Request mock agent with claude-only account
    let err = mgr
        .create_with_context("Task".into(), "mock".into(), None, Some(id_claude))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("IncompatibleAccount"),
        "expected IncompatibleAccount error, got: {err}"
    );
}

#[tokio::test]
async fn test_deterministic_least_loaded_selection_with_tie_breaker() {
    let account_store = AccountStore::open_in_memory().unwrap();
    let mut acct_mgr = AccountManager::new(account_store).unwrap();

    // Register two accounts with identical load (0) and cap (5)
    // Account Z has label "z-account"
    // Account A has label "a-account"
    let id_z = acct_mgr
        .register(make_account("z-account", "anthropic", vec!["mock"], 5))
        .unwrap();
    let id_a = acct_mgr
        .register(make_account("a-account", "anthropic", vec!["mock"], 5))
        .unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = start_session_manager(None, acct_mgr, proj_reg);

    // Auto-selection must tie-break deterministically by label: "a-account" < "z-account"
    let sid1 = mgr
        .create_with_context("Auto Task 1".into(), "mock".into(), None, None)
        .await
        .unwrap();
    let s1 = mgr.get(sid1).await.unwrap().unwrap();
    assert_eq!(
        s1.account_id,
        Some(id_a.clone()),
        "should pick a-account by tie breaker"
    );

    // Now a-account has load=1, z-account has load=0.
    // Next auto-selection must pick least-loaded: z-account
    let sid2 = mgr
        .create_with_context("Auto Task 2".into(), "mock".into(), None, None)
        .await
        .unwrap();
    let s2 = mgr.get(sid2).await.unwrap().unwrap();
    assert_eq!(
        s2.account_id,
        Some(id_z.clone()),
        "should pick least-loaded z-account"
    );
}

// ── 2. Account Switching: Before Start (Idle Rebind) ───────────────────────────

#[tokio::test]
async fn test_switch_account_before_start_idle_rebind() {
    let account_store = AccountStore::open_in_memory().unwrap();
    let mut acct_mgr = AccountManager::new(account_store).unwrap();

    let id1 = acct_mgr
        .register(make_account("acct-1", "anthropic", vec!["mock"], 2))
        .unwrap();
    let id2 = acct_mgr
        .register(make_account("acct-2", "anthropic", vec!["mock"], 2))
        .unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = start_session_manager(None, acct_mgr, proj_reg);

    // Create session in Idle state with Account 1
    let sid = mgr
        .create_with_context(
            "Idle switch task".into(),
            "mock".into(),
            None,
            Some(id1.clone()),
        )
        .await
        .unwrap();

    // Switch account before starting session
    let active_id = mgr.switch_account(sid.clone(), id2.clone()).await.unwrap();
    assert_eq!(active_id, sid, "Idle switch retains same session ID");

    // Verify session updated and state is still Idle
    let session = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session.account_id, Some(id2.clone()));
    assert_eq!(session.state, SessionState::Idle);

    // Verify events recorded
    let events = mgr.query_events(Some(sid.clone()), None).await.unwrap();
    let has_account_selected = events
        .iter()
        .any(|e| e.kind == EventKind::AccountSelected && e.payload["account_id"] == id2.0);
    assert!(
        has_account_selected,
        "AccountSelected event must be recorded for target account"
    );
}

// ── 3. Account Switching: Running Session (Controlled Hand-Off) ────────────────

#[tokio::test]
async fn test_switch_account_during_run_controlled_handoff() {
    let account_store = AccountStore::open_in_memory().unwrap();
    let mut acct_mgr = AccountManager::new(account_store).unwrap();

    let id1 = acct_mgr
        .register(make_account("acct-running-1", "anthropic", vec!["mock"], 2))
        .unwrap();
    let id2 = acct_mgr
        .register(make_account("acct-running-2", "anthropic", vec!["mock"], 2))
        .unwrap();

    let mut proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let proj = make_project("test-handoff-proj", WorkspacePolicy::Shared);
    let proj_id = proj_reg.register(proj).unwrap();

    // Mock adapter configured with RequiresRestart switch mode
    let caps = ProviderCapabilities::new(AccountSwitchMode::RequiresRestart, true, true);
    let (mgr, _rx) = start_session_manager(Some(caps), acct_mgr, proj_reg);

    // Create and start session
    let sid1 = mgr
        .create_with_context(
            "Long running task".into(),
            "mock".into(),
            Some(proj_id.clone()),
            Some(id1.clone()),
        )
        .await
        .unwrap();
    mgr.start(sid1.clone()).await.unwrap();

    // Wait for session to enter Working state
    tokio::time::sleep(Duration::from_millis(50)).await;
    let s1_before = mgr.get(sid1.clone()).await.unwrap().unwrap();
    assert_eq!(s1_before.state, SessionState::Working);

    // Execute controlled hand-off to Account 2
    let sid2 = mgr.switch_account(sid1.clone(), id2.clone()).await.unwrap();
    assert_ne!(
        sid1, sid2,
        "Hand-off must yield a distinct successor session ID"
    );

    // Predecessor verification
    let predecessor = mgr.get(sid1.clone()).await.unwrap().unwrap();
    assert_eq!(predecessor.state, SessionState::HandedOff);
    assert_eq!(predecessor.successor_id, Some(sid2.clone()));
    assert!(predecessor.context_snapshot_id.is_some());

    // Successor verification
    let successor = mgr.get(sid2.clone()).await.unwrap().unwrap();
    assert_eq!(successor.predecessor_id, Some(sid1.clone()));
    assert_eq!(successor.account_id, Some(id2.clone()));
    assert_eq!(successor.project_id, Some(proj_id.clone()));
    assert_eq!(successor.workspace_id, predecessor.workspace_id);
    assert_eq!(successor.task_description, predecessor.task_description);

    // Verify event audit trail for predecessor
    let pred_events = mgr.query_events(Some(sid1.clone()), None).await.unwrap();
    let kinds: Vec<EventKind> = pred_events.iter().map(|e| e.kind.clone()).collect();
    assert!(kinds.contains(&EventKind::AccountSwitchRequested));
    assert!(kinds.contains(&EventKind::SessionSnapshotCreated));
    assert!(kinds.contains(&EventKind::SessionHandOffStarted));
    assert!(kinds.contains(&EventKind::SessionHandOffCompleted));
    assert!(kinds.contains(&EventKind::AccountSwitchCompleted));
}

#[tokio::test]
async fn test_unsupported_switch_mode_rejection() {
    let account_store = AccountStore::open_in_memory().unwrap();
    let mut acct_mgr = AccountManager::new(account_store).unwrap();

    let id1 = acct_mgr
        .register(make_account("acct-u1", "anthropic", vec!["mock"], 2))
        .unwrap();
    let id2 = acct_mgr
        .register(make_account("acct-u2", "anthropic", vec!["mock"], 2))
        .unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let caps = ProviderCapabilities::new(AccountSwitchMode::Unsupported, false, false);
    let (mgr, _rx) = start_session_manager(Some(caps), acct_mgr, proj_reg);

    let sid = mgr
        .create_with_context("Test task".into(), "mock".into(), None, Some(id1.clone()))
        .await
        .unwrap();
    mgr.start(sid.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Switch should be rejected with UnsupportedCapability
    let err = mgr
        .switch_account(sid.clone(), id2.clone())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("UnsupportedCapability"),
        "expected UnsupportedCapability error, got: {err}"
    );

    // Original session remains working on Account 1
    let session = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);
    assert_eq!(session.account_id, Some(id1));
}

#[tokio::test]
async fn test_switch_to_exhausted_account_fails() {
    let account_store = AccountStore::open_in_memory().unwrap();
    let mut acct_mgr = AccountManager::new(account_store).unwrap();

    let id1 = acct_mgr
        .register(make_account("acct-src", "anthropic", vec!["mock"], 2))
        .unwrap();
    // Target account with cap=1
    let id_full = acct_mgr
        .register(make_account("acct-full", "anthropic", vec!["mock"], 1))
        .unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let caps = ProviderCapabilities::new(AccountSwitchMode::RequiresRestart, true, true);
    let (mgr, _rx) = start_session_manager(Some(caps), acct_mgr, proj_reg);

    // Fill id_full capacity
    let _occupant = mgr
        .create_with_context(
            "Occupant".into(),
            "mock".into(),
            None,
            Some(id_full.clone()),
        )
        .await
        .unwrap();

    // Start session on id1
    let sid = mgr
        .create_with_context("Src session".into(), "mock".into(), None, Some(id1.clone()))
        .await
        .unwrap();
    mgr.start(sid.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Attempt switch to exhausted account
    let err = mgr
        .switch_account(sid.clone(), id_full.clone())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("ConcurrencyLimitReached"),
        "expected ConcurrencyLimitReached, got: {err}"
    );

    // Predecessor session is undamaged
    let session = mgr.get(sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);
    assert_eq!(session.account_id, Some(id1));
}

#[tokio::test]
async fn test_stale_session_switch_fails() {
    let account_store = AccountStore::open_in_memory().unwrap();
    let mut acct_mgr = AccountManager::new(account_store).unwrap();
    let id1 = acct_mgr
        .register(make_account("acct-stale", "anthropic", vec!["mock"], 2))
        .unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = start_session_manager(None, acct_mgr, proj_reg);

    let fake_sid = Id::from("stale-session-id");
    let err = mgr.switch_account(fake_sid, id1).await.unwrap_err();
    assert!(
        err.to_string().contains("not found") || err.to_string().contains("StaleSession"),
        "expected not found / stale error, got: {err}"
    );
}

// ── 4. Session Snapshot Model & Secret Exclusion ───────────────────────────────

#[test]
fn test_session_snapshot_serialization_and_zero_secret_leakage() {
    let session_id = Id::from("sess-01JABCDEF");
    let mut snapshot = SessionSnapshot::new(
        session_id.clone(),
        "claude".into(),
        "Refactor authentication module".into(),
        SessionState::Working,
        42,
    );
    snapshot.project_id = Some(Id::from("proj-auth"));
    snapshot.workspace_id = Some(Id::from("ws-auth-1"));
    snapshot.workspace_path = Some("/home/user/workspace/auth".into());
    snapshot.summary = Some("Safe snapshot for hand-off".into());
    snapshot.metadata.insert("step".into(), "linting".into());

    // Serialize to JSON
    let json_str = serde_json::to_string(&snapshot).unwrap();

    // Verify snapshot schema version
    assert_eq!(snapshot.version, SNAPSHOT_SCHEMA_VERSION);
    assert!(json_str.contains(r#""version":1"#));

    // Verify zero secret leakage: raw secrets or credential tokens must not exist
    let forbidden_tokens = [
        "sk-ant-api-test-secret-12345",
        "bearer_token",
        "api_key",
        "raw_credentials",
        "password",
    ];
    for token in forbidden_tokens {
        assert!(
            !json_str.to_lowercase().contains(token),
            "Snapshot leaked sensitive key/pattern: {token}"
        );
    }

    // Deserialization round-trip
    let deserialized: SessionSnapshot = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.session_id, session_id);
    assert_eq!(deserialized.agent_type, "claude");
    assert_eq!(deserialized.event_seq, 42);
    assert_eq!(
        deserialized.workspace_path,
        Some("/home/user/workspace/auth".into())
    );
    assert_eq!(
        deserialized.metadata.get("step").map(|s| s.as_str()),
        Some("linting")
    );
}

// ── 5. IPC Server Phase 6 Round-Trip ───────────────────────────────────────────

#[tokio::test]
async fn test_ipc_phase6_commands_roundtrip() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let sock_path = tmp_dir.path().join("agentcontrol_p6_test.sock");

    let event_store = EventStore::open_in_memory().unwrap();
    let accounts_db = tmp_dir.path().join("accounts_ipc.db");
    let projects_db = tmp_dir.path().join("projects_ipc.db");

    let account_store = AccountStore::open(&accounts_db).unwrap();
    let account_mgr = AccountManager::new(account_store).unwrap();
    let acct_handle = AccountManagerHandle::new(account_mgr);

    let project_store = ProjectStore::open(&projects_db).unwrap();
    let project_reg = ProjectRegistry::new(project_store).unwrap();
    let proj_handle = ProjectRegistryHandle::new(project_reg);

    let (event_tx, _event_rx) = broadcast::channel(256);
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let factory = MockAdapterFactory::always(long_running_script()).with_capabilities(
        ProviderCapabilities::new(AccountSwitchMode::RequiresRestart, true, true),
    );

    let session_mgr =
        SessionManager::new(event_store, cmd_rx, event_tx.clone(), 3, Box::new(factory))
            .with_account_manager_handle(acct_handle.clone())
            .with_project_registry_handle(proj_handle.clone());

    let session_handle = SessionManagerHandle::new(cmd_tx);
    tokio::spawn(async move { session_mgr.run().await });

    let server = IpcServer::bind(&sock_path, session_handle.clone(), event_tx)
        .unwrap()
        .with_account_manager(acct_handle)
        .with_project_registry(proj_handle);

    tokio::spawn(async move { server.run().await });
    tokio::time::sleep(Duration::from_millis(50)).await;

    async fn send_ipc(
        socket_path: &std::path::Path,
        cmd: &str,
        params: serde_json::Value,
    ) -> ApiResponse {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixStream;

        let stream = UnixStream::connect(socket_path).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();

        let req = ApiRequest {
            v: 1,
            id: "test-req".into(),
            cmd: cmd.into(),
            params,
        };
        let line = serde_json::to_string(&req).unwrap() + "\n";
        write_half.write_all(line.as_bytes()).await.unwrap();

        let mut reader = BufReader::new(read_half).lines();
        let resp_line = reader.next_line().await.unwrap().unwrap();
        serde_json::from_str(&resp_line).unwrap()
    }

    // Register accounts via IPC
    let resp1 = send_ipc(
        &sock_path,
        "account.register",
        serde_json::json!({
            "label": "ipc-acct-1",
            "provider": "anthropic",
            "agent_types": ["mock"],
            "credential_ref": "ref:ipc1",
            "concurrency_cap": 2,
            "tags": ["test"],
        }),
    )
    .await;
    assert!(resp1.ok, "register account 1 failed: {:?}", resp1.error);
    let aid1 = resp1.result.unwrap()["account_id"]
        .as_str()
        .map(Id::from)
        .unwrap();

    let resp2 = send_ipc(
        &sock_path,
        "account.register",
        serde_json::json!({
            "label": "ipc-acct-2",
            "provider": "anthropic",
            "agent_types": ["mock"],
            "credential_ref": "ref:ipc2",
            "concurrency_cap": 2,
            "tags": ["test"],
        }),
    )
    .await;
    assert!(resp2.ok, "register account 2 failed: {:?}", resp2.error);
    let aid2 = resp2.result.unwrap()["account_id"]
        .as_str()
        .map(Id::from)
        .unwrap();

    // 1. account.query_availability
    let avail_resp = send_ipc(
        &sock_path,
        "account.query_availability",
        serde_json::json!({ "agent_type": "mock", "tags": [] }),
    )
    .await;
    assert!(avail_resp.ok);
    let accounts_val = avail_resp.result.unwrap()["accounts"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(accounts_val.len(), 2);
    assert!(accounts_val
        .iter()
        .all(|a| a["is_available"].as_bool().unwrap_or(false)));

    // 2. Create session with aid1
    let create_resp = send_ipc(
        &sock_path,
        "session.create",
        serde_json::json!({
            "task_description": "IPC switch test",
            "agent_type": "mock",
            "account_id": aid1.0,
        }),
    )
    .await;
    assert!(create_resp.ok);
    let sid = create_resp.result.unwrap()["session_id"]
        .as_str()
        .map(Id::from)
        .unwrap();

    // 3. session.snapshot
    let snap_resp = send_ipc(
        &sock_path,
        "session.snapshot",
        serde_json::json!({ "session_id": sid.0 }),
    )
    .await;
    assert!(snap_resp.ok);
    let snap_obj = &snap_resp.result.unwrap()["snapshot"];
    assert_eq!(snap_obj["session_id"].as_str().unwrap(), sid.0);
    assert_eq!(snap_obj["version"].as_u64().unwrap(), 1);

    // 4. session.select_account (rebind before start)
    let sel_resp = send_ipc(
        &sock_path,
        "session.select_account",
        serde_json::json!({ "session_id": sid.0, "account_id": aid2.0 }),
    )
    .await;
    assert!(sel_resp.ok);

    let sess_after_select = send_ipc(
        &sock_path,
        "session.get",
        serde_json::json!({ "session_id": sid.0 }),
    )
    .await
    .result
    .unwrap();
    assert_eq!(sess_after_select["account_id"].as_str().unwrap(), aid2.0);

    // 5. Start session and perform session.switch_account
    send_ipc(
        &sock_path,
        "session.start",
        serde_json::json!({ "session_id": sid.0 }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let switch_resp = send_ipc(
        &sock_path,
        "session.switch_account",
        serde_json::json!({ "session_id": sid.0, "target_account_id": aid1.0 }),
    )
    .await;
    assert!(switch_resp.ok, "switch failed: {:?}", switch_resp.error);
    let active_id = switch_resp.result.unwrap()["active_session_id"]
        .as_str()
        .map(Id::from)
        .unwrap();
    assert_ne!(
        active_id, sid,
        "Controlled hand-off via IPC must create successor session"
    );

    let successor_sess = send_ipc(
        &sock_path,
        "session.get",
        serde_json::json!({ "session_id": active_id.0 }),
    )
    .await
    .result
    .unwrap();
    assert_eq!(successor_sess["predecessor_id"].as_str().unwrap(), sid.0);
    assert_eq!(successor_sess["account_id"].as_str().unwrap(), aid1.0);
}
