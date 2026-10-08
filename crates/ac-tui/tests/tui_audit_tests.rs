//! Comprehensive TUI reliability and UX regression test suite.
//!
//! Tests covers:
//! 1. Bracketed paste mode parsing and querying on virtual terminal buffer
//! 2. Terminal paste routing to active modal text fields
//! 3. Hotkey immunity when pasting text containing command letters (q, r, d, n, s, etc.)
//! 4. Multiline, large, and multi-byte Unicode paste handling
//! 5. Headless rendering under extreme viewport dimensions (0x0, 1x1, 10x4, 80x24, 300x100)
//! 6. Multi-byte Unicode display-width safety (preventing byte-slicing panics)
//! 7. Selection boundary clamping, empty collection safety, and page navigation (PageUp/Down/Home/End)
//! 8. Modal focus management, Ctrl+C dismissal, and Esc dismissal

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;

use ac_core::types::{
    Account, AgentEvent, AgentSession, EventKind, Id, Interaction, Project, SessionState,
    WorkspacePolicy,
};
use ac_tui::{
    app::{App, Modal, Tab},
    client::ApiClient,
    event,
    terminal_buffer::TerminalBuffer,
    ui,
    views::truncate_display_width,
};

fn create_test_client() -> ApiClient {
    ApiClient::new(PathBuf::from("/tmp/nonexistent-ac-test.sock"))
}

fn sample_app_with_unicode() -> App {
    let mut app = App::new();

    let s1 = AgentSession::new(
        Id::from("01HXYZ00000000000000000001"),
        "🚀 Antigravity 測試 任務: αβγδε 🦀".into(),
        "claude-code".into(),
    );
    let mut s2 = AgentSession::new(
        Id::from("01HXYZ00000000000000000002"),
        "日本語タスク説明文 — Очень длинная строка для проверки переноса".into(),
        "generic-pty".into(),
    );
    s2.state = SessionState::WaitingForHuman;
    app.sessions = vec![s1, s2];

    let i1 = Interaction::new_question(
        Id::from("01HXYZ00000000000000000002"),
        "❓ Approve deployment to 🌍 production cluster? [y/N]".into(),
    );
    app.interactions = vec![i1];

    let mut a1 = Account::new(
        "✨ Pro Google 帳號 ✨".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:test_cred".into(),
        2,
        vec!["ai".into()],
    );
    a1.id = Id::from("01HXYZ000000000000000000A1");
    app.accounts = vec![a1];

    let mut p1 = Project::new(
        "📁 Проект_Alpha_🎉".into(),
        "/home/user/workspace/プロジェクト".into(),
        Some("claude-code".into()),
        vec!["production".into()],
        WorkspacePolicy::Shared,
    );
    p1.id = Id::from("01HXYZ000000000000000000P1");
    app.projects = vec![p1];

    let e1 = AgentEvent::new(
        EventKind::StateChanged,
        Some(Id::from("01HXYZ00000000000000000001")),
        json!({ "details": "Started session 🚀 with UTF-8 data: こんにちは世界" }),
        "system",
    );
    app.events = vec![e1];

    app
}

#[test]
fn test_terminal_buffer_bracketed_paste_mode() {
    let mut buf = TerminalBuffer::new(500);
    assert!(!buf.bracketed_paste_enabled());

    // Send DEC private mode 2004 set: \x1b[?2004h
    buf.push_str("\x1b[?2004h");
    assert!(
        buf.bracketed_paste_enabled(),
        "Bracketed paste should be enabled after \\x1b[?2004h"
    );

    // Send DEC private mode 2004 reset: \x1b[?2004l
    buf.push_str("\x1b[?2004l");
    assert!(
        !buf.bracketed_paste_enabled(),
        "Bracketed paste should be disabled after \\x1b[?2004l"
    );

    // Verify clear resets bracketed paste
    buf.push_str("\x1b[?2004h");
    assert!(buf.bracketed_paste_enabled());
    buf.clear();
    assert!(!buf.bracketed_paste_enabled());
}

