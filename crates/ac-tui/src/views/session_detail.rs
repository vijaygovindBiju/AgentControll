//! Session detail view rendering with a dedicated, ANSI-safe Agent Terminal.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::{
    app::App,
    terminal_buffer::TerminalBuffer,
    views::session_state_badge,
};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let session = match app.selected_session_or_detail() {
        Some(s) => s,
        None => {
            let p = Paragraph::new("Session not found or has been removed. Press Esc to return.")
                .style(Style::default().fg(Color::Red))
                .block(Block::default().borders(Borders::ALL).title(" Session Detail "));
            f.render_widget(p, area);
            return;
        }
    };

    // Session-specific pending interactions
    let session_pending: Vec<_> = app
        .pending_interactions()
        .into_iter()
        .filter(|i| i.session_id == session.id)
        .collect();

    let interactions_height = if session_pending.is_empty() {
        3u16
    } else {
        (session_pending.len() as u16 + 2).min(5)
    };

    // Redesigned structured layout according to section 2:
    // 1. Compact Session Header (height: 3)
    // 2. Dedicated Agent Terminal (fills available height, min: 8)
    // 3. Compact Session Info (height: 4)
    // 4. Pending Interactions (height: 3 or expandable)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),                   // 1. Session Header
            Constraint::Min(8),                      // 2. Agent Terminal
            Constraint::Length(4),                   // 3. Session Info
            Constraint::Length(interactions_height), // 4. Pending Interactions
        ])
        .split(area);

    // ── 1. Compact Session Header ───────────────────────────────────────────
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(30), Constraint::Length(26)])
        .split(chunks[0]);

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let task_display = if session.task_description.len() > 45 {
        format!("{}...", &session.task_description[..42])
    } else if !session.task_description.is_empty() {
        session.task_description.clone()
    } else {
        session.id.0.clone()
    };

    let title_line = Line::from(vec![
        Span::styled(" Session: ", Style::default().fg(Color::DarkGray)),
        Span::styled(&session.agent_type, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(" (", Style::default().fg(Color::DarkGray)),
        Span::styled(&session.id.0, Style::default().fg(Color::White)),
        Span::styled(")  ", Style::default().fg(Color::DarkGray)),
        Span::styled(task_display, Style::default().fg(Color::White)),
    ]);

    let badge_line = Line::from(vec![
        session_state_badge(&session.state),
        Span::raw(" "),
    ]);

    f.render_widget(Paragraph::new(title_line).block(header_block), chunks[0]);
    f.render_widget(
        Paragraph::new(badge_line).alignment(Alignment::Right),
        Rect {
            x: header_chunks[1].x,
            y: chunks[0].y + 1,
            width: header_chunks[1].width,
            height: 1,
        },
    );

    // ── 2. Dedicated Agent Terminal ─────────────────────────────────────────
    let default_buf = TerminalBuffer::default();
    let term_buf = app
        .session_terminal_buffers
        .get(&session.id.0)
        .unwrap_or(&default_buf);

    let term_inner_height = chunks[1].height.saturating_sub(2) as usize;
    let (visible_lines, scroll_info) = term_buf.get_visible_lines(term_inner_height);

    let (border_color, title_spans) = if scroll_info.follow {
        (
            Color::Cyan,
            vec![
                Span::styled(" Agent Terminal ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled("● LIVE ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(format!("({} lines) ", scroll_info.total_lines), Style::default().fg(Color::DarkGray)),
            ],
        )
    } else {
        (
            Color::Yellow,
            vec![
                Span::styled(" Agent Terminal ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("[SCROLL MODE: {} lines up | Press End to Follow] ", scroll_info.scroll_offset),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
            ],
        )
    };

    let mut term_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Line::from(title_spans));

    if !scroll_info.follow {
        term_block = term_block.title_bottom(Line::from(vec![
            Span::styled(
                " [↑/↓ Scroll | PgUp/PgDn | End to Follow] ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    if visible_lines.is_empty() {
        let empty_msg = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Waiting for agent output... ", Style::default().fg(Color::DarkGray)),
                Span::styled("(Press 's' to steer, 'p' to pause, 'a' to switch account)", Style::default().fg(Color::DarkGray)),
            ]),
        ];
        f.render_widget(Paragraph::new(empty_msg).block(term_block), chunks[1]);
    } else {
        f.render_widget(Paragraph::new(visible_lines).block(term_block), chunks[1]);
    }

    // ── 3. Compact Session Info ─────────────────────────────────────────────
    let acct_label = app.account_label(session.account_id.as_ref());
    let acct_str = match &session.account_id {
        Some(aid) => format!("{} ({})", acct_label, aid.0),
        None => "None".to_string(),
    };
    let proj_str = session.project_id.as_ref().map(|p| p.0.as_str()).unwrap_or("None");
    let ws_str = session.workspace_id.as_ref().map(|w| w.0.as_str()).unwrap_or("None");
    let created_str = session.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string();

    let info_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(" Session Info ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));

    let info_lines = vec![
        Line::from(vec![
            Span::styled("Account: ", Style::default().fg(Color::DarkGray)),
            Span::styled(acct_str, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("    "),
            Span::styled("Project: ", Style::default().fg(Color::DarkGray)),
            Span::styled(proj_str, Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Restarts: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", session.restart_count), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Workspace: ", Style::default().fg(Color::DarkGray)),
            Span::styled(ws_str, Style::default().fg(Color::Cyan)),
            Span::raw("    "),
            Span::styled("Created: ", Style::default().fg(Color::DarkGray)),
            Span::styled(created_str, Style::default().fg(Color::Gray)),
        ]),
    ];

    f.render_widget(Paragraph::new(info_lines).block(info_block), chunks[2]);

    // ── 4. Pending Interactions ─────────────────────────────────────────────
    let pending_title = if session_pending.is_empty() {
        Span::styled(" Pending Interactions: 0 ", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(
            format!(" Pending Interactions: {} ", session_pending.len()),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )
    };

    let pending_border_color = if session_pending.is_empty() {
        Color::DarkGray
    } else {
        Color::Yellow
    };

    let pending_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pending_border_color))
        .title(pending_title);

    if session_pending.is_empty() {
        let p = Paragraph::new(Line::from(vec![
            Span::styled("No pending human interactions required.", Style::default().fg(Color::DarkGray)),
        ])).block(pending_block);
        f.render_widget(p, chunks[3]);
    } else {
        let mut lines = Vec::new();
        for item in session_pending.iter().take(2) {
            let detail = item.tool_name.as_deref().unwrap_or(&item.prompt);
            lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", item.kind), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(detail, Style::default().fg(Color::White)),
            ]));
        }
        lines.push(Line::from(Span::styled(
            "Press 3 to switch to Inbox to review and resolve",
            Style::default().fg(Color::Cyan),
        )));
        let p = Paragraph::new(lines).block(pending_block);
        f.render_widget(p, chunks[3]);
    }
}
