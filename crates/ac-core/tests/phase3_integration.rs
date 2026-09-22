//! Phase 3 integration tests: Interaction Hub + Policy Engine + Session state machine integration.

use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

use ac_core::{
    adapter::mock::{MockAdapterFactory, MockScript, ScriptedEvent},
    event_store::EventStore,
    interaction_hub::{InteractionHub, InteractionHubHandle, InteractionStore},
    policy_engine::{PolicyEngine, PolicyStore},
    session::manager::{SessionManager, SessionManagerHandle},
    types::{
        AdapterEvent, AgentEvent, EventKind, InteractionKind, InteractionState, Policy,
        PolicyCondition, PolicyDecision, PolicyScope, SessionState,
    },
};

// ── Test helpers ──────────────────────────────────────────────────────────────

fn make_test_hub() -> InteractionHubHandle {
    let pol_store = PolicyStore::open_in_memory().unwrap();
    let pol_engine = PolicyEngine::new(pol_store).unwrap();
    let int_store = InteractionStore::open_in_memory().unwrap();
    let hub = InteractionHub::new(int_store, pol_engine).unwrap();
    InteractionHubHandle::new(hub)
}

fn make_manager_with_hub(
    event_store: EventStore,
    hub_handle: InteractionHubHandle,
    script: MockScript,
) -> (SessionManagerHandle, broadcast::Receiver<AgentEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (event_tx, event_rx) = broadcast::channel(256);
    let manager = SessionManager::new(
        event_store,
        cmd_rx,
        event_tx,
        3,
        Box::new(MockAdapterFactory::always(script)),
    )
    .with_interaction_hub(hub_handle);
    tokio::spawn(async move { manager.run().await });
    (SessionManagerHandle::new(cmd_tx), event_rx)
}

async fn wait_for_event<F>(rx: &mut broadcast::Receiver<AgentEvent>, pred: F) -> AgentEvent
where
    F: Fn(&AgentEvent) -> bool,
{
    loop {
        let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("Timed out waiting for event")
            .expect("Event channel closed");
        if pred(&event) {
            return event;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn approval_auto_approved_session_remains_working() {
    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    // Configure policy: allow "read_file"
    {
        let mut hub = hub_handle.0.lock().unwrap();
        hub.policy_engine_mut()
            .create_policy(Policy::new(
                "Allow reading files".into(),
                PolicyScope::Global,
                10,
                vec![PolicyCondition::ToolNameEquals("read_file".into())],
                PolicyDecision::Allow,
            ))
            .unwrap();
    }

    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::ApprovalRequested {
                tool_name: "read_file".into(),
                prompt: "Read README.md".into(),
            },
        ),
    ];

    let (mgr, mut rx) = make_manager_with_hub(store, hub_handle.clone(), script);
    let sid = mgr.create("task 1".into(), "mock".into()).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();

    // Wait for auto-approval event
    let event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::ApprovalAutoApproved && e.session_id.as_ref() == Some(&sid)
    })
    .await;

    assert_eq!(event.payload["decision"], "allow");
    assert_eq!(event.payload["tool_name"], "read_file");

    // Session remains in Working state uninterrupted
    let session = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);

    // Verify interaction in hub is auto_resolved
    let hub = hub_handle.0.lock().unwrap();
    let pending = hub.list_pending(Some(&sid));
    assert!(pending.is_empty(), "No interactions should be pending");

    let all = hub.list(Some(&sid), None);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].state, InteractionState::AutoResolved);
    assert_eq!(all[0].decision, Some(PolicyDecision::Allow));

    // Audit log has entry
    let audits = hub.policy_engine().query_audit(Some(&sid), None).unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "allow");
}

#[tokio::test]
async fn approval_denied_by_policy_session_remains_working() {
    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    // Configure policy: deny "deploy_prod"
    {
        let mut hub = hub_handle.0.lock().unwrap();
        hub.policy_engine_mut()
            .create_policy(Policy::new(
                "Deny prod deployment".into(),
                PolicyScope::Global,
                20,
                vec![PolicyCondition::ToolNameEquals("deploy_prod".into())],
                PolicyDecision::Deny,
            ))
            .unwrap();
    }

    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::ApprovalRequested {
                tool_name: "deploy_prod".into(),
                prompt: "Deploying to production".into(),
            },
        ),
    ];

    let (mgr, mut rx) = make_manager_with_hub(store, hub_handle.clone(), script);
    let sid = mgr.create("task deploy".into(), "mock".into()).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();

    let event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::ApprovalDenied && e.session_id.as_ref() == Some(&sid)
    })
    .await;

    assert_eq!(event.payload["decision"], "deny");

    // Session remains working
    let session = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);

    let hub = hub_handle.0.lock().unwrap();
    let all = hub.list(Some(&sid), None);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].state, InteractionState::AutoResolved);
    assert_eq!(all[0].decision, Some(PolicyDecision::Deny));
}

