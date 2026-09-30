//! UI and State tests for Agent Control TUI.

use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;

use ac_core::types::{
    Account, AgentEvent, AgentSession, EventKind, Id, Interaction, InteractionState, Project,
    SessionState, WorkspacePolicy,
};
use ac_tui::{
    app::{AccountOption, App, Modal, StatusType, SwitchModalStep, Tab},
    ui,
};

fn create_sample_app() -> App {
    let mut app = App::new();

    let s1 = AgentSession::new(
        Id::from("01HXYZ00000000000000000001"),
        "Task alpha".into(),
        "claude-code".into(),
    );
    let mut s2 = AgentSession::new(
        Id::from("01HXYZ00000000000000000002"),
        "Task beta".into(),
        "generic-pty".into(),
    );
    s2.state = SessionState::WaitingForHuman;
    let mut s3 = AgentSession::new(
        Id::from("01HXYZ00000000000000000003"),
        "Task gamma".into(),
        "mock".into(),
    );
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
    assert_eq!(app.current_tab, Tab::Accounts);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Activity);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Agents);

    app.next_tab();
    assert_eq!(app.current_tab, Tab::Settings);

    // Wraps around to Dashboard
    app.next_tab();
    assert_eq!(app.current_tab, Tab::Dashboard);

    // Previous tab wraps around to Settings
    app.prev_tab();
    assert_eq!(app.current_tab, Tab::Settings);

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

    // On Accounts tab
    let a2 = Account::new(
        "Secondary Anthropic".into(),
        "anthropic".into(),
        vec!["claude-code".into()],
        "ref:anthropic_key_2".into(),
        3,
        vec!["staging".into()],
    );
    app.accounts.push(a2);
    app.set_tab(Tab::Accounts);
    assert_eq!(app.selected_account, 0);
    app.next_row();
    assert_eq!(app.selected_account, 1);
    app.next_row();
    assert_eq!(app.selected_account, 0);
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
    assert!(transcript
        .iter()
        .any(|line| line.contains("[STATE] idle -> working")));

    // Apply OutputChunk event
    let out_event = AgentEvent::new(
        EventKind::AgentOutputReceived,
        Some(sid.clone()),
        json!({ "text": "Compiling project crate..." }),
        "adapter",
    );
    app.apply_event(out_event);

    let transcript = app.session_transcripts.get(&sid.0).unwrap();
    assert!(transcript
        .iter()
        .any(|line| line.contains("Compiling project crate...")));

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
    assert!(content.contains("Agent Terminal"));
    app.close_session_detail();

    // 4. Accounts View
    app.set_tab(Tab::Accounts);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Primary Anthropic"));

    // 5. Activity View
    app.set_tab(Tab::Activity);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Activity Stream"));

    // 6. Settings View
    app.set_tab(Tab::Settings);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Settings"));

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
    assert_eq!(
        app.sessions[0].successor_id,
        Some(Id::from("01HXYZ00000000000000000004"))
    );
}

#[test]
fn test_btop_dashboard_and_sidebar_rendering() {
    let mut app = create_sample_app();
    app.daemon_connected = true;
    let backend = TestBackend::new(140, 45);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Dashboard View with Full-Width 6-Panel Grid (No Sidebar Menu)
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);

    assert!(content.contains("AGENT CONTROL v1.0.0"));
    assert!(!content.contains("▸ Menu"));
    assert!(content.contains("System Status"));
    assert!(content.contains("Recent Events"));
    assert!(content.contains("Details"));
    assert!(content.contains("Antigravity"));
    assert!(content.contains("Claude Code"));

    // 2. Agents View
    app.set_tab(Tab::Agents);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("AGENTS"));
    assert!(content.contains("Agent Specifications & Capabilities"));

    // 3. Settings View
    app.set_tab(Tab::Settings);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("Settings"));
    assert!(content.contains("General"));
    assert!(content.contains("Accounts"));
    assert!(content.contains("Projects & Workspaces"));
}

