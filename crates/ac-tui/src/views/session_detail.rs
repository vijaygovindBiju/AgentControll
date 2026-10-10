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
    let (header_h, footer_h) = if app.fullscreen_terminal {
        (0, 0)
    } else if area.height < 3 {
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

    if header_h > 0 {
        let header_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(28)])
            .split(chunks[0]);

        let mut left_spans = if term_buf.follow {
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
                    format!("[SCROLL: {} lines up] ", term_buf.scroll_offset),
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
    }

    if let Some(ref split_id) = app.split_session_id {
        let split_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Length(1),
                Constraint::Percentage(50),
            ])
            .split(chunks[1]);

        // Left pane (primary session)
        render_single_terminal(
            f,
            app,
            term_buf,
            split_chunks[0],
            !app.split_focus_right,
        );

        // Divider column
        let divider_lines: Vec<Line> = (0..split_chunks[1].height)
            .map(|_| Line::from(Span::styled("│", Style::default().fg(Color::DarkGray))))
            .collect();
        f.render_widget(Paragraph::new(divider_lines), split_chunks[1]);

        // Right pane (split session)
        let split_buf = app
            .session_terminal_buffers
            .get(&split_id.0)
            .unwrap_or(&default_buf);
        render_single_terminal(
            f,
            app,
            split_buf,
            split_chunks[2],
            app.split_focus_right,
        );
    } else {
        render_single_terminal(f, app, term_buf, chunks[1], true);
    }

    // ── 3. Minimal Footer ───────────────────────────────────────────────────
    if footer_h > 0 {
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
        } else if term_buf.follow {
            let detach_k = app.user_settings.keybindings.get("detach_session");
            let cmd_k = app.user_settings.keybindings.get("command_palette");
            let split_k = app.user_settings.keybindings.get("toggle_split");
            let switch_k = app.user_settings.keybindings.get("switch_split_focus");
            let fs_k = app.user_settings.keybindings.get("toggle_fullscreen");
            let url_k = app.user_settings.keybindings.get("url_picker");

            let mut shortcuts = vec![
                Span::styled(
                    format!(" [{detach_k}]"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Back  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("[{cmd_k}]"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Control  ", Style::default().fg(Color::DarkGray)),
            ];

            if app.split_session_id.is_some() {
                shortcuts.push(Span::styled(
                    format!("[{switch_k}]"),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ));
                shortcuts.push(Span::styled(" Switch  ", Style::default().fg(Color::DarkGray)));
                shortcuts.push(Span::styled(
                    format!("[{split_k}]"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
                shortcuts.push(Span::styled(" Close Split  ", Style::default().fg(Color::DarkGray)));
            } else {
                shortcuts.push(Span::styled(
                    format!("[{split_k}]"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ));
                shortcuts.push(Span::styled(" Split  ", Style::default().fg(Color::DarkGray)));
            }

            shortcuts.extend(vec![
                Span::styled(
                    format!("[{fs_k}]"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" Fullscreen  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("[{url_k}]"),
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
            ]);
            Line::from(shortcuts)
        } else if term_buf.is_selecting() {
            Line::from(vec![
                Span::styled(
                    " [VISUAL MODE: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "drag with mouse or move with arrows | [y/Ctrl+Shift+C] Copy | [Esc] Cancel] ",
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
                    format!("{} lines up", term_buf.scroll_offset),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " | [k/j] Line | [u/d] Page | [v] Select | [/] Search | [o] Links | [Esc/q] Live] ",
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
}

fn render_single_terminal(
    f: &mut Frame,
    app: &App,
    term_buf: &TerminalBuffer,
    area: Rect,
    is_focused: bool,
) {
    let term_inner_width = area.width as usize;
    let term_inner_height = area.height as usize;
    let (visible_lines, scroll_info, cursor_rel) =
        term_buf.get_visible_lines_wrapped(term_inner_height, term_inner_width);

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
        f.render_widget(Paragraph::new(empty_msg), area);
    } else {
        f.render_widget(Paragraph::new(visible_lines), area);
    }

    // Set hardware terminal cursor where the agent expects it (only if this pane is focused)
    if is_focused && scroll_info.follow {
        if let Some((rel_col, rel_row)) = cursor_rel {
            let cursor_x = area.x + rel_col;
            let cursor_y = area.y + rel_row;
            if cursor_x < area.right() && cursor_y < area.bottom() {
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
}
