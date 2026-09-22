//! Phase 4 integration tests — Real Agent Adapters (Claude Code & Generic PTY).
//!
//! Tests verify end-to-end integration:
//! Real Adapter -> Session Manager -> State Machine -> Interaction Hub -> Policy Engine.

use std::fs;
use std::time::Duration;

use ac_core::adapter::claude::{ClaudeAdapterFactory, ClaudeConfig};
use ac_core::adapter::pty::{GenericPtyAdapterFactory, PtyConfig, PtyPatternKind};
use ac_core::adapter::{AdapterFactory, CompositeAdapterFactory};
use ac_core::event_store::EventStore;
use ac_core::interaction_hub::{InteractionHub, InteractionHubHandle, InteractionStore};
use ac_core::policy_engine::{PolicyEngine, PolicyStore};
use ac_core::project_registry::{ProjectRegistry, ProjectStore};
use ac_core::session::manager::{SessionManager, SessionManagerHandle};
use ac_core::types::*;
use tempfile::tempdir;
use tokio::sync::{broadcast, mpsc};

struct TestHarness {
    _dir: tempfile::TempDir,
    manager_handle: SessionManagerHandle,
    hub_handle: InteractionHubHandle,
    event_rx: broadcast::Receiver<AgentEvent>,
    project_id: Option<Id>,
}

impl TestHarness {
    fn new(factory: Box<dyn AdapterFactory + Send>) -> Self {
        Self::new_with_project(factory, None)
    }

    fn new_with_project(
        factory: Box<dyn AdapterFactory + Send>,
        project: Option<Project>,
    ) -> Self {
        let dir = tempdir().expect("tempdir");
        let events_db = dir.path().join("events.db");
        let interactions_db = dir.path().join("interactions.db");
        let policies_db = dir.path().join("policies.db");
        let projects_db = dir.path().join("projects.db");

        let store = EventStore::open(&events_db).expect("event store");
        let p_store = PolicyStore::open(&policies_db).expect("policy store");
        let engine = PolicyEngine::new(p_store).expect("policy engine");
        let i_store = InteractionStore::open(&interactions_db).expect("interaction store");
        let hub = InteractionHub::new(i_store, engine).expect("interaction hub");
        let hub_handle = InteractionHubHandle::new(hub);

        let proj_store = ProjectStore::open(&projects_db).expect("project store");
        let mut proj_reg = ProjectRegistry::new(proj_store).expect("project reg");

        let mut project_id = None;
        if let Some(p) = project {
            project_id = Some(proj_reg.register(p).expect("register project"));
        }

        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (event_tx, event_rx) = broadcast::channel(128);

        let manager = SessionManager::new(
            store,
            cmd_rx,
            event_tx,
            3,
            factory,
        )
        .with_project_registry(proj_reg)
        .with_interaction_hub(hub_handle.clone());

        tokio::spawn(async move {
            manager.run().await;
        });

        Self {
            _dir: dir,
            manager_handle: SessionManagerHandle::new(cmd_tx),
            hub_handle,
            event_rx,
            project_id,
        }
    }
}

// Helper to wait for a specific event
async fn wait_for_event<F>(rx: &mut broadcast::Receiver<AgentEvent>, predicate: F) -> AgentEvent
where
    F: Fn(&AgentEvent) -> bool,
{
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            panic!("timed out waiting for event");
        }
        if let Ok(evt) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            if let Ok(e) = evt {
                if predicate(&e) {
                    return e;
                }
            }
        }
    }
}

// ── Claude Code Adapter Integration Tests ─────────────────────────────────────

