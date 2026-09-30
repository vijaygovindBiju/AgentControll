//! Antigravity (`agy`) authentication service.
//!
//! This is the single implementation used by the daemon, the CLI and the TUI for:
//!  - browser OAuth login (loopback callback + PKCE, token exchange over HTTPS),
//!  - importing an existing local `agy` login,
//!  - the canonical credential file format and legacy migration,
//!  - credential validation (never treating a login code as a token),
//!  - per-account `agy` profiles (isolated `HOME`),
//!  - write-back of tokens refreshed by `agy`,
//!  - removal of all Agent Control–owned data for an account.
//!
//! Secrets (access/refresh tokens, authorization codes, OAuth client secret)
//! never appear in error messages, logs, events or process arguments.

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::types::Id;

/// Provider identifier for Antigravity accounts.
pub const PROVIDER: &str = "agy";
/// Agent types an Antigravity account may run.
pub const AGENT_TYPES: [&str; 2] = ["agy", "antigravity"];
/// Canonical credential format version.
pub const CREDENTIAL_VERSION: u32 = 2;

const TOKEN_FILE: &str = "antigravity-oauth-token";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Scopes requested by agy's own login (verified against a token issued to agy).
const OAUTH_SCOPES: &str = "openid email profile https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/aicode https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
/// Public OAuth client identifier of the Antigravity installed application.
/// Client IDs are not secret: they appear in every authorization URL.
pub const AGY_OAUTH_CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
/// How long a browser login may take before it is abandoned.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Environment variables that could make `agy` authenticate as someone other
/// than the selected account; they are removed from the child environment.
pub const AMBIENT_AUTH_ENV: [&str; 5] = [
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_GENAI_USE_VERTEXAI",
    "GOOGLE_CLOUD_ACCESS_TOKEN",
];

// ── Errors ────────────────────────────────────────────────────────────────────

/// User-facing Antigravity authentication / credential errors. Never contains secrets.
#[derive(Debug, thiserror::Error)]
pub enum AgyAuthError {
    #[error("{}", start_failure(.label, "No Antigravity credential is saved for this account."))]
    NoCredential { label: String },
    #[error("{}", start_failure(.label, &format!("The saved credential is malformed ({}).", .detail)))]
    MalformedCredential { label: String, detail: String },
    #[error("{}", start_failure(.label, "Only a one-time login code was saved; the login never completed."))]
    AuthorizationCodeOnly { label: String },
    #[error("{}", start_failure(.label, &format!("The saved credential is not valid ({}).", .reason)))]
    InvalidCredential { label: String, reason: String },
    #[error("{}", start_failure(.label, &format!("The refreshed Antigravity credential could not be used ({}).", .reason)))]
    RefreshFailed { label: String, reason: String },
    #[error("Antigravity account \"{label}\" cannot be started.\n\nReason:\nCould not prepare the account profile at {path}: {reason}")]
    ProfileFailure { label: String, path: String, reason: String },
    #[error("Antigravity account \"{label}\" cannot be started.\n\nReason:\nThe agy process could not be launched: {reason}")]
    LaunchFailure { label: String, reason: String },
    #[error("No Antigravity account is selected for this session.\nAgent Control never falls back to the machine's default agy login.")]
    NoAccountSelected,
    #[error("Unable to start Antigravity login callback.\n\nReason:\n{0}\n\nPlease retry.")]
    CallbackUnavailable(String),
    #[error("Antigravity browser login requires OAuth configuration.\n\nReason:\n{0}\n\nInstall or update agy (Agent Control reads its login configuration from the agy installation), or import an existing agy login.")]
    OAuthNotConfigured(String),
    #[error("Antigravity login failed.\n\nThe account was not added.\n\nReason:\n{0}")]
    LoginFailed(String),
    #[error("Failed to save the Antigravity credential: {0}")]
    Storage(String),
}

fn start_failure(label: &str, reason: &str) -> String {
    format!("Antigravity account \"{label}\" cannot be started.\n\nReason:\n{reason}\n\nPlease sign in again (remove the account and add it again).")
}

pub type AuthResult<T> = std::result::Result<T, AgyAuthError>;

// ── Paths ─────────────────────────────────────────────────────────────────────

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// `~/.config/agentcontrol` (honours `XDG_CONFIG_HOME`).
pub fn agentcontrol_config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".config"))
        .join("agentcontrol")
}

pub fn credentials_dir() -> PathBuf {
    agentcontrol_config_dir().join("credentials")
}

pub fn profiles_dir() -> PathBuf {
    agentcontrol_config_dir().join("profiles")
}

/// `~/.config/agentcontrol/profiles/<account-id>/`
pub fn profile_dir(account_id: &Id) -> AuthResult<PathBuf> {
    let id = account_id.0.as_str();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(AgyAuthError::ProfileFailure {
            label: id.to_string(),
            path: profiles_dir().display().to_string(),
            reason: "invalid account id".into(),
        });
    }
    Ok(profiles_dir().join(id))
}

fn oauth_client_config_path() -> PathBuf {
    agentcontrol_config_dir().join("antigravity-oauth-client.json")
}

fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Atomically write a private (0600) file.
fn write_private(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("cred"),
        ulid::Ulid::new()
    ));
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        std::io::Write::write_all(&mut f, content)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

// ── Token material ────────────────────────────────────────────────────────────

/// A usable Antigravity OAuth credential in the exact on-disk format `agy` uses.
#[derive(Debug, Clone, PartialEq)]
pub struct AgyToken {
    native: Value,
}

