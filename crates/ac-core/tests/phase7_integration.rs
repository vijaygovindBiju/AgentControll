//! Phase 7 Integration Tests: Control API v1 Schema Freeze, WebSocket Loopback,
//! Token Authentication, Scope Enforcement, Subscription Filtering & Catch-Up Replay.

use ac_core::{
    account_manager::{AccountManager, AccountManagerHandle, AccountStore},
    adapter::CompositeAdapterFactory,
    event_store::EventStore,
    ipc::IpcServer,
    session::manager::{SessionManager, SessionManagerHandle},
    types::{AgentEvent, ApiResponse, EventKind, TokenScope},
    ws::{TokenRegistry, WsServer},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use tempfile::tempdir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::{broadcast, mpsc},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::header::AUTHORIZATION, Message},
};

/// Helper to set up a running session manager with an event store.
async fn setup_test_harness() -> (
    SessionManagerHandle,
    AccountManagerHandle,
    broadcast::Sender<AgentEvent>,
    tempfile::TempDir,
) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("events.db");
    let accts_db = dir.path().join("accounts.db");

    let store = EventStore::open(&db_path).unwrap();
    let (event_tx, _) = broadcast::channel::<AgentEvent>(256);
    let (cmd_tx, cmd_rx) = mpsc::channel(64);

    let acct_store = AccountStore::open(&accts_db).unwrap();
    let mut acct_mgr = AccountManager::new(acct_store).unwrap();
    let mock_acct = ac_core::types::Account::new(
        "Default Mock".into(),
        "mock".into(),
        vec!["mock".into()],
        "mock-ref".into(),
        10,
        vec![],
    );
    acct_mgr.register(mock_acct).unwrap();
    let acct_handle = AccountManagerHandle::new(acct_mgr);

    let acct_store2 = AccountStore::open(&accts_db).unwrap();
    let acct_mgr2 = AccountManager::new(acct_store2).unwrap();

    let adapter_factory = Box::new(CompositeAdapterFactory::new());
    let mut manager = SessionManager::new(store, cmd_rx, event_tx.clone(), 3, adapter_factory)
        .with_account_manager(acct_mgr2);

    manager.recover_from_store().unwrap();
    let mgr_handle = SessionManagerHandle::new(cmd_tx);

    tokio::spawn(async move { manager.run().await });

    (mgr_handle, acct_handle, event_tx, dir)
}

#[tokio::test]
async fn test_ws_unauthenticated_connection_rejected_or_denied() {
    let (mgr, acct, evt_tx, _dir) = setup_test_harness().await;

    let tokens = TokenRegistry::new()
        .with_token("admin-token", TokenScope::Admin)
        .with_token("read-token", TokenScope::Read);

    let ws_server = WsServer::bind("127.0.0.1:0", mgr, evt_tx, tokens)
        .await
        .unwrap()
        .with_account_manager(acct);
    let local_addr = ws_server.local_addr().unwrap();
    tokio::spawn(async move { ws_server.run().await });

    // 1. Connect without any token in headers/query
    let ws_url = format!("ws://{local_addr}");
    let (mut ws, _) = connect_async(&ws_url).await.unwrap();

    // 2. Try to run command without authenticating
    let req = json!({
        "v": 1,
        "id": "test-1",
        "cmd": "daemon.status"
    });
    ws.send(Message::Text(serde_json::to_string(&req).unwrap().into()))
        .await
        .unwrap();

    let msg = ws.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(!resp.ok);
    assert_eq!(resp.error.as_ref().unwrap().code, "Unauthorized");

    // 3. Authenticate in-band with invalid token
    let auth_bad = json!({
        "v": 1,
        "id": "auth-bad",
        "cmd": "auth",
        "params": { "token": "invalid-tok" }
    });
    ws.send(Message::Text(
        serde_json::to_string(&auth_bad).unwrap().into(),
    ))
    .await
    .unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(!resp.ok);
    assert_eq!(resp.error.as_ref().unwrap().code, "Unauthorized");

    // 4. Authenticate in-band with valid token
    let auth_ok = json!({
        "v": 1,
        "id": "auth-ok",
        "cmd": "auth",
        "params": { "token": "read-token" }
    });
    ws.send(Message::Text(
        serde_json::to_string(&auth_ok).unwrap().into(),
    ))
    .await
    .unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(resp.ok);

    // Now command succeeds
    ws.send(Message::Text(serde_json::to_string(&req).unwrap().into()))
        .await
        .unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(resp.ok);
}