#[tokio::test]
async fn test_paste_into_modals_routes_to_input_fields() {
    let mut app = App::new();
    let client = create_test_client();

    // 1. Steer Modal
    app.active_modal = Some(Modal::Steer {
        session_id: Id::from("sess-1"),
        input: "initial ".into(),
    });
    event::handle_paste(&mut app, &client, "pasted prompt --flag=1")
        .await
        .unwrap();
    match &app.active_modal {
        Some(Modal::Steer { input, .. }) => {
            assert_eq!(input, "initial pasted prompt --flag=1");
        }
        other => panic!("Expected Steer modal, got {:?}", other),
    }

    // 2. Reply Modal
    app.active_modal = Some(Modal::Reply {
        interaction_id: Id::from("intr-1"),
        input: String::new(),
    });
    event::handle_paste(&mut app, &client, "yes, proceed with changes")
        .await
        .unwrap();
    match &app.active_modal {
        Some(Modal::Reply { input, .. }) => {
            assert_eq!(input, "yes, proceed with changes");
        }
        other => panic!("Expected Reply modal, got {:?}", other),
    }

    // 3. FilterActivity Modal
    app.active_modal = Some(Modal::FilterActivity {
        input: "kind:".into(),
    });
    event::handle_paste(&mut app, &client, "state_change")
        .await
        .unwrap();
    match &app.active_modal {
        Some(Modal::FilterActivity { input }) => {
            assert_eq!(input, "kind:state_change");
        }
        other => panic!("Expected FilterActivity modal, got {:?}", other),
    }

    // 4. SetDefaultWorkingDir Modal
    app.active_modal = Some(Modal::SetDefaultWorkingDir {
        input: String::new(),
        error: None,
    });
    event::handle_paste(&mut app, &client, "~/projects/work-dir/\n")
        .await
        .unwrap();
    match &app.active_modal {
        Some(Modal::SetDefaultWorkingDir { input, .. }) => {
            assert_eq!(input, "~/projects/work-dir/");
        }
        other => panic!("Expected SetDefaultWorkingDir modal, got {:?}", other),
    }

    // 5. NewSession Modal
    app.active_modal = Some(Modal::NewSession {
        account_index: 0,
        session_name: "test-".into(),
        active_field: 1,
    });
    event::handle_paste(&mut app, &client, "special-task")
        .await
        .unwrap();
    match &app.active_modal {
        Some(Modal::NewSession { session_name, .. }) => {
            assert_eq!(session_name, "test-special-task");
        }
        other => panic!("Expected NewSession modal, got {:?}", other),
    }

    // 6. AddAccount Modal (token field)
    app.active_modal = Some(Modal::AddAccount {
        label: "MyAccount".into(),
        provider: "claude".into(),
        auth_method: 1,
        token: String::new(),
        active_field: 3,
    });
    event::handle_paste(&mut app, &client, "sk-ant-api03-secret-token-value")
        .await
        .unwrap();
    match &app.active_modal {
        Some(Modal::AddAccount { token, .. }) => {
            assert_eq!(token, "sk-ant-api03-secret-token-value");
        }
        other => panic!("Expected AddAccount modal, got {:?}", other),
    }
}

#[tokio::test]
async fn test_paste_hotkey_immunity_on_main_tabs() {
    let mut app = sample_app_with_unicode();
    let client = create_test_client();

    for tab in Tab::ALL {
        app.set_tab(tab);
        app.active_modal = None;
        app.should_quit = false;
        let initial_sessions_count = app.sessions.len();

        // Paste string containing dangerous shortcut characters:
        // 'q' (quit), 'r' (refresh), 'd' (delete), 's' (stop/steer), 'n' (new session), '1'..'6' (tab switch)
        let hostile_paste = "quick red delete stop new 123456";
        event::handle_paste(&mut app, &client, hostile_paste)
            .await
            .unwrap();

        // Assert no side-effects happened
        assert!(
            !app.should_quit,
            "Pasting 'q' on tab {:?} must not set should_quit",
            tab
        );
        assert!(
            app.active_modal.is_none(),
            "Pasting commands on tab {:?} must not open a modal",
            tab
        );
        assert_eq!(
            app.current_tab, tab,
            "Pasting digits on tab {:?} must not switch tabs",
            tab
        );
        assert_eq!(
            app.sessions.len(),
            initial_sessions_count,
            "Pasting 'd' must not remove sessions"
        );
    }
}

#[tokio::test]
async fn test_multiline_large_and_unicode_paste() {
    let mut app = App::new();
    let client = create_test_client();

    app.active_modal = Some(Modal::Steer {
        session_id: Id::from("sess-1"),
        input: String::new(),
    });

    let large_multiline_unicode = "🌟 Line 1: 日本語・中国語・한국어\n\
        Line 2: 🚀 Unicode Symbols & Accents: café, naïve, résumé\n\
        Line 3: Code block: `fn main() { println!(\"Hello\"); }`\n"
        .repeat(20);

    event::handle_paste(&mut app, &client, &large_multiline_unicode)
        .await
        .unwrap();

    match &app.active_modal {
        Some(Modal::Steer { input, .. }) => {
            assert_eq!(input, &large_multiline_unicode);
            assert!(input.len() > 1000);
        }
        other => panic!("Expected Steer modal, got {:?}", other),
    }
}

#[test]
fn test_unicode_display_width_truncation_helper() {
    // Normal ASCII
    assert_eq!(truncate_display_width("hello world", 5), "he...");
    assert_eq!(truncate_display_width("hello", 10), "hello");
    assert_eq!(truncate_display_width("hello", 2), "he"); // max_width <= 3 truncates without ellipsis

    // Multi-byte Chinese/Japanese (each wide character is 2 columns)
    let cjk = "日本語テスト";
    // 6 chars * 2 width = 12 columns. Truncating to 7: (7 - 3 = 4 cols -> "日本") + "..." = "日本..."
    assert_eq!(truncate_display_width(cjk, 7), "日本...");
    // Truncating to 3 (<= 3): can fit 1 CJK char (2 cols)
    assert_eq!(truncate_display_width(cjk, 3), "日");

    // Emoji with variable widths (🚀 is 2 cols, 🦀 is 2 cols, 🎉 is 2 cols = total 6 cols)
    let emoji = "🚀🦀🎉";
    // 6 cols fits in 7 cols without truncation:
    assert_eq!(truncate_display_width(emoji, 7), "🚀🦀🎉");
    // Truncating to 5: (5 - 3 = 2 cols -> "🚀") + "..." = "🚀..."
    assert_eq!(truncate_display_width(emoji, 5), "🚀...");

    // Accented European chars (1 column width, 2 bytes UTF-8)
    let accented = "café crème";
    assert_eq!(truncate_display_width(accented, 7), "café...");
}

