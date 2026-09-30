//! TUI tests for Antigravity account removal and add-account safety.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

use ac_core::types::{Account, AgentSession, Id, SessionState};
use ac_tui::{
    app::{AgyAddStep, AgyLoginTask, AgyLoginUpdate, App, Modal, StatusType, Tab},
    client::ApiClient,
    event::{handle_key, poll_agy_login},
    ui,
};

fn app_with_account(active_session: bool) -> (App, ApiClient, Id) {
    let mut app = App::new();
    let acct = Account::new("Personal Google".into(), "agy".into(), vec!["agy".into()], "ref:agy:01X".into(), 2, vec![]);
    let aid = acct.id.clone();
    app.accounts = vec![acct];
    if active_session {
        let mut s = AgentSession::new(Id::new(), "[interactive] s".into(), "agy".into());
        s.account_id = Some(aid.clone());
        s.state = SessionState::Working;
        app.sessions = vec![s];
    }
    app.set_tab(Tab::Accounts);
    // Unreachable daemon: any accidental removal call would surface as an error.
    let client = ApiClient::new(std::env::temp_dir().join(format!("no-daemon-{}.sock", ulid::Ulid::new())));
    (app, client, aid)
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn render(app: &App) -> String {
    let mut t = Terminal::new(TestBackend::new(120, 40)).unwrap();
    t.draw(|f| ui::draw(f, app)).unwrap();
    format!("{:?}", t.backend().buffer())
}

#[tokio::test]
async fn delete_key_opens_confirmation_instead_of_removing() {
    let (mut app, client, aid) = app_with_account(false);
    handle_key(&mut app, &client, press(KeyCode::Char('d'))).await.unwrap();
    assert!(matches!(&app.active_modal, Some(Modal::ConfirmRemoveAccount { account_id, active_sessions: 0, .. }) if *account_id == aid));
    assert!(app.status_message.is_none(), "no removal may have been attempted");
    let screen = render(&app);
    assert!(screen.contains("Remove Antigravity Account"));
    assert!(screen.contains("Remove account \"Personal Google\"?"));
    assert!(screen.contains("Saved Antigravity credentials will be deleted."));
    assert!(screen.contains("[Enter] Remove") && screen.contains("[Esc] Cancel"));

    handle_key(&mut app, &client, press(KeyCode::Esc)).await.unwrap();
    assert!(app.active_modal.is_none());
    assert_eq!(app.accounts.len(), 1);
}

#[tokio::test]
async fn active_sessions_block_removal() {
    let (mut app, client, _) = app_with_account(true);
    handle_key(&mut app, &client, press(KeyCode::Char('d'))).await.unwrap();
    assert!(matches!(&app.active_modal, Some(Modal::ConfirmRemoveAccount { active_sessions: 1, .. })));
    let screen = render(&app);
    assert!(screen.contains("Cannot remove \"Personal Google\"."));
    assert!(screen.contains("Active sessions: 1"));
    assert!(screen.contains("Stop the session first"));

    handle_key(&mut app, &client, press(KeyCode::Enter)).await.unwrap();
    assert!(app.active_modal.is_some(), "Enter must not remove an account in use");
    assert!(app.status_message.is_none());
}

#[tokio::test]
async fn removal_failure_is_visible() {
    let (mut app, client, _) = app_with_account(false);
    handle_key(&mut app, &client, press(KeyCode::Char('d'))).await.unwrap();
    handle_key(&mut app, &client, press(KeyCode::Enter)).await.unwrap();
    let (msg, kind, _) = app.status_message.clone().unwrap();
    assert_eq!(kind, StatusType::Error);
    assert!(msg.contains("Failed to remove account"), "{msg}");
}

#[tokio::test]
async fn legacy_add_dialog_routes_antigravity_to_the_browser_login_flow() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", tmp.path());
    let (mut app, client, _) = app_with_account(false);
    app.active_modal = Some(Modal::AddAccount {
        label: "Work Google".into(),
        provider: "agy".into(),
        auth_method: 1,
        token: "4/0SOMECODE".into(),
        active_field: 3,
    });
    handle_key(&mut app, &client, press(KeyCode::Enter)).await.unwrap();
    assert!(matches!(&app.active_modal, Some(Modal::AgyAddAccount { step: AgyAddStep::Name { .. }, .. })));
    assert!(!tmp.path().join("agentcontrol/credentials").exists(), "no credential may be written");
}

fn type_str(s: &str) -> Vec<KeyEvent> {
    s.chars().map(|c| press(KeyCode::Char(c))).collect()
}

