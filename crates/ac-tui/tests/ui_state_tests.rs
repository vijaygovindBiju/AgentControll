//! UI and State tests for Agent Control TUI.

use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;

use ac_core::types::{
    Account, AgentEvent, AgentSession, EventKind, Id, Interaction,
    InteractionState, Project, SessionState, WorkspacePolicy,
};
use ac_tui::{
    app::{AccountOption, App, Modal, StatusType, SwitchModalStep, Tab},
    ui,
};

fn create_sample_app() -> App {
    let mut app = App::new();

    let s1 = AgentSession::new(Id::from("01HXYZ00000000000000000001"), "Task alpha".into(), "claude-code".into());
    let mut s2 = AgentSession::new(Id::from("01HXYZ00000000000000000002"), "Task beta".into(), "generic-pty".into());
    s2.state = SessionState::WaitingForHuman;
    let mut s3 = AgentSession::new(Id::from("01HXYZ00000000000000000003"), "Task gamma".into(), "mock".into());
    s3.state = SessionState::Stopped;

    app.sessions = vec![s1, s2, s3];

    let i1 = Interaction::new_approval_request(
        Id::from("01HXYZ00000000000000000002"),
        "bash".into(),
        Some(json!({ "cmd": "git pull" })),
        "Allow executing git pull?".into(),
    );
    let i2 = Interaction::new_question(
        Id::from("01HXYZ00000000000000000002"),
        "Which database migration should I apply?".into(),
    );
    app.interactions = vec![i1, i2];

    let mut a1 = Account::new(
        "Primary Anthropic".into(),
        "anthropic".into(),
        vec!["claude-code".into()],
        "ref:anthropic_key".into(),
        3,
        vec!["production".into()],
    );
    a1.id = Id::from("01HXYZ000000000000000000A1");
    a1.active_session_count = 1;
    app.accounts = vec![a1];

    let mut p1 = Project::new(
        "agent-control".into(),
        "/workspace/agent-control".into(),
        Some("claude-code".into()),
        vec!["production".into()],
        WorkspacePolicy::Shared,
    );
    p1.id = Id::from("01HXYZ000000000000000000P1");
    app.projects = vec![p1];

    let e1 = AgentEvent::new(EventKind::DaemonStarted, None, json!({}), "system");
    let e2 = AgentEvent::new(
        EventKind::StateChanged,
        Some(Id::from("01HXYZ00000000000000000001")),
        json!({ "from": "idle", "to": "working" }),
        "human",
    );
    app.events = vec![e2, e1];

    app
}

#[test]
fn test_navigation_and_tab_switching() {
    let mut app = create_sample_app();
    assert_eq!(app.current_tab, Tab::Dashboard);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Sessions);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Inbox);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Accounts);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Projects);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Activity);

    // Wraps around to Dashboard
    app.next_tab();
    assert_eq!(app.current_tab, Tab::Dashboard);

    // Previous tab wraps around to Activity
    app.prev_tab();
    assert_eq!(app.current_tab, Tab::Activity);

    // Direct tab selection
    app.set_tab(Tab::Sessions);
    assert_eq!(app.current_tab, Tab::Sessions);
    assert_eq!(app.session_detail_id, None);
}

#[test]
fn test_selection_and_row_navigation() {
    let mut app = create_sample_app();

    // On Sessions tab
    app.set_tab(Tab::Sessions);
    assert_eq!(app.selected_session, 0);

    app.next_row();
    assert_eq!(app.selected_session, 1);

    app.next_row();
    assert_eq!(app.selected_session, 2);

    // Wraps around
    app.next_row();
    assert_eq!(app.selected_session, 0);

    // Previous row wraps
    app.prev_row();
    assert_eq!(app.selected_session, 2);

    // On Inbox tab
    app.set_tab(Tab::Inbox);
    assert_eq!(app.selected_interaction, 0);
    app.next_row();
    assert_eq!(app.selected_interaction, 1);
    app.next_row();
    assert_eq!(app.selected_interaction, 0);
}

#[test]
fn test_session_detail_open_and_close() {
    let mut app = create_sample_app();
    app.set_tab(Tab::Sessions);
    app.selected_session = 1;

    app.open_selected_session_detail();
    assert_eq!(
        app.session_detail_id,
        Some(Id::from("01HXYZ00000000000000000002"))
    );

    let current = app.selected_session_or_detail().unwrap();
    assert_eq!(current.id.0, "01HXYZ00000000000000000002");
    assert_eq!(current.state, SessionState::WaitingForHuman);

    app.close_session_detail();
    assert_eq!(app.session_detail_id, None);
}

#[test]
fn test_state_updates_from_events() {
    let mut app = create_sample_app();
    let sid = Id::from("01HXYZ00000000000000000001");

    // Initially s1 is Idle
    assert_eq!(app.sessions[0].state, SessionState::Idle);

    // Apply StateChanged event
    let event = AgentEvent::new(
        EventKind::StateChanged,
        Some(sid.clone()),
        json!({ "from": "idle", "to": "working" }),
        "system",
    );
    app.apply_event(event);

    // Session state should now be Working in-place
    assert_eq!(app.sessions[0].state, SessionState::Working);

    // Transcript should have recorded the state change
    let transcript = app.session_transcripts.get(&sid.0).unwrap();
    assert!(transcript.iter().any(|line| line.contains("[STATE] idle -> working")));

    // Apply OutputChunk event
    let out_event = AgentEvent::new(
        EventKind::AgentOutputReceived,
        Some(sid.clone()),
        json!({ "text": "Compiling project crate..." }),
        "adapter",
    );
    app.apply_event(out_event);

    let transcript = app.session_transcripts.get(&sid.0).unwrap();
    assert!(transcript.iter().any(|line| line.contains("Compiling project crate...")));

    // Apply SessionStopped event
    let stop_event = AgentEvent::new(
        EventKind::SessionStopped,
        Some(sid.clone()),
        json!({}),
        "human",
    );
    app.apply_event(stop_event);
    assert_eq!(app.sessions[0].state, SessionState::Stopped);
}