#[test]
fn test_headless_rendering_at_extreme_dimensions() {
    let app = sample_app_with_unicode();

    let test_sizes = [
        (0, 0),
        (1, 1),
        (5, 2),
        (10, 3),
        (20, 5),
        (40, 10),
        (80, 24),
        (120, 35),
        (250, 80),
        (400, 120),
    ];

    for &(width, height) in &test_sizes {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();

        // Render every tab with both empty app and populated app
        let empty_app = App::new();
        for &tab in &Tab::ALL {
            let mut test_app = app.clone();
            test_app.set_tab(tab);

            // Populated app draw
            let draw_res = terminal.draw(|f| ui::draw(f, &test_app));
            assert!(
                draw_res.is_ok(),
                "Failed drawing populated tab {:?} at {}x{}",
                tab,
                width,
                height
            );

            // Empty app draw
            let mut empty = empty_app.clone();
            empty.set_tab(tab);
            let empty_draw_res = terminal.draw(|f| ui::draw(f, &empty));
            assert!(
                empty_draw_res.is_ok(),
                "Failed drawing empty tab {:?} at {}x{}",
                tab,
                width,
                height
            );
        }

        // Render modals at extreme dimensions
        let modals = [
            Modal::Help,
            Modal::FilterActivity {
                input: "search text".into(),
            },
            Modal::Steer {
                session_id: Id::from("sess-1"),
                input: "steer query".into(),
            },
            Modal::Reply {
                interaction_id: Id::from("intr-1"),
                input: "reply text".into(),
            },
            Modal::ConfirmStop {
                session_id: Id::from("sess-1"),
            },
            Modal::ConfirmRemoveSession {
                session_id: Id::from("sess-1"),
            },
            Modal::SetDefaultWorkingDir {
                input: "/test/dir".into(),
                error: Some("Error message that is quite long to test wrapping".into()),
            },
            Modal::CommandPalette {
                session_id: Id::from("sess-1"),
                selected_index: 0,
            },
            Modal::NewSession {
                account_index: 0,
                session_name: "test-sess".into(),
                active_field: 0,
            },
            Modal::UrlPicker {
                session_id: Id::from("sess-1"),
                urls: vec![
                    "https://github.com/agentcontrol".into(),
                    "https://docs.agentcontrol.dev".into(),
                ],
                selected_index: 0,
            },
        ];

        for modal in modals {
            let mut modal_app = app.clone();
            modal_app.active_modal = Some(modal);
            let draw_modal_res = terminal.draw(|f| ui::draw(f, &modal_app));
            assert!(
                draw_modal_res.is_ok(),
                "Failed drawing modal at {}x{}",
                width,
                height
            );
        }
    }
}

#[test]
fn test_selection_clamping_and_page_navigation() {
    let mut app = sample_app_with_unicode();
    assert_eq!(app.sessions.len(), 2);

    // Navigate next / prev
    app.set_tab(Tab::Sessions);
    app.selected_session = 0;
    app.next_row();
    assert_eq!(app.selected_session, 1);
    app.next_row();
    assert_eq!(app.selected_session, 0); // cycles
    app.prev_row();
    assert_eq!(app.selected_session, 1);

    // Page down / up
    app.page_down(5);
    assert_eq!(app.selected_session, 1); // clamped to max
    app.page_up(5);
    assert_eq!(app.selected_session, 0); // clamped to 0

    // First / Last row
    app.last_row();
    assert_eq!(app.selected_session, 1);
    app.first_row();
    assert_eq!(app.selected_session, 0);

    // Clamping when collections are emptied
    app.sessions.clear();
    app.accounts.clear();
    app.projects.clear();
    app.events.clear();
    app.interactions.clear();

    app.selected_session = 10;
    app.selected_account = 5;
    app.selected_project = 3;
    app.selected_event = 8;
    app.selected_interaction = 4;

    app.clamp_selections();

    assert_eq!(app.selected_session, 0);
    assert_eq!(app.selected_account, 0);
    assert_eq!(app.selected_project, 0);
    assert_eq!(app.selected_event, 0);
    assert_eq!(app.selected_interaction, 0);

    // Calling next_row / prev_row on empty collections must not panic
    app.next_row();
    assert_eq!(app.selected_session, 0);
    app.prev_row();
    assert_eq!(app.selected_session, 0);
}

#[tokio::test]
async fn test_modal_ctrl_c_and_esc_dismissal() {
    let mut app = App::new();
    let client = create_test_client();

    // 1. Esc closes modal
    app.active_modal = Some(Modal::Help);
    let esc_key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    event::handle_key(&mut app, &client, esc_key).await.unwrap();
    assert!(app.active_modal.is_none());

    // 2. Ctrl+C closes modal
    app.active_modal = Some(Modal::Steer {
        session_id: Id::from("sess-1"),
        input: "some input".into(),
    });
    let ctrl_c_key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    event::handle_key(&mut app, &client, ctrl_c_key)
        .await
        .unwrap();
    assert!(app.active_modal.is_none());
    assert!(
        !app.should_quit,
        "Ctrl+C on a modal should dismiss the modal, not exit the app"
    );

    // 3. Ctrl+C when no modal is open quits the app
    let ctrl_c_global = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    event::handle_key(&mut app, &client, ctrl_c_global)
        .await
        .unwrap();
    assert!(app.should_quit, "Ctrl+C globally should set should_quit");
}

