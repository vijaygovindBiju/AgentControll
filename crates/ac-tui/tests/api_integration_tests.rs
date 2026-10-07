//! Integration tests for ApiClient and TUI input handling against real IPC server.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use std::{path::PathBuf, time::Duration};
use tokio::sync::broadcast;

use ac_core::{
    account_manager::{AccountManager, AccountManagerHandle, AccountStore},
    adapter::mock::{MockAdapterFactory, ScriptedEvent},
    event_store::EventStore,
    interaction_hub::{InteractionHub, InteractionHubHandle, InteractionStore},
    ipc::IpcServer,
    policy_engine::{PolicyEngine, PolicyStore},
    project_registry::{ProjectRegistry, ProjectRegistryHandle, ProjectStore},
    session::manager::{SessionManager, SessionManagerHandle},
    types::{
        Account, AdapterEvent, Id, InteractionKind, InteractionState, PolicyDecision, Project,
        SessionState, WorkspacePolicy,
    },
};
use ac_tui::{
    app::{App, Modal, Tab},
    client::{ApiClient, ClientError},
    event::{handle_key, refresh_data},
};

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

async fn setup_test_server(
    script: Vec<ScriptedEvent>,
) -> (
    PathBuf,
    tempfile::TempDir,
    broadcast::Sender<ac_core::types::AgentEvent>,
) {
    let tmp = tempfile::tempdir().unwrap();
    let sock = tmp.path().join("test.sock");

    let store = EventStore::open_in_memory().unwrap();
    let (event_tx, _) = broadcast::channel(256);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(64);

    let pstore = PolicyStore::open(&tmp.path().join("policies.db")).unwrap();
    let policy_engine = PolicyEngine::new(pstore).unwrap();

    let istore = InteractionStore::open(&tmp.path().join("interactions.db")).unwrap();
    let hub = InteractionHub::new(istore, policy_engine).unwrap();
    let hub_handle = InteractionHubHandle::new(hub);

    let astore = AccountStore::open(&tmp.path().join("accounts.db")).unwrap();
    let mut acct_mgr = AccountManager::new(astore).unwrap();
    let acct = Account::new(
        "Test Account".into(),
        "mock".into(),
        vec!["mock".into()],
        "ref:test".into(),
        2,
        vec!["test".into()],
    );
    let _ = acct_mgr.register(acct);
    let acct_handle = AccountManagerHandle::new(acct_mgr);

    let astore2 = AccountStore::open(&tmp.path().join("accounts.db")).unwrap();
    let acct_mgr2 = AccountManager::new(astore2).unwrap();

    let prstore = ProjectStore::open(&tmp.path().join("projects.db")).unwrap();
    let mut proj_reg = ProjectRegistry::new(prstore).unwrap();
    let proj = Project::new(
        "Test Project".into(),
        tmp.path().to_string_lossy().into(),
        Some("mock".into()),
        vec!["test".into()],
        WorkspacePolicy::Shared,
    );
    let _ = proj_reg.register(proj);
    let proj_handle = ProjectRegistryHandle::new(proj_reg);

    let prstore2 = ProjectStore::open(&tmp.path().join("projects.db")).unwrap();
    let proj_reg2 = ProjectRegistry::new(prstore2).unwrap();

    let adapter_factory = Box::new(MockAdapterFactory::always(script));

    let session_mgr = SessionManager::new(store, cmd_rx, event_tx.clone(), 2, adapter_factory)
        .with_account_manager(acct_mgr2)
        .with_project_registry(proj_reg2)
        .with_interaction_hub(hub_handle.clone());

    let mgr_handle = SessionManagerHandle::new(cmd_tx);

    tokio::spawn(async move {
        session_mgr.run().await;
    });

    let server = IpcServer::bind(&sock, mgr_handle, event_tx.clone())
        .unwrap()
        .with_account_manager(acct_handle)
        .with_project_registry(proj_handle)
        .with_interaction_hub(hub_handle);

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    (sock, tmp, event_tx)
}

#[tokio::test]
async fn test_api_session_lifecycle_and_commands() {
    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            50,
            AdapterEvent::OutputChunk {
                text: "Hello from mock agent".into(),
                confidence: ac_core::types::Confidence::High,
            },
        ),
    ];

    let (sock, _tmp, _) = setup_test_server(script).await;
    let client = ApiClient::new(sock);

    assert!(client.check_daemon().await);

    // 1. Create Session
    let sid = client
        .create_session("Test task", "mock", None, None)
        .await
        .unwrap();

    // 2. Get Session
    let session = client.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(session.task_description, "Test task");
    assert_eq!(session.state, SessionState::Idle);

    // 3. Start Session
    client.start_session(&sid).await.unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;

    let session = client.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);

    // 4. Steer Session
    client
        .steer_session(&sid, "Steering instruction: prioritize login")
        .await
        .unwrap();

    // 5. Pause & Resume Session
    client.pause_session(&sid).await.unwrap();
    let session = client.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Paused);

    client.resume_session(&sid).await.unwrap();
    let session = client.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);

    // 6. List Sessions
    let sessions = client.list_sessions().await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, sid);

    // 7. Stop Session
    client
        .stop_session(&sid, Some("Test finished"))
        .await
        .unwrap();
    let session = client.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Stopped);

    // 8. Remove Session
    client.remove_session(&sid).await.unwrap();
    let get_res = client.get_session(&sid).await;
    assert!(matches!(get_res, Err(ClientError::ApiError { ref code, .. }) if code == "NotFound"));
    let sessions = client.list_sessions().await.unwrap();
    assert!(sessions.is_empty());
}