impl AgyToken {
    /// Parse from `agy`'s native token-file JSON.
    pub fn from_native(v: Value) -> std::result::Result<Self, String> {
        let t = v.get("token").ok_or("missing token object")?;
        let access = t.get("access_token").and_then(Value::as_str).unwrap_or("");
        let refresh = t.get("refresh_token").and_then(Value::as_str).unwrap_or("");
        if access.is_empty() && refresh.is_empty() {
            return Err("no access or refresh token present".into());
        }
        Ok(Self { native: v })
    }

    /// Build from a Google OAuth token endpoint response.
    pub fn from_google_response(v: &Value, now: DateTime<Utc>) -> std::result::Result<Self, String> {
        let access = v.get("access_token").and_then(Value::as_str).filter(|s| !s.is_empty())
            .ok_or("response has no access token")?;
        let refresh = v.get("refresh_token").and_then(Value::as_str).filter(|s| !s.is_empty())
            .ok_or("response has no refresh token")?;
        let expires_in = v.get("expires_in").and_then(Value::as_i64).unwrap_or(0);
        Self::from_native(serde_json::json!({
            "token": {
                "access_token": access,
                "token_type": v.get("token_type").and_then(Value::as_str).unwrap_or("Bearer"),
                "refresh_token": refresh,
                "expiry": (now + chrono::Duration::seconds(expires_in)).to_rfc3339(),
            },
            "auth_method": "consumer",
            "id_token": v.get("id_token").and_then(Value::as_str).unwrap_or(""),
        }))
    }

    pub fn native(&self) -> &Value {
        &self.native
    }

    fn field(&self, k: &str) -> &str {
        self.native["token"].get(k).and_then(Value::as_str).unwrap_or("")
    }

    pub fn has_refresh_token(&self) -> bool {
        !self.field("refresh_token").is_empty()
    }

    pub fn expiry(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(self.field("expiry")).ok().map(|d| d.with_timezone(&Utc))
    }

    /// Google account e-mail from the id_token (non-secret identity).
    pub fn email(&self) -> Option<String> {
        self.native.get("id_token").and_then(Value::as_str).and_then(email_from_jwt)
    }

    fn same_secret_as(&self, other: &AgyToken) -> bool {
        self.field("access_token") == other.field("access_token")
            && self.field("refresh_token") == other.field("refresh_token")
    }

    /// Validate that the credential can be used to start `agy`.
    pub fn validate(&self, now: DateTime<Utc>) -> std::result::Result<(), String> {
        let expired = self.expiry().map(|e| e <= now).unwrap_or(true);
        if expired && !self.has_refresh_token() {
            return Err("access token expired and no refresh token is available".into());
        }
        Ok(())
    }
}

/// What a piece of raw stored material actually is.
#[derive(Debug, PartialEq)]
pub enum TokenMaterial {
    Empty,
    AuthorizationCode,
    Usable(AgyToken),
    Invalid(String),
}

/// Classify raw token material without ever assuming "non-empty ⇒ token".
pub fn classify_material(raw: &str, now: DateTime<Utc>) -> TokenMaterial {
    let raw = raw.trim();
    if raw.is_empty() {
        return TokenMaterial::Empty;
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(v) if v.get("token").is_some() => match AgyToken::from_native(v) {
            Ok(t) => TokenMaterial::Usable(t),
            Err(e) => TokenMaterial::Invalid(e),
        },
        Ok(v) if v.get("access_token").is_some() => match AgyToken::from_google_response(&v, now) {
            Ok(t) => TokenMaterial::Usable(t),
            Err(e) => TokenMaterial::Invalid(e),
        },
        Ok(_) => TokenMaterial::Invalid("unrecognised credential JSON".into()),
        Err(_) if looks_like_auth_code(raw) => TokenMaterial::AuthorizationCode,
        Err(_) => TokenMaterial::Invalid("not an Antigravity OAuth credential".into()),
    }
}

fn looks_like_auth_code(s: &str) -> bool {
    let s = percent_decode(s);
    s.starts_with("4/") || s == "auth_completed"
}

// ── Canonical credential file ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    AntigravityOauth,
}

/// Canonical (v2) Antigravity credential file: `credentials/agy_<id>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialFile {
    pub version: u32,
    pub id: String,
    pub provider: String,
    pub label: String,
    pub credential_type: CredentialType,
    /// How the credential was obtained: `browser_login` or `local_import`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// `agy`-native token file content.
    pub credential_data: Value,
}

impl CredentialFile {
    pub fn token(&self) -> std::result::Result<AgyToken, String> {
        AgyToken::from_native(self.credential_data.clone())
    }
}

/// Legacy (v1) file written by earlier CLI/TUI versions.
#[derive(Debug, Deserialize)]
struct LegacyCredentialFile {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    auth_mode: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    created_at: String,
}

/// Parse a credential reference `ref:<provider>:<id>` into the file path.
/// Accepts the canonical `agy` provider and the legacy `antigravity` provider.
pub fn credential_path(cred_ref: &str) -> Option<PathBuf> {
    let (prefix, id) = cred_ref.strip_prefix("ref:")?.split_once(':')?;
    if !matches!(prefix, "agy" | "antigravity") || id.is_empty()
        || !id.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(credentials_dir().join(format!("{prefix}_{id}.json")))
}

