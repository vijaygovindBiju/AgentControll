//! Agents view rendering.
//!
//! Provides a dedicated view of supported coding agents:
//! - Antigravity (agy)
//! - Claude Code (claude)
//! - Codex / Generic PTY (codex / pty)

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::app::App;
use ac_core::types::SessionState;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(8)])
        .split(area);

    render_agents_table(f, app, chunks[0]);
    render_agent_detail(f, app, chunks[1]);
}

fn render_agents_table(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = [
        "#",
        "AGENT NAME",
        "TYPE",
        "ACCOUNTS",
        "RUNNING",
        "STATUS",
        "SWITCH CAPABILITY",
    ]
    .iter()
    .map(|h| {
        Cell::from(*h).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let agent_defs = [
        (
            "1",
            "Antigravity",
            "agy",
            vec!["agy", "antigravity"],
            "RestartRequired",
            "Ready",
        ),
        (
            "2",
            "Claude Code",
            "claude",
            vec!["claude", "claude-code"],
            "RestartRequired",
            "Ready",
        ),
        (
            "3",
            "Codex / Generic PTY",
            "pty",
            vec!["pty", "generic-pty", "codex"],
            "None",
            "Ready",
        ),
    ];

    let rows =
        agent_defs
            .iter()
            .enumerate()
            .map(|(idx, (num, name, code, provs, switch_mode, _))| {
                let is_selected = idx == app.selected_agent;
                let style = if is_selected {
                    Style::default()
                        .bg(Color::Rgb(0, 119, 182))
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let acct_count = app
                    .accounts
                    .iter()
                    .filter(|a| {
                        provs.contains(&a.provider.as_str())
                            || provs.iter().any(|t| a.supports_agent_type(t))
                    })
                    .count();

                let running_count = app
                    .sessions
                    .iter()
                    .filter(|s| {
                        s.state == SessionState::Working && provs.contains(&s.agent_type.as_str())
                    })
                    .count();

                let status_badge = if acct_count > 0 {
                    Span::styled(
                        " Ready ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(
                        " Not Configured ",
                        Style::default().fg(Color::DarkGray).bg(Color::Reset),
                    )
                };

                let cells = vec![
                    Cell::from(*num).style(Style::default().fg(Color::DarkGray)),
                    Cell::from(*name).style(
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Cell::from(*code).style(Style::default().fg(Color::Cyan)),
                    Cell::from(format!("{acct_count}")).style(Style::default().fg(Color::White)),
                    Cell::from(format!("{running_count}")).style(if running_count > 0 {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }),
                    Cell::from(Line::from(vec![status_badge])),
                    Cell::from(*switch_mode).style(Style::default().fg(Color::Yellow)),
                ];

                Row::new(cells).style(style).height(1)
            });

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(22),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(18),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " ⚡ AGENTS ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(table, area);
}

fn render_agent_detail(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Agent Specifications & Capabilities ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));

    let lines = match app.selected_agent {
        0 => vec![
            Line::from(vec![
                Span::styled("Name:             ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Antigravity (agy)", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Adapter Type:     ", Style::default().fg(Color::Cyan)),
                Span::styled("PTY Subprocess Wrapper (ANSI terminal streaming)", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Default Binary:   ", Style::default().fg(Color::Cyan)),
                Span::styled("~/.local/bin/agy (or system PATH)", Style::default().fg(Color::Yellow)),
            ]),
            Line::from(vec![
                Span::styled("Supported Auth:   ", Style::default().fg(Color::Cyan)),
                Span::styled("Google OAuth 2.0 (Loopback IP flow), ~/.gemini session token import", Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![
                Span::styled("Account Switching:", Style::default().fg(Color::Cyan)),
                Span::styled("RestartRequired — Workspace is preserved, process environment reloads with new credentials", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Capabilities:     ", Style::default().fg(Color::Cyan)),
                Span::styled("Live PTY streaming, interactive human steering, safe snapshots, graceful stop", Style::default().fg(Color::White)),
            ]),
        ],
        1 => vec![
            Line::from(vec![
                Span::styled("Name:             ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Claude Code", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Adapter Type:     ", Style::default().fg(Color::Cyan)),
                Span::styled("Structured Stream NDJSON / PTY process", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Supported Auth:   ", Style::default().fg(Color::Cyan)),
                Span::styled("Anthropic API Key, OAuth session", Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![
                Span::styled("Account Switching:", Style::default().fg(Color::Cyan)),
                Span::styled("RestartRequired / Dynamic quota switch", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Capabilities:     ", Style::default().fg(Color::Cyan)),
                Span::styled("Automated policy evaluation, tool approval interception, subagent dispatch", Style::default().fg(Color::White)),
            ]),
        ],
        _ => vec![
            Line::from(vec![
                Span::styled("Name:             ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled("Codex / Generic PTY Runner", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Adapter Type:     ", Style::default().fg(Color::Cyan)),
                Span::styled("Generic Portable PTY Process Runner", Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Supported Auth:   ", Style::default().fg(Color::Cyan)),
                Span::styled("Environment variables, custom credential references", Style::default().fg(Color::Green)),
            ]),
            Line::from(vec![
                Span::styled("Capabilities:     ", Style::default().fg(Color::Cyan)),
                Span::styled("Arbitrary shell agent execution, raw terminal capture, signal management", Style::default().fg(Color::White)),
            ]),
        ],
    };

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}
