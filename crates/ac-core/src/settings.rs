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

    /// Configurable keybindings for common actions and navigation.
    #[serde(default)]
    pub keybindings: KeybindingsConfig,
}

/// Configurable keybindings for AgentControll actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindingsConfig {
    #[serde(default = "default_detach_session")]
    pub detach_session: String,

    #[serde(default = "default_toggle_split")]
    pub toggle_split: String,

    #[serde(default = "default_switch_split_focus")]
    pub switch_split_focus: String,

    #[serde(default = "default_toggle_fullscreen")]
    pub toggle_fullscreen: String,

    #[serde(default = "default_command_palette")]
    pub command_palette: String,

    #[serde(default = "default_url_picker")]
    pub url_picker: String,

    #[serde(default = "default_new_session")]
    pub new_session: String,

    #[serde(default = "default_steer_session")]
    pub steer_session: String,

    #[serde(default = "default_pause_session")]
    pub pause_session: String,

    #[serde(default = "default_resume_session")]
    pub resume_session: String,

    #[serde(default = "default_stop_session")]
    pub stop_session: String,

    #[serde(default = "default_switch_account")]
    pub switch_account: String,

    #[serde(default = "default_remove_session")]
    pub remove_session: String,

    #[serde(default = "default_add_account")]
    pub add_account: String,

    #[serde(default = "default_search_scrollback")]
    pub search_scrollback: String,

    #[serde(default = "default_visual_mode")]
    pub visual_mode: String,

    #[serde(default = "default_yank_selection")]
    pub yank_selection: String,

    #[serde(default = "default_refresh")]
    pub refresh: String,

    #[serde(default = "default_open_help")]
    pub open_help: String,

    #[serde(default = "default_quit")]
    pub quit: String,
}

fn default_detach_session() -> String { "Ctrl+Q".into() }
fn default_toggle_split() -> String { "Alt+S".into() }
fn default_switch_split_focus() -> String { "Alt+W".into() }
fn default_toggle_fullscreen() -> String { "Ctrl+F".into() }
fn default_command_palette() -> String { "Ctrl+P".into() }
fn default_url_picker() -> String { "Ctrl+O".into() }
fn default_new_session() -> String { "n".into() }
fn default_steer_session() -> String { "s".into() }
fn default_pause_session() -> String { "p".into() }
fn default_resume_session() -> String { "Space".into() }
fn default_stop_session() -> String { "x".into() }
fn default_switch_account() -> String { "w".into() }
fn default_remove_session() -> String { "d".into() }
fn default_add_account() -> String { "a".into() }
fn default_search_scrollback() -> String { "/".into() }
fn default_visual_mode() -> String { "v".into() }
fn default_yank_selection() -> String { "y".into() }
fn default_refresh() -> String { "r".into() }
fn default_open_help() -> String { "?".into() }
fn default_quit() -> String { "q".into() }

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            detach_session: default_detach_session(),
            toggle_split: default_toggle_split(),
            switch_split_focus: default_switch_split_focus(),
            toggle_fullscreen: default_toggle_fullscreen(),
            command_palette: default_command_palette(),
            url_picker: default_url_picker(),
            new_session: default_new_session(),
            steer_session: default_steer_session(),
            pause_session: default_pause_session(),
            resume_session: default_resume_session(),
            stop_session: default_stop_session(),
            switch_account: default_switch_account(),
            remove_session: default_remove_session(),
            add_account: default_add_account(),
            search_scrollback: default_search_scrollback(),
            visual_mode: default_visual_mode(),
            yank_selection: default_yank_selection(),
            refresh: default_refresh(),
            open_help: default_open_help(),
            quit: default_quit(),
        }
    }
}

impl KeybindingsConfig {
    pub const ACTION_COUNT: usize = 20;

