//! Interactive terminal agent control screen.
//!
//! Renders:
//! ╭────────────────────────────────────────────╮
//! │ Antigravity                                │
//! │ Account: Personal Google                   │
//! │ State: Working                             │
//! ├────────────────────────────────────────────┤
//! │                                            │
//! │ Agent output...                            │
//! │                                            │
//! ├────────────────────────────────────────────┤
//! │ [s] Steer  [p] Pause  [r] Resume  [x] Stop│
//! │ [a] Switch Account   [q] Quit             │
//! ╰────────────────────────────────────────────╯

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{
    io::{self, stdout, Write},
    time::Duration,
};
use tokio::sync::mpsc;

use ac_core::types::{Account, AgentEvent, EventKind, Id, SessionState};
use crate::client::DaemonClient;

#[derive(Debug, Clone)]
pub enum InteractiveModal {
    Steer { input: String },
    SwitchAccount {
        options: Vec<Account>,
        selected_index: usize,
        confirming: bool,
    },
}

pub struct InteractiveSessionState {
    pub session_id: Id,
    pub account_label: String,
    pub agent_type: String,
    pub state: SessionState,
    pub transcript: Vec<String>,
    pub status_message: Option<String>,
    pub modal: Option<InteractiveModal>,
    pub should_quit: bool,
}

impl InteractiveSessionState {
    pub fn new(session_id: Id, account_label: String, agent_type: String) -> Self {
        Self {
            session_id,
            account_label,
            agent_type,
            state: SessionState::Working,
            transcript: Vec::new(),
            status_message: None,
            modal: None,
            should_quit: false,
        }
    }

    pub fn append_output(&mut self, text: &str) {
        for line in text.lines() {
            // Strip any raw control escapes for clean display
            self.transcript.push(line.to_string());
        }
        if self.transcript.len() > 1000 {
            self.transcript.drain(0..self.transcript.len() - 1000);
        }
    }
}

/// Attach user to the session in interactive mode.
pub async fn attach_interactive(
    client: &DaemonClient,
    session_id: Id,
    account_label: String,
    agent_type: String,
) -> Result<()> {
    // Non-TTY fallback (for CI/piped execution)
    if !crossterm::tty::IsTty::is_tty(&stdout()) {
        println!("Connected to session {} (non-TTY mode).", session_id);
        let mut rx = client.subscribe_events(Some(session_id.clone())).await?;
        while let Some(event) = rx.recv().await {
            if event.kind == EventKind::AgentOutputReceived {
                if let Some(text) = event.payload.get("text").and_then(|t| t.as_str()) {
                    print!("{text}");
                    let _ = io::stdout().flush();
                }
            } else if event.kind == EventKind::StateChanged {
                if let Some(to) = event.payload.get("to").and_then(|t| t.as_str()) {
                    println!("[State: {to}]");
                    if to == "stopped" || to == "failed" {
                        break;
                    }
                }
            }
        }
        return Ok(());
    }

    // Terminal raw mode
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = InteractiveSessionState::new(session_id.clone(), account_label.clone(), agent_type.clone());

    // Subscribe to event stream
    let mut rx = client.subscribe_events(Some(session_id.clone())).await?;

    let res = run_event_loop(&mut terminal, client, &mut state, &mut rx).await;

    // Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    println!("\nDetached from Antigravity session {}.", state.session_id);
    println!("The agent session continues running under Agent Control supervision.\n");

    res
}

