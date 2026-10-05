//! `agentcontroll` — Main user-facing interactive interface for AgentControll.
//!
//! Provides the primary interactive supervisor dashboard and session manager.

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

use ac_cli::client::DaemonClient;

#[derive(Parser)]
#[command(
    name = "agentcontroll",
    version,
    about = "AgentControll Interactive Supervisor Dashboard",
    long_about = "Manage multiple agent accounts, monitor running sessions, and control approvals in real time."
)]
struct Cli {
    /// Path to the daemon Unix socket.
    #[arg(long, env = "AC_SOCKET")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Diagnose AgentControll installation, configuration, Antigravity integration, accounts, profiles, and sessions.
    Doctor(DoctorArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Perform additional safe deep diagnostics.
    #[arg(long)]
    pub deep: bool,

    /// Return machine-readable JSON.
    #[arg(long)]
    pub json: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(|| {
        std::env::var("AC_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|_| ac_core::config::Config::default().socket_path)
    });

    if let Some(Commands::Doctor(args)) = cli.command {
        let code = ac_cli::doctor::run_doctor(ac_core::doctor::DoctorOpts {
            deep: args.deep,
            json: args.json,
            socket_path: Some(socket),
            ..Default::default()
        })
        .await?;
        std::process::exit(code);
    }

    let client = DaemonClient::new(socket.clone());
    client.ensure_daemon_running().await?;

    ac_tui::run_tui(socket).await
}
