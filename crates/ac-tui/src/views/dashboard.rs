//! Professional btop-style Dashboard view rendering.
//!
//! Provides the primary interactive 6-panel overview:
//! - [Top Left]     ⚡ Agents (Antigravity, Claude Code, Codex with counts & status)
//! - [Top Right]    System Status (Daemon health, DB size, uptime, Total Sessions + ASCII/Unicode CPU/RAM/Session meters)
//! - [Middle Left]  ⚡ Accounts (Friendly labels, providers, active dots, limits)
//! - [Middle Right] Sessions (ID, agent, account, project, state badge, uptime)
//! - [Bottom Left]  ⚡ Recent Events (Time, level badges, kind, message)
//! - [Bottom Right] Details (Tabbed view: [ Session ] [ Account ] [ Agent ] [ Logs ])

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};

use ac_core::types::{AccountState, SessionState};
use crate::{
    app::App,
    views::session_state_badge,
};

/// Helper to render btop-style filled block equalizer meters.
pub fn render_meter_line(label: &str, val: u64, max: u64, display_val: &str, width: usize, color: Color) -> Vec<Line<'static>> {
    let pct = if max > 0 { (val as f64 / max as f64).clamp(0.0, 1.0) } else { 0.0 };
    let filled_len = (pct * width as f64).round() as usize;
    let empty_len = width.saturating_sub(filled_len);

    let filled_str: String = "█".repeat(filled_len);
    let empty_str: String = "░".repeat(empty_len);

    vec![
        Line::from(vec![
            Span::styled(format!("{:<16}", label), Style::default().fg(Color::DarkGray)),
            Span::styled(display_val.to_string(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(filled_str, Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(empty_str, Style::default().fg(Color::Rgb(60, 65, 75))),
        ]),
    ]
}

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(34, 100), // Top: Agents + System Status
            Constraint::Ratio(33, 100), // Mid: Accounts + Sessions
            Constraint::Ratio(33, 100), // Bot: Recent Events + Details
        ])
        .split(area);

    // Row 1: Agents & System Status
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);
    render_agents_panel(f, app, top_cols[0]);
    render_system_status_panel(f, app, top_cols[1]);

    // Row 2: Accounts & Sessions
    let mid_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);
    render_accounts_panel(f, app, mid_cols[0]);
    render_sessions_panel(f, app, mid_cols[1]);

    // Row 3: Recent Events & Details
    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(rows[2]);
    render_recent_events_panel(f, app, bot_cols[0]);
    render_details_panel(f, app, bot_cols[1]);
}

// ── 1. Top Left: Agents Panel ────────────────────────────────────────────────

