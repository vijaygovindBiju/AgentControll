//! `ac_tui` — Agent Control Ratatui TUI crate.

pub mod app;
pub mod client;
pub mod clipboard;
pub mod event;
pub mod launch;
pub mod prompt_history;
pub mod terminal_buffer;
pub mod ui;
pub mod views;

use anyhow::Result;
use crossterm::{
    cursor::Show,
    event::{DisableBracketedPaste, EnableBracketedPaste, Event, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{io::stdout, path::PathBuf, time::Duration};
use tokio_stream::StreamExt;

pub use app::App;
pub use client::ApiClient;

/// RAII guard ensuring the terminal is always returned to canonical state on exit or panic.
pub struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            Show,
        );
    }
}

/// Run the interactive Ratatui TUI dashboard until user exits.
pub async fn run_tui(socket_path: PathBuf) -> Result<()> {
    // ── 1. Setup terminal ───────────────────────────────────────────────────
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
    let _guard = TerminalGuard;

    // Install panic hook to restore terminal before printing panics
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            Show,
        );
        prev_hook(info);
    }));

    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;

    // ── 2. Setup App & API Client ───────────────────────────────────────────
    let client = ApiClient::new(socket_path);
    let mut app = App::new();

    // Initial data load
    event::refresh_data(&mut app, &client).await;

    // Detect the installed agy's permission modes off the UI thread so the
    // start-session form opens instantly.
    tokio::task::spawn_blocking(ac_core::agy_launch::supported_permission_modes);

    // Connect event subscriber stream
    let mut event_rx = client.subscribe_events().await.ok();

    // Event reader stream
    let mut reader = EventStream::new();
    let mut tick_interval = tokio::time::interval(Duration::from_millis(33));

    // ── 3. Main Event Loop ──────────────────────────────────────────────────
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            // OS interrupt signal (Ctrl+C from outside / SIGINT)
            _ = tokio::signal::ctrl_c() => {
                app.should_quit = true;
            }

            // Terminal input
            Some(Ok(evt)) = reader.next() => {
                match evt {
                    Event::Key(key) => {
                        let _ = event::handle_key(&mut app, &client, key).await;
                    }
                    Event::Paste(ref text) => {
                        let _ = event::handle_paste(&mut app, &client, text).await;
                    }
                    Event::Resize(cols, rows) => {
                        let _ = terminal.clear();
                        let p_rows = rows.saturating_sub(2).max(1);
                        let p_cols = cols.max(1);
                        app.resize_session_terminals(p_rows, p_cols);
                        if let Some(ref sid) = app.session_detail_id {
                            let _ = client.resize_session(sid, p_rows, p_cols).await;
                        }
                    }
                    _ => {}
                }
            }

            // Daemon live event stream
            Some(agent_evt) = async {
                match &mut event_rx {
                    Some(rx) => rx.recv().await,
                    None => None,
                }
            } => {
                app.apply_event(agent_evt);
                if let Some(ref mut rx) = event_rx {
                    while let Ok(next_evt) = rx.try_recv() {
                        app.apply_event(next_evt);
                    }
                }
            }

            // Periodic tick for animations, status expiry, and reconnection
            _ = tick_interval.tick() => {
                app.clear_expired_status();
                event::poll_agy_login(&mut app, &client).await;
                launch::poll_jobs(&mut app);
                if app.drain_background_notices() {
                    event::refresh_data(&mut app, &client).await;
                }
                // Check reconnection if disconnected
                let reachable = client.check_daemon().await;
                if reachable != app.daemon_connected {
                    app.daemon_connected = reachable;
                    if reachable {
                        app.set_status("Reconnected to daemon", app::StatusType::Success);
                        event::refresh_data(&mut app, &client).await;
                        if event_rx.is_none() {
                            event_rx = client.subscribe_events().await.ok();
                        }
                    } else {
                        app.set_status("Lost connection to daemon", app::StatusType::Warning);
                        event_rx = None;
                    }
                }
            }
        }
    }

    // ── 4. Restore terminal ─────────────────────────────────────────────────
    // Never leave a login callback listener running after the TUI exits.
    event::cancel_agy_login(&mut app);
    drop(_guard);

    Ok(())
}
