//! Top-level UI drawing coordinator for Ratatui.

use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use crate::{
    app::{App, Tab},
    views::{self, render_footer, render_header, render_sidebar},
};

pub fn draw(f: &mut Frame, app: &App) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Header (title + info)
            Constraint::Min(10),   // Middle Area: Sidebar + Content
            Constraint::Length(2), // Footer (shortcuts + status)
        ])
        .split(f.area());

    // 1. Header
    render_header(f, app, main_layout[0]);

    // 2. Middle Area (Sidebar + Active View)
    let middle_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(18), // Sidebar
            Constraint::Min(50),   // Active View
        ])
        .split(main_layout[1]);

    render_sidebar(f, app, middle_layout[0]);

    let content_area = middle_layout[1];
    if app.session_detail_id.is_some() {
        views::session_detail::render(f, app, content_area);
    } else {
        match app.current_tab {
            Tab::Dashboard => views::dashboard::render(f, app, content_area),
            Tab::Sessions => views::sessions::render(f, app, content_area),
            Tab::Inbox => views::inbox::render(f, app, content_area),
            Tab::Accounts => views::accounts::render(f, app, content_area),
            Tab::Projects => views::projects::render(f, app, content_area),
            Tab::Activity => views::activity::render(f, app, content_area),
            Tab::Agents => views::agents::render(f, app, content_area),
            Tab::Settings => views::settings::render(f, app, content_area),
        }
    }

    // 3. Footer
    render_footer(f, app, main_layout[2]);

    // 4. Modal Dialog (if active, rendered on top)
    views::modal::render(f, app);
}