/// Save a new usable credential in canonical form; returns its `ref:agy:<id>`.
pub fn save_new_credential(label: &str, token: &AgyToken, source: &str) -> AuthResult<String> {
    let id = ulid::Ulid::new().to_string();
    let now = Utc::now().to_rfc3339();
    let file = CredentialFile {
        version: CREDENTIAL_VERSION,
        id: id.clone(),
        provider: PROVIDER.into(),
        label: label.into(),
        credential_type: CredentialType::AntigravityOauth,
        source: source.into(),
        email: token.email(),
        created_at: now.clone(),
        updated_at: now,
        credential_data: token.native().clone(),
    };
    ensure_private_dir(&credentials_dir()).map_err(|e| AgyAuthError::Storage(e.to_string()))?;
    let path = credentials_dir().join(format!("{PROVIDER}_{id}.json"));
    write_credential_file(&path, &file)?;
    Ok(format!("ref:{PROVIDER}:{id}"))
}

fn write_credential_file(path: &Path, file: &CredentialFile) -> AuthResult<()> {
    let body = serde_json::to_vec_pretty(file).map_err(|e| AgyAuthError::Storage(e.to_string()))?;
    write_private(path, &body).map_err(|e| AgyAuthError::Storage(e.to_string()))
}

/// Load and validate a credential. Legacy files holding a usable token are
/// migrated in place to the canonical format; unusable ones are reported.
pub fn load_credential(cred_ref: &str, label_hint: &str) -> AuthResult<(PathBuf, CredentialFile)> {
    let label = label_hint.to_string();
    let path = credential_path(cred_ref).ok_or_else(|| AgyAuthError::MalformedCredential {
        label: label.clone(),
        detail: "unrecognised credential reference".into(),
    })?;
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AgyAuthError::NoCredential { label })
        }
        Err(e) => {
            return Err(AgyAuthError::MalformedCredential { label, detail: format!("unreadable: {}", e.kind()) })
        }
    };
    let raw: Value = serde_json::from_str(&content).map_err(|_| AgyAuthError::MalformedCredential {
        label: label.clone(),
        detail: "file is not valid JSON".into(),
    })?;

    if raw.get("version").and_then(Value::as_u64) == Some(CREDENTIAL_VERSION as u64) {
        let file: CredentialFile = serde_json::from_value(raw).map_err(|_| AgyAuthError::MalformedCredential {
            label: label.clone(),
            detail: "unexpected credential file layout".into(),
        })?;
        let lbl = if file.label.is_empty() { label } else { file.label.clone() };
        let tok = file.token().map_err(|reason| AgyAuthError::InvalidCredential { label: lbl.clone(), reason })?;
        tok.validate(Utc::now()).map_err(|reason| AgyAuthError::InvalidCredential { label: lbl, reason })?;
        return Ok((path, file));
    }

    // Legacy migration.
    let legacy: LegacyCredentialFile = serde_json::from_value(raw).map_err(|_| AgyAuthError::MalformedCredential {
        label: label.clone(),
        detail: "unexpected legacy credential layout".into(),
    })?;
    let lbl = if legacy.label.is_empty() { label } else { legacy.label.clone() };
    let tok = match classify_material(legacy.token.as_deref().unwrap_or(""), Utc::now()) {
        TokenMaterial::Empty => return Err(AgyAuthError::NoCredential { label: lbl }),
        TokenMaterial::AuthorizationCode => return Err(AgyAuthError::AuthorizationCodeOnly { label: lbl }),
        TokenMaterial::Invalid(reason) => return Err(AgyAuthError::InvalidCredential { label: lbl, reason }),
        TokenMaterial::Usable(t) => t,
    };
    tok.validate(Utc::now()).map_err(|reason| AgyAuthError::InvalidCredential { label: lbl.clone(), reason })?;
    let now = Utc::now().to_rfc3339();
    let file = CredentialFile {
        version: CREDENTIAL_VERSION,
        id: legacy.id,
        provider: PROVIDER.into(),
        label: lbl,
        credential_type: CredentialType::AntigravityOauth,
        source: if legacy.auth_mode.contains("import") { "local_import".into() } else { "browser_login".into() },
        email: tok.email(),
        created_at: if legacy.created_at.is_empty() { now.clone() } else { legacy.created_at },
        updated_at: now,
        credential_data: tok.native().clone(),
    };
    write_credential_file(&path, &file)?;
    tracing::info!("Migrated Antigravity credential {} to format v{}", file.id, CREDENTIAL_VERSION);
    Ok((path, file))
}

/// Validate a credential reference without touching any profile.
pub fn validate_credential(cred_ref: &str, label_hint: &str) -> AuthResult<CredentialFile> {
    load_credential(cred_ref, label_hint).map(|(_, f)| f)
}

// ── Profiles ──────────────────────────────────────────────────────────────────

/// Entries of `~/.gemini` shared (symlinked) into every profile. Everything
/// else — tokens, account lists, caches, conversation state — stays per-profile.
const SHARED_GEMINI: [&str; 4] = ["settings.json", "trustedFolders.json", "config", "installation_id"];
const SHARED_AGY_CLI: [&str; 6] = ["bin", "builtin", "settings.json", "keybindings.json", "updater", "installation_id"];

#[cfg(unix)]
fn link_entries(src: &Path, dst: &Path, filter: impl Fn(&str) -> bool) {
    let Ok(entries) = fs::read_dir(src) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(n) = name.to_str() else { continue };
        if !filter(n) {
            continue;
        }
        let target = dst.join(&name);
        if target.symlink_metadata().is_err() {
            let _ = std::os::unix::fs::symlink(entry.path(), &target);
        }
    }
}

fn read_profile_token(path: &Path) -> Option<std::result::Result<AgyToken, String>> {
    let content = fs::read_to_string(path).ok()?;
    Some(
        serde_json::from_str::<Value>(&content)
            .map_err(|_| "profile token is not valid JSON".to_string())
            .and_then(AgyToken::from_native),
    )
}

