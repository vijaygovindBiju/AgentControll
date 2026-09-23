//! Keyboard input and async event handling.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use ac_core::types::{PolicyDecision, SessionState};
use crate::{
    app::{App, Modal, StatusType, Tab},
    client::ApiClient,
};

fn open_google_oauth_and_listen(app: &mut App, client: ApiClient) {
    let auth_url = ac_core::credentials::default_google_oauth_url();
    ac_core::credentials::open_browser(&auth_url);
    app.set_status("Opening Google sign-in in browser... Copy code or allow callback.", StatusType::Info);

    tokio::spawn(async move {
        let bind_addr = format!("127.0.0.1:{}", ac_core::credentials::DEFAULT_OAUTH_PORT);
        if let Ok(listener) = tokio::net::TcpListener::bind(&bind_addr).await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok(Ok((mut socket, _))) = tokio::time::timeout(std::time::Duration::from_secs(120), listener.accept()).await {
                let mut buf = [0u8; 2048];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req_str = String::from_utf8_lossy(&buf[..n]);

                let code = ac_core::credentials::extract_code_from_http_request(&req_str)
                    .unwrap_or_else(|| "auth_completed".to_string());

                let html_resp = ac_core::credentials::build_oauth_success_html(&code);
                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html_resp.as_bytes().len(),
                    html_resp
                );
                let _ = socket.write_all(http_resp.as_bytes()).await;
                let _ = socket.flush().await;

                // Also exchange code and register account in the background
                if !code.is_empty() && code != "auth_completed" {
                    let redirect_uri = format!("http://localhost:{}/oauth2callback", ac_core::credentials::DEFAULT_OAUTH_PORT);
                    if let Ok(json_tokens) = ac_core::credentials::exchange_code_for_google_tokens(&code, &redirect_uri).await {
                        let cred_id = ulid::Ulid::new().to_string();
                        let email = json_tokens["id_token"].as_str()
                            .and_then(ac_core::credentials::extract_email_from_jwt)
                            .unwrap_or_else(|| "Personal Google".to_string());
                        let token_str = serde_json::to_string_pretty(&json_tokens).unwrap_or_else(|_| code.clone());
                        if let Ok(cred_ref) = ac_core::credentials::save_credential("agy", &cred_id, &email, "oauth2_token", Some(token_str)) {
                            let _ = client.register_account(&email, "agy", &["agy", "antigravity"], &cred_ref, 2, &["google"]).await;
                        }
                    }
                }
            }
        }
    });
}

