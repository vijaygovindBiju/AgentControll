//! Phase 2 integration tests: Account Manager + Project Registry + Session integration.

use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

use ac_core::{
    account_manager::{AccountManager, AccountStore},
    adapter::mock::{default_script, MockAdapterFactory},
    event_store::EventStore,
    project_registry::{ProjectRegistry, ProjectStore},
    session::manager::{SessionManager, SessionManagerHandle},
    types::{
        Account, AgentEvent, EventKind, Id, Project, SessionState,
        WorkspacePolicy,
    },
};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_account(label: &str, tags: Vec<&str>) -> Account {
    Account::new(
        label.into(),
        "anthropic".into(),
        vec!["mock".into()],
        "ref:test".into(),
        2,
        tags.into_iter().map(String::from).collect(),
    )
}

fn make_project(name: &str, policy: WorkspacePolicy) -> Project {
    Project::new(
        name.into(),
        format!("/tmp/{name}"),
        Some("mock".into()),
        vec![],
        policy,
    )
}

/// Build a SessionManager wired with Phase 2 managers.
fn make_phase2_manager(
    event_store: EventStore,
    account_mgr: AccountManager,
    project_reg: ProjectRegistry,
) -> (SessionManagerHandle, broadcast::Receiver<AgentEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (event_tx, event_rx) = broadcast::channel(256);
    let manager = SessionManager::new(
        event_store,
        cmd_rx,
        event_tx,
        3,
        Box::new(MockAdapterFactory::always(default_script())),
    )
    .with_account_manager(account_mgr)
    .with_project_registry(project_reg);
    tokio::spawn(async move { manager.run().await });
    (SessionManagerHandle::new(cmd_tx), event_rx)
}

async fn wait_for_event<F>(rx: &mut broadcast::Receiver<AgentEvent>, pred: F) -> AgentEvent
where
    F: Fn(&AgentEvent) -> bool,
{
    loop {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("Timed out waiting for event")
            .expect("Event channel closed");
        if pred(&event) {
            return event;
        }
    }
}

// ── Session + Account auto-selection ─────────────────────────────────────────

#[tokio::test]
async fn session_auto_selects_available_account() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let aid = acct_mgr.register(make_account("auto", vec![])).unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let sid = mgr.create_with_context("task".into(), "mock".into(), None, None).await.unwrap();
    let session = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session.account_id, Some(aid));
}

#[tokio::test]
async fn session_explicit_account_assignment() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let aid1 = acct_mgr.register(make_account("a1", vec![])).unwrap();
    let _aid2 = acct_mgr.register(make_account("a2", vec![])).unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    // Explicitly request aid1
    let sid = mgr
        .create_with_context("task".into(), "mock".into(), None, Some(aid1.clone()))
        .await
        .unwrap();
    let session = mgr.get(sid).await.unwrap().unwrap();
    assert_eq!(session.account_id, Some(aid1));
}

#[tokio::test]
async fn no_available_account_returns_error() {
    // Account manager with no accounts registered
    let acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let result = mgr.create_with_context("task".into(), "mock".into(), None, None).await;
    assert!(result.is_err(), "Expected NoAccountAvailable error");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("NoAccountAvailable"), "Error should contain NoAccountAvailable, got: {msg}");
}

#[tokio::test]
async fn disabled_account_not_selected() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let aid = acct_mgr.register(make_account("a", vec![])).unwrap();
    acct_mgr.disable(&aid).unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let result = mgr.create_with_context("task".into(), "mock".into(), None, None).await;
    assert!(result.is_err(), "Disabled account must not be selected");
}

#[tokio::test]
async fn account_load_incremented_on_create() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let aid = acct_mgr.register(make_account("a", vec![])).unwrap();
    assert_eq!(acct_mgr.get(&aid).unwrap().active_session_count, 0);

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    // Create two sessions on the same account
    mgr.create_with_context("t1".into(), "mock".into(), None, Some(aid.clone())).await.unwrap();
    mgr.create_with_context("t2".into(), "mock".into(), None, Some(aid.clone())).await.unwrap();

    // At capacity (cap=2), third should fail
    let result = mgr.create_with_context("t3".into(), "mock".into(), None, Some(aid.clone())).await;
    assert!(result.is_err(), "Account at capacity should be rejected");
}

