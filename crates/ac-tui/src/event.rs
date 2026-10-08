//! Keyboard input and async event handling.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    app::{AgyAddStep, AgyLoginTask, AgyLoginUpdate, App, Modal, StatusType, Tab},
    client::ApiClient,
};
use ac_core::types::{Id, PolicyDecision, SessionState};

/// Collapse a multi-line user-facing error into one status-bar line.
pub fn one_line(msg: &str) -> String {
    msg.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Register a freshly saved Antigravity credential; on failure the credential
/// file is deleted so no partially authenticated account remains.
async fn register_agy_account(
    client: &ApiClient,
    label: &str,
    credential_ref: &str,
) -> std::result::Result<Id, String> {
    client
        .register_account(
            label,
            ac_core::agy_auth::PROVIDER,
            &ac_core::agy_auth::AGENT_TYPES,
            credential_ref,
            2,
            &["google"],
        )
        .await
        .map_err(|e| {
            ac_core::agy_auth::discard_credential(credential_ref);
            format!("Registering the account failed: {e}")
        })
}

/// Extract the human-readable reason from a multi-line auth error.
pub fn failure_reason(msg: &str) -> String {
    one_line(msg.split_once("Reason:").map(|(_, r)| r).unwrap_or(msg))
}

/// Open the "Add Antigravity Account" dialog.
pub fn open_agy_add_account(app: &mut App) {
    app.active_modal = Some(Modal::AgyAddAccount {
        label: String::new(),
        step: AgyAddStep::Name { error: None },
    });
}

/// Start the shared Antigravity browser login in the background. At most one
/// login runs at a time; cancelling aborts the task, which closes the callback
/// listener and drops the in-memory OAuth state and PKCE verifier.
pub fn start_agy_browser_login(app: &mut App, client: ApiClient, label: String) {
    start_agy_login(app, client, label, false)
}

/// Start an Antigravity login in the background.
///
/// `link = false`: open the browser and wait for the loopback callback.
/// `link = true`: generate a shareable login link without opening anything;
/// it completes through the loopback callback (link opened on this machine) or
/// through the redirect address the user pastes back (link opened elsewhere).
/// Both use the same short-lived state + PKCE login; see
/// `ac_core::agy_auth::finish_login_with_handoff`.
pub fn start_agy_login(app: &mut App, client: ApiClient, label: String, link: bool) {
    if app
        .agy_login
        .as_ref()
        .is_some_and(|t| !t.abort.is_finished())
    {
        app.set_status(
            "An Antigravity login is already in progress.",
            StatusType::Warning,
        );
        return;
    }
    let updates: std::sync::Arc<std::sync::Mutex<Vec<AgyLoginUpdate>>> = Default::default();
    let tx = updates.clone();
    let push = move |u: AgyLoginUpdate| {
        if let Ok(mut v) = tx.lock() {
            v.push(u);
        }
    };
    let (paste_tx, mut paste_rx) = tokio::sync::mpsc::channel::<String>(4);
    let lbl = label.clone();
    let handle = tokio::spawn(async move {
        let result: std::result::Result<(Id, Option<String>), String> = async {
            let login = ac_core::agy_auth::begin_browser_login()
                .await
                .map_err(|e| e.to_string())?;
            let timeout = ac_core::agy_auth::LOGIN_TIMEOUT;
            let (cref, token) = if link {
                push(AgyLoginUpdate::LinkReady {
                    url: login.authorization_url().to_string(),
                });
                let rejected = |reason: String| {
                    push(AgyLoginUpdate::PasteRejected {
                        reason: one_line(&reason),
                    })
                };
                ac_core::agy_auth::finish_login_with_handoff(
                    &login,
                    &lbl,
                    timeout,
                    &mut paste_rx,
                    rejected,
                )
                .await
            } else {
                let opened = login.open_in_browser();
                push(AgyLoginUpdate::Waiting {
                    url: login.authorization_url().to_string(),
                    browser_opened: opened,
                });
                ac_core::agy_auth::finish_browser_login(&login, &lbl, timeout).await
            }
            .map_err(|e| e.to_string())?;
            let id = register_agy_account(&client, &lbl, &cref).await?;
            Ok((id, token.email()))
        }
        .await;
        push(match result {
            Ok((account_id, email)) => AgyLoginUpdate::Succeeded { account_id, email },
            Err(e) => AgyLoginUpdate::Failed {
                reason: failure_reason(&e),
            },
        });
    });
    app.agy_login = Some(AgyLoginTask {
        abort: handle.abort_handle(),
        updates,
    });
    app.agy_login_paste = link.then_some(paste_tx);
    app.active_modal = Some(Modal::AgyAddAccount {
        label,
        step: AgyAddStep::Waiting {
            url: String::new(),
            browser_opened: !link,
            deadline: std::time::Instant::now() + ac_core::agy_auth::LOGIN_TIMEOUT,
        },
    });
}

/// Cancel the in-flight login (if any). Aborting the task drops the callback
/// listener, the OAuth state and the PKCE verifier, so the link stops working.
pub fn cancel_agy_login(app: &mut App) {
    app.agy_login_paste = None;
    if let Some(t) = app.agy_login.take() {
        t.abort.abort();
    }
}

/// Close the add-account dialog, returning to the start-session form if it was opened from there.
fn close_add_account(app: &mut App) {
    match app.resume_start_session.take() {
        Some(form) => crate::launch::open_form(app, form),
        None => app.active_modal = None,
    }
}

/// Copy the login link to the clipboard and describe the result.
fn copy_notice(url: &str) -> String {
    if crate::clipboard::copy(url) {
        "Login link copied to the clipboard.".into()
    } else {
        "Could not access the clipboard — select the link above to copy it.".into()
    }
}

/// Apply background login progress to the dialog. Called on every tick.
pub async fn poll_agy_login(app: &mut App, client: &ApiClient) {
    let Some(task) = app.agy_login.clone() else {
        return;
    };
    let updates: Vec<AgyLoginUpdate> = task
        .updates
        .lock()
        .map(|mut v| v.drain(..).collect())
        .unwrap_or_default();
    for u in updates {
        let label = match &app.active_modal {
            Some(Modal::AgyAddAccount { label, .. }) => label.clone(),
            _ => String::new(),
        };
        let step = match u {
            AgyLoginUpdate::Waiting {
                url,
                browser_opened,
            } => AgyAddStep::Waiting {
                url,
                browser_opened,
                deadline: std::time::Instant::now() + ac_core::agy_auth::LOGIN_TIMEOUT,
            },
            AgyLoginUpdate::LinkReady { url } => AgyAddStep::Link {
                url,
                deadline: std::time::Instant::now() + ac_core::agy_auth::LOGIN_TIMEOUT,
                paste: String::new(),
                notice: None,
            },
            AgyLoginUpdate::PasteRejected { reason } => match &app.active_modal {
                Some(Modal::AgyAddAccount {
                    step:
                        AgyAddStep::Link {
                            url,
                            deadline,
                            paste,
                            ..
                        },
                    ..
                }) => AgyAddStep::Link {
                    url: url.clone(),
                    deadline: *deadline,
                    paste: paste.clone(),
                    notice: Some(format!("Not accepted: {reason}")),
                },
                _ => continue,
            },
            AgyLoginUpdate::Succeeded { account_id, email } => {
                app.agy_login = None;
                app.agy_login_paste = None;
                refresh_data(app, client).await;
                if let Some(i) = app.accounts.iter().position(|a| a.id == account_id) {
                    app.selected_account = i;
                }
                AgyAddStep::Success { account_id, email }
            }
            AgyLoginUpdate::Failed { reason } => {
                app.agy_login = None;
                app.agy_login_paste = None;
                AgyAddStep::Failed { reason }
            }
        };
        if matches!(app.active_modal, Some(Modal::AgyAddAccount { .. })) {
            app.active_modal = Some(Modal::AgyAddAccount { label, step });
        }
    }
}

/// Import the machine's existing agy login as a new account.
async fn import_agy_login(app: &mut App, client: &ApiClient, label: String) {
    let step = match ac_core::agy_auth::detect_local_login() {
        None => AgyAddStep::Failed {
            reason: "No valid existing agy login was found on this machine.".into(),
        },
        Some((_, tok)) => {
            match ac_core::agy_auth::save_new_credential(&label, &tok, "local_import") {
                Err(e) => AgyAddStep::Failed {
                    reason: failure_reason(&e.to_string()),
                },
                Ok(cref) => match register_agy_account(client, &label, &cref).await {
                    Err(e) => AgyAddStep::Failed { reason: e },
                    Ok(account_id) => {
                        refresh_data(app, client).await;
                        if let Some(i) = app.accounts.iter().position(|a| a.id == account_id) {
                            app.selected_account = i;
                        }
                        AgyAddStep::Success {
                            account_id,
                            email: tok.email(),
                        }
                    }
                },
            }
        }
    };
    app.active_modal = Some(Modal::AgyAddAccount { label, step });
}

/// Create and start a session, then open its terminal.
pub async fn launch_session_and_open(
    app: &mut App,
    client: &ApiClient,
    task_desc: &str,
    agent_type: &str,
    account: Option<&Id>,
    launch: Option<&ac_core::agy_launch::AgyLaunchOptions>,
) {
    match client
        .create_session_with_launch(task_desc, agent_type, None, account, launch)
        .await
    {
        Ok(new_id) => match client.start_session(&new_id).await {
            Ok(()) => {
                app.set_status(
                    format!("✓ Launched session {} successfully!", new_id.0),
                    StatusType::Success,
                );
                refresh_data(app, client).await;
                app.set_tab(Tab::Sessions);
                app.session_detail_id = Some(new_id.clone());
                load_session_history_if_needed(app, client, &new_id).await;
                if let Ok((cols, rows)) = crossterm::terminal::size() {
                    let _ = client
                        .resize_session(&new_id, rows.saturating_sub(2), cols)
                        .await;
                }
            }
            Err(e) => app.set_status(
                format!("Created session {} but failed to start: {e}", new_id.0),
                StatusType::Error,
            ),
        },
        Err(e) => app.set_status(format!("Failed to create session: {e}"), StatusType::Error),
    }
}

/// Number of non-terminal sessions currently bound to an account.
pub fn active_sessions_for_account(app: &App, account_id: &Id) -> usize {
    app.sessions
        .iter()
        .filter(|s| s.account_id.as_ref() == Some(account_id) && !s.state.is_terminal())
        .count()
}

/// Handle mouse scroll wheel and click events.
pub async fn handle_mouse(
    app: &mut App,
    client: &ApiClient,
    mouse: crossterm::event::MouseEvent,
) -> Result<()> {
    match mouse.kind {
        crossterm::event::MouseEventKind::ScrollUp => {
            if let Some(ref mut modal) = app.active_modal {
                match modal {
                    Modal::UrlPicker { selected_index, .. } => {
                        *selected_index = selected_index.saturating_sub(1);
                    }
                    Modal::CommandPalette { selected_index, .. } => {
                        *selected_index = selected_index.saturating_sub(1);
                    }
                    _ => {}
                }
            } else if let Some(ref detail_id) = app.session_detail_id.clone() {
                app.scroll_session_terminal_up(&detail_id.0, 3);
            } else {
                app.prev_row();
            }
        }
        crossterm::event::MouseEventKind::ScrollDown => {
            if let Some(ref mut modal) = app.active_modal {
                match modal {
                    Modal::UrlPicker {
                        selected_index,
                        urls,
                        ..
                    } => {
                        if *selected_index + 1 < urls.len() {
                            *selected_index += 1;
                        }
                    }
                    Modal::CommandPalette { selected_index, .. } => {
                        *selected_index += 1;
                    }
                    _ => {}
                }
            } else if let Some(ref detail_id) = app.session_detail_id.clone() {
                app.scroll_session_terminal_down(&detail_id.0, 3);
            } else {
                app.next_row();
            }
        }
        crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            if app.active_modal.is_none() {
                if let Some(ref detail_id) = app.session_detail_id.clone() {
                    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                    let (header_h, footer_h) = if rows < 3 {
                        (0, 0)
                    } else if rows < 5 {
                        (1, 0)
                    } else {
                        (1, 1)
                    };
                    let term_inner_h = rows.saturating_sub(header_h + footer_h) as usize;
                    let term_inner_w = cols as usize;

                    if mouse.row >= header_h && mouse.row < header_h + term_inner_h as u16 {
                        let rel_row = (mouse.row - header_h) as usize;
                        let rel_col = (mouse.column as usize).min(term_inner_w.saturating_sub(1));

                        let now = std::time::Instant::now();
                        let is_multiclick = app
                            .mouse_selection
                            .last_click_time
                            .map(|t| now.duration_since(t) < std::time::Duration::from_millis(400))
                            .unwrap_or(false)
                            && app.mouse_selection.last_click_pos == (mouse.column, mouse.row);

                        let click_count = if is_multiclick {
                            (app.mouse_selection.click_count % 3) + 1
                        } else {
                            1
                        };
                        app.mouse_selection.click_count = click_count;
                        app.mouse_selection.last_click_time = Some(now);
                        app.mouse_selection.last_click_pos = (mouse.column, mouse.row);
                        app.mouse_selection.drag_pos = Some((mouse.column, mouse.row));
                        app.mouse_selection.is_dragging = true;

                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            let buf_pos = buf.screen_to_buffer_pos(
                                rel_col,
                                rel_row,
                                term_inner_h,
                                term_inner_w,
                            );
                            match click_count {
                                1 => buf.start_selection_at(buf_pos),
                                2 => buf.select_word_at(buf_pos),
                                3 => buf.select_line_at(buf_pos.row),
                                _ => buf.start_selection_at(buf_pos),
                            }
                        }
                    }
                }
            }
        }
        crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
            if app.active_modal.is_none() && app.mouse_selection.is_dragging {
                app.mouse_selection.drag_pos = Some((mouse.column, mouse.row));
                if let Some(ref detail_id) = app.session_detail_id.clone() {
                    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                    let (header_h, footer_h) = if rows < 3 {
                        (0, 0)
                    } else if rows < 5 {
                        (1, 0)
                    } else {
                        (1, 1)
                    };
                    let term_inner_h = rows.saturating_sub(header_h + footer_h) as usize;
                    let term_inner_w = cols as usize;

                    // Auto-scroll when dragging to or past viewport boundaries
                    if mouse.row <= header_h {
                        let delta = if mouse.row < header_h { 2 } else { 1 };
                        app.scroll_session_terminal_up(&detail_id.0, delta);
                    } else if mouse.row + 1 >= header_h + term_inner_h as u16 {
                        let delta = if mouse.row >= header_h + term_inner_h as u16 { 2 } else { 1 };
                        app.scroll_session_terminal_down(&detail_id.0, delta);
                    }

                    let rel_row = (mouse.row.saturating_sub(header_h) as usize)
                        .min(term_inner_h.saturating_sub(1));
                    let rel_col = (mouse.column as usize).min(term_inner_w.saturating_sub(1));

                    if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                        let buf_pos = buf.screen_to_buffer_pos(
                            rel_col,
                            rel_row,
                            term_inner_h,
                            term_inner_w,
                        );
                        buf.update_selection_cursor(buf_pos);
                    }
                }
            }
        }
        crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
            if app.mouse_selection.is_dragging {
                app.mouse_selection.is_dragging = false;
                app.mouse_selection.drag_pos = None;
                if let Some(ref detail_id) = app.session_detail_id.clone() {
                    let copied_text = if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                        let should_extract = buf.selection.as_ref().map_or(false, |sel| {
                            !sel.is_empty() || app.mouse_selection.click_count > 1
                        });
                        if should_extract {
                            buf.extract_selected_text()
                        } else {
                            buf.clear_selection();
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(text) = copied_text {
                        if !text.is_empty() {
                            crate::clipboard::copy(&text);
                            let line_count = text.lines().count();
                            let char_count = text.len();
                            app.set_status(
                                format!(
                                    "Copied {line_count} line(s), {char_count} char(s) to clipboard"
                                ),
                                StatusType::Success,
                            );
                        }
                    }
                }
            }
        }
        crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Right) => {
            if app.active_modal.is_none() {
                if let Some(ref detail_id) = app.session_detail_id.clone() {
                    let has_selection = app
                        .session_terminal_buffers
                        .get(&detail_id.0)
                        .map(|b| b.is_selecting())
                        .unwrap_or(false);

                    if has_selection {
                        let copied_text = if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            let text = buf.extract_selected_text();
                            buf.clear_selection();
                            text
                        } else {
                            None
                        };

                        if let Some(text) = copied_text {
                            if !text.is_empty() {
                                crate::clipboard::copy(&text);
                                let line_count = text.lines().count();
                                let char_count = text.len();
                                app.set_status(
                                    format!(
                                        "Copied {line_count} line(s), {char_count} char(s) to clipboard"
                                    ),
                                    StatusType::Success,
                                );
                            }
                        }
                    } else if let Some(pasted) = crate::clipboard::paste() {
                        handle_paste(app, client, &pasted).await?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Auto-scroll terminal buffer and expand visual selection when mouse cursor is held at or past viewport edges.
pub fn handle_mouse_autoscroll(app: &mut App) {
    if !app.mouse_selection.is_dragging {
        return;
    }
    let Some((col, row)) = app.mouse_selection.drag_pos else {
        return;
    };
    let Some(ref detail_id) = app.session_detail_id.clone() else {
        return;
    };

    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let (header_h, footer_h) = if rows < 3 {
        (0, 0)
    } else if rows < 5 {
        (1, 0)
    } else {
        (1, 1)
    };
    let term_inner_h = rows.saturating_sub(header_h + footer_h) as usize;
    let term_inner_w = cols as usize;

    let is_top = row <= header_h;
    let is_bottom = row + 1 >= header_h + term_inner_h as u16;

    if !is_top && !is_bottom {
        return;
    }

    let now = std::time::Instant::now();
    if let Some(last_scroll) = app.mouse_selection.last_autoscroll_time {
        if now.duration_since(last_scroll) < std::time::Duration::from_millis(50) {
            return;
        }
    }
    app.mouse_selection.last_autoscroll_time = Some(now);

    if is_top {
        let delta = if row < header_h { 2 } else { 1 };
        app.scroll_session_terminal_up(&detail_id.0, delta);
        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
            let buf_pos = buf.screen_to_buffer_pos(
                (col as usize).min(term_inner_w.saturating_sub(1)),
                0,
                term_inner_h,
                term_inner_w,
            );
            buf.update_selection_cursor(buf_pos);
        }
    } else if is_bottom {
        let delta = if row >= header_h + term_inner_h as u16 { 2 } else { 1 };
        app.scroll_session_terminal_down(&detail_id.0, delta);
        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
            let buf_pos = buf.screen_to_buffer_pos(
                (col as usize).min(term_inner_w.saturating_sub(1)),
                term_inner_h.saturating_sub(1),
                term_inner_h,
                term_inner_w,
            );
            buf.update_selection_cursor(buf_pos);
        }
    }
}

/// Handle pasted text received either via bracketed paste or explicit clipboard paste.
pub async fn handle_paste(app: &mut App, client: &ApiClient, text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }

    // 1. If a modal is open, route paste to the active text field of the modal.
    if let Some(modal) = app.active_modal.take() {
        match modal {
            Modal::Steer {
                session_id,
                mut input,
            } => {
                input.push_str(text);
                app.active_modal = Some(Modal::Steer { session_id, input });
            }
            Modal::Reply {
                interaction_id,
                mut input,
            } => {
                input.push_str(text);
                app.active_modal = Some(Modal::Reply {
                    interaction_id,
                    input,
                });
            }
            Modal::StartSession(form) => {
                crate::launch::handle_paste(app, form, text);
            }
            Modal::AgyAddAccount { mut label, step } => match step {
                AgyAddStep::Name { .. } => {
                    let cleaned: String =
                        text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                    label.push_str(&cleaned);
                    app.active_modal = Some(Modal::AgyAddAccount {
                        label,
                        step: AgyAddStep::Name { error: None },
                    });
                }
                AgyAddStep::Link {
                    url,
                    deadline,
                    paste: _,
                    notice: _,
                } => {
                    let paste = text.trim().to_string();
                    let sent = app
                        .agy_login_paste
                        .as_ref()
                        .is_some_and(|tx| tx.try_send(paste.clone()).is_ok());
                    let notice = Some(if sent {
                        "Verifying…".into()
                    } else {
                        "The login is no longer active.".into()
                    });
                    app.active_modal = Some(Modal::AgyAddAccount {
                        label,
                        step: AgyAddStep::Link {
                            url,
                            deadline,
                            paste,
                            notice,
                        },
                    });
                }
                other => {
                    app.active_modal = Some(Modal::AgyAddAccount { label, step: other });
                }
            },
            Modal::AddAccount {
                mut label,
                provider,
                auth_method,
                mut token,
                active_field,
            } => {
                let cleaned: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                match active_field {
                    0 => label.push_str(&cleaned),
                    3 => token.push_str(&cleaned),
                    _ => {}
                }
                app.active_modal = Some(Modal::AddAccount {
                    label,
                    provider,
                    auth_method,
                    token,
                    active_field,
                });
            }
            Modal::NewSession {
                account_index,
                mut session_name,
                active_field,
            } => {
                if active_field == 1 {
                    let cleaned: String =
                        text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                    session_name.push_str(&cleaned);
                }
                app.active_modal = Some(Modal::NewSession {
                    account_index,
                    session_name,
                    active_field,
                });
            }
            Modal::FilterActivity { mut input } => {
                let cleaned: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                input.push_str(&cleaned);
                app.active_modal = Some(Modal::FilterActivity { input });
            }
            Modal::RegisterProject {
                mut name,
                mut repo_path,
                policy_index,
                active_field,
                error: _,
            } => {
                let cleaned: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                match active_field {
                    0 => name.push_str(&cleaned),
                    1 => repo_path.push_str(&cleaned),
                    _ => {}
                }
                app.active_modal = Some(Modal::RegisterProject {
                    name,
                    repo_path,
                    policy_index,
                    active_field,
                    error: None,
                });
            }
            Modal::SetDefaultWorkingDir {
                mut input,
                error: _,
            } => {
                let cleaned: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                input.push_str(&cleaned);
                app.active_modal = Some(Modal::SetDefaultWorkingDir { input, error: None });
            }
            other => {
                // Non-text input modals ignore paste safely
                app.active_modal = Some(other);
            }
        }
        return Ok(());
    }

    // 2. If in session detail view, route paste to the agent PTY stdin (or search query if searching).
    if let Some(detail_id) = app.session_detail_id.clone() {
        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
            if buf.search.active && buf.search.editing {
                let clean = text.replace(['\r', '\n'], "");
                for c in clean.chars() {
                    buf.push_search_char(c);
                }
                return Ok(());
            }
        }

        let bracketed = app
            .session_terminal_buffers
            .get(&detail_id.0)
            .map(|b| b.bracketed_paste_enabled())
            .unwrap_or(false);

        if !app.session_in_alt_screen(&detail_id.0) {
            let session_key = detail_id.0.clone();
            app.set_session_completion_dismissed(&session_key, false);
            let buf = app
                .session_prompt_buffers
                .entry(session_key)
                .or_default();
            buf.push_str(text);
        }

        if bracketed {
            let wrapped = format!("\x1b[200~{}\x1b[201~", text);
            let _ = client.send_input(&detail_id, &wrapped).await;
        } else {
            let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
            let _ = client.send_input(&detail_id, &normalized).await;
        }

        app.scroll_session_terminal_bottom(&detail_id.0);
        return Ok(());
    }

    // 3. On main views (tabs), ignore pasted text so hotkeys (q, d, r, n, etc.) are NOT triggered.
    Ok(())
}