/// Persist a token refreshed by `agy` back into Agent Control's credential store.
///
/// `agy` keeps its OAuth state in `$HOME/.gemini/antigravity-cli/antigravity-oauth-token`.
/// Because Agent Control points `HOME` at the account profile, that file is
/// the supported place where refreshed credentials appear. It is only written
/// back when it is valid, belongs to the same Google identity, differs from
/// the stored token and is not older than it.
pub fn sync_profile_back(account_id: &Id, cred_ref: &str) -> AuthResult<bool> {
    let (path, mut file) = load_credential(cred_ref, &account_id.0)?;
    let profile_token = profile_dir(account_id)?.join(".gemini/antigravity-cli").join(TOKEN_FILE);
    let fresh = match read_profile_token(&profile_token) {
        None => return Ok(false),
        Some(Err(reason)) => return Err(AgyAuthError::RefreshFailed { label: file.label.clone(), reason }),
        Some(Ok(t)) => t,
    };
    let stored = file.token().map_err(|reason| AgyAuthError::InvalidCredential { label: file.label.clone(), reason })?;
    if fresh.same_secret_as(&stored) || fresh.validate(Utc::now()).is_err() {
        return Ok(false);
    }
    if let (Some(a), Some(b)) = (&file.email, fresh.email()) {
        if !a.eq_ignore_ascii_case(&b) {
            return Err(AgyAuthError::RefreshFailed {
                label: file.label.clone(),
                reason: "the profile is signed in to a different Google account".into(),
            });
        }
    }
    if matches!((fresh.expiry(), stored.expiry()), (Some(f), Some(s)) if f < s) {
        return Ok(false);
    }
    file.credential_data = fresh.native().clone();
    file.updated_at = Utc::now().to_rfc3339();
    write_credential_file(&path, &file)?;
    tracing::info!("Stored refreshed Antigravity credential for account {}", account_id);
    Ok(true)
}

/// Prepare the isolated profile for an account and return the environment the
/// `agy` process must run with. Fails (never falls back) on any problem.
#[cfg(unix)]
pub fn prepare_profile(account_id: &Id, cred_ref: &str) -> AuthResult<HashMap<String, String>> {
    let (_, file) = load_credential(cred_ref, &account_id.0)?;
    let label = file.label.clone();
    let home = profile_dir(account_id)?;
    let profile_err = |reason: String| AgyAuthError::ProfileFailure {
        label: label.clone(),
        path: home.display().to_string(),
        reason,
    };
    let agy_dir = home.join(".gemini").join("antigravity-cli");
    ensure_private_dir(&home).map_err(|e| profile_err(e.to_string()))?;
    fs::create_dir_all(&agy_dir).map_err(|e| profile_err(e.to_string()))?;

    // Keep git/ssh/shell configuration available, but never the real ~/.gemini.
    let real_home = home_dir();
    link_entries(&real_home, &home, |n| n != ".gemini");
    let real_gemini = real_home.join(".gemini");
    link_entries(&real_gemini, &home.join(".gemini"), |n| SHARED_GEMINI.contains(&n));
    link_entries(&real_gemini.join("antigravity-cli"), &agy_dir, |n| SHARED_AGY_CLI.contains(&n));

    // Pick up any token agy refreshed during a previous run, then make the
    // profile hold exactly the stored credential.
    match sync_profile_back(account_id, cred_ref) {
        Ok(_) => {}
        // An unusable profile token is replaced by the stored credential below.
        Err(e @ AgyAuthError::RefreshFailed { .. }) => tracing::warn!("Antigravity profile for {}: {}", account_id, e.to_string().replace('\n', " ")),
        Err(e) => return Err(e),
    }
    let (_, file) = load_credential(cred_ref, &label)?;
    let stored = file.token().map_err(|reason| AgyAuthError::InvalidCredential { label: label.clone(), reason })?;
    let token_path = agy_dir.join(TOKEN_FILE);
    if token_path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        fs::remove_file(&token_path).map_err(|e| profile_err(e.to_string()))?;
    }
    let up_to_date = matches!(read_profile_token(&token_path), Some(Ok(t)) if t.same_secret_as(&stored));
    if !up_to_date {
        let body = serde_json::to_vec_pretty(stored.native()).map_err(|e| profile_err(e.to_string()))?;
        write_private(&token_path, &body).map_err(|e| profile_err(e.to_string()))?;
    }

    let mut envs = HashMap::new();
    for (var, default) in [("XDG_CONFIG_HOME", ".config"), ("XDG_DATA_HOME", ".local/share"), ("XDG_CACHE_HOME", ".cache")] {
        let v = std::env::var(var).unwrap_or_else(|_| real_home.join(default).to_string_lossy().into_owned());
        envs.insert(var.to_string(), v);
    }
    envs.insert("HOME".into(), home.to_string_lossy().into_owned());
    envs.insert("AGENTCONTROL_ACCOUNT_ID".into(), account_id.0.clone());
    Ok(envs)
}

/// Result of removing an account's Agent Control–owned data.
#[derive(Debug, Default)]
pub struct RemovalStaging {
    staged: Vec<(PathBuf, PathBuf)>,
}