#[tokio::test]
async fn test_modal_steer_prompt_history_navigation() {
    let mut app = App::new();
    let client = create_test_client();
    let session_id = Id::from("sess-hist-1");

    // 1. Submit prompt A in Modal::Steer
    app.active_modal = Some(Modal::Steer {
        session_id: session_id.clone(),
        input: "tell me about the project".into(),
    });
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert!(app.active_modal.is_none());

    // 2. Submit prompt B in Modal::Steer
    app.active_modal = Some(Modal::Steer {
        session_id: session_id.clone(),
        input: "explain the architecture".into(),
    });
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    // 3. Open Modal::Steer with a draft
    app.active_modal = Some(Modal::Steer {
        session_id: session_id.clone(),
        input: "draft prompt".into(),
    });

    // Press Up -> previous prompt (prompt B)
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    if let Some(Modal::Steer { ref input, .. }) = app.active_modal {
        assert_eq!(input, "explain the architecture");
    } else {
        panic!("Modal::Steer should remain open");
    }

    // Press Up again -> older prompt (prompt A)
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    if let Some(Modal::Steer { ref input, .. }) = app.active_modal {
        assert_eq!(input, "tell me about the project");
    }

    // Press Up at oldest boundary -> stays at prompt A
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    if let Some(Modal::Steer { ref input, .. }) = app.active_modal {
        assert_eq!(input, "tell me about the project");
    }

    // Press Down -> forward to prompt B
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    if let Some(Modal::Steer { ref input, .. }) = app.active_modal {
        assert_eq!(input, "explain the architecture");
    }

    // Press Down again -> restores the original draft!
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    if let Some(Modal::Steer { ref input, .. }) = app.active_modal {
        assert_eq!(input, "draft prompt");
    }

    // Press Down at newest boundary -> stays at draft
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    if let Some(Modal::Steer { ref input, .. }) = app.active_modal {
        assert_eq!(input, "draft prompt");
    }
}

#[tokio::test]
async fn test_session_detail_prompt_history_lifecycle() {
    let mut app = App::new();
    let client = create_test_client();
    let sid = Id::from("sess-pty-1");
    app.session_detail_id = Some(sid.clone());

    // 1. Type and submit prompt A: "cargo check"
    for c in "cargo check".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo check");
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "");

    // 2. Type and submit prompt B: "cargo test"
    for c in "cargo test".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "");

    // 3. Type a draft: "carg"
    for c in "carg".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session_prompt_buffer(&sid.0), "carg");

    // Press Up -> loads prompt B ("cargo test")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");

    // Press Up -> loads prompt A ("cargo check")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo check");

    // Press Up at oldest boundary -> stays at "cargo check"
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo check");

    // Press Down -> forward to "cargo test"
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");

    // Press Down past newest entry -> restores draft "carg"
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "carg");

    // Press Down at newest boundary -> stays at "carg"
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "carg");

    // Verify PgUp and PgDn still work for transcript scrolling and Up does not scroll
    let mut term_buf = ac_tui::terminal_buffer::TerminalBuffer::default();
    for i in 0..50 {
        term_buf.push_str(&format!("log line {}\r\n", i));
    }
    app.session_terminal_buffers.insert(sid.0.clone(), term_buf);

    // PgUp scrolls up into scrollback mode
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert!(!app.session_terminal_buffers.get(&sid.0).unwrap().follow);

    // Up while in session detail snaps back to live bottom and navigates prompt history
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert!(app.session_terminal_buffers.get(&sid.0).unwrap().follow);
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");
}

#[tokio::test]
async fn test_prompt_history_exact_requirement_verification() {
    let mut app = App::new();
    let client = create_test_client();
    let sid = Id::from("sess-verify-14");
    app.session_detail_id = Some(sid.clone());

    // 1. Submit prompt A: "tell me about the project"
    for c in "tell me about the project".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    // 2. Submit prompt B: "explain the architecture"
    for c in "explain the architecture".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    // 3. Press Up -> B ("explain the architecture")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "explain the architecture"
    );

    // 4. Press Up -> A ("tell me about the project")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "tell me about the project"
    );

    // 5. Press Down -> B ("explain the architecture")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "explain the architecture"
    );

    // 6. Press Down -> original empty draft ("")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "");

    // 7. Type a draft, navigate Up, then Down -> original draft restored
    for c in "draft partial".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session_prompt_buffer(&sid.0), "draft partial");

    // Navigate Up -> B
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "explain the architecture"
    );

    // Navigate Down -> original draft restored
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "draft partial");

    // 8. PgUp/PgDn still scroll transcript
    let mut term_buf = ac_tui::terminal_buffer::TerminalBuffer::default();
    for i in 0..100 {
        term_buf.push_str(&format!("output line {}\r\n", i));
    }
    app.session_terminal_buffers.insert(sid.0.clone(), term_buf);

    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert!(
        !app.session_terminal_buffers.get(&sid.0).unwrap().follow,
        "PgUp should engage scroll mode"
    );

    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    // 9. Up/Down no longer scroll transcript while input is focused
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert!(
        app.session_terminal_buffers.get(&sid.0).unwrap().follow,
        "Up while focused must not scroll transcript"
    );
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "explain the architecture"
    );

    // 10. Terminal resize still works
    app.resize_session_terminals(40, 120);
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "explain the architecture"
    );

    // 11. Paste still works
    event::handle_paste(&mut app, &client, " pasted snippet")
        .await
        .unwrap();
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "explain the architecture pasted snippet"
    );
}