#[test]
fn test_interaction_state_updates_from_events() {
    let mut app = create_sample_app();
    let iid = app.interactions[0].id.clone();
    assert_eq!(app.interactions[0].state, InteractionState::Pending);

    // Apply InteractionHumanResolved
    let event = AgentEvent::new(
        EventKind::InteractionHumanResolved,
        Some(app.interactions[0].session_id.clone()),
        json!({ "interaction_id": iid.0 }),
        "human",
    );
    app.apply_event(event);

    assert_eq!(app.interactions[0].state, InteractionState::HumanResolved);
    // Pending count should now be 1 instead of 2
    assert_eq!(app.pending_interactions().len(), 1);
}

#[test]
fn test_status_messages_and_expiry() {
    let mut app = create_sample_app();
    assert!(app.status_message.is_none());

    app.set_status("Operation succeeded", StatusType::Success);
    assert!(app.status_message.is_some());
    let (msg, st, _) = app.status_message.as_ref().unwrap();
    assert_eq!(msg, "Operation succeeded");
    assert_eq!(*st, StatusType::Success);
}

#[test]
fn test_render_all_views_headless() {
    let mut app = create_sample_app();
    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Dashboard View
    app.set_tab(Tab::Dashboard);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("AGENT CONTROL"));
    assert!(content.contains("Total Sessions"));

    // 2. Sessions View
    app.set_tab(Tab::Sessions);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Task alpha"));
    assert!(content.contains("claude-code"));

    // 3. Session Detail View
    app.open_selected_session_detail();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Live Transcript & Agent Output"));
    app.close_session_detail();

    // 4. Inbox View
    app.set_tab(Tab::Inbox);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Pending Interactions"));
    assert!(content.contains("APPROVAL"));

    // 5. Accounts View
    app.set_tab(Tab::Accounts);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Primary Anthropic"));

    // 6. Projects View
    app.set_tab(Tab::Projects);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("agent-control"));

    // 7. Activity View
    app.set_tab(Tab::Activity);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Activity Stream"));

    // 8. Modals (Steer, Reply, ConfirmStop, Help)
    app.active_modal = Some(Modal::Steer {
        session_id: Id::from("01HXYZ00000000000000000001"),
        input: "Focus on unit tests first".into(),
    });
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Steer Session"));
    assert!(content.contains("Focus on unit tests first"));

    app.active_modal = Some(Modal::Help);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Help & Shortcuts"));
}

#[test]
fn test_switch_account_modal_progression_and_render() {
    let mut app = create_sample_app();
    let backend = TestBackend::new(120, 40);
    let mut terminal = Terminal::new(backend).unwrap();

    let acct_opt = AccountOption {
        account_id: Id::from("01HXYZ000000000000000000A2"),
        label: "Secondary Claude Account".into(),
        provider: "anthropic".into(),
        usable: true,
        active_sessions: 0,
        concurrency_cap: 2,
        reason: None,
    };

    // Step 1: Render SelectAccount step in modal
    app.active_modal = Some(Modal::SwitchAccount {
        session_id: Id::from("01HXYZ00000000000000000001"),
        agent_type: "claude-code".into(),
        current_account_id: Some(Id::from("01HXYZ000000000000000000A1")),
        current_account_label: "Primary Anthropic".into(),
        options: vec![acct_opt.clone()],
        selected_index: 0,
        step: SwitchModalStep::SelectAccount,
    });
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Switch Account"));
    assert!(content.contains("Secondary Claude Account"));

    // Step 2: Render ConfirmRestart step in modal
    app.active_modal = Some(Modal::SwitchAccount {
        session_id: Id::from("01HXYZ00000000000000000001"),
        agent_type: "claude-code".into(),
        current_account_id: Some(Id::from("01HXYZ000000000000000000A1")),
        current_account_label: "Primary Anthropic".into(),
        options: vec![acct_opt.clone()],
        selected_index: 0,
        step: SwitchModalStep::ConfirmRestart {
            target_account_id: acct_opt.account_id.clone(),
            target_account_label: acct_opt.label.clone(),
            reason: "Provider requires controlled process restart".into(),
        },
    });
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Confirm Account Switch"));
    assert!(content.contains("Workspace will be preserved"));
    assert!(content.contains("Confirm Hand-off"));

    // Event handling test: SessionHandedOff updates predecessor session state in app
    let ev = AgentEvent::new(
        EventKind::SessionHandedOff,
        Some(Id::from("01HXYZ00000000000000000001")),
        json!({
            "predecessor_id": "01HXYZ00000000000000000001",
            "successor_id": "01HXYZ00000000000000000004",
            "target_account_id": "01HXYZ000000000000000000A2",
        }),
        "system",
    );
    app.apply_event(ev);
    assert_eq!(app.sessions[0].state, SessionState::HandedOff);
    assert_eq!(app.sessions[0].successor_id, Some(Id::from("01HXYZ00000000000000000004")));
}
