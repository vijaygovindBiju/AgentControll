//! Views module for Ratatui TUI.

pub mod dashboard;
pub mod sessions;
pub mod session_detail;
pub mod inbox;
pub mod accounts;
pub mod projects;
pub mod activity;
pub mod modal;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Tabs},
    Frame,
};

use ac_core::types::{AccountState, SessionState};
use crate::app::{App, StatusType, Tab};

/// Return a styled Span badge for a session state.
pub fn session_state_badge(state: &SessionState) -> Span<'static> {
    match state {
        SessionState::Starting => Span::styled(
            " STARTING ",
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        SessionState::Idle => Span::styled(
            " IDLE ",
            Style::default().fg(Color::Black).bg(Color::DarkGray),
        ),
        SessionState::Working => Span::styled(
            " WORKING ",
            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD),
        ),
        SessionState::WaitingForHuman => Span::styled(
            " WAITING FOR HUMAN ",
            Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        SessionState::Paused => Span::styled(
            " PAUSED ",
            Style::default().fg(Color::White).bg(Color::Blue),
        ),
        SessionState::RateLimited => Span::styled(
            " RATE LIMITED ",
            Style::default().fg(Color::White).bg(Color::Magenta).add_modifier(Modifier::BOLD),
        ),
        SessionState::Stopping => Span::styled(
            " STOPPING ",
            Style::default().fg(Color::Black).bg(Color::DarkGray),
        ),
        SessionState::Stopped => Span::styled(
            " STOPPED ",
            Style::default().fg(Color::Gray).bg(Color::Reset),
        ),
        SessionState::Crashed => Span::styled(
            " CRASHED ",
            Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        SessionState::Restarting => Span::styled(
            " RESTARTING ",
            Style::default().fg(Color::Black).bg(Color::LightYellow),
        ),
        SessionState::Failed => Span::styled(
            " FAILED ",
            Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        SessionState::HandedOff => Span::styled(
            " HANDED OFF ",
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ),
    }
}

/// Return a styled Span badge for an account state.
pub fn account_state_badge(state: &AccountState) -> Span<'static> {
    match state {
        AccountState::Active => Span::styled(
            " ACTIVE ",
            Style::default().fg(Color::Black).bg(Color::Green),
        ),
        AccountState::RateLimited => Span::styled(
            " RATE LIMITED ",
            Style::default().fg(Color::White).bg(Color::Magenta),
        ),
        AccountState::Cooldown => Span::styled(
            " COOLDOWN ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ),
        AccountState::Exhausted => Span::styled(
            " EXHAUSTED ",
            Style::default().fg(Color::White).bg(Color::Red),
        ),
        AccountState::Invalid => Span::styled(
            " INVALID ",
            Style::default().fg(Color::White).bg(Color::Red),
        ),
        AccountState::Disabled => Span::styled(
            " DISABLED ",
            Style::default().fg(Color::Gray).bg(Color::Reset),
        ),
    }
}

/// Render the header with title, daemon connection status, and navigation tabs.
pub fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(area);

    // Top title bar: Title on left, Daemon status on right
    let status_span = if app.daemon_connected {
        Span::styled("● Daemon Connected", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("○ Daemon Disconnected", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    };

    let title_line = Line::from(vec![
        Span::styled(
            "  AGENT CONTROL  ",
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  Local-first Agent Supervisor"),
        Span::raw(" ".repeat(area.width.saturating_sub(60) as usize)),
        status_span,
        Span::raw("  "),
    ]);
    f.render_widget(Paragraph::new(title_line), chunks[0]);

    // Navigation Tabs
    let pending_count = app.pending_interactions().len();
    let inbox_title = if pending_count > 0 {
        format!("3: Inbox ({pending_count} pending)")
    } else {
        "3: Inbox".to_string()
    };

    let titles = vec![
        Line::from("1: Dashboard"),
        Line::from(format!("2: Sessions ({})", app.sessions.len())),
        Line::from(inbox_title),
        Line::from(format!("4: Accounts ({})", app.accounts.len())),
        Line::from(format!("5: Projects ({})", app.projects.len())),
        Line::from("6: Activity"),
    ];

    let selected_index = if app.session_detail_id.is_some() {
        1 // Sessions tab
    } else {
        app.current_tab.to_index()
    };

    let tabs = Tabs::new(titles)
        .select(selected_index)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .divider(Span::raw(" | "));

    f.render_widget(tabs, chunks[1]);
}

/// Render the bottom footer with contextual keybindings and notification status.
pub fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Status / Notification message
    if let Some((msg, status_type, _)) = &app.status_message {
        let (color, prefix) = match status_type {
            StatusType::Info => (Color::Cyan, "[INFO]"),
            StatusType::Success => (Color::Green, "[OK]"),
            StatusType::Warning => (Color::Yellow, "[WARN]"),
            StatusType::Error => (Color::Red, "[ERR]"),
        };
        let status_p = Paragraph::new(Line::from(vec![
            Span::styled(format!(" {prefix} "), Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(msg, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]));
        f.render_widget(status_p, chunks[0]);
    }

    // Keybindings bar
    let shortcuts = match (&app.current_tab, app.session_detail_id.is_some()) {
        (_, true) => "Esc: Back | s: Steer | p: Pause | Space: Resume | x: Stop | q: Quit",
        (Tab::Dashboard, false) => "1-6: Tabs | ↑/↓: Navigate | Enter: Details | r: Refresh | ?: Help | q: Quit",
        (Tab::Sessions, false) => "↑/↓: Navigate | Enter/t: Detail | s: Steer | p: Pause | Space: Resume | x: Stop | r: Refresh | q: Quit",
        (Tab::Inbox, false) => "↑/↓: Navigate | a: Approve | d: Deny | r: Reply | x: Dismiss | r: Refresh | q: Quit",
        (Tab::Accounts, false) => "1-6: Tabs | ↑/↓: Navigate | r: Refresh | ?: Help | q: Quit",
        (Tab::Projects, false) => "1-6: Tabs | ↑/↓: Navigate | r: Refresh | ?: Help | q: Quit",
        (Tab::Activity, false) => "↑/↓: Navigate | /: Filter | c: Clear Filter | r: Refresh | q: Quit",
    };

    let help_line = Line::from(vec![
        Span::styled(" Shortcuts: ", Style::default().fg(Color::DarkGray)),
        Span::styled(shortcuts, Style::default().fg(Color::Yellow)),
    ]);
    f.render_widget(Paragraph::new(help_line), chunks[1]);
}