#[test]
fn test_backspace_removes_stale_characters_completely() {
    let mut buf = TerminalBuffer::new(500);

    // 1. Initial state: user types "hello world"
    buf.push_str("hello world");
    assert_eq!(buf.lines[0].to_plain_string(), "hello world");
    assert_eq!(buf.cursor_col, 11);

    // 2. Backspace 6 times down to "hello" (testing both \x08 and \x08 \x08 echoes)
    // Characters: 'd', 'l', 'r', 'o', 'w', ' '
    buf.push_str("\x08 \x08"); // 'd'
    buf.push_str("\x08 \x08"); // 'l'
    buf.push_str("\x08 \x08"); // 'r'
    buf.push_str("\x08 \x08"); // 'o'
    buf.push_str("\x08 \x08"); // 'w'
    buf.push_str("\x08"); // ' ' (bare backspace without space)
    assert_eq!(buf.lines[0].to_plain_string(), "hello");
    assert_eq!(buf.cursor_col, 5);

    // Visible lines wrapped must show exactly "hello" with no trailing spaces or " world"
    let (vis, _, _) = buf.get_visible_lines_wrapped(5, 80);
    assert_eq!(vis[0].spans[0].content, "hello");
    assert_eq!(vis[0].spans.len(), 1);

    // 3. Backspace 3 more times down to "he" ('o', 'l', 'l')
    buf.push_str("\x08 \x08"); // 'o'
    buf.push_str("\x08 \x08"); // 'l'
    buf.push_str("\x08 \x08"); // 'l'
    assert_eq!(buf.lines[0].to_plain_string(), "he");
    assert_eq!(buf.cursor_col, 2);

    let (vis, _, _) = buf.get_visible_lines_wrapped(5, 80);
    assert_eq!(vis[0].spans[0].content, "he");
    assert_eq!(vis[0].spans.len(), 1);

    // 4. Backspace 2 more times down to <empty> ('e', 'h')
    buf.push_str("\x08 \x08"); // 'e'
    buf.push_str("\x08 \x08"); // 'h'
    assert_eq!(buf.lines[0].to_plain_string(), "");
    assert_eq!(buf.cursor_col, 0);

    let (vis, _, _) = buf.get_visible_lines_wrapped(5, 80);
    assert_eq!(vis[0], ratatui::text::Line::from(""));
}

#[test]
fn test_backspace_h_colon_prompt_disappears_completely() {
    let mut buf = TerminalBuffer::new(500);

    // User types "h : query"
    buf.push_str("h : query");
    assert_eq!(buf.lines[0].to_plain_string(), "h : query");

    // Backspace all 9 characters (mix of \x08, \x7f, and \x08 \x08)
    for _ in 0..5 {
        buf.push_str("\x08 \x08"); // "query"
    }
    buf.push_str("\x08"); // ' '
    buf.push_str("\x08 \x08"); // ':'
    buf.push_str("\x08"); // ' '
    buf.push_str("\x7f"); // 'h' (DEL byte)

    assert_eq!(buf.lines[0].to_plain_string(), "");
    assert_eq!(buf.cursor_col, 0);

    let (vis, _, _) = buf.get_visible_lines_wrapped(5, 80);
    assert_eq!(vis[0], ratatui::text::Line::from(""));
}

#[test]
fn test_backspace_from_middle_preserves_tail() {
    let mut buf = TerminalBuffer::new(500);

    buf.push_str("hello world");
    // Move cursor left by 6 (to between "hello" and " world")
    buf.push_str("\x1b[6D");
    assert_eq!(buf.cursor_col, 5);

    // Delete 'o' from middle using DCH (\x1b[P)
    buf.push_str("\x1b[1D\x1b[1P");
    assert_eq!(buf.lines[0].to_plain_string(), "hell world");
}

#[tokio::test]
async fn test_prompt_history_replace_longer_with_shorter_in_session_detail() {
    let mut app = App::new();
    let sid = Id::from("01SESSION_HIST_TEST");
    app.session_detail_id = Some(sid.clone());

    let client = create_test_client();

    // 1. Submit prompt A: "explain the architecture"
    for c in "explain the architecture".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "explain the architecture"
    );
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "");

    // 2. Submit prompt B: "cargo test"
    for c in "cargo test".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "");

    // 3. User types long draft: "tell me about the project"
    for c in "tell me about the project".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    // Verify actual input state
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "tell me about the project"
    );

    // 4. Press Up -> restores prompt B: "cargo test"
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    // Actual input state must be "cargo test"
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");

    // 5. Press Up -> restores prompt A: "explain the architecture"
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "explain the architecture"
    );

    // 6. Press Down -> forward to prompt B: "cargo test" (shorter than A)
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");

    // 7. Press Down -> restores original draft: "tell me about the project"
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(
        app.session_prompt_buffer(&sid.0),
        "tell me about the project"
    );

    // 8. Backspace draft until empty
    for _ in 0..25 {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    // Verify actual input state is empty
    assert_eq!(app.session_prompt_buffer(&sid.0), "");

    // 9. Now verify terminal buffer handling when replacing a longer prompt with shorter:
    let mut tb = TerminalBuffer::new(500);
    tb.push_str("> tell me about the project");
    assert_eq!(tb.lines[0].to_plain_string(), "> tell me about the project");

    // Simulate PTY receiving 25 backspaces and "cargo test":
    for _ in 0..25 {
        tb.push_str("\x08 \x08");
    }
    tb.push_str("cargo test");
    // Displayed buffer must contain "cargo test" with NO leftover "project" or "about"
    assert_eq!(tb.lines[0].to_plain_string(), "> cargo test");

    // Backspace "cargo test" down to "hello world", then "hello", then "he", then empty
    tb.lines[0].clear();
    tb.cursor_col = 0;
    tb.push_str("hello world");
    assert_eq!(tb.lines[0].to_plain_string(), "hello world");

    // Backspace 6 times -> "hello"
    for _ in 0..6 {
        tb.push_str("\x08 \x08");
    }
    assert_eq!(tb.lines[0].to_plain_string(), "hello");

    // Backspace 3 times -> "he"
    for _ in 0..3 {
        tb.push_str("\x08 \x08");
    }
    assert_eq!(tb.lines[0].to_plain_string(), "he");

    // Backspace 2 times -> empty
    for _ in 0..2 {
        tb.push_str("\x08 \x08");
    }
    assert_eq!(tb.lines[0].to_plain_string(), "");
}