/// Handle a key event received from the terminal.
pub async fn handle_key(app: &mut App, client: &ApiClient, key: KeyEvent) -> Result<()> {
    // ── 1. If a Modal dialog is open, route keystrokes to it ────────────────
    if let Some(modal) = app.active_modal.take() {
        match modal {
            Modal::Steer { session_id, mut input } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Enter => {
                    app.active_modal = None;
                    if !input.trim().is_empty() {
                        match client.steer_session(&session_id, &input).await {
                            Ok(()) => app.set_status(format!("Steered session {}", session_id.0), StatusType::Success),
                            Err(e) => app.set_status(format!("Failed to steer: {e}"), StatusType::Error),
                        }
                    }
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

            Modal::Reply { interaction_id, mut input } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Enter => {
                    app.active_modal = None;
                    match client
                        .resolve_interaction(
                            &interaction_id,
                            Some(PolicyDecision::Allow),
                            if input.trim().is_empty() { None } else { Some(&input) },
                        )
                        .await
                    {
                        Ok(_) => {
                            app.set_status(format!("Replied to interaction {}", interaction_id.0), StatusType::Success);
                            refresh_data(app, client).await;
                        }
                        Err(e) => app.set_status(format!("Failed to reply: {e}"), StatusType::Error),
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                    app.active_modal = Some(Modal::Reply { interaction_id, input });
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    app.active_modal = Some(Modal::Reply { interaction_id, input });
                }
                _ => {
                    app.active_modal = Some(Modal::Reply { interaction_id, input });
                }
            },

            Modal::ConfirmStop { session_id } => match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    app.active_modal = None;
                    match client.stop_session(&session_id, None).await {
                        Ok(()) => {
                            app.set_status(format!("Stopped session {}", session_id.0), StatusType::Success);
                            refresh_data(app, client).await;
                        }
                        Err(e) => app.set_status(format!("Failed to stop session: {e}"), StatusType::Error),
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    app.active_modal = None;
                }
                _ => {
                    app.active_modal = Some(Modal::ConfirmStop { session_id });
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
                                    format!("Cannot switch to {}: {}", chosen_label, chosen_reason.as_deref().unwrap_or("account unavailable")),
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
                                app.set_status(format!("Session is already bound to {}", chosen_label), StatusType::Info);
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
                                        reason: "This provider requires a controlled session restart.".to_string(),
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
                                app.set_status(
                                    format!("Switched session to {} (active session: {})", target_account_label, active_id.0),
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
            if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                && (key.code == KeyCode::Char('o') || key.code == KeyCode::Char('O'))
            {
                open_google_oauth_and_listen(app, client.clone());
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
                        0 => { label.pop(); }
                        3 => { token.pop(); }
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
                        0 => { label.push(c); }
                        1 => {
                            if c == 'c' || c == 'C' { provider = "claude".to_string(); }
                            else if c == 'a' || c == 'A' { provider = "agy".to_string(); }
                            else if c == 'p' || c == 'P' { provider = "pty".to_string(); }
                        }
                        2 => {
                            if c == '1' { auth_method = 0; }
                            else if c == '2' { auth_method = 1; }
                            else if c == '3' { auth_method = 2; }
                            else if c == 'o' || c == 'O' {
                                open_google_oauth_and_listen(app, client.clone());
                            }
                        }
                        3 => { token.push(c); }
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
                KeyCode::Enter => {
                    if active_field == 2 && auth_method == 0 && token.trim().is_empty() {
                        open_google_oauth_and_listen(app, client.clone());
                        app.active_modal = Some(Modal::AddAccount {
                            label,
                            provider,
                            auth_method,
                            token,
                            active_field: 3,
                        });
                        return Ok(());
                    }

                    let cred_id = ulid::Ulid::new().to_string();
                    let (auth_mode, token_data, default_label) = match auth_method {
                        0 => {
                            let t = if token.trim().is_empty() {
                                None
                            } else {
                                let c = token.trim();
                                let redirect_uri = format!("http://localhost:{}/oauth2callback", ac_core::credentials::DEFAULT_OAUTH_PORT);
                                let token_val = match ac_core::credentials::exchange_code_for_google_tokens(c, &redirect_uri).await {
                                    Ok(json_tokens) => {
                                        if let Some(jwt) = json_tokens["id_token"].as_str() {
                                            if let Some(email) = ac_core::credentials::extract_email_from_jwt(jwt) {
                                                if label.trim().is_empty() {
                                                    label = email;
                                                }
                                            }
                                        }
                                        serde_json::to_string_pretty(&json_tokens).unwrap_or_else(|_| c.to_string())
                                    }
                                    Err(_) => c.to_string(),
                                };
                                Some(token_val)
                            };
                            ("oauth2_token", t, "Personal Google".to_string())
                        }
                        1 => {
                            let t = if token.trim().is_empty() { None } else { Some(token.trim().to_string()) };
                            ("api_token", t, if provider == "claude" { "Claude Account".to_string() } else { "Personal Google".to_string() })
                        }
                        2 => {
                            let detected = ac_core::credentials::detect_existing_antigravity_token();
                            let (ident, content) = match detected {
                                Some((i, c)) => (i, Some(c)),
                                None => ("Personal Google".to_string(), None),
                            };
                            ("imported_session", content, if ident.is_empty() { "Personal Google".to_string() } else { ident })
                        }
                        _ => ("api_token", None, "Personal Google".to_string()),
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
                            auth_mode,
                            token_data,
                        ) {
                            Ok(cred_ref) => {
                                let agent_types: Vec<&str> = match provider.as_str() {
                                    "agy" => vec!["agy", "antigravity"],
                                    "claude" => vec!["claude", "claude-code"],
                                    _ => vec!["pty", "generic-pty"],
                                };

                                match client.register_account(
                                    &final_label,
                                    &provider,
                                    &agent_types,
                                    &cred_ref,
                                    2,
                                    &[],
                                ).await {
                                    Ok(_) => {
                                        app.active_modal = None;
                                        app.set_status(format!("✓ Account \"{}\" added successfully!", final_label), StatusType::Success);
                                        refresh_data(app, client).await;
                                    }
                                    Err(e) => {
                                        app.set_status(format!("Failed to register account: {}", e), StatusType::Error);
                                    }
                                }
                            }
                            Err(e) => {
                                app.set_status(format!("Failed to save credential: {}", e), StatusType::Error);
                            }
                        }
                    }
                    _ => {}
                }
            }

            Modal::NewSession {
                mut account_index,
                mut session_name,
                mut task,
                mut active_field,
            } => match key.code {
                KeyCode::Esc => {
                    app.active_modal = None;
                }
                KeyCode::Tab | KeyCode::Down => {
                    active_field = (active_field + 1) % 3;
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        task,
                        active_field,
                    });
                }
                KeyCode::BackTab | KeyCode::Up => {
                    active_field = if active_field == 0 { 2 } else { active_field - 1 };
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        task,
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
                        task,
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
                        task,
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
                        task,
                        active_field,
                    });
                }
                KeyCode::Backspace => {
                    match active_field {
                        1 => { session_name.pop(); }
                        2 => { task.pop(); }
                        _ => {}
                    }
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        task,
                        active_field,
                    });
                }
                KeyCode::Char(c) => {
                    match active_field {
                        1 => { session_name.push(c); }
                        2 => { task.push(c); }
                        _ => {}
                    }
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        task,
                        active_field,
                    });
                }
                KeyCode::Enter => {
                    let name_trimmed = session_name.trim().to_string();
                    let task_trimmed = task.trim().to_string();
                    if name_trimmed.is_empty() && task_trimmed.is_empty() {
                        app.set_status("Session name or task cannot be empty", StatusType::Warning);
                        app.active_modal = Some(Modal::NewSession {
                            account_index,
                            session_name,
                            task,
                            active_field,
                        });
                    } else {
                        app.active_modal = None;
                        let full_task = if !name_trimmed.is_empty() && !task_trimmed.is_empty() {
                            format!("{}: {}", name_trimmed, task_trimmed)
                        } else if !task_trimmed.is_empty() {
                            task_trimmed
                        } else {
                            name_trimmed
                        };

                        let chosen_acct = if !app.accounts.is_empty() {
                            let idx = account_index % app.accounts.len();
                            Some(app.accounts[idx].id.clone())
                        } else {
                            None
                        };

                        match client
                            .create_session(&full_task, "agy", None, chosen_acct.as_ref())
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
                                }
                                Err(e) => {
                                    app.set_status(
                                        format!(
                                            "Created session {} but failed to start: {e}",
                                            new_id.0
                                        ),
                                        StatusType::Error,
                                    );
                                }
                            },
                            Err(e) => {
                                app.set_status(
                                    format!("Failed to create session: {e}"),
                                    StatusType::Error,
                                );
                            }
                        }
                    }
                }
                _ => {
                    app.active_modal = Some(Modal::NewSession {
                        account_index,
                        session_name,
                        task,
                        active_field,
                    });
                }
            },
        }
        return Ok(());
    }

    // ── 2. Global Keybindings ───────────────────────────────────────────────
    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
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
            app.set_tab(Tab::Inbox);
            return Ok(());
        }
        KeyCode::Char('4') => {
            app.set_tab(Tab::Accounts);
            return Ok(());
        }
        KeyCode::Char('5') => {
            app.set_tab(Tab::Projects);
            return Ok(());
        }
        KeyCode::Char('6') => {
            app.set_tab(Tab::Activity);
            return Ok(());
        }
        KeyCode::Char('7') => {
            app.set_tab(Tab::Agents);
            return Ok(());
        }
        KeyCode::Char('8') => {
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
                app.active_modal = Some(Modal::NewSession {
                    account_index: app.selected_account,
                    session_name: String::new(),
                    task: String::new(),
                    active_field: 0,
                });
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

    // ── 3. View-Specific Keybindings ────────────────────────────────────────

    // If currently viewing Session Detail
    if app.session_detail_id.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.close_session_detail();
            }
            KeyCode::Char('s') => {
                if let Some(s) = app.selected_session_or_detail() {
                    app.active_modal = Some(Modal::Steer {
                        session_id: s.id.clone(),
                        input: String::new(),
                    });
                }
            }
            KeyCode::Char('p') => {
                if let Some(s) = app.selected_session_or_detail() {
                    let sid = s.id.clone();
                    match client.pause_session(&sid).await {
                        Ok(()) => app.set_status(format!("Paused session {}", sid.0), StatusType::Success),
                        Err(e) => app.set_status(format!("Failed to pause: {e}"), StatusType::Error),
                    }
                }
            }
            KeyCode::Char(' ') | KeyCode::Char('u') => {
                if let Some(s) = app.selected_session_or_detail().cloned() {
                    handle_session_resume_or_run(app, client, &s).await;
                }
            }
            KeyCode::Char('x') => {
                if let Some(s) = app.selected_session_or_detail() {
                    app.active_modal = Some(Modal::ConfirmStop {
                        session_id: s.id.clone(),
                    });
                }
            }
            KeyCode::Char('w') => {
                open_switch_account_modal(app, client).await;
            }
            _ => {}
        }
        return Ok(());
    }

    // Otherwise handle by active tab
    match app.current_tab {
        Tab::Dashboard => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Enter | KeyCode::Char('t') => {
                app.open_selected_session_detail();
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                app.active_modal = Some(Modal::AddAccount {
                    label: String::new(),
                    provider: "agy".to_string(),
                    auth_method: 0,
                    token: String::new(),
                    active_field: 0,
                });
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                app.set_tab(Tab::Sessions);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.active_modal = Some(Modal::NewSession {
                    account_index: app.selected_account,
                    session_name: String::new(),
                    task: String::new(),
                    active_field: 0,
                });
            }
            _ => {}
        },

        Tab::Sessions => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Enter | KeyCode::Char('t') => {
                app.open_selected_session_detail();
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.active_modal = Some(Modal::NewSession {
                    account_index: app.selected_account,
                    session_name: String::new(),
                    task: String::new(),
                    active_field: 0,
                });
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
                        Ok(()) => app.set_status(format!("Paused session {}", sid.0), StatusType::Success),
                        Err(e) => app.set_status(format!("Failed to pause: {e}"), StatusType::Error),
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
            KeyCode::Char('w') => {
                open_switch_account_modal(app, client).await;
            }
            _ => {}
        },

        Tab::Inbox => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Char('a') => {
                if let Some(item) = app.selected_pending_interaction() {
                    let iid = item.id.clone();
                    match client.resolve_interaction(&iid, Some(PolicyDecision::Allow), None).await {
                        Ok(_) => {
                            app.set_status(format!("Approved interaction {}", iid.0), StatusType::Success);
                            refresh_data(app, client).await;
                        }
                        Err(e) => app.set_status(format!("Failed to approve: {e}"), StatusType::Error),
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(item) = app.selected_pending_interaction() {
                    let iid = item.id.clone();
                    match client.resolve_interaction(&iid, Some(PolicyDecision::Deny), None).await {
                        Ok(_) => {
                            app.set_status(format!("Denied interaction {}", iid.0), StatusType::Success);
                            refresh_data(app, client).await;
                        }
                        Err(e) => app.set_status(format!("Failed to deny: {e}"), StatusType::Error),
                    }
                }
            }
            KeyCode::Char('r') => {
                if let Some(item) = app.selected_pending_interaction() {
                    app.active_modal = Some(Modal::Reply {
                        interaction_id: item.id.clone(),
                        input: String::new(),
                    });
                }
            }
            KeyCode::Char('x') => {
                if let Some(item) = app.selected_pending_interaction() {
                    let iid = item.id.clone();
                    match client.dismiss_interaction(&iid).await {
                        Ok(_) => {
                            app.set_status(format!("Dismissed interaction {}", iid.0), StatusType::Success);
                            refresh_data(app, client).await;
                        }
                        Err(e) => app.set_status(format!("Failed to dismiss: {e}"), StatusType::Error),
                    }
                }
            }
            _ => {}
        },

        Tab::Accounts => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Char('a') | KeyCode::Char('A') => {
                app.active_modal = Some(Modal::AddAccount {
                    label: String::new(),
                    provider: "agy".to_string(),
                    auth_method: 0,
                    token: String::new(),
                    active_field: 0,
                });
            }
            KeyCode::Char('d') | KeyCode::Char('D') | KeyCode::Delete => {
                if let Some(acct) = app.selected_account().cloned() {
                    let aid = acct.id.clone();
                    let label = acct.label.clone();
                    match client.remove_account(&aid).await {
                        Ok(()) => {
                            app.set_status(format!("Removed account: {}", label), StatusType::Success);
                            refresh_data(app, client).await;
                        }
                        Err(e) => {
                            app.set_status(format!("Failed to remove account: {e}"), StatusType::Error);
                        }
                    }
                }
            }
            _ => {}
        },

        Tab::Projects => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
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
                app.active_modal = Some(Modal::NewSession {
                    account_index: app.selected_account,
                    session_name: String::new(),
                    task: String::new(),
                    active_field: 0,
                });
            }
            _ => {}
        },

        Tab::Settings => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            _ => {}
        },
    }

    Ok(())
}