#[test]
fn test_session_detail_terminal_buffer_rendering_and_scrolling() {
    let mut app = create_sample_app();
    let backend = TestBackend::new(120, 35);
    let mut terminal = Terminal::new(backend).unwrap();

    let sid = app.sessions[0].id.clone();
    app.session_detail_id = Some(sid.clone());

    // 1. Feed raw PTY output with ANSI escapes, colors, carriage returns
    let pty_chunk = AgentEvent::new(
        EventKind::AgentOutputReceived,
        Some(sid.clone()),
        serde_json::json!({
            "text": "\x1b[?25l\x1b[1;32mCompiling ac-core\x1b[0m\n\x1b[33mRunning tests...\x1b[0m\r\x1b[1;32mAll 195 tests passed\x1b[0m\n"
        }),
        "adapter",
    );
    app.apply_event(pty_chunk);

    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);

    // Verify target layout elements
    assert!(content.contains("Agent Terminal"));
    assert!(content.contains("● LIVE"));
    assert!(content.contains("Agent Control"));
    assert!(content.contains("Ctrl+P"));
    assert!(content.contains("Ctrl+Q"));
    assert!(content.contains("All 195 tests passed"));
    assert!(content.contains("Compiling ac-core"));

    // Verify Session Info is available in SessionInfo modal
    app.active_modal = Some(Modal::SessionInfo {
        session_id: sid.clone(),
    });
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer_modal = terminal.backend().buffer().clone();
    let content_modal = format!("{:?}", buffer_modal);
    assert!(content_modal.contains("Session Info"));
    app.active_modal = None;

    // Verify escape codes never leaked into the buffer text
    assert!(!content.contains("\\u{1b}[?25l"));
    assert!(!content.contains("\\u{1b}[1;32m"));

    // 2. Test scrolling
    app.scroll_session_terminal_up(&sid.0, 1);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer_scrolled = terminal.backend().buffer().clone();
    let content_scrolled = format!("{:?}", buffer_scrolled);
    assert!(content_scrolled.contains("SCROLL MODE"));

    // 3. Test scrolling to bottom restores follow mode
    app.scroll_session_terminal_bottom(&sid.0);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer_restored = terminal.backend().buffer().clone();
    let content_restored = format!("{:?}", buffer_restored);
    assert!(content_restored.contains("● LIVE"));
}

#[test]
fn test_terminal_cursor_and_edge_to_edge_rendering() {
    let mut app = create_sample_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let sid = app.sessions[0].id.clone();
    app.session_detail_id = Some(sid.clone());

    let pty_chunk = AgentEvent::new(
        EventKind::AgentOutputReceived,
        Some(sid.clone()),
        serde_json::json!({
            "text": "agent> "
        }),
        "adapter",
    );
    app.apply_event(pty_chunk);

    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let cursor_pos = terminal.get_cursor_position().unwrap();

    // Verify cursor is set right at the end of "agent> " (column 7, line 1 because line 0 is compact header)
    assert_eq!(cursor_pos.x, 7);
    assert_eq!(cursor_pos.y, 1);
}

#[test]
fn test_remove_session_state_and_modal_rendering() {
    let mut app = create_sample_app();
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let sid = app.sessions[0].id.clone();
    assert_eq!(app.sessions.len(), 3);

    // 1. Open session detail and establish terminal buffer
    app.session_detail_id = Some(sid.clone());
    app.session_terminal_buffers
        .insert(sid.0.clone(), Default::default());

    // 2. Render ConfirmRemoveSession modal
    app.active_modal = Some(Modal::ConfirmRemoveSession {
        session_id: sid.clone(),
    });
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);

    assert!(content.contains("Confirm Remove Session"));
    assert!(content.contains("Permanently remove session"));
    assert!(content.contains("[y/Enter] Confirm Remove"));
    assert!(content.contains("[n/Esc] Cancel"));

    // 3. Test reactive removal via EventKind::SessionRemoved
    app.active_modal = None;
    let remove_evt = AgentEvent::new(
        EventKind::SessionRemoved,
        Some(sid.clone()),
        serde_json::json!({ "session_id": sid.0 }),
        "human",
    );
    app.apply_event(remove_evt);

    // Verify session was purged from sessions, buffers, and active detail
    assert_eq!(app.sessions.len(), 2);
    assert!(app.sessions.iter().all(|s| s.id != sid));
    assert!(!app.session_terminal_buffers.contains_key(&sid.0));
    assert_eq!(app.session_detail_id, None);
    assert!(app.selected_session < app.sessions.len());
}

