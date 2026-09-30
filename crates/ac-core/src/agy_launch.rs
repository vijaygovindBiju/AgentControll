//! Per-session launch options for Antigravity (`agy`) sessions:
//! execution mode, permission / access mode, model and working directory.
//!
//! Two distinct concepts are enforced:
//! A. AGY Execution Mode: `default`, `accept-edits`, `plan` (via `--mode=<val>`)
//! B. AGY Permission / Access Control: `normal`, `dangerously-skip-permissions`
//!    (via `--dangerously-skip-permissions`)
//!
//! Only options that the installed `agy` genuinely supports on its command line
//! are offered. Support is detected from the usage strings embedded in the
//! installed binary (the binary is never executed for detection).

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::types::Id;

/// Per-session execution mode for Antigravity CLI.
///
/// Selected on the CLI via `--mode=accept-edits` or `--mode=plan`.
/// When `Default` is selected, no mode override is passed (standard agy behavior).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgyExecutionMode {
    /// Standard agy interactive workflow (no `--mode` override).
    #[default]
    Default,
    /// `--mode=accept-edits`: auto-approve file edits, prompt for commands.
    #[serde(alias = "accept_edits")]
    AcceptEdits,
    /// `--mode=plan`: research and plan without making changes.
    Plan,
}

impl AgyExecutionMode {
    pub const ALL: [Self; 3] = [Self::Default, Self::AcceptEdits, Self::Plan];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::AcceptEdits => "Accept Edits",
            Self::Plan => "Plan",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Default => "Standard AGY flow with normal prompt and review",
            Self::AcceptEdits => {
                "Auto-approve file edits, prompt for commands (--mode=accept-edits)"
            }
            Self::Plan => "Research and plan only, no file edits (--mode=plan)",
        }
    }

    /// CLI argument string for this mode, if an override is needed.
    pub fn flag(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::AcceptEdits => Some("--mode=accept-edits"),
            Self::Plan => Some("--mode=plan"),
        }
    }

    /// Execution modes supported according to agy's usage text.
    pub fn supported_in_usage(usage: &str) -> Vec<Self> {
        let mut modes = vec![Self::Default];
        // Check for accept-edits and plan in CLI help / strings
        if usage.contains("accept-edits") {
            modes.push(Self::AcceptEdits);
        }
        if usage.contains("plan") {
            modes.push(Self::Plan);
        }
        modes
    }
}

/// Per-session permission / access control for Antigravity CLI.
///
/// Permission handling is separate from execution mode.
/// Dangerous permission bypass passes `--dangerously-skip-permissions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgyPermissionMode {
    /// Normal AGY permissions (no flag; agy prompts for approval per its policy).
    #[default]
    Normal,
    /// `--dangerously-skip-permissions`: auto-approve all tool actions without prompting.
    #[serde(alias = "dangerously_skip_permissions")]
    DangerouslySkipPermissions,
}

impl AgyPermissionMode {
    pub const ALL: [Self; 2] = [Self::Normal, Self::DangerouslySkipPermissions];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal / AGY default permissions",
            Self::DangerouslySkipPermissions => "Dangerously Skip Permissions",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Normal => "Asks before file changes and commands",
            Self::DangerouslySkipPermissions => {
                "Auto-approve tool actions without prompting (--dangerously-skip-permissions)"
            }
        }
    }

    pub fn is_dangerous(self) -> bool {
        self == Self::DangerouslySkipPermissions
    }

    /// CLI argument string for this permission option, if any.
    pub fn flag(self) -> Option<&'static str> {
        match self {
            Self::Normal => None,
            Self::DangerouslySkipPermissions => Some("--dangerously-skip-permissions"),
        }
    }

    /// Permission modes supported according to agy's usage text.
    pub fn supported_in_usage(usage: &str) -> Vec<Self> {
        let mut modes = vec![Self::Normal];
        if usage.contains("dangerously-skip-permissions") {
            modes.push(Self::DangerouslySkipPermissions);
        }
        modes
    }
}

