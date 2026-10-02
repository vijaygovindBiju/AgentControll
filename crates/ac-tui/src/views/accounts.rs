use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::{
    app::App,
    views::{account_state_badge, truncate_chars, truncate_display_width},
};

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(6)])
        .split(area);

    render_accounts_table(f, app, chunks[0]);
    render_account_detail(f, app, chunks[1]);
}

fn render_accounts_table(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = [
        "ACCOUNT ID",
        "LABEL",
        "PROVIDER",
        "STATE",
        "LOAD / CAP",
        "AGENT TYPES",
        "TAGS",
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

    let rows = app.accounts.iter().enumerate().map(|(idx, a)| {
        let is_selected = idx == app.selected_account;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(40, 45, 60))
        } else {
            Style::default()
        };

        let aid = truncate_chars(&a.id.0, 10);
        let label = truncate_display_width(&a.label, 16);

        let load_str = format!("{}/{}", a.active_session_count, a.concurrency_cap);
        let agents_str = truncate_display_width(&a.agent_types.join(", "), 20);
        let tags_str = if a.tags.is_empty() {
            "-".to_string()
        } else {
            truncate_display_width(&a.tags.join(", "), 16)
        };

        let row_cells = vec![
            Cell::from(aid).style(if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(label),
            Cell::from(a.provider.clone()),
            Cell::from(Line::from(vec![account_state_badge(&a.state)])),
            Cell::from(load_str),
            Cell::from(agents_str),
            Cell::from(tags_str),
        ];
        Row::new(row_cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(18),
            Constraint::Length(12),
            Constraint::Length(20),
            Constraint::Min(16),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                format!(" Accounts ({}) ", app.accounts.len()),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
    );

    let mut state = TableState::default();
    if !app.accounts.is_empty() {
        state.select(Some(app.selected_account.min(app.accounts.len() - 1)));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn render_account_detail(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(
            " Selected Account Details ",
            Style::default().fg(Color::Gray),
        ));

    if let Some(a) = app.accounts.get(app.selected_account) {
        let cooldown_str = a
            .cooldown_until
            .map(|t| format!("Until {}", t.format("%H:%M:%S UTC")))
            .unwrap_or_else(|| "None".into());

        let lines = vec![
            Line::from(vec![
                Span::styled("Account ID: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    &a.id.0,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("    "),
                Span::styled("Label: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&a.label, Style::default().fg(Color::White)),
                Span::raw("    "),
                Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&a.provider, Style::default().fg(Color::White)),
                Span::raw("    "),
                Span::styled("State: ", Style::default().fg(Color::DarkGray)),
                account_state_badge(&a.state),
            ]),
            Line::from(vec![
                Span::styled("Credential Ref: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&a.credential_ref, Style::default().fg(Color::Gray)),
                Span::raw("    "),
                Span::styled("Cooldown: ", Style::default().fg(Color::DarkGray)),
                Span::styled(cooldown_str, Style::default().fg(Color::Yellow)),
                Span::raw("    "),
                Span::styled("Concurrency: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}/{}", a.active_session_count, a.concurrency_cap),
                    Style::default().fg(Color::White),
                ),
            ]),
        ];
        f.render_widget(Paragraph::new(lines).block(block), area);
    } else {
        let p = Paragraph::new("No accounts registered.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(p, area);
    }
}
