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
    let area = f.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    // ── Full-Screen Terminal Mode ───────────────────────────────────────────
    // When a session is selected and open, the terminal takes over the entire screen.
    // Navigation menu, sidebar, and dashboard panels are completely removed.
    if app.session_detail_id.is_some() {
        views::session_detail::render(f, app, area);
        views::modal::render(f, app);
        return;
    }

    // ── Normal Dashboard / Navigation Mode ──────────────────────────────────
    // Scale header/footer constraints if terminal is very small
    let (header_len, footer_len) = if area.height < 4 {
        (1, 0)
    } else if area.height < 6 {
        (1, 1)
    } else {
        (2, 2)
    };

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_len),
            Constraint::Min(1),
            Constraint::Length(footer_len),
        ])
        .split(area);

    // 1. Header
    if header_len > 0 {
        render_header(f, app, main_layout[0]);
    }

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
    if footer_len > 0 {
        render_footer(f, app, main_layout[2]);
    }

    // 4. Modal Dialog (if active, rendered on top)
    views::modal::render(f, app);
}
