//! `agy` — User-friendly Antigravity Agent Launcher & Account UX.
//!
//! Usage:
//!   agy                     Launch Antigravity (interactive account selection if multiple)
//!   agy "Personal Google"   Launch Antigravity using the saved account "Personal Google"
//!   agy account add         Add a new Antigravity account (browser login / manual code)
//!   agy account list        List all configured Antigravity accounts

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use ac_cli::{auth, client::DaemonClient, launcher};

#[derive(Parser)]
#[command(
    name = "agy",
    version,
    about = "Antigravity Agent Launcher (powered by Agent Control)",
    long_about = "Launch and manage Antigravity agent sessions and multiple Google accounts effortlessly."
)]
struct Cli {
    /// Friendly account name to use (e.g. "Personal Google", "Work Google").
    #[arg(index = 1)]
    account: Option<String>,

    /// Subcommands for account management.
    #[command(subcommand)]
    command: Option<Commands>,

    /// Optional path to the daemon Unix socket.
    #[arg(long, env = "AC_SOCKET")]
    socket: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Account management commands.
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// Add a new account (shortcut for 'agy account add').
    Add {
        /// Optional friendly label for the account.
        #[arg(short, long)]
        name: Option<String>,
        /// Perform login purely in terminal without browser.
        #[arg(long)]
        cli: bool,
    },
    /// List configured accounts (shortcut for 'agy account list').
    List,
}

#[derive(Subcommand)]
enum AccountAction {
    /// Add a new Antigravity account using browser authentication or CLI.
    Add {
        /// Optional friendly label for the account.
        #[arg(short, long)]
        name: Option<String>,
        /// Perform login purely in terminal without browser.
        #[arg(long)]
        cli: bool,
    },
    /// List all configured Antigravity accounts.
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = if let Some(ref p) = cli.socket {
        DaemonClient::new(p.clone())
    } else {
        DaemonClient::default_client()
    };

    if let Some(cmd) = cli.command {
        client.ensure_daemon_running().await?;

        match cmd {
            Commands::Account { action } => match action {
                AccountAction::Add { name, cli } => {
                    auth::run_add_account_flow(&client, name, cli).await?;
                }
                AccountAction::List => {
                    list_antigravity_accounts(&client).await?;
                }
            },
            Commands::Add { name, cli } => {
                auth::run_add_account_flow(&client, name, cli).await?;
            }
            Commands::List => {
                list_antigravity_accounts(&client).await?;
            }
        }
        return Ok(());
    }

    // Standard launcher invocation: agy or agy "Account Name"
    launcher::run_agy_launcher(cli.account, cli.socket).await
}

async fn list_antigravity_accounts(client: &DaemonClient) -> Result<()> {
    let accounts = client.list_accounts().await?;
    let agy_accounts: Vec<_> = accounts
        .into_iter()
        .filter(|a| launcher::is_antigravity_account(a))
        .collect();

    if agy_accounts.is_empty() {
        println!("No Antigravity accounts are configured.");
        println!("To add an account, run:\n  agy account add\n");
        return Ok(());
    }

    println!("\nConfigured Antigravity Accounts:");
    println!("{:<24}  {:<12}  {}", "ACCOUNT NAME", "STATUS", "CONCURRENCY");
    println!("{}", "-".repeat(50));

    for a in agy_accounts {
        let status = match a.state {
            ac_core::types::AccountState::Active => {
                if a.active_session_count >= a.concurrency_cap {
                    format!("Busy ({}/{})", a.active_session_count, a.concurrency_cap)
                } else {
                    "Ready".to_string()
                }
            }
            ac_core::types::AccountState::RateLimited | ac_core::types::AccountState::Cooldown => {
                "Cooldown".to_string()
            }
            ac_core::types::AccountState::Exhausted => "Exhausted".to_string(),
            ac_core::types::AccountState::Disabled => "Disabled".to_string(),
            ac_core::types::AccountState::Invalid => "Invalid".to_string(),
        };
        println!("{:<24}  {:<12}  {}/{}", a.label, status, a.active_session_count, a.concurrency_cap);
    }
    println!();
    Ok(())
}
