//! `ac-tui` — Agent Control Ratatui Dashboard binary.

use std::path::PathBuf;
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "ac-tui",
    version,
    about = "Agent Control Interactive Terminal Dashboard & Inbox"
)]
struct Args {
    /// Path to the daemon Unix socket.
    #[arg(
        long,
        env = "AC_SOCKET",
        default_value = "/tmp/agentcontrol.sock"
    )]
    socket: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    ac_tui::run_tui(args.socket).await
}