    pub const ACTION_LIST: [(&'static str, &'static str, &'static str); 20] = [
        ("detach_session", "Detach Session", "Exit session terminal view back to tabs"),
        ("toggle_split", "Toggle Split Pane", "Side-by-side terminal split toggle"),
        ("switch_split_focus", "Switch Split Focus", "Switch active focus between split panes"),
        ("toggle_fullscreen", "Toggle Fullscreen", "Distraction-free edge-to-edge terminal"),
        ("command_palette", "Command Palette", "Open command palette / actions in terminal"),
        ("url_picker", "URL & Link Picker", "Extract & open hyperlinks from terminal"),
        ("new_session", "New Session", "Open session launch dialog in main tabs"),
        ("steer_session", "Steer Session", "Inject steering instruction into session"),
        ("pause_session", "Pause Session", "Pause working session in Sessions tab"),
        ("resume_session", "Resume Session", "Resume paused session in Sessions tab"),
        ("stop_session", "Stop Session", "Stop running session in Sessions tab"),
        ("switch_account", "Switch Account", "Switch session account / handoff dialog"),
        ("remove_session", "Remove Session", "Open session remove confirmation dialog"),
        ("add_account", "Add Account", "Add Antigravity account (browser/import)"),
        ("search_scrollback", "Search History", "Search terminal scrollback buffer"),
        ("visual_mode", "Visual Mode", "Start visual text selection in terminal"),
        ("yank_selection", "Yank / Copy", "Copy visual text selection to clipboard"),
        ("refresh", "Refresh Daemon", "Force re-sync daemon state and sessions"),
        ("open_help", "Open Help", "Open keyboard shortcut help dialog"),
        ("quit", "Quit Application", "Exit Agent Control TUI"),
    ];

    pub fn get(&self, action: &str) -> &str {
        match action {
            "detach_session" => &self.detach_session,
            "toggle_split" => &self.toggle_split,
            "switch_split_focus" => &self.switch_split_focus,
            "toggle_fullscreen" => &self.toggle_fullscreen,
            "command_palette" => &self.command_palette,
            "url_picker" => &self.url_picker,
            "new_session" => &self.new_session,
            "steer_session" => &self.steer_session,
            "pause_session" => &self.pause_session,
            "resume_session" => &self.resume_session,
            "stop_session" => &self.stop_session,
            "switch_account" => &self.switch_account,
            "remove_session" => &self.remove_session,
            "add_account" => &self.add_account,
            "search_scrollback" => &self.search_scrollback,
            "visual_mode" => &self.visual_mode,
            "yank_selection" => &self.yank_selection,
            "refresh" => &self.refresh,
            "open_help" => &self.open_help,
            "quit" => &self.quit,
            _ => "",
        }
    }

    pub fn set(&mut self, action: &str, key: String) {
        match action {
            "detach_session" => self.detach_session = key,
            "toggle_split" => self.toggle_split = key,
            "switch_split_focus" => self.switch_split_focus = key,
            "toggle_fullscreen" => self.toggle_fullscreen = key,
            "command_palette" => self.command_palette = key,
            "url_picker" => self.url_picker = key,
            "new_session" => self.new_session = key,
            "steer_session" => self.steer_session = key,
            "pause_session" => self.pause_session = key,
            "resume_session" => self.resume_session = key,
            "stop_session" => self.stop_session = key,
            "switch_account" => self.switch_account = key,
            "remove_session" => self.remove_session = key,
            "add_account" => self.add_account = key,
            "search_scrollback" => self.search_scrollback = key,
            "visual_mode" => self.visual_mode = key,
            "yank_selection" => self.yank_selection = key,
            "refresh" => self.refresh = key,
            "open_help" => self.open_help = key,
            "quit" => self.quit = key,
            _ => {}
        }
    }

    pub fn default_for_action(action: &str) -> &'static str {
        match action {
            "detach_session" => "Ctrl+Q",
            "toggle_split" => "Alt+S",
            "switch_split_focus" => "Alt+W",
            "toggle_fullscreen" => "Ctrl+F",
            "command_palette" => "Ctrl+P",
            "url_picker" => "Ctrl+O",
            "new_session" => "n",
            "steer_session" => "s",
            "pause_session" => "p",
            "resume_session" => "Space",
            "stop_session" => "x",
            "switch_account" => "w",
            "remove_session" => "d",
            "add_account" => "a",
            "search_scrollback" => "/",
            "visual_mode" => "v",
            "yank_selection" => "y",
            "refresh" => "r",
            "open_help" => "?",
            "quit" => "q",
            _ => "",
        }
    }

    pub fn is_custom(&self, action: &str) -> bool {
        self.get(action).trim().to_lowercase() != Self::default_for_action(action).trim().to_lowercase()
    }
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
            keybindings: KeybindingsConfig::default(),
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
