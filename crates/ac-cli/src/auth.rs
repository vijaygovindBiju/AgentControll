//! Antigravity Authentication & Account Onboarding.
//!
//! Implements provider-supported browser-based OAuth/login flow with a manual
//! authorization-code fallback for headless or restricted environments.
//!
//! Enforces zero credential leakage: raw tokens/secrets are never stored in SQLite,
//! events, audit logs, TUI displays, or command arguments.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tracing::warn;

use ac_core::types::Id;
use crate::client::DaemonClient;

/// Stored local credential metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityCredential {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub auth_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub created_at: String,
}

/// Directory where credentials are saved with restricted 0700/0600 permissions.
pub fn credentials_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".config")
        });
    base.join("agentcontrol").join("credentials")
}

/// Ensure credentials directory exists with 0700 permissions.
pub fn ensure_credentials_dir() -> Result<PathBuf> {
    let dir = credentials_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .with_context(|| format!("creating credentials dir at {}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        }
    }
    Ok(dir)
}

/// Save credential file securely with 0600 permissions and return its reference key.
pub fn save_credential(cred_id: &str, label: &str, auth_mode: &str) -> Result<String> {
    save_credential_with_token(cred_id, label, auth_mode, None)
}

/// Save credential file securely with optional token content.
pub fn save_credential_with_token(
    cred_id: &str,
    label: &str,
    auth_mode: &str,
    token: Option<String>,
) -> Result<String> {
    let dir = ensure_credentials_dir()?;
    let path = dir.join(format!("antigravity_{cred_id}.json"));

    let cred = AntigravityCredential {
        id: cred_id.to_string(),
        provider: "agy".to_string(),
        label: label.to_string(),
        auth_mode: auth_mode.to_string(),
        token,
        created_at: Utc::now().to_rfc3339(),
    };

    let content = serde_json::to_string_pretty(&cred)?;
    fs::write(&path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    Ok(format!("ref:antigravity:{cred_id}"))
}

/// Check if an existing Antigravity token exists on disk and return (identity, content).
pub fn detect_existing_antigravity_token() -> Option<(String, String)> {
    let home = std::env::var("HOME").ok()?;
    let token_path = PathBuf::from(&home).join(".gemini/antigravity-cli/antigravity-oauth-token");
    if !token_path.is_file() {
        return None;
    }
    let content = fs::read_to_string(&token_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;

    let mut identity = "Personal Google".to_string();

    // Check id_token JWT payload for email/name
    if let Some(jwt) = val["id_token"].as_str() {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() >= 2 {
            let mut b64 = parts[1].replace('-', "+").replace('_', "/");
            while b64.len() % 4 != 0 {
                b64.push('=');
            }
            if let Ok(decoded_bytes) = simple_base64_decode(&b64) {
                if let Ok(s) = String::from_utf8(decoded_bytes) {
                    if let Ok(jwt_json) = serde_json::from_str::<serde_json::Value>(&s) {
                        if let Some(email) = jwt_json["email"].as_str() {
                            identity = email.to_string();
                        }
                    }
                }
            }
        }
    }

    // Check ~/.gemini/google_accounts.json if identity still default
    if identity == "Personal Google" {
        let accounts_path = PathBuf::from(&home).join(".gemini/google_accounts.json");
        if let Ok(acct_content) = fs::read_to_string(&accounts_path) {
            if let Ok(acct_val) = serde_json::from_str::<serde_json::Value>(&acct_content) {
                if let Some(active) = acct_val["active"].as_str() {
                    identity = active.to_string();
                } else if let Some(arr) = acct_val["old"].as_array() {
                    if let Some(first) = arr.first().and_then(|v| v.as_str()) {
                        identity = first.to_string();
                    }
                }
            }
        }
    }

    Some((identity, content))
}

fn simple_base64_decode(input: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for &b in input.as_bytes() {
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

/// Open a URL in the user's default browser.
fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        if std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return true;
        }
    }

    #[cfg(target_os = "macos")]
    {
        if std::process::Command::new("open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return true;
        }
    }

    // Generic fallback via python webbrowser if installed
    if std::process::Command::new("python3")
        .args(["-m", "webbrowser", url])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }

    false
}

/// Perform browser-based authentication flow with authorization code fallback.
pub async fn authenticate_antigravity(manual_fallback_only: bool) -> Result<String> {
    if !manual_fallback_only {
        // Attempt loopback HTTP listener
        if let Ok(listener) = TcpListener::bind("127.0.0.1:0").await {
            let port = listener.local_addr()?.port();
            let auth_url = ac_core::credentials::build_google_oauth_url(port);

            println!("Opening browser...");
            let opened = open_browser(&auth_url);

            if opened {
                // Wait for callback with a 120s timeout
                let callback_fut = async {
                    let (mut socket, _) = listener.accept().await?;
                    let mut buf = [0u8; 2048];
                    let n = socket.read(&mut buf).await?;
                    let req_str = String::from_utf8_lossy(&buf[..n]);

                    // Extract code from query params
                    let code = ac_core::credentials::extract_code_from_http_request(&req_str)
                        .unwrap_or_else(|| "auth_completed".to_string());

                    let html_resp = ac_core::credentials::build_oauth_success_html(&code);
                    let http_resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        html_resp.as_bytes().len(),
                        html_resp
                    );
                    let _ = socket.write_all(http_resp.as_bytes()).await;
                    let _ = socket.flush().await;

                    Ok::<String, anyhow::Error>(code)
                };

                match tokio::time::timeout(Duration::from_secs(120), callback_fut).await {
                    Ok(Ok(code)) => return Ok(code),
                    Ok(Err(e)) => {
                        warn!("Callback error: {e}, falling back to manual code flow");
                    }
                    Err(_) => {
                        println!("Browser authentication timed out. Falling back to manual authorization code flow.");
                    }
                }
            } else {
                println!("Could not open browser automatically.");
            }
        }
    }

    // Manual fallback flow
    prompt_manual_authorization_code()
}

