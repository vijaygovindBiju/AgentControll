//! Project registry view rendering.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;

pub fn render(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(6)])
        .split(area);

    render_projects_table(f, app, chunks[0]);
    render_project_detail(f, app, chunks[1]);
}

fn render_projects_table(f: &mut Frame, app: &App, area: Rect) {
    let header_cells = [
        "PROJECT ID", "NAME", "REPO PATH", "WORKSPACE POLICY", "DEFAULT TAGS", "ACTIVE SESSIONS",
    ]
    .iter()
    .map(|h| Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(1);

    let rows = app.projects.iter().enumerate().map(|(idx, p)| {
        let is_selected = idx == app.selected_project;
        let style = if is_selected {
            Style::default().bg(Color::Rgb(40, 45, 60))
        } else {
            Style::default()
        };

        let pid = if p.id.0.len() > 10 {
            format!("{}...", &p.id.0[..8])
        } else {
            p.id.0.clone()
        };

        let tags_str = if p.default_account_tags.is_empty() {
            "-".to_string()
        } else {
            p.default_account_tags.join(", ")
        };

        let active_count = app
            .sessions
            .iter()
            .filter(|s| s.project_id.as_ref() == Some(&p.id))
            .count();

        let row_cells = vec![
            Cell::from(pid).style(if is_selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(p.name.clone()),
            Cell::from(p.repo_path.clone()),
            Cell::from(format!("{}", p.workspace_policy)),
            Cell::from(tags_str),
            Cell::from(format!("{active_count}")),
        ];
        Row::new(row_cells).style(style).height(1)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(16),
            Constraint::Length(28),
            Constraint::Length(20),
            Constraint::Length(18),
            Constraint::Min(16),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                format!(" Projects ({}) ", app.projects.len()),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(table, area);
}

fn render_project_detail(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(" Selected Project Details ", Style::default().fg(Color::Gray)));

    if let Some(p) = app.projects.get(app.selected_project) {
        let active_sessions: Vec<_> = app
            .sessions
            .iter()
            .filter(|s| s.project_id.as_ref() == Some(&p.id))
            .map(|s| s.id.0.as_str())
            .collect();
        let sessions_str = if active_sessions.is_empty() {
            "None".to_string()
        } else {
            active_sessions.join(", ")
        };

        let lines = vec![
            Line::from(vec![
                Span::styled("Project ID: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&p.id.0, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("    "),
                Span::styled("Name: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&p.name, Style::default().fg(Color::White)),
                Span::raw("    "),
                Span::styled("Workspace Policy: ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", p.workspace_policy), Style::default().fg(Color::Yellow)),
            ]),
            Line::from(vec![
                Span::styled("Repository Path: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&p.repo_path, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Bound Sessions: ", Style::default().fg(Color::DarkGray)),
                Span::styled(sessions_str, Style::default().fg(Color::Cyan)),
            ]),
        ];
        f.render_widget(Paragraph::new(lines).block(block), area);
    } else {
        let p = Paragraph::new("No projects registered.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(p, area);
    }
}
