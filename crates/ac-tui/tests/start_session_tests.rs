//! TUI tests for the "Start Antigravity Session" form (account, permission
//! mode, working directory with TAB completion, model) and the login-link UI.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};

use ac_core::{
    agy_launch::{AgyExecutionMode, AgyPermissionMode},
    types::{Account, Id},
};
use ac_tui::{
    app::{AgyAddStep, AgyLoginTask, AgyLoginUpdate, App, Modal},
    client::ApiClient,
    event::{handle_key, poll_agy_login},
    launch::{
        self, StartSessionForm, FIELD_BUTTONS, FIELD_DIR, FIELD_EXEC_MODE, FIELD_MODEL,
        FIELD_PERM_MODE,
    },
    ui,
};

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn render(app: &App) -> String {
    let mut t = Terminal::new(TestBackend::new(120, 40)).unwrap();
    t.draw(|f| ui::draw(f, app)).unwrap();
    t.backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<Vec<_>>()
        .chunks(120)
        .map(|r| r.concat())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A tree like the one in the spec, plus a file and an unreadable directory.
fn tree() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    for d in [
        "AgentControl",
        "AgentDesk/.git",
        "AgentDesk/core",
        "AgentDesk/docs",
        "AgentDesk/mobile",
        "AgentDesk/tests",
        "AgentMesh",
        "Hybrid",
        "locked",
    ] {
        std::fs::create_dir_all(t.path().join(d)).unwrap();
    }
    std::fs::write(t.path().join("notes.txt"), "not a dir").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            t.path().join("locked"),
            std::fs::Permissions::from_mode(0o000),
        )
        .unwrap();
    }
    t
}

fn setup(root: &Path) -> (App, ApiClient, Id) {
    let mut app = App::new();
    let a = Account::new(
        "vijaygovindbiju@gmail.com".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:agy:01A".into(),
        2,
        vec![],
    );
    let b = Account::new(
        "24ct406@mgits.ac.in".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:agy:01B".into(),
        2,
        vec![],
    );
    let aid = a.id.clone();
    app.accounts = vec![a, b];
    let form = StartSessionForm::new(
        root.to_path_buf(),
        root.to_path_buf(),
        AgyExecutionMode::ALL.to_vec(),
        AgyPermissionMode::ALL.to_vec(),
    );
    app.active_modal = Some(Modal::StartSession(Box::new(form)));
    let client =
        ApiClient::new(std::env::temp_dir().join(format!("no-daemon-{}.sock", ulid::Ulid::new())));
    (app, client, aid)
}

fn form(app: &App) -> &StartSessionForm {
    match &app.active_modal {
        Some(Modal::StartSession(f)) => f,
        other => panic!("form not open: {other:?}"),
    }
}

fn form_mut(app: &mut App) -> &mut StartSessionForm {
    match &mut app.active_modal {
        Some(Modal::StartSession(f)) => f,
        other => panic!("form not open: {other:?}"),
    }
}

async fn keys(app: &mut App, client: &ApiClient, ks: &[KeyCode]) {
    for k in ks {
        handle_key(app, client, press(*k)).await.unwrap();
    }
}

async fn type_str(app: &mut App, client: &ApiClient, s: &str) {
    for c in s.chars() {
        handle_key(app, client, press(KeyCode::Char(c)))
            .await
            .unwrap();
    }
}

