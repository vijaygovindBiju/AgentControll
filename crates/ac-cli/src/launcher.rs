//! User-friendly Antigravity agent launcher and account UX.
//!
//! Enforces:
//! - Explicit account choices are NEVER silently overridden.
//! - Invalid or ambiguous input never selects an account implicitly.
//! - Clear, normal-user error messages.
//! - Zero credential leakage.
//! - Raw terminal mode is always restored.
//! - Controlled session hand-off for account switching.

use anyhow::{Context, Result};
use chrono::Utc;
use crossterm::{
    cursor::{Hide, MoveToPreviousLine, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    queue,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
    tty::IsTty,
};
use std::{
    io::{self, stdout, Write},
    path::PathBuf,
};

use crate::{auth, client::DaemonClient, interactive};
use ac_core::{
    agy_auth,
    types::{Account, AccountState, Id},
};

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
        None => handle_unspecified_account(&client, &agy_accounts).await?,
    };

    let chosen_account = match chosen_account {
        Some(a) => a,
        None => return Ok(()), // User cancelled
    };

    // Fail early with a clear message; the daemon re-validates at launch.
    let cred = agy_auth::validate_credential(&chosen_account.credential_ref, &chosen_account.label)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Determine current project if cwd is registered
    let project_id = find_current_project(&client).await;

    println!(
        "Starting Antigravity with account \"{}\"...",
        chosen_account.label
    );
    if let Some(email) = &cred.email {
        println!("Google account: {email}");
    }

    let session_id = client
        .create_and_start_session(
            "[interactive] Antigravity coding session",
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

/// Result of resolving a user-supplied account name or ID.
#[derive(Debug)]
pub enum AccountMatch<'a> {
    One(&'a Account),
    Many(Vec<&'a Account>),
    NotFound,
}

/// Resolve by exact account ID first, then by (case-insensitive) exact name.
/// Never guesses: multiple name matches are reported as ambiguous.
pub fn resolve_account<'a>(accounts: &'a [Account], needle: &str) -> AccountMatch<'a> {
    let needle = needle.trim();
    if let Some(a) = accounts.iter().find(|a| a.id.0 == needle) {
        return AccountMatch::One(a);
    }
    let mut named: Vec<&Account> = accounts
        .iter()
        .filter(|a| a.label.eq_ignore_ascii_case(needle))
        .collect();
    match named.len() {
        0 => AccountMatch::NotFound,
        1 => AccountMatch::One(named.remove(0)),
        _ => AccountMatch::Many(named),
    }
}

/// Parse a 1-based numeric selection.
/// `Ok(Some(n))` valid, `Ok(None)` cancel (`q`), `Err` invalid (caller re-prompts).
pub fn parse_selection(input: &str, max: usize) -> std::result::Result<Option<usize>, String> {
    let s = input.trim();
    if s.eq_ignore_ascii_case("q") || s.eq_ignore_ascii_case("quit") {
        return Ok(None);
    }
    match s.parse::<usize>() {
        Ok(n) if (1..=max).contains(&n) => Ok(Some(n)),
        _ if s.is_empty() => Err(format!("Please enter a number from 1 to {max}.")),
        _ => Err(format!(
            "Invalid selection \"{s}\". Enter a number from 1 to {max}."
        )),
    }
}

fn not_found_message(needle: &str, agy_accounts: &[Account]) -> String {
    let mut msg = format!("Account \"{}\" does not exist.\n", needle.trim());
    if agy_accounts.is_empty() {
        msg.push_str("\nNo Antigravity accounts are configured.");
    } else {
        msg.push_str("\nAvailable accounts:");
        for a in agy_accounts {
            msg.push_str(&format!("\n- {}", a.label));
        }
    }
    msg.push_str("\n\nTo add a new account, run:\n  agy account add");
    msg
}

fn describe_with_id(a: &Account) -> String {
    format!(
        "{}   Account ID: {}   (added {})",
        a.label,
        a.id,
        a.created_at.format("%Y-%m-%d")
    )
}

/// Handle explicit account invocation: `agy "Personal Google"` or `agy <ACCOUNT-ID>`.
async fn handle_explicit_account(
    client: &DaemonClient,
    all_accounts: &[Account],
    agy_accounts: &[Account],
    target: &str,
) -> Result<Option<Account>> {
    let account = match resolve_account(agy_accounts, target) {
        AccountMatch::One(a) => a.clone(),
        AccountMatch::Many(matches) => {
            let owned: Vec<Account> = matches.into_iter().cloned().collect();
            let title = format!(
                "Multiple Antigravity accounts match \"{}\". Choose one explicitly:",
                target.trim()
            );
            let lines: Vec<String> = owned.iter().map(describe_with_id).collect();
            match choose(&title, &lines, false).await? {
                Choice::Selected(i) => owned[i].clone(),
                Choice::Add | Choice::Cancelled => {
                    println!("Cancelled. You can also start a specific account with: agy <ACCOUNT-ID>");
                    return Ok(None);
                }
                Choice::Unavailable => anyhow::bail!(
                    "Multiple Antigravity accounts match \"{}\".\n\n{}\n\nStart one explicitly with: agy <ACCOUNT-ID>",
                    target.trim(),
                    lines.join("\n")
                ),
            }
        }
        AccountMatch::NotFound => {
            if let AccountMatch::One(other) = resolve_account(all_accounts, target) {
                anyhow::bail!(
                    "\"{}\" cannot be used with Antigravity.\n\nProvider: {}\nSupported agents: {}",
                    other.label,
                    other.provider,
                    other.agent_types.join(", ")
                );
            }
            anyhow::bail!("{}", not_found_message(target, agy_accounts));
        }
    };

    validate_account_availability(client, agy_accounts, &account).await
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush()?;
    Ok(matches!(
        auth::read_line()?
            .as_deref()
            .map(str::to_lowercase)
            .as_deref(),
        Some("y" | "yes")
    ))
}

