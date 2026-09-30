//! Integration tests: session lifecycle, recovery, and event store.
//!
//! These tests drive the `SessionManager` directly (no IPC socket).

use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

use ac_core::{
    adapter::mock::{
        default_script, start_fail_script, MockAdapterFactory,
    },
    event_store::EventStore,
    session::manager::{SessionManager, SessionManagerHandle},
    types::{AgentEvent, EventKind, Id, SessionState},
};

// ── Test helpers ──────────────────────────────────────────────────────────────

fn make_manager_with_factory(
    store: EventStore,
    factory: MockAdapterFactory,
) -> (SessionManagerHandle, broadcast::Receiver<AgentEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (event_tx, event_rx) = broadcast::channel(256);
    let manager = SessionManager::new(
        store,
        cmd_rx,
        event_tx,
        3, // max_restarts
        Box::new(factory),
    );
    tokio::spawn(async move { manager.run().await });
    (SessionManagerHandle::new(cmd_tx), event_rx)
}

fn make_manager(store: EventStore) -> (SessionManagerHandle, broadcast::Receiver<AgentEvent>) {
    make_manager_with_factory(store, MockAdapterFactory::always(default_script()))
}

// Drain broadcast receiver until we find an event matching the predicate,
// or time out.
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

// ── Session lifecycle ─────────────────────────────────────────────────────────

#[tokio::test]
async fn create_session_produces_event() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, mut rx) = make_manager(store);

    let id = mgr.create("test task".into(), "mock".into()).await.unwrap();

    let event = wait_for_event(&mut rx, |e| e.kind == EventKind::SessionCreated).await;
    assert_eq!(event.session_id.as_ref().unwrap(), &id);
}

#[tokio::test]
async fn start_session_reaches_working() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, mut rx) = make_manager(store);

    let id = mgr.create("test task".into(), "mock".into()).await.unwrap();
    mgr.start(id.clone()).await.unwrap();

    let event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.payload["to"].as_str() == Some("working")
    })
    .await;
    assert_eq!(event.session_id.as_ref().unwrap(), &id);

    let session = mgr.get(id.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);
}

#[tokio::test]
async fn pause_and_resume_session() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, mut rx) = make_manager(store);

    let id = mgr.create("test task".into(), "mock".into()).await.unwrap();
    mgr.start(id.clone()).await.unwrap();

    // Wait until working
    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
    })
    .await;

    mgr.pause(id.clone()).await.unwrap();
    let session = mgr.get(id.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Paused);

    mgr.resume(id.clone()).await.unwrap();
    let session = mgr.get(id.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);
}

#[tokio::test]
async fn stop_session_reaches_stopped() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, mut rx) = make_manager(store);

    let id = mgr.create("test task".into(), "mock".into()).await.unwrap();
    mgr.start(id.clone()).await.unwrap();

    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
    })
    .await;

    mgr.stop(id.clone(), Some("done".into())).await.unwrap();

    let session = mgr.get(id.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Stopped);
    assert!(session.stopped_at.is_some());
}

#[tokio::test]
async fn stop_idle_session_also_works() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, _rx) = make_manager(store);

    let id = mgr.create("task".into(), "mock".into()).await.unwrap();
    // Stop without starting
    mgr.stop(id.clone(), None).await.unwrap();

    let session = mgr.get(id.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Stopped);
}

#[tokio::test]
async fn cannot_start_already_started_session() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, mut rx) = make_manager(store);

    let id = mgr.create("task".into(), "mock".into()).await.unwrap();
    mgr.start(id.clone()).await.unwrap();

    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
    })
    .await;

    let result = mgr.start(id.clone()).await;
    assert!(result.is_err(), "Expected error starting an already-working session");
}

#[tokio::test]
async fn start_fail_script_transitions_to_failed() {
    let store = EventStore::open_in_memory().unwrap();
    let factory = MockAdapterFactory::always(start_fail_script("intentional failure"));
    let (mgr, _rx) = make_manager_with_factory(store, factory);

    let id = mgr.create("task".into(), "mock".into()).await.unwrap();
    let result = mgr.start(id.clone()).await;
    assert!(result.is_err());

    let session = mgr.get(id.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Failed);
}

