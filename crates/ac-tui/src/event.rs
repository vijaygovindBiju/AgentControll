//! Keyboard input and async event handling.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use ac_core::types::PolicyDecision;
use crate::{
    app::{App, Modal, StatusType, Tab},
    client::ApiClient,
};

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
                if let Some(s) = app.selected_session_or_detail() {
                    let sid = s.id.clone();
                    match client.resume_session(&sid).await {
                        Ok(()) => app.set_status(format!("Resumed session {}", sid.0), StatusType::Success),
                        Err(e) => app.set_status(format!("Failed to resume: {e}"), StatusType::Error),
                    }
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
            _ => {}
        },

        Tab::Sessions => match key.code {
            KeyCode::Up | KeyCode::Char('k') => app.prev_row(),
            KeyCode::Down | KeyCode::Char('j') => app.next_row(),
            KeyCode::Enter | KeyCode::Char('t') => {
                app.open_selected_session_detail();
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
                if let Some(s) = app.selected_session() {
                    let sid = s.id.clone();
                    match client.resume_session(&sid).await {
                        Ok(()) => app.set_status(format!("Resumed session {}", sid.0), StatusType::Success),
                        Err(e) => app.set_status(format!("Failed to resume: {e}"), StatusType::Error),
                    }
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