#[tokio::test]
async fn test_api_interaction_commands_flow() {
    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::QuestionRaised {
                prompt: "Should I proceed with refactoring?".into(),
            },
        ),
    ];

    let (sock, _tmp, _) = setup_test_server(script).await;
    let client = ApiClient::new(sock);

    let sid = client
        .create_session("Refactoring task", "mock", None, None)
        .await
        .unwrap();
    client.start_session(&sid).await.unwrap();

    // Wait for question to be raised and session to transition to WaitingForHuman
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 1. List pending interactions
    let pending = client.list_interactions(Some(&sid), true).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, InteractionKind::Question);
    assert_eq!(pending[0].prompt, "Should I proceed with refactoring?");

    // 2. Resolve interaction with human response
    let resolved = client
        .resolve_interaction(
            &pending[0].id,
            Some(PolicyDecision::Allow),
            Some("Yes, refactor the database layer"),
        )
        .await
        .unwrap();

    assert_eq!(resolved.state, InteractionState::HumanResolved);
    assert_eq!(
        resolved.response.as_deref(),
        Some("Yes, refactor the database layer")
    );

    // Session resumes to Working
    tokio::time::sleep(Duration::from_millis(50)).await;
    let session = client.get_session(&sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);
}

#[tokio::test]
async fn test_api_accounts_projects_and_events() {
    let script = vec![ScriptedEvent::immediate(AdapterEvent::Ready)];
    let (sock, _tmp, _) = setup_test_server(script).await;
    let client = ApiClient::new(sock);

    // 1. List accounts
    let accounts = client.list_accounts().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].label, "Test Account");

    // 2. List projects
    let projects = client.list_projects().await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "Test Project");

    // 3. Query events
    let sid = client
        .create_session("Event query task", "mock", None, None)
        .await
        .unwrap();
    client.start_session(&sid).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let events = client.query_events(Some(&sid), Some(10)).await.unwrap();
    assert!(!events.is_empty(), "Events should be recorded");
}

#[tokio::test]
async fn test_api_event_subscription_stream() {
    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            30,
            AdapterEvent::OutputChunk {
                text: "Chunk 1".into(),
                confidence: ac_core::types::Confidence::High,
            },
        ),
    ];

    let (sock, _tmp, _) = setup_test_server(script).await;
    let client = ApiClient::new(sock);

    let mut rx = client.subscribe_events().await.unwrap();

    let sid = client
        .create_session("Sub task", "mock", None, None)
        .await
        .unwrap();
    client.start_session(&sid).await.unwrap();

    // Verify events arrive over subscription stream
    let mut received = Vec::new();
    let timeout = tokio::time::sleep(Duration::from_millis(300));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            Some(evt) = rx.recv() => {
                received.push(evt.kind);
                if received.len() >= 3 {
                    break;
                }
            }
            _ = &mut timeout => {
                break;
            }
        }
    }

    assert!(!received.is_empty(), "Expected subscription stream events");
}

#[tokio::test]
async fn test_failure_handling_daemon_unavailable() {
    let nonexistent = PathBuf::from("/tmp/nonexistent_socket_test_12345.sock");
    let client = ApiClient::new(nonexistent);

    assert!(!client.check_daemon().await);

    let res = client.list_sessions().await;
    assert!(matches!(res, Err(ClientError::DaemonUnavailable(_, _))));
}

#[tokio::test]
async fn test_failure_handling_invalid_state_and_not_found() {
    let script = vec![ScriptedEvent::immediate(AdapterEvent::Ready)];
    let (sock, _tmp, _) = setup_test_server(script).await;
    let client = ApiClient::new(sock);

    // 1. Get non-existent session returns None
    let missing_id = Id::new();
    let res = client.get_session(&missing_id).await;
    assert!(matches!(res, Err(ClientError::ApiError { ref code, .. }) if code == "NotFound"));

    // 2. Pause idle session returns InvalidState error
    let sid = client
        .create_session("Idle task", "mock", None, None)
        .await
        .unwrap();
    let pause_res = client.pause_session(&sid).await;
    assert!(
        matches!(pause_res, Err(ClientError::ApiError { ref code, .. }) if code == "InvalidState")
    );
}

