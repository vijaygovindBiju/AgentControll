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
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header (title + tabs)
            Constraint::Min(10),   // Active View content
            Constraint::Length(2), // Footer (status + shortcuts)
        ])
        .split(f.area());

    // 1. Header
    render_header(f, app, main_layout[0]);

    // 2. Main View
    if app.session_detail_id.is_some() {
        views::session_detail::render(f, app, main_layout[1]);
    } else {
        match app.current_tab {
            Tab::Dashboard => views::dashboard::render(f, app, main_layout[1]),
            Tab::Sessions => views::sessions::render(f, app, main_layout[1]),
            Tab::Inbox => views::inbox::render(f, app, main_layout[1]),
            Tab::Accounts => views::accounts::render(f, app, main_layout[1]),
            Tab::Projects => views::projects::render(f, app, main_layout[1]),
            Tab::Activity => views::activity::render(f, app, main_layout[1]),
        }
    }

    // 3. Footer
    render_footer(f, app, main_layout[2]);

    // 4. Modal Dialog (if active, rendered on top)
    views::modal::render(f, app);
}