/// Launch options chosen for one Antigravity session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgyLaunchOptions {
    /// Agent execution mode (Default, AcceptEdits, Plan).
    #[serde(default)]
    pub execution_mode: AgyExecutionMode,

    /// Permission / access control (Normal, DangerouslySkipPermissions).
    #[serde(default)]
    pub permission_mode: AgyPermissionMode,

    /// Optional sandbox mode (`--sandbox`).
    #[serde(default)]
    pub sandbox: bool,

    /// Model id as listed by `agy models` (e.g. `gemini-3.8-flash-medium`); None = agy default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Absolute working directory for the agent; None = daemon default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

impl AgyLaunchOptions {
    /// Reject values that could not have come from the launch UI.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(m) = &self.model {
            if !is_valid_model_id(m) {
                return Err("invalid model id".into());
            }
        }
        if let Some(d) = &self.working_dir {
            let p = Path::new(d);
            if !p.is_absolute() {
                return Err(format!("working directory must be an absolute path: {d}"));
            }
            if !p.is_dir() {
                return Err(format!(
                    "working directory does not exist or is not a directory: {d}"
                ));
            }
        }
        Ok(())
    }

    /// `agy` arguments for these options.
    pub fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(f) = self.execution_mode.flag() {
            args.push(f.to_string());
        }
        if let Some(f) = self.permission_mode.flag() {
            args.push(f.to_string());
        }
        if self.sandbox {
            args.push("--sandbox".to_string());
        }
        if let Some(m) = self.model.as_ref().filter(|m| is_valid_model_id(m)) {
            args.push("--model".into());
            args.push(m.clone());
        }
        args
    }

    /// Read options from a session's `agent_config`; anything unparsable means defaults.
    /// Also supports legacy payload shapes with `permission_mode` holding execution modes.
    pub fn from_agent_config(cfg: Option<&serde_json::Value>) -> Self {
        let Some(v) = cfg else {
            return Self::default();
        };

        // If direct deserialization succeeds, use it
        if let Ok(opts) = serde_json::from_value::<AgyLaunchOptions>(v.clone()) {
            return opts;
        }

        // Backward compatibility parsing
        let mut opts = AgyLaunchOptions::default();

        if let Some(mode_val) = v.get("execution_mode").and_then(|s| s.as_str()) {
            opts.execution_mode = match mode_val {
                "accept-edits" | "accept_edits" => AgyExecutionMode::AcceptEdits,
                "plan" => AgyExecutionMode::Plan,
                _ => AgyExecutionMode::Default,
            };
        } else if let Some(perm_val) = v.get("permission_mode").and_then(|s| s.as_str()) {
            match perm_val {
                "accept-edits" | "accept_edits" => {
                    opts.execution_mode = AgyExecutionMode::AcceptEdits
                }
                "plan" => opts.execution_mode = AgyExecutionMode::Plan,
                "sandbox" => opts.sandbox = true,
                "dangerously-skip-permissions" | "dangerously_skip_permissions" => {
                    opts.permission_mode = AgyPermissionMode::DangerouslySkipPermissions;
                }
                _ => {}
            }
        }

        if let Some(perm_val) = v.get("permission_mode").and_then(|s| s.as_str()) {
            if perm_val.contains("dangerously") {
                opts.permission_mode = AgyPermissionMode::DangerouslySkipPermissions;
            } else if perm_val == "normal" {
                opts.permission_mode = AgyPermissionMode::Normal;
            }
        }

        if let Some(sb) = v.get("sandbox").and_then(|b| b.as_bool()) {
            opts.sandbox = sb;
        }

        if let Some(m) = v.get("model").and_then(|s| s.as_str()) {
            if is_valid_model_id(m) {
                opts.model = Some(m.to_string());
            }
        }

        if let Some(d) = v.get("working_dir").and_then(|s| s.as_str()) {
            opts.working_dir = Some(d.to_string());
        }

        opts
    }
}

fn is_valid_model_id(m: &str) -> bool {
    !m.is_empty()
        && m.len() <= 128
        && !m.starts_with('-')
        && m.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Execution modes supported by the installed agy (cached for the process).
pub fn supported_execution_modes() -> Vec<AgyExecutionMode> {
    static CACHE: OnceLock<Vec<AgyExecutionMode>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            crate::agy_auth::find_agy_binary()
                .and_then(|bin| usage_strings_from_binary(&bin))
                .map(|usage| AgyExecutionMode::supported_in_usage(&usage))
                .unwrap_or_else(|| AgyExecutionMode::ALL.to_vec())
        })
        .clone()
}

