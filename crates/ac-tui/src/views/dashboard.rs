//! Dashboard view rendering.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use ac_core::types::SessionState;
use crate::{
    app::App,
    views::session_state_badge,
};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8)])
        .split(area);

    // ── 1. Top Summary Stat Cards ───────────────────────────────────────────
    render_summary_cards(f, app, main_chunks[0]);

    // ── 2. Split Area: Sessions Table on Left, Inbox & Activity on Right ────
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(main_chunks[1]);

    render_sessions_table(f, app, content_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(content_chunks[1]);

    render_pending_inbox_card(f, app, right_chunks[0]);
    render_recent_activity_card(f, app, right_chunks[1]);
}

fn render_summary_cards(f: &mut Frame, app: &App, area: Rect) {
    let card_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
        ])
        .split(area);

    let total_sessions = app.sessions.len();
    let working = app.sessions.iter().filter(|s| s.state == SessionState::Working).count();
    let waiting_human = app.sessions.iter().filter(|s| s.state == SessionState::WaitingForHuman).count();
    let pending_inbox = app.pending_interactions().len();
    let active_accounts = app.accounts.iter().filter(|a| a.state == ac_core::types::AccountState::Active).count();

    let cards = [
        ("Total Sessions", format!("{total_sessions}"), Color::Cyan),
        ("Working", format!("{working}"), Color::Green),
        ("Waiting Human", format!("{waiting_human}"), if waiting_human > 0 { Color::Yellow } else { Color::DarkGray }),
        ("Pending Inbox", format!("{pending_inbox}"), if pending_inbox > 0 { Color::Red } else { Color::DarkGray }),
        ("Active Accounts", format!("{active_accounts}"), Color::Blue),
    ];

    for (i, (label, val, color)) in cards.iter().enumerate() {
        let p = Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(*label, Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(val, Style::default().fg(*color).add_modifier(Modifier::BOLD)),
            ]),
        ])
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)));
        f.render_widget(p, card_chunks[i]);
    }
}

fn render_sessions_table(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = ["SESSION ID", "AGENT", "STATE", "PROJECT", "TASK"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.sessions.iter().enumerate().map(|(idx, s)| {
        let is_selected = idx == app.selected_session;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(40, 45, 60))
        } else {
            Style::default()
        };

        let sid = if s.id.0.len() > 10 {
            format!("{}...", &s.id.0[..8])
        } else {
            s.id.0.clone()
        };

        let proj = s.project_id.as_ref().map(|p| {
            if p.0.len() > 8 {
                format!("{}...", &p.0[..6])
            } else {
                p.0.clone()
            }
        }).unwrap_or_else(|| "-".into());

        let task = if s.task_description.len() > 30 {
            format!("{}...", &s.task_description[..27])
        } else {
            s.task_description.clone()
        };

        let row_cells = vec![
            Cell::from(sid).style(if is_selected { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::White) }),
            Cell::from(s.agent_type.clone()),
            Cell::from(Line::from(vec![session_state_badge(&s.state)])),
            Cell::from(proj),
            Cell::from(task),
        ];
        Row::new(row_cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(22),
            Constraint::Length(10),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(" Sessions (Enter/t to inspect) ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
    );

    f.render_widget(table, area);
}

fn render_pending_inbox_card(f: &mut Frame, app: &App, area: Rect) {
    let pending = app.pending_interactions();
    let title = format!(" Action Required: Inbox ({}) ", pending.len());
    let border_color = if pending.is_empty() { Color::DarkGray } else { Color::Yellow };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(title, Style::default().fg(border_color).add_modifier(Modifier::BOLD)));

    if pending.is_empty() {
        let p = Paragraph::new("No pending human interactions. All sessions are running or idle.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(p, area);
        return;
    }

    let mut lines = Vec::new();
    for (i, item) in pending.iter().take(6).enumerate() {
        let kind_str = match item.kind {
            ac_core::types::InteractionKind::Question => "Question",
            ac_core::types::InteractionKind::ApprovalRequest => "Approval",
        };
        let detail = item.tool_name.as_deref().unwrap_or(&item.prompt);
        let detail_trunc = if detail.len() > 32 {
            format!("{}...", &detail[..29])
        } else {
            detail.to_string()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{}. [{}] ", i + 1, kind_str), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(detail_trunc, Style::default().fg(Color::White)),
        ]));
    }

    if pending.len() > 6 {
        lines.push(Line::from(Span::styled(
            format!("... and {} more (Press 3 to open Inbox)", pending.len() - 6),
            Style::default().fg(Color::DarkGray),
        )));
    }

    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn render_recent_activity_card(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(" Recent Activity (Press 6 for Activity) ", Style::default().fg(Color::Gray)));

    if app.events.is_empty() {
        let p = Paragraph::new("No recent events.").style(Style::default().fg(Color::DarkGray)).block(block);
        f.render_widget(p, area);
        return;
    }

    let mut lines = Vec::new();
    for event in app.events.iter().take(6) {
        let time_str = event.timestamp.format("%H:%M:%S").to_string();
        let sid = event.session_id.as_ref().map(|s| {
            if s.0.len() > 8 {
                format!("{}..", &s.0[..6])
            } else {
                s.0.clone()
            }
        }).unwrap_or_else(|| "sys".into());

        lines.push(Line::from(vec![
            Span::styled(format!("[{time_str}] "), Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{:<8} ", sid), Style::default().fg(Color::Cyan)),
            Span::styled(format!("{}", event.kind), Style::default().fg(Color::White)),
        ]));
    }

    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}