#[tokio::test]
async fn test_ws_header_auth_and_scope_enforcement() {
    let (mgr, acct, evt_tx, _dir) = setup_test_harness().await;

    let tokens = TokenRegistry::new()
        .with_token("admin-key", TokenScope::Admin)
        .with_token("read-key", TokenScope::Read);

    let ws_server = WsServer::bind("127.0.0.1:0", mgr, evt_tx, tokens)
        .await
        .unwrap()
        .with_account_manager(acct);
    let local_addr = ws_server.local_addr().unwrap();
    tokio::spawn(async move { ws_server.run().await });

    // Client 1: Read-only scope via Authorization header
    let ws_url = format!("ws://{local_addr}");
    let mut req = ws_url.clone().into_client_request().unwrap();
    req.headers_mut()
        .insert(AUTHORIZATION, "Bearer read-key".parse().unwrap());
    let (mut ws_read, _) = connect_async(req).await.unwrap();

    // Read command allowed
    let list_req = json!({ "v": 1, "id": "list-1", "cmd": "session.list" });
    ws_read
        .send(Message::Text(
            serde_json::to_string(&list_req).unwrap().into(),
        ))
        .await
        .unwrap();
    let msg = ws_read.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(resp.ok);

    // Mutating command rejected for Read scope
    let mutate_req = json!({
        "v": 1,
        "id": "create-1",
        "cmd": "session.create",
        "params": {
            "task_description": "Write scope test",
            "agent_type": "mock"
        }
    });
    ws_read
        .send(Message::Text(
            serde_json::to_string(&mutate_req).unwrap().into(),
        ))
        .await
        .unwrap();
    let msg = ws_read.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(!resp.ok);
    assert_eq!(resp.error.as_ref().unwrap().code, "PermissionDenied");

    // Client 2: Admin scope via query parameter ?token=admin-key
    let admin_url = format!("ws://{local_addr}/?token=admin-key");
    let (mut ws_admin, _) = connect_async(&admin_url).await.unwrap();

    // Mutating command allowed for Admin scope
    ws_admin
        .send(Message::Text(
            serde_json::to_string(&mutate_req).unwrap().into(),
        ))
        .await
        .unwrap();
    let msg = ws_admin.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(resp.ok);
    assert!(resp.result.as_ref().unwrap().get("session_id").is_some());
}