/// Handle a key event received from the terminal.
pub async fn handle_key(app: &mut App, client: &ApiClient, key: KeyEvent) -> Result<()> {
    // ── 1. If a Modal dialog is open, route keystrokes to it ────────────────
    if let Some(modal) = app.active_modal.take() {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')) {
                if matches!(
                    modal,
                    Modal::AgyAddAccount {
                        step: AgyAddStep::Waiting { .. } | AgyAddStep::Link { .. },
                        ..
                    }
                ) {
                    cancel_agy_login(app);
                    close_add_account(app);
                } else {
                    app.active_modal = None;
                }
                return Ok(());
            }
            if matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')) {
                if let Some(pasted) = crate::clipboard::paste() {
                    app.active_modal = Some(modal);
                    handle_paste(app, client, &pasted).await?;
                    return Ok(());
                }
            }
        }
        match modal {
            Modal::Steer {
                session_id,
                mut input,
            } => match key.code {
                KeyCode::Esc => {
                    app.prompt_history_for_session_mut(&session_id.0)
                        .reset_nav();
                    app.active_modal = None;
                }
                KeyCode::Enter => {
                    app.active_modal = None;
                    if !input.trim().is_empty() {
                        app.prompt_history_for_session_mut(&session_id.0)
                            .record_submission(&input);
                        match client.steer_session(&session_id, &input).await {
                            Ok(()) => app.set_status(
                                format!("Steered session {}", session_id.0),
                                StatusType::Success,
                            ),
                            Err(e) => {
                                app.set_status(format!("Failed to steer: {e}"), StatusType::Error)
                            }
                        }
                    } else {
                        app.prompt_history_for_session_mut(&session_id.0)
                            .reset_nav();
                    }
                }
                KeyCode::Up => {
                    if let Some(entry) = app
                        .prompt_history_for_session_mut(&session_id.0)
                        .navigate_up(&input)
                    {
                        input = entry.to_string();
                    }
                    app.active_modal = Some(Modal::Steer { session_id, input });
                }
                KeyCode::Down => {
                    if let Some(entry) = app
                        .prompt_history_for_session_mut(&session_id.0)
                        .navigate_down()
                    {
                        input = entry.to_string();
                    }
                    app.active_modal = Some(Modal::Steer { session_id, input });
                }
                KeyCode::Backspace => {
                    input.pop();
                    app.active_modal = Some(Modal::Steer { session_id, input });
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    app.active_modal = Some(Modal::Steer { session_id, input });
                }
                _ => {
                    app.active_modal = Some(Modal::Steer { session_id, input });
                }
            },

            Modal::Reply {
                interaction_id,
                mut input,
            } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Enter => {
                    app.active_modal = None;
                    match client
                        .resolve_interaction(
                            &interaction_id,
                            Some(PolicyDecision::Allow),
                            if input.trim().is_empty() {
                                None
                            } else {
                                Some(&input)
                            },
                        )
                        .await
                    {
                        Ok(_) => {
                            app.set_status(
                                format!("Replied to interaction {}", interaction_id.0),
                                StatusType::Success,
                            );
                            refresh_data(app, client).await;
                        }
                        Err(e) => {
                            app.set_status(format!("Failed to reply: {e}"), StatusType::Error)
                        }
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                    app.active_modal = Some(Modal::Reply {
                        interaction_id,
                        input,
                    });
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    app.active_modal = Some(Modal::Reply {
                        interaction_id,
                        input,
                    });
                }
                _ => {
                    app.active_modal = Some(Modal::Reply {
                        interaction_id,
                        input,
                    });
                }
            },

            Modal::ConfirmStop { session_id } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    app.active_modal = None;
                    match client.stop_session(&session_id, None).await {
                        Ok(()) => {
                            app.close_session_detail();
                            app.set_status(
                                format!("Stopped session {}", session_id.0),
                                StatusType::Success,
                            );
                            refresh_data(app, client).await;
                        }
                        Err(e) => app
                            .set_status(format!("Failed to stop session: {e}"), StatusType::Error),
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    app.active_modal = None;
                }
                _ => {
                    app.active_modal = Some(Modal::ConfirmStop { session_id });
                }
            },

            Modal::StartSession(form) => crate::launch::handle_key(app, client, form, key).await,

            Modal::AgyAddAccount { mut label, step } => match step {
                AgyAddStep::Name { .. } => match key.code {
                    KeyCode::Esc => close_add_account(app),
                    KeyCode::Char('o') | KeyCode::Char('O')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider: "claude".into(),
                            auth_method: 1,
                            token: String::new(),
                            active_field: 0,
                        });
                    }
                    KeyCode::Enter => {
                        let name = label.trim().to_string();
                        let error = if name.is_empty() {
                            Some("Account name cannot be empty.".to_string())
                        } else if app
                            .accounts
                            .iter()
                            .any(|a| a.label.eq_ignore_ascii_case(&name))
                        {
                            Some(format!("You already have an account named \"{name}\"."))
                        } else {
                            None
                        };
                        let step = if error.is_some() {
                            AgyAddStep::Name { error }
                        } else {
                            AgyAddStep::Method
                        };
                        app.active_modal = Some(Modal::AgyAddAccount { label: name, step });
                    }
                    KeyCode::Backspace => {
                        label.pop();
                        app.active_modal = Some(Modal::AgyAddAccount {
                            label,
                            step: AgyAddStep::Name { error: None },
                        });
                    }
                    KeyCode::Char(c) => {
                        label.push(c);
                        app.active_modal = Some(Modal::AgyAddAccount {
                            label,
                            step: AgyAddStep::Name { error: None },
                        });
                    }
                    _ => {
                        app.active_modal = Some(Modal::AgyAddAccount {
                            label,
                            step: AgyAddStep::Name { error: None },
                        })
                    }
                },
                AgyAddStep::Method => match key.code {
                    KeyCode::Up | KeyCode::Down => {
                        app.agy_add_method = if key.code == KeyCode::Down {
                            (app.agy_add_method + 1) % 3
                        } else {
                            (app.agy_add_method + 2) % 3
                        };
                        app.active_modal = Some(Modal::AgyAddAccount {
                            label,
                            step: AgyAddStep::Method,
                        });
                    }
                    KeyCode::Enter => match app.agy_add_method {
                        1 => start_agy_login(app, client.clone(), label, true),
                        2 => import_agy_login(app, client, label).await,
                        _ => start_agy_browser_login(app, client.clone(), label),
                    },
                    KeyCode::Char('l') | KeyCode::Char('L') => {
                        start_agy_login(app, client.clone(), label, true)
                    }
                    KeyCode::Char('i') | KeyCode::Char('I') => {
                        import_agy_login(app, client, label).await
                    }
                    KeyCode::Esc => close_add_account(app),
                    _ => {
                        app.active_modal = Some(Modal::AgyAddAccount {
                            label,
                            step: AgyAddStep::Method,
                        })
                    }
                },
                AgyAddStep::Waiting { .. } | AgyAddStep::Link { .. }
                    if key.code == KeyCode::Esc =>
                {
                    cancel_agy_login(app);
                    close_add_account(app);
                    app.set_status(
                        "Antigravity login cancelled. No credential was saved.",
                        StatusType::Info,
                    );
                }
                AgyAddStep::Link {
                    url,
                    deadline,
                    mut paste,
                    notice,
                } => {
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    let mut notice = notice;
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') if ctrl => {
                            notice = Some(copy_notice(&url))
                        }
                        KeyCode::Char('c') | KeyCode::Char('C') if paste.is_empty() && !ctrl => {
                            notice = Some(copy_notice(&url))
                        }
                        KeyCode::Char(c) if !ctrl => paste.push(c),
                        KeyCode::Backspace => {
                            paste.pop();
                        }
                        KeyCode::Enter if !paste.trim().is_empty() => {
                            let sent = app
                                .agy_login_paste
                                .as_ref()
                                .is_some_and(|tx| tx.try_send(std::mem::take(&mut paste)).is_ok());
                            notice = Some(if sent {
                                "Verifying…".into()
                            } else {
                                "The login is no longer active.".into()
                            });
                        }
                        _ => {}
                    }
                    app.active_modal = Some(Modal::AgyAddAccount {
                        label,
                        step: AgyAddStep::Link {
                            url,
                            deadline,
                            paste,
                            notice,
                        },
                    });
                }
                AgyAddStep::Failed { .. }
                    if matches!(key.code, KeyCode::Char('r') | KeyCode::Char('R')) =>
                {
                    let link = app.agy_add_method == 1;
                    start_agy_login(app, client.clone(), label, link);
                }
                AgyAddStep::Failed { .. } if key.code == KeyCode::Esc => close_add_account(app),
                AgyAddStep::Success { account_id, .. }
                    if matches!(key.code, KeyCode::Enter | KeyCode::Esc) =>
                {
                    match app.resume_start_session.take() {
                        Some(mut form) => {
                            let accts = crate::launch::agy_accounts(app);
                            form.account_index = accts
                                .iter()
                                .position(|a| a.id == account_id)
                                .unwrap_or(form.account_index);
                            form.error = None;
                            crate::launch::open_form(app, form);
                        }
                        None => {
                            app.active_modal = None;
                            app.set_tab(Tab::Accounts);
                        }
                    }
                }
                step => app.active_modal = Some(Modal::AgyAddAccount { label, step }),
            },

            Modal::ConfirmRemoveAccount {
                account_id,
                label,
                provider,
                active_sessions,
            } => match key.code {
                KeyCode::Enter if active_sessions == 0 => {
                    app.active_modal = None;
                    match client.remove_account(&account_id).await {
                        Ok(errors) if errors.is_empty() => {
                            app.set_status(
                                format!("✓ Account \"{label}\" removed"),
                                StatusType::Success,
                            );
                        }
                        Ok(errors) => {
                            app.set_status(
                                format!("Account \"{label}\" removed, but cleanup failed: {} — please delete it to finish cleanup.", errors.join("; ")),
                                StatusType::Warning,
                            );
                        }
                        Err(e) => app.set_status(
                            one_line(&format!("Failed to remove account: {e}")),
                            StatusType::Error,
                        ),
                    }
                    refresh_data(app, client).await;
                    if app.selected_account >= app.accounts.len() {
                        app.selected_account = app.accounts.len().saturating_sub(1);
                    }
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {
                    app.active_modal = None;
                }
                _ => {
                    app.active_modal = Some(Modal::ConfirmRemoveAccount {
                        account_id,
                        label,
                        provider,
                        active_sessions,
                    });
                }
            },

            Modal::ConfirmRemoveSession { session_id } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    app.active_modal = None;
                    match client.remove_session(&session_id).await {
                        Ok(()) => {
                            app.remove_session(&session_id.0);
                            app.set_status(
                                format!("Removed session {}", session_id.0),
                                StatusType::Success,
                            );
                            refresh_data(app, client).await;
                        }
                        Err(e) => app.set_status(
                            format!("Failed to remove session: {e}"),
                            StatusType::Error,
                        ),
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    app.active_modal = None;
                }
                _ => {
                    app.active_modal = Some(Modal::ConfirmRemoveSession { session_id });
                }
            },

            Modal::FilterActivity { mut input } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Enter => {
                    app.active_modal = None;
                    if input.trim().is_empty() {
                        app.activity_filter = None;
                    } else {
                        app.activity_filter = Some(input);
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                    app.active_modal = Some(Modal::FilterActivity { input });
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    app.active_modal = Some(Modal::FilterActivity { input });
                }
                _ => {
                    app.active_modal = Some(Modal::FilterActivity { input });
                }
            },

            Modal::Help => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                    app.active_modal = None;
                }
                _ => {
                    app.active_modal = Some(Modal::Help);
                }
            },

            Modal::SwitchAccount {
                session_id,
                agent_type,
                current_account_id,
                current_account_label,
                options,
                mut selected_index,
                step,
            } => match step {
                crate::app::SwitchModalStep::SelectAccount => match key.code {
                    KeyCode::Esc => {
                        app.active_modal = None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if selected_index > 0 {
                            selected_index -= 1;
                        }
                        app.active_modal = Some(Modal::SwitchAccount {
                            session_id,
                            agent_type,
                            current_account_id,
                            current_account_label,
                            options,
                            selected_index,
                            step: crate::app::SwitchModalStep::SelectAccount,
                        });
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if !options.is_empty() && selected_index + 1 < options.len() {
                            selected_index += 1;
                        }
                        app.active_modal = Some(Modal::SwitchAccount {
                            session_id,
                            agent_type,
                            current_account_id,
                            current_account_label,
                            options,
                            selected_index,
                            step: crate::app::SwitchModalStep::SelectAccount,
                        });
                    }
                    KeyCode::Enter => {
                        if let Some(chosen) = options.get(selected_index) {
                            let chosen_usable = chosen.usable;
                            let chosen_reason = chosen.reason.clone();
                            let chosen_label = chosen.label.clone();
                            let chosen_account_id = chosen.account_id.clone();

                            if !chosen_usable {
                                app.set_status(
                                    format!(
                                        "Cannot switch to {}: {}",
                                        chosen_label,
                                        chosen_reason.as_deref().unwrap_or("account unavailable")
                                    ),
                                    StatusType::Warning,
                                );
                                app.active_modal = Some(Modal::SwitchAccount {
                                    session_id,
                                    agent_type,
                                    current_account_id,
                                    current_account_label,
                                    options,
                                    selected_index,
                                    step: crate::app::SwitchModalStep::SelectAccount,
                                });
                            } else if current_account_id.as_ref() == Some(&chosen_account_id) {
                                app.set_status(
                                    format!("Session is already bound to {}", chosen_label),
                                    StatusType::Info,
                                );
                                app.active_modal = None;
                            } else {
                                app.active_modal = Some(Modal::SwitchAccount {
                                    session_id,
                                    agent_type,
                                    current_account_id,
                                    current_account_label,
                                    options,
                                    selected_index,
                                    step: crate::app::SwitchModalStep::ConfirmRestart {
                                        target_account_id: chosen_account_id,
                                        target_account_label: chosen_label,
                                        reason:
                                            "This provider requires a controlled session restart."
                                                .to_string(),
                                    },
                                });
                            }
                        } else {
                            app.active_modal = None;
                        }
                    }
                    _ => {
                        app.active_modal = Some(Modal::SwitchAccount {
                            session_id,
                            agent_type,
                            current_account_id,
                            current_account_label,
                            options,
                            selected_index,
                            step: crate::app::SwitchModalStep::SelectAccount,
                        });
                    }
                },
                crate::app::SwitchModalStep::ConfirmRestart {
                    target_account_id,
                    target_account_label,
                    reason,
                } => match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        app.active_modal = None;
                        match client.switch_account(&session_id, &target_account_id).await {
                            Ok(active_id) => {
                                app.session_detail_id = Some(active_id.clone());
                                app.set_status(
                                    format!(
                                        "Switched session to {} (active session: {})",
                                        target_account_label, active_id.0
                                    ),
                                    StatusType::Success,
                                );
                                refresh_data(app, client).await;
                            }
                            Err(e) => {
                                app.set_status(format!("Switch failed: {e}"), StatusType::Error);
                            }
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        app.active_modal = Some(Modal::SwitchAccount {
                            session_id,
                            agent_type,
                            current_account_id,
                            current_account_label,
                            options,
                            selected_index,
                            step: crate::app::SwitchModalStep::SelectAccount,
                        });
                    }
                    _ => {
                        app.active_modal = Some(Modal::SwitchAccount {
                            session_id,
                            agent_type,
                            current_account_id,
                            current_account_label,
                            options,
                            selected_index,
                            step: crate::app::SwitchModalStep::ConfirmRestart {
                                target_account_id,
                                target_account_label,
                                reason,
                            },
                        });
                    }
                },
            },
            Modal::AddAccount {
                mut label,
                mut provider,
                mut auth_method,
                mut token,
                mut active_field,
            } => {
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL)
                    && (key.code == KeyCode::Char('o') || key.code == KeyCode::Char('O'))
                {
                    if provider == "agy" {
                        app.active_modal = Some(Modal::AgyAddAccount {
                            label,
                            step: AgyAddStep::Name { error: None },
                        });
                    } else {
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    return Ok(());
                }

                match key.code {
                    KeyCode::Esc => {
                        app.active_modal = None;
                    }
                    KeyCode::Tab | KeyCode::Down => {
                        let max_fields = if auth_method == 2 { 3 } else { 4 };
                        active_field = (active_field + 1) % max_fields;
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        let max_fields = if auth_method == 2 { 3 } else { 4 };
                        active_field = if active_field == 0 {
                            max_fields - 1
                        } else {
                            active_field - 1
                        };
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    KeyCode::Left if active_field == 2 => {
                        auth_method = if auth_method == 0 { 2 } else { auth_method - 1 };
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    KeyCode::Right if active_field == 2 => {
                        auth_method = (auth_method + 1) % 3;
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    KeyCode::Char(' ') if active_field == 1 => {
                        provider = match provider.as_str() {
                            "agy" => "claude".to_string(),
                            "claude" => "pty".to_string(),
                            _ => "agy".to_string(),
                        };
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    KeyCode::Char(' ') if active_field == 2 => {
                        auth_method = (auth_method + 1) % 3;
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    KeyCode::Backspace => {
                        match active_field {
                            0 => {
                                label.pop();
                            }
                            3 => {
                                token.pop();
                            }
                            _ => {}
                        }
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    KeyCode::Char(c) => {
                        match active_field {
                            0 => {
                                label.push(c);
                            }
                            1 => {
                                if c == 'c' || c == 'C' {
                                    provider = "claude".to_string();
                                } else if c == 'a' || c == 'A' {
                                    provider = "agy".to_string();
                                } else if c == 'p' || c == 'P' {
                                    provider = "pty".to_string();
                                }
                            }
                            2 => {
                                if c == '1' {
                                    auth_method = 0;
                                } else if c == '2' {
                                    auth_method = 1;
                                } else if c == '3' {
                                    auth_method = 2;
                                } else if (c == 'o' || c == 'O') && provider == "agy" {
                                    app.active_modal = Some(Modal::AgyAddAccount {
                                        label,
                                        step: AgyAddStep::Name { error: None },
                                    });
                                    return Ok(());
                                }
                            }
                            3 => {
                                token.push(c);
                            }
                            _ => {}
                        }
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field,
                        });
                    }
                    KeyCode::Enter if provider == "agy" => {
                        app.active_modal = Some(Modal::AgyAddAccount {
                            label,
                            step: AgyAddStep::Name { error: None },
                        });
                    }
                    KeyCode::Enter => {
                        if auth_method != 1 {
                            app.set_status(
                                "Only Token / Key is supported for this provider.",
                                StatusType::Error,
                            );
                            app.active_modal = Some(Modal::AddAccount {
                                label,
                                provider,
                                auth_method,
                                token,
                                active_field,
                            });
                            return Ok(());
                        }
                        let cred_id = ulid::Ulid::new().to_string();
                        let token_data = if token.trim().is_empty() {
                            None
                        } else {
                            Some(token.trim().to_string())
                        };
                        let default_label = if provider == "claude" {
                            "Claude Account".to_string()
                        } else {
                            "Personal Google".to_string()
                        };
                        let final_label = if label.trim().is_empty() {
                            default_label
                        } else {
                            label.trim().to_string()
                        };

                        match ac_core::credentials::save_credential(
                            &provider,
                            &cred_id,
                            &final_label,
                            "api_token",
                            token_data,
                        ) {
                            Ok(cred_ref) => {
                                let agent_types: Vec<&str> = match provider.as_str() {
                                    "claude" => vec!["claude", "claude-code"],
                                    _ => vec!["pty", "generic-pty"],
                                };

                                match client
                                    .register_account(
                                        &final_label,
                                        &provider,
                                        &agent_types,
                                        &cred_ref,
                                        2,
                                        &[],
                                    )
                                    .await
                                {
                                    Ok(_) => {
                                        app.active_modal = None;
                                        app.set_status(
                                            format!(
                                                "✓ Account \"{}\" added successfully!",
                                                final_label
                                            ),
                                            StatusType::Success,
                                        );
                                        refresh_data(app, client).await;
                                    }
                                    Err(e) => {
                                        app.set_status(
                                            format!("Failed to register account: {}", e),
                                            StatusType::Error,
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                app.set_status(
                                    format!("Failed to save credential: {}", e),
                                    StatusType::Error,
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }

            Modal::NewSession {
                mut account_index,
                mut session_name,
                mut active_field,
            } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Tab | KeyCode::Down => {
                    active_field = (active_field + 1) % 2;
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        active_field,
                    });
                }
                KeyCode::BackTab | KeyCode::Up => {
                    active_field = if active_field == 0 {
                        1
                    } else {
                        active_field - 1
                    };
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        active_field,
                    });
                }
                KeyCode::Left if active_field == 0 => {
                    if !app.accounts.is_empty() {
                        account_index = if account_index == 0 {
                            app.accounts.len() - 1
                        } else {
                            account_index - 1
                        };
                    }
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        active_field,
                    });
                }
                KeyCode::Right if active_field == 0 => {
                    if !app.accounts.is_empty() {
                        account_index = (account_index + 1) % app.accounts.len();
                    }
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        active_field,
                    });
                }
                KeyCode::Char(' ') if active_field == 0 => {
                    if !app.accounts.is_empty() {
                        account_index = (account_index + 1) % app.accounts.len();
                    }
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        active_field,
                    });
                }
                KeyCode::Backspace => {
                    if active_field == 1 {
                        session_name.pop();
                    }
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        active_field,
                    });
                }
                KeyCode::Char(c) => {
                    if active_field == 1 {
                        session_name.push(c);
                    }
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        active_field,
                    });
                }
                KeyCode::Enter => {
                    let name_trimmed = session_name.trim().to_string();
                    let session_title = if name_trimmed.is_empty() {
                        "session".to_string()
                    } else {
                        name_trimmed
                    };
                    app.active_modal = None;

                    let (agent_type, chosen_acct) = if !app.accounts.is_empty() {
                        let idx = account_index % app.accounts.len();
                        let a = &app.accounts[idx];
                        let agent = match a.provider.to_lowercase().as_str() {
                            p if p.contains("claude") => "claude",
                            p if p.contains("codex") => "codex",
                            _ => "agy",
                        };
                        (agent, Some(a.id.clone()))
                    } else {
                        ("agy", None)
                    };

                    let task_desc = format!("[interactive] {}", session_title);
                    launch_session_and_open(
                        app,
                        client,
                        &task_desc,
                        agent_type,
                        chosen_acct.as_ref(),
                        None,
                    )
                    .await;
                }
                _ => {
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        active_field,
                    });
                }
            },

            Modal::CommandPalette {
                session_id,
                mut selected_index,
            } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    selected_index = if selected_index == 0 {
                        8
                    } else {
                        selected_index - 1
                    };
                    app.active_modal = Some(Modal::CommandPalette {
                        session_id,
                        selected_index,
                    });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    selected_index = (selected_index + 1) % 9;
                    app.active_modal = Some(Modal::CommandPalette {
                        session_id,
                        selected_index,
                    });
                }
                KeyCode::Enter => {
                    app.active_modal = None;
                    match selected_index {
                        0 => {
                            // Pause
                            let sid = session_id.clone();
                            match client.pause_session(&sid).await {
                                Ok(()) => app.set_status(
                                    format!("Paused session {}", sid.0),
                                    StatusType::Success,
                                ),
                                Err(e) => app
                                    .set_status(format!("Failed to pause: {e}"), StatusType::Error),
                            }
                        }
                        1 => {
                            // Resume
                            if let Some(s) =
                                app.sessions.iter().find(|s| s.id == session_id).cloned()
                            {
                                handle_session_resume_or_run(app, client, &s).await;
                            }
                        }
                        2 => {
                            // Switch Account
                            open_switch_account_modal(app, client).await;
                        }
                        3 => {
                            // Stop Session
                            app.active_modal = Some(Modal::ConfirmStop { session_id });
                        }
                        4 => {
                            // Remove Session
                            app.active_modal = Some(Modal::ConfirmRemoveSession { session_id });
                        }
                        5 => {
                            // Session Info
                            app.active_modal = Some(Modal::SessionInfo { session_id });
                        }
                        6 => {
                            // View Events
                            app.close_session_detail();
                            app.set_tab(Tab::Activity);
                        }
                        7 => {
                            // Copy Session ID
                            app.set_status(
                                format!("Session ID: {}", session_id.0),
                                StatusType::Info,
                            );
                        }
                        8 => {
                            // Return to Sessions
                            app.close_session_detail();
                            app.set_tab(Tab::Sessions);
                        }
                        _ => {}
                    }
                }
                _ => {
                    app.active_modal = Some(Modal::CommandPalette {
                        session_id,
                        selected_index,
                    });
                }
            },

            Modal::Approval {
                interaction_id,
                tool_name: _,
                prompt: _,
            } => match key.code {
                KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Enter => {
                    app.active_modal = None;
                    match client
                        .resolve_interaction(&interaction_id, Some(PolicyDecision::Allow), None)
                        .await
                    {
                        Ok(_) => {
                            app.set_status("Approved execution", StatusType::Success);
                            refresh_data(app, client).await;
                        }
                        Err(e) => {
                            app.set_status(format!("Approval failed: {e}"), StatusType::Error)
                        }
                    }
                }
                KeyCode::Char('d') | KeyCode::Char('D') => {
                    app.active_modal = None;
                    match client
                        .resolve_interaction(&interaction_id, Some(PolicyDecision::Deny), None)
                        .await
                    {
                        Ok(_) => {
                            app.set_status("Denied execution", StatusType::Warning);
                            refresh_data(app, client).await;
                        }
                        Err(e) => app.set_status(format!("Denial failed: {e}"), StatusType::Error),
                    }
                }
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                _ => {}
            },

            Modal::SessionInfo { session_id: _ } => match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    app.active_modal = None;
                }
                _ => {}
            },

            Modal::RegisterProject {
                mut name,
                mut repo_path,
                mut policy_index,
                mut active_field,
                error: _,
            } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Tab | KeyCode::Down => {
                    active_field = (active_field + 1) % 5;
                    app.active_modal = Some(Modal::RegisterProject {
                        name,
                        repo_path,
                        policy_index,
                        active_field,
                        error: None,
                    });
                }
                KeyCode::BackTab | KeyCode::Up => {
                    active_field = if active_field == 0 {
                        4
                    } else {
                        active_field - 1
                    };
                    app.active_modal = Some(Modal::RegisterProject {
                        name,
                        repo_path,
                        policy_index,
                        active_field,
                        error: None,
                    });
                }
                KeyCode::Left | KeyCode::Right if active_field == 2 => {
                    policy_index = (policy_index + 1) % 3;
                    app.active_modal = Some(Modal::RegisterProject {
                        name,
                        repo_path,
                        policy_index,
                        active_field,
                        error: None,
                    });
                }
                KeyCode::Left | KeyCode::Right if active_field == 3 || active_field == 4 => {
                    active_field = if active_field == 3 { 4 } else { 3 };
                    app.active_modal = Some(Modal::RegisterProject {
                        name,
                        repo_path,
                        policy_index,
                        active_field,
                        error: None,
                    });
                }
                KeyCode::Backspace => {
                    if active_field == 0 {
                        name.pop();
                    } else if active_field == 1 {
                        repo_path.pop();
                    }
                    app.active_modal = Some(Modal::RegisterProject {
                        name,
                        repo_path,
                        policy_index,
                        active_field,
                        error: None,
                    });
                }
                KeyCode::Char(c) if active_field == 0 => {
                    name.push(c);
                    app.active_modal = Some(Modal::RegisterProject {
                        name,
                        repo_path,
                        policy_index,
                        active_field,
                        error: None,
                    });
                }
                KeyCode::Char(c) if active_field == 1 => {
                    repo_path.push(c);
                    app.active_modal = Some(Modal::RegisterProject {
                        name,
                        repo_path,
                        policy_index,
                        active_field,
                        error: None,
                    });
                }
                KeyCode::Enter => {
                    if active_field == 4 {
                        app.active_modal = None;
                        return Ok(());
                    }
                    let trimmed_name = name.trim().to_string();
                    let trimmed_repo = repo_path.trim().to_string();
                    if trimmed_name.is_empty() {
                        app.active_modal = Some(Modal::RegisterProject {
                            name,
                            repo_path,
                            policy_index,
                            active_field: 0,
                            error: Some("Project name cannot be empty".into()),
                        });
                        return Ok(());
                    }
                    if trimmed_repo.is_empty() {
                        app.active_modal = Some(Modal::RegisterProject {
                            name,
                            repo_path,
                            policy_index,
                            active_field: 1,
                            error: Some("Repository path cannot be empty".into()),
                        });
                        return Ok(());
                    }
                    let policies = [
                        "isolated-worktree",
                        "read-only-workspace",
                        "current-working-dir",
                    ];
                    let pol = policies.get(policy_index).copied();
                    match client
                        .register_project(&trimmed_name, &trimmed_repo, Some("agy"), &[], pol)
                        .await
                    {
                        Ok(pid) => {
                            app.active_modal = None;
                            app.set_status(
                                format!("Project '{}' registered ({})", trimmed_name, pid.0),
                                StatusType::Success,
                            );
                            refresh_data(app, client).await;
                        }
                        Err(e) => {
                            app.active_modal = Some(Modal::RegisterProject {
                                name,
                                repo_path,
                                policy_index,
                                active_field,
                                error: Some(format!("{e}")),
                            });
                        }
                    }
                }
                _ => {}
            },

            Modal::ConfirmRemoveProject { project_id, name } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    app.active_modal = None;
                    match client.remove_project(&project_id).await {
                        Ok(()) => {
                            app.set_status(
                                format!("Project '{name}' removed"),
                                StatusType::Success,
                            );
                            refresh_data(app, client).await;
                        }
                        Err(e) => {
                            app.set_status(
                                format!("Failed to remove project: {e}"),
                                StatusType::Error,
                            );
                        }
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    app.active_modal = None;
                }
                _ => {}
            },

            Modal::SetDefaultWorkingDir {
                mut input,
                error: _,
            } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Backspace => {
                    input.pop();
                    app.active_modal = Some(Modal::SetDefaultWorkingDir { input, error: None });
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    app.active_modal = Some(Modal::SetDefaultWorkingDir { input, error: None });
                }
                KeyCode::Enter => {
                    let trimmed = input.trim();
                    let home = std::env::var("HOME")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|_| std::path::PathBuf::from("/"));
                    let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
                    let resolved = ac_core::agy_launch::resolve_dir_input(trimmed, &cwd, &home);
                    if !resolved.is_dir() {
                        app.active_modal = Some(Modal::SetDefaultWorkingDir {
                            input,
                            error: Some(format!(
                                "Directory does not exist: {}",
                                resolved.display()
                            )),
                        });
                        return Ok(());
                    }
                    app.active_modal = None;
                    app.user_settings.default_working_dir = Some(trimmed.to_string());
                    let _ = app.user_settings.save();
                    app.set_status(
                        format!("Default working directory set to: {trimmed}"),
                        StatusType::Success,
                    );
                }
                _ => {}
            },

            Modal::UrlPicker {
                session_id,
                urls,
                mut selected_index,
            } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if !urls.is_empty() {
                        selected_index = if selected_index == 0 {
                            urls.len() - 1
                        } else {
                            selected_index - 1
                        };
                    }
                    app.active_modal = Some(Modal::UrlPicker {
                        session_id,
                        urls,
                        selected_index,
                    });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !urls.is_empty() {
                        selected_index = (selected_index + 1) % urls.len();
                    }
                    app.active_modal = Some(Modal::UrlPicker {
                        session_id,
                        urls,
                        selected_index,
                    });
                }
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    app.active_modal = None;
                    if let Some(url) = urls.get(selected_index) {
                        crate::clipboard::copy(url);
                        app.set_status(
                            format!("Copied URL to clipboard: {url}"),
                            StatusType::Success,
                        );
                    }
                }
                KeyCode::Enter => {
                    app.active_modal = None;
                    if let Some(url) = urls.get(selected_index) {
                        if ac_core::agy_auth::open_browser(url) {
                            app.set_status(
                                format!("Opened in browser: {url}"),
                                StatusType::Success,
                            );
                        } else {
                            crate::clipboard::copy(url);
                            app.set_status(
                                format!("Could not open browser. Copied URL: {url}"),
                                StatusType::Warning,
                            );
                        }
                    }
                }
                _ => {
                    app.active_modal = Some(Modal::UrlPicker {
                        session_id,
                        urls,
                        selected_index,
                    });
                }
            },
        }
        return Ok(());
    }

    // ── 2. Terminal-First Session Mode (Direct Keyboard Input to PTY) ────────
    if let Some(detail_id) = app.session_detail_id.clone() {
        // If session not found or removed, allow Ctrl+Q or Esc to return
        if app.selected_session_or_detail().is_none()
            && (key.code == KeyCode::Esc
                || (key.modifiers.contains(KeyModifiers::CONTROL)
                    && (key.code == KeyCode::Char('q') || key.code == KeyCode::Char('Q'))))
        {
            app.close_session_detail();
            return Ok(());
        }

        // Detach / Return from session view: Ctrl+Q
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && (key.code == KeyCode::Char('q') || key.code == KeyCode::Char('Q'))
        {
            app.close_session_detail();
            return Ok(());
        }

        let in_scroll_mode = app
            .session_terminal_buffers
            .get(&detail_id.0)
            .map(|b| !b.follow)
            .unwrap_or(false);

        // Command palette shortcut: Ctrl+P or F1
        if (key.modifiers.contains(KeyModifiers::CONTROL)
            && (key.code == KeyCode::Char('p') || key.code == KeyCode::Char('P')))
            || key.code == KeyCode::F(1)
        {
            app.active_modal = Some(Modal::CommandPalette {
                session_id: detail_id,
                selected_index: 0,
            });
            return Ok(());
        }

        // Scrollback Search shortcut: Ctrl+F
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && (key.code == KeyCode::Char('f') || key.code == KeyCode::Char('F'))
        {
            if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                buf.start_search();
            }
            return Ok(());
        }

        // URL Quick-Open Palette shortcut: Ctrl+O
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && (key.code == KeyCode::Char('o') || key.code == KeyCode::Char('O'))
        {
            let urls = if let Some(buf) = app.session_terminal_buffers.get(&detail_id.0) {
                buf.extract_all_urls()
            } else {
                Vec::new()
            };
            if urls.is_empty() {
                app.set_status(
                    "No URLs or hyperlinks found in terminal buffer",
                    StatusType::Warning,
                );
            } else {
                app.active_modal = Some(Modal::UrlPicker {
                    session_id: detail_id,
                    urls,
                    selected_index: 0,
                });
            }
            return Ok(());
        }

        // Copy selection shortcut: Ctrl+Shift+C
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
        {
            if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                if let Some(text) = buf.extract_selected_text() {
                    if !text.is_empty() {
                        crate::clipboard::copy(&text);
                        let line_count = text.lines().count();
                        let char_count = text.len();
                        app.set_status(
                            format!("Copied {line_count} line(s), {char_count} char(s) to clipboard"),
                            StatusType::Success,
                        );
                        return Ok(());
                    }
                }
            }
        }

        // Cancel selection shortcut: Esc (when selection is active and not searching)
        if key.code == KeyCode::Esc {
            let is_selecting = app
                .session_terminal_buffers
                .get(&detail_id.0)
                .map(|b| b.is_selecting())
                .unwrap_or(false);
            if is_selecting {
                if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                    buf.clear_selection();
                }
                app.set_status("Selection cleared", StatusType::Info);
                return Ok(());
            }
        }

        // If interactive scrollback search is currently active:
        let is_search_active = app
            .session_terminal_buffers
            .get(&detail_id.0)
            .map(|b| b.search.active)
            .unwrap_or(false);

        if is_search_active {
            let is_editing = app
                .session_terminal_buffers
                .get(&detail_id.0)
                .map(|b| b.search.editing)
                .unwrap_or(false);

            if is_editing {
                match key.code {
                    KeyCode::Esc => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.cancel_search();
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.search.editing = false;
                        }
                    }
                    KeyCode::Backspace => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.pop_search_char();
                        }
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.push_search_char(c);
                        }
                    }
                    KeyCode::Up => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.prev_search_match(24);
                        }
                    }
                    KeyCode::Down => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.next_search_match(24);
                        }
                    }
                    _ => {}
                }
                return Ok(());
            } else {
                // Search match navigation mode (browsing matches):
                match key.code {
                    KeyCode::Esc => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.cancel_search();
                        }
                        return Ok(());
                    }
                    KeyCode::Char('n') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.next_search_match(24);
                        }
                        return Ok(());
                    }
                    KeyCode::Char('N') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.prev_search_match(24);
                        }
                        return Ok(());
                    }
                    KeyCode::Char('/') | KeyCode::Enter => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.search.editing = true;
                        }
                        return Ok(());
                    }
                    KeyCode::PageUp => {
                        app.scroll_session_terminal_up(&detail_id.0, 10);
                        return Ok(());
                    }
                    KeyCode::PageDown => {
                        app.scroll_session_terminal_down(&detail_id.0, 10);
                        return Ok(());
                    }
                    KeyCode::Home => {
                        app.scroll_session_terminal_top(&detail_id.0);
                        return Ok(());
                    }
                    KeyCode::End => {
                        app.scroll_session_terminal_bottom(&detail_id.0);
                        return Ok(());
                    }
                    _ => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.cancel_search();
                        }
                    }
                }
            }
        }

        // Dedicated scroll navigation (works in both normal and alternate screen):
        // PageUp / PageDown, Shift+Up / Down, Ctrl+Up / Down, Alt+Up / Down
        if matches!(key.code, KeyCode::PageUp)
            || (key.code == KeyCode::Up
                && (key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.modifiers.contains(KeyModifiers::ALT)))
        {
            let delta = if key.code == KeyCode::PageUp { 10 } else { 3 };
            app.scroll_session_terminal_up(&detail_id.0, delta);
            return Ok(());
        }
        if matches!(key.code, KeyCode::PageDown)
            || (key.code == KeyCode::Down
                && (key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::CONTROL)
                    || key.modifiers.contains(KeyModifiers::ALT)))
        {
            let delta = if key.code == KeyCode::PageDown { 10 } else { 3 };
            app.scroll_session_terminal_down(&detail_id.0, delta);
            return Ok(());
        }

        // If in scrollback mode, handle visual selection, scroll keys or snap back on typing
        if in_scroll_mode {
            let is_selecting = app
                .session_terminal_buffers
                .get(&detail_id.0)
                .map(|b| b.is_selecting())
                .unwrap_or(false);

            if is_selecting {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('v') | KeyCode::Char('V') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.clear_selection();
                        }
                        app.set_status("Visual selection cancelled", StatusType::Info);
                        return Ok(());
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            if let Some(text) = buf.extract_selected_text() {
                                let line_count = text.lines().count();
                                let char_count = text.len();
                                crate::clipboard::copy(&text);
                                buf.clear_selection();
                                app.set_status(
                                    format!("Yanked {line_count} line(s), {char_count} char(s) to clipboard"),
                                    StatusType::Success,
                                );
                            } else {
                                buf.clear_selection();
                            }
                        }
                        return Ok(());
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.move_selection_cursor(-1, 0);
                        }
                        return Ok(());
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.move_selection_cursor(1, 0);
                        }
                        return Ok(());
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.move_selection_cursor(0, -1);
                        }
                        return Ok(());
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.move_selection_cursor(0, 1);
                        }
                        return Ok(());
                    }
                    KeyCode::Home | KeyCode::Char('0') | KeyCode::Char('^') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.move_selection_to_line_start();
                        }
                        return Ok(());
                    }
                    KeyCode::End | KeyCode::Char('$') => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.move_selection_to_line_end();
                        }
                        return Ok(());
                    }
                    KeyCode::PageUp => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.move_selection_cursor(-10, 0);
                            buf.scroll_up(10);
                        }
                        return Ok(());
                    }
                    KeyCode::PageDown => {
                        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                            buf.move_selection_cursor(10, 0);
                            buf.scroll_down(10);
                        }
                        return Ok(());
                    }
                    _ => {
                        return Ok(());
                    }
                }
            }

            match key.code {
                KeyCode::Char('v') | KeyCode::Char('V') => {
                    if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                        buf.start_selection();
                    }
                    app.set_status(
                        "Visual mode: move with hjkl/arrows, [y] to yank, [Esc/v] cancel",
                        StatusType::Info,
                    );
                    return Ok(());
                }
                KeyCode::Char('/') => {
                    if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
                        buf.start_search();
                    }
                    return Ok(());
                }
                KeyCode::Char('o') | KeyCode::Char('O') => {
                    let urls = if let Some(buf) = app.session_terminal_buffers.get(&detail_id.0) {
                        buf.extract_all_urls()
                    } else {
                        Vec::new()
                    };
                    if urls.is_empty() {
                        app.set_status(
                            "No URLs or hyperlinks found in terminal buffer",
                            StatusType::Warning,
                        );
                    } else {
                        app.active_modal = Some(Modal::UrlPicker {
                            session_id: detail_id,
                            urls,
                            selected_index: 0,
                        });
                    }
                    return Ok(());
                }
                KeyCode::Home | KeyCode::Char('g') => {
                    app.scroll_session_terminal_top(&detail_id.0);
                    return Ok(());
                }
                KeyCode::End | KeyCode::Char('G') => {
                    app.scroll_session_terminal_bottom(&detail_id.0);
                    return Ok(());
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    app.scroll_session_terminal_bottom(&detail_id.0);
                    return Ok(());
                }
                KeyCode::Up | KeyCode::Down => {
                    // Up and Down are dedicated to prompt history navigation;
                    // snap to bottom so user is at the live input line.
                    app.scroll_session_terminal_bottom(&detail_id.0);
                }
                KeyCode::Char('k') => {
                    app.scroll_session_terminal_up(&detail_id.0, 1);
                    return Ok(());
                }
                KeyCode::Char('j') => {
                    app.scroll_session_terminal_down(&detail_id.0, 1);
                    return Ok(());
                }
                KeyCode::PageUp | KeyCode::Char('u') => {
                    app.scroll_session_terminal_up(&detail_id.0, 10);
                    return Ok(());
                }
                KeyCode::PageDown | KeyCode::Char('d') => {
                    app.scroll_session_terminal_down(&detail_id.0, 10);
                    return Ok(());
                }
                _ => {
                    // Typing any key immediately snaps back to live bottom to interact
                    app.scroll_session_terminal_bottom(&detail_id.0);
                }
            }
        }

        // Control characters: Map Ctrl+A..Z to ASCII control bytes 0x01..0x1A
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if let KeyCode::Char(c) = key.code {
                let byte = match c.to_ascii_lowercase() {
                    'a'..='z' => c.to_ascii_lowercase() as u8 - b'a' + 1,
                    '@' | ' ' => 0x00,
                    '[' => 0x1b,
                    '\\' => 0x1c,
                    ']' => 0x1d,
                    '^' => 0x1e,
                    '_' => 0x1f,
                    _ => return Ok(()),
                };
                if byte == 0x03 {
                    // Ctrl+C: cancel current draft line and reset history navigation
                    let session_key = detail_id.0.clone();
                    app.clear_session_prompt_buffer(&session_key);
                    app.prompt_history_for_session_mut(&session_key).reset_nav();
                } else if byte == 0x0c {
                    // Ctrl+L: clear terminal buffer
                    app.clear_session_terminal(&detail_id.0);
                }
                let bytes = [byte];
                let s = std::str::from_utf8(&bytes).unwrap_or("");
                if !s.is_empty() {
                    let _ = client.send_input(&detail_id, s).await;
                }
                return Ok(());
            }
        }

        // Alt (Meta) combinations
        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char(c) => {
                    let _ = client.send_input(&detail_id, &format!("\x1b{}", c)).await;
                    return Ok(());
                }
                KeyCode::Backspace => {
                    let _ = client.send_input(&detail_id, "\x1b\x7f").await;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Direct keyboard input to agent PTY stdin
        if let Some(buf) = app.session_terminal_buffers.get_mut(&detail_id.0) {
            if buf.is_selecting() {
                buf.clear_selection();
            }
        }

        match key.code {
            KeyCode::Char(c) => {
                if !app.session_in_alt_screen(&detail_id.0) {
                    let session_key = detail_id.0.clone();
                    app.set_session_completion_dismissed(&session_key, false);
                    app.session_prompt_buffers
                        .entry(session_key)
                        .or_default()
                        .push(c);
                }
                let _ = client.send_input(&detail_id, &c.to_string()).await;
            }
            KeyCode::Enter => {
                if !app.session_in_alt_screen(&detail_id.0) {
                    let session_key = detail_id.0.clone();
                    app.set_session_completion_dismissed(&session_key, true);
                    let term_prompt = app.session_terminal_prompt(&session_key);
                    let submitted = if let Some(tp) = term_prompt {
                        if tp.starts_with('/') {
                            app.clear_session_prompt_buffer(&session_key);
                            tp
                        } else {
                            let buf = app
                                .session_prompt_buffers
                                .remove(&session_key)
                                .unwrap_or_default();
                            if !buf.trim().is_empty() {
                                buf
                            } else {
                                tp
                            }
                        }
                    } else {
                        app.session_prompt_buffers
                            .remove(&session_key)
                            .unwrap_or_default()
                    };
                    if !submitted.trim().is_empty() {
                        app.prompt_history_for_session_mut(&session_key)
                            .record_submission(&submitted);
                    } else {
                        app.prompt_history_for_session_mut(&session_key).reset_nav();
                    }
                }
                let _ = client.send_input(&detail_id, "\r").await;
            }
            KeyCode::Backspace => {
                if !app.session_in_alt_screen(&detail_id.0) {
                    let session_key = detail_id.0.clone();
                    app.set_session_completion_dismissed(&session_key, false);
                    if let Some(buf) = app.session_prompt_buffers.get_mut(&session_key) {
                        buf.pop();
                    }
                }
                let _ = client.send_input(&detail_id, "\x7f").await;
            }
            KeyCode::Tab => {
                if !app.session_in_alt_screen(&detail_id.0) {
                    let session_key = detail_id.0.clone();
                    app.set_session_completion_dismissed(&session_key, true);
                }
                let _ = client.send_input(&detail_id, "\t").await;
            }
            KeyCode::BackTab => {
                let _ = client.send_input(&detail_id, "\x1b[Z").await;
            }
            KeyCode::Delete => {
                let _ = client.send_input(&detail_id, "\x1b[3~").await;
            }
            KeyCode::Insert => {
                let _ = client.send_input(&detail_id, "\x1b[2~").await;
            }
            KeyCode::Esc => {
                if !app.session_in_alt_screen(&detail_id.0) {
                    let session_key = detail_id.0.clone();
                    app.set_session_completion_dismissed(&session_key, true);
                    app.prompt_history_for_session_mut(&session_key).reset_nav();
                }
                let _ = client.send_input(&detail_id, "\x1b").await;
            }
            KeyCode::Up
                if !app.session_in_alt_screen(&detail_id.0)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                let session_key = detail_id.0.clone();
                let is_nav = app
                    .prompt_history_for_session(&session_key)
                    .map(|h| h.is_navigating())
                    .unwrap_or(false);
                if !is_nav && app.is_session_completion_open(&session_key) {
                    let _ = client.send_input(&detail_id, "\x1b[A").await;
                } else {
                    let current_buf = app.session_prompt_buffer(&session_key).to_string();
                    let next_prompt = app
                        .prompt_history_for_session_mut(&session_key)
                        .navigate_up(&current_buf)
                        .map(|s| s.to_string());
                    if let Some(new_text) = next_prompt {
                        if new_text != current_buf {
                            let char_count = current_buf.chars().count();
                            let mut seq = String::with_capacity(char_count + new_text.len() + 16);
                            seq.push_str("\x1b[F"); // Move cursor to end of line
                            for _ in 0..char_count {
                                seq.push('\x7f');
                            }
                            let bracketed = app
                                .session_terminal_buffers
                                .get(&detail_id.0)
                                .map(|b| b.bracketed_paste_enabled())
                                .unwrap_or(false);
                            if bracketed && (new_text.contains('\n') || new_text.contains('\r')) {
                                seq.push_str("\x1b[200~");
                                seq.push_str(&new_text);
                                seq.push_str("\x1b[201~");
                            } else {
                                seq.push_str(&new_text);
                            }
                            let _ = client.send_input(&detail_id, &seq).await;
                            app.set_session_prompt_buffer(&session_key, new_text);
                        }
                    }
                }
            }
            KeyCode::Down
                if !app.session_in_alt_screen(&detail_id.0)
                    && !key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                let session_key = detail_id.0.clone();
                let is_nav = app
                    .prompt_history_for_session(&session_key)
                    .map(|h| h.is_navigating())
                    .unwrap_or(false);
                if !is_nav && app.is_session_completion_open(&session_key) {
                    let _ = client.send_input(&detail_id, "\x1b[B").await;
                } else {
                    let current_buf = app.session_prompt_buffer(&session_key).to_string();
                    let next_prompt = app
                        .prompt_history_for_session_mut(&session_key)
                        .navigate_down()
                        .map(|s| s.to_string());
                    if let Some(new_text) = next_prompt {
                        if new_text != current_buf {
                            let char_count = current_buf.chars().count();
                            let mut seq = String::with_capacity(char_count + new_text.len() + 16);
                            seq.push_str("\x1b[F"); // Move cursor to end of line
                            for _ in 0..char_count {
                                seq.push('\x7f');
                            }
                            let bracketed = app
                                .session_terminal_buffers
                                .get(&detail_id.0)
                                .map(|b| b.bracketed_paste_enabled())
                                .unwrap_or(false);
                            if bracketed && (new_text.contains('\n') || new_text.contains('\r')) {
                                seq.push_str("\x1b[200~");
                                seq.push_str(&new_text);
                                seq.push_str("\x1b[201~");
                            } else {
                                seq.push_str(&new_text);
                            }
                            let _ = client.send_input(&detail_id, &seq).await;
                            app.set_session_prompt_buffer(&session_key, new_text);
                        }
                    }
                }
            }
            KeyCode::Up => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    let _ = client.send_input(&detail_id, "\x1b[1;2A").await;
                } else {
                    let _ = client.send_input(&detail_id, "\x1b[A").await;
                }
            }
            KeyCode::Down => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    let _ = client.send_input(&detail_id, "\x1b[1;2B").await;
                } else {
                    let _ = client.send_input(&detail_id, "\x1b[B").await;
                }
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    let _ = client.send_input(&detail_id, "\x1b[1;2C").await;
                } else {
                    let _ = client.send_input(&detail_id, "\x1b[C").await;
                }
            }
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    let _ = client.send_input(&detail_id, "\x1b[1;2D").await;
                } else {
                    let _ = client.send_input(&detail_id, "\x1b[D").await;
                }
            }
            KeyCode::Home => {
                let _ = client.send_input(&detail_id, "\x1b[H").await;
            }
            KeyCode::End => {
                let _ = client.send_input(&detail_id, "\x1b[F").await;
            }
            _ => {}
        }

        return Ok(());
    }

    // ── 3. Global Keybindings ───────────────────────────────────────────────
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        app.should_quit = true;
        return Ok(());
    }

    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            return Ok(());
        }
        KeyCode::PageUp => {
            app.page_up(10);
            return Ok(());
        }
        KeyCode::PageDown => {
            app.page_down(10);
            return Ok(());
        }
        KeyCode::Home => {
            app.first_row();
            return Ok(());
        }
        KeyCode::End => {
            app.last_row();
            return Ok(());
        }
        KeyCode::Char('?') => {
            app.active_modal = Some(Modal::Help);
            return Ok(());
        }
        KeyCode::Char('r') => {
            refresh_data(app, client).await;
            app.set_status("Refreshed state from daemon", StatusType::Info);
            return Ok(());
        }
        KeyCode::Tab => {
            app.next_tab();
            return Ok(());
        }
        KeyCode::BackTab => {
            app.prev_tab();
            return Ok(());
        }
        KeyCode::Char('1') => {
            app.set_tab(Tab::Dashboard);
            return Ok(());
        }
        KeyCode::Char('2') => {
            app.set_tab(Tab::Sessions);
            return Ok(());
        }
        KeyCode::Char('3') => {
            app.set_tab(Tab::Accounts);
            return Ok(());
        }
        KeyCode::Char('4') => {
            app.set_tab(Tab::Activity);
            return Ok(());
        }
        KeyCode::Char('5') => {
            app.set_tab(Tab::Agents);
            return Ok(());
        }
        KeyCode::Char('6') => {
            app.set_tab(Tab::Settings);
            return Ok(());
        }
        KeyCode::Left => {
            if app.current_tab == Tab::Dashboard {
                app.prev_detail_subtab();
                return Ok(());
            }
        }
        KeyCode::Right => {
            if app.current_tab == Tab::Dashboard {
                app.next_detail_subtab();
                return Ok(());
            }
        }
        KeyCode::Char('g') => {
            if app.current_tab == Tab::Dashboard {
                crate::launch::open_new_session(app);
                return Ok(());
            }
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            if app.current_tab == Tab::Dashboard {
                app.set_tab(Tab::Activity);
                return Ok(());
            }
        }
        _ => {}
    }

    // ── 4. View-Specific Keybindings ────────────────────────────────────────

    // Otherwise handle by active tab
    match app.current_tab {
        Tab::Dashboard => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Enter | KeyCode::Char('t') => {
                app.open_selected_session_detail();
                if let Some(ref sid) = app.session_detail_id.clone() {
                    load_session_history_if_needed(app, client, sid).await;
                    if let Ok((cols, rows)) = crossterm::terminal::size() {
                        let _ = client
                            .resize_session(sid, rows.saturating_sub(2), cols)
                            .await;
                    }
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                open_agy_add_account(app);
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                app.set_tab(Tab::Sessions);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                crate::launch::open_new_session(app);
            }
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                if let Some(s) = app.selected_session() {
                    app.active_modal = Some(Modal::ConfirmRemoveSession {
                        session_id: s.id.clone(),
                    });
                }
            }
            _ => {}
        },

        Tab::Sessions => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Enter | KeyCode::Char('t') => {
                app.open_selected_session_detail();
                if let Some(ref sid) = app.session_detail_id.clone() {
                    load_session_history_if_needed(app, client, sid).await;
                    if let Ok((cols, rows)) = crossterm::terminal::size() {
                        let _ = client
                            .resize_session(sid, rows.saturating_sub(2), cols)
                            .await;
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                crate::launch::open_new_session(app);
            }
            KeyCode::Char('s') => {
                if let Some(s) = app.selected_session() {
                    app.active_modal = Some(Modal::Steer {
                        session_id: s.id.clone(),
                        input: String::new(),
                    });
                }
            }
            KeyCode::Char('p') => {
                if let Some(s) = app.selected_session() {
                    let sid = s.id.clone();
                    match client.pause_session(&sid).await {
                        Ok(()) => {
                            app.set_status(format!("Paused session {}", sid.0), StatusType::Success)
                        }
                        Err(e) => {
                            app.set_status(format!("Failed to pause: {e}"), StatusType::Error)
                        }
                    }
                }
            }
            KeyCode::Char(' ') | KeyCode::Char('u') => {
                if let Some(s) = app.selected_session().cloned() {
                    handle_session_resume_or_run(app, client, &s).await;
                }
            }
            KeyCode::Char('x') => {
                if let Some(s) = app.selected_session() {
                    app.active_modal = Some(Modal::ConfirmStop {
                        session_id: s.id.clone(),
                    });
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                if let Some(s) = app.selected_session() {
                    app.active_modal = Some(Modal::ConfirmRemoveSession {
                        session_id: s.id.clone(),
                    });
                }
            }
            KeyCode::Char('w') => {
                open_switch_account_modal(app, client).await;
            }
            _ => {}
        },

        Tab::Accounts => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Char('a') | KeyCode::Char('A') => {
                open_agy_add_account(app);
            }
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                if let Some(acct) = app.selected_account().cloned() {
                    let active = active_sessions_for_account(app, &acct.id)
                        .max(acct.active_session_count as usize);
                    app.active_modal = Some(Modal::ConfirmRemoveAccount {
                        account_id: acct.id,
                        label: acct.label,
                        provider: acct.provider,
                        active_sessions: active,
                    });
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if let Some(label) = app.selected_account().map(|a| a.label.clone()) {
                    app.user_settings.default_account = Some(label.clone());
                    let _ = app.user_settings.save();
                    app.set_status(
                        format!("Default account set to: {}", label),
                        StatusType::Success,
                    );
                }
            }
            _ => {}
        },

        Tab::Activity => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Char('/') => {
                app.active_modal = Some(Modal::FilterActivity {
                    input: app.activity_filter.clone().unwrap_or_default(),
                });
            }
            KeyCode::Char('c') => {
                app.activity_filter = None;
                app.set_status("Cleared activity filter", StatusType::Info);
            }
            _ => {}
        },

        Tab::Agents => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Enter => {
                crate::launch::open_new_session(app);
            }
            _ => {}
        },

        Tab::Settings => {
            if !app.settings_focus_panel {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
                    KeyCode::Down | KeyCode::Char('j') => app.next_row(),
                    KeyCode::Enter | KeyCode::Right | KeyCode::Tab => {
                        app.settings_focus_panel = true;
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Esc | KeyCode::BackTab => {
                        app.settings_focus_panel = false;
                    }
                    _ => match app.current_settings_section() {
                        crate::app::SettingsSection::General => match key.code {
                            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
                            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
                            KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Char(' ')
                            | KeyCode::Enter => {
                                let fwd = key.code != KeyCode::Left;
                                match app.settings_general_item {
                                    0 => {
                                        let agents = ["Antigravity", "Claude Code", "Generic PTY"];
                                        let cur = agents
                                            .iter()
                                            .position(|a| *a == app.user_settings.default_agent)
                                            .unwrap_or(0);
                                        let next = if fwd {
                                            (cur + 1) % agents.len()
                                        } else {
                                            (cur + agents.len() - 1) % agents.len()
                                        };
                                        app.user_settings.default_agent = agents[next].to_string();
                                        let _ = app.user_settings.save();
                                        app.set_status(
                                            format!(
                                                "Default agent set to {}",
                                                app.user_settings.default_agent
                                            ),
                                            StatusType::Success,
                                        );
                                    }
                                    1 => {
                                        let mut labels = vec![None];
                                        labels.extend(
                                            app.accounts.iter().map(|a| Some(a.label.clone())),
                                        );
                                        let cur = labels
                                            .iter()
                                            .position(|l| *l == app.user_settings.default_account)
                                            .unwrap_or(0);
                                        let next = if fwd {
                                            (cur + 1) % labels.len()
                                        } else {
                                            (cur + labels.len() - 1) % labels.len()
                                        };
                                        app.user_settings.default_account = labels[next].clone();
                                        let _ = app.user_settings.save();
                                        let name = app
                                            .user_settings
                                            .default_account
                                            .as_deref()
                                            .unwrap_or("None");
                                        app.set_status(
                                            format!("Default account set to {name}"),
                                            StatusType::Success,
                                        );
                                    }
                                    2 => {
                                        app.active_modal = Some(Modal::SetDefaultWorkingDir {
                                            input: app
                                                .user_settings
                                                .default_working_dir
                                                .clone()
                                                .unwrap_or_default(),
                                            error: None,
                                        });
                                    }
                                    3 => {
                                        let modes = ac_core::agy_launch::AgyExecutionMode::ALL;
                                        let cur = modes
                                            .iter()
                                            .position(|m| {
                                                *m == app.user_settings.default_execution_mode
                                            })
                                            .unwrap_or(0);
                                        let next = if fwd {
                                            (cur + 1) % modes.len()
                                        } else {
                                            (cur + modes.len() - 1) % modes.len()
                                        };
                                        app.user_settings.default_execution_mode = modes[next];
                                        let _ = app.user_settings.save();
                                        app.set_status(
                                            format!(
                                                "Default execution mode: {}",
                                                app.user_settings.default_execution_mode.label()
                                            ),
                                            StatusType::Success,
                                        );
                                    }
                                    4 => {
                                        let perms = ac_core::agy_launch::AgyPermissionMode::ALL;
                                        let cur = perms
                                            .iter()
                                            .position(|p| {
                                                *p == app.user_settings.default_permission_mode
                                            })
                                            .unwrap_or(0);
                                        let next = if fwd {
                                            (cur + 1) % perms.len()
                                        } else {
                                            (cur + perms.len() - 1) % perms.len()
                                        };
                                        app.user_settings.default_permission_mode = perms[next];
                                        let _ = app.user_settings.save();
                                        app.set_status(
                                            format!(
                                                "Default permission mode: {}",
                                                app.user_settings.default_permission_mode.label()
                                            ),
                                            StatusType::Success,
                                        );
                                    }
                                    5 => {
                                        let themes = ["Default", "Dark", "High-Contrast"];
                                        let cur = themes
                                            .iter()
                                            .position(|t| *t == app.user_settings.terminal_theme)
                                            .unwrap_or(0);
                                        let next = if fwd {
                                            (cur + 1) % themes.len()
                                        } else {
                                            (cur + themes.len() - 1) % themes.len()
                                        };
                                        app.user_settings.terminal_theme = themes[next].to_string();
                                        let _ = app.user_settings.save();
                                        app.set_status(
                                            format!(
                                                "Theme set to {}",
                                                app.user_settings.terminal_theme
                                            ),
                                            StatusType::Success,
                                        );
                                    }
                                    _ => {}
                                }
                            }
                            KeyCode::Char('w') | KeyCode::Char('W') => {
                                app.active_modal = Some(Modal::SetDefaultWorkingDir {
                                    input: app
                                        .user_settings
                                        .default_working_dir
                                        .clone()
                                        .unwrap_or_default(),
                                    error: None,
                                });
                            }
                            _ => {}
                        },
                        crate::app::SettingsSection::Accounts => match key.code {
                            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
                            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                open_agy_add_account(app);
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                                if let Some(acct) =
                                    app.accounts.get(app.settings_account_selected).cloned()
                                {
                                    let active = active_sessions_for_account(app, &acct.id)
                                        .max(acct.active_session_count as usize);
                                    app.active_modal = Some(Modal::ConfirmRemoveAccount {
                                        account_id: acct.id,
                                        label: acct.label,
                                        provider: acct.provider,
                                        active_sessions: active,
                                    });
                                }
                            }
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                if let Some(acct) = app.accounts.get(app.settings_account_selected)
                                {
                                    app.user_settings.default_account = Some(acct.label.clone());
                                    let _ = app.user_settings.save();
                                    app.set_status(
                                        format!("Default account set to: {}", acct.label),
                                        StatusType::Success,
                                    );
                                }
                            }
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                refresh_data(app, client).await;
                                app.set_status("Refreshed accounts", StatusType::Info);
                            }
                            _ => {}
                        },
                        crate::app::SettingsSection::Projects => match key.code {
                            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
                            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                app.active_modal = Some(Modal::RegisterProject {
                                    name: String::new(),
                                    repo_path: String::new(),
                                    policy_index: 0,
                                    active_field: 0,
                                    error: None,
                                });
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                                if let Some(p) =
                                    app.projects.get(app.settings_project_selected).cloned()
                                {
                                    app.active_modal = Some(Modal::ConfirmRemoveProject {
                                        project_id: p.id,
                                        name: p.name,
                                    });
                                }
                            }
                            KeyCode::Char('s') | KeyCode::Char('S') => {
                                if let Some(p) = app.projects.get(app.settings_project_selected) {
                                    app.user_settings.default_project = Some(p.name.clone());
                                    let _ = app.user_settings.save();
                                    app.set_status(
                                        format!("Default project set to: {}", p.name),
                                        StatusType::Success,
                                    );
                                }
                            }
                            KeyCode::Char('w') | KeyCode::Char('W') => {
                                app.active_modal = Some(Modal::SetDefaultWorkingDir {
                                    input: app
                                        .user_settings
                                        .default_working_dir
                                        .clone()
                                        .unwrap_or_default(),
                                    error: None,
                                });
                            }
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                refresh_data(app, client).await;
                                app.set_status("Refreshed projects", StatusType::Info);
                            }
                            _ => {}
                        },
                        crate::app::SettingsSection::Agents => match key.code {
                            KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Enter => {
                                let agents = ["Antigravity", "Claude Code", "Generic PTY"];
                                let cur = (app.selected_agent + 1) % agents.len();
                                app.selected_agent = cur;
                                app.user_settings.default_agent = agents[cur].to_string();
                                let _ = app.user_settings.save();
                                app.set_status(
                                    format!("Default agent set to {}", agents[cur]),
                                    StatusType::Success,
                                );
                            }
                            _ => {}
                        },
                        crate::app::SettingsSection::Permissions => match key.code {
                            KeyCode::Char('1') => {
                                let modes = ac_core::agy_launch::AgyExecutionMode::ALL;
                                let cur = modes
                                    .iter()
                                    .position(|m| *m == app.user_settings.default_execution_mode)
                                    .unwrap_or(0);
                                app.user_settings.default_execution_mode =
                                    modes[(cur + 1) % modes.len()];
                                let _ = app.user_settings.save();
                                app.set_status(
                                    format!(
                                        "Default execution mode: {}",
                                        app.user_settings.default_execution_mode.label()
                                    ),
                                    StatusType::Success,
                                );
                            }
                            KeyCode::Char('2') => {
                                let perms = ac_core::agy_launch::AgyPermissionMode::ALL;
                                let cur = perms
                                    .iter()
                                    .position(|p| *p == app.user_settings.default_permission_mode)
                                    .unwrap_or(0);
                                app.user_settings.default_permission_mode =
                                    perms[(cur + 1) % perms.len()];
                                let _ = app.user_settings.save();
                                app.set_status(
                                    format!(
                                        "Default permission mode: {}",
                                        app.user_settings.default_permission_mode.label()
                                    ),
                                    StatusType::Success,
                                );
                            }
                            _ => {}
                        },
                        crate::app::SettingsSection::Authentication => match key.code {
                            KeyCode::Char('b') | KeyCode::Char('B') => {
                                let lbl = format!("Account {}", app.accounts.len() + 1);
                                start_agy_browser_login(app, client.clone(), lbl);
                            }
                            KeyCode::Char('l') | KeyCode::Char('L') => {
                                let lbl = format!("Account {}", app.accounts.len() + 1);
                                start_agy_login(app, client.clone(), lbl, true);
                            }
                            _ => {}
                        },
                        crate::app::SettingsSection::Terminal => match key.code {
                            KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Char(' ')
                            | KeyCode::Enter => {
                                let themes = ["Default", "Dark", "High-Contrast"];
                                let cur = themes
                                    .iter()
                                    .position(|t| *t == app.user_settings.terminal_theme)
                                    .unwrap_or(0);
                                app.user_settings.terminal_theme =
                                    themes[(cur + 1) % themes.len()].to_string();
                                let _ = app.user_settings.save();
                                app.set_status(
                                    format!("Theme set to {}", app.user_settings.terminal_theme),
                                    StatusType::Success,
                                );
                            }
                            _ => {}
                        },
                        _ => {}
                    },
                }
            }
        }
    }

    Ok(())
}