/// Refresh all entity collections from the daemon.
pub async fn refresh_data(app: &mut App, client: &ApiClient) {
    if let Ok(sessions) = client.list_sessions().await {
        app.sessions = sessions;
        if app.selected_session >= app.sessions.len() && !app.sessions.is_empty() {
            app.selected_session = app.sessions.len() - 1;
        }
    }
    if let Ok(interactions) = client.list_interactions(None, false).await {
        app.interactions = interactions;
        let pending_len = app.pending_interactions().len();
        if app.selected_interaction >= pending_len && pending_len > 0 {
            app.selected_interaction = pending_len - 1;
        }
    }
    if let Ok(accounts) = client.list_accounts().await {
        app.accounts = accounts;
        if app.selected_account >= app.accounts.len() && !app.accounts.is_empty() {
            app.selected_account = app.accounts.len() - 1;
        }
    }
    if let Ok(projects) = client.list_projects().await {
        app.projects = projects;
        if app.selected_project >= app.projects.len() && !app.projects.is_empty() {
            app.selected_project = app.projects.len() - 1;
        }
    }
    if let Ok(events) = client.query_events(None, Some(100)).await {
        app.events = events;
        if app.selected_event >= app.events.len() && !app.events.is_empty() {
            app.selected_event = app.events.len() - 1;
        }
    }
    app.daemon_connected = client.check_daemon().await;
}

async fn open_switch_account_modal(app: &mut App, client: &ApiClient) {
    if let Some(s) = app.selected_session_or_detail().cloned() {
        let agent_type = s.agent_type.clone();
        let session_id = s.id.clone();
        let current_account_id = s.account_id.clone();
        let current_account_label = app.account_label(current_account_id.as_ref());

        match client.query_account_availability(Some(&agent_type), &[]).await {
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
                Err(e) => app.set_status(format!("Failed to re-run session: {e}"), StatusType::Error),
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