/// Validate availability for an explicitly chosen account without silent fallback.
/// Choosing a different account always requires an explicit user action.
async fn validate_account_availability(
    client: &DaemonClient,
    agy_accounts: &[Account],
    account: &Account,
) -> Result<Option<Account>> {
    let reason = if matches!(
        account.state,
        AccountState::RateLimited | AccountState::Cooldown
    ) {
        let remaining = account
            .cooldown_until
            .map(|t| (t - Utc::now()).num_seconds().max(1))
            .unwrap_or(30);
        Some(format!("Rate limited for another {remaining} seconds."))
    } else if !account.state.is_available() {
        Some(format!("Account is in state '{}'.", account.state))
    } else if account.active_session_count >= account.concurrency_cap {
        Some(format!(
            "Already running its maximum number of Antigravity sessions ({}/{}).",
            account.active_session_count, account.concurrency_cap
        ))
    } else {
        None
    };

    let Some(reason) = reason else {
        return Ok(Some(account.clone()));
    };
    eprintln!(
        "\nAccount \"{}\" is currently unavailable.\n\nReason:\n{}\n",
        account.label, reason
    );
    let others: Vec<Account> = agy_accounts
        .iter()
        .filter(|a| a.id != account.id)
        .cloned()
        .collect();
    if others.is_empty() || !confirm("Choose a different account?")? {
        return Ok(None);
    }
    select_interactive_account(client, &others).await
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
        println!("  1. Sign in / add an account");
        println!("  2. Cancel");

        return match auth::prompt_choice(2)? {
            Some(1) => {
                let aid = auth::run_add_account_flow(client, None, false).await?;
                let accounts = client.list_accounts().await?;
                Ok(accounts.into_iter().find(|a| a.id == aid))
            }
            _ => {
                println!("Cancelled.");
                Ok(None)
            }
        };
    }

    // Exactly 1 account and it is usable
    if agy_accounts.len() == 1 && agy_accounts[0].can_accept_session() {
        println!("Using account: {}", agy_accounts[0].label);
        return Ok(Some(agy_accounts[0].clone()));
    }

    // Multiple accounts: show interactive account selector
    select_interactive_account(client, agy_accounts).await
}

// ── Selector ──────────────────────────────────────────────────────────────────

/// Outcome of a selection prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Selected(usize),
    Add,
    Cancelled,
    /// No interactive input available (stdin is not a terminal and EOF).
    Unavailable,
}

/// Enables raw mode and hides the cursor; restores both on drop — including on
/// errors, early returns and panics.
pub struct RawModeGuard(());

impl RawModeGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let guard = RawModeGuard(());
        let _ = queue!(stdout(), Hide).and_then(|_| stdout().flush());
        Ok(guard)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = queue!(stdout(), Show).and_then(|_| stdout().flush());
        let _ = disable_raw_mode();
    }
}

/// Pure selector state machine + renderer (unit-testable, no terminal needed).
pub struct Selector {
    title: String,
    items: Vec<String>,
    allow_add: bool,
    pub selected: usize,
    rendered_lines: u16,
}

impl Selector {
    pub fn new(title: impl Into<String>, items: Vec<String>, allow_add: bool) -> Self {
        Self {
            title: title.into(),
            items,
            allow_add,
            selected: 0,
            rendered_lines: 0,
        }
    }

    /// Apply a key press. Returns `Some` once the user has decided.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Choice> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') | KeyCode::Char('d') if ctrl => Some(Choice::Cancelled),
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < self.items.len() {
                    self.selected += 1;
                }
                None
            }
            KeyCode::Enter if !self.items.is_empty() => Some(Choice::Selected(self.selected)),
            KeyCode::Char('a') | KeyCode::Char('A') if self.allow_add => Some(Choice::Add),
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Some(Choice::Cancelled),
            _ => None,
        }
    }

    /// Redraw in place: move back over the previous frame, clear it, and print
    /// each line terminated with `\r\n` (required in raw mode).
    pub fn render<W: Write>(&mut self, w: &mut W) -> io::Result<()> {
        if self.rendered_lines > 0 {
            queue!(w, MoveToPreviousLine(self.rendered_lines))?;
        }
        queue!(w, Clear(ClearType::FromCursorDown))?;
        let mut lines = vec![self.title.clone(), String::new()];
        for (i, item) in self.items.iter().enumerate() {
            lines.push(if i == self.selected {
                format!("\x1b[1;36m> {item}\x1b[0m")
            } else {
                format!("  {item}")
            });
        }
        lines.push(String::new());
        let help = if self.allow_add {
            "[↑/↓] Move   [Enter] Select   [A] Add Account   [Q/Esc] Cancel"
        } else {
            "[↑/↓] Move   [Enter] Select   [Q/Esc] Cancel"
        };
        lines.push(format!("\x1b[90m{help}\x1b[0m"));
        for l in &lines {
            write!(w, "{l}\r\n")?;
        }
        w.flush()?;
        self.rendered_lines = lines.len() as u16;
        Ok(())
    }
}