/// Refresh all entity collections from the daemon.
pub async fn refresh_data(app: &mut App, client: &ApiClient) {
    if let Ok(sessions) = client.list_sessions().await {
        app.sessions = sessions;
    }
    if let Ok(interactions) = client.list_interactions(None, false).await {
        app.interactions = interactions;
    }
    if let Ok(accounts) = client.list_accounts().await {
        app.accounts = accounts;
    }
    if let Ok(projects) = client.list_projects().await {
        app.projects = projects;
    }
    if let Ok(events) = client.query_events(None, Some(100)).await {
        app.events = events;
    }
    app.clamp_selections();
    app.daemon_connected = client.check_daemon().await;

    // Preload history for the active session detail or currently selected session
    if let Some(sid) = app.session_detail_id.clone() {
        load_session_history_if_needed(app, client, &sid).await;
    } else if let Some(s) = app.selected_session().cloned() {
        load_session_history_if_needed(app, client, &s.id).await;
    }
}

pub async fn load_session_history_if_needed(app: &mut App, client: &ApiClient, session_id: &Id) {
    let needs_load = app
        .session_terminal_buffers
        .get(&session_id.0)
        .map(|b| b.total_lines() == 0)
        .unwrap_or(true);

    if needs_load {
        if let Ok(events) = client.query_events(Some(session_id), Some(2000)).await {
            for event in events {
                app.apply_event(event);
            }
        }
    }
}

