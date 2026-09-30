//! User configuration and global defaults persistence.
//!
//! Stores user preferences in `~/.config/agentcontrol/settings.json`
//! (honours `XDG_CONFIG_HOME`). Defaults can be configured globally in
//! the Settings view and overridden per-session.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::agy_launch::{AgyExecutionMode, AgyPermissionMode};

fn default_agent() -> String {
    "Antigravity".into()
}

fn default_theme() -> String {
    "Default".into()
}

fn default_scrollback() -> usize {
    5000
}

fn default_cursor() -> String {
    "Block".into()
}

/// Global user settings persisted across application runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSettings {
    /// Default agent type / name (e.g. "Antigravity").
    #[serde(default = "default_agent")]
    pub default_agent: String,

    /// Default account label or ID to pre-select for new sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_account: Option<String>,

    /// Default project name or ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_project: Option<String>,

    /// Default working directory for session launches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_working_dir: Option<String>,

    /// Default AGY execution mode (`default`, `accept-edits`, `plan`).
    #[serde(default)]
    pub default_execution_mode: AgyExecutionMode,

    /// Default AGY permission mode (`normal`, `dangerously-skip-permissions`).
    #[serde(default)]
    pub default_permission_mode: AgyPermissionMode,

    /// Default model ID for Antigravity sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,

    /// Terminal theme name (e.g. "Default", "Dark", "High-Contrast").
    #[serde(default = "default_theme")]
    pub terminal_theme: String,

    /// Terminal scrollback buffer line count.
    #[serde(default = "default_scrollback")]
    pub terminal_scrollback: usize,

    /// Terminal cursor style ("Block", "Bar", "Underline").
    #[serde(default = "default_cursor")]
    pub terminal_cursor: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            default_agent: default_agent(),
            default_account: None,
            default_project: None,
            default_working_dir: None,
            default_execution_mode: AgyExecutionMode::Default,
            default_permission_mode: AgyPermissionMode::Normal,
            default_model: None,
            terminal_theme: default_theme(),
            terminal_scrollback: default_scrollback(),
            terminal_cursor: default_cursor(),
        }
    }
}

impl UserSettings {
    /// Path to `~/.config/agentcontrol/settings.json`.
    pub fn file_path() -> PathBuf {
        crate::agy_auth::agentcontrol_config_dir().join("settings.json")
    }

    /// Load persisted settings from disk, or return default settings.
    pub fn load() -> Self {
        let p = Self::file_path();
        if let Ok(content) = std::fs::read_to_string(&p) {
            if let Ok(settings) = serde_json::from_str::<Self>(&content) {
                return settings;
            }
        }
        Self::default()
    }

    /// Save the current settings to disk.
    pub fn save(&self) -> Result<(), String> {
        let p = Self::file_path();
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&p, json).map_err(|e| e.to_string())
    }
}