#[tokio::test]
async fn account_load_decremented_on_stop() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let aid = acct_mgr.register(make_account("a", vec![])).unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, mut rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let sid = mgr.create_with_context("task".into(), "mock".into(), None, Some(aid.clone())).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();
    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
    }).await;
    mgr.stop(sid.clone(), None).await.unwrap();

    // The session is stopped; the account load should be back to 0.
    // We verify indirectly: creating another session on the same explicit account should succeed.
    let result = mgr.create_with_context("t2".into(), "mock".into(), None, Some(aid.clone())).await;
    assert!(result.is_ok(), "Account should be available again after session stop");
}

// ── Session + Project binding ─────────────────────────────────────────────────

#[tokio::test]
async fn session_binds_to_project_workspace() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    acct_mgr.register(make_account("a", vec![])).unwrap();

    let mut proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let pid = proj_reg.register(make_project("myproject", WorkspacePolicy::Shared)).unwrap();

    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let sid = mgr.create_with_context("task".into(), "mock".into(), Some(pid.clone()), None).await.unwrap();
    let session = mgr.get(sid).await.unwrap().unwrap();
    assert_eq!(session.project_id, Some(pid));
    assert!(session.workspace_id.is_some(), "Session should have a workspace assigned");
}

#[tokio::test]
async fn session_with_nonexistent_project_fails() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    acct_mgr.register(make_account("a", vec![])).unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let result = mgr.create_with_context(
        "task".into(),
        "mock".into(),
        Some(Id::from("does-not-exist")),
        None,
    ).await;
    assert!(result.is_err(), "Session with non-existent project should fail");
}

#[tokio::test]
async fn worktree_policy_creates_separate_workspaces() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    // Enough cap for 4 sessions
    let mut a = make_account("a", vec![]);
    a.concurrency_cap = 4;
    acct_mgr.register(a).unwrap();

    let mut proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let pid = proj_reg
        .register(make_project("wt", WorkspacePolicy::WorktreePerSession))
        .unwrap();

    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let sid1 = mgr.create_with_context("t1".into(), "mock".into(), Some(pid.clone()), None).await.unwrap();
    let sid2 = mgr.create_with_context("t2".into(), "mock".into(), Some(pid.clone()), None).await.unwrap();

    let s1 = mgr.get(sid1).await.unwrap().unwrap();
    let s2 = mgr.get(sid2).await.unwrap().unwrap();
    assert_ne!(
        s1.workspace_id, s2.workspace_id,
        "WorktreePerSession should assign different workspaces"
    );
}

#[tokio::test]
async fn shared_policy_reuses_workspace() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let mut a = make_account("a", vec![]);
    a.concurrency_cap = 4;
    acct_mgr.register(a).unwrap();

    let mut proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let pid = proj_reg
        .register(make_project("shared", WorkspacePolicy::Shared))
        .unwrap();

    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let sid1 = mgr.create_with_context("t1".into(), "mock".into(), Some(pid.clone()), None).await.unwrap();
    let sid2 = mgr.create_with_context("t2".into(), "mock".into(), Some(pid.clone()), None).await.unwrap();

    let s1 = mgr.get(sid1).await.unwrap().unwrap();
    let s2 = mgr.get(sid2).await.unwrap().unwrap();
    assert_eq!(
        s1.workspace_id, s2.workspace_id,
        "Shared policy should reuse the same workspace"
    );
}

// ── Phase 1 compatibility ─────────────────────────────────────────────────────

#[tokio::test]
async fn phase1_create_without_project_or_account_still_works() {
    // No account manager, no project registry: old Phase 1 path
    let store = EventStore::open_in_memory().unwrap();
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (event_tx, _rx) = broadcast::channel(256);
    let manager = SessionManager::new(
        store,
        cmd_rx,
        event_tx,
        3,
        Box::new(MockAdapterFactory::always(default_script())),
    );
    // No with_account_manager / with_project_registry calls
    tokio::spawn(async move { manager.run().await });
    let mgr = SessionManagerHandle::new(cmd_tx);

    // Phase 1 API still works
    let sid = mgr.create("task".into(), "mock".into()).await.unwrap();
    let session = mgr.get(sid).await.unwrap().unwrap();
    assert!(session.account_id.is_none());
    assert!(session.project_id.is_none());
}

// ── Persistence across restart ────────────────────────────────────────────────