/// Wait for background directory listings to be applied (as the tick does).
async fn settle(app: &mut App) {
    for _ in 0..300 {
        launch::poll_jobs(app);
        if !matches!(&app.active_modal, Some(Modal::StartSession(f)) if f.listing.is_some()) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("directory listing never finished");
}

async fn set_dir(app: &mut App, client: &ApiClient, s: &str) {
    form_mut(app).field = FIELD_DIR;
    form_mut(app).dir_input.clear();
    form_mut(app).completion = None;
    type_str(app, client, s).await;
}

fn popup(app: &App) -> Vec<String> {
    let f = form(app);
    f.completion
        .as_ref()
        .map(|c| {
            c.matches(launch::split_input(&f.dir_input).1)
                .into_iter()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn tab_lists_only_child_directories_and_filters_by_typed_text() {
    let t = tree();
    let (mut app, client, _) = setup(t.path());
    set_dir(&mut app, &client, "~/").await;
    keys(&mut app, &client, &[KeyCode::Tab]).await;
    settle(&mut app).await;
    assert_eq!(
        popup(&app),
        [
            "AgentControl/",
            "AgentDesk/",
            "AgentMesh/",
            "Hybrid/",
            "locked/"
        ],
        "files are not listed"
    );
    let screen = render(&app);
    assert!(
        screen.contains("Tab complete")
            && screen.contains("Enter open")
            && screen.contains("AgentDesk/"),
        "{screen}"
    );

    // Typing keeps the popup and filters it (case-insensitive).
    type_str(&mut app, &client, "a").await;
    assert_eq!(popup(&app), ["AgentControl/", "AgentDesk/", "AgentMesh/"]);
    // ↓ select, Enter accepts the directory.
    keys(&mut app, &client, &[KeyCode::Down, KeyCode::Enter]).await;
    assert_eq!(form(&app).dir_input, "~/AgentDesk/");
    assert!(form(&app).completion.is_none());

    // TAB again: the children of the selected directory (hidden ones last).
    keys(&mut app, &client, &[KeyCode::Tab]).await;
    settle(&mut app).await;
    assert_eq!(
        popup(&app),
        ["core/", "docs/", "mobile/", "tests/", ".git/"]
    );
    // TAB cycles, Esc closes the popup but keeps the form.
    keys(&mut app, &client, &[KeyCode::Tab, KeyCode::Tab]).await;
    assert_eq!(form(&app).completion.as_ref().unwrap().selected, 2);
    keys(&mut app, &client, &[KeyCode::Esc]).await;
    assert!(form(&app).completion.is_none());
    assert_eq!(form(&app).dir_input, "~/AgentDesk/");
    assert_eq!(form(&app).resolved_dir(), t.path().join("AgentDesk"));
}

#[tokio::test]
async fn tab_completes_like_a_shell() {
    let t = tree();
    let (mut app, client, _) = setup(t.path());
    // Common prefix is filled in, popup offers the candidates.
    set_dir(&mut app, &client, "~/a").await;
    keys(&mut app, &client, &[KeyCode::Tab]).await;
    settle(&mut app).await;
    assert_eq!(form(&app).dir_input, "~/Agent");
    assert_eq!(popup(&app).len(), 3);

    // Unique match completes directly; nested navigation.
    set_dir(&mut app, &client, "~/hy").await;
    keys(&mut app, &client, &[KeyCode::Tab]).await;
    settle(&mut app).await;
    assert_eq!(form(&app).dir_input, "~/Hybrid/");
    set_dir(&mut app, &client, "~/AgentDesk/co").await;
    keys(&mut app, &client, &[KeyCode::Tab]).await;
    settle(&mut app).await;
    assert_eq!(form(&app).dir_input, "~/AgentDesk/core/");

    // Absolute and relative paths.
    set_dir(
        &mut app,
        &client,
        &format!("{}/AgentDesk/d", t.path().display()),
    )
    .await;
    keys(&mut app, &client, &[KeyCode::Tab]).await;
    settle(&mut app).await;
    assert_eq!(
        form(&app).dir_input,
        format!("{}/AgentDesk/docs/", t.path().display())
    );
    set_dir(&mut app, &client, "AgentDesk/m").await;
    keys(&mut app, &client, &[KeyCode::Tab]).await;
    settle(&mut app).await;
    assert_eq!(form(&app).dir_input, "AgentDesk/mobile/");
    assert_eq!(form(&app).resolved_dir(), t.path().join("AgentDesk/mobile"));

    // No match → notice, no popup.
    set_dir(&mut app, &client, "~/zz").await;
    keys(&mut app, &client, &[KeyCode::Tab]).await;
    settle(&mut app).await;
    assert_eq!(
        form(&app).dir_notice.as_deref(),
        Some("No matching directories")
    );
    assert!(form(&app).completion.is_none());
}

#[tokio::test]
async fn invalid_and_inaccessible_directories_are_reported_not_launched() {
    let t = tree();
    let (mut app, client, _) = setup(t.path());
    set_dir(&mut app, &client, "~/does-not-exist/").await;
    assert!(render(&app).contains("not a directory"));
    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert!(form(&app)
        .dir_notice
        .as_ref()
        .unwrap()
        .contains("does not exist"));
    // Start refuses as well.
    form_mut(&mut app).field = FIELD_BUTTONS;
    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert!(form(&app)
        .error
        .as_ref()
        .unwrap()
        .contains("does not exist"));
    assert!(app.status_message.is_none(), "nothing was launched");

    #[cfg(unix)]
    if !nix_is_root() {
        set_dir(&mut app, &client, "~/locked/").await;
        keys(&mut app, &client, &[KeyCode::Tab]).await;
        settle(&mut app).await;
        assert!(
            form(&app)
                .dir_notice
                .as_ref()
                .unwrap()
                .contains("Permission denied"),
            "{:?}",
            form(&app).dir_notice
        );
        keys(&mut app, &client, &[KeyCode::Enter]).await;
        assert!(form(&app)
            .dir_notice
            .as_ref()
            .unwrap()
            .contains("not accessible"));
    }

    // A file is not a directory.
    set_dir(&mut app, &client, "~/notes.txt").await;
    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert!(form(&app)
        .dir_notice
        .as_ref()
        .unwrap()
        .contains("does not exist"));

    // Manual entry of a valid path without completion is accepted and normalised.
    set_dir(&mut app, &client, "~/AgentDesk/../Hybrid").await;
    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert_eq!(form(&app).dir_input, "~/Hybrid");
    assert_eq!(form(&app).field, FIELD_EXEC_MODE);
}

#[cfg(unix)]
fn nix_is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .map(|s| {
            s.lines()
                .any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0"))
        })
        .unwrap_or(false)
}

#[tokio::test]
async fn permission_mode_account_and_model_selection_build_the_launch_options() {
    let t = tree();
    let (mut app, client, aid) = setup(t.path());
    let b = app.accounts[1].id.clone();
    app.agy_models.insert(
        b.0.clone(),
        Some(Ok(vec![(
            "gemini-3.8-flash-medium".into(),
            "Gemini 3.8 Flash (Medium)".into(),
        )])),
    );

    // Defaults: first account, Default exec mode, Normal perm mode, the TUI's directory, agy's default model.
    let o = launch::launch_options(&app, form(&app)).unwrap();
    assert_eq!(o.execution_mode, AgyExecutionMode::Default);
    assert_eq!(o.permission_mode, AgyPermissionMode::Normal);
    assert_eq!(o.model, None);
    assert_eq!(PathBuf::from(o.working_dir.unwrap()), t.path());
    assert_eq!(launch::agy_accounts(&app)[form(&app).account_index].id, aid);

    // Account: →, including the "+ Add" entry, wraps around.
    keys(&mut app, &client, &[KeyCode::Right]).await;
    assert_eq!(launch::agy_accounts(&app)[form(&app).account_index].id, b);
    keys(&mut app, &client, &[KeyCode::Right]).await;
    assert!(render(&app).contains("+ Add Antigravity Account"));
    keys(&mut app, &client, &[KeyCode::Right, KeyCode::Right]).await;
    assert_eq!(launch::agy_accounts(&app)[form(&app).account_index].id, b);

    // Execution mode selection: Default, Accept Edits, Plan.
    form_mut(&mut app).field = FIELD_EXEC_MODE;
    for m in &AgyExecutionMode::ALL[1..] {
        keys(&mut app, &client, &[KeyCode::Right]).await;
        assert_eq!(form(&app).exec_mode(), *m);
        assert!(render(&app).contains(m.label()));
    }
    keys(&mut app, &client, &[KeyCode::Right]).await;
    assert_eq!(
        form(&app).exec_mode(),
        AgyExecutionMode::Default,
        "wraps back to Default"
    );
    keys(&mut app, &client, &[KeyCode::Right, KeyCode::Right]).await; // Selects Plan

    // Permission / Access mode selection: Normal, Dangerously Skip Permissions.
    form_mut(&mut app).field = FIELD_PERM_MODE;
    for m in &AgyPermissionMode::ALL[1..] {
        keys(&mut app, &client, &[KeyCode::Right]).await;
        assert_eq!(form(&app).perm_mode(), *m);
        assert!(render(&app).contains(m.label()));
    }
    keys(&mut app, &client, &[KeyCode::Right]).await;
    assert_eq!(
        form(&app).perm_mode(),
        AgyPermissionMode::Normal,
        "wraps back to Normal"
    );

    form_mut(&mut app).field = FIELD_MODEL;
    keys(&mut app, &client, &[KeyCode::Right]).await;
    assert!(render(&app).contains("Gemini 3.8 Flash (Medium)"));
    let o = launch::launch_options(&app, form(&app)).unwrap();
    assert_eq!(o.execution_mode, AgyExecutionMode::Plan);
    assert_eq!(o.permission_mode, AgyPermissionMode::Normal);
    assert_eq!(o.model.as_deref(), Some("gemini-3.8-flash-medium"));
}

#[tokio::test]
async fn dangerous_mode_requires_explicit_confirmation() {
    let t = tree();
    let (mut app, client, _) = setup(t.path());
    form_mut(&mut app).perm_mode_index = AgyPermissionMode::ALL
        .iter()
        .position(|m| m.is_dangerous())
        .unwrap();
    form_mut(&mut app).field = FIELD_BUTTONS;

    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert_eq!(
        form(&app).confirm_dangerous,
        Some(0),
        "Cancel is focused by default"
    );
    let screen = render(&app);
    assert!(
        screen.contains("Dangerously Skip Permissions")
            && screen.contains("automatically approve")
            && screen.contains("[ Continue ]")
    );
    assert!(app.status_message.is_none(), "not launched yet");

    // Enter on Cancel returns to the form without launching.
    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert_eq!(form(&app).confirm_dangerous, None);
    assert!(app.status_message.is_none());
    // Esc also backs out.
    keys(&mut app, &client, &[KeyCode::Enter, KeyCode::Esc]).await;
    assert!(app.active_modal.is_some() && app.status_message.is_none());

    // Continue launches (the unreachable test daemon reports the attempt).
    keys(
        &mut app,
        &client,
        &[KeyCode::Enter, KeyCode::Right, KeyCode::Enter],
    )
    .await;
    assert!(app.active_modal.is_none());
    assert!(app
        .status_message
        .as_ref()
        .unwrap()
        .0
        .contains("Failed to create session"));
}

#[tokio::test]
async fn safe_modes_launch_without_a_confirmation_and_esc_cancels() {
    let t = tree();
    let (mut app, client, _) = setup(t.path());
    let screen = render(&app);
    for s in [
        "Start Antigravity Session",
        "Account",
        "vijaygovindbiju@gmail.com",
        "Execution Mode",
        "Default",
        "Permission / Access",
        "Normal",
        "Working Directory",
        "Resolved:",
        "[ TAB completion ]",
        "Model",
        "Default (agy setting)",
        "[ Start Session ]",
        "[ Cancel ]",
    ] {
        assert!(screen.contains(s), "missing {s:?}\n{screen}");
    }
    keys(&mut app, &client, &[KeyCode::Esc]).await;
    assert!(app.active_modal.is_none() && app.status_message.is_none());

    let (mut app, client, _) = setup(t.path());
    form_mut(&mut app).exec_mode_index = 2; // Plan
    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert!(
        app.active_modal.is_none(),
        "no confirmation for non-dangerous modes"
    );
    assert!(app
        .status_message
        .as_ref()
        .unwrap()
        .0
        .contains("Failed to create session"));
}

#[tokio::test]
async fn adding_an_account_from_the_form_returns_to_it() {
    let t = tree();
    let (mut app, client, _) = setup(t.path());
    form_mut(&mut app).account_index = 2; // "+ Add Antigravity Account"
    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert!(matches!(
        &app.active_modal,
        Some(Modal::AgyAddAccount {
            step: AgyAddStep::Name { .. },
            ..
        })
    ));
    keys(&mut app, &client, &[KeyCode::Esc]).await;
    assert!(
        matches!(&app.active_modal, Some(Modal::StartSession(_))),
        "cancel returns to the form"
    );
}

#[tokio::test]
async fn login_link_screen_masks_pasted_address_and_forwards_it() {
    let (mut app, client, _) = setup(Path::new("/"));
    app.agy_add_method = 0;
    app.active_modal = Some(Modal::AgyAddAccount {
        label: "Phone".into(),
        step: AgyAddStep::Method,
    });
    keys(&mut app, &client, &[KeyCode::Down]).await;
    assert!(render(&app).contains("Get a link to open on another device"));

    let updates: std::sync::Arc<std::sync::Mutex<Vec<AgyLoginUpdate>>> = Default::default();
    let task = tokio::spawn(async { tokio::time::sleep(std::time::Duration::from_secs(60)).await });
    app.agy_login = Some(AgyLoginTask {
        abort: task.abort_handle(),
        updates: updates.clone(),
    });
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    app.agy_login_paste = Some(tx);
    updates.lock().unwrap().push(AgyLoginUpdate::LinkReady {
        url: "https://accounts.google.com/o/oauth2/v2/auth?client_id=x&state=S".into(),
    });
    poll_agy_login(&mut app, &client).await;
    let screen = render(&app);
    assert!(
        screen.contains("Login Link")
            && screen.contains("Copy Link")
            && screen.contains("Waiting for authentication")
            && screen.contains("single use"),
        "{screen}"
    );

    let pasted = "http://127.0.0.1:4242/oauth2callback?state=S&code=4/0SECRETCODE";
    type_str(&mut app, &client, pasted).await;
    let screen = render(&app);
    assert!(
        !screen.contains("SECRETCODE") && screen.contains("chars)"),
        "pasted code must be masked"
    );
    keys(&mut app, &client, &[KeyCode::Enter]).await;
    assert_eq!(rx.recv().await.unwrap(), pasted);
    assert!(render(&app).contains("Verifying"));

    updates.lock().unwrap().push(AgyLoginUpdate::PasteRejected {
        reason: "Invalid OAuth state.".into(),
    });
    poll_agy_login(&mut app, &client).await;
    assert!(render(&app).contains("Not accepted: Invalid OAuth state."));

    // Cancel invalidates the pending login (task aborted, paste channel dropped).
    keys(&mut app, &client, &[KeyCode::Esc]).await;
    assert!(app.agy_login.is_none() && app.agy_login_paste.is_none());
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(rx.recv().await.is_none(), "paste channel closed");
}