/// Step 1 of removal: move the credential file and profile out of the active
/// locations into a staging area (atomic renames). Nothing is deleted yet.
pub fn stage_account_removal(account_id: &Id, cred_ref: &str) -> AuthResult<RemovalStaging> {
    let trash = agentcontrol_config_dir().join(".removing").join(format!("{}-{}", account_id.0, ulid::Ulid::new()));
    let mut staging = RemovalStaging::default();
    let mut targets = vec![profile_dir(account_id)?];
    if let Some(p) = credential_path(cred_ref) {
        targets.push(p);
    }
    for src in targets {
        if src.symlink_metadata().is_err() {
            continue;
        }
        let dst = trash.join(src.file_name().unwrap_or_default());
        let res = fs::create_dir_all(&trash).and_then(|_| fs::rename(&src, &dst));
        if let Err(e) = res {
            staging.rollback();
            return Err(AgyAuthError::Storage(format!("could not remove {}: {e}", src.display())));
        }
        staging.staged.push((src, dst));
    }
    Ok(staging)
}

impl RemovalStaging {
    /// Undo staging (used when the database removal fails).
    pub fn rollback(&mut self) {
        for (src, dst) in self.staged.drain(..).rev() {
            let _ = fs::rename(&dst, &src);
        }
    }

    /// Permanently delete staged data. Returns human-readable cleanup failures.
    /// `remove_dir_all` removes symlinks without following them, so the user's
    /// real `~/.gemini`, `~/.ssh`, etc. are never touched.
    pub fn commit(mut self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut parent = None;
        for (_, dst) in self.staged.drain(..) {
            parent = dst.parent().map(Path::to_path_buf);
            let res = if dst.is_dir() && !dst.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
                fs::remove_dir_all(&dst)
            } else {
                fs::remove_file(&dst)
            };
            if let Err(e) = res {
                errors.push(format!("{}: {e}", dst.display()));
            }
        }
        if let Some(p) = parent {
            let _ = fs::remove_dir(p);
        }
        errors
    }
}

// ── Local import ──────────────────────────────────────────────────────────────

/// Detect the machine's existing `agy` login (`~/.gemini/antigravity-cli/antigravity-oauth-token`).
pub fn detect_local_login() -> Option<(String, AgyToken)> {
    let path = home_dir().join(".gemini/antigravity-cli").join(TOKEN_FILE);
    let tok = read_profile_token(&path)?.ok()?;
    let identity = tok.email().unwrap_or_else(|| "local agy login".into());
    Some((identity, tok))
}

// ── Browser OAuth login ───────────────────────────────────────────────────────

/// OAuth client used for Antigravity browser login.
///
/// Antigravity uses a Google "installed application" OAuth client. For that
/// client type Google does not treat the client secret as confidential (it is
/// shipped inside the `agy` binary), and security instead comes from the
/// loopback redirect, `state`, and PKCE. Agent Control therefore does not
/// embed it in source; it is read at runtime from the environment or a
/// private config file, and it is only ever sent in an HTTPS request body.
#[derive(Clone)]
pub struct OAuthClient {
    pub client_id: String,
    client_secret: String,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient").field("client_id", &self.client_id).finish_non_exhaustive()
    }
}

impl OAuthClient {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self { client_id: client_id.into(), client_secret: client_secret.into() }
    }

    pub fn load() -> AuthResult<Self> {
        let not_configured = || AgyAuthError::OAuthNotConfigured(oauth_client_config_path().display().to_string());
        if let (Ok(id), Ok(secret)) = (std::env::var("AC_AGY_OAUTH_CLIENT_ID"), std::env::var("AC_AGY_OAUTH_CLIENT_SECRET")) {
            if !id.trim().is_empty() && !secret.trim().is_empty() {
                return Ok(Self::new(id.trim(), secret.trim()));
            }
        }
        let content = fs::read_to_string(oauth_client_config_path()).map_err(|_| not_configured())?;
        let v: Value = serde_json::from_str(&content).map_err(|_| not_configured())?;
        match (v["client_id"].as_str(), v["client_secret"].as_str()) {
            (Some(id), Some(s)) if !id.is_empty() && !s.is_empty() => Ok(Self::new(id, s)),
            _ => Err(not_configured()),
        }
    }
}

impl OAuthClient {
    /// Resolve the OAuth client without any user configuration:
    ///  1. `AC_AGY_OAUTH_CLIENT_ID` / `AC_AGY_OAUTH_CLIENT_SECRET` (advanced override),
    ///  2. the private cache written by a previous discovery,
    ///  3. discovery from the user's installed `agy` binary, verified with Google.
    pub async fn resolve() -> AuthResult<Self> {
        if let Ok(c) = Self::load() {
            return Ok(c);
        }
        let bin = find_agy_binary().ok_or_else(|| AgyAuthError::OAuthNotConfigured("agy is not installed on this machine.".into()))?;
        let client = Self::discover_from_binary(&bin, GOOGLE_TOKEN_URL).await?;
        let body = serde_json::json!({
            "client_id": client.client_id,
            "client_secret": client.client_secret,
            "source": "discovered from the local agy installation",
        });
        let _ = ensure_private_dir(&agentcontrol_config_dir())
            .and_then(|_| write_private(&oauth_client_config_path(), &serde_json::to_vec_pretty(&body).unwrap_or_default()));
        Ok(client)
    }