async fn run_event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    client: &DaemonClient,
    state: &mut InteractiveSessionState,
    rx: &mut mpsc::Receiver<AgentEvent>,
) -> Result<()> {
    loop {
        terminal.draw(|f| render_ui(f, state))?;

        if state.should_quit {
            break;
        }

        tokio::select! {
            // Receive background events
            Some(event) = rx.recv() => {
                if event.session_id.as_ref() == Some(&state.session_id) {
                    match event.kind {
                        EventKind::AgentOutputReceived => {
                            if let Some(text) = event.payload.get("text").and_then(|t| t.as_str()) {
                                state.append_output(text);
                            }
                        }
                        EventKind::StateChanged => {
                            if let Some(to_str) = event.payload.get("to").and_then(|t| t.as_str()) {
                                if let Ok(s) = serde_json::from_value::<SessionState>(serde_json::Value::String(to_str.to_string())) {
                                    state.state = s;
                                }
                            }
                        }
                        EventKind::SessionStopped => {
                            state.state = SessionState::Stopped;
                            state.status_message = Some("Session stopped.".into());
                        }
                        _ => {}
                    }
                }
            }

            // Keyboard input
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                while event::poll(Duration::from_millis(0))? {
                    if let Event::Key(key) = event::read()? {
                        handle_key_event(client, state, key, rx).await?;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn handle_key_event(
    client: &DaemonClient,
    state: &mut InteractiveSessionState,
    key: KeyEvent,
    rx: &mut mpsc::Receiver<AgentEvent>,
) -> Result<()> {
    // ── Handle Modal Input ──────────────────────────────────────────────────
    if let Some(modal) = state.modal.take() {
        match modal {
            InteractiveModal::Steer { mut input } => match key.code {
                KeyCode::Esc => {
                    state.modal = None;
                }
                KeyCode::Enter => {
                    state.modal = None;
                    if !input.trim().is_empty() {
                        match client.steer_session(&state.session_id, &input).await {
                            Ok(()) => {
                                state.status_message = Some(format!("Instruction sent to agent."));
                            }
                            Err(e) => {
                                state.status_message = Some(format!("Failed to steer: {e}"));
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                    state.modal = Some(InteractiveModal::Steer { input });
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    state.modal = Some(InteractiveModal::Steer { input });
                }
                _ => {
                    state.modal = Some(InteractiveModal::Steer { input });
                }
            },

            InteractiveModal::SwitchAccount {
                options,
                mut selected_index,
                confirming,
            } => {
                if !confirming {
                    match key.code {
                        KeyCode::Esc => {
                            state.modal = None;
                        }
                        KeyCode::Up => {
                            if selected_index > 0 {
                                selected_index -= 1;
                            }
                            state.modal = Some(InteractiveModal::SwitchAccount {
                                options,
                                selected_index,
                                confirming: false,
                            });
                        }
                        KeyCode::Down => {
                            if selected_index + 1 < options.len() {
                                selected_index += 1;
                            }
                            state.modal = Some(InteractiveModal::SwitchAccount {
                                options,
                                selected_index,
                                confirming: false,
                            });
                        }
                        KeyCode::Enter => {
                            if !options.is_empty() {
                                state.modal = Some(InteractiveModal::SwitchAccount {
                                    options,
                                    selected_index,
                                    confirming: true,
                                });
                            }
                        }
                        _ => {
                            state.modal = Some(InteractiveModal::SwitchAccount {
                                options,
                                selected_index,
                                confirming: false,
                            });
                        }
                    }
                } else {
                    // Confirm restart step
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            let target = &options[selected_index];
                            let target_id = target.id.clone();
                            let target_label = target.label.clone();
                            state.modal = None;

                            state.status_message = Some(format!("Switching to {}...", target_label));

                            match client.switch_account(&state.session_id, &target_id).await {
                                Ok(successor_id) => {
                                    state.session_id = successor_id.clone();
                                    state.account_label = target_label.clone();
                                    state.state = SessionState::Working;
                                    state.append_output(&format!(
                                        "\n--- Switched to account {} (workspace & project files preserved) ---\n",
                                        target_label
                                    ));
                                    state.status_message = Some(format!("Switched to {}", target_label));

                                    // Resubscribe to new session
                                    if let Ok(new_rx) = client.subscribe_events(Some(successor_id)).await {
                                        *rx = new_rx;
                                    }
                                }
                                Err(e) => {
                                    state.status_message = Some(format!("Account switch failed: {e}"));
                                }
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            state.modal = Some(InteractiveModal::SwitchAccount {
                                options,
                                selected_index,
                                confirming: false,
                            });
                        }
                        _ => {
                            state.modal = Some(InteractiveModal::SwitchAccount {
                                options,
                                selected_index,
                                confirming: true,
                            });
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    // ── Handle Main Keybindings ─────────────────────────────────────────────
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            state.should_quit = true;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        KeyCode::Char('s') => {
            state.modal = Some(InteractiveModal::Steer { input: String::new() });
        }
        KeyCode::Char('p') => {
            match client.pause_session(&state.session_id).await {
                Ok(()) => {
                    state.state = SessionState::Paused;
                    state.status_message = Some("Session paused.".into());
                }
                Err(e) => state.status_message = Some(format!("Failed to pause: {e}")),
            }
        }
        KeyCode::Char('r') => {
            match client.resume_session(&state.session_id).await {
                Ok(()) => {
                    state.state = SessionState::Working;
                    state.status_message = Some("Session resumed.".into());
                }
                Err(e) => state.status_message = Some(format!("Failed to resume: {e}")),
            }
        }
        KeyCode::Char('x') => {
            match client.stop_session(&state.session_id, Some("Stopped by user")).await {
                Ok(()) => {
                    state.state = SessionState::Stopped;
                    state.status_message = Some("Session stopped.".into());
                }
                Err(e) => state.status_message = Some(format!("Failed to stop: {e}")),
            }
        }
        KeyCode::Char('a') => {
            // Open account switch modal
            let all_accounts = client.list_accounts().await.unwrap_or_default();
            let other_accounts: Vec<Account> = all_accounts
                .into_iter()
                .filter(|a| a.label != state.account_label)
                .filter(|a| a.provider == "agy" || a.supports_agent_type("agy"))
                .collect();

            if other_accounts.is_empty() {
                state.status_message = Some("No other Antigravity accounts configured.".into());
            } else {
                state.modal = Some(InteractiveModal::SwitchAccount {
                    options: other_accounts,
                    selected_index: 0,
                    confirming: false,
                });
            }
        }
        _ => {}
    }

    Ok(())
}

fn render_ui(f: &mut Frame, state: &InteractiveSessionState) {
    let area = f.area();

    // Main frame block
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Antigravity ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    f.render_widget(block, area);

    let inner_area = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header: Account & State
            Constraint::Min(5),    // Transcript output
            Constraint::Length(2), // Status bar
            Constraint::Length(2), // Shortcuts footer
        ])
        .split(inner_area);

    // 1. Header Area
    let state_badge = match state.state {
        SessionState::Working => Span::styled(" WORKING ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
        SessionState::Paused => Span::styled(" PAUSED ", Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD)),
        SessionState::WaitingForHuman => Span::styled(" WAITING FOR HUMAN ", Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
        SessionState::Stopped => Span::styled(" STOPPED ", Style::default().fg(Color::Gray).bg(Color::Reset)),
        SessionState::Crashed => Span::styled(" CRASHED ", Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD)),
        _ => Span::styled(format!(" {:?} ", state.state).to_uppercase(), Style::default().fg(Color::Cyan)),
    };

    let header_lines = vec![
        Line::from(vec![
            Span::styled("Account: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.account_label, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("State:   ", Style::default().fg(Color::DarkGray)),
            state_badge,
        ]),
    ];
    let header_p = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(header_p, layout[0]);

    // 2. Transcript Area
    let visible_lines = layout[1].height as usize;
    let start_idx = state.transcript.len().saturating_sub(visible_lines);
    let lines_to_show: Vec<Line> = state
        .transcript
        .iter()
        .skip(start_idx)
        .map(|l| Line::from(Span::raw(l)))
        .collect();

    let transcript_p = Paragraph::new(lines_to_show)
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(transcript_p, layout[1]);

    // 3. Status Message
    if let Some(ref msg) = state.status_message {
        let status_p = Paragraph::new(Line::from(vec![
            Span::styled("● ", Style::default().fg(Color::Yellow)),
            Span::styled(msg, Style::default().fg(Color::Yellow)),
        ]));
        f.render_widget(status_p, layout[2]);
    }

    // 4. Footer Shortcuts
    let shortcuts_p = Paragraph::new(Line::from(vec![
        Span::styled("[s]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" Steer  "),
        Span::styled("[p]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" Pause  "),
        Span::styled("[r]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" Resume  "),
        Span::styled("[x]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" Stop  "),
        Span::styled("[a]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" Switch Account  "),
        Span::styled("[q]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::raw(" Quit"),
    ]))
    .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(shortcuts_p, layout[3]);

    // ── Render Modals if active ─────────────────────────────────────────────
    if let Some(ref modal) = state.modal {
        match modal {
            InteractiveModal::Steer { input } => {
                let modal_area = centered_rect(60, 20, area);
                f.render_widget(Clear, modal_area);

                let p = Paragraph::new(vec![
                    Line::from(Span::styled("Enter instruction to inject:", Style::default().fg(Color::White))),
                    Line::from(""),
                    Line::from(Span::styled(format!("> {input}_"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
                    Line::from(""),
                    Line::from(Span::styled("[Enter] Send   [Esc] Cancel", Style::default().fg(Color::DarkGray))),
                ])
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow))
                        .title(" Steer Agent "),
                );
                f.render_widget(p, modal_area);
            }

            InteractiveModal::SwitchAccount {
                options,
                selected_index,
                confirming,
            } => {
                let modal_area = centered_rect(70, 40, area);
                f.render_widget(Clear, modal_area);

                if !confirming {
                    let mut lines = vec![
                        Line::from(vec![
                            Span::styled("Current account: ", Style::default().fg(Color::DarkGray)),
                            Span::styled(&state.account_label, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        ]),
                        Line::from(""),
                        Line::from(Span::styled("Switch to:", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
                        Line::from(""),
                    ];

                    for (i, opt) in options.iter().enumerate() {
                        let is_sel = i == *selected_index;
                        let prefix = if is_sel { "> " } else { "  " };
                        let style = if is_sel {
                            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        lines.push(Line::from(vec![
                            Span::styled(prefix, style),
                            Span::styled(&opt.label, style),
                        ]));
                    }

                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "[↑/↓] Select   [Enter] Switch   [Esc] Cancel",
                        Style::default().fg(Color::DarkGray),
                    )));

                    let p = Paragraph::new(lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Cyan))
                            .title(" Switch Account "),
                    );
                    f.render_widget(p, modal_area);
                } else {
                    let target = &options[*selected_index];
                    let lines = vec![
                        Line::from(Span::styled("Switching account...", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
                        Line::from(""),
                        Line::from("This provider requires restarting the agent."),
                        Line::from(Span::styled(
                            "Your project files and workspace will be preserved.",
                            Style::default().fg(Color::Green),
                        )),
                        Line::from(""),
                        Line::from(vec![
                            Span::raw("Switch from "),
                            Span::styled(&state.account_label, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Span::raw(" to "),
                            Span::styled(&target.label, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                            Span::raw("?"),
                        ]),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Continue? [Y/n]",
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                        )),
                    ];

                    let p = Paragraph::new(lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Yellow))
                            .title(" Confirm Account Switch "),
                    );
                    f.render_widget(p, modal_area);
                }
            }
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