#[tokio::test]
async fn never_auto_approve_boundary_cannot_be_overridden_and_escalates() {
    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    // Attempt to configure an Allow policy for "sudo" (never-auto-approve boundary)
    {
        let mut hub = hub_handle.0.lock().unwrap();
        hub.policy_engine_mut()
            .create_policy(Policy::new(
                "Allow sudo".into(),
                PolicyScope::Global,
                100,
                vec![PolicyCondition::ToolNameEquals("sudo".into())],
                PolicyDecision::Allow,
            ))
            .unwrap();
    }

    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::ApprovalRequested {
                tool_name: "sudo".into(),
                prompt: "Execute privileged command".into(),
            },
        ),
    ];

    let (mgr, mut rx) = make_manager_with_hub(store, hub_handle.clone(), script);
    let sid = mgr.create("task sudo".into(), "mock".into()).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();

    // Should NOT auto-approve; should escalate and transition to WaitingForHuman
    let state_event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "waiting_for_human"
    })
    .await;
    assert_eq!(state_event.payload["from"], "working");

    let session = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::WaitingForHuman);

    let hub = hub_handle.0.lock().unwrap();
    let pending = hub.list_pending(Some(&sid));
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].tool_name.as_deref(), Some("sudo"));
}

#[tokio::test]
async fn human_approval_flow_resumes_session() {
    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::ApprovalRequested {
                tool_name: "write_file".into(),
                prompt: "Modify database schema".into(),
            },
        ),
    ];

    let (mgr, mut rx) = make_manager_with_hub(store, hub_handle.clone(), script);
    let sid = mgr.create("task write".into(), "mock".into()).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();

    // Session transitions Working -> WaitingForHuman
    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "waiting_for_human"
    })
    .await;

    let session = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::WaitingForHuman);

    // Get pending interaction
    let pending_id = {
        let hub = hub_handle.0.lock().unwrap();
        let pending = hub.list_pending(Some(&sid));
        assert_eq!(pending.len(), 1);
        pending[0].id.clone()
    };

    // Human operator approves the interaction
    let resolved = mgr
        .respond_interaction(
            pending_id.clone(),
            Some(PolicyDecision::Allow),
            Some("Approved by security team".into()),
            Some("admin".into()),
        )
        .await
        .unwrap();

    assert_eq!(resolved.state, InteractionState::HumanResolved);
    assert_eq!(resolved.decision, Some(PolicyDecision::Allow));

    // Session transitions WaitingForHuman -> Working
    let resume_event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "working"
    })
    .await;
    assert_eq!(resume_event.payload["from"], "waiting_for_human");

    let session_after = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session_after.state, SessionState::Working);

    // Audit log records human decision
    let hub = hub_handle.0.lock().unwrap();
    let audits = hub.policy_engine().query_audit(Some(&sid), None).unwrap();
    let human_audit = audits.iter().find(|a| a.actor == "admin").unwrap();
    assert_eq!(human_audit.action, "allow");
    assert_eq!(
        human_audit.rationale.as_deref(),
        Some("Approved by security team")
    );
}

#[tokio::test]
async fn human_question_flow_resumes_session() {
    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::QuestionRaised {
                prompt: "Which database port should I use?".into(),
            },
        ),
    ];

    let (mgr, mut rx) = make_manager_with_hub(store, hub_handle.clone(), script);
    let sid = mgr.create("task question".into(), "mock".into()).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();

    // Session transitions Working -> WaitingForHuman
    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "waiting_for_human"
    })
    .await;

    let pending_id = {
        let hub = hub_handle.0.lock().unwrap();
        let pending = hub.list_pending(Some(&sid));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, InteractionKind::Question);
        pending[0].id.clone()
    };

    // Human operator replies with the answer
    let resolved = mgr
        .respond_interaction(
            pending_id.clone(),
            None,
            Some("Use port 5432".into()),
            Some("developer".into()),
        )
        .await
        .unwrap();

    assert_eq!(resolved.state, InteractionState::HumanResolved);
    assert_eq!(resolved.response.as_deref(), Some("Use port 5432"));

    // Session resumes Working
    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "working"
    })
    .await;

    let session = mgr.get(sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);
}

