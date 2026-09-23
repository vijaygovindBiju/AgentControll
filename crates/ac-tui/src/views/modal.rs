//! Modal dialog popup rendering.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, Modal};

/// Helper to center a rectangular area on the screen.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn render(f: &mut Frame, app: &App) {
    let modal = match &app.active_modal {
        Some(m) => m,
        None => return,
    };

    match modal {
        Modal::Steer { session_id, input } => {
            let area = centered_rect(60, 25, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    format!(" Steer Session: {} ", session_id.0),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(Span::styled(
                    "Type instruction to inject into the running agent:",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(input, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("█", Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[Enter] Send Steer", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("    "),
                    Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }

        Modal::Reply { interaction_id, input } => {
            let area = centered_rect(60, 25, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    format!(" Reply to Interaction: {} ", interaction_id.0),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(Span::styled(
                    "Type response to answer agent's question or decision rationale:",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled(input, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("█", Style::default().fg(Color::Yellow)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[Enter] Submit Reply", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("    "),
                    Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }

        Modal::ConfirmStop { session_id } => {
            let area = centered_rect(50, 20, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(Span::styled(" Confirm Stop Session ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)));

            let lines = vec![
                Line::from(Span::styled(
                    format!("Are you sure you want to stop session {}?", session_id.0),
                    Style::default().fg(Color::White),
                )),
                Line::from(Span::styled(
                    "This will terminate the agent adapter and reclaim workspaces.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  [y/Enter] Confirm Stop  ", Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::raw("     "),
                    Span::styled("  [n/Esc] Cancel  ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines).block(block).alignment(Alignment::Center);
            f.render_widget(p, area);
        }

        Modal::FilterActivity { input } => {
            let area = centered_rect(55, 20, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(" Filter Activity Events ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));

            let lines = vec![
                Line::from(Span::styled(
                    "Enter filter string (matches session ID, event kind, or actor):",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Filter: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(input, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    Span::styled("█", Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("[Enter] Apply Filter", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("    "),
                    Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines).block(block);
            f.render_widget(p, area);
        }

        Modal::Help => {
            let area = centered_rect(65, 60, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(" Agent Control TUI — Help & Shortcuts ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));

            let lines = vec![
                Line::from(Span::styled("Navigation:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
                Line::from("  1 - 6          Switch tabs (1:Dashboard, 2:Sessions, 3:Inbox, 4:Accounts, 5:Projects, 6:Activity)"),
                Line::from("  Tab / BackTab  Cycle next/previous tab"),
                Line::from("  ↑ / ↓, j / k   Navigate rows in tables"),
                Line::from("  Enter / t      Open selected session detail view"),
                Line::from("  Esc            Go back / Close modal dialog"),
                Line::from("  q              Quit TUI"),
                Line::from("  r              Refresh data from daemon"),
                Line::from(""),
                Line::from(Span::styled("Session Actions (in Sessions / Session Detail):", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
                Line::from("  s              Steer session (inject human instruction)"),
                Line::from("  p              Pause running session"),
                Line::from("  Space          Resume paused session"),
                Line::from("  w              Switch account (controlled hand-off or rebind)"),
                Line::from("  x              Stop session"),
                Line::from(""),
                Line::from(Span::styled("Inbox Actions (in Inbox view):", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
                Line::from("  a              Approve pending approval request"),
                Line::from("  d              Deny pending approval request"),
                Line::from("  r              Reply to pending question/approval with text"),
                Line::from("  x              Dismiss interaction without decision"),
                Line::from(""),
                Line::from(Span::styled("Press Esc or Enter to close this help window.", Style::default().fg(Color::DarkGray))),
            ];

            let p = Paragraph::new(lines).block(block);
            f.render_widget(p, area);
        }

        Modal::SwitchAccount {
            session_id,
            agent_type,
            current_account_id: _,
            current_account_label,
            options,
            selected_index,
            step,
        } => match step {
            crate::app::SwitchModalStep::SelectAccount => {
                let area = centered_rect(70, 50, f.area());
                f.render_widget(Clear, area);

                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(Span::styled(
                        format!(" Switch Account — Session: {} ({}) ", session_id.0, agent_type),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ));

                let mut lines = vec![
                    Line::from(vec![
                        Span::styled("Current Account: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(current_account_label.as_str(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(Span::styled(
                        "Select target account for this session:",
                        Style::default().fg(Color::DarkGray),
                    )),
                    Line::from(""),
                ];

                if options.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "No compatible accounts found for this agent type.",
                        Style::default().fg(Color::Red),
                    )));
                } else {
                    for (i, opt) in options.iter().enumerate() {
                        let is_sel = i == *selected_index;
                        let prefix = if is_sel { " > " } else { "   " };
                        let load_str = format!("{}/{}", opt.active_sessions, opt.concurrency_cap);

                        let (status_span, line_style) = if !opt.usable {
                            let reason = opt.reason.as_deref().unwrap_or("Unavailable");
                            (
                                Span::styled(format!("[Unavailable: {reason}]"), Style::default().fg(Color::Red)),
                                if is_sel { Style::default().bg(Color::Rgb(50, 20, 20)) } else { Style::default().fg(Color::DarkGray) },
                            )
                        } else {
                            (
                                Span::styled("[Available]", Style::default().fg(Color::Green)),
                                if is_sel { Style::default().bg(Color::Rgb(30, 40, 60)) } else { Style::default() },
                            )
                        };

                        lines.push(Line::from(vec![
                            Span::styled(prefix, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                            Span::styled(format!("{:<18}", opt.label), line_style.fg(Color::White).add_modifier(Modifier::BOLD)),
                            Span::raw("  "),
                            Span::styled(format!("{:<10}", opt.provider), line_style.fg(Color::Gray)),
                            Span::raw("  "),
                            Span::styled(format!("Load: {:<6}", load_str), line_style.fg(Color::Yellow)),
                            Span::raw("  "),
                            status_span,
                        ]));
                    }
                }

                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("[↑/↓] Select Account", Style::default().fg(Color::Cyan)),
                    Span::raw("   "),
                    Span::styled("[Enter] Choose Account", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("   "),
                    Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
                ]));

                let p = Paragraph::new(lines).block(block);
                f.render_widget(p, area);
            }

            crate::app::SwitchModalStep::ConfirmRestart {
                target_account_id: _,
                target_account_label,
                reason,
            } => {
                let area = centered_rect(65, 45, f.area());
                f.render_widget(Clear, area);

                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(Span::styled(
                        " Confirm Account Switch & Session Hand-Off ",
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ));

                let lines = vec![
                    Line::from(Span::styled("Switch session from:", Style::default().fg(Color::DarkGray))),
                    Line::from(Span::styled(format!("  {}", current_account_label), Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
                    Line::from(""),
                    Line::from(Span::styled("to:", Style::default().fg(Color::DarkGray))),
                    Line::from(Span::styled(format!("  {}", target_account_label), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
                    Line::from(""),
                    Line::from(Span::styled(reason.as_str(), Style::default().fg(Color::Yellow))),
                    Line::from(Span::styled("• Workspace will be preserved.", Style::default().fg(Color::White))),
                    Line::from(Span::styled("• Process-local context cannot be preserved.", Style::default().fg(Color::Yellow))),
                    Line::from(""),
                    Line::from(Span::styled("A replacement session will be spawned and linked.", Style::default().fg(Color::DarkGray))),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("  [y/Enter] Confirm Hand-off  ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
                        Span::raw("     "),
                        Span::styled("  [n/Esc] Cancel  ", Style::default().fg(Color::White).bg(Color::DarkGray)),
                    ]),
                ];

                let p = Paragraph::new(lines).block(block).alignment(Alignment::Center);
                f.render_widget(p, area);
            }
        },

        Modal::AddAccount {
            label,
            provider,
            auth_method,
            token,
            active_field,
        } => {
            let area = centered_rect(70, 60, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " Add New Account ",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ));

            let normal_style = Style::default().fg(Color::White);
            let active_border = |active: bool| {
                if active {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                }
            };

            let label_display = if label.is_empty() {
                Span::styled("(e.g. Personal Google, Work Account)", Style::default().fg(Color::DarkGray))
            } else {
                Span::styled(label.as_str(), normal_style)
            };

            let provider_display = Span::styled(
                format!("{} (press Tab/Space to change: agy, claude, pty)", provider),
                if *active_field == 1 { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { normal_style },
            );

            // Auth method options
            let m0_style = if *auth_method == 0 {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let m1_style = if *auth_method == 1 {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let m2_style = if *auth_method == 2 {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let mut lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(if *active_field == 0 { "> " } else { "  " }, active_border(*active_field == 0)),
                    Span::styled("Account Name: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    label_display,
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(if *active_field == 1 { "> " } else { "  " }, active_border(*active_field == 1)),
                    Span::styled("Provider:     ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    provider_display,
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(if *active_field == 2 { "> " } else { "  " }, active_border(*active_field == 2)),
                    Span::styled("Auth Method:  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::styled(if *auth_method == 0 { "[● 1: OAuth Code] " } else { "[○ 1: OAuth Code] " }, m0_style),
                    Span::styled(if *auth_method == 1 { "[● 2: Token / Key] " } else { "[○ 2: Token / Key] " }, m1_style),
                    Span::styled(if *auth_method == 2 { "[● 3: Detected Session]" } else { "[○ 3: Detected Session]" }, m2_style),
                ]),
            ];

            if *active_field == 2 {
                lines.push(Line::from(vec![
                    Span::styled("                ", Style::default()),
                    Span::styled("(Press 1/2/3, Left/Right, or Space to toggle method)", Style::default().fg(Color::Yellow)),
                ]));
            }

            lines.push(Line::from(""));

            match *auth_method {
                0 => {
                    let token_display = if token.is_empty() {
                        Span::styled("(Paste authorization code here)", Style::default().fg(Color::DarkGray))
                    } else {
                        Span::styled(token.as_str(), normal_style)
                    };

                    lines.push(Line::from(vec![
                        Span::styled("  Google Sign-in:  ", Style::default().fg(Color::DarkGray)),
                        Span::styled("Press [O] to open Google sign-in page in browser", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("                   URL: https://accounts.google.com/o/oauth2/v2/auth?...", Style::default().fg(Color::DarkGray)),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![
                        Span::styled(if *active_field == 3 { "> " } else { "  " }, active_border(*active_field == 3)),
                        Span::styled("OAuth Code:   ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        token_display,
                    ]));
                }
                1 => {
                    let token_display = if token.is_empty() {
                        Span::styled("(Paste API token or access token here)", Style::default().fg(Color::DarkGray))
                    } else {
                        Span::styled(token.as_str(), normal_style)
                    };

                    lines.push(Line::from(vec![
                        Span::styled("  API Token:       Enter API token, access token, or service key", Style::default().fg(Color::DarkGray)),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![
                        Span::styled(if *active_field == 3 { "> " } else { "  " }, active_border(*active_field == 3)),
                        Span::styled("Token / Key:  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        token_display,
                    ]));
                }
                _ => {
                    let detected = ac_core::credentials::detect_existing_antigravity_token();
                    if let Some((identity, _)) = detected {
                        lines.push(Line::from(vec![
                            Span::styled("  Local Session:   ", Style::default().fg(Color::DarkGray)),
                            Span::styled(format!("Found ~/.gemini/antigravity-cli session ({})", identity), Style::default().fg(Color::Green)),
                        ]));
                        lines.push(Line::from(""));
                        lines.push(Line::from(vec![
                            Span::styled("                   Press [Enter] to import and link this session.", Style::default().fg(Color::Yellow)),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled("  Local Session:   ", Style::default().fg(Color::DarkGray)),
                            Span::styled("No active local ~/.gemini token found. Use OAuth or Token.", Style::default().fg(Color::Red)),
                        ]));
                    }
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from("────────────────────────────────────────────────────────────────────────"));
            lines.push(Line::from(Span::styled(
                " [Enter] Save Account  |  [Tab/↓] Next  |  [O] Open Google Login  |  [Esc] Cancel ",
                Style::default().fg(Color::DarkGray),
            )));

            let p = Paragraph::new(lines).block(block);
            f.render_widget(p, area);
        }

        Modal::NewSession {
            account_index,
            session_name,
            task,
            active_field,
        } => {
            let area = centered_rect(70, 45, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(Span::styled(
                    " Launch New Agent Session ",
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                ));

            let acct_label = if app.accounts.is_empty() {
                "No accounts available (will use default)".to_string()
            } else {
                let idx = *account_index % app.accounts.len();
                let a = &app.accounts[idx];
                format!("< {} ({}) >", a.label, a.provider)
            };

            let f0_style = if *active_field == 0 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let f1_style = if *active_field == 1 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let f2_style = if *active_field == 2 {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let lines = vec![
                Line::from(Span::styled(
                    "Configure agent session parameters and account:",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(if *active_field == 0 { "> 1. Account:      " } else { "  1. Account:      " }, f0_style),
                    Span::styled(acct_label, f0_style),
                    Span::styled(if *active_field == 0 { "  (use ←/→ or Space to switch)" } else { "" }, Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(if *active_field == 1 { "> 2. Session Name: " } else { "  2. Session Name: " }, f1_style),
                    Span::styled(session_name, f1_style),
                    Span::styled(if *active_field == 1 { "█" } else { "" }, Style::default().fg(Color::Yellow)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(if *active_field == 2 { "> 3. Task / Goal:  " } else { "  3. Task / Goal:  " }, f2_style),
                    Span::styled(task, f2_style),
                    Span::styled(if *active_field == 2 { "█" } else { "" }, Style::default().fg(Color::Yellow)),
                ]),
                Line::from(""),
                Line::from(Span::styled("───────────────────────────────────────────────────────────────────", Style::default().fg(Color::DarkGray))),
                Line::from(vec![
                    Span::styled("[Enter] Launch Session", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::raw("    "),
                    Span::styled("[Tab/↓] Next Field", Style::default().fg(Color::Cyan)),
                    Span::raw("    "),
                    Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }
    }
}