#[tokio::test]
async fn test_claude_adapter_session_lifecycle_with_auto_approval() {
    // Claude script that requests approval for read_file (which policy auto-approves)
    let script = r#"
printf '{"type": "ready"}\n'
printf '{"type": "output", "text": "Scanning files..."}\n'
printf '{"type": "approval_requested", "tool_name": "read_file", "prompt": "Read config.toml"}\n'
# Read response from stdin
read response
printf '{"type": "completed", "summary": "Read config successfully"}\n'
"#;

    let claude_cfg = ClaudeConfig::custom("sh", vec!["-c".to_string(), script.to_string()]);
    let factory = Box::new(ClaudeAdapterFactory::new(claude_cfg));
    let mut harness = TestHarness::new(factory);

    // Register auto-approval policy for read_file
    {
        let mut hub = harness.hub_handle.0.lock().unwrap();
        hub.policy_engine_mut()
            .create_policy(Policy::new(
                "auto-read".into(),
                PolicyScope::Global,
                10,
                vec![PolicyCondition::ToolNameEquals("read_file".into())],
                PolicyDecision::Allow,
            ))
            .unwrap();
    }

    // Create and start session
    let sid = harness
        .manager_handle
        .create("test task".into(), "claude-code".into())
        .await
        .unwrap();

    harness.manager_handle.start(sid.clone()).await.unwrap();

    // Verify SessionReady
    let ready_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::SessionReady && e.session_id == Some(sid.clone())
    })
    .await;
    assert_eq!(ready_evt.kind, EventKind::SessionReady);

    // Verify ApprovalAutoApproved
    let auto_app_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::ApprovalAutoApproved && e.session_id == Some(sid.clone())
    })
    .await;
    assert_eq!(auto_app_evt.kind, EventKind::ApprovalAutoApproved);

    // Session remains in Working state (never pauses or waits for human)
    let session = harness.manager_handle.get(sid.clone()).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Working);

    // Verify completion
    let stop_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::SessionStopped && e.session_id == Some(sid.clone())
    })
    .await;
    assert_eq!(stop_evt.kind, EventKind::SessionStopped);
}

#[tokio::test]
async fn test_claude_adapter_human_approval_flow() {
    // Claude script requests approval for an unknown or dangerous tool
    let script = r#"
printf '{"type": "ready"}\n'
printf '{"type": "approval_requested", "tool_name": "rm", "prompt": "Delete build artifacts"}\n'
read response
printf '{"type": "completed", "summary": "Finished after human approval"}\n'
"#;

    let claude_cfg = ClaudeConfig::custom("sh", vec!["-c".to_string(), script.to_string()]);
    let factory = Box::new(ClaudeAdapterFactory::new(claude_cfg));
    let mut harness = TestHarness::new(factory);

    let sid = harness
        .manager_handle
        .create("delete artifacts".into(), "claude-code".into())
        .await
        .unwrap();

    harness.manager_handle.start(sid.clone()).await.unwrap();

    // Session transitions to WaitingForHuman due to hard never-auto-approve boundary for "rm"
    let wait_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id == Some(sid.clone())
            && e.payload.get("to") == Some(&serde_json::json!("waiting_for_human"))
    })
    .await;
    assert_eq!(wait_evt.kind, EventKind::StateChanged);

    // Query pending interactions
    let interaction_id = {
        let h = harness.hub_handle.0.lock().unwrap();
        let pending = h.list_pending(Some(&sid));
        assert_eq!(pending.len(), 1);
        pending[0].id.clone()
    };

    // Human approves interaction
    harness
        .manager_handle
        .respond_interaction(
            interaction_id,
            Some(PolicyDecision::Allow),
            Some("approved by human".into()),
            Some("human:alice".into()),
        )
        .await
        .unwrap();

    // Session transitions back to Working
    let resumed_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id == Some(sid.clone())
            && e.payload.get("to") == Some(&serde_json::json!("working"))
    })
    .await;
    assert_eq!(resumed_evt.kind, EventKind::StateChanged);

    // Session completes
    let stop_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::SessionStopped && e.session_id == Some(sid.clone())
    })
    .await;
    assert_eq!(stop_evt.kind, EventKind::SessionStopped);
}

