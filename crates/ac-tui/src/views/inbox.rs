//! Interaction Inbox view rendering.

use chrono::Utc;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};

use ac_core::types::InteractionKind;
use crate::app::App;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),    // Pending table
            Constraint::Length(10), // Selected interaction detail & action controls
        ])
        .split(area);

    render_inbox_table(f, app, chunks[0]);
    render_inbox_detail(f, app, chunks[1]);
}

fn render_inbox_table(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = [
        "ID", "SESSION", "KIND", "TOOL / ACTION", "PROMPT / QUESTION", "WAITING",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let pending = app.pending_interactions();

    let rows = pending.iter().enumerate().map(|(idx, item)| {
        let is_selected = idx == app.selected_interaction;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(40, 45, 60))
        } else {
            Style::default()
        };

        let iid = if item.id.0.len() > 10 {
            format!("{}...", &item.id.0[..8])
        } else {
            item.id.0.clone()
        };

        let sid = if item.session_id.0.len() > 10 {
            format!("{}...", &item.session_id.0[..8])
        } else {
            item.session_id.0.clone()
        };

        let (kind_badge, kind_color) = match item.kind {
            InteractionKind::ApprovalRequest => ("APPROVAL", Color::Yellow),
            InteractionKind::Question => ("QUESTION", Color::Cyan),
        };

        let tool_str = item.tool_name.as_deref().unwrap_or("-");

        let prompt_trunc = if item.prompt.len() > 36 {
            format!("{}...", &item.prompt[..33])
        } else {
            item.prompt.clone()
        };

        let elapsed = Utc::now()
            .signed_duration_since(item.created_at)
            .num_seconds();
        let waiting_str = if elapsed < 60 {
            format!("{elapsed}s")
        } else {
            format!("{}m", elapsed / 60)
        };

        let row_cells = vec![
            Cell::from(iid).style(if is_selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(sid),
            Cell::from(Span::styled(
                format!(" {kind_badge} "),
                Style::default().fg(Color::Black).bg(kind_color).add_modifier(Modifier::BOLD),
            )),
            Cell::from(tool_str).style(Style::default().fg(Color::Yellow)),
            Cell::from(prompt_trunc),
            Cell::from(waiting_str).style(Style::default().fg(Color::Gray)),
        ];
        Row::new(row_cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Length(18),
            Constraint::Min(24),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if pending.is_empty() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Yellow)
            })
            .title(Span::styled(
                format!(" Pending Interactions ({}) ", pending.len()),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(table, area);
}

fn render_inbox_detail(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(" Interaction Decision Controls ", Style::default().fg(Color::White)));

    if let Some(item) = app.selected_pending_interaction() {
        let kind_str = match item.kind {
            InteractionKind::ApprovalRequest => "Approval Request",
            InteractionKind::Question => "Question",
        };

        let tool_info = if let Some(t) = &item.tool_name {
            format!(" | Tool: {}", t)
        } else {
            "".to_string()
        };

        let args_str = item
            .tool_args
            .as_ref()
            .map(|a| serde_json::to_string(a).unwrap_or_default())
            .unwrap_or_else(|| "None".into());

        let lines = vec![
            Line::from(vec![
                Span::styled("Type: ", Style::default().fg(Color::DarkGray)),
                Span::styled(kind_str, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(tool_info, Style::default().fg(Color::Cyan)),
                Span::raw("    "),
                Span::styled("Session ID: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&item.session_id.0, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Prompt: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&item.prompt, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Tool Args: ", Style::default().fg(Color::DarkGray)),
                Span::styled(args_str, Style::default().fg(Color::Gray)),
            ]),
            Line::from(vec![
                Span::styled("Actions: ", Style::default().fg(Color::DarkGray)),
                Span::styled("  [a] Approve  ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                Span::styled("  [d] Deny  ", Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                Span::styled("  [r] Reply (with text)  ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                Span::styled("  [x] Dismiss  ", Style::default().fg(Color::White).bg(Color::DarkGray)),
            ]),
        ];
        f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: true }), area);
    } else {
        let p = Paragraph::new("No pending interactions requiring human action.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(p, area);
    }
}