#[tokio::test]
async fn dismiss_interaction_resumes_session() {
    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::QuestionRaised {
                prompt: "Non-critical clarification?".into(),
            },
        ),
    ];

    let (mgr, mut rx) = make_manager_with_hub(store, hub_handle.clone(), script);
    let sid = mgr.create("task dismiss".into(), "mock".into()).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();

    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "waiting_for_human"
    })
    .await;

    let pending_id = {
        let hub = hub_handle.0.lock().unwrap();
        hub.list_pending(Some(&sid))[0].id.clone()
    };

    let dismissed = mgr
        .dismiss_interaction(pending_id, Some("operator".into()))
        .await
        .unwrap();
    assert_eq!(dismissed.state, InteractionState::Dismissed);

    // Session resumes
    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "working"
    })
    .await;

    let session = mgr.get(sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);
}

#[tokio::test]
async fn stopping_session_expires_pending_interactions() {
    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::QuestionRaised {
                prompt: "Need confirmation".into(),
            },
        ),
    ];

    let (mgr, mut rx) = make_manager_with_hub(store, hub_handle.clone(), script);
    let sid = mgr.create("task stop expire".into(), "mock".into()).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();

    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "waiting_for_human"
    })
    .await;

    let pending_id = {
        let hub = hub_handle.0.lock().unwrap();
        hub.list_pending(Some(&sid))[0].id.clone()
    };

    // Stop session while interaction is still pending
    mgr.stop(sid.clone(), Some("User cancelled".into())).await.unwrap();

    // Verify interaction expired event emitted
    let expired_event = wait_for_event(&mut rx, |e| {
        e.kind == EventKind::InteractionExpired && e.session_id.as_ref() == Some(&sid)
    })
    .await;
    assert_eq!(expired_event.payload["interaction_id"], pending_id.0);

    let hub = hub_handle.0.lock().unwrap();
    let interaction = hub.get(&pending_id).unwrap();
    assert_eq!(interaction.state, InteractionState::Expired);
}

#[tokio::test]
async fn multiple_pending_interactions_session_stays_waiting_until_all_resolved() {
    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    let script = vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::QuestionRaised {
                prompt: "Question 1".into(),
            },
        ),
        ScriptedEvent::delayed(
            40,
            AdapterEvent::QuestionRaised {
                prompt: "Question 2".into(),
            },
        ),
    ];

    let (mgr, mut rx) = make_manager_with_hub(store, hub_handle.clone(), script);
    let sid = mgr.create("task multi".into(), "mock".into()).await.unwrap();
    mgr.start(sid.clone()).await.unwrap();

    // Wait until waiting_for_human
    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "waiting_for_human"
    })
    .await;

    // Give adapter time to emit question 2
    tokio::time::sleep(Duration::from_millis(60)).await;

    let (id1, id2) = {
        let hub = hub_handle.0.lock().unwrap();
        let pending = hub.list_pending(Some(&sid));
        assert_eq!(pending.len(), 2);
        (pending[0].id.clone(), pending[1].id.clone())
    };

    // Resolve first question
    mgr.respond_interaction(id1, None, Some("answer 1".into()), None)
        .await
        .unwrap();

    // Session should STILL be WaitingForHuman because question 2 is pending
    let session_mid = mgr.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session_mid.state, SessionState::WaitingForHuman);

    // Resolve second question
    mgr.respond_interaction(id2, None, Some("answer 2".into()), None)
        .await
        .unwrap();

    // Now session resumes Working
    wait_for_event(&mut rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id.as_ref() == Some(&sid)
            && e.payload["to"] == "working"
    })
    .await;

    let session_final = mgr.get(sid).await.unwrap().unwrap();
    assert_eq!(session_final.state, SessionState::Working);
}

// ── IPC Socket Integration Tests ──────────────────────────────────────────────