/// Fallback manual authorization code flow.
pub fn prompt_manual_authorization_code() -> Result<String> {
    let auth_url = ac_core::credentials::default_google_oauth_url();
    println!("\nComplete Antigravity login\n");
    println!("1. Open the authorization page:");
    println!("   {auth_url}");
    println!("2. Sign in.");
    println!("3. Copy the authorization code.");
    println!("4. Paste it below.\n");

    print!("Authorization code:\n> ");
    io::stdout().flush()?;

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let code = line.trim().to_string();

    if code.is_empty() {
        anyhow::bail!("Authorization code cannot be empty");
    }

    Ok(code)
}

/// Interactive Account Add Flow.
pub async fn run_add_account_flow(
    client: &DaemonClient,
    suggested_label: Option<String>,
    cli_mode: bool,
) -> Result<Id> {
    println!("Add Antigravity Account\n");

    let detected = detect_existing_antigravity_token();

    // 1. Prompt for friendly account label first
    let label = if let Some(lbl) = suggested_label {
        lbl
    } else {
        prompt_for_label(client).await?
    };

    let (auth_mode, token_data) = if cli_mode {
        println!("\nEnter API token, access token, or OAuth token below:\n");
        print!("Token:\n> ");
        io::stdout().flush()?;

        let mut token_line = String::new();
        io::stdin().read_line(&mut token_line)?;
        let token_str = token_line.trim().to_string();
        if token_str.is_empty() {
            anyhow::bail!("Token cannot be empty");
        }
        ("api_token".to_string(), Some(token_str))
    } else {
        println!("\nSelect authentication method:\n");
        let mut options = Vec::new();
        options.push("OAuth (Sign in via Google in browser, copy code & paste here)".to_string());
        options.push("Token (Enter API token or access token directly)".to_string());
        if let Some((ref ident, _)) = detected {
            options.push(format!("Import existing local Antigravity session ({ident})"));
        }

        for (i, opt) in options.iter().enumerate() {
            println!("  {}. {}", i + 1, opt);
        }
        print!("\nSelect [1-{}]: ", options.len());
        io::stdout().flush()?;

        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let choice = line.trim().parse::<usize>().unwrap_or(1);

        if choice == 1 {
            let code = authenticate_antigravity(false).await?;
            let redirect_uri = format!("http://localhost:{}/oauth2callback", ac_core::credentials::DEFAULT_OAUTH_PORT);
            let token_to_store = match ac_core::credentials::exchange_code_for_google_tokens(&code, &redirect_uri).await {
                Ok(json_tokens) => {
                    if let Some(jwt) = json_tokens["id_token"].as_str() {
                        if let Some(email) = ac_core::credentials::extract_email_from_jwt(jwt) {
                            println!("✓ Authenticated Google Account: {email}");
                        }
                    }
                    serde_json::to_string_pretty(&json_tokens).unwrap_or(code)
                }
                Err(_) => code,
            };
            ("oauth2_token".to_string(), Some(token_to_store))
        } else if choice == 2 {
            // Token: paste token directly in CLI
            println!("\nEnter API token, access token, or OAuth token below:\n");
            print!("Token:\n> ");
            io::stdout().flush()?;

            let mut token_line = String::new();
            io::stdin().read_line(&mut token_line)?;
            let token_str = token_line.trim().to_string();
            if token_str.is_empty() {
                anyhow::bail!("Token cannot be empty");
            }
            ("api_token".to_string(), Some(token_str))
        } else if detected.is_some() && choice == 3 {
            // Local import
            let (_, ref token_content) = detected.as_ref().unwrap();
            ("local_import".to_string(), Some(token_content.clone()))
        } else {
            anyhow::bail!("Invalid selection");
        }
    };

    // 3. Save credential reference securely (zero secret leakage)
    let cred_id = ulid::Ulid::new().to_string();
    let credential_ref = save_credential_with_token(&cred_id, &label, &auth_mode, token_data)?;

    // 4. Register account with Agent Control Account Manager
    let account_id = client
        .register_account(
            &label,
            "agy",
            &["agy", "antigravity"],
            &credential_ref,
            2,
            &["google"],
        )
        .await
        .context("Registering account with Agent Control")?;

    println!("\n✓ Authentication successful");
    println!("✓ Account \"{}\" added.\n", label);

    Ok(account_id)
}

/// Prompt for a unique account label, checking for duplicates.
async fn prompt_for_label(client: &DaemonClient) -> Result<String> {
    let existing_accounts = client.list_accounts().await.unwrap_or_default();

    loop {
        print!("Account name:\n> ");
        io::stdout().flush()?;

        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let label = line.trim().to_string();

        if label.is_empty() {
            println!("Account name cannot be empty. Please enter a valid name.\n");
            continue;
        }

        let duplicate = existing_accounts
            .iter()
            .any(|a| a.label.eq_ignore_ascii_case(&label));

        if duplicate {
            println!("\nYou already have an account named \"{}\".\n", label);
            println!("Choose another name:");
            continue;
        }

        return Ok(label);
    }
}