async fn open_switch_account_modal(app: &mut App, client: &ApiClient) {
    if let Some(s) = app.selected_session_or_detail().cloned() {
        let agent_type = s.agent_type.clone();
        let session_id = s.id.clone();
        let current_account_id = s.account_id.clone();
        let current_account_label = app.account_label(current_account_id.as_ref());

        match client
            .query_account_availability(Some(&agent_type), &[])
            .await
        {
            Ok(avail_list) => {
                let options: Vec<crate::app::AccountOption> = avail_list
                    .into_iter()
                    .map(|a| crate::app::AccountOption {
                        account_id: a.account_id,
                        label: a.label,
                        provider: a.provider,
                        usable: a.is_available,
                        active_sessions: a.active_session_count as u32,
                        concurrency_cap: a.concurrency_cap,
                        reason: a.reason,
                    })
                    .collect();

                let initial_index = options
                    .iter()
                    .position(|o| Some(&o.account_id) != current_account_id.as_ref() && o.usable)
                    .unwrap_or(0);

                app.active_modal = Some(Modal::SwitchAccount {
                    session_id,
                    agent_type,
                    current_account_id,
                    current_account_label,
                    options,
                    selected_index: initial_index,
                    step: crate::app::SwitchModalStep::SelectAccount,
                });
            }
            Err(e) => {
                app.set_status(format!("Failed to query accounts: {e}"), StatusType::Error);
            }
        }
    }
}

