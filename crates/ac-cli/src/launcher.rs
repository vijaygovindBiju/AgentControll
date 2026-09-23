//! User-friendly Antigravity agent launcher and account UX.
//!
//! Enforces:
//! - Explicit account choices are NEVER silently overridden.
//! - Clear, normal-user error messages.
//! - Zero credential leakage.
//! - Controlled session hand-off for account switching.

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::{
    io::{self, stdout, Write},
    path::PathBuf,
    time::Duration,
};

use ac_core::types::{Account, AccountState, Id};
use crate::{auth, client::DaemonClient, interactive};

/// Main entry point for the `agy` launcher command.
pub async fn run_agy_launcher(
    explicit_label: Option<String>,
    socket_path: Option<PathBuf>,
) -> Result<()> {
    let client = if let Some(p) = socket_path {
        DaemonClient::new(p)
    } else {
        DaemonClient::default_client()
    };

    // Ensure daemon is running
    client.ensure_daemon_running().await?;

    let all_accounts = client.list_accounts().await?;
    let agy_accounts: Vec<Account> = all_accounts
        .iter()
        .filter(|a| is_antigravity_account(a))
        .cloned()
        .collect();

    let chosen_account = match explicit_label {
        Some(label) => {
            handle_explicit_account(&client, &all_accounts, &agy_accounts, &label).await?
        }
        None => {
            handle_unspecified_account(&client, &agy_accounts).await?
        }
    };

    let chosen_account = match chosen_account {
        Some(a) => a,
        None => return Ok(()), // User cancelled
    };

    // Determine current project if cwd is registered
    let project_id = find_current_project(&client).await;

    println!("Starting Antigravity with account \"{}\"...", chosen_account.label);

    let session_id = client
        .create_and_start_session(
            "Interactive Antigravity coding session",
            "agy",
            project_id,
            Some(chosen_account.id.clone()),
        )
        .await
        .context("Starting Antigravity session")?;

    println!("Session {} started. Attaching to agent...", session_id);

    // Attach user to interactive screen
    interactive::attach_interactive(&client, session_id, chosen_account.label, "agy".into()).await
}

/// Helper to determine if an account is compatible with Antigravity.
pub fn is_antigravity_account(account: &Account) -> bool {
    account.provider == "agy"
        || account.provider == "antigravity"
        || account.supports_agent_type("agy")
        || account.supports_agent_type("antigravity")
}

/// Handle explicit account invocation: `agy "Personal Google"`
async fn handle_explicit_account(
    client: &DaemonClient,
    all_accounts: &[Account],
    agy_accounts: &[Account],
    target_label: &str,
) -> Result<Option<Account>> {
    let needle = target_label.trim();

    // Check if account exists under Antigravity
    let matching_agy: Vec<Account> = agy_accounts
        .iter()
        .filter(|a| a.label.eq_ignore_ascii_case(needle))
        .cloned()
        .collect();

    if matching_agy.is_empty() {
        // Check if it belongs to another provider (e.g. Claude)
        if let Some(other) = all_accounts.iter().find(|a| a.label.eq_ignore_ascii_case(needle)) {
            eprintln!("\n\"{}\" cannot be used with Antigravity.\n", other.label);
            eprintln!("Provider: {}", other.provider);
            eprintln!("Supported agents: {}\n", other.agent_types.join(", "));
            anyhow::bail!("Incompatible account");
        }

        // Account not found at all
        eprintln!("\nAccount \"{}\" not found.\n", target_label);
        if agy_accounts.is_empty() {
            eprintln!("No Antigravity accounts are configured.");
            eprintln!("To add a new account, run:\n  agy account add\n");
        } else {
            eprintln!("Available Antigravity accounts:");
            for a in agy_accounts {
                eprintln!("  • {}", a.label);
            }
            eprintln!("\nTo add a new account, run:\n  agy account add\n");
        }
        anyhow::bail!("AccountNotFound");
    }

    // Handle duplicate labels gracefully
    let account = if matching_agy.len() == 1 {
        matching_agy.into_iter().next().unwrap()
    } else {
        println!("\nMultiple accounts found named \"{}\":", target_label);
        for (i, a) in matching_agy.iter().enumerate() {
            println!("  {}. {} (ID: {}, Created: {})", i + 1, a.label, a.id, a.created_at.format("%Y-%m-%d"));
        }
        print!("Select account [1-{}]: ", matching_agy.len());
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let idx = line.trim().parse::<usize>().unwrap_or(1);
        let idx = if idx >= 1 && idx <= matching_agy.len() { idx - 1 } else { 0 };
        matching_agy[idx].clone()
    };

    // Verify account availability
    validate_account_availability(client, agy_accounts, &account).await
}

