//! "Start Antigravity Session" form and its lightweight directory completion popup.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::App,
    launch::{
        self, split_input, StartSessionForm, FIELD_ACCOUNT, FIELD_BUTTONS, FIELD_DIR,
        FIELD_EXEC_MODE, FIELD_MODEL, FIELD_PERM_MODE,
    },
};

fn centered(w: u16, h: u16, r: Rect) -> Rect {
    let w = w.min(r.width);
    let h = h.min(r.height);
    Rect::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, w, h)
}

pub fn render(f: &mut Frame, app: &App, form: &StartSessionForm) {
    let area = centered(74, 28, f.area());
    f.render_widget(Clear, area);
    let dim = Style::default().fg(Color::DarkGray);
    let label = |i: usize, t: &'static str| {
        Span::styled(
            t,
            if form.field == i {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            },
        )
    };
    let value = |i: usize, t: String| {
        let focused = form.field == i;
        Line::from(vec![
            Span::styled(
                if focused { " > " } else { "   " },
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                t,
                if focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
            Span::styled(
                if focused && i != FIELD_DIR {
                    "   ←/→"
                } else {
                    ""
                },
                dim,
            ),
        ])
    };

    let accounts = launch::agy_accounts(app);
    let account = match accounts.get(form.account_index) {
        Some(a) => a.label.clone(),
        None => "+ Add Antigravity Account".into(),
    };
    let exec_mode = form.exec_mode();
    let perm_mode = form.perm_mode();
    let (models, model_status) = launch::model_options(app, form);
    let model = models
        .get(form.model_index)
        .map(|(_, n)| n.clone())
        .unwrap_or_default();

    let mut lines = vec![
        Line::from(""),
        // Agent (Fixed display)
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "Agent",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("   "),
            Span::styled("Antigravity (agy CLI)", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(""),
        // Account
        Line::from(vec![Span::raw(" "), label(FIELD_ACCOUNT, "Account")]),
        value(FIELD_ACCOUNT, account),
        Line::from(""),
        // Working Directory
        Line::from(vec![Span::raw(" "), label(FIELD_DIR, "Working Directory")]),
    ];

    let dir_row = lines.len();
    let mut dir_line = value(FIELD_DIR, form.dir_input.clone());
    if form.field == FIELD_DIR {
        dir_line
            .spans
            .push(Span::styled("█", Style::default().fg(Color::Cyan)));
    }
    lines.push(dir_line);
    let resolved = form.resolved_dir();
    let (mark, mark_style) = if resolved.is_dir() {
        ("", dim)
    } else {
        ("  ✗ not a directory", Style::default().fg(Color::Red))
    };
    lines.push(Line::from(vec![
        Span::styled(format!("   Resolved: {}", resolved.display()), dim),
        Span::styled(mark, mark_style),
    ]));
    let dir_hint = if form.listing.is_some() {
        Span::styled("   reading directory…", dim)
    } else if let Some(n) = &form.dir_notice {
        Span::styled(format!("   {n}"), Style::default().fg(Color::Yellow))
    } else {
        Span::styled(
            "   [ TAB completion ]",
            if form.field == FIELD_DIR {
                Style::default().fg(Color::Cyan)
            } else {
                dim
            },
        )
    };
    lines.push(Line::from(dir_hint));
    lines.push(Line::from(""));

    // Execution Mode
    lines.push(Line::from(vec![
        Span::raw(" "),
        label(FIELD_EXEC_MODE, "Execution Mode"),
    ]));
    lines.push(value(FIELD_EXEC_MODE, exec_mode.label().to_string()));
    lines.push(Line::from(Span::styled(
        format!("   {}", exec_mode.description()),
        dim,
    )));
    lines.push(Line::from(""));

    // Permission / Access
    lines.push(Line::from(vec![
        Span::raw(" "),
        label(FIELD_PERM_MODE, "Permission / Access"),
    ]));
    lines.push(value(FIELD_PERM_MODE, perm_mode.label().to_string()));
    lines.push(Line::from(Span::styled(
        format!("   {}", perm_mode.description()),
        if perm_mode.is_dangerous() {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            dim
        },
    )));
    lines.push(Line::from(""));

    // Model
    lines.push(Line::from(vec![
        Span::raw(" "),
        label(FIELD_MODEL, "Model"),
    ]));
    lines.push(value(FIELD_MODEL, model));
    if let Some(s) = model_status {
        lines.push(Line::from(Span::styled(format!("   {s}"), dim)));
    }
    lines.push(Line::from(""));

    if let Some(e) = &form.error {
        lines.push(Line::from(Span::styled(
            format!(" {e}"),
            Style::default().fg(Color::Red),
        )));
    }

    let button = |i: usize, t: &'static str| {
        let on = form.field == FIELD_BUTTONS && form.button == i;
        Span::styled(
            t,
            if on {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        )
    };
    lines.push(Line::from(vec![
        Span::raw("                    "),
        button(0, "[ Start Session ]"),
        Span::raw("  "),
        button(1, "[ Cancel ]"),
    ]));
    lines.push(Line::from(Span::styled(
        " ↑↓ field  ←/→ cycle option  Tab complete dir  Enter select/start  Esc cancel",
        dim,
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Start Antigravity Session ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );

    // ── Completion popup (terminal-style, under the directory input) ────────
    if let (Some(c), true) = (&form.completion, form.field == FIELD_DIR) {
        let matches = c.matches(split_input(&form.dir_input).1);
        let visible = 8usize;
        let start = c
            .selected
            .saturating_sub(visible - 1)
            .min(matches.len().saturating_sub(visible));
        let shown: Vec<&str> = matches.iter().skip(start).take(visible).copied().collect();
        let width = shown
            .iter()
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(0)
            .max(28) as u16
            + 6;
        let height = shown.len().max(1) as u16 + 4;
        let x = area.x + 4;
        let y = area.y + 1 + dir_row as u16 + 1;
        let frame = f.area();
        let popup = Rect::new(
            x,
            y.min(frame.bottom().saturating_sub(height)),
            width.min(frame.right().saturating_sub(x)),
            height.min(frame.height),
        );
        let mut pl: Vec<Line> = if shown.is_empty() {
            vec![Line::from(Span::styled(" (no matches)", dim))]
        } else {
            shown
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let sel = start + i == c.selected;
                    Line::from(vec![
                        Span::styled(
                            format!(" {name:<w$}", w = width as usize - 6),
                            if sel {
                                Style::default().fg(Color::Black).bg(Color::Cyan)
                            } else {
                                Style::default().fg(Color::White)
                            },
                        ),
                        Span::styled(
                            if sel { " ←" } else { "  " },
                            Style::default().fg(Color::Cyan),
                        ),
                    ])
                })
                .collect()
        };
        pl.push(Line::from(Span::styled(" Tab complete  ↑↓ select", dim)));
        pl.push(Line::from(Span::styled(" Enter open   Esc close", dim)));
        f.render_widget(Clear, popup);
        f.render_widget(
            Paragraph::new(pl).block(Block::default().borders(Borders::ALL).border_style(dim)),
            popup,
        );
    }

    // ── Dangerous-mode confirmation ────────────────────────────────────────
    if let Some(btn) = form.confirm_dangerous {
        let d = centered(64, 11, f.area());
        f.render_widget(Clear, d);
        let b = |i: usize, t: &'static str| {
            Span::styled(
                t,
                if btn == i {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Red)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                },
            )
        };
        let body = vec![
            Line::from(""),
            Line::from(Span::styled(
                " This allows AGY to automatically approve",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                " tool permission requests, including",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                " file changes and command execution.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from(vec![
                Span::raw("            "),
                b(0, "[ Cancel ]"),
                Span::raw("             "),
                b(1, "[ Continue ]"),
            ]),
        ];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .title(Span::styled(
                " Dangerously Skip Permissions ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
        f.render_widget(Paragraph::new(body).block(block), d);
    }
}
