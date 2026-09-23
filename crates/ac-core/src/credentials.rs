//! Secure Local Credential Management for Provider Accounts.
//!
//! Stores credentials securely on disk with restricted permissions (0700/0600)
//! under ~/.config/agentcontrol/credentials/ so that secrets are never stored
//! in SQLite databases, logs, or UI history.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// Official Google Antigravity OAuth Client ID
pub const ANTIGRAVITY_CLIENT_ID: &str =
    "mock-client-id.apps.googleusercontent.com";

/// Official Google Antigravity OAuth Client Secret
pub const ANTIGRAVITY_CLIENT_SECRET: &str =
    "mock-client-secret";

pub const DEFAULT_OAUTH_PORT: u16 = 54321;

/// Build the Google OAuth 2.0 authorization URL for a specific redirect port.
pub fn build_google_oauth_url(port: u16) -> String {
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={ANTIGRAVITY_CLIENT_ID}&redirect_uri=http%3A%2F%2Flocalhost%3A{port}%2Foauth2callback&response_type=code&scope=openid%20email%20profile%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform&access_type=offline&prompt=consent"
    )
}

/// Default Google OAuth authorization URL using default port 54321.
pub fn default_google_oauth_url() -> String {
    build_google_oauth_url(DEFAULT_OAUTH_PORT)
}

pub const GOOGLE_OAUTH_URL: &str =
    "https://accounts.google.com/o/oauth2/v2/auth?client_id=mock-client-id.apps.googleusercontent.com&redirect_uri=http%3A%2F%2Flocalhost%3A54321%2Foauth2callback&response_type=code&scope=openid%20email%20profile%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform&access_type=offline&prompt=consent";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountCredential {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub auth_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub created_at: String,
}

pub fn credentials_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".config")
        });
    base.join("agentcontrol").join("credentials")
}

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

pub fn save_credential(
    provider: &str,
    cred_id: &str,
    label: &str,
    auth_mode: &str,
    token: Option<String>,
) -> Result<String> {
    let dir = ensure_credentials_dir()?;
    let path = dir.join(format!("{provider}_{cred_id}.json"));

    let cred = AccountCredential {
        id: cred_id.to_string(),
        provider: provider.to_string(),
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

    Ok(format!("ref:{provider}:{cred_id}"))
}

/// Exchange an authorization code with Google OAuth endpoint for real tokens.
pub async fn exchange_code_for_google_tokens(code: &str, redirect_uri: &str) -> Result<serde_json::Value> {
    let output = tokio::process::Command::new("curl")
        .args([
            "-s",
            "-X", "POST",
            "https://oauth2.googleapis.com/token",
            "-d", &format!("client_id={ANTIGRAVITY_CLIENT_ID}"),
            "-d", &format!("client_secret={ANTIGRAVITY_CLIENT_SECRET}"),
            "-d", &format!("code={code}"),
            "-d", "grant_type=authorization_code",
            "-d", &format!("redirect_uri={redirect_uri}"),
        ])
        .output()
        .await
        .context("executing curl to exchange Google OAuth code")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let val: serde_json::Value = serde_json::from_str(&stdout)
        .with_context(|| format!("parsing Google OAuth response: {stdout}"))?;

    if let Some(err) = val.get("error") {
        let desc = val.get("error_description").and_then(|d| d.as_str()).unwrap_or("");
        anyhow::bail!("Google OAuth token exchange failed ({err}): {desc}");
    }

    Ok(val)
}

/// Extract user email from Google JWT id_token without external dependencies.
pub fn extract_email_from_jwt(jwt: &str) -> Option<String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let mut b64 = parts[1].replace('-', "+").replace('_', "/");
    while b64.len() % 4 != 0 {
        b64.push('=');
    }
    let decoded = simple_base64_decode(&b64).ok()?;
    let s = String::from_utf8(decoded).ok()?;
    let val: serde_json::Value = serde_json::from_str(&s).ok()?;
    val["email"].as_str().map(|s| s.to_string())
}

pub fn detect_existing_antigravity_token() -> Option<(String, String)> {
    let home = std::env::var("HOME").ok()?;
    let token_path = PathBuf::from(&home).join(".gemini/antigravity-cli/antigravity-oauth-token");
    if !token_path.is_file() {
        return None;
    }
    let content = fs::read_to_string(&token_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;

    let mut identity = "Personal Google".to_string();

    if let Some(jwt) = val["id_token"].as_str() {
        if let Some(email) = extract_email_from_jwt(jwt) {
            identity = email;
        }
    }

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

pub fn open_browser(url: &str) -> bool {
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

/// Extract authorization code from an incoming HTTP GET request string.
pub fn extract_code_from_http_request(req_str: &str) -> Option<String> {
    let pos = req_str.find("code=")?;
    let query = &req_str[pos + 5..];
    let end = query.find('&').or_else(|| query.find(' ')).unwrap_or(query.len());
    let code = query[..end].trim();
    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

/// Build rich HTML page displaying the authorization code, copy button, and instructions.
pub fn build_oauth_success_html(code: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Google Antigravity Authentication</title>
  <style>
    body {{
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif;
      background-color: #f8f9fa;
      color: #202124;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      min-height: 85vh;
      margin: 0;
      padding: 20px;
    }}
    .card {{
      background: #ffffff;
      border: 1px solid #dadce0;
      border-radius: 12px;
      box-shadow: 0 4px 16px rgba(0,0,0,0.08);
      max-width: 580px;
      width: 100%;
      padding: 36px 32px;
      text-align: center;
    }}
    .icon {{
      font-size: 48px;
      color: #1a73e8;
      margin-bottom: 12px;
    }}
    h2 {{
      margin: 0 0 12px;
      font-size: 24px;
      font-weight: 600;
      color: #1a73e8;
    }}
    p {{
      color: #5f6368;
      font-size: 15px;
      line-height: 1.5;
      margin: 0 0 20px;
    }}
    .code-box {{
      background: #f1f3f4;
      border: 1px solid #c6c6c6;
      border-radius: 8px;
      padding: 14px;
      font-family: 'Roboto Mono', monospace;
      font-size: 14px;
      word-break: break-all;
      color: #1e293b;
      margin-bottom: 16px;
      user-select: all;
    }}
    .btn {{
      background-color: #1a73e8;
      color: #ffffff;
      border: none;
      padding: 10px 24px;
      font-size: 14px;
      font-weight: 500;
      border-radius: 6px;
      cursor: pointer;
      transition: background 0.2s;
    }}
    .btn:hover {{
      background-color: #1557b0;
    }}
    .status {{
      margin-top: 14px;
      font-size: 13px;
      color: #188038;
      font-weight: 500;
    }}
  </style>
</head>
<body>
  <div class="card">
    <div class="icon">&#10003;</div>
    <h2>Authentication Successful!</h2>
    <p>Your Google authorization code has been received. If not automatically linked, copy the code below and paste it into Agent Control:</p>
    <div class="code-box" id="authCode">{}</div>
    <button class="btn" onclick="copyCode()">Copy Code to Clipboard</button>
    <div class="status" id="copiedStatus"></div>
  </div>
  <script>
    function copyCode() {{
      const code = document.getElementById('authCode').innerText.trim();
      navigator.clipboard.writeText(code).then(() => {{
        document.getElementById('copiedStatus').innerText = '&#10003; Copied to clipboard!';
      }});
    }}
    try {{
      const code = document.getElementById('authCode').innerText.trim();
      navigator.clipboard.writeText(code);
    }} catch(e) {{}}
  </script>
</body>
</html>"#,
        code
    )
}