/// Permission modes supported by the installed agy (cached for the process).
pub fn supported_permission_modes() -> Vec<AgyPermissionMode> {
    static CACHE: OnceLock<Vec<AgyPermissionMode>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            crate::agy_auth::find_agy_binary()
                .and_then(|bin| usage_strings_from_binary(&bin))
                .map(|usage| AgyPermissionMode::supported_in_usage(&usage))
                .unwrap_or_else(|| AgyPermissionMode::ALL.to_vec())
        })
        .clone()
}

/// Extract the flag usage strings that matter for capability detection.
fn usage_strings_from_binary(bin: &Path) -> Option<String> {
    let bytes = std::fs::read(bin).ok()?;
    let re = regex::bytes::Regex::new(
        r"Set the agent execution mode for this session \([a-z%, -]{1,80}\)|'(?:accept-edits|plan)': |Run in a sandbox with terminal restrictions|dangerously-skip-permissions|Auto-approve all tool permission requests",
    )
    .ok()?;
    let found: Vec<String> = re
        .find_iter(&bytes)
        .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
        .collect();
    Some(found.join("\n"))
}

/// A model offered by `agy models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgyModel {
    pub id: String,
    pub name: String,
}

/// Parse `agy models` output (`<id>\t<display name>` per line).
pub fn parse_models(output: &str) -> Vec<AgyModel> {
    output
        .lines()
        .filter_map(|l| {
            let (id, name) = l.split_once('\t')?;
            let id = id.trim();
            is_valid_model_id(id).then(|| AgyModel {
                id: id.to_string(),
                name: name.trim().to_string(),
            })
        })
        .collect()
}

/// List the models available to an account by running `agy models` inside the
/// account's isolated profile (same preparation as a session launch; the
/// credential is validated first and never passed on the command line).
#[cfg(unix)]
pub fn list_models(
    account_id: &Id,
    cred_ref: &str,
    timeout: Duration,
) -> Result<Vec<AgyModel>, String> {
    use std::io::Read;
    let envs = crate::agy_auth::prepare_profile(account_id, cred_ref).map_err(|e| {
        e.to_string()
            .split('\n')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    })?;
    let mut cmd = std::process::Command::new(crate::adapter::resolve_real_antigravity_bin());
    cmd.arg("models")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    for k in crate::agy_auth::AMBIENT_AUTH_ENV {
        cmd.env_remove(k);
    }
    cmd.envs(envs);
    if let Some(home) = std::env::var_os("HOME") {
        cmd.current_dir(home);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not run agy: {}", e.kind()))?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("agy models timed out".into());
            }
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    let _ = crate::agy_auth::sync_profile_back(account_id, cred_ref);
    let models = parse_models(&out);
    if models.is_empty() {
        return Err("agy returned no models".into());
    }
    Ok(models)
}

