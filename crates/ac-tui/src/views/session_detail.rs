//! Session detail view rendering.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::App,
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // Metadata header
            Constraint::Min(10),   // Transcript and interactions split
            Constraint::Length(3), // Controls bar
        ])
        .split(area);

    // ── 1. Metadata Header ──────────────────────────────────────────────────
    let proj_str = session.project_id.as_ref().map(|p| p.0.as_str()).unwrap_or("None");
    let acct_label = app.account_label(session.account_id.as_ref());
    let acct_str = match &session.account_id {
        Some(aid) => format!("{} ({})", acct_label, aid.0),
        None => "None".to_string(),
    };
    let ws_str = session.workspace_id.as_ref().map(|w| w.0.as_str()).unwrap_or("None");
    let created_str = session.created_at.format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let updated_str = session.updated_at.format("%Y-%m-%d %H:%M:%S UTC").to_string();

    let mut meta_lines = vec![
        Line::from(vec![
            Span::styled("Session ID: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&session.id.0, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("    "),
            Span::styled("State: ", Style::default().fg(Color::DarkGray)),
            session_state_badge(&session.state),
            Span::raw("    "),
            Span::styled("Agent Type: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&session.agent_type, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw("    "),
            Span::styled("Restarts: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", session.restart_count), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Task: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&session.task_description, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Project ID: ", Style::default().fg(Color::DarkGray)),
            Span::styled(proj_str, Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("Account: ", Style::default().fg(Color::DarkGray)),
            Span::styled(acct_str, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("    "),
            Span::styled("Workspace ID: ", Style::default().fg(Color::DarkGray)),
            Span::styled(ws_str, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Created: ", Style::default().fg(Color::DarkGray)),
            Span::styled(created_str, Style::default().fg(Color::Gray)),
            Span::raw("    "),
            Span::styled("Last Updated: ", Style::default().fg(Color::DarkGray)),
            Span::styled(updated_str, Style::default().fg(Color::Gray)),
        ]),
    ];

    if session.predecessor_id.is_some() || session.successor_id.is_some() {
        let pred = session.predecessor_id.as_ref().map(|p| p.0.as_str()).unwrap_or("None");
        let succ = session.successor_id.as_ref().map(|s| s.0.as_str()).unwrap_or("None");
        meta_lines.push(Line::from(vec![
            Span::styled("Predecessor: ", Style::default().fg(Color::DarkGray)),
            Span::styled(pred, Style::default().fg(Color::Cyan)),
            Span::raw("    "),
            Span::styled("Successor: ", Style::default().fg(Color::DarkGray)),
            Span::styled(succ, Style::default().fg(Color::Green)),
        ]));
    }

    let meta_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            format!(" Session Detail: {} ", session.id.0),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(Paragraph::new(meta_lines).block(meta_block), chunks[0]);

    // ── 2. Middle Split: Transcript on Left, Interactions & Events on Right ─
    let mid_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(chunks[1]);

    // Live Transcript Buffer
    let transcript = app
        .session_transcripts
        .get(&session.id.0)
        .cloned()
        .unwrap_or_default();

    let transcript_lines: Vec<Line> = if transcript.is_empty() {
        vec![Line::from(Span::styled(
            "Waiting for agent output and events...",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        // Show recent transcript lines
        transcript
            .iter()
            .rev()
            .take(50)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|l| {
                let color = if l.contains("[OUT]") {
                    Color::White
                } else if l.contains("[STATE]") {
                    Color::Cyan
                } else if l.contains("[APPROVAL_REQ]") || l.contains("[QUESTION]") {
                    Color::Yellow
                } else if l.contains("[STEER]") {
                    Color::Green
                } else if l.contains("[CRASHED]") || l.contains("[FAILED]") {
                    Color::Red
                } else {
                    Color::Gray
                };
                Line::from(Span::styled(l.clone(), Style::default().fg(color)))
            })
            .collect()
    };

    let transcript_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Live Transcript & Agent Output ",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(
        Paragraph::new(transcript_lines)
            .block(transcript_block)
            .wrap(Wrap { trim: false }),
        mid_chunks[0],
    );

    // Right Side: Pending Interactions & Session Events
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(mid_chunks[1]);

    // Session-specific pending interactions
    let session_pending: Vec<_> = app
        .pending_interactions()
        .into_iter()
        .filter(|i| i.session_id == session.id)
        .collect();

    let pending_block = Block::default()
        .borders(Borders::ALL)
        .border_style(if session_pending.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Yellow)
        })
        .title(Span::styled(
            format!(" Pending Interactions ({}) ", session_pending.len()),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));

    if session_pending.is_empty() {
        let p = Paragraph::new("No pending interactions for this session.")
            .style(Style::default().fg(Color::DarkGray))
            .block(pending_block);
        f.render_widget(p, right_chunks[0]);
    } else {
        let mut lines = Vec::new();
        for item in session_pending.iter().take(4) {
            let detail = item.tool_name.as_deref().unwrap_or(&item.prompt);
            lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", item.kind), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::styled(detail, Style::default().fg(Color::White)),
            ]));
        }
        lines.push(Line::from(Span::styled(
            "Press 3 to switch to Inbox to resolve",
            Style::default().fg(Color::Cyan),
        )));
        let p = Paragraph::new(lines).block(pending_block);
        f.render_widget(p, right_chunks[0]);
    }

    // Session-specific recent events
    let session_events: Vec<_> = app
        .events
        .iter()
        .filter(|e| e.session_id.as_ref() == Some(&session.id))
        .take(8)
        .collect();

    let events_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(" Recent Session Events ", Style::default().fg(Color::Gray)));

    if session_events.is_empty() {
        let p = Paragraph::new("No events recorded for this session yet.")
            .style(Style::default().fg(Color::DarkGray))
            .block(events_block);
        f.render_widget(p, right_chunks[1]);
    } else {
        let lines: Vec<Line> = session_events
            .iter()
            .map(|e| {
                let time_str = e.timestamp.format("%H:%M:%S").to_string();
                Line::from(vec![
                    Span::styled(format!("[{time_str}] "), Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}", e.kind), Style::default().fg(Color::White)),
                ])
            })
            .collect();
        let p = Paragraph::new(lines).block(events_block);
        f.render_widget(p, right_chunks[1]);
    }

    // ── 3. Controls Bar ─────────────────────────────────────────────────────
    let ctrl_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(" Session Controls ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));

    let ctrl_line = Line::from(vec![
        Span::styled("  [w] Switch Account  ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("  [s] Steer (Inject message)  ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("  [p] Pause  ", Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("  [Space] Resume  ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("  [x] Stop Session  ", Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
        Span::styled("  [Esc] Back to Sessions  ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(ctrl_line).block(ctrl_block), chunks[2]);
}