    /// Find the installed-app client secret embedded in the agy binary. Each
    /// candidate is verified by sending Google a deliberately invalid code:
    /// `invalid_grant` means the client credentials are correct,
    /// `invalid_client` means they are not. No user data is involved.
    pub async fn discover_from_binary(bin: &Path, token_url: &str) -> AuthResult<Self> {
        let bytes = fs::read(bin).map_err(|e| AgyAuthError::OAuthNotConfigured(format!("could not read {}: {}", bin.display(), e.kind())))?;
        let re = regex::bytes::Regex::new(r"GOCSPX-[A-Za-z0-9_-]{28}").expect("valid regex");
        let mut candidates: Vec<String> = re.find_iter(&bytes).map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned()).collect();
        candidates.sort();
        candidates.dedup();
        drop(bytes);
        if candidates.is_empty() {
            return Err(AgyAuthError::OAuthNotConfigured("the installed agy does not contain a login configuration.".into()));
        }
        let http = http_client()?;
        for secret in candidates {
            let form = [
                ("client_id", AGY_OAUTH_CLIENT_ID),
                ("client_secret", secret.as_str()),
                ("code", "agentcontrol-client-probe"),
                ("grant_type", "authorization_code"),
                ("redirect_uri", "http://127.0.0.1/oauth2callback"),
            ];
            let body = form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
            let resp = http
                .post(token_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|e| AgyAuthError::OAuthNotConfigured(format!("could not reach Google to verify the login configuration: {}", e.without_url())))?;
            let text = resp.text().await.unwrap_or_default();
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if v.get("error").and_then(Value::as_str) == Some("invalid_grant") {
                return Ok(Self::new(AGY_OAUTH_CLIENT_ID, secret));
            }
        }
        Err(AgyAuthError::OAuthNotConfigured("the installed agy's login configuration was not accepted by Google.".into()))
    }
}

/// Locate the real agy binary (honours ANTIGRAVITY_BIN / AGY_BIN, then PATH).
pub fn find_agy_binary() -> Option<PathBuf> {
    let p = PathBuf::from(crate::adapter::resolve_real_antigravity_bin());
    if p.is_absolute() {
        return p.is_file().then_some(p);
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(&p)).find(|c| c.is_file()))
}

fn http_client() -> AuthResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| AgyAuthError::LoginFailed(format!("HTTP client error: {e}")))
}

/// Start a browser login: resolve the OAuth client and bind the callback listener.
pub async fn begin_browser_login() -> AuthResult<BrowserLogin> {
    BrowserLogin::start(OAuthClient::resolve().await?).await
}

/// Wait for the callback, exchange the code, validate and only then save the
/// credential. Returns `(credential_ref, token)`. Nothing is written on failure.
pub async fn finish_browser_login(login: &BrowserLogin, label: &str, timeout: Duration) -> AuthResult<(String, AgyToken)> {
    let code = login.wait_for_callback(timeout).await?;
    let token = login.exchange(&code).await?;
    let cref = save_new_credential(label, &token, "browser_login")?;
    Ok((cref, token))
}

/// Complete a login started for a shareable login link.
///
/// Google's installed-application clients (which is what Antigravity uses)
/// only support loopback redirects, so a link opened on another device ends on
/// an unreachable `http://127.0.0.1:<port>/oauth2callback?...` page. The
/// supported handoff is for the user to paste that final address back here.
/// Either path completes the same pending login:
///  - the local loopback callback (link opened on this machine), or
///  - a pasted redirect address received on `pasted`.
///
/// Security properties: the `state` generated for this login must match; the
/// PKCE verifier never leaves this process, so a code is useless to anyone
/// else; the first valid code ends the login (one-time use) and dropping the
/// `BrowserLogin` afterwards invalidates the state; the whole login expires
/// after `timeout`. A rejected paste is reported via `on_rejected` and the
/// login keeps waiting. Nothing is saved unless the token exchange and
/// validation succeed.
pub async fn finish_login_with_handoff(
    login: &BrowserLogin,
    label: &str,
    timeout: Duration,
    pasted: &mut tokio::sync::mpsc::Receiver<String>,
    on_rejected: impl Fn(String),
) -> AuthResult<(String, AgyToken)> {
    let deadline = tokio::time::Instant::now() + timeout;
    let timed_out = || AgyAuthError::LoginFailed("Authentication timed out. No credential was saved.".into());
    let mut paste_open = true;
    let code = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(timed_out());
        }
        tokio::select! {
            res = login.wait_for_callback(remaining) => break res?,
            p = pasted.recv(), if paste_open => match p {
                None => paste_open = false,
                Some(text) => match login.code_from_pasted(&text) {
                    Ok(code) => break code,
                    Err(e) => on_rejected(e.to_string().replace("Antigravity login failed.\n\nThe account was not added.\n\nReason:\n", "")),
                },
            },
        }
    };
    let token = login.exchange(&code).await?;
    let cref = save_new_credential(label, &token, "browser_login")?;
    Ok((cref, token))
}

/// Delete a credential that was saved but could not be registered.
pub fn discard_credential(cred_ref: &str) {
    if let Some(p) = credential_path(cred_ref) {
        let _ = fs::remove_file(p);
    }
}