#[test]
fn test_terminal_resize_with_trimmed_lines_leaves_no_phantom_lines() {
    let mut buf = TerminalBuffer::new(500);

    // Type a prompt and backspace 21 times
    buf.push_str("cargo build --workspace --all-targets");
    for _ in 0..21 {
        buf.push_str("\x08 \x08");
    }
    assert_eq!(buf.lines[0].to_plain_string(), "cargo build --wo");

    // Normal width (80)
    let (vis80, _, _) = buf.get_visible_lines_wrapped(5, 80);
    assert_eq!(vis80[0].spans[0].content, "cargo build --wo");
    assert_eq!(vis80[1], ratatui::text::Line::from(""));

    // Narrow width (10): wraps into exactly 2 lines
    let (vis10, _, _) = buf.get_visible_lines_wrapped(5, 10);
    assert_eq!(vis10[0].spans[0].content, "cargo buil");
    assert_eq!(vis10[1].spans[0].content, "d --wo");
    assert_eq!(vis10[2], ratatui::text::Line::from(""));
}

#[test]
fn test_terminal_buffer_differential_slash_completion_linefeed_preserves_columns() {
    let mut buf = TerminalBuffer::new(500);
    buf.resize(20, 80);

    // 1. Initial prompt line and completion menu when user types '/'
    buf.push_str("> /\r\n───\r\n> /add-dir           Add a directory to the workspace\r\n  /agents            List available custom agents\r\n  /model             Set a model\r\n  ↑/↓ Navigate · enter Select · tab Complete\r\n\x1b[90mesc to cancel\x1b[m\r\x1b[6A\x1b[3C");
    assert_eq!(buf.cursor_row, 0);
    assert_eq!(buf.cursor_col, 3);
    assert!(buf.is_completion_open());

    // 2. User types 'm': AGY appends 'm' on line 0 (cursor now at col 4),
    // then sends \n\n\x08 to move to line 2, col 3, and updates candidate to 'mcp'
    buf.push_str("m\n\n\x08\x1b[94mmcp\x1b[15X\x1b[m\r\x1b[2A\x1b[4C");
    assert_eq!(buf.lines[0].to_plain_string(), "> /m");
    assert!(buf.lines[2].to_plain_string().starts_with("> /mcp"));
    assert!(buf.is_completion_open());

    // 3. User types 'o': AGY appends 'o' on line 0 (cursor now at col 5),
    // then sends \n\n\x08 to move to line 2, col 4, and updates candidate with 'odel' (starting at col 4)
    buf.push_str("o\n\n\x08\x1b[94model             \x1b[m  Set a model, or run a single prompt\r\x1b[2A\x1b[5C");
    assert_eq!(buf.lines[0].to_plain_string(), "> /mo");
    // Line 2 MUST be "> /model", NOT "odel" or "/odel"!
    assert!(buf.lines[2].to_plain_string().starts_with("> /model"));
    assert!(!buf.lines[2].to_plain_string().starts_with("/odel"));
    assert!(!buf.lines[2].to_plain_string().starts_with("odel"));
    assert!(buf.is_completion_open());

    // 4. User types 'd':
    buf.push_str("d\r\x1b[2A\x1b[6C");
    assert_eq!(buf.lines[0].to_plain_string(), "> /mod");

    // 5. User cancels with Esc: AGY clears menu with \x1b[J from line 1
    buf.push_str("\r\n\x1b[J\r\x1b[1A\x1b[6C");
    assert_eq!(buf.lines[0].to_plain_string(), "> /mod");
    assert!(!buf.is_completion_open());
}