#[tokio::test]
async fn list_returns_all_sessions() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, _rx) = make_manager(store);

    mgr.create("task 1".into(), "mock".into()).await.unwrap();
    mgr.create("task 2".into(), "mock".into()).await.unwrap();
    mgr.create("task 3".into(), "mock".into()).await.unwrap();

    let sessions = mgr.list().await.unwrap();
    assert_eq!(sessions.len(), 3);
}

#[tokio::test]
async fn get_nonexistent_session_returns_none() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, _rx) = make_manager(store);

    let result = mgr.get(Id::from("does-not-exist")).await.unwrap();
    assert!(result.is_none());
}

// ── Event store persistence ───────────────────────────────────────────────────

#[tokio::test]
async fn events_are_persisted_to_store() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("events.db");

    {
        // First daemon instance
        let store = EventStore::open(&db_path).unwrap();
        let (mgr, mut rx) = make_manager(store);

        let id = mgr.create("persisted task".into(), "mock".into()).await.unwrap();
        mgr.start(id.clone()).await.unwrap();

        wait_for_event(&mut rx, |e| {
            e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
        })
        .await;
    }

    // Reopen the store and verify events are present
    let store = EventStore::open(&db_path).unwrap();
    let events = store.query(0, None, None).unwrap();
    // Should have: SessionCreated, SessionStartRequested, StateChanged(→Starting),
    //              SessionReady, StateChanged(→Working), SessionStarted
    assert!(events.len() >= 3, "Expected at least 3 persisted events, got {}", events.len());
}

// ── Recovery from event log ───────────────────────────────────────────────────

#[tokio::test]
async fn recovery_rebuilds_session_states() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("recovery.db");

    let session_id;

    {
        // First daemon run: create and start a session
        let store = EventStore::open(&db_path).unwrap();
        let (mgr, mut rx) = make_manager(store);

        session_id = mgr.create("recovery task".into(), "mock".into()).await.unwrap();
        mgr.start(session_id.clone()).await.unwrap();

        wait_for_event(&mut rx, |e| {
            e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
        })
        .await;
        // session_id is Working; drop the manager (simulates daemon restart)
    }

    // Second daemon run: recover
    {
        let store = EventStore::open(&db_path).unwrap();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (event_tx, _event_rx) = broadcast::channel::<AgentEvent>(256);

        let mut manager = SessionManager::new(
            store,
            cmd_rx,
            event_tx,
            3,
            Box::new(MockAdapterFactory::always(default_script())),
        );

        // Recovery: session was Working at shutdown → should be Crashed after recovery
        manager.recover_from_store().unwrap();
        tokio::spawn(async move { manager.run().await });

        let mgr = SessionManagerHandle::new(cmd_tx);
        let session = mgr.get(session_id.clone()).await.unwrap().unwrap();

        // After recovery, orphaned Working session becomes Crashed
        assert_eq!(
            session.state,
            SessionState::Crashed,
            "Expected Crashed after recovery, got {:?}",
            session.state
        );
    }
}

#[tokio::test]
async fn recovery_stopped_session_stays_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("stopped_recovery.db");

    let session_id;

    {
        let store = EventStore::open(&db_path).unwrap();
        let (mgr, mut rx) = make_manager(store);

        session_id = mgr.create("task".into(), "mock".into()).await.unwrap();
        mgr.start(session_id.clone()).await.unwrap();

        wait_for_event(&mut rx, |e| {
            e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
        })
        .await;

        mgr.stop(session_id.clone(), None).await.unwrap();
    }

    {
        let store = EventStore::open(&db_path).unwrap();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (event_tx, _) = broadcast::channel::<AgentEvent>(256);

        let mut manager = SessionManager::new(
            store,
            cmd_rx,
            event_tx,
            3,
            Box::new(MockAdapterFactory::always(default_script())),
        );
        manager.recover_from_store().unwrap();
        tokio::spawn(async move { manager.run().await });

        let mgr = SessionManagerHandle::new(cmd_tx);
        let session = mgr.get(session_id.clone()).await.unwrap().unwrap();

        // Terminal state; stays Stopped after recovery
        assert_eq!(session.state, SessionState::Stopped);
    }
}