#[tokio::test]
async fn test_ws_subscription_filtering_and_catchup_replay() {
    let (mgr, acct, evt_tx, _dir) = setup_test_harness().await;

    let tokens = TokenRegistry::new().with_token("admin-key", TokenScope::Admin);
    let ws_server = WsServer::bind("127.0.0.1:0", mgr.clone(), evt_tx.clone(), tokens)
        .await
        .unwrap()
        .with_account_manager(acct);
    let local_addr = ws_server.local_addr().unwrap();
    tokio::spawn(async move { ws_server.run().await });

    // Create 2 sessions to populate historical events in the store
    let s1 = mgr
        .create_with_context("Task 1".into(), "mock".into(), None, None)
        .await
        .unwrap();
    let s2 = mgr
        .create_with_context("Task 2".into(), "mock".into(), None, None)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect WebSocket client with Admin token
    let ws_url = format!("ws://{local_addr}/?token=admin-key");
    let (mut ws, _) = connect_async(&ws_url).await.unwrap();

    // Subscribe with filter for session s1 only, replay from sequence 0
    let sub_req = json!({
        "v": 1,
        "id": "sub-1",
        "cmd": "events.subscribe",
        "params": {
            "session_id": s1.0,
            "since_seq": 0
        }
    });
    ws.send(Message::Text(
        serde_json::to_string(&sub_req).unwrap().into(),
    ))
    .await
    .unwrap();

    // First frame is subscribe command ack
    let ack_msg = ws.next().await.unwrap().unwrap();
    let ack: ApiResponse = serde_json::from_str(&ack_msg.to_string()).unwrap();
    assert!(ack.ok);
    assert_eq!(ack.id, "sub-1");

    // Catch-up replay should yield event(s) for session s1 only
    let replayed_msg = ws.next().await.unwrap().unwrap();
    let replayed_val: serde_json::Value = serde_json::from_str(&replayed_msg.to_string()).unwrap();
    assert_eq!(replayed_val["v"], 1);
    let event = &replayed_val["event"];
    assert_eq!(event["session_id"].as_str().unwrap(), s1.0);

    // Emit live event for s2 (should be ignored by filter) and for s1 (should be delivered)
    let _ = evt_tx.send(AgentEvent::new(
        EventKind::StateChanged,
        Some(s2.clone()),
        json!({ "from": "Idle", "to": "Starting" }),
        "system",
    ));
    let _ = evt_tx.send(AgentEvent::new(
        EventKind::StateChanged,
        Some(s1.clone()),
        json!({ "from": "Idle", "to": "Starting" }),
        "system",
    ));

    // Next frame received must be s1 event!
    let live_msg = ws.next().await.unwrap().unwrap();
    let live_val: serde_json::Value = serde_json::from_str(&live_msg.to_string()).unwrap();
    let live_event = &live_val["event"];
    assert_eq!(live_event["session_id"].as_str().unwrap(), s1.0);
}

#[tokio::test]
async fn test_version_mismatch_and_validation() {
    let (mgr, acct, evt_tx, _dir) = setup_test_harness().await;

    let tokens = TokenRegistry::new().with_token("admin-key", TokenScope::Admin);
    let ws_server = WsServer::bind("127.0.0.1:0", mgr, evt_tx, tokens)
        .await
        .unwrap()
        .with_account_manager(acct);
    let local_addr = ws_server.local_addr().unwrap();
    tokio::spawn(async move { ws_server.run().await });

    let ws_url = format!("ws://{local_addr}/?token=admin-key");
    let (mut ws, _) = connect_async(&ws_url).await.unwrap();

    // 1. Schema version 2 (unsupported)
    let bad_version_req = json!({
        "v": 2,
        "id": "bad-v",
        "cmd": "daemon.status"
    });
    ws.send(Message::Text(
        serde_json::to_string(&bad_version_req).unwrap().into(),
    ))
    .await
    .unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(!resp.ok);
    assert_eq!(resp.error.as_ref().unwrap().code, "VersionMismatch");

    // 2. Malformed JSON
    ws.send(Message::Text("{ malformed json".into()))
        .await
        .unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&msg.to_string()).unwrap();
    assert!(!resp.ok);
    assert_eq!(resp.error.as_ref().unwrap().code, "ValidationError");
}