#[tokio::test]
async fn test_event_routing_slash_completion_navigation_vs_prompt_history() {
    let mut app = App::new();
    let sid = Id::from("01SESSION_COMPLETION_TEST");
    let sess = ac_core::types::AgentSession::new(sid.clone(), "test-session".into(), "agy".into());
    app.sessions.push(sess);
    app.session_detail_id = Some(sid.clone());

    let client = create_test_client();

    // 1. Submit prompt A: "cargo check"
    for c in "cargo check".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    // 2. Submit prompt B: "cargo test"
    for c in "cargo test".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    // 3. User types "/mod"
    for c in "/mod".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mod");

    // Simulate AGY opening completion popup in terminal buffer
    let mut tb = TerminalBuffer::new(500);
    tb.push_str("> /mod\r\n───\r\n> /model\r\n  ↑/↓ Navigate · enter Select · tab Complete\r\nesc to cancel\r\n");
    app.session_terminal_buffers.insert(sid.0.clone(), tb);
    assert!(app.is_session_completion_open(&sid.0));

    // 1. /mod: User types "/mod", completion popup appears
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mod");
    assert!(app.is_session_completion_open(&sid.0));

    // 2. Up/Down completion navigation: while completion menu is OPEN,
    // Up and Down navigate completion candidates and MUST NOT touch prompt history!
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mod");

    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mod");

    // 3. Tab completion: completes and closes completion menu
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert!(!app.is_session_completion_open(&sid.0));

    // 4. Esc cancellation: close completion without modifying input unexpectedly
    app.set_session_completion_dismissed(&sid.0, false);
    assert!(app.is_session_completion_open(&sid.0));
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert!(!app.is_session_completion_open(&sid.0));
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mod");

    // 5. Backspace while completion is open
    app.set_session_completion_dismissed(&sid.0, false);
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mo");

    // 6. Typing more characters
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    )
    .await
    .unwrap();
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mode");

    // 7. Deleting all characters
    for _ in 0..5 {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session_prompt_buffer(&sid.0), "");

    // 8. Reopen completion
    for c in "/mod".chars() {
        event::handle_key(
            &mut app,
            &client,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mod");
    assert!(app.is_session_completion_open(&sid.0));

    // Close completion with Esc before testing prompt history
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert!(!app.is_session_completion_open(&sid.0));
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mod");

    // 9. Normal prompt history after completion closes:
    // Up loads newest prompt B ("cargo test")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");

    // Up loads older prompt A ("cargo check")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo check");

    // Down moves forward to prompt B ("cargo test")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "cargo test");

    // Down restores original draft ("/mod")
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "/mod");

    // 10. Paste while completion is open/closed
    // 10a. Paste while completion is closed:
    event::handle_paste(&mut app, &client, "el").await.unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "/model");

    // 10b. Paste while completion is open:
    app.set_session_completion_dismissed(&sid.0, false);
    assert!(app.is_session_completion_open(&sid.0));
    event::handle_paste(&mut app, &client, " --help")
        .await
        .unwrap();
    assert_eq!(app.session_prompt_buffer(&sid.0), "/model --help");
}

#[tokio::test]
async fn test_mouse_cursor_selection_in_terminal() {
    let mut app = App::new();
    let sid = Id::new();
    let sess = ac_core::types::AgentSession::new(sid.clone(), "test-session".into(), "agy".into());
    app.sessions.push(sess);
    app.session_detail_id = Some(sid.clone());

    let mut buf = TerminalBuffer::new(100);
    buf.push_str("Selected terminal text with cursor\r\nSecond line\r\n");
    app.session_terminal_buffers.insert(sid.0.clone(), buf);

    let client = ApiClient::new("http://127.0.0.1:0".to_string());

    // Mouse Left Down at column 0, row 1 (terminal area begins at row 1)
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 0,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();

    let tb = app.session_terminal_buffers.get(&sid.0).unwrap();
    assert!(tb.is_selecting());
    assert!(!tb.follow);

    // Mouse Left Drag to column 7, row 1 ("Selected")
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
            column: 7,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();

    let tb = app.session_terminal_buffers.get(&sid.0).unwrap();
    assert_eq!(tb.extract_selected_text(), Some("Selected".to_string()));

    // Mouse Left Up copies and retains selection
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
            column: 7,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();

    let tb = app.session_terminal_buffers.get(&sid.0).unwrap();
    assert!(tb.is_selecting());

    // Esc cancels selection
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    let tb = app.session_terminal_buffers.get(&sid.0).unwrap();
    assert!(!tb.is_selecting());

    // Double-click at column 12, row 1 (word "terminal")
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 12,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
            column: 12,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();
    // Second click within 400ms at same position
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 12,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
            column: 12,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();

    let tb = app.session_terminal_buffers.get(&sid.0).unwrap();
    assert!(tb.is_selecting());
    assert_eq!(tb.extract_selected_text(), Some("terminal".to_string()));
}

#[tokio::test]
async fn test_mouse_cursor_autoscroll_at_top_boundary() {
    let mut app = App::new();
    let sid = Id::new();
    let sess = ac_core::types::AgentSession::new(sid.clone(), "test-session".into(), "agy".into());
    app.sessions.push(sess);
    app.session_detail_id = Some(sid.clone());

    let mut buf = TerminalBuffer::new(500);
    for i in 1..=60 {
        buf.push_str(&format!("Line {i:02} of terminal build output\r\n"));
    }
    app.session_terminal_buffers.insert(sid.0.clone(), buf);

    let client = ApiClient::new("http://127.0.0.1:0".to_string());

    // 1. Start selection at row 10
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 5,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();

    let initial_offset = app.session_terminal_buffers.get(&sid.0).unwrap().scroll_offset;
    assert_eq!(initial_offset, 0);

    // 2. Drag cursor to row 0 (top header boundary)
    event::handle_mouse(
        &mut app,
        &client,
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
            column: 5,
            row: 0,
            modifiers: crossterm::event::KeyModifiers::NONE,
        },
    )
    .await
    .unwrap();

    let tb = app.session_terminal_buffers.get(&sid.0).unwrap();
    assert!(tb.is_selecting());
    assert!(tb.scroll_offset > 0, "Expected immediate scroll up on drag to top");
    let first_scroll_offset = tb.scroll_offset;

    // 3. User holds cursor at the top: simulate periodic tick autoscrolling
    app.mouse_selection.last_autoscroll_time = None;
    event::handle_mouse_autoscroll(&mut app);

    let tb = app.session_terminal_buffers.get(&sid.0).unwrap();
    assert!(tb.scroll_offset > first_scroll_offset, "Expected continuous autoscroll up on tick");
    let second_scroll_offset = tb.scroll_offset;

    // Further ticks continue moving the terminal automatically upwards
    app.mouse_selection.last_autoscroll_time = None;
    event::handle_mouse_autoscroll(&mut app);
    let tb = app.session_terminal_buffers.get(&sid.0).unwrap();
    assert!(tb.scroll_offset > second_scroll_offset, "Expected terminal to keep moving upwards");
}