/// Validate availability for an explicitly chosen account without silent fallback.
async fn validate_account_availability(
    client: &DaemonClient,
    agy_accounts: &[Account],
    account: &Account,
) -> Result<Option<Account>> {
    // 1. Check rate limit / cooldown
    if matches!(account.state, AccountState::RateLimited | AccountState::Cooldown) {
        let remaining_secs = account
            .cooldown_until
            .map(|t| (t - Utc::now()).num_seconds().max(1))
            .unwrap_or(30);

        eprintln!("\nAccount \"{}\" is currently unavailable.\n", account.label);
        eprintln!("Reason:\nRate limited for another {} seconds.\n", remaining_secs);

        print!("Try another account? [Y/n] ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let choice = line.trim().to_lowercase();

        if choice.is_empty() || choice == "y" || choice == "yes" {
            let other_accounts: Vec<Account> = agy_accounts
                .iter()
                .filter(|a| a.id != account.id)
                .cloned()
                .collect();
            return select_interactive_account(client, &other_accounts).await;
        } else {
            return Ok(None);
        }
    }

    // 2. Check disabled / exhausted
    if !account.state.is_available() {
        eprintln!("\nAccount \"{}\" is currently unavailable.\n", account.label);
        eprintln!("Reason:\nAccount is in state '{}'.\n", account.state);

        print!("Try another account? [Y/n] ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let choice = line.trim().to_lowercase();

        if choice.is_empty() || choice == "y" || choice == "yes" {
            let other_accounts: Vec<Account> = agy_accounts
                .iter()
                .filter(|a| a.id != account.id)
                .cloned()
                .collect();
            return select_interactive_account(client, &other_accounts).await;
        } else {
            return Ok(None);
        }
    }

    // 3. Check concurrency limit
    if account.active_session_count >= account.concurrency_cap {
        eprintln!(
            "\n\"{}\" is already running its maximum\nnumber of Antigravity sessions ({}/{}).\n",
            account.label, account.active_session_count, account.concurrency_cap
        );
        eprintln!("Choose another account.\n");

        let other_accounts: Vec<Account> = agy_accounts
            .iter()
            .filter(|a| a.id != account.id)
            .cloned()
            .collect();
        return select_interactive_account(client, &other_accounts).await;
    }

    Ok(Some(account.clone()))
}

/// Handle unspecified account invocation: `agy`
async fn handle_unspecified_account(
    client: &DaemonClient,
    agy_accounts: &[Account],
) -> Result<Option<Account>> {
    // 0 accounts configured
    if agy_accounts.is_empty() {
        println!("No Antigravity accounts are configured.\n");
        println!("Add an account?\n");
        println!("> 1. Login with browser");
        println!("  2. Cancel\n");

        print!("Select [1-2]: ");
        io::stdout().flush()?;

        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let choice = line.trim();

        if choice == "1" || choice.eq_ignore_ascii_case("login") || choice.is_empty() {
            let aid = auth::run_add_account_flow(client, None, false).await?;
            let accounts = client.list_accounts().await?;
            return Ok(accounts.into_iter().find(|a| a.id == aid));
        } else {
            println!("Cancelled.");
            return Ok(None);
        }
    }

    // Exactly 1 usable account
    let usable_accounts: Vec<&Account> = agy_accounts
        .iter()
        .filter(|a| a.can_accept_session())
        .collect();

    if agy_accounts.len() == 1 && usable_accounts.len() == 1 {
        let single = usable_accounts[0];
        println!("Using account: {}", single.label);
        return Ok(Some(single.clone()));
    }

    // Multiple accounts: show interactive account selector
    select_interactive_account(client, agy_accounts).await
}

/// Interactive terminal account selector.
pub async fn select_interactive_account(
    client: &DaemonClient,
    accounts: &[Account],
) -> Result<Option<Account>> {
    if accounts.is_empty() {
        println!("No accounts available to select.");
        return Ok(None);
    }

    // Fallback to simple number prompt if not a TTY
    if !crossterm::tty::IsTty::is_tty(&stdout()) {
        println!("\nSelect Antigravity account:\n");
        for (i, a) in accounts.iter().enumerate() {
            let status = format_account_status(a);
            println!("  {}. {:<20} {}", i + 1, a.label, status);
        }
        print!("\nEnter number [1-{}]: ", accounts.len());
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let idx = line.trim().parse::<usize>().unwrap_or(1);
        let idx = if idx >= 1 && idx <= accounts.len() { idx - 1 } else { 0 };
        return Ok(Some(accounts[idx].clone()));
    }

    enable_raw_mode()?;
    let mut selected_idx: usize = 0;

    let res = loop {
        print_selector(accounts, selected_idx)?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        if selected_idx > 0 {
                            selected_idx -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if selected_idx + 1 < accounts.len() {
                            selected_idx += 1;
                        }
                    }
                    KeyCode::Enter => {
                        let chosen = accounts[selected_idx].clone();
                        let _ = disable_raw_mode();
                        println!("\nSelected: {}", chosen.label);
                        break Ok(Some(chosen));
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        let _ = disable_raw_mode();
                        println!();
                        let aid = auth::run_add_account_flow(client, None, false).await?;
                        let all = client.list_accounts().await?;
                        break Ok(all.into_iter().find(|a| a.id == aid));
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        let _ = disable_raw_mode();
                        println!("\nCancelled.");
                        break Ok(None);
                    }
                    _ => {}
                }
            }
        }
    };

    let _ = disable_raw_mode();
    res
}

