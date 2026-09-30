//! Views module for Ratatui TUI.

pub mod dashboard;
pub mod sessions;
pub mod session_detail;
pub mod inbox;
pub mod accounts;
pub mod projects;
pub mod activity;
pub mod agents;
pub mod settings;
pub mod modal;
pub mod start_session;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
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

/// Render the header with title, system description, timestamp, and daemon status.
pub fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let status_span = if app.daemon_connected {
        Span::styled("● Daemon: Running", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("○ Daemon: Disconnected", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
    };

    let now_str = chrono::Local::now().format("%a %b %d %H:%M").to_string();

    // Line 1: AGENT CONTROL title + description + date/time
    let header_line1 = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(23),
            Constraint::Min(20),
            Constraint::Length(18),
        ])
        .split(chunks[0]);

    let left_title = Paragraph::new(Span::styled(" AGENT CONTROL v1.0.0", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    let center_desc = Paragraph::new(Span::styled("Coding Agents • Multiple Accounts • Full Control", Style::default().fg(Color::Rgb(140, 150, 165))))
        .alignment(ratatui::layout::Alignment::Center);
    let right_time = Paragraph::new(Span::styled(format!("{now_str} "), Style::default().fg(Color::DarkGray)))
        .alignment(ratatui::layout::Alignment::Right);

    f.render_widget(left_title, header_line1[0]);
    f.render_widget(center_desc, header_line1[1]);
    f.render_widget(right_time, header_line1[2]);

    let titles = vec![
        Line::from("1: Dashboard"),
        Line::from(format!("2: Sessions ({})", app.sessions.len())),
        Line::from(format!("3: Accounts ({})", app.accounts.len())),
        Line::from("4: Activity"),
        Line::from("5: Agents"),
        Line::from("6: Settings"),
    ];

    let selected_index = if app.session_detail_id.is_some() {
        1
    } else {
        app.current_tab.to_index().min(5)
    };

    let nav_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(50), Constraint::Length(25)])
        .split(chunks[1]);

    let tabs = Tabs::new(titles)
        .select(selected_index)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
        .divider(Span::raw(" | "));

    f.render_widget(tabs, nav_chunks[0]);

    let status_p = Paragraph::new(Line::from(vec![
        status_span,
        Span::raw(" "),
    ])).alignment(ratatui::layout::Alignment::Right);
    f.render_widget(status_p, nav_chunks[1]);
}

