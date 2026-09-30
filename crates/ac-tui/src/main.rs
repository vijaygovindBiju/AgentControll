//! `ac-tui` — Agent Control Ratatui Dashboard binary.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

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
        default_value_os_t = ac_core::config::Config::default().socket_path
    )]
    socket: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    ac_tui::run_tui(args.socket).await
}