fn print_selector(accounts: &[Account], selected_idx: usize) -> Result<()> {
    print!("\r\x1b[2J\x1b[H"); // Clear screen and move cursor to top
    println!("Select Antigravity account\n");

    for (i, a) in accounts.iter().enumerate() {
        let is_selected = i == selected_idx;
        let prefix = if is_selected { "> " } else { "  " };
        let status = format_account_status(a);

        if is_selected {
            println!("\x1b[1;36m{}{:<22} {}\x1b[0m", prefix, a.label, status);
        } else {
            println!("{}{:<22} {}", prefix, a.label, status);
        }
    }

    println!("\n\x1b[90m[Enter] Select   [A] Add Account   [Q] Quit\x1b[0m");
    io::stdout().flush()?;
    Ok(())
}

fn format_account_status(account: &Account) -> String {
    match account.state {
        AccountState::Active => {
            if account.active_session_count >= account.concurrency_cap {
                format!("Busy ({}/{})", account.active_session_count, account.concurrency_cap)
            } else {
                "Ready".to_string()
            }
        }
        AccountState::RateLimited | AccountState::Cooldown => {
            let secs = account
                .cooldown_until
                .map(|t| (t - Utc::now()).num_seconds().max(1))
                .unwrap_or(30);
            format!("Cooldown ({}s)", secs)
        }
        AccountState::Exhausted => "Exhausted".to_string(),
        AccountState::Disabled => "Disabled".to_string(),
        AccountState::Invalid => "Invalid".to_string(),
    }
}

async fn find_current_project(client: &DaemonClient) -> Option<Id> {
    let current_dir = std::env::current_dir().ok()?;
    let current_path_str = current_dir.to_string_lossy().to_string();

    let projects = client.list_projects().await.ok()?;
    for p in projects {
        if p.repo_path == current_path_str {
            return Some(p.id);
        }
    }
    None
}