#[tokio::test]
async fn ipc_policy_and_interaction_roundtrip() {
    use ac_core::ipc::IpcServer;
    use ac_core::types::{ApiRequest, ApiResponse};
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("test_phase3.sock");

    let store = EventStore::open_in_memory().unwrap();
    let hub_handle = make_test_hub();

    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (event_tx, _event_rx) = broadcast::channel(256);
    let manager = SessionManager::new(
        store,
        cmd_rx,
        event_tx.clone(),
        3,
        Box::new(MockAdapterFactory::always(vec![
            ScriptedEvent::immediate(AdapterEvent::Ready),
            ScriptedEvent::delayed(
                20,
                AdapterEvent::QuestionRaised {
                    prompt: "What is your username?".into(),
                },
            ),
        ])),
    )
    .with_interaction_hub(hub_handle.clone());
    tokio::spawn(async move { manager.run().await });

    let mgr_handle = SessionManagerHandle::new(cmd_tx);

    let server = IpcServer::bind(&socket_path, mgr_handle, event_tx)
        .unwrap()
        .with_interaction_hub(hub_handle);
    tokio::spawn(async move { server.run().await });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Connect client
    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).lines();

    // 1. Create a policy rule via IPC
    let req = ApiRequest {
        v: 1,
        id: "p1".into(),
        cmd: "policy.upsert".into(),
        params: json!({
            "name": "Allow tests",
            "scope": "global",
            "priority": 5,
            "decision": "allow",
            "conditions": [{ "tool_name_equals": "run_test" }],
            "enabled": true
        }),
    };
    write_half
        .write_all(format!("{}\n", serde_json::to_string(&req).unwrap()).as_bytes())
        .await
        .unwrap();

    let resp_line = reader.next_line().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&resp_line).unwrap();
    assert!(resp.ok);
    let pol_id = resp.result.unwrap()["policy_id"].as_str().unwrap().to_owned();

    // 2. Test policy evaluation via IPC
    let req_test = ApiRequest {
        v: 1,
        id: "p2".into(),
        cmd: "policy.test".into(),
        params: json!({
            "tool_name": "run_test"
        }),
    };
    write_half
        .write_all(format!("{}\n", serde_json::to_string(&req_test).unwrap()).as_bytes())
        .await
        .unwrap();

    let resp_line = reader.next_line().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&resp_line).unwrap();
    assert!(resp.ok);
    assert_eq!(resp.result.unwrap()["decision"], "allow");

    // 3. Create and start a session via IPC
    let req_run = ApiRequest {
        v: 1,
        id: "s1".into(),
        cmd: "session.create_and_start".into(),
        params: json!({
            "task_description": "IPC task",
            "agent_type": "mock"
        }),
    };
    write_half
        .write_all(format!("{}\n", serde_json::to_string(&req_run).unwrap()).as_bytes())
        .await
        .unwrap();

    let resp_line = reader.next_line().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&resp_line).unwrap();
    assert!(resp.ok);
    let session_id = resp.result.unwrap()["session_id"].as_str().unwrap().to_owned();

    // Wait for the question to be raised and session to transition to waiting_for_human
    tokio::time::sleep(Duration::from_millis(80)).await;

    // 4. List pending interactions via IPC
    let req_pending = ApiRequest {
        v: 1,
        id: "i1".into(),
        cmd: "interaction.list_pending".into(),
        params: json!({ "session_id": session_id }),
    };
    write_half
        .write_all(format!("{}\n", serde_json::to_string(&req_pending).unwrap()).as_bytes())
        .await
        .unwrap();

    let resp_line = reader.next_line().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&resp_line).unwrap();
    assert!(resp.ok);
    let interactions = resp.result.unwrap()["interactions"].as_array().unwrap().clone();
    assert_eq!(interactions.len(), 1);
    let interaction_id = interactions[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(interactions[0]["prompt"], "What is your username?");

    // 5. Reply to interaction via IPC
    let req_reply = ApiRequest {
        v: 1,
        id: "i2".into(),
        cmd: "interaction.reply".into(),
        params: json!({
            "interaction_id": interaction_id,
            "response": "alice"
        }),
    };
    write_half
        .write_all(format!("{}\n", serde_json::to_string(&req_reply).unwrap()).as_bytes())
        .await
        .unwrap();

    let resp_line = reader.next_line().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&resp_line).unwrap();
    assert!(resp.ok);
    assert_eq!(resp.result.unwrap()["state"], "human_resolved");

    // 6. Query audit log via IPC
    let req_audit = ApiRequest {
        v: 1,
        id: "a1".into(),
        cmd: "audit.list".into(),
        params: json!({ "session_id": session_id }),
    };
    write_half
        .write_all(format!("{}\n", serde_json::to_string(&req_audit).unwrap()).as_bytes())
        .await
        .unwrap();

    let resp_line = reader.next_line().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&resp_line).unwrap();
    assert!(resp.ok);
    let audit_entries = resp.result.unwrap()["audit_entries"].as_array().unwrap().clone();
    assert!(!audit_entries.is_empty(), "Audit log should have entries");

    // 7. Delete the policy via IPC
    let req_del = ApiRequest {
        v: 1,
        id: "p3".into(),
        cmd: "policy.remove".into(),
        params: json!({ "policy_id": pol_id }),
    };
    write_half
        .write_all(format!("{}\n", serde_json::to_string(&req_del).unwrap()).as_bytes())
        .await
        .unwrap();

    let resp_line = reader.next_line().await.unwrap().unwrap();
    let resp: ApiResponse = serde_json::from_str(&resp_line).unwrap();
    assert!(resp.ok);
}