#[tokio::test]
async fn test_claude_adapter_question_flow() {
    let script = r#"
printf '{"type": "ready"}\n'
printf '{"type": "question", "prompt": "Which branch should I target?"}\n'
read response
printf '{"type": "output", "text": "Targeting branch"}\n'
printf '{"type": "completed", "summary": "Done"}\n'
"#;

    let claude_cfg = ClaudeConfig::custom("sh", vec!["-c".to_string(), script.to_string()]);
    let factory = Box::new(ClaudeAdapterFactory::new(claude_cfg));
    let mut harness = TestHarness::new(factory);

    let sid = harness
        .manager_handle
        .create("branch task".into(), "claude-code".into())
        .await
        .unwrap();

    harness.manager_handle.start(sid.clone()).await.unwrap();

    // Wait for WaitingForHuman
    let wait_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id == Some(sid.clone())
            && e.payload.get("to") == Some(&serde_json::json!("waiting_for_human"))
    })
    .await;
    assert_eq!(wait_evt.kind, EventKind::StateChanged);

    // Human responds to question
    let iid = {
        let h = harness.hub_handle.0.lock().unwrap();
        let pending = h.list_pending(Some(&sid));
        assert_eq!(pending.len(), 1);
        pending[0].id.clone()
    };

    harness
        .manager_handle
        .respond_interaction(
            iid,
            Some(PolicyDecision::Allow),
            Some("main".into()),
            Some("human:alice".into()),
        )
        .await
        .unwrap();

    // Session resumes to Working
    let resumed_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id == Some(sid.clone())
            && e.payload.get("to") == Some(&serde_json::json!("working"))
    })
    .await;
    assert_eq!(resumed_evt.kind, EventKind::StateChanged);
}

#[tokio::test]
async fn test_claude_adapter_workspace_isolation() {
    let temp = tempdir().unwrap();
    let repo_dir = temp.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();

    let script = r#"
printf '{"type": "ready"}\n'
pwd
printf '{"type": "completed"}\n'
"#;

    let claude_cfg = ClaudeConfig::custom("sh", vec!["-c".to_string(), script.to_string()]);
    let factory = Box::new(ClaudeAdapterFactory::new(claude_cfg));

    // Register project
    let project = Project::new(
        "isolation-project".into(),
        repo_dir.to_str().unwrap().to_string(),
        Some("claude-code".into()),
        vec![],
        WorkspacePolicy::Shared,
    );

    let mut harness = TestHarness::new_with_project(factory, Some(project));
    let pid = harness.project_id.clone().unwrap();

    // Create session bound to project
    let sid = harness
        .manager_handle
        .create_with_context("test isolation".into(), "claude-code".into(), Some(pid), None)
        .await
        .unwrap();

    harness.manager_handle.start(sid.clone()).await.unwrap();

    // Verify OutputChunk contains workspace path
    let output_evt = wait_for_event(&mut harness.event_rx, |e| {
        if e.kind == EventKind::AgentOutputReceived && e.session_id == Some(sid.clone()) {
            if let Some(text) = e.payload.get("text").and_then(|v| v.as_str()) {
                return text.contains(repo_dir.file_name().unwrap().to_str().unwrap());
            }
        }
        false
    })
    .await;
    assert!(output_evt.payload["text"].as_str().unwrap().contains("repo"));
}

// ── Generic PTY Adapter Integration Tests ────────────────────────────────────

