//! Event activity log view rendering.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::app::App;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Filter status bar
            Constraint::Min(8),    // Events table
            Constraint::Length(6), // Selected event payload preview
        ])
        .split(area);

    render_filter_bar(f, app, chunks[0]);
    render_activity_table(f, app, chunks[1]);
    render_event_detail(f, app, chunks[2]);
}

fn render_filter_bar(f: &mut Frame, app: &App, area: Rect) {
    let filter_text = app
        .activity_filter
        .as_deref()
        .map(|f| format!("Active Filter: \"{f}\" (Press 'c' to clear)"))
        .unwrap_or_else(|| "No active filter (Press '/' to filter by session/kind/trigger)".into());

    let p = Paragraph::new(Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(filter_text, Style::default().fg(if app.activity_filter.is_some() { Color::Yellow } else { Color::DarkGray })),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Span::styled(" Filter Activity ", Style::default().fg(Color::Gray))),
    );
    f.render_widget(p, area);
}

fn render_activity_table(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = ["SEQ", "TIME", "KIND", "SESSION", "TRIGGERED BY", "PAYLOAD"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let events = app.filtered_events();

    let rows = events.iter().enumerate().map(|(idx, e)| {
        let is_selected = idx == app.selected_event;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(40, 45, 60))
        } else {
            Style::default()
        };

        let sid = e
            .session_id
            .as_ref()
            .map(|s| {
                if s.0.len() > 10 {
                    format!("{}...", &s.0[..8])
                } else {
                    s.0.clone()
                }
            })
            .unwrap_or_else(|| "-".into());

        let payload_str = serde_json::to_string(&e.payload).unwrap_or_default();
        let payload_trunc = if payload_str.len() > 40 {
            format!("{}...", &payload_str[..37])
        } else {
            payload_str
        };

        let time_str = e.timestamp.format("%H:%M:%S").to_string();

        let (kind_color, is_bold) = match e.kind {
            ac_core::types::EventKind::SessionCrashed | ac_core::types::EventKind::SessionFailed => (Color::Red, true),
            ac_core::types::EventKind::ApprovalRequested | ac_core::types::EventKind::AgentQuestion => (Color::Yellow, true),
            ac_core::types::EventKind::SessionStarted | ac_core::types::EventKind::SessionReady => (Color::Green, false),
            ac_core::types::EventKind::StateChanged => (Color::Cyan, false),
            _ => (Color::White, false),
        };

        let mut kind_style = Style::default().fg(kind_color);
        if is_bold {
            kind_style = kind_style.add_modifier(Modifier::BOLD);
        }

        let row_cells = vec![
            Cell::from(format!("{}", e.seq)).style(Style::default().fg(Color::DarkGray)),
            Cell::from(time_str).style(Style::default().fg(Color::Gray)),
            Cell::from(format!("{}", e.kind)).style(kind_style),
            Cell::from(sid).style(if is_selected { Style::default().fg(Color::Cyan) } else { Style::default() }),
            Cell::from(e.triggered_by.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(payload_trunc),
        ];
        Row::new(row_cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(24),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Min(30),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                format!(" Activity Stream ({} events) ", events.len()),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(table, area);
}

fn render_event_detail(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(" Event Payload Detail ", Style::default().fg(Color::Gray)));

    let events = app.filtered_events();
    if let Some(e) = events.get(app.selected_event) {
        let formatted_payload = serde_json::to_string_pretty(&e.payload).unwrap_or_default();
        let lines = vec![
            Line::from(vec![
                Span::styled("Event ID: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&e.id.0, Style::default().fg(Color::Cyan)),
                Span::raw("   "),
                Span::styled("Seq: ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", e.seq), Style::default().fg(Color::White)),
                Span::raw("   "),
                Span::styled("Kind: ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", e.kind), Style::default().fg(Color::Yellow)),
                Span::raw("   "),
                Span::styled("Triggered By: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&e.triggered_by, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Payload: ", Style::default().fg(Color::DarkGray)),
                Span::styled(formatted_payload, Style::default().fg(Color::White)),
            ]),
        ];
        f.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
    } else {
        let p = Paragraph::new("No event selected.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(p, area);
    }
}
