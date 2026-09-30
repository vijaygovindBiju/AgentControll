//! Modal dialog popup rendering.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{AgyAddStep, App, Modal};

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
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(Span::styled(
                    "Type instruction to inject into the running agent:",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "> ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        input,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("█", Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "[Enter] Send Steer",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("    "),
                    Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }

        Modal::Reply {
            interaction_id,
            input,
        } => {
            let area = centered_rect(60, 25, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    format!(" Reply to Interaction: {} ", interaction_id.0),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(Span::styled(
                    "Type response to answer agent's question or decision rationale:",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "> ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        input,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("█", Style::default().fg(Color::Yellow)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "[Enter] Submit Reply",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("    "),
                    Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }

        Modal::ConfirmStop { session_id } => {
            let area = centered_rect(50, 20, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(Span::styled(
                    " Confirm Stop Session ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));

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
                    Span::styled(
                        "  [y/Enter] Confirm Stop  ",
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::Red)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("     "),
                    Span::styled(
                        "  [n/Esc] Cancel  ",
                        Style::default().fg(Color::Black).bg(Color::DarkGray),
                    ),
                ]),
            ];

            let p = Paragraph::new(lines)
                .block(block)
                .alignment(Alignment::Center);
            f.render_widget(p, area);
        }

        Modal::AgyAddAccount { label, step } => {
            let key = |k: &'static str| {
                Span::styled(
                    k,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            };
            let dim = Style::default().fg(Color::DarkGray);
            let bold = Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD);
            let (title, color, lines): (&str, Color, Vec<Line>) = match step {
                AgyAddStep::Name { error } => {
                    let mut l = vec![
                        Line::from(""),
                        Line::from(Span::styled(" Account name:", bold)),
                        Line::from(vec![
                            Span::styled(" > ", Style::default().fg(Color::Cyan)),
                            Span::styled(label.as_str(), bold),
                            Span::styled("█", Style::default().fg(Color::Cyan)),
                        ]),
                        Line::from(Span::styled("   e.g. Personal Google, College Google", dim)),
                        Line::from(""),
                    ];
                    if let Some(e) = error {
                        l.push(Line::from(Span::styled(
                            format!(" {e}"),
                            Style::default().fg(Color::Red),
                        )));
                        l.push(Line::from(""));
                    }
                    l.push(Line::from(vec![
                        key(" Enter "),
                        Span::raw(" Continue   "),
                        key(" Esc "),
                        Span::raw(" Cancel   "),
                        Span::styled("Ctrl+O other provider", dim),
                    ]));
                    (" Add Antigravity Account ", Color::Cyan, l)
                }
                AgyAddStep::Method => {
                    let options = [
                        (
                            "Login with Browser",
                            "Open your default browser on this machine.",
                        ),
                        (
                            "Generate Login Link",
                            "Get a link to open on another device or send to someone.",
                        ),
                        (
                            "Import Existing Credentials",
                            "Use this machine's existing agy login.",
                        ),
                    ];
                    let mut l = vec![
                        Line::from(""),
                        Line::from(Span::styled(format!(" Account: {label}"), bold)),
                        Line::from(""),
                        Line::from(Span::styled(" Authentication:", bold)),
                        Line::from(""),
                    ];
                    for (i, (name, desc)) in options.iter().enumerate() {
                        let sel = i == app.agy_add_method % options.len();
                        l.push(Line::from(vec![
                            Span::styled(
                                if sel { "  > " } else { "    " },
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                *name,
                                if sel {
                                    bold.fg(Color::Cyan)
                                } else {
                                    Style::default().fg(Color::White)
                                },
                            ),
                        ]));
                        if sel {
                            l.push(Line::from(Span::styled(format!("      {desc}"), dim)));
                        }
                    }
                    l.push(Line::from(""));
                    l.push(Line::from(vec![
                        key(" ↑↓ "),
                        Span::raw(" Select   "),
                        key(" Enter "),
                        Span::raw(" Continue   "),
                        key(" Esc "),
                        Span::raw(" Cancel"),
                    ]));
                    (" Add Antigravity Account ", Color::Cyan, l)
                }
                AgyAddStep::Link {
                    url,
                    deadline,
                    paste,
                    notice,
                } => {
                    let left = deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .as_secs();
                    let mut l = vec![
                        Line::from(""),
                        Line::from(Span::styled(format!(" Account: {label}"), bold)),
                        Line::from(""),
                        Line::from(Span::styled(" Login Link", bold)),
                        Line::from(Span::styled(url.as_str(), Style::default().fg(Color::Cyan))),
                        Line::from(""),
                        Line::from(vec![Span::raw(" "), key(" [ Copy Link ] "), Span::styled("  c / Ctrl+Y", dim)]),
                        Line::from(""),
                        Line::from(Span::styled(" Waiting for authentication...", bold)),
                        Line::from(Span::styled(" • Opened on this computer: completes automatically.", dim)),
                        Line::from(Span::styled(" • Opened on another device: after signing in, the browser shows an", dim)),
                        Line::from(Span::styled("   error page at 127.0.0.1 — copy that page's full address and paste it here:", dim)),
                    ];
                    // The pasted address contains a one-time code: never shown in clear.
                    let shown = if paste.is_empty() {
                        String::new()
                    } else {
                        format!("•••••••• ({} chars)", paste.chars().count())
                    };
                    l.push(Line::from(vec![
                        Span::styled(" > ", Style::default().fg(Color::Cyan)),
                        Span::styled(shown, bold),
                        Span::styled("█", Style::default().fg(Color::Cyan)),
                    ]));
                    if let Some(n) = notice {
                        let color = if n.starts_with("Not accepted")
                            || n.starts_with("Could not")
                            || n.contains("no longer")
                        {
                            Color::Yellow
                        } else {
                            Color::Green
                        };
                        l.push(Line::from(Span::styled(
                            format!(" {n}"),
                            Style::default().fg(color),
                        )));
                    }
                    l.push(Line::from(""));
                    l.push(Line::from(format!(
                        " Link expires in: {:02}:{:02}  (single use)",
                        left / 60,
                        left % 60
                    )));
                    l.push(Line::from(vec![
                        key(" Enter "),
                        Span::raw(" Submit address   "),
                        key(" Esc "),
                        Span::raw(" Cancel"),
                    ]));
                    (" Add Antigravity Account ", Color::Cyan, l)
                }
                AgyAddStep::Waiting {
                    url,
                    browser_opened,
                    deadline,
                } => {
                    let left = deadline
                        .saturating_duration_since(std::time::Instant::now())
                        .as_secs();
                    let mut l = vec![Line::from("")];
                    if url.is_empty() {
                        l.push(Line::from(" Preparing secure login..."));
                    } else if *browser_opened {
                        l.push(Line::from(Span::styled(
                            " Waiting for browser authentication...",
                            bold,
                        )));
                        l.push(Line::from(Span::styled(
                            " Complete the Google sign-in in your browser.",
                            dim,
                        )));
                    } else {
                        l.push(Line::from(Span::styled(
                            " Unable to open your default browser.",
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        )));
                        l.push(Line::from(""));
                        l.push(Line::from(" Open this URL manually:"));
                        l.push(Line::from(Span::styled(
                            format!(" {url}"),
                            Style::default().fg(Color::Cyan),
                        )));
                        l.push(Line::from(Span::styled(
                            " Login completes automatically when the browser returns.",
                            dim,
                        )));
                    }
                    l.push(Line::from(""));
                    l.push(Line::from(format!(
                        " Time remaining: {:02}:{:02}",
                        left / 60,
                        left % 60
                    )));
                    l.push(Line::from(""));
                    l.push(Line::from(vec![key(" Esc "), Span::raw(" Cancel")]));
                    (" Antigravity Login ", Color::Cyan, l)
                }
                AgyAddStep::Success { email, .. } => {
                    let mut l = vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            " ✓ Login successful",
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        )),
                        Line::from(""),
                        Line::from(format!(" Account:  {label}")),
                    ];
                    if let Some(e) = email {
                        l.push(Line::from(format!(" Google:   {e}")));
                    }
                    l.push(Line::from(" Provider: Antigravity"));
                    l.push(Line::from(vec![
                        Span::raw(" Status:   "),
                        Span::styled("● Ready", Style::default().fg(Color::Green)),
                    ]));
                    l.push(Line::from(""));
                    l.push(Line::from(vec![key(" Enter "), Span::raw(" Continue")]));
                    (" Antigravity Account Added ", Color::Green, l)
                }
                AgyAddStep::Failed { reason } => (
                    " Antigravity Login Failed ",
                    Color::Red,
                    vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            " ✗ Login failed",
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(""),
                        Line::from(" The account was not added."),
                        Line::from(""),
                        Line::from(format!(" Reason: {reason}")),
                        Line::from(""),
                        Line::from(vec![
                            key("  r  "),
                            Span::raw(" Retry       "),
                            key(" Esc "),
                            Span::raw(" Cancel"),
                        ]),
                    ],
                ),
            };
            let area = if matches!(step, AgyAddStep::Link { .. }) {
                centered_rect(84, 80, f.area())
            } else {
                centered_rect(64, 45, f.area())
            };
            f.render_widget(Clear, area);
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color))
                .title(Span::styled(
                    title,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
            f.render_widget(
                Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false }),
                area,
            );
        }

        Modal::StartSession(form) => super::start_session::render(f, app, form),

        Modal::ConfirmRemoveAccount {
            label,
            provider,
            active_sessions,
            ..
        } => {
            let area = centered_rect(64, 32, f.area());
            f.render_widget(Clear, area);
            let title = if provider == "agy" {
                " Remove Antigravity Account "
            } else {
                " Remove Account "
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(Span::styled(
                    title,
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
            let dim = Style::default().fg(Color::DarkGray);
            let mut lines = vec![Line::from("")];
            if *active_sessions > 0 {
                lines.extend([
                    Line::from(Span::styled(
                        format!("Cannot remove \"{label}\"."),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        format!("Active sessions: {active_sessions}"),
                        Style::default().fg(Color::Yellow),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Stop the session first before removing this account.",
                        dim,
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  [Esc] Close  ",
                        Style::default().fg(Color::Black).bg(Color::DarkGray),
                    )),
                ]);
            } else {
                lines.extend([
                    Line::from(Span::styled(
                        format!("Remove account \"{label}\"?"),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "This will remove the account from Agent Control.",
                        dim,
                    )),
                    Line::from(Span::styled(
                        if provider == "agy" {
                            "Saved Antigravity credentials will be deleted."
                        } else {
                            "The account record will be deleted."
                        },
                        dim,
                    )),
                    Line::from(Span::styled(
                        "The account cannot be used for new sessions.",
                        dim,
                    )),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(
                            "  [Enter] Remove  ",
                            Style::default()
                                .fg(Color::White)
                                .bg(Color::Red)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("     "),
                        Span::styled(
                            "  [Esc] Cancel  ",
                            Style::default().fg(Color::Black).bg(Color::DarkGray),
                        ),
                    ]),
                ]);
            }
            let p = Paragraph::new(lines)
                .block(block)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            f.render_widget(p, area);
        }

        Modal::ConfirmRemoveSession { session_id } => {
            let area = centered_rect(52, 20, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(Span::styled(
                    " Confirm Remove Session ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(Span::styled(
                    format!("Permanently remove session {}?", session_id.0),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    "This will stop active processes, free workspaces, and purge the session.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  [y/Enter] Confirm Remove  ",
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::Red)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("     "),
                    Span::styled(
                        "  [n/Esc] Cancel  ",
                        Style::default().fg(Color::Black).bg(Color::DarkGray),
                    ),
                ]),
            ];

            let p = Paragraph::new(lines)
                .block(block)
                .alignment(Alignment::Center);
            f.render_widget(p, area);
        }

        Modal::FilterActivity { input } => {
            let area = centered_rect(55, 20, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " Filter Activity Events ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(Span::styled(
                    "Enter filter string (matches session ID, event kind, or actor):",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "Filter: ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        input,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("█", Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "[Enter] Apply Filter",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
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
                .title(Span::styled(
                    " Agent Control TUI — Help & Shortcuts ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(Span::styled("Navigation:", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
                Line::from("  1 - 6          Switch tabs (1:Dashboard, 2:Sessions, 3:Inbox, 4:Accounts, 5:Projects, 6:Activity)"),
                Line::from("  Tab / BackTab  Cycle next/previous tab"),
                Line::from("  ↑ / ↓, j / k   Navigate rows in tables"),
                Line::from("  Enter / t      Open selected session detail view"),
                Line::from("  Ctrl+Q         Detach / Return from session terminal view"),
                Line::from("  Esc            Go back / Close modal dialog (forwarded to session terminal)"),
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
                        format!(
                            " Switch Account — Session: {} ({}) ",
                            session_id.0, agent_type
                        ),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ));

                let mut lines = vec![
                    Line::from(vec![
                        Span::styled("Current Account: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            current_account_label.as_str(),
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD),
                        ),
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
                                Span::styled(
                                    format!("[Unavailable: {reason}]"),
                                    Style::default().fg(Color::Red),
                                ),
                                if is_sel {
                                    Style::default().bg(Color::Rgb(50, 20, 20))
                                } else {
                                    Style::default().fg(Color::DarkGray)
                                },
                            )
                        } else {
                            (
                                Span::styled("[Available]", Style::default().fg(Color::Green)),
                                if is_sel {
                                    Style::default().bg(Color::Rgb(30, 40, 60))
                                } else {
                                    Style::default()
                                },
                            )
                        };

                        lines.push(Line::from(vec![
                            Span::styled(
                                prefix,
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!("{:<18}", opt.label),
                                line_style.fg(Color::White).add_modifier(Modifier::BOLD),
                            ),
                            Span::raw("  "),
                            Span::styled(
                                format!("{:<10}", opt.provider),
                                line_style.fg(Color::Gray),
                            ),
                            Span::raw("  "),
                            Span::styled(
                                format!("Load: {:<6}", load_str),
                                line_style.fg(Color::Yellow),
                            ),
                            Span::raw("  "),
                            status_span,
                        ]));
                    }
                }

                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("[↑/↓] Select Account", Style::default().fg(Color::Cyan)),
                    Span::raw("   "),
                    Span::styled(
                        "[Enter] Choose Account",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
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
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));

                let lines = vec![
                    Line::from(Span::styled(
                        "Switch session from:",
                        Style::default().fg(Color::DarkGray),
                    )),
                    Line::from(Span::styled(
                        format!("  {}", current_account_label),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled("to:", Style::default().fg(Color::DarkGray))),
                    Line::from(Span::styled(
                        format!("  {}", target_account_label),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        reason.as_str(),
                        Style::default().fg(Color::Yellow),
                    )),
                    Line::from(Span::styled(
                        "• Workspace will be preserved.",
                        Style::default().fg(Color::White),
                    )),
                    Line::from(Span::styled(
                        "• Process-local context cannot be preserved.",
                        Style::default().fg(Color::Yellow),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "A replacement session will be spawned and linked.",
                        Style::default().fg(Color::DarkGray),
                    )),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(
                            "  [y/Enter] Confirm Hand-off  ",
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("     "),
                        Span::styled(
                            "  [n/Esc] Cancel  ",
                            Style::default().fg(Color::White).bg(Color::DarkGray),
                        ),
                    ]),
                ];

                let p = Paragraph::new(lines)
                    .block(block)
                    .alignment(Alignment::Center);
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
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));

            let normal_style = Style::default().fg(Color::White);
            let active_border = |active: bool| {
                if active {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                }
            };

            let label_display = if label.is_empty() {
                Span::styled(
                    "(e.g. Personal Google, Work Account)",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                Span::styled(label.as_str(), normal_style)
            };

            let provider_display = Span::styled(
                format!("{} (press Tab/Space to change: agy, claude, pty)", provider),
                if *active_field == 1 {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    normal_style
                },
            );

            // Auth method options
            let m0_style = if *auth_method == 0 {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let m1_style = if *auth_method == 1 {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let m2_style = if *auth_method == 2 {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let mut lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        if *active_field == 0 { "> " } else { "  " },
                        active_border(*active_field == 0),
                    ),
                    Span::styled(
                        "Account Name: ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    label_display,
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        if *active_field == 1 { "> " } else { "  " },
                        active_border(*active_field == 1),
                    ),
                    Span::styled(
                        "Provider:     ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    provider_display,
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        if *active_field == 2 { "> " } else { "  " },
                        active_border(*active_field == 2),
                    ),
                    Span::styled(
                        "Auth Method:  ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        if *auth_method == 0 {
                            "[● 1: Browser Sign-in] "
                        } else {
                            "[○ 1: Browser Sign-in] "
                        },
                        m0_style,
                    ),
                    Span::styled(
                        if *auth_method == 1 {
                            "[● 2: Token / Key] "
                        } else {
                            "[○ 2: Token / Key] "
                        },
                        m1_style,
                    ),
                    Span::styled(
                        if *auth_method == 2 {
                            "[● 3: Detected Session]"
                        } else {
                            "[○ 3: Detected Session]"
                        },
                        m2_style,
                    ),
                ]),
            ];

            if *active_field == 2 {
                lines.push(Line::from(vec![
                    Span::styled("                ", Style::default()),
                    Span::styled(
                        "(Press 1/2/3, Left/Right, or Space to toggle method)",
                        Style::default().fg(Color::Yellow),
                    ),
                ]));
            }

            lines.push(Line::from(""));

            match *auth_method {
                0 => {
                    let text = if provider == "agy" {
                        "Press [Enter] to sign in with Google in your browser. The account is added only after login succeeds."
                    } else {
                        "Browser sign-in is only available for Antigravity accounts."
                    };
                    lines.push(Line::from(vec![
                        Span::styled("  Google Sign-in:  ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            text,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                    lines.push(Line::from(vec![Span::styled(
                        "                   Headless machine? Use: agy account add --cli",
                        Style::default().fg(Color::DarkGray),
                    )]));
                }
                1 => {
                    let token_display = if token.is_empty() {
                        Span::styled(
                            "(Paste API token or access token here)",
                            Style::default().fg(Color::DarkGray),
                        )
                    } else {
                        Span::styled(token.as_str(), normal_style)
                    };

                    lines.push(Line::from(vec![Span::styled(
                        "  API Token:       Enter API token, access token, or service key",
                        Style::default().fg(Color::DarkGray),
                    )]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![
                        Span::styled(
                            if *active_field == 3 { "> " } else { "  " },
                            active_border(*active_field == 3),
                        ),
                        Span::styled(
                            "Token / Key:  ",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        token_display,
                    ]));
                }
                _ => {
                    let detected = ac_core::agy_auth::detect_local_login();
                    if let Some((identity, _)) = detected {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "  Local Session:   ",
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled(
                                format!("Found ~/.gemini/antigravity-cli session ({})", identity),
                                Style::default().fg(Color::Green),
                            ),
                        ]));
                        lines.push(Line::from(""));
                        lines.push(Line::from(vec![Span::styled(
                            "                   Press [Enter] to import and link this session.",
                            Style::default().fg(Color::Yellow),
                        )]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "  Local Session:   ",
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::styled(
                                "No valid local agy login found. Use browser sign-in.",
                                Style::default().fg(Color::Red),
                            ),
                        ]));
                    }
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(
                "────────────────────────────────────────────────────────────────────────",
            ));
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
            active_field,
        } => {
            let area = centered_rect(65, 35, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(Span::styled(
                    " Launch New Agent Session ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ));

            let acct_label = if app.accounts.is_empty() {
                "No accounts available (will use default)".to_string()
            } else {
                let idx = *account_index % app.accounts.len();
                let a = &app.accounts[idx];
                format!("< {} ({}) >", a.label, a.provider)
            };

            let f0_style = if *active_field == 0 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let f1_style = if *active_field == 1 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let lines = vec![
                Line::from(Span::styled(
                    "Configure agent session parameters:",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        if *active_field == 0 {
                            "> 1. Account:      "
                        } else {
                            "  1. Account:      "
                        },
                        f0_style,
                    ),
                    Span::styled(acct_label, f0_style),
                    Span::styled(
                        if *active_field == 0 {
                            "  (use ←/→ or Space to switch)"
                        } else {
                            ""
                        },
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        if *active_field == 1 {
                            "> 2. Session Name: "
                        } else {
                            "  2. Session Name: "
                        },
                        f1_style,
                    ),
                    Span::styled(session_name, f1_style),
                    Span::styled(
                        if *active_field == 1 { "█" } else { "" },
                        Style::default().fg(Color::Yellow),
                    ),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "ℹ Task prompt will be provided interactively when the terminal opens.",
                    Style::default().fg(Color::Cyan),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "─────────────────────────────────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(vec![
                    Span::styled(
                        "[Enter] Launch & Open Terminal",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("    "),
                    Span::styled("[Tab/↓] Switch Field", Style::default().fg(Color::Cyan)),
                    Span::raw("    "),
                    Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }

        Modal::CommandPalette {
            session_id,
            selected_index,
        } => {
            let area = centered_rect(50, 45, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " Agent Control ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));

            let items = [
                "Pause Session",
                "Resume Session",
                "Switch Account",
                "Stop Session",
                "Remove Session",
                "Session Information",
                "View Events",
                "Copy Session ID",
                "Return to Sessions",
            ];

            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Session: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        &session_id.0,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(
                    "───────────────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            for (idx, item) in items.iter().enumerate() {
                if idx == *selected_index {
                    lines.push(Line::from(vec![
                        Span::styled(
                            " > ",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" {item} "),
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled("   ", Style::default()),
                        Span::styled(*item, Style::default().fg(Color::White)),
                    ]));
                }
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "↑↓ Select   Enter Execute   Esc Close",
                Style::default().fg(Color::DarkGray),
            )));

            let p = Paragraph::new(lines).block(block);
            f.render_widget(p, area);
        }

        Modal::Approval {
            interaction_id: _,
            tool_name,
            prompt,
        } => {
            let area = centered_rect(65, 30, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(Span::styled(
                    " Agent Control: Approval Required ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));

            let detail = tool_name.as_deref().unwrap_or(prompt.as_str());
            let lines = vec![
                Line::from(Span::styled(
                    "Agent requested permission to execute:",
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  $ ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        detail,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  [a] Approve  ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("     "),
                    Span::styled(
                        "  [d] Deny  ",
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::Red)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("     "),
                    Span::styled(
                        "  [Esc] Back  ",
                        Style::default().fg(Color::Black).bg(Color::DarkGray),
                    ),
                ]),
            ];

            let p = Paragraph::new(lines)
                .block(block)
                .alignment(Alignment::Center);
            f.render_widget(p, area);
        }

        Modal::SessionInfo { session_id } => {
            let area = centered_rect(65, 48, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    format!(" Session Information: {} ", session_id.0),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));

            let session = app.sessions.iter().find(|s| &s.id == session_id);
            let lines = if let Some(s) = session {
                let acct_label = app.account_label(s.account_id.as_ref());
                let proj = s
                    .project_id
                    .as_ref()
                    .map(|p| p.0.as_str())
                    .unwrap_or("None");
                let ws = s
                    .workspace_id
                    .as_ref()
                    .map(|w| w.0.as_str())
                    .unwrap_or("None");
                let created = s.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string();
                let updated = s.updated_at.format("%Y-%m-%d %H:%M:%S UTC").to_string();

                vec![
                    Line::from(vec![
                        Span::styled("State:        ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            format!("{}", s.state),
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Agent Type:   ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&s.agent_type, Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("Account:      ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            acct_label,
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Project:      ", Style::default().fg(Color::DarkGray)),
                        Span::styled(proj, Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("Workspace:    ", Style::default().fg(Color::DarkGray)),
                        Span::styled(ws, Style::default().fg(Color::Cyan)),
                    ]),
                    Line::from(vec![
                        Span::styled("Task Goal:    ", Style::default().fg(Color::DarkGray)),
                        Span::styled(&s.task_description, Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("Restarts:     ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            format!("{}", s.restart_count),
                            Style::default().fg(Color::White),
                        ),
                    ]),
                    Line::from(vec![
                        Span::styled("Created:      ", Style::default().fg(Color::DarkGray)),
                        Span::styled(created, Style::default().fg(Color::DarkGray)),
                    ]),
                    Line::from(vec![
                        Span::styled("Updated:      ", Style::default().fg(Color::DarkGray)),
                        Span::styled(updated, Style::default().fg(Color::DarkGray)),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Press [Esc] or [Enter] to return to terminal",
                        Style::default().fg(Color::Cyan),
                    )),
                ]
            } else {
                vec![Line::from("Session not found.")]
            };

            let p = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }

        Modal::RegisterProject {
            name,
            repo_path,
            policy_index,
            active_field,
            error,
        } => {
            let area = centered_rect(64, 45, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " Register Project ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));

            let policies = ["IsolatedWorktree", "ReadOnlyWorkspace", "CurrentWorkingDir"];
            let pol_str = policies
                .get(*policy_index)
                .copied()
                .unwrap_or("IsolatedWorktree");

            let dim = Style::default().fg(Color::DarkGray);
            let lbl = |f_idx: usize, text: &'static str| {
                Span::styled(
                    text,
                    if *active_field == f_idx {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    },
                )
            };
            let cursor = |f_idx: usize| {
                if *active_field == f_idx {
                    "█"
                } else {
                    ""
                }
            };

            let mut lines = vec![
                Line::from(""),
                Line::from(vec![Span::raw(" "), lbl(0, "Project Name")]),
                Line::from(vec![
                    Span::styled(
                        if *active_field == 0 { " > " } else { "   " },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        if name.is_empty() {
                            "(enter name)"
                        } else {
                            name
                        },
                        if name.is_empty() {
                            dim
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                    Span::styled(cursor(0), Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![Span::raw(" "), lbl(1, "Repository Path")]),
                Line::from(vec![
                    Span::styled(
                        if *active_field == 1 { " > " } else { "   " },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        if repo_path.is_empty() {
                            "(e.g. /home/user/myproject or ~/myproject)"
                        } else {
                            repo_path
                        },
                        if repo_path.is_empty() {
                            dim
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                    Span::styled(cursor(1), Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
                Line::from(vec![Span::raw(" "), lbl(2, "Workspace Policy")]),
                Line::from(vec![
                    Span::styled(
                        if *active_field == 2 { " > " } else { "   " },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        pol_str,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        if *active_field == 2 {
                            "   ←/→ cycle"
                        } else {
                            ""
                        },
                        dim,
                    ),
                ]),
                Line::from(""),
            ];

            if let Some(e) = error {
                lines.push(Line::from(Span::styled(
                    format!(" Error: {e}"),
                    Style::default().fg(Color::Red),
                )));
                lines.push(Line::from(""));
            }

            let b0_sel = *active_field == 3;
            let b1_sel = *active_field == 4;
            lines.push(Line::from(vec![
                Span::raw("             "),
                Span::styled(
                    "[ Register ]",
                    if b0_sel {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::raw("    "),
                Span::styled(
                    "[ Cancel ]",
                    if b1_sel {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Tab/↑↓ field  ←/→ option  Enter submit  Esc cancel",
                dim,
            )));

            let p = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }

        Modal::ConfirmRemoveProject {
            project_id: _,
            name,
        } => {
            let area = centered_rect(56, 20, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(Span::styled(
                    " Remove Project ",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));

            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!(" Remove project '{name}' from Agent Control?"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " This removes the project from the registry.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    " Repository files on disk will NOT be deleted.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "[Enter/y] Remove",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("    "),
                    Span::styled("[Esc/n] Cancel", Style::default().fg(Color::DarkGray)),
                ]),
            ];

            let p = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }

        Modal::SetDefaultWorkingDir { input, error } => {
            let area = centered_rect(64, 22, f.area());
            f.render_widget(Clear, area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    " Set Default Working Directory ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));

            let mut lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Enter directory path (~ and absolute paths supported):",
                    Style::default().fg(Color::White),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "> ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        input,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("█", Style::default().fg(Color::Cyan)),
                ]),
                Line::from(""),
            ];

            if let Some(e) = error {
                lines.push(Line::from(Span::styled(
                    format!(" Error: {e}"),
                    Style::default().fg(Color::Red),
                )));
                lines.push(Line::from(""));
            }

            lines.push(Line::from(vec![
                Span::styled(
                    "[Enter] Save",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("    "),
                Span::styled("[Esc] Cancel", Style::default().fg(Color::DarkGray)),
            ]));

            let p = Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false });
            f.render_widget(p, area);
        }
    }
}
