//! Full-screen terminal-first session view for Agent Control.
//!
//! When a session is selected, this view takes over the entire screen.
//! There are no sidebars, no multi-panel widgets, and no artificial metadata boxes.
//! The terminal is the primary interface.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{app::App, terminal_buffer::TerminalBuffer, views::session_state_badge};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let session = match app.selected_session_or_detail() {
        Some(s) => s,
        None => {
            let p = Paragraph::new(
                "Session not found or has been removed. Press Ctrl+Q or Esc to return.",
            )
            .style(Style::default().fg(Color::Red));
            f.render_widget(p, area);
            return;
        }
    };

    // ── 3-Row Vertical Layout: Sleek 1-line Header, Full Terminal, Minimal 1-line Footer ──
    let (header_h, footer_h) = if area.height < 3 {
        (0, 0)
    } else if area.height < 5 {
        (1, 0)
    } else {
        (1, 1)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h),
            Constraint::Min(1),
            Constraint::Length(footer_h),
        ])
        .split(area);

    // ── 2. Full-Screen Dedicated Agent Terminal ─────────────────────────────
    let default_buf = TerminalBuffer::default();
    let term_buf = app
        .session_terminal_buffers
        .get(&session.id.0)
        .unwrap_or(&default_buf);

    let acct_label = app.account_label(session.account_id.as_ref());
    let is_agy = session.agent_type == "agy" || session.agent_type == "antigravity";
    let agent_name = if let Some(title) = term_buf.title() {
        if title.starts_with("Agent Control") {
            title.to_string()
        } else {
            format!("Agent Control • {}", title)
        }
    } else if is_agy {
        "Agent Control • Antigravity".to_string()
    } else {
        "Agent Control • Agent Terminal".to_string()
    };

    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(40), Constraint::Length(28)])
        .split(chunks[0]);

    let term_inner_width = chunks[1].width as usize;
    let term_inner_height = chunks[1].height as usize;
    let (visible_lines, scroll_info, cursor_rel) =
        term_buf.get_visible_lines_wrapped(term_inner_height, term_inner_width);

    let mut left_spans = if scroll_info.follow {
        vec![
            Span::raw(" "),
            Span::styled(
                "● LIVE ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            session_state_badge(&session.state),
            Span::raw("  "),
            Span::styled(
                agent_name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    } else {
        vec![
            Span::raw(" "),
            Span::styled(
                format!("[SCROLL: {} lines up] ", scroll_info.scroll_offset),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            session_state_badge(&session.state),
            Span::raw("  "),
            Span::styled(
                agent_name.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    };
    if chunks[0].width >= 95 && !acct_label.is_empty() && acct_label != "None" {
        left_spans.push(Span::styled(" • ", Style::default().fg(Color::DarkGray)));
        left_spans.push(Span::styled(acct_label, Style::default().fg(Color::Yellow)));
    }
    if let Some(cwd) = term_buf.cwd() {
        if chunks[0].width >= 80 {
            let cwd_display = cwd.display().to_string();
            let display_str = if cwd_display.len() > 30 {
                format!("…{}", &cwd_display[cwd_display.len().saturating_sub(29)..])
            } else {
                cwd_display
            };
            left_spans.push(Span::styled(" • ", Style::default().fg(Color::DarkGray)));
            left_spans.push(Span::styled(display_str, Style::default().fg(Color::LightBlue)));
        }
    }
    f.render_widget(Paragraph::new(Line::from(left_spans)), header_chunks[0]);

    let model_tag = if is_agy {
        "Gemini 3.8 Flash · medium"
    } else {
        &session.agent_type
    };
    let right_spans = vec![
        Span::styled(model_tag, Style::default().fg(Color::LightCyan)),
        Span::raw(" "),
    ];
    f.render_widget(
        Paragraph::new(Line::from(right_spans)).alignment(Alignment::Right),
        header_chunks[1],
    );

    let all_blank = visible_lines.iter().all(|l| {
        l.spans.is_empty() || (l.spans.len() == 1 && l.spans[0].content.trim().is_empty())
    });

    if all_blank {
        let empty_msg = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "  Waiting for agent output... ",
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    "(Type prompts directly into the terminal)",
                    Style::default().fg(Color::Cyan),
                ),
            ]),
        ];
        f.render_widget(Paragraph::new(empty_msg), chunks[1]);
    } else {
        f.render_widget(Paragraph::new(visible_lines), chunks[1]);
    }

    // Set hardware terminal cursor where the agent expects it
    if scroll_info.follow {
        if let Some((rel_col, rel_row)) = cursor_rel {
            let cursor_x = chunks[1].x + rel_col;
            let cursor_y = chunks[1].y + rel_row;
            if cursor_x < chunks[1].right() && cursor_y < chunks[1].bottom() {
                f.set_cursor_position(Position::new(cursor_x, cursor_y));

                let cursor_style = match term_buf.cursor_shape() {
                    crate::terminal_buffer::CursorShape::Default => {
                        match app.user_settings.terminal_cursor.to_lowercase().as_str() {
                            "bar" => crossterm::cursor::SetCursorStyle::SteadyBar,
                            "underline" => crossterm::cursor::SetCursorStyle::SteadyUnderScore,
                            _ => crossterm::cursor::SetCursorStyle::SteadyBlock,
                        }
                    }
                    shape => shape.to_crossterm(),
                };
                let _ = crossterm::execute!(std::io::stdout(), cursor_style);
            }
        }
    }

    // ── 3. Minimal Footer ───────────────────────────────────────────────────
    let footer_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(40), Constraint::Length(20)])
        .split(chunks[2]);

    let shortcuts_line = if term_buf.search.active {
        let match_info = if term_buf.search.matches.is_empty() {
            if term_buf.search.query.is_empty() {
                "Type to search...".to_string()
            } else {
                "No matches".to_string()
            }
        } else {
            format!("{}/{} matches", term_buf.search.current_idx + 1, term_buf.search.matches.len())
        };
        let status_badge = if term_buf.search.editing {
            Span::styled(" [SEARCH EDIT] ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(" [SEARCH NAV] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        };
        Line::from(vec![
            status_badge,
            Span::styled("Query: ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("\"{}\"", term_buf.search.query), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("[{}] ", match_info), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("[n/N]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(" Next/Prev  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Enter]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(" Lock  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Esc]", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(" Exit", Style::default().fg(Color::DarkGray)),
        ])
    } else if scroll_info.follow {
        Line::from(vec![
            Span::styled(
                " [Ctrl+Q]",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Back  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Ctrl+P]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Control  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Ctrl+F]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Search  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Ctrl+O]",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Links  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[F1]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Help  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[PgUp/PgDn]", Style::default().fg(Color::White)),
            Span::styled(" Scroll", Style::default().fg(Color::DarkGray)),
        ])
    } else if term_buf.is_selecting() {
        Line::from(vec![
            Span::styled(
                " [VISUAL MODE: ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "move with hjkl/arrows | [y] Yank | [v/Esc] Cancel] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " [SCROLL MODE: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} lines up", scroll_info.scroll_offset),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " | [/] Search | [v] Select | [o] Links | Press End/Esc to return] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    };

    let conn_span = if app.daemon_connected {
        Span::styled("● Connected ", Style::default().fg(Color::Green))
    } else {
        Span::styled("○ Disconnected ", Style::default().fg(Color::Red))
    };

    f.render_widget(Paragraph::new(shortcuts_line), footer_chunks[0]);
    f.render_widget(
        Paragraph::new(Line::from(conn_span)).alignment(Alignment::Right),
        footer_chunks[1],
    );
}