#[test]
fn test_agy_session_rendering_flow_and_visual_polish() {
    let mut app = create_sample_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Open an AGY session
    let sid = Id::from("session-agy-001");
    let mut agy_session = AgentSession::new(sid.clone(), "interactive".into(), "agy".into());
    agy_session.state = SessionState::Working;
    app.sessions.push(agy_session);
    app.session_detail_id = Some(sid.clone());

    // Initial draw: verify minimal header with LIVE, WORKING, Antigravity, and Gemini model
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content = format!("{:?}", buffer);
    assert!(content.contains("● LIVE"));
    assert!(content.contains("WORKING"));
    assert!(content.contains("Agent Control • Antigravity"));
    assert!(content.contains("Gemini 3.8 Flash · medium"));
    assert!(content.contains("Ctrl+P"));
    assert!(content.contains("Ctrl+Q"));

    // 2. Type "say hi" prompt: AGY terminal prompt sequence
    let prompt_chunk = AgentEvent::new(
        EventKind::AgentOutputReceived,
        Some(sid.clone()),
        serde_json::json!({
            "text": "─────────────────────────────────────────────────────────────\n> say hi"
        }),
        "adapter",
    );
    app.apply_event(prompt_chunk);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let cursor_pos = terminal.get_cursor_position().unwrap();
    // Cursor should be right after "say hi" on line 2 (line 0 is header, line 1 is line rule, line 2 is prompt)
    assert_eq!(cursor_pos.x, 8); // "> say hi" is 8 characters
    assert_eq!(cursor_pos.y, 2);

    // 3. Generating state with cursor hidden (\x1b[?25l)
    let gen_chunk = AgentEvent::new(
        EventKind::AgentOutputReceived,
        Some(sid.clone()),
        serde_json::json!({
            "text": "\n─────────────────────────────────────────────────────────────\n\x1b[?25l⣾ Generating..."
        }),
        "adapter",
    );
    app.apply_event(gen_chunk);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let gen_buffer = terminal.backend().buffer().clone();
    let gen_content = format!("{:?}", gen_buffer);
    assert!(gen_content.contains("Generating..."));

    // 4. Observe response: AGY outputs text response that wraps
    let long_response =
        "Antigravity CLI is Google DeepMind's advanced coding assistant for native environments. "
            .repeat(3);
    let resp_chunk = AgentEvent::new(
        EventKind::AgentOutputReceived,
        Some(sid.clone()),
        serde_json::json!({
            "text": format!("\n{}\n", long_response)
        }),
        "adapter",
    );
    app.apply_event(resp_chunk);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let resp_buffer = terminal.backend().buffer().clone();
    let resp_content = format!("{:?}", resp_buffer);
    assert!(resp_content.contains("advanced coding assistant"));

    // 5. Enter another prompt and restore cursor (\x1b[?25h)
    let prompt2_chunk = AgentEvent::new(
        EventKind::AgentOutputReceived,
        Some(sid.clone()),
        serde_json::json!({
            "text": "─────────────────────────────────────────────────────────────\n> tell me a joke\x1b[?25h"
        }),
        "adapter",
    );
    app.apply_event(prompt2_chunk);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let p2_cursor = terminal.get_cursor_position().unwrap();
    assert!(p2_cursor.x > 0);

    // 6. Scroll through previous output
    app.scroll_session_terminal_up(&sid.0, 5);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let scroll_content = format!("{:?}", terminal.backend().buffer());
    assert!(
        scroll_content.contains("SCROLL: 5 lines up") || scroll_content.contains("SCROLL MODE")
    );

    // Snap back to live
    app.scroll_session_terminal_bottom(&sid.0);
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let live_content = format!("{:?}", terminal.backend().buffer());
    assert!(live_content.contains("● LIVE"));

    // 7. Resize the terminal window
    let mut resized_terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    resized_terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let resize_content = format!("{:?}", resized_terminal.backend().buffer());
    assert!(resize_content.contains("● LIVE"));
    assert!(resize_content.contains("Gemini 3.8 Flash · medium"));

    // 8. Return with Ctrl+Q
    app.close_session_detail();
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let dashboard_content = format!("{:?}", terminal.backend().buffer());
    assert!(!dashboard_content.contains("Gemini 3.8 Flash · medium"));

    // 9. Re-enter the session
    app.session_detail_id = Some(sid.clone());
    terminal.draw(|f| ui::draw(f, &app)).unwrap();
    let reentered_content = format!("{:?}", terminal.backend().buffer());
    assert!(reentered_content.contains("● LIVE"));
    assert!(reentered_content.contains("Gemini 3.8 Flash · medium"));
}
