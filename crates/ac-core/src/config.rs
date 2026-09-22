//! Daemon configuration loaded from a TOML file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Path to the SQLite database file.
    pub db_path: PathBuf,
    /// Path for the Unix domain socket.
    pub socket_path: PathBuf,
    /// Maximum restart attempts before a session reaches Failed state.
    pub max_restarts: u32,
    /// Base back-off delay in milliseconds for restart attempts.
    pub restart_backoff_base_ms: u64,
    /// Log level: error, warn, info, debug, trace.
    pub log_level: String,
    /// Whether the loopback WebSocket server is enabled (Phase 7 / ADR-009).
    pub ws_enabled: bool,
    /// Loopback bind address for the WebSocket server (e.g. "127.0.0.1:4242").
    pub ws_bind_addr: String,
    /// Optional path to a file containing authentication tokens (one per line: `<token>:<scope>`).
    pub ws_auth_token_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        let base = dirs_base();
        Self {
            db_path: base.join("events.db"),
            socket_path: runtime_dir().join("agentcontrol.sock"),
            max_restarts: 3,
            restart_backoff_base_ms: 1000,
            log_level: "info".to_owned(),
            ws_enabled: true,
            ws_bind_addr: "127.0.0.1:4242".to_owned(),
            ws_auth_token_path: None,
        }
    }
}

impl Config {
    /// Load configuration from a TOML file, falling back to defaults for
    /// any missing fields.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file: {}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parsing config file: {}", path.display()))?;
        Ok(cfg)
    }

    /// Load from path if it exists, otherwise return defaults.
    pub fn load_or_default(path: &Path) -> Self {
        if path.exists() {
            Self::load(path).unwrap_or_default()
        } else {
            Self::default()
        }
    }
}

fn dirs_base() -> PathBuf {
    // XDG_DATA_HOME or ~/.local/share/agentcontrol
    std::env::var("XDG_DATA_HOME")
        .map(|d| PathBuf::from(d).join("agentcontrol"))
        .unwrap_or_else(|_| {
            dirs_home()
                .join(".local")
                .join("share")
                .join("agentcontrol")
        })
}

fn runtime_dir() -> PathBuf {
    // XDG_RUNTIME_DIR or /tmp
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}