fn random_urlsafe(bytes: usize) -> AuthResult<String> {
    let mut buf = vec![0u8; bytes];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| AgyAuthError::LoginFailed(format!("no secure randomness available: {e}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf))
}

/// A pending browser login. The loopback listener, the authorization URL and
/// the token exchange all use the same dynamically allocated port.
pub struct BrowserLogin {
    listener: tokio::net::TcpListener,
    client: OAuthClient,
    redirect_uri: String,
    state: String,
    verifier: String,
    auth_url: String,
    token_url: String,
}

impl BrowserLogin {
    /// Bind the callback listener on an ephemeral loopback port.
    pub async fn start(client: OAuthClient) -> AuthResult<Self> {
        Self::start_with(client, "127.0.0.1:0", GOOGLE_TOKEN_URL).await
    }

    /// Same as [`start`](Self::start) with an explicit bind address / token endpoint (tests).
    pub async fn start_with(client: OAuthClient, bind: &str, token_url: &str) -> AuthResult<Self> {
        let listener = tokio::net::TcpListener::bind(bind).await.map_err(|e| {
            AgyAuthError::CallbackUnavailable(format!("Port unavailable ({bind}): {e}"))
        })?;
        let port = listener
            .local_addr()
            .map_err(|e| AgyAuthError::CallbackUnavailable(e.to_string()))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/oauth2callback");
        let state = random_urlsafe(24)?;
        let verifier = random_urlsafe(48)?;
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", OAUTH_SCOPES)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent select_account")
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .finish();
        Ok(Self {
            listener,
            client,
            auth_url: format!("{GOOGLE_AUTH_URL}?{query}"),
            redirect_uri,
            state,
            verifier,
            token_url: token_url.to_string(),
        })
    }

    pub fn authorization_url(&self) -> &str {
        &self.auth_url
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Wait for the browser redirect and return the authorization code.
    pub async fn wait_for_callback(&self, timeout: Duration) -> AuthResult<String> {
        tokio::time::timeout(timeout, async {
            loop {
                let (mut sock, _) = self.listener.accept().await.map_err(|e| {
                    AgyAuthError::LoginFailed(format!("login callback failed: {e}"))
                })?;
                let mut buf = vec![0u8; 8192];
                let n = tokio::time::timeout(Duration::from_secs(10), sock.read(&mut buf))
                    .await
                    .unwrap_or(Ok(0))
                    .unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let target = req.split_whitespace().nth(1).unwrap_or("");
                if !target.starts_with("/oauth2callback") {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    continue;
                }
                let result = self.parse_callback(target);
                let (title, body) = match &result {
                    Ok(_) => ("Signed in", "Antigravity login received. You can close this window and return to Agent Control."),
                    Err(_) => ("Login failed", "Antigravity login failed. Return to Agent Control for details."),
                };
                let html = format!("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body style=\"font-family:sans-serif;text-align:center;margin-top:15vh\"><h2>{title}</h2><p>{body}</p></body></html>");
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{html}",
                    html.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
                return result;
            }
        })
        .await
        .map_err(|_| AgyAuthError::LoginFailed("Authentication timed out. No credential was saved.".into()))?
    }

    /// Open the authorization URL in the default browser.
    pub fn open_in_browser(&self) -> bool {
        open_browser(&self.auth_url)
    }

    /// Accept a pasted redirect URL (or bare code) for headless logins.
    pub fn code_from_pasted(&self, pasted: &str) -> AuthResult<String> {
        let pasted = pasted.trim();
        if pasted.is_empty() {
            return Err(AgyAuthError::LoginFailed("no authorization code was entered".into()));
        }
        match pasted.find("/oauth2callback") {
            Some(i) => self.parse_callback(&pasted[i..]),
            None if pasted.contains("code=") => self.parse_callback(&format!("/oauth2callback?{}", pasted.trim_start_matches('?'))),
            None => Err(AgyAuthError::LoginFailed("Paste the full address from the browser's address bar.".into())),
        }
    }

    fn parse_callback(&self, target: &str) -> AuthResult<String> {
        let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
        let params: HashMap<String, String> = form_urlencoded::parse(query.as_bytes()).into_owned().collect();
        if let Some(err) = params.get("error") {
            return Err(AgyAuthError::LoginFailed(match err.as_str() {
                "access_denied" => "Authorization was denied.".to_string(),
                other => format!("Google returned an error: {}", sanitize(other)),
            }));
        }
        if params.get("state") != Some(&self.state) {
            return Err(AgyAuthError::LoginFailed("Invalid OAuth state.".into()));
        }
        params
            .get("code")
            .filter(|c| !c.is_empty())
            .cloned()
            .ok_or_else(|| AgyAuthError::LoginFailed("callback did not contain an authorization code".into()))
    }

    /// Exchange the authorization code for tokens. The code, verifier and
    /// client secret travel only in the HTTPS request body.
    pub async fn exchange(&self, code: &str) -> AuthResult<AgyToken> {
        let form = [
            ("client_id", self.client.client_id.as_str()),
            ("client_secret", self.client.client_secret.as_str()),
            ("code", code),
            ("code_verifier", self.verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", self.redirect_uri.as_str()),
        ];
        let body = form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
        let resp = http_client()?
            .post(&self.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| AgyAuthError::LoginFailed(format!("could not reach Google token endpoint: {}", e.without_url())))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| AgyAuthError::LoginFailed(format!("token response unreadable: {}", e.without_url())))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|_| AgyAuthError::LoginFailed(format!("malformed token response (HTTP {status})")))?;
        if let Some(err) = v.get("error").and_then(Value::as_str) {
            let desc = v.get("error_description").and_then(Value::as_str).unwrap_or("");
            return Err(AgyAuthError::LoginFailed(format!("token exchange rejected: {} {}", sanitize(err), sanitize(desc)).trim().to_string()));
        }
        if !status.is_success() {
            return Err(AgyAuthError::LoginFailed(format!("token exchange failed (HTTP {status})")));
        }
        let tok = AgyToken::from_google_response(&v, Utc::now()).map_err(|e| AgyAuthError::LoginFailed(format!("invalid token response: {e}")))?;
        tok.validate(Utc::now()).map_err(AgyAuthError::LoginFailed)?;
        Ok(tok)
    }
}

fn sanitize(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(200).collect()
}

// ── Small shared helpers ──────────────────────────────────────────────────────

fn percent_decode(s: &str) -> String {
    form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

/// Extract the `email` claim from a JWT payload (no signature verification;
/// used for display / identity matching only).
pub fn email_from_jwt(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("email").and_then(Value::as_str).map(str::to_string)
}

/// Open a URL in the user's browser. The URL is passed as a single argument
/// and never contains secrets (only client_id, state and PKCE challenge).
pub fn open_browser(url: &str) -> bool {
    let candidates: &[&str] = if cfg!(target_os = "macos") { &["open"] } else { &["xdg-open"] };
    candidates.iter().any(|cmd| {
        std::process::Command::new(cmd)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(email: &str) -> String {
        let p = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{{\"email\":\"{email}\"}}"));
        format!("h.{p}.s")
    }

    #[test]
    fn google_response_converts_to_native() {
        let now = Utc::now();
        let v = serde_json::json!({"access_token":"a","expires_in":3600,"refresh_token":"r","token_type":"Bearer","id_token": jwt("a@x.com")});
        let t = AgyToken::from_google_response(&v, now).unwrap();
        assert_eq!(t.native()["auth_method"], "consumer");
        assert!(t.has_refresh_token());
        assert_eq!(t.email().as_deref(), Some("a@x.com"));
        assert!(t.validate(now).is_ok());
    }

    #[test]
    fn response_without_refresh_token_rejected() {
        let v = serde_json::json!({"access_token":"a","expires_in":3600});
        assert!(AgyToken::from_google_response(&v, Utc::now()).is_err());
    }

    #[test]
    fn classification_never_treats_code_as_token() {
        let now = Utc::now();
        assert_eq!(classify_material("", now), TokenMaterial::Empty);
        assert_eq!(classify_material("   ", now), TokenMaterial::Empty);
        assert_eq!(classify_material("4/0AbCdEf-xyz", now), TokenMaterial::AuthorizationCode);
        assert_eq!(classify_material("4%2F0AbCdEf", now), TokenMaterial::AuthorizationCode);
        assert!(matches!(classify_material("random-string", now), TokenMaterial::Invalid(_)));
        assert!(matches!(classify_material("{\"foo\":1}", now), TokenMaterial::Invalid(_)));
        assert!(matches!(classify_material("{\"token\":{}}", now), TokenMaterial::Invalid(_)));
        let native = r#"{"token":{"access_token":"a","refresh_token":"r","expiry":"2000-01-01T00:00:00Z"},"auth_method":"consumer","id_token":""}"#;
        assert!(matches!(classify_material(native, now), TokenMaterial::Usable(_)));
    }

    #[test]
    fn expired_without_refresh_is_invalid() {
        let v = serde_json::json!({"token":{"access_token":"a","expiry":"2000-01-01T00:00:00Z"}});
        let t = AgyToken::from_native(v).unwrap();
        assert!(t.validate(Utc::now()).is_err());
    }

    #[test]
    fn credential_refs_are_strict() {
        assert!(credential_path("ref:agy:01ABC").is_some());
        assert!(credential_path("ref:antigravity:01ABC").is_some());
        assert!(credential_path("ref:agy:../../etc").is_none());
        assert!(credential_path("ref:claude:01ABC").is_none());
        assert!(credential_path("plain").is_none());
    }

    #[test]
    fn profile_dir_rejects_traversal() {
        assert!(profile_dir(&Id::from("../x")).is_err());
        assert!(profile_dir(&Id::from("")).is_err());
        assert!(profile_dir(&Id::from("01ABC")).is_ok());
    }

    #[test]
    fn errors_never_contain_secrets() {
        let e = AgyAuthError::AuthorizationCodeOnly { label: "College Google".into() };
        let s = e.to_string();
        assert!(s.contains("College Google") && s.contains("cannot be started") && s.contains("sign in again"));
        let dbg = format!("{:?}", OAuthClient::new("id", "SUPERSECRET"));
        assert!(!dbg.contains("SUPERSECRET"));
    }

    #[tokio::test]
    async fn callback_port_matches_redirect_and_exchange() {
        let login = BrowserLogin::start(OAuthClient::new("cid", "sec")).await.unwrap();
        let port = login.listener.local_addr().unwrap().port();
        assert_eq!(login.redirect_uri(), format!("http://127.0.0.1:{port}/oauth2callback"));
        let enc = form_urlencoded::byte_serialize(login.redirect_uri().as_bytes()).collect::<String>();
        assert!(login.authorization_url().contains(&format!("redirect_uri={enc}")));
        assert!(login.authorization_url().contains("code_challenge_method=S256"));
        assert!(!login.authorization_url().contains("sec"));
    }

    #[tokio::test]
    async fn callback_port_unavailable_is_reported() {
        let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = busy.local_addr().unwrap().to_string();
        let err = BrowserLogin::start_with(OAuthClient::new("c", "s"), &addr, GOOGLE_TOKEN_URL).await.err().unwrap();
        assert!(matches!(err, AgyAuthError::CallbackUnavailable(_)));
        assert!(err.to_string().contains("Unable to start Antigravity login callback"));
    }

    #[tokio::test]
    async fn state_mismatch_and_error_callbacks_fail() {
        let login = BrowserLogin::start(OAuthClient::new("c", "s")).await.unwrap();
        assert!(login.code_from_pasted("http://127.0.0.1:1/oauth2callback?code=abc&state=WRONG").is_err());
        assert!(login.code_from_pasted("http://127.0.0.1:1/oauth2callback?error=access_denied").is_err());
        let ok = format!("http://127.0.0.1:1/oauth2callback?code=4%2F0Ab&state={}", login.state);
        assert_eq!(login.code_from_pasted(&ok).unwrap(), "4/0Ab");
    }
}