// ── StateChanged events ───────────────────────────────────────────────────────

#[tokio::test]
async fn state_changed_events_carry_from_and_to() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, mut rx) = make_manager(store);

    let id = mgr.create("task".into(), "mock".into()).await.unwrap();
    mgr.start(id.clone()).await.unwrap();

    let event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged && e.payload["from"].as_str() == Some("idle")
    })
    .await;

    assert_eq!(event.payload["from"].as_str(), Some("idle"));
    assert_eq!(event.payload["to"].as_str(), Some("starting"));
}

// ── Pause/resume from Idle is rejected ───────────────────────────────────────

#[tokio::test]
async fn pause_idle_session_is_rejected() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, _rx) = make_manager(store);

    let id = mgr.create("task".into(), "mock".into()).await.unwrap();
    let result = mgr.pause(id.clone()).await;
    assert!(result.is_err(), "Expected error pausing an Idle session");

    // State should still be Idle
    let session = mgr.get(id.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Idle);
}

// ── Session removal ─────────────────────────────────────────────────────────

#[tokio::test]
async fn remove_session_stops_and_removes_from_manager() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, mut rx) = make_manager(store);

    let id = mgr.create("test task".into(), "mock".into()).await.unwrap();
    mgr.start(id.clone()).await.unwrap();

    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
    })
    .await;

    // Remove the running session
    mgr.remove(id.clone()).await.unwrap();

    // Verify SessionRemoved event was broadcast
    let removed_event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::SessionRemoved && e.session_id == Some(id.clone())
    })
    .await;
    assert_eq!(removed_event.payload["session_id"].as_str(), Some(id.0.as_str()));

    // Verify session no longer exists in manager
    let session = mgr.get(id.clone()).await.unwrap();
    assert!(session.is_none());

    let all_sessions = mgr.list().await.unwrap();
    assert!(all_sessions.iter().all(|s| s.id != id));
}

#[tokio::test]
async fn remove_idle_session_removes_immediately() {
    let store = EventStore::open_in_memory().unwrap();
    let (mgr, mut rx) = make_manager(store);

    let id = mgr.create("idle task".into(), "mock".into()).await.unwrap();
    assert!(mgr.get(id.clone()).await.unwrap().is_some());

    mgr.remove(id.clone()).await.unwrap();

    let removed_event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::SessionRemoved && e.session_id == Some(id.clone())
    })
    .await;
    assert_eq!(removed_event.session_id, Some(id.clone()));

    assert!(mgr.get(id.clone()).await.unwrap().is_none());
}

#[tokio::test]
async fn recovery_replayed_removed_session_stays_removed() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test_events.db");
    let session_id: Id;

    {
        let store = EventStore::open(&db_path).unwrap();
        let (mgr, mut rx) = make_manager(store);

        session_id = mgr.create("task to remove".into(), "mock".into()).await.unwrap();
        mgr.start(session_id.clone()).await.unwrap();

        wait_for_event(&mut rx, |e| {
            e.kind == EventKind::StateChanged && e.payload["to"].as_str() == Some("working")
        })
        .await;

        mgr.remove(session_id.clone()).await.unwrap();
        assert!(mgr.get(session_id.clone()).await.unwrap().is_none());
    }

    {
        let store = EventStore::open(&db_path).unwrap();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (event_tx, _) = broadcast::channel::<AgentEvent>(256);

        let mut manager = SessionManager::new(
            store,
            cmd_rx,
            event_tx,
            3,
            Box::new(MockAdapterFactory::always(default_script())),
        );
        manager.recover_from_store().unwrap();
        tokio::spawn(async move { manager.run().await });

        let mgr = SessionManagerHandle::new(cmd_tx);
        let session = mgr.get(session_id.clone()).await.unwrap();
        assert!(session.is_none(), "Removed session should remain absent after store recovery");
    }
}

