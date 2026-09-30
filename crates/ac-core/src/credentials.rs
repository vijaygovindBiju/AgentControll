//! Secure Local Credential Management for Provider Accounts.
//!
//! Stores credentials securely on disk with restricted permissions (0700/0600)
//! under ~/.config/agentcontrol/credentials/ so that secrets are never stored
//! in SQLite databases, logs, or UI history.
//!
//! Antigravity (`agy`) credentials are handled exclusively by
//! [`crate::agy_auth`], which owns the typed credential format, validation,
//! browser login and per-account profiles.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

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
    crate::agy_auth::credentials_dir()
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

/// Save a generic (non-Antigravity) provider credential.
pub fn save_credential(
    provider: &str,
    cred_id: &str,
    label: &str,
    auth_mode: &str,
    token: Option<String>,
) -> Result<String> {
    if matches!(provider, "agy" | "antigravity") {
        anyhow::bail!("Antigravity credentials must be saved through agy_auth");
    }
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