#[tokio::test]
async fn test_failure_handling_stale_interaction() {
    let script = vec![ScriptedEvent::immediate(AdapterEvent::Ready)];
    let (sock, _tmp, _) = setup_test_server(script).await;
    let client = ApiClient::new(sock);

    // Resolve unknown interaction
    let bogus_id = Id::new();
    let res = client
        .resolve_interaction(&bogus_id, Some(PolicyDecision::Allow), None)
        .await;
    assert!(matches!(res, Err(ClientError::ApiError { ref code, .. }) if code == "NotFound"));
}

#[tokio::test]
async fn test_keyboard_input_handling_and_modals() {
    let script = vec![ScriptedEvent::immediate(AdapterEvent::Ready)];
    let (sock, _tmp, _) = setup_test_server(script).await;
    let client = ApiClient::new(sock);

    let mut app = App::new();
    refresh_data(&mut app, &client).await;

    let sid = client
        .create_session("Key task", "mock", None, None)
        .await
        .unwrap();
    client.start_session(&sid).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    refresh_data(&mut app, &client).await;

    // 1. Switch to Sessions tab using '2'
    handle_key(&mut app, &client, make_key(KeyCode::Char('2')))
        .await
        .unwrap();
    assert_eq!(app.current_tab, Tab::Sessions);

    // 2. Open Steer modal using 's'
    handle_key(&mut app, &client, make_key(KeyCode::Char('s')))
        .await
        .unwrap();
    assert!(matches!(app.active_modal, Some(Modal::Steer { .. })));

    // Type "continue"
    for c in "continue".chars() {
        handle_key(&mut app, &client, make_key(KeyCode::Char(c)))
            .await
            .unwrap();
    }
    if let Some(Modal::Steer { ref input, .. }) = app.active_modal {
        assert_eq!(input, "continue");
    }

    // Submit with Enter
    handle_key(&mut app, &client, make_key(KeyCode::Enter))
        .await
        .unwrap();
    assert!(app.active_modal.is_none());
    assert!(app
        .status_message
        .as_ref()
        .map(|(m, _, _)| m.contains("Steered"))
        .unwrap_or(false));

    // 3. Open Stop modal using 'x'
    handle_key(&mut app, &client, make_key(KeyCode::Char('x')))
        .await
        .unwrap();
    assert!(matches!(app.active_modal, Some(Modal::ConfirmStop { .. })));

    // Cancel with 'n'
    handle_key(&mut app, &client, make_key(KeyCode::Char('n')))
        .await
        .unwrap();
    assert!(app.active_modal.is_none());

    // 4. Open Help modal using '?'
    handle_key(&mut app, &client, make_key(KeyCode::Char('?')))
        .await
        .unwrap();
    assert!(matches!(app.active_modal, Some(Modal::Help)));
    handle_key(&mut app, &client, make_key(KeyCode::Esc))
        .await
        .unwrap();
    assert!(app.active_modal.is_none());

    // 5. Open Session Detail using Enter
    handle_key(&mut app, &client, make_key(KeyCode::Enter))
        .await
        .unwrap();
    assert!(app.session_detail_id.is_some());
    // Esc is sent to PTY, does NOT close session
    handle_key(&mut app, &client, make_key(KeyCode::Esc))
        .await
        .unwrap();
    assert!(app.session_detail_id.is_some());
    // Close with Ctrl+Q
    handle_key(
        &mut app,
        &client,
        KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        },
    )
    .await
    .unwrap();
    assert!(app.session_detail_id.is_none());

    // 6. Quit with 'q'
    handle_key(&mut app, &client, make_key(KeyCode::Char('q')))
        .await
        .unwrap();
    assert!(app.should_quit);
}

#[tokio::test]
async fn test_url_picker_modal_navigation_and_actions() {
    let mut app = App::new();
    let client = ApiClient::new(PathBuf::from("/nonexistent.sock"));

    let urls = vec![
        "https://github.com/agentcontrol".to_string(),
        "https://docs.agentcontrol.dev".to_string(),
        "http://localhost:3000".to_string(),
    ];

    app.active_modal = Some(Modal::UrlPicker {
        session_id: Id::from("sess-1"),
        urls: urls.clone(),
        selected_index: 0,
    });

    // Down arrow advances selected index
    handle_key(&mut app, &client, make_key(KeyCode::Down))
        .await
        .unwrap();
    assert_eq!(
        app.active_modal,
        Some(Modal::UrlPicker {
            session_id: Id::from("sess-1"),
            urls: urls.clone(),
            selected_index: 1,
        })
    );

    // Up arrow returns to 0
    handle_key(&mut app, &client, make_key(KeyCode::Up))
        .await
        .unwrap();
    assert_eq!(
        app.active_modal,
        Some(Modal::UrlPicker {
            session_id: Id::from("sess-1"),
            urls: urls.clone(),
            selected_index: 0,
        })
    );

    // 'y' copies to clipboard and dismisses modal
    handle_key(&mut app, &client, make_key(KeyCode::Char('y')))
        .await
        .unwrap();
    assert!(app.active_modal.is_none());
    assert!(app.status_message.is_some());
    let (msg, _, _) = app.status_message.unwrap();
    assert!(msg.contains("https://github.com/agentcontrol"));
}