#[tokio::test]
async fn test_ipc_subscription_filtering_and_catchup_replay() {
    let (mgr, acct, evt_tx, dir) = setup_test_harness().await;
    let sock_path = dir.path().join("test_control.sock");

    let server = IpcServer::bind(&sock_path, mgr.clone(), evt_tx.clone())
        .unwrap()
        .with_account_manager(acct);
    tokio::spawn(async move { server.run().await });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Create session to populate store
    let s1 = mgr
        .create_with_context("IPC Task".into(), "mock".into(), None, None)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect via Unix Domain Socket
    let stream = UnixStream::connect(&sock_path).await.unwrap();
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    // Send events.subscribe with since_seq: 0 and session_id filter
    let sub_req = json!({
        "v": 1,
        "id": "ipc-sub-1",
        "cmd": "events.subscribe",
        "params": {
            "session_id": s1.0,
            "since_seq": 0
        }
    });
    write_half
        .write_all(format!("{}\n", serde_json::to_string(&sub_req).unwrap()).as_bytes())
        .await
        .unwrap();

    // 1. First line is subscription acknowledgement
    let ack_line = lines.next_line().await.unwrap().unwrap();
    let ack: ApiResponse = serde_json::from_str(&ack_line).unwrap();
    assert!(ack.ok);
    assert_eq!(ack.id, "ipc-sub-1");

    // 2. Next line is historical replayed event
    let event_line = lines.next_line().await.unwrap().unwrap();
    let val: serde_json::Value = serde_json::from_str(&event_line).unwrap();
    assert_eq!(val["v"], 1);
    assert_eq!(val["event"]["session_id"].as_str().unwrap(), s1.0);
}

#[tokio::test]
async fn test_ws_ping_pong_and_unsubscribe() {
    let (mgr, acct, evt_tx, _dir) = setup_test_harness().await;

    let tokens = TokenRegistry::new().with_token("admin-key", TokenScope::Admin);
    let ws_server = WsServer::bind("127.0.0.1:0", mgr, evt_tx, tokens)
        .await
        .unwrap()
        .with_account_manager(acct);
    let local_addr = ws_server.local_addr().unwrap();
    tokio::spawn(async move { ws_server.run().await });

    let ws_url = format!("ws://{local_addr}/?token=admin-key");
    let (mut ws, _) = connect_async(&ws_url).await.unwrap();

    // 1. Send Ping, receive Pong
    ws.send(Message::Ping(vec![1, 2, 3, 4].into()))
        .await
        .unwrap();
    let resp_msg = ws.next().await.unwrap().unwrap();
    assert_eq!(resp_msg, Message::Pong(vec![1, 2, 3, 4].into()));

    // 2. Subscribe and then unsubscribe
    let sub_req = json!({
        "v": 1,
        "id": "sub-1",
        "cmd": "events.subscribe"
    });
    ws.send(Message::Text(
        serde_json::to_string(&sub_req).unwrap().into(),
    ))
    .await
    .unwrap();
    let ack_msg = ws.next().await.unwrap().unwrap();
    let ack: ApiResponse = serde_json::from_str(&ack_msg.to_string()).unwrap();
    assert!(ack.ok);

    let unsub_req = json!({
        "v": 1,
        "id": "unsub-1",
        "cmd": "events.unsubscribe"
    });
    ws.send(Message::Text(
        serde_json::to_string(&unsub_req).unwrap().into(),
    ))
    .await
    .unwrap();
    let ack_msg = ws.next().await.unwrap().unwrap();
    let ack: ApiResponse = serde_json::from_str(&ack_msg.to_string()).unwrap();
    assert!(ack.ok);
    assert_eq!(ack.id, "unsub-1");
}

#[tokio::test]
async fn test_ws_invalid_token_header_rejected() {
    let (mgr, acct, evt_tx, _dir) = setup_test_harness().await;

    let tokens = TokenRegistry::new().with_token("admin-key", TokenScope::Admin);
    let ws_server = WsServer::bind("127.0.0.1:0", mgr, evt_tx, tokens)
        .await
        .unwrap()
        .with_account_manager(acct);
    let local_addr = ws_server.local_addr().unwrap();
    tokio::spawn(async move { ws_server.run().await });

    let ws_url = format!("ws://{local_addr}/");
    let mut req = ws_url.into_client_request().unwrap();
    req.headers_mut().insert(
        AUTHORIZATION,
        "Bearer totally-invalid-token".parse().unwrap(),
    );

    // The handshake should fail because of invalid bearer token
    let connect_res = connect_async(req).await;
    assert!(connect_res.is_err());
}