/// Resolve a user-entered directory (absolute, relative to `cwd`, or `~/…`)
/// to an absolute, normalised path. Does not require the path to exist.
pub fn resolve_dir_input(input: &str, cwd: &Path, home: &Path) -> PathBuf {
    let input = input.trim();
    let raw = if input == "~" {
        home.to_path_buf()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else if input.is_empty() {
        cwd.to_path_buf()
    } else {
        let p = Path::new(input);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = "  --dangerously-skip-permissions  Auto-approve all tool permission requests without prompting\n  --mode   Set the agent execution mode for this session (accept-edits, plan)\n  --sandbox   Run in a sandbox with terminal restrictions enabled";

    #[test]
    fn modes_follow_usage_text() {
        assert_eq!(
            AgyExecutionMode::supported_in_usage(HELP),
            AgyExecutionMode::ALL.to_vec()
        );
        assert_eq!(
            AgyExecutionMode::supported_in_usage(""),
            vec![AgyExecutionMode::Default]
        );
        assert_eq!(
            AgyPermissionMode::supported_in_usage(HELP),
            AgyPermissionMode::ALL.to_vec()
        );
        assert_eq!(
            AgyPermissionMode::supported_in_usage(""),
            vec![AgyPermissionMode::Normal]
        );

        let only_plan = "Set the agent execution mode for this session (plan)";
        assert_eq!(
            AgyExecutionMode::supported_in_usage(only_plan),
            vec![AgyExecutionMode::Default, AgyExecutionMode::Plan]
        );
    }

    #[test]
    fn args_per_mode_and_default_is_safe() {
        assert!(AgyLaunchOptions::default().args().is_empty());
        assert_eq!(AgyExecutionMode::default(), AgyExecutionMode::Default);
        assert_eq!(AgyPermissionMode::default(), AgyPermissionMode::Normal);

        assert_eq!(AgyExecutionMode::Default.flag(), None);
        assert_eq!(
            AgyExecutionMode::AcceptEdits.flag(),
            Some("--mode=accept-edits")
        );
        assert_eq!(AgyExecutionMode::Plan.flag(), Some("--mode=plan"));

        assert_eq!(AgyPermissionMode::Normal.flag(), None);
        assert_eq!(
            AgyPermissionMode::DangerouslySkipPermissions.flag(),
            Some("--dangerously-skip-permissions")
        );

        let opts = AgyLaunchOptions {
            execution_mode: AgyExecutionMode::AcceptEdits,
            permission_mode: AgyPermissionMode::Normal,
            sandbox: false,
            model: None,
            working_dir: None,
        };
        assert_eq!(opts.args(), vec!["--mode=accept-edits"]);

        let dangerous = AgyLaunchOptions {
            execution_mode: AgyExecutionMode::Default,
            permission_mode: AgyPermissionMode::DangerouslySkipPermissions,
            sandbox: false,
            model: None,
            working_dir: None,
        };
        assert_eq!(dangerous.args(), vec!["--dangerously-skip-permissions"]);

        let plan_with_model = AgyLaunchOptions {
            execution_mode: AgyExecutionMode::Plan,
            permission_mode: AgyPermissionMode::Normal,
            sandbox: false,
            model: Some("gemini-3.8-flash-medium".into()),
            working_dir: None,
        };
        assert_eq!(
            plan_with_model.args(),
            vec!["--mode=plan", "--model", "gemini-3.8-flash-medium"]
        );

        // Unparsable config never turns into a dangerous launch
        assert_eq!(
            AgyLaunchOptions::from_agent_config(Some(
                &serde_json::json!({"execution_mode": "bogus"})
            )),
            AgyLaunchOptions::default()
        );
        assert_eq!(
            AgyLaunchOptions::from_agent_config(None),
            AgyLaunchOptions::default()
        );
    }

    #[test]
    fn validation_rejects_bad_models_and_dirs() {
        let bad = AgyLaunchOptions {
            model: Some("--dangerously-skip-permissions".into()),
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        assert!(bad.args().is_empty());
        let rel = AgyLaunchOptions {
            working_dir: Some("relative/dir".into()),
            ..Default::default()
        };
        assert!(rel.validate().is_err());
        let missing = AgyLaunchOptions {
            working_dir: Some("/definitely/not/here".into()),
            ..Default::default()
        };
        assert!(missing.validate().is_err());
        let ok = AgyLaunchOptions {
            working_dir: Some(std::env::temp_dir().display().to_string()),
            ..Default::default()
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn parse_models_output() {
        let m = parse_models("gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\nFetching...\nclaude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\n");
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].id, "gemini-3.8-flash-medium");
        assert_eq!(m[1].name, "Claude Sonnet 4.6 (Thinking)");
    }

    #[test]
    fn resolve_dir_variants() {
        let cwd = Path::new("/work/here");
        let home = Path::new("/home/u");
        assert_eq!(
            resolve_dir_input("~/projects/", cwd, home),
            PathBuf::from("/home/u/projects")
        );
        assert_eq!(resolve_dir_input("~", cwd, home), PathBuf::from("/home/u"));
        assert_eq!(
            resolve_dir_input("/abs/x", cwd, home),
            PathBuf::from("/abs/x")
        );
        assert_eq!(
            resolve_dir_input("sub/../other", cwd, home),
            PathBuf::from("/work/here/other")
        );
        assert_eq!(
            resolve_dir_input("", cwd, home),
            PathBuf::from("/work/here")
        );
    }
}
