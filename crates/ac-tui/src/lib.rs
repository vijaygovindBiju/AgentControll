//! `ac_tui` — Agent Control Ratatui TUI crate.

pub mod app;
pub mod client;
pub mod event;
pub mod terminal_buffer;
pub mod ui;
pub mod views;

use std::{io::stdout, path::PathBuf, time::Duration};
use anyhow::Result;
use crossterm::{
    event::{Event, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio_stream::StreamExt;

pub use app::App;
pub use client::ApiClient;

/// Run the interactive Ratatui TUI dashboard until user exits.
pub async fn run_tui(socket_path: PathBuf) -> Result<()> {
    // ── 1. Setup terminal ───────────────────────────────────────────────────
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // ── 2. Setup App & API Client ───────────────────────────────────────────
    let client = ApiClient::new(socket_path);
    let mut app = App::new();

    // Initial data load
    event::refresh_data(&mut app, &client).await;

    // Connect event subscriber stream
    let mut event_rx = client.subscribe_events().await.ok();

    // Event reader stream
    let mut reader = EventStream::new();
    let mut tick_interval = tokio::time::interval(Duration::from_millis(200));

    // ── 3. Main Event Loop ──────────────────────────────────────────────────
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            // Terminal input
            Some(Ok(evt)) = reader.next() => {
                if let Event::Key(key) = evt {
                    let _ = event::handle_key(&mut app, &client, key).await;
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
            }

            // Periodic tick for animations, status expiry, and reconnection
            _ = tick_interval.tick() => {
                app.clear_expired_status();
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
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}