#[tokio::test]
async fn test_generic_pty_adapter_lifecycle_and_pattern_matching() {
    let script = r#"
printf 'System ready\n'
printf 'Do you want to run make?\n'
read answer
printf 'User answered: %s\n' "$answer"
printf 'Task finished\n'
"#;

    let pty_cfg = PtyConfig::new("sh")
        .with_args(["-c", script])
        .with_pattern("System ready", PtyPatternKind::Ready)
        .unwrap()
        .with_pattern(
            r"Do you want to run make\?",
            PtyPatternKind::ApprovalRequested {
                tool_name: Some("make".into()),
            },
        )
        .unwrap()
        .with_pattern("Task finished", PtyPatternKind::Completed { summary: Some("Done".into()) })
        .unwrap();

    let factory = Box::new(GenericPtyAdapterFactory::new(pty_cfg));
    let mut harness = TestHarness::new(factory);

    let sid = harness
        .manager_handle
        .create("pty test".into(), "generic-pty".into())
        .await
        .unwrap();

    harness.manager_handle.start(sid.clone()).await.unwrap();

    // Session transitions to WaitingForHuman
    let wait_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id == Some(sid.clone())
            && e.payload.get("to") == Some(&serde_json::json!("waiting_for_human"))
    })
    .await;
    assert_eq!(wait_evt.kind, EventKind::StateChanged);

    // Approve interaction
    let iid = {
        let h = harness.hub_handle.0.lock().unwrap();
        let pending = h.list_pending(Some(&sid));
        assert_eq!(pending.len(), 1);
        pending[0].id.clone()
    };

    harness
        .manager_handle
        .respond_interaction(
            iid,
            Some(PolicyDecision::Allow),
            Some("yes".into()),
            Some("human:bob".into()),
        )
        .await
        .unwrap();

    // Session resumes to Working
    let resumed_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::StateChanged
            && e.session_id == Some(sid.clone())
            && e.payload.get("to") == Some(&serde_json::json!("working"))
    })
    .await;
    assert_eq!(resumed_evt.kind, EventKind::StateChanged);

    // Session completes
    let stop_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::SessionStopped && e.session_id == Some(sid.clone())
    })
    .await;
    assert_eq!(stop_evt.kind, EventKind::SessionStopped);
}

#[tokio::test]
async fn test_generic_pty_adapter_crash_transitions_state_machine() {
    let pty_cfg = PtyConfig::new("sh").with_args(["-c", "exit 7"]);
    let factory = Box::new(GenericPtyAdapterFactory::new(pty_cfg));
    let mut harness = TestHarness::new(factory);

    let sid = harness
        .manager_handle
        .create("crash test".into(), "generic-pty".into())
        .await
        .unwrap();

    harness.manager_handle.start(sid.clone()).await.unwrap();

    // Verify SessionCrashed event
    let crash_evt = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::SessionCrashed && e.session_id == Some(sid.clone())
    })
    .await;
    assert_eq!(crash_evt.kind, EventKind::SessionCrashed);

    let session = harness.manager_handle.get(sid).await.unwrap().unwrap();
    assert_eq!(session.state, SessionState::Crashed);
}

// ── Composite Adapter Factory Routing ─────────────────────────────────────────

#[tokio::test]
async fn test_composite_adapter_factory_dispatch() {
    let claude_script = r#"
printf '{"type": "ready"}\n'
printf '{"type": "output", "text": "I am claude"}\n'
printf '{"type": "completed"}\n'
"#;
    let pty_script = r#"
echo "I am pty"
"#;

    let composite = CompositeAdapterFactory::new()
        .with_claude_config(ClaudeConfig::custom("sh", vec!["-c".to_string(), claude_script.to_string()]))
        .with_pty_config(PtyConfig::new("sh").with_args(["-c", pty_script]));

    let mut harness = TestHarness::new(Box::new(composite));

    // 1. Run Claude Code session
    let sid1 = harness
        .manager_handle
        .create("claude task".into(), "claude-code".into())
        .await
        .unwrap();
    harness.manager_handle.start(sid1.clone()).await.unwrap();

    let claude_out = wait_for_event(&mut harness.event_rx, |e| {
        e.kind == EventKind::AgentOutputReceived
            && e.session_id == Some(sid1.clone())
            && e.payload.get("text").and_then(|v| v.as_str()) == Some("I am claude")
    })
    .await;
    assert_eq!(claude_out.session_id, Some(sid1.clone()));

    // 2. Run Generic PTY session
    let sid2 = harness
        .manager_handle
        .create("pty task".into(), "generic-pty".into())
        .await
        .unwrap();
    harness.manager_handle.start(sid2.clone()).await.unwrap();

    let pty_out = wait_for_event(&mut harness.event_rx, |e| {
        if e.kind == EventKind::AgentOutputReceived && e.session_id == Some(sid2.clone()) {
            if let Some(text) = e.payload.get("text").and_then(|v| v.as_str()) {
                return text.contains("I am pty");
            }
        }
        false
    })
    .await;
    assert_eq!(pty_out.session_id, Some(sid2));
}
