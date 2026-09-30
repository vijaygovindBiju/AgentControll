//! Antigravity account onboarding (CLI front end).
//!
//! All authentication, validation and storage logic lives in
//! [`ac_core::agy_auth`] and is shared with the TUI. This module only handles
//! terminal prompts. An account is registered only after a real, validated
//! credential has been saved; a failed login never creates an account.

use anyhow::{Context, Result};
use std::io::{self, BufRead, Write};

use ac_core::{agy_auth, types::Id};
use crate::client::DaemonClient;

/// Read one line from stdin. Returns `None` on EOF.
pub fn read_line() -> Result<Option<String>> {
    let mut line = String::new();
    let n = io::stdin().lock().read_line(&mut line)?;
    Ok((n > 0).then(|| line.trim().to_string()))
}

/// One browser login attempt using the shared service. On success the
/// credential has been validated and saved; returns `(credential_ref, token)`.
///
/// `paste_mode` (`--cli`) skips opening a browser: the user opens the URL on
/// any machine and pastes the final redirect address back.
pub async fn browser_login(label: &str, paste_mode: bool) -> std::result::Result<(String, agy_auth::AgyToken), agy_auth::AgyAuthError> {
    let login = agy_auth::begin_browser_login().await?;
    if paste_mode {
        println!("\nOpen this URL in a browser and sign in:\n\n  {}\n", login.authorization_url());
        print!("Then paste the full address from the browser's address bar:\n> ");
        let _ = io::stdout().flush();
        let pasted = read_line().ok().flatten().unwrap_or_default();
        let code = login.code_from_pasted(&pasted)?;
        let token = login.exchange(&code).await?;
        let cref = agy_auth::save_new_credential(label, &token, "browser_login")?;
        return Ok((cref, token));
    }
    if login.open_in_browser() {
        println!("Opening your default browser for Google sign-in...");
    } else {
        println!("Unable to open your default browser.\n\nOpen this URL manually:\n  {}\n", login.authorization_url());
    }
    println!("Waiting for browser authentication (up to {} minutes; Ctrl+C to cancel)...", agy_auth::LOGIN_TIMEOUT.as_secs() / 60);
    agy_auth::finish_browser_login(&login, label, agy_auth::LOGIN_TIMEOUT).await
}

/// Interactive "add Antigravity account" flow. Returns the new account ID.
pub async fn run_add_account_flow(
    client: &DaemonClient,
    suggested_label: Option<String>,
    paste_mode: bool,
) -> Result<Id> {
    println!("Add Antigravity Account\n");

    let label = match suggested_label {
        Some(l) if !l.trim().is_empty() => {
            ensure_label_unique(client, l.trim()).await?;
            l.trim().to_string()
        }
        _ => prompt_for_label(client).await?,
    };

    let detected = agy_auth::detect_local_login();
    println!("\nLogin to Antigravity:\n");
    println!("  1. Login with Browser (recommended)");
    if let Some((ident, _)) = &detected {
        println!("  2. Import existing agy login ({ident})");
    }
    let max = if detected.is_some() { 2 } else { 1 };
    let choice = prompt_choice(max)?.context("Cancelled. The account was not added.")?;

    let (credential_ref, token) = if choice == 1 {
        loop {
            match browser_login(&label, paste_mode).await {
                Ok(r) => break r,
                Err(e) => {
                    eprintln!("\n{e}\n");
                    print!("[r] Retry   [Enter] Cancel: ");
                    io::stdout().flush()?;
                    if !matches!(read_line()?.as_deref(), Some("r" | "R")) {
                        anyhow::bail!("Cancelled. The account was not added.");
                    }
                }
            }
        }
    } else {
        let tok = detected.map(|(_, t)| t).context("No local agy login found")?;
        (agy_auth::save_new_credential(&label, &tok, "local_import")?, tok)
    };

    let account_id = match client
        .register_account(&label, agy_auth::PROVIDER, &agy_auth::AGENT_TYPES, &credential_ref, 2, &["google"])
        .await
    {
        Ok(id) => id,
        Err(e) => {
            agy_auth::discard_credential(&credential_ref);
            return Err(e.context("The account was not added"));
        }
    };

    println!("\n✓ Login successful");
    println!("  Account:  {label}");
    if let Some(email) = token.email() {
        println!("  Google:   {email}");
    }
    println!("  Provider: Antigravity");
    println!("  Status:   ● Ready\n");
    Ok(account_id)
}

/// Prompt for a numbered choice in `1..=max`. Re-prompts on invalid input;
/// returns `None` on EOF or `q`. Never defaults to an option.
pub fn prompt_choice(max: usize) -> Result<Option<usize>> {
    loop {
        print!("\nSelect [1-{max}] (q to cancel): ");
        io::stdout().flush()?;
        let Some(line) = read_line()? else { return Ok(None) };
        match crate::launcher::parse_selection(&line, max) {
            Ok(Some(n)) => return Ok(Some(n)),
            Ok(None) => return Ok(None),
            Err(msg) => println!("{msg}"),
        }
    }
}

async fn ensure_label_unique(client: &DaemonClient, label: &str) -> Result<()> {
    let existing = client.list_accounts().await?;
    if existing.iter().any(|a| a.label.eq_ignore_ascii_case(label)) {
        anyhow::bail!("You already have an account named \"{label}\". Choose another name.");
    }
    Ok(())
}

/// Prompt for a unique account label, checking for duplicates.
async fn prompt_for_label(client: &DaemonClient) -> Result<String> {
    let existing_accounts = client.list_accounts().await?;

    loop {
        print!("Account name:\n> ");
        io::stdout().flush()?;
        let label = read_line()?.context("Cancelled. Account was NOT added.")?;

        if label.is_empty() {
            println!("Account name cannot be empty. Please enter a valid name.\n");
            continue;
        }
        if existing_accounts.iter().any(|a| a.label.eq_ignore_ascii_case(&label)) {
            println!("\nYou already have an account named \"{label}\".\n\nChoose another name:");
            continue;
        }
        return Ok(label);
    }
}