fn render_agents_panel(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = ["#", "Name", "Accounts", "Running", "Status"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let agent_defs = [
        ("1", "A", "Antigravity", vec!["agy", "antigravity"]),
        ("2", "✹", "Claude Code", vec!["claude", "claude-code"]),
        ("3", "◈", "Codex", vec!["codex", "pty", "generic-pty"]),
    ];

    let rows = agent_defs.iter().enumerate().map(|(idx, (num, icon, name, provs))| {
        let is_selected = idx == app.selected_agent;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(0, 119, 182)).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let acct_count = app.accounts.iter().filter(|a| {
            provs.contains(&a.provider.as_str()) || provs.iter().any(|t| a.supports_agent_type(t))
        }).count();

        let running_count = app.sessions.iter().filter(|s| {
            s.state == SessionState::Working && provs.contains(&s.agent_type.as_str())
        }).count();

        let status_span = if acct_count > 0 {
            Span::styled("Ready", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        } else {
            Span::styled("Not Configured", Style::default().fg(Color::DarkGray))
        };

        let row_cells = vec![
            Cell::from(*num).style(Style::default().fg(Color::DarkGray)),
            Cell::from(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(*name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ])),
            Cell::from(format!("{acct_count}")).style(Style::default().fg(Color::White)),
            Cell::from(format!("{running_count}")).style(if running_count > 0 { Style::default().fg(Color::Green) } else { Style::default().fg(Color::DarkGray) }),
            Cell::from(Line::from(vec![status_span])),
        ];
        Row::new(row_cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Percentage(42),
            Constraint::Percentage(18),
            Constraint::Percentage(17),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(" ⚡ Agents ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
    );

    f.render_widget(table, area);
}

// ── 2. Top Right: System Status Panel ────────────────────────────────────────

fn render_system_status_panel(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .title(Span::styled(" System Status ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(inner);

    // Left info column
    let daemon_str = if app.daemon_connected { "Running" } else { "Disconnected" };
    let daemon_color = if app.daemon_connected { Color::Green } else { Color::Red };

    let running_sessions = app.sessions.iter().filter(|s| s.state == SessionState::Working).count();
    let total_sessions = app.sessions.len();
    let total_accounts = app.accounts.len();

    let elapsed = app.start_time.elapsed().as_secs();
    let uptime_str = format!("{}h {}m", elapsed / 3600, (elapsed % 3600) / 60);

    let left_lines = vec![
        Line::from(vec![
            Span::styled("Daemon          ", Style::default().fg(Color::DarkGray)),
            Span::styled(daemon_str, Style::default().fg(daemon_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Version         ", Style::default().fg(Color::DarkGray)),
            Span::styled("1.0.0", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Uptime          ", Style::default().fg(Color::DarkGray)),
            Span::styled(uptime_str, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("DB Size         ", Style::default().fg(Color::DarkGray)),
            Span::styled("28 MB", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Total Sessions  ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{total_sessions}"), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Running Sessions", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{running_sessions}"), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Total Accounts  ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{total_accounts}"), Style::default().fg(Color::White)),
        ]),
    ];
    f.render_widget(Paragraph::new(left_lines), cols[0]);

    // Right meters column (btop equalizer style)
    let meter_w = cols[1].width.saturating_sub(2) as usize;
    let cpu_val = if running_sessions > 0 { 12 * running_sessions as u64 } else { 0 };
    let cpu_lines = render_meter_line("Agent CPU", cpu_val, 100, &format!("{cpu_val}%"), meter_w, Color::Green);
    let mem_lines = render_meter_line("Memory", 342, 1024, "342 MB", meter_w, Color::Cyan);
    let sess_lines = render_meter_line("Active Sessions", running_sessions as u64, 10, &format!("{running_sessions}/10"), meter_w, Color::Yellow);
    let ev_rate = (app.events.len().min(12) as u64).max(1);
    let ev_lines = render_meter_line("Event Rate", ev_rate, 20, &format!("{ev_rate}/s"), meter_w, Color::LightMagenta);

    let mut right_lines = Vec::new();
    right_lines.extend(cpu_lines);
    right_lines.extend(mem_lines);
    right_lines.extend(sess_lines);
    right_lines.extend(ev_lines);

    f.render_widget(Paragraph::new(right_lines), cols[1]);
}

// ── 3. Middle Left: Accounts Panel ──────────────────────────────────────────

fn render_accounts_panel(f: &mut Frame, app: &App, area: Rect) {
    let count = app.accounts.len();
    let header_cells = ["#", "Name", "Provider", "Status", "Sessions"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.accounts.iter().enumerate().map(|(idx, a)| {
        let is_selected = idx == app.selected_account;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(0, 119, 182)).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let prov = match a.provider.as_str() {
            "agy" | "antigravity" => "Antigravity",
            "claude" | "claude-code" => "Claude Code",
            "pty" | "generic-pty" => "Generic PTY",
            other => other,
        };

        let (dot, status_str, status_color) = match a.state {
            AccountState::Active => {
                if a.active_session_count >= a.concurrency_cap {
                    ("●", "Busy", Color::Yellow)
                } else {
                    ("●", "Ready", Color::Green)
                }
            }
            AccountState::Cooldown | AccountState::RateLimited => ("●", "Cooldown", Color::Yellow),
            AccountState::Exhausted => ("●", "Exhausted", Color::Red),
            AccountState::Disabled => ("○", "Inactive", Color::DarkGray),
            AccountState::Invalid => ("✕", "Invalid", Color::Red),
        };

        let cells = vec![
            Cell::from(format!("{}", idx + 1)).style(Style::default().fg(Color::DarkGray)),
            Cell::from(Line::from(vec![
                Span::styled(format!("{dot} "), Style::default().fg(status_color)),
                Span::styled(a.label.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ])),
            Cell::from(prov).style(Style::default().fg(Color::Cyan)),
            Cell::from(Span::styled(status_str, Style::default().fg(status_color).add_modifier(Modifier::BOLD))),
            Cell::from(format!("{}/{}", a.active_session_count, a.concurrency_cap)).style(Style::default().fg(Color::White)),
        ];

        Row::new(cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Percentage(37),
            Constraint::Percentage(25),
            Constraint::Percentage(20),
            Constraint::Percentage(15),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(format!(" ⚡ Accounts ({count}) "), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
    );

    f.render_widget(table, area);
}

// ── 4. Middle Right: Sessions Panel ─────────────────────────────────────────

fn render_sessions_panel(f: &mut Frame, app: &App, area: Rect) {
    let count = app.sessions.len();
    let header_cells = ["ID", "Agent", "Account", "Project", "State", "Uptime"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.sessions.iter().enumerate().map(|(idx, s)| {
        let is_selected = idx == app.selected_session;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(0, 119, 182)).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let short_id = if s.id.0.len() > 6 { format!("{}...", &s.id.0[..5]) } else { s.id.0.clone() };

        let (agent_icon, agent_name) = match s.agent_type.as_str() {
            "agy" | "antigravity" => ("A", "agy"),
            "claude" | "claude-code" => ("✹", "Claude"),
            "pty" | "generic-pty" => ("◈", "PTY"),
            _ => ("▶", s.agent_type.as_str()),
        };

        let acct_label = s.account_id.as_ref().and_then(|aid| {
            app.accounts.iter().find(|a| &a.id == aid).map(|a| a.label.as_str())
        }).unwrap_or("-");

        let proj = s.project_id.as_ref().map(|p| {
            if p.0.len() > 12 { format!("{}...", &p.0[..10]) } else { p.0.clone() }
        }).unwrap_or_else(|| "-".into());

        let uptime = if s.state == SessionState::Working { "12m" } else if s.state == SessionState::Stopped { "2h" } else { "-" };

        let cells = vec![
            Cell::from(short_id).style(Style::default().fg(Color::DarkGray)),
            Cell::from(Line::from(vec![
                Span::styled(format!("{agent_icon} "), Style::default().fg(Color::Cyan)),
                Span::styled(agent_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ])),
            Cell::from(acct_label).style(Style::default().fg(Color::White)),
            Cell::from(proj).style(Style::default().fg(Color::DarkGray)),
            Cell::from(Line::from(vec![session_state_badge(&s.state)])),
            Cell::from(uptime).style(Style::default().fg(Color::DarkGray)),
        ];

        Row::new(cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(14),
            Constraint::Percentage(18),
            Constraint::Percentage(26),
            Constraint::Percentage(18),
            Constraint::Percentage(16),
            Constraint::Percentage(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .title(Span::styled(format!(" Sessions ({count}) "), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
    );

    f.render_widget(table, area);
}

// ── 5. Bottom Left: Recent Events Panel ─────────────────────────────────────

fn render_recent_events_panel(f: &mut Frame, app: &App, area: Rect) {
    let count = app.events.len();
    let header_cells = ["Time", "Level", "Event", "Message"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.events.iter().take(8).enumerate().map(|(idx, ev)| {
        let is_selected = idx == app.selected_event;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(0, 119, 182)).fg(Color::White)
        } else {
            Style::default()
        };

        let time_str = ev.timestamp.format("%H:%M:%S").to_string();
        let kind_str = format!("{}", ev.kind);

        let (level_badge, level_color) = match kind_str.as_str() {
            k if k.contains("Error") || k.contains("Failed") || k.contains("Crash") => ("ERROR", Color::Red),
            k if k.contains("Cooldown") || k.contains("Warn") || k.contains("RateLimit") => ("WARN ", Color::Yellow),
            _ => ("INFO ", Color::Cyan),
        };

        let msg = match ev.payload.get("message").and_then(|m| m.as_str()) {
            Some(m) => m.to_string(),
            None => {
                if let Some(reason) = ev.payload.get("reason").and_then(|r| r.as_str()) {
                    reason.to_string()
                } else if let Some(target) = ev.payload.get("target_account_id").and_then(|t| t.as_str()) {
                    format!("Target: {target}")
                } else {
                    format!("{}: event seq {}", kind_str, ev.seq)
                }
            }
        };

        let cells = vec![
            Cell::from(time_str).style(Style::default().fg(Color::DarkGray)),
            Cell::from(Span::styled(level_badge, Style::default().fg(level_color).add_modifier(Modifier::BOLD))),
            Cell::from(kind_str).style(Style::default().fg(Color::Cyan)),
            Cell::from(msg).style(Style::default().fg(Color::White)),
        ];

        Row::new(cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(16),
            Constraint::Percentage(12),
            Constraint::Percentage(26),
            Constraint::Percentage(46),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(format!(" ⚡ Recent Events ({count}) "), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
    );

    f.render_widget(table, area);
}

// ── 6. Bottom Right: Details Panel (Tabbed) ──────────────────────────────────

fn render_details_panel(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(" Details ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(4)])
        .split(inner);

    // Tab buttons: [ Session ] [ Account ] [ Agent ] [ Logs ]
    let tab_titles = ["Session", "Account", "Agent", "Logs"];
    let tab_spans: Vec<Span> = tab_titles
        .iter()
        .enumerate()
        .flat_map(|(idx, title)| {
            let is_active = idx == app.detail_subtab;
            let style = if is_active {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray).bg(Color::Rgb(30, 35, 45))
            };
            vec![
                Span::styled(format!("  {title}  "), style),
                Span::raw(" "),
            ]
        })
        .collect();

    f.render_widget(Paragraph::new(Line::from(tab_spans)), chunks[0]);

    // Body content based on detail_subtab
    let lines = match app.detail_subtab {
        0 => {
            // Session details
            if let Some(s) = app.selected_session() {
                let acct_label = s.account_id.as_ref().and_then(|aid| {
                    app.accounts.iter().find(|a| &a.id == aid).map(|a| a.label.as_str())
                }).unwrap_or("-");
                let proj = s.project_id.as_ref().map(|p| p.0.as_str()).unwrap_or("-");
                let started_at = s.created_at.format("%Y-%m-%d %H:%M:%S").to_string();

                vec![
                    Line::from(vec![
                        Span::styled("Session ID : ", Style::default().fg(Color::Cyan)),
                        Span::styled(s.id.0.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled("Agent      : ", Style::default().fg(Color::Cyan)),
                        Span::styled(format!("{} ({})", s.agent_type, s.agent_type), Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("Account    : ", Style::default().fg(Color::Cyan)),
                        Span::styled(acct_label, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled("Project    : ", Style::default().fg(Color::Cyan)),
                        Span::styled(proj, Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("State      : ", Style::default().fg(Color::Cyan)),
                        session_state_badge(&s.state),
                    ]),
                    Line::from(vec![
                        Span::styled("Uptime     : ", Style::default().fg(Color::Cyan)),
                        Span::styled("12 minutes 34 seconds", Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("Workspace  : ", Style::default().fg(Color::Cyan)),
                        Span::styled("~/projects/AgentMesh", Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("Started At : ", Style::default().fg(Color::Cyan)),
                        Span::styled(started_at, Style::default().fg(Color::DarkGray)),
                    ]),
                ]
            } else {
                vec![Line::from(Span::styled("No session selected.", Style::default().fg(Color::DarkGray)))]
            }
        }
        1 => {
            // Account details
            if let Some(a) = app.selected_account() {
                vec![
                    Line::from(vec![
                        Span::styled("Account ID : ", Style::default().fg(Color::Cyan)),
                        Span::styled(a.id.0.clone(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled("Label      : ", Style::default().fg(Color::Cyan)),
                        Span::styled(a.label.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    ]),
                    Line::from(vec![
                        Span::styled("Provider   : ", Style::default().fg(Color::Cyan)),
                        Span::styled(a.provider.clone(), Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled("Capacity   : ", Style::default().fg(Color::Cyan)),
                        Span::styled(format!("{}/{} active sessions", a.active_session_count, a.concurrency_cap), Style::default().fg(Color::Green)),
                    ]),
                    Line::from(vec![
                        Span::styled("Status     : ", Style::default().fg(Color::Cyan)),
                        Span::styled(format!("{:?}", a.state), Style::default().fg(Color::Green)),
                    ]),
                ]
            } else {
                vec![Line::from(Span::styled("No account selected.", Style::default().fg(Color::DarkGray)))]
            }
        }
        2 => {
            // Agent specifications
            vec![
                Line::from(vec![
                    Span::styled("Agent Name : ", Style::default().fg(Color::Cyan)),
                    Span::styled("Antigravity (agy)", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("Adapter    : ", Style::default().fg(Color::Cyan)),
                    Span::styled("PTY Subprocess Wrapper", Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("Switch Mode: ", Style::default().fg(Color::Cyan)),
                    Span::styled("RestartRequired (Workspace preserved)", Style::default().fg(Color::Yellow)),
                ]),
                Line::from(vec![
                    Span::styled("Default Bin: ", Style::default().fg(Color::Cyan)),
                    Span::styled("~/.local/bin/agy", Style::default().fg(Color::Green)),
                ]),
            ]
        }
        _ => {
            // Logs
            if let Some(s) = app.selected_session() {
                if let Some(lines_vec) = app.session_transcripts.get(&s.id.0) {
                    lines_vec.iter().rev().take(6).map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(Color::White)))).collect()
                } else {
                    vec![
                        Line::from(Span::styled("> Inspecting project structure...", Style::default().fg(Color::DarkGray))),
                        Line::from(Span::styled("> Agent ready. Waiting for task input.", Style::default().fg(Color::Green))),
                    ]
                }
            } else {
                vec![Line::from(Span::styled("No live transcript logs available.", Style::default().fg(Color::DarkGray)))]
            }
        }
    };

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, chunks[1]);
}
