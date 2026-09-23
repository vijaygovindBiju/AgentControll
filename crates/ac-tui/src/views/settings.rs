//! Settings view rendering.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::App;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(" ⚙ System Settings & Environment ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));

    let daemon_state = if app.daemon_connected {
        Span::styled("● Connected (Active)", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("○ Disconnected", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Daemon Status:    ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            daemon_state,
        ]),
        Line::from(vec![
            Span::styled("Control Socket:   ", Style::default().fg(Color::Cyan)),
            Span::styled("/run/user/1000/agentcontrol.sock", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("WebSocket API:    ", Style::default().fg(Color::Cyan)),
            Span::styled("ws://127.0.0.1:4242", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled("Data Stores & Persistence (SQLite):", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  Events DB:      ", Style::default().fg(Color::DarkGray)),
            Span::styled("~/.local/share/agentcontrol/events.db (Append-only hash store)", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Accounts DB:    ", Style::default().fg(Color::DarkGray)),
            Span::styled("~/.local/share/agentcontrol/accounts.db", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Projects DB:    ", Style::default().fg(Color::DarkGray)),
            Span::styled("~/.local/share/agentcontrol/projects.db", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Policies DB:    ", Style::default().fg(Color::DarkGray)),
            Span::styled("~/.local/share/agentcontrol/policies.db", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("  Interactions DB:", Style::default().fg(Color::DarkGray)),
            Span::styled("~/.local/share/agentcontrol/interactions.db", Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled("Security & Credentials:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  Credentials Dir:", Style::default().fg(Color::DarkGray)),
            Span::styled("~/.config/agentcontrol/credentials/ (0600 file permissions)", Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("  Zero Secret Leak:", Style::default().fg(Color::DarkGray)),
            Span::styled("Tokens are never emitted over event log, API, or TUI display", Style::default().fg(Color::Green)),
        ]),
    ];

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(p, area);
}
