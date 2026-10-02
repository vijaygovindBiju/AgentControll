//! Settings view: central configuration, account management, project management,
//! agent capabilities, models, permissions, authentication, terminal, and security.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

use crate::{
    app::{App, SettingsSection},
    views::{account_state_badge, truncate_chars, truncate_display_width},
};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(26), Constraint::Min(40)])
        .split(area);

    render_sections_list(f, app, chunks[0]);
    render_section_content(f, app, chunks[1]);
}

fn render_sections_list(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = !app.settings_focus_panel;
    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            if is_focused {
                " ▸ Settings "
            } else {
                " Settings "
            },
            Style::default()
                .fg(if is_focused {
                    Color::Cyan
                } else {
                    Color::White
                })
                .add_modifier(Modifier::BOLD),
        ));

    let mut lines = vec![Line::from("")];
    for (i, section) in SettingsSection::ALL.iter().enumerate() {
        let is_selected = i == app.settings_section_index;
        let prefix = if is_selected { " > " } else { "   " };
        let style = if is_selected {
            if is_focused {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            }
        } else {
            Style::default().fg(Color::White)
        };

        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(section.title(), style),
        ]));
        lines.push(Line::from(""));
    }

    let hint = if is_focused {
        Line::from(Span::styled(
            " ↑↓/jk select\n Enter/→ focus",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(Span::styled(
            " Esc/← return",
            Style::default().fg(Color::DarkGray),
        ))
    };
    lines.push(Line::from(""));
    lines.push(hint);

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_section_content(f: &mut Frame, app: &App, area: Rect) {
    match app.current_settings_section() {
        SettingsSection::General => render_general_section(f, app, area),
        SettingsSection::Accounts => render_accounts_section(f, app, area),
        SettingsSection::Projects => render_projects_section(f, app, area),
        SettingsSection::Agents => render_agents_section(f, app, area),
        SettingsSection::Models => render_models_section(f, app, area),
        SettingsSection::Permissions => render_permissions_section(f, app, area),
        SettingsSection::Authentication => render_authentication_section(f, app, area),
        SettingsSection::Terminal => render_terminal_section(f, app, area),
        SettingsSection::Security => render_security_section(f, app, area),
    }
}

// ── 1. General Section ──────────────────────────────────────────────────────────

fn render_general_section(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.settings_focus_panel;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
        .title(Span::styled(
            " ⚙ General Settings & Global Defaults ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let dim = Style::default().fg(Color::DarkGray);
    let item = |idx: usize, label: &'static str, val: String, desc: &'static str| {
        let focused = is_focused && app.settings_general_item == idx;
        vec![
            Line::from(vec![
                Span::styled(
                    if focused { " > " } else { "   " },
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    label,
                    if focused {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    },
                ),
                Span::styled(" : ", dim),
                Span::styled(val, Style::default().fg(Color::Cyan)),
                Span::styled(if focused { "   [←/→ change]" } else { "" }, dim),
            ]),
            Line::from(vec![Span::raw("     "), Span::styled(desc, dim)]),
            Line::from(""),
        ]
    };

    let default_acc_label = app
        .user_settings
        .default_account
        .as_deref()
        .unwrap_or("None (manual selection)");
    let default_dir_label = app
        .user_settings
        .default_working_dir
        .as_deref()
        .unwrap_or("None (current process dir)");

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " Global defaults applied when starting new agent sessions:",
            Style::default().fg(Color::White),
        )),
        Line::from(""),
    ];

    lines.extend(item(
        0,
        "Default Agent",
        app.user_settings.default_agent.clone(),
        "Agent engine selected when creating a new session",
    ));
    lines.extend(item(
        1,
        "Default Account",
        default_acc_label.to_string(),
        "Pre-selected account on the launch screen",
    ));
    lines.extend(item(
        2,
        "Default Working Dir",
        default_dir_label.to_string(),
        "Pre-filled working directory for sessions (press 'w' to edit)",
    ));
    lines.extend(item(
        3,
        "Default Execution Mode",
        app.user_settings.default_execution_mode.label().to_string(),
        "Standard AGY execution mode (Default, Accept Edits, Plan)",
    ));
    lines.extend(item(
        4,
        "Default Permission Mode",
        app.user_settings
            .default_permission_mode
            .label()
            .to_string(),
        "Normal AGY permissions or Dangerously Skip Permissions",
    ));
    lines.extend(item(
        5,
        "Terminal Theme",
        app.user_settings.terminal_theme.clone(),
        "Interface color palette and accent style",
    ));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " [Persisted to ~/.config/agentcontrol/settings.json]",
        Style::default().fg(Color::Green),
    )));
    lines.push(Line::from(Span::styled(
        " Hotkeys: ↑↓ select item  ←/→ cycle option  w change working dir  Esc return",
        dim,
    )));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── 2. Accounts Section ────────────────────────────────────────────────────────