#[tokio::test]
async fn test_multiline_paste_and_prompt_history_cohesion() {
    let mut app = App::new();
    let client = create_test_client();
    let sid = Id::from("sess-multiline-paste-1");
    app.session_detail_id = Some(sid.clone());

    let multiline_prompt = "Plan for Orion Terminal:\n1. Foundation in Rust\n2. Linux PTY & Systems\n3. Build Terminal Core";

    // 1. Paste multiline prompt into the terminal
    event::handle_paste(&mut app, &client, multiline_prompt)
        .await
        .unwrap();

    // The draft prompt buffer must contain the entire multiline prompt, NOT just the trailing line
    assert_eq!(app.session_prompt_buffer(&sid.0), multiline_prompt);

    // History must NOT be polluted with intermediate lines before Enter is pressed
    assert!(
        app.prompt_history_for_session(&sid.0).is_none()
            || app.prompt_history_for_session(&sid.0).unwrap().is_empty()
    );

    // 2. Press Enter to submit the prompt
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    // Exactly 1 history entry should exist containing the complete multiline prompt
    let history = app.prompt_history_for_session(&sid.0).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history.entries()[0], multiline_prompt);
    assert_eq!(app.session_prompt_buffer(&sid.0), "");

    // 3. Press Up -> recalls the complete multiline prompt, NOT line by line
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    assert_eq!(app.session_prompt_buffer(&sid.0), multiline_prompt);

    // 4. Press Down -> restores the original empty draft
    event::handle_key(
        &mut app,
        &client,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .await
    .unwrap();

    assert_eq!(app.session_prompt_buffer(&sid.0), "");
}

#[test]
fn test_left_and_right_arrow_navigation_preserves_input_text() {
    let mut buf = TerminalBuffer::new(500);

    // 1. Initial input: user types "cargo test"
    buf.push_str("cargo test");
    assert_eq!(buf.lines[0].to_plain_string(), "cargo test");
    assert_eq!(buf.cursor_col, 10);

    // 2. Press Left Arrow: agy/readline sends \x08 (terminfo cub1)
    // Moving cursor backward MUST NOT truncate or erase characters.
    buf.push_str("\x08"); // cursor over 't'
    assert_eq!(buf.cursor_col, 9);
    assert_eq!(buf.lines[0].to_plain_string(), "cargo test");

    buf.push_str("\x08"); // cursor over 's'
    assert_eq!(buf.cursor_col, 8);
    assert_eq!(buf.lines[0].to_plain_string(), "cargo test");

    buf.push_str("\x08"); // cursor over 'e'
    assert_eq!(buf.cursor_col, 7);
    assert_eq!(buf.lines[0].to_plain_string(), "cargo test");

    buf.push_str("\x08"); // cursor over 't'
    assert_eq!(buf.cursor_col, 6);
    assert_eq!(buf.lines[0].to_plain_string(), "cargo test");

    // 3. Move cursor back to beginning via CSI D
    buf.push_str("\x1b[6D");
    assert_eq!(buf.cursor_col, 0);
    assert_eq!(buf.lines[0].to_plain_string(), "cargo test");

    // 4. Move cursor forward to end via CSI C (Right Arrow)
    buf.push_str("\x1b[10C");
    assert_eq!(buf.cursor_col, 10);
    assert_eq!(buf.lines[0].to_plain_string(), "cargo test");

    // Visible rendered line must contain full "cargo test" without missing characters
    let (vis, _, _) = buf.get_visible_lines_wrapped(5, 80);
    assert_eq!(vis[0].spans[0].content, "cargo test");
}

#[test]
fn test_backspace_with_styled_trailing_spaces_cleans_prompt_buffer_completely() {
    let mut buf = TerminalBuffer::new(500);

    // 1. Initial input: prompt with color style
    buf.push_str("\x1b[36m> dsafdf  sa\x1b[0m");
    assert_eq!(buf.lines[0].to_plain_string(), "> dsafdf  sa");

    // 2. Erase each character using standard CLI \x08 \x08 sequences with active style
    let len = "dsafdf  sa".len();
    for _ in 0..len {
        buf.push_str("\x08 \x08");
    }

    // Must leave only "> " without any leftover "dsafdf  sa" ghost characters
    assert_eq!(buf.lines[0].to_plain_string(), ">");
    let (vis, _, _) = buf.get_visible_lines_wrapped(5, 80);
    assert_eq!(vis[0].spans[0].content, ">");
}