/// Render the btop-style left sidebar menu and quick-launch panel.
pub fn render_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(14), // Menu
            Constraint::Min(6),     // Quick Launch
        ])
        .split(area);

    // 1. Menu block
    let menu_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(" ▸ Menu ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));

    let items = [
        (0, "Dashboard", Tab::Dashboard),
        (1, "Agents", Tab::Agents),
        (2, "Accounts", Tab::Accounts),
        (3, "Sessions", Tab::Sessions),
        (4, "Events", Tab::Activity),
        (5, "Add Account", Tab::Accounts),
        (6, "New Agent", Tab::Agents),
        (7, "Settings", Tab::Settings),
        (8, "Help", Tab::Dashboard),
        (9, "Quit", Tab::Dashboard),
    ];

    let mut lines = Vec::new();
    for (idx, name, _tab) in items.iter() {
        if *idx == 5 || *idx == 7 {
            lines.push(Line::from(Span::styled(" ─────────────", Style::default().fg(Color::DarkGray))));
        }

        let is_active_tab = match idx {
            0 => app.current_tab == Tab::Dashboard && app.session_detail_id.is_none(),
            1 => app.current_tab == Tab::Agents,
            2 => app.current_tab == Tab::Accounts,
            3 => app.current_tab == Tab::Sessions || app.session_detail_id.is_some(),
            4 => app.current_tab == Tab::Activity,
            7 => app.current_tab == Tab::Settings,
            _ => false,
        };

        let is_cursor = app.sidebar_selected == *idx;

        let line = if is_cursor || is_active_tab {
            Line::from(vec![
                Span::styled(" ▶ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{:<11}", name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ])
        } else {
            Line::from(vec![
                Span::raw("   "),
                Span::styled(format!("{:<11}", name), Style::default().fg(Color::Rgb(160, 170, 185))),
            ])
        };
        lines.push(line);
    }

    let menu_p = Paragraph::new(lines).block(menu_block);
    f.render_widget(menu_p, chunks[0]);

    // 2. Quick Launch block
    let ql_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(" Quick Launch ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));

    let ql_lines = vec![
        Line::from(vec![
            Span::styled(" [g] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("Antigravity", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" [c] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("Claude Code", Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(" [x] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("Codex / PTY", Style::default().fg(Color::White)),
        ]),
    ];

    let ql_p = Paragraph::new(ql_lines).block(ql_block);
    f.render_widget(ql_p, chunks[1]);
}

/// Render the bottom footer with contextual keybinding pills and status.
pub fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Line 0: Status notification (if active)
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
    } else {
        let ready_line = Line::from(vec![
            Span::styled(" Ready", Style::default().fg(Color::DarkGray)),
            Span::raw(" ".repeat(area.width.saturating_sub(15) as usize)),
            Span::styled("● Ready", Style::default().fg(Color::Green)),
        ]);
        f.render_widget(Paragraph::new(ready_line), chunks[0]);
    }

    // Line 1: Tag pills for shortcuts
    let shortcut_pills: Vec<Span> = match (&app.current_tab, app.session_detail_id.is_some()) {
        (_, true) => vec![
            tag_pill("s", "Steer"),
            tag_pill("p", "Pause"),
            tag_pill("r", "Resume"),
            tag_pill("x", "Stop"),
            tag_pill("a", "Switch Account"),
            tag_pill("Esc", "Back"),
            tag_pill("↑↓", "Scroll"),
            tag_pill("End", "Follow"),
        ],
        (Tab::Dashboard, false) => vec![
            tag_pill("↑↓", "Select"),
            tag_pill("Enter", "Open"),
            tag_pill("a", "Add Account"),
            tag_pill("n", "New Agent"),
            tag_pill("s", "Sessions"),
            tag_pill("e", "Events"),
            tag_pill("/", "Search"),
            tag_pill("?", "Help"),
            tag_pill("q", "Quit"),
        ],
        (Tab::Sessions, false) => vec![
            tag_pill("↑↓", "Navigate"),
            tag_pill("Enter", "Detail"),
            tag_pill("Space", "Run"),
            tag_pill("w", "Switch Acct"),
            tag_pill("s", "Steer"),
            tag_pill("p", "Pause"),
            tag_pill("x", "Stop"),
            tag_pill("q", "Quit"),
        ],
        (Tab::Accounts, false) => vec![
            tag_pill("↑↓", "Navigate"),
            tag_pill("a", "Add Account"),
            tag_pill("d", "Delete Account"),
            tag_pill("s", "Set Default"),
            tag_pill("r", "Refresh"),
            tag_pill("?", "Help"),
            tag_pill("q", "Quit"),
        ],
        (Tab::Activity, false) => vec![
            tag_pill("↑↓", "Navigate"),
            tag_pill("/", "Filter"),
            tag_pill("c", "Clear Filter"),
            tag_pill("r", "Refresh"),
            tag_pill("q", "Quit"),
        ],
        (Tab::Agents, false) => vec![
            tag_pill("↑↓", "Select"),
            tag_pill("Enter", "New Session"),
            tag_pill("1-6", "Tabs"),
            tag_pill("?", "Help"),
            tag_pill("q", "Quit"),
        ],
        (Tab::Settings, false) => vec![
            tag_pill("↑↓", "Select"),
            tag_pill("Enter/→", "Focus Panel"),
            tag_pill("Esc/←", "Sections"),
            tag_pill("a", "Add/Register"),
            tag_pill("d", "Remove"),
            tag_pill("s", "Set Default"),
            tag_pill("1-6", "Tabs"),
            tag_pill("q", "Quit"),
        ],
    };

    let flat_spans: Vec<Span> = shortcut_pills;
    f.render_widget(Paragraph::new(Line::from(flat_spans)), chunks[1]);
}

fn tag_pill(key: &'static str, action: &'static str) -> Span<'static> {
    Span::styled(
        format!(" [{key}] {action} "),
        Style::default().fg(Color::Yellow),
    )
}