fn render_accounts_section(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(9)])
        .split(area);

    let is_focused = app.settings_focus_panel;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
        .title(Span::styled(
            format!(" Accounts ({}) ", app.accounts.len()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let header_cells = [
        "ACCOUNT ID",
        "LABEL",
        "PROVIDER",
        "STATE",
        "LOAD / CAP",
        "TAGS",
    ]
    .iter()
    .map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let selected_idx = if app.accounts.is_empty() {
        0
    } else {
        app.settings_account_selected.min(app.accounts.len() - 1)
    };

    let rows = app.accounts.iter().enumerate().map(|(idx, a)| {
        let is_selected = idx == selected_idx;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(40, 45, 60))
        } else {
            Style::default()
        };

        let aid = truncate_chars(&a.id.0, 10);
        let load_str = format!("{}/{}", a.active_session_count, a.concurrency_cap);
        let tags_str = if a.tags.is_empty() {
            "-".to_string()
        } else {
            truncate_display_width(&a.tags.join(", "), 16)
        };
        let is_default = app.user_settings.default_account.as_deref() == Some(&a.label)
            || app.user_settings.default_account.as_deref() == Some(&a.id.0);
        let label_display = if is_default {
            truncate_display_width(&format!("★ {}", a.label), 24)
        } else {
            truncate_display_width(&a.label, 24)
        };

        let row_cells = vec![
            Cell::from(aid).style(if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(label_display).style(if is_default {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(a.provider.clone()),
            Cell::from(Line::from(vec![account_state_badge(&a.state)])),
            Cell::from(load_str),
            Cell::from(tags_str),
        ];
        Row::new(row_cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(24),
            Constraint::Length(14),
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Min(16),
        ],
    )
    .header(header)
    .block(block);

    let mut state = TableState::default();
    if !app.accounts.is_empty() {
        state.select(Some(selected_idx));
    }
    f.render_stateful_widget(table, chunks[0], &mut state);

    // Detail Panel
    render_account_detail_panel(f, app, chunks[1]);
}

fn render_account_detail_panel(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Selected Account Details & Actions ",
            Style::default().fg(Color::Cyan),
        ));

    let dim = Style::default().fg(Color::DarkGray);

    if let Some(account) = app.accounts.get(app.settings_account_selected) {
        let is_default = app.user_settings.default_account.as_deref() == Some(&account.label)
            || app.user_settings.default_account.as_deref() == Some(&account.id.0);
        let default_badge = if is_default {
            Span::styled(
                " [★ DEFAULT ACCOUNT] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(" [Press 's' to set as Default] ", dim)
        };

        let lines = vec![
            Line::from(vec![
                Span::styled("Label: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    account.label.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                default_badge,
                Span::raw("   "),
                account_state_badge(&account.state),
            ]),
            Line::from(vec![
                Span::styled("Account ID: ", Style::default().fg(Color::Cyan)),
                Span::styled(account.id.0.clone(), Style::default().fg(Color::White)),
                Span::styled("   Provider: ", Style::default().fg(Color::Cyan)),
                Span::styled(account.provider.clone(), Style::default().fg(Color::White)),
                Span::styled("   Active Sessions: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!(
                        "{}/{}",
                        account.active_session_count, account.concurrency_cap
                    ),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("Credentials Ref: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!(
                        "~/.config/agentcontrol/credentials/{}",
                        account.credential_ref
                    ),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Actions: ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "[a] Add Account (Browser / Login Link)  ",
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled("[d] Remove Account  ", Style::default().fg(Color::Red)),
                Span::styled("[s] Set as Default  ", Style::default().fg(Color::Yellow)),
                Span::styled("[r] Refresh", Style::default().fg(Color::White)),
            ]),
        ];
        f.render_widget(Paragraph::new(lines).block(block), area);
    } else {
        let p = Paragraph::new(vec![
            Line::from("No accounts registered yet."),
            Line::from(Span::styled(
                "Press [a] to add an Antigravity account via Browser Login or Login Link.",
                Style::default().fg(Color::Cyan),
            )),
        ])
        .block(block);
        f.render_widget(p, area);
    }
}

// ── 3. Projects & Workspaces Section ───────────────────────────────────────────

fn render_projects_section(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(9)])
        .split(area);

    let is_focused = app.settings_focus_panel;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if is_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        }))
        .title(Span::styled(
            format!(" Registered Projects ({}) ", app.projects.len()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let header_cells = [
        "PROJECT ID",
        "NAME",
        "REPO PATH",
        "WORKSPACE POLICY",
        "ACTIVE SESSIONS",
    ]
    .iter()
    .map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let selected_idx = if app.projects.is_empty() {
        0
    } else {
        app.settings_project_selected.min(app.projects.len() - 1)
    };

    let rows = app.projects.iter().enumerate().map(|(idx, p)| {
        let is_selected = idx == selected_idx;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(40, 45, 60))
        } else {
            Style::default()
        };

        let pid = truncate_chars(&p.id.0, 10);
        let is_default = app.user_settings.default_project.as_deref() == Some(&p.name)
            || app.user_settings.default_project.as_deref() == Some(&p.id.0);
        let name_display = if is_default {
            truncate_display_width(&format!("★ {}", p.name), 22)
        } else {
            truncate_display_width(&p.name, 22)
        };
        let repo_path = truncate_display_width(&p.repo_path, 32);
        let active_count = app
            .sessions
            .iter()
            .filter(|s| s.project_id.as_ref() == Some(&p.id))
            .count();

        let row_cells = vec![
            Cell::from(pid).style(if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(name_display).style(if is_default {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(repo_path),
            Cell::from(format!("{}", p.workspace_policy)),
            Cell::from(format!("{active_count}")),
        ];
        Row::new(row_cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(22),
            Constraint::Length(32),
            Constraint::Length(20),
            Constraint::Min(16),
        ],
    )
    .header(header)
    .block(block);

    let mut state = TableState::default();
    if !app.projects.is_empty() {
        state.select(Some(selected_idx));
    }
    f.render_stateful_widget(table, chunks[0], &mut state);

    render_project_detail_panel(f, app, chunks[1]);
}

fn render_project_detail_panel(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Project Configuration & Actions ",
            Style::default().fg(Color::Cyan),
        ));

    let dim = Style::default().fg(Color::DarkGray);
    let def_dir = app
        .user_settings
        .default_working_dir
        .as_deref()
        .unwrap_or("(none configured)");

    if let Some(project) = app.projects.get(app.settings_project_selected) {
        let is_default = app.user_settings.default_project.as_deref() == Some(&project.name)
            || app.user_settings.default_project.as_deref() == Some(&project.id.0);
        let default_badge = if is_default {
            Span::styled(
                " [★ DEFAULT PROJECT] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(" [Press 's' to set as Default] ", dim)
        };

        let lines = vec![
            Line::from(vec![
                Span::styled("Project: ", Style::default().fg(Color::Cyan)),
                Span::styled(project.name.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                default_badge,
                Span::styled("   Default Working Dir: ", Style::default().fg(Color::Cyan)),
                Span::styled(def_dir, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Repository: ", Style::default().fg(Color::Cyan)),
                Span::styled(project.repo_path.clone(), Style::default().fg(Color::White)),
                Span::styled("   Workspace Policy: ", Style::default().fg(Color::Cyan)),
                Span::styled(format!("{}", project.workspace_policy), Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![
                Span::styled("Worktree Config: ", Style::default().fg(Color::Cyan)),
                Span::styled("Isolated worktrees create separate branches; sessions are free to edit without colliding.", dim),
            ]),
            Line::from(vec![
                Span::styled("Actions: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled("[a] Register Project  ", Style::default().fg(Color::Cyan)),
                Span::styled("[d] Remove Project  ", Style::default().fg(Color::Red)),
                Span::styled("[s] Set as Default Project  ", Style::default().fg(Color::Yellow)),
                Span::styled("[w] Set Default Working Directory", Style::default().fg(Color::White)),
            ]),
        ];
        f.render_widget(Paragraph::new(lines).block(block), area);
    } else {
        let lines = vec![
            Line::from("No projects registered in Agent Control."),
            Line::from(vec![
                Span::styled("Default Working Dir: ", Style::default().fg(Color::Cyan)),
                Span::styled(def_dir, Style::default().fg(Color::White)),
                Span::styled("  [Press 'w' to change]", dim),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Press [a] to register a project from an existing Git repository.",
                Style::default().fg(Color::Cyan),
            )),
        ];
        f.render_widget(Paragraph::new(lines).block(block), area);
    }
}

// ── 4. Agents Section ──────────────────────────────────────────────────────────

fn render_agents_section(f: &mut Frame, _app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Agent Engines & Integration Status ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let agy_bin = ac_core::agy_auth::find_agy_binary()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "not found".to_string());

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " 1. Antigravity (`agy` CLI)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                "    Status:          ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "● Installed and verified",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" (v1.2.13)", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(
                "    Binary:          ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(agy_bin, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(
                "    Execution Modes: ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "default, accept-edits (--mode=accept-edits), plan (--mode=plan)",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "    Permission Flag: ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "--dangerously-skip-permissions (Auto-approve tool permission requests)",
                Style::default().fg(Color::Yellow),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "    Sandbox Flag:    ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "--sandbox (Run in sandbox with terminal restrictions enabled)",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "    Profile Isolation:",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "Isolated HOME ~/.config/agentcontrol/profiles/<account-id>/ per session",
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " 2. Claude Code (`claude`)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                "    Status:          ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "Supported via Anthropic API and interactive PTY adapter",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " 3. Generic PTY (`generic-pty`)",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                "    Status:          ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                "Universal pseudo-terminal execution for any command-line agent",
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── 5. Models Section ──────────────────────────────────────────────────────────

fn render_models_section(f: &mut Frame, _app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Antigravity Model Catalog ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let models = [
        (
            "gemini-3.8-flash-high",
            "Gemini 3.8 Flash (High)",
            "Recommended default for reasoning and coding",
        ),
        (
            "gemini-3.8-flash-medium",
            "Gemini 3.8 Flash (Medium)",
            "Balanced latency and performance",
        ),
        (
            "gemini-3.8-flash-low",
            "Gemini 3.8 Flash (Low)",
            "Low reasoning effort for rapid responses",
        ),
        (
            "gemini-3.7-flash-high",
            "Gemini 3.7 Flash (High)",
            "Previous stable flash generation",
        ),
        (
            "gemini-3.1-pro-high",
            "Gemini 3.1 Pro (High)",
            "Deep reasoning model for architecture",
        ),
        (
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6 (Thinking)",
            "Anthropic Sonnet model via Antigravity",
        ),
        (
            "claude-opus-4-6-thinking",
            "Claude Opus 4.6 (Thinking)",
            "High capacity reasoning and synthesis",
        ),
        (
            "gpt-oss-120b-medium",
            "GPT-OSS 120B (Medium)",
            "Open weights model integration",
        ),
    ];

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " Available models discovered from Antigravity engine:",
            Style::default().fg(Color::White),
        )),
        Line::from(""),
    ];

    for (id, name, desc) in models {
        lines.push(Line::from(vec![
            Span::styled("   • ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{name:<30}"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("({id})"), Style::default().fg(Color::Yellow)),
        ]));
        lines.push(Line::from(vec![
            Span::raw("     "),
            Span::styled(desc, Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(""));
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── 6. Permissions Section ────────────────────────────────────────────────────

fn render_permissions_section(f: &mut Frame, _app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Execution Modes & Permission / Access Controls ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let dim = Style::default().fg(Color::DarkGray);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(" A. AGY EXECUTION MODE", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("    Controls how Antigravity approaches tasks during a session:", dim)),
        Line::from(vec![
            Span::styled("    • Default:      ", Style::default().fg(Color::Yellow)),
            Span::styled("Standard AGY interactive flow (no mode override).", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("    • Accept Edits: ", Style::default().fg(Color::Yellow)),
            Span::styled("--mode=accept-edits  (Auto-approve file changes, still prompts for shell commands)", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("    • Plan:         ", Style::default().fg(Color::Yellow)),
            Span::styled("--mode=plan          (Research and plan only, makes no modifications)", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(" B. AGY PERMISSION / ACCESS CONTROL", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("    Controls whether tool permission prompts require human approval:", dim)),
        Line::from(vec![
            Span::styled("    • Normal:       ", Style::default().fg(Color::Green)),
            Span::styled("Standard permissions. AGY prompts before modifying files or executing commands.", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("    • Dangerous:    ", Style::default().fg(Color::Red)),
            Span::styled("--dangerously-skip-permissions  (Auto-approves tool requests without prompting).", Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled("      Protected by mandatory confirmation modal on session launch.", Style::default().fg(Color::Red))),
        Line::from(""),
        Line::from(Span::styled(" C. SANDBOX ISOLATION", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("    • Sandbox:      ", Style::default().fg(Color::Yellow)),
            Span::styled("--sandbox  (Run with terminal restrictions enabled)", Style::default().fg(Color::White)),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── 7. Authentication Section ─────────────────────────────────────────────────

fn render_authentication_section(f: &mut Frame, _app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Antigravity Authentication Architecture ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let creds_dir = ac_core::agy_auth::agentcontrol_config_dir().join("credentials");
    let profiles_dir = ac_core::agy_auth::agentcontrol_config_dir().join("profiles");

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            " Login Mechanisms Supported:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                " 1. Login with Browser: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Starts a local loopback server on 127.0.0.1 and opens the system browser.",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                " 2. Generate Login Link: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Produces a short-lived OAuth 2.0 PKCE link for remote or headless machines.",
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(Span::styled(
            "    The user signs in on any device and pastes the redirect callback URL back.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Security & Isolation Boundaries:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" • Credential Storage: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{} (File permissions 0600)", creds_dir.display()),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::styled(" • Account Profiles:    ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!(
                    "{} (Dedicated isolated HOME per account)",
                    profiles_dir.display()
                ),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::styled(" • Ambient Auth:       ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "Stripped on spawn (GEMINI_API_KEY, GOOGLE_APPLICATION_CREDENTIALS)",
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " Quick Actions: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "[b] Launch Browser Login   ",
                Style::default().fg(Color::Cyan),
            ),
            Span::styled("[l] Generate Login Link", Style::default().fg(Color::Cyan)),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── 8. Terminal Section ───────────────────────────────────────────────────────

fn render_terminal_section(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Terminal Emulation & Display Settings ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" Theme:               ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(app.user_settings.terminal_theme.clone(), Style::default().fg(Color::White)),
            Span::styled("   [Default / Dark / High-Contrast]", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(" Scrollback Limit:    ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{} lines", app.user_settings.terminal_scrollback), Style::default().fg(Color::White)),
            Span::styled("   [Fixed memory bounded ring buffer]", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(" Cursor Style:        ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(app.user_settings.terminal_cursor.clone(), Style::default().fg(Color::White)),
            Span::styled("   [Block / Bar / Underline]", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled(" PTY & Virtual Screen Emulation:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled(" • UTF-8 Stream:      ", Style::default().fg(Color::Cyan)),
            Span::styled("decode_utf8_stream decodes split bytes; multi-byte glyphs are never replaced with U+FFFD", Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled(" • Alternate Screen:  ", Style::default().fg(Color::Cyan)),
            Span::styled("Preserves full-screen interactive grids (btop, vim, agy interactive panels)", Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled(" • Dynamic Resize:    ", Style::default().fg(Color::Cyan)),
            Span::styled("Clamps cursor and synchronizes grid with host terminal viewport", Style::default().fg(Color::Green)),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

// ── 9. Security Section ───────────────────────────────────────────────────────

fn render_security_section(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Security Audit & Enforced Boundaries ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let daemon_state = if app.daemon_connected {
        Span::styled(
            "● Connected (Active)",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "○ Disconnected",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    };

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(" Daemon Status:       ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            daemon_state,
        ]),
        Line::from(vec![
            Span::styled(" Socket Path:         ", Style::default().fg(Color::Cyan)),
            Span::styled("/run/user/1000/agentcontrol.sock (0700 current UID only)", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(" Enforced Security Invariants:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled(" [T1] Zero Secret Leakage:      ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("OAuth tokens and API keys are NEVER logged or displayed in TUI", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" [T2] Account Isolation:        ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("Each session spawns with its own HOME profile; configs cannot leak across accounts", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" [T3] Workspace Boundaries:     ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("Session paths are validated against allowed directories; path traversal blocked", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" [T4] Permission Gatekeeper:    ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled("Dangerous permission overrides require explicit human confirmation dialog", Style::default().fg(Color::White)),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}
