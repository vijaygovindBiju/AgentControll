//! `agent-control` — Main user-facing interactive interface for Agent Control.
//!
//! Provides the primary interactive dashboard:
//! AGENT CONTROL
//!
//! AGENTS
//!   Antigravity       3 accounts     1 running
//!   Claude Code       2 accounts     0 running
//!
//! ACCOUNTS
//!   Personal Google   Antigravity    Ready
//!   College Google    Antigravity    Ready
//!   Work Google       Antigravity    Cooldown
//!   Claude Main       Claude Code    Ready
//!
//! SESSIONS
//!   Antigravity       Personal Google    AgentMesh     Working
//!
//! [Enter] Open
//! [A] Add Account
//! [N] New Agent
//! [S] Sessions
//! [Q] Quit

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use ac_cli::client::DaemonClient;

#[derive(Parser)]
#[command(
    name = "agent-control",
    version,
    about = "Agent Control Interactive Supervisor Dashboard",
    long_about = "Manage multiple agent accounts, monitor running sessions, and control approvals in real time."
)]
struct Cli {
    /// Path to the daemon Unix socket.
    #[arg(long, env = "AC_SOCKET")]
    socket: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(|| {
        std::env::var("AC_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|_| ac_core::config::Config::default().socket_path)
    });

    let client = DaemonClient::new(socket.clone());
    client.ensure_daemon_running().await?;

    ac_tui::run_tui(socket).await
}