#[tokio::test]
async fn add_account_dialog_validates_name_then_offers_browser_login_first() {
    let (mut app, client, _) = app_with_account(false);
    handle_key(&mut app, &client, press(KeyCode::Char('a'))).await.unwrap();
    assert!(matches!(&app.active_modal, Some(Modal::AgyAddAccount { step: AgyAddStep::Name { .. }, .. })));
    let screen = render(&app);
    assert!(screen.contains("Add Antigravity Account") && screen.contains("Account name:"));

    // Empty and duplicate names are refused inline.
    handle_key(&mut app, &client, press(KeyCode::Enter)).await.unwrap();
    assert!(render(&app).contains("Account name cannot be empty."));
    for k in type_str("personal google") {
        handle_key(&mut app, &client, k).await.unwrap();
    }
    handle_key(&mut app, &client, press(KeyCode::Enter)).await.unwrap();
    assert!(render(&app).contains("You already have an account named"));

    for _ in 0..15 {
        handle_key(&mut app, &client, press(KeyCode::Backspace)).await.unwrap();
    }
    for k in type_str("College Google") {
        handle_key(&mut app, &client, k).await.unwrap();
    }
    handle_key(&mut app, &client, press(KeyCode::Enter)).await.unwrap();
    assert!(matches!(&app.active_modal, Some(Modal::AgyAddAccount { step: AgyAddStep::Method, label }) if label == "College Google"));
    let screen = render(&app);
    assert!(screen.contains("Authentication:") && screen.contains("Login with Browser") && screen.contains("Generate Login Link") && screen.contains("Import Existing Credentials"));
    handle_key(&mut app, &client, press(KeyCode::Esc)).await.unwrap();
    assert!(app.active_modal.is_none());
}

#[tokio::test]
async fn cancelling_a_waiting_login_aborts_it_and_saves_nothing() {
    let (mut app, client, _) = app_with_account(false);
    let task = tokio::spawn(async { tokio::time::sleep(std::time::Duration::from_secs(60)).await });
    app.agy_login = Some(AgyLoginTask { abort: task.abort_handle(), updates: Default::default() });
    app.active_modal = Some(Modal::AgyAddAccount {
        label: "College Google".into(),
        step: AgyAddStep::Waiting { url: "https://accounts.google.com/x".into(), browser_opened: false, deadline: std::time::Instant::now() + std::time::Duration::from_secs(272) },
    });
    let screen = render(&app);
    assert!(screen.contains("Unable to open your default browser") && screen.contains("Time remaining: 04:3"));
    handle_key(&mut app, &client, press(KeyCode::Esc)).await.unwrap();
    assert!(app.active_modal.is_none() && app.agy_login.is_none());
    assert!(task.await.unwrap_err().is_cancelled(), "login task must be aborted");
    assert!(app.status_message.as_ref().unwrap().0.contains("No credential was saved"));
}

#[tokio::test]
async fn login_results_drive_success_and_failure_screens() {
    let (mut app, client, aid) = app_with_account(false);
    let updates: std::sync::Arc<std::sync::Mutex<Vec<AgyLoginUpdate>>> = Default::default();
    let task = tokio::spawn(async {});
    app.agy_login = Some(AgyLoginTask { abort: task.abort_handle(), updates: updates.clone() });
    app.active_modal = Some(Modal::AgyAddAccount { label: "Personal Google".into(), step: AgyAddStep::Method });

    updates.lock().unwrap().push(AgyLoginUpdate::Failed { reason: "Authorization was denied.".into() });
    poll_agy_login(&mut app, &client).await;
    let screen = render(&app);
    assert!(screen.contains("Antigravity Login Failed") && screen.contains("Reason: Authorization was denied.") && screen.contains("Retry"));
    assert!(app.agy_login.is_none());
    assert_eq!(app.accounts.len(), 1, "failure must not add an account");

    app.agy_login = Some(AgyLoginTask { abort: task.abort_handle(), updates: updates.clone() });
    updates.lock().unwrap().push(AgyLoginUpdate::Succeeded { account_id: aid, email: Some("a@x.com".into()) });
    poll_agy_login(&mut app, &client).await;
    let screen = render(&app);
    assert!(screen.contains("Antigravity Account Added") && screen.contains("Login successful") && screen.contains("● Ready"));
    handle_key(&mut app, &client, press(KeyCode::Enter)).await.unwrap();
    assert!(app.active_modal.is_none());
}

#[test]
fn background_login_results_reach_the_status_bar() {
    let mut app = App::new();
    app.background_notices
        .lock()
        .unwrap()
        .push(("Antigravity login failed. The account was not added.".into(), StatusType::Error));
    assert!(app.drain_background_notices());
    let (msg, kind, _) = app.status_message.clone().unwrap();
    assert_eq!(kind, StatusType::Error);
    assert!(msg.contains("not added"));
    assert!(!app.drain_background_notices());
}