async fn handle_session_resume_or_run(
    app: &mut App,
    client: &ApiClient,
    s: &ac_core::types::AgentSession,
) {
    let sid = s.id.clone();
    match s.state {
        SessionState::Idle => match client.start_session(&sid).await {
            Ok(()) => {
                app.set_status(format!("Started session {}", sid.0), StatusType::Success);
                refresh_data(app, client).await;
            }
            Err(e) => app.set_status(format!("Failed to start: {e}"), StatusType::Error),
        },
        SessionState::Paused => match client.resume_session(&sid).await {
            Ok(()) => {
                app.set_status(format!("Resumed session {}", sid.0), StatusType::Success);
                refresh_data(app, client).await;
            }
            Err(e) => app.set_status(format!("Failed to resume: {e}"), StatusType::Error),
        },
        SessionState::Stopped | SessionState::Failed | SessionState::Crashed => {
            match client
                .create_session(
                    &s.task_description,
                    &s.agent_type,
                    s.project_id.as_ref(),
                    s.account_id.as_ref(),
                )
                .await
            {
                Ok(new_id) => match client.start_session(&new_id).await {
                    Ok(()) => {
                        app.set_status(
                            format!("Re-ran session as {} (started)", new_id.0),
                            StatusType::Success,
                        );
                        refresh_data(app, client).await;
                    }
                    Err(e) => app.set_status(
                        format!("Created session {} but failed to start: {e}", new_id.0),
                        StatusType::Error,
                    ),
                },
                Err(e) => {
                    app.set_status(format!("Failed to re-run session: {e}"), StatusType::Error)
                }
            }
        }
        _ => {
            app.set_status(
                format!("Session is currently {:?}", s.state),
                StatusType::Info,
            );
        }
    }
}