/// Ask the user to pick one item. Uses an arrow-key selector on a TTY and a
/// numbered prompt otherwise. Never returns a default choice.
pub async fn choose(title: &str, items: &[String], allow_add: bool) -> Result<Choice> {
    if items.is_empty() {
        return Ok(Choice::Cancelled);
    }
    if !(stdout().is_tty() && io::stdin().is_tty()) {
        println!("\n{title}\n");
        for (i, item) in items.iter().enumerate() {
            println!("  {}. {item}", i + 1);
        }
        return Ok(match auth::prompt_choice(items.len())? {
            Some(n) => Choice::Selected(n - 1),
            None if io::stdin().is_tty() => Choice::Cancelled,
            None => Choice::Unavailable,
        });
    }

    let mut selector = Selector::new(title, items.to_vec(), allow_add);
    let _guard = RawModeGuard::enter()?;
    let mut out = stdout();
    selector.render(&mut out)?;
    loop {
        if let Event::Key(key) = event::read()? {
            if let Some(choice) = selector.handle_key(key) {
                return Ok(choice);
            }
            selector.render(&mut out)?;
        }
    }
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
    let lines: Vec<String> = accounts
        .iter()
        .map(|a| format!("{:<24} {}", a.label, format_account_status(a)))
        .collect();
    match choose("Select Antigravity account", &lines, true).await? {
        Choice::Selected(i) => {
            println!("Selected: {}", accounts[i].label);
            Ok(Some(accounts[i].clone()))
        }
        Choice::Add => {
            let aid = auth::run_add_account_flow(client, None, false).await?;
            Ok(client
                .list_accounts()
                .await?
                .into_iter()
                .find(|a| a.id == aid))
        }
        Choice::Cancelled => {
            println!("Cancelled.");
            Ok(None)
        }
        Choice::Unavailable => {
            anyhow::bail!("No account selected. Run: agy \"<account name>\" or agy <ACCOUNT-ID>")
        }
    }
}

fn format_account_status(account: &Account) -> String {
    match account.state {
        AccountState::Active => {
            if account.active_session_count >= account.concurrency_cap {
                format!(
                    "Busy ({}/{})",
                    account.active_session_count, account.concurrency_cap
                )
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

// ── Account removal ───────────────────────────────────────────────────────────

/// `ac account remove <ACCOUNT>` / `agy account remove <ACCOUNT>`.
/// Accepts an exact account ID or an unambiguous exact name.
pub async fn remove_account_command(
    client: &DaemonClient,
    needle: &str,
    assume_yes: bool,
) -> Result<()> {
    let accounts = client.list_accounts().await?;
    let account = match resolve_account(&accounts, needle) {
        AccountMatch::One(a) => a.clone(),
        AccountMatch::Many(m) => anyhow::bail!(
            "Multiple accounts match \"{}\".\n\n{}\n\nPlease select an exact account ID.",
            needle.trim(),
            m.iter()
                .map(|a| format!("- {}", describe_with_id(a)))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        AccountMatch::NotFound => {
            let mut msg = format!("Account \"{}\" does not exist.", needle.trim());
            if !accounts.is_empty() {
                msg.push_str("\n\nAvailable accounts:");
                for a in &accounts {
                    msg.push_str(&format!("\n- {} ({})", a.label, a.id));
                }
            }
            anyhow::bail!(msg);
        }
    };

    if !assume_yes {
        if !io::stdin().is_tty() {
            anyhow::bail!(
                "Refusing to remove \"{}\" without confirmation. Re-run with --yes.",
                account.label
            );
        }
        println!("Remove account \"{}\" (ID: {})?", account.label, account.id);
        if is_antigravity_account(&account) {
            println!("Saved Antigravity credentials and the account profile will be deleted.");
        }
        println!("The account cannot be used for new sessions.");
        if !confirm("Remove?")? {
            println!("Cancelled. Nothing was removed.");
            return Ok(());
        }
    }

    let cleanup_errors = client.remove_account(&account.id).await?;
    if cleanup_errors.is_empty() {
        println!("✓ Account \"{}\" removed", account.label);
        return Ok(());
    }
    anyhow::bail!(
        "Account \"{}\" removed from Agent Control, but cleanup failed:\n\n{}\n\nPlease delete the path(s) above to finish cleanup.",
        account.label,
        cleanup_errors.join("\n")
    )
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