#[tokio::test]
async fn account_persists_across_reload() {
    let dir = tempfile::tempdir().unwrap();
    let acct_db = dir.path().join("accounts.db");
    let aid;
    {
        let store = AccountStore::open(&acct_db).unwrap();
        let mut mgr = AccountManager::new(store).unwrap();
        aid = mgr.register(make_account("persisted", vec!["tag1"])).unwrap();
    }
    {
        let store = AccountStore::open(&acct_db).unwrap();
        let mgr = AccountManager::new(store).unwrap();
        let a = mgr.get(&aid).unwrap();
        assert_eq!(a.label, "persisted");
        assert!(a.tags.contains(&"tag1".to_string()));
    }
}

#[tokio::test]
async fn project_and_workspace_persist_across_reload() {
    let dir = tempfile::tempdir().unwrap();
    let proj_db = dir.path().join("projects.db");
    let pid;
    {
        let store = ProjectStore::open(&proj_db).unwrap();
        let mut reg = ProjectRegistry::new(store).unwrap();
        pid = reg.register(make_project("proj", WorkspacePolicy::Shared)).unwrap();
        reg.resolve_workspace(&pid, &Id::new()).unwrap();
    }
    {
        let store = ProjectStore::open(&proj_db).unwrap();
        let reg = ProjectRegistry::new(store).unwrap();
        assert!(reg.get(&pid).is_some());
        assert_eq!(reg.workspaces_for(&pid).len(), 1);
    }
}

#[tokio::test]
async fn session_account_binding_recovered_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let events_db = dir.path().join("events.db");
    let acct_db = dir.path().join("accounts.db");

    let sid;
    let aid;

    // First run: create a session with account binding
    {
        let store = AccountStore::open(&acct_db).unwrap();
        let mut acct_mgr = AccountManager::new(store).unwrap();
        aid = acct_mgr.register(make_account("a", vec![])).unwrap();

        let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
        let (mgr, mut rx) = make_phase2_manager(
            EventStore::open(&events_db).unwrap(),
            acct_mgr,
            proj_reg,
        );

        sid = mgr.create_with_context("task".into(), "mock".into(), None, Some(aid.clone())).await.unwrap();
        mgr.start(sid.clone()).await.unwrap();
        wait_for_event(&mut rx, |e| {
            e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
        }).await;
    }

    // Second run: recover from event log
    {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (event_tx, _) = broadcast::channel::<AgentEvent>(256);

        let store = AccountStore::open(&acct_db).unwrap();
        let acct_mgr2 = AccountManager::new(store).unwrap();
        let proj_reg2 = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();

        let mut manager = SessionManager::new(
            EventStore::open(&events_db).unwrap(),
            cmd_rx,
            event_tx,
            3,
            Box::new(MockAdapterFactory::always(default_script())),
        )
        .with_account_manager(acct_mgr2)
        .with_project_registry(proj_reg2);

        manager.recover_from_store().unwrap();
        tokio::spawn(async move { manager.run().await });

        let mgr = SessionManagerHandle::new(cmd_tx);
        let session = mgr.get(sid.clone()).await.unwrap().unwrap();

        // Session should have been recovered (Crashed after restart)
        assert_eq!(session.state, SessionState::Crashed);
        // Phase 2 fields should be restored from event log
        assert_eq!(session.account_id, Some(aid));
    }
}

// ── Cooldown handling ─────────────────────────────────────────────────────────

#[tokio::test]
async fn rate_limited_account_not_selected() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let aid = acct_mgr.register(make_account("a", vec![])).unwrap();
    acct_mgr.apply_rate_limit(&aid, None).unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let result = mgr.create_with_context("t".into(), "mock".into(), None, None).await;
    assert!(result.is_err(), "Rate-limited account must not be auto-selected");
}

#[tokio::test]
async fn exhausted_account_not_selected() {
    let mut acct_mgr = AccountManager::new(AccountStore::open_in_memory().unwrap()).unwrap();
    let aid = acct_mgr.register(make_account("a", vec![])).unwrap();
    acct_mgr.mark_exhausted(&aid).unwrap();

    let proj_reg = ProjectRegistry::new(ProjectStore::open_in_memory().unwrap()).unwrap();
    let (mgr, _rx) = make_phase2_manager(
        EventStore::open_in_memory().unwrap(),
        acct_mgr,
        proj_reg,
    );

    let result = mgr.create_with_context("t".into(), "mock".into(), None, None).await;
    assert!(result.is_err(), "Exhausted account must not be selected");
}
