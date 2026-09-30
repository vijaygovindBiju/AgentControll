//! Top-level UI drawing coordinator for Ratatui.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use crate::{
    app::{App, Tab},
    views::{self, render_footer, render_header},
};

pub fn draw(f: &mut Frame, app: &App) {
    // ── Full-Screen Terminal Mode ───────────────────────────────────────────
    // When a session is selected and open, the terminal takes over the entire screen.
    // Navigation menu, sidebar, and dashboard panels are completely removed.
    if app.session_detail_id.is_some() {
        views::session_detail::render(f, app, f.area());
        views::modal::render(f, app);
        return;
    }

    // ── Normal Dashboard / Navigation Mode ──────────────────────────────────
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Header (title + info)
            Constraint::Min(10),   // Active View (full width, no sidebar menu)
            Constraint::Length(2), // Footer (shortcuts + status)
        ])
        .split(f.area());

    // 1. Header
    render_header(f, app, main_layout[0]);

    // 2. Active View
    let content_area = main_layout[1];
    match app.current_tab {
        Tab::Dashboard => views::dashboard::render(f, app, content_area),
        Tab::Sessions => views::sessions::render(f, app, content_area),
        Tab::Accounts => views::accounts::render(f, app, content_area),
        Tab::Activity => views::activity::render(f, app, content_area),
        Tab::Agents => views::agents::render(f, app, content_area),
        Tab::Settings => views::settings::render(f, app, content_area),
    };

    // 3. Footer
    render_footer(f, app, main_layout[2]);

    // 4. Modal Dialog (if active, rendered on top)
    views::modal::render(f, app);
}
