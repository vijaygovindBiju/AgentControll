use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::{
    app::App,
    views::{session_state_badge, truncate_chars, truncate_display_width},
};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(7)])
        .split(area);

    render_sessions_table(f, app, chunks[0]);
    render_session_quick_info(f, app, chunks[1]);
}

fn render_sessions_table(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = [
        "SESSION ID",
        "AGENT",
        "STATE",
        "PROJECT",
        "ACCOUNT",
        "RESTARTS",
        "TASK",
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

    let rows = app.sessions.iter().enumerate().map(|(idx, s)| {
        let is_selected = idx == app.selected_session;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(40, 45, 60))
        } else {
            Style::default()
        };

        let sid = truncate_chars(&s.id.0, 10);
        let proj = s
            .project_id
            .as_ref()
            .map(|p| truncate_chars(&p.0, 8))
            .unwrap_or_else(|| "-".into());

        let acct_label = app.account_label(s.account_id.as_ref());
        let acct = truncate_display_width(&acct_label, 14);
        let task = truncate_display_width(&s.task_description, 36);

        let row_cells = vec![
            Cell::from(sid).style(if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(s.agent_type.clone()),
            Cell::from(Line::from(vec![session_state_badge(&s.state)])),
            Cell::from(proj),
            Cell::from(acct),
            Cell::from(format!("{}", s.restart_count)),
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
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Min(24),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                format!(" Sessions ({}) ", app.sessions.len()),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
    );

    let mut state = TableState::default();
    if !app.sessions.is_empty() {
        state.select(Some(app.selected_session.min(app.sessions.len() - 1)));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn render_session_quick_info(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Selected Session Actions & Details ",
            Style::default().fg(Color::Gray),
        ));

    if let Some(s) = app.selected_session() {
        let lines = vec![
            Line::from(vec![
                Span::styled("ID: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    &s.id.0,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::styled("State: ", Style::default().fg(Color::DarkGray)),
                session_state_badge(&s.state),
                Span::raw("   "),
                Span::styled("Agent: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&s.agent_type, Style::default().fg(Color::White)),
                Span::raw("   "),
                Span::styled("Account: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    app.account_label(s.account_id.as_ref()),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Task: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&s.task_description, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Actions: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "[n] New Session",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" | "),
                Span::styled("[Space] Run / Resume", Style::default().fg(Color::Green)),
                Span::raw(" | "),
                Span::styled("[w] Switch Account", Style::default().fg(Color::Yellow)),
                Span::raw(" | "),
                Span::styled("[Enter/t] Detail", Style::default().fg(Color::Cyan)),
                Span::raw(" | "),
                Span::styled("[s] Steer", Style::default().fg(Color::Cyan)),
                Span::raw(" | "),
                Span::styled("[p] Pause", Style::default().fg(Color::Blue)),
                Span::raw(" | "),
                Span::styled("[x] Stop", Style::default().fg(Color::Yellow)),
                Span::raw(" | "),
                Span::styled("[d] Remove", Style::default().fg(Color::Red)),
            ]),
        ];
        let p = Paragraph::new(lines).block(block);
        f.render_widget(p, area);
    } else {
        let p = Paragraph::new("No session selected. Use session.create to launch one.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(p, area);
    }
}
