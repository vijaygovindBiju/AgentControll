//! Claude Code agent adapter.
//!
//! Supports both:
//! - **Structured mode** (preferred, high confidence): NDJSON streaming over stdio
//!   via `--output-format stream-json --input-format stream-json`.
//! - **PTY fallback mode** (mandatory, low confidence): pseudo-terminal execution
//!   using the `GenericPtyAdapter` configured with Claude CLI patterns.
//!
//! Enforces workspace isolation (sets `cwd` to `workspace_path`), credential
//! resolution via `credential_ref` without secret leakage, and bi-directional
//! translation between Claude events/commands and Agent Control types.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::adapter::pty::{GenericPtyAdapter, PtyConfig, PtyPatternKind};
use crate::adapter::AdapterHandle;
use crate::types::{
    AccountSwitchMode, AdapterEvent, AgentCommand, Confidence, ProviderCapabilities, SessionContext,
};

/// Operating mode for the Claude Code adapter.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ClaudeMode {
    /// Structured mode with NDJSON communication (High confidence).
    #[default]
    Structured,
    /// Pseudo-terminal execution with regex pattern matching (Low confidence).
    Pty,
}

/// Configuration for the Claude Code adapter.
#[derive(Debug, Clone)]
pub struct ClaudeConfig {
    /// Path or command name for the Claude executable.
    pub executable: String,
    /// Arguments to pass to Claude Code.
    pub args: Vec<String>,
    /// Environment variables (e.g. non-secret settings).
    pub envs: HashMap<String, String>,
    /// Operating mode.
    pub mode: ClaudeMode,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        let executable = std::env::var("CLAUDE_CODE_BIN").unwrap_or_else(|_| "claude".into());
        Self {
            executable,
            args: vec![
                "--output-format".into(),
                "stream-json".into(),
                "--input-format".into(),
                "stream-json".into(),
            ],
            envs: HashMap::new(),
            mode: ClaudeMode::Structured,
        }
    }
}

impl ClaudeConfig {
    pub fn new(executable: impl Into<String>) -> Self {
        Self {
            executable: executable.into(),
            args: vec![
                "--output-format".into(),
                "stream-json".into(),
                "--input-format".into(),
                "stream-json".into(),
            ],
            envs: HashMap::new(),
            mode: ClaudeMode::Structured,
        }
    }

    /// Construct configuration with custom arguments (useful for tests or wrapper scripts).
    pub fn custom(executable: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            executable: executable.into(),
            args,
            envs: HashMap::new(),
            mode: ClaudeMode::Structured,
        }
    }

    pub fn with_mode(mut self, mode: ClaudeMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for a in args {
            self.args.push(a.into());
        }
        self
    }

    pub fn with_env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.envs.insert(k.into(), v.into());
        self
    }
}

/// Parse a single line of Claude Code output into an `AdapterEvent`.
pub fn parse_claude_event(line: &str) -> Option<AdapterEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match event_type {
            "system" | "ready" => {
                let subtype = val.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
                if event_type == "ready" || subtype == "ready" || subtype == "init" {
                    Some(AdapterEvent::Ready)
                } else {
                    Some(AdapterEvent::OutputChunk {
                        text: format!("[system] {}", trimmed),
                        confidence: Confidence::High,
                    })
                }
            }
            "assistant" => {
                // Check for nested message.content array
                if let Some(content) = val
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array())
                {
                    let texts: Vec<String> = content
                        .iter()
                        .filter_map(|block| {
                            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                block
                                    .get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .collect();
                    if !texts.is_empty() {
                        return Some(AdapterEvent::OutputChunk {
                            text: texts.join("\n"),
                            confidence: Confidence::High,
                        });
                    }
                }
                let text = val
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or(trimmed)
                    .to_string();
                Some(AdapterEvent::OutputChunk {
                    text,
                    confidence: Confidence::High,
                })
            }
            "content_block_delta" => {
                let text = val
                    .get("delta")
                    .and_then(|d| d.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(AdapterEvent::OutputChunk {
                    text,
                    confidence: Confidence::High,
                })
            }
            "output" | "text" => {
                let text = val
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or(trimmed)
                    .to_string();
                Some(AdapterEvent::OutputChunk {
                    text,
                    confidence: Confidence::High,
                })
            }
            "progress" | "status" => {
                let text = val
                    .get("text")
                    .or_else(|| val.get("status"))
                    .and_then(|t| t.as_str())
                    .unwrap_or(trimmed)
                    .to_string();
                Some(AdapterEvent::OutputChunk {
                    text: format!("[progress] {}", text),
                    confidence: Confidence::High,
                })
            }
            "approval_requested" => {
                let tool_name = val
                    .get("tool_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let prompt = val
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or(trimmed)
                    .to_string();
                Some(AdapterEvent::ApprovalRequested { tool_name, prompt })
            }
            "tool_use" | "action" => {
                let tool_name = val
                    .get("name")
                    .or_else(|| val.get("tool"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let prompt = if let Some(input) = val.get("input").or_else(|| val.get("args")) {
                    format!("Run tool '{}' with input: {}", tool_name, input)
                } else {
                    format!("Run tool '{}'", tool_name)
                };
                Some(AdapterEvent::ApprovalRequested { tool_name, prompt })
            }
            "question" | "ask" => {
                let prompt = val
                    .get("prompt")
                    .or_else(|| val.get("question"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(trimmed)
                    .to_string();
                Some(AdapterEvent::QuestionRaised { prompt })
            }
            "rate_limit" => {
                let back_off_secs = val.get("back_off_secs").and_then(|v| v.as_u64());
                Some(AdapterEvent::RateLimitSignal { back_off_secs })
            }
            "error" => {
                // Check if this error represents rate-limiting
                let err_type = val
                    .get("error")
                    .and_then(|e| e.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let msg = val
                    .get("message")
                    .or_else(|| val.get("error").and_then(|e| e.get("message")))
                    .and_then(|m| m.as_str())
                    .unwrap_or(trimmed);

                if err_type == "rate_limit_error" || msg.contains("429") || msg.contains("rate limit") {
                    let back_off_secs = val.get("back_off_secs").and_then(|v| v.as_u64()).or(Some(30));
                    Some(AdapterEvent::RateLimitSignal { back_off_secs })
                } else {
                    Some(AdapterEvent::OutputChunk {
                        text: format!("[error] {}", msg),
                        confidence: Confidence::High,
                    })
                }
            }
            "completed" | "result" => {
                let summary = val
                    .get("summary")
                    .or_else(|| val.get("result"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Some(AdapterEvent::Completed { summary })
            }
            _ => {
                // Unknown JSON type - surface as High-confidence chunk
                Some(AdapterEvent::OutputChunk {
                    text: trimmed.to_string(),
                    confidence: Confidence::High,
                })
            }
        }
    } else {
        // Non-JSON output line - surface as Low-confidence chunk
        Some(AdapterEvent::OutputChunk {
            text: trimmed.to_string(),
            confidence: Confidence::Low,
        })
    }
}

/// Format an `AgentCommand` into Claude Code NDJSON format.
pub fn format_claude_command(cmd: &AgentCommand) -> Option<String> {
    match cmd {
        AgentCommand::Respond { allow, response } => {
            let json_obj = json!({
                "type": "response",
                "allow": allow,
                "response": response,
            });
            Some(format!("{}\n", json_obj))
        }
        AgentCommand::Steer { message } => {
            let json_obj = json!({
                "type": "steer",
                "message": message,
            });
            Some(format!("{}\n", json_obj))
        }
        AgentCommand::Stop { reason } => {
            let json_obj = json!({
                "type": "stop",
                "reason": reason,
            });
            Some(format!("{}\n", json_obj))
        }
        AgentCommand::Snapshot => {
            let json_obj = json!({
                "type": "snapshot",
            });
            Some(format!("{}\n", json_obj))
        }
        AgentCommand::Input { data } => {
            let json_obj = json!({
                "type": "user_input",
                "message": data,
            });
            Some(format!("{}\n", json_obj))
        }
        AgentCommand::Resize { .. }
        | AgentCommand::Pause
        | AgentCommand::Resume
        | AgentCommand::Start { .. }
        | AgentCommand::SwitchAccount { .. } => None,
    }
}

/// Claude Code adapter implementation.
pub struct ClaudeAdapter;

impl ClaudeAdapter {
    /// Provider capabilities for Claude Code.
    /// Switching accounts requires stopping the current process and restarting (session hand-off).
    pub fn capabilities() -> ProviderCapabilities {
        ProviderCapabilities::new(
            AccountSwitchMode::RequiresRestart,
            true, // supports session resume via task & branch
            true, // supports snapshot restore
        )
    }

    /// Spawns Claude Code inside the assigned workspace according to the configuration.
    pub fn spawn(
        ctx: &SessionContext,
        config: ClaudeConfig,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        // 1. Enforce workspace isolation
        if let Some(ref ws) = ctx.workspace_path {
            let p = PathBuf::from(ws);
            if !p.exists() || !p.is_dir() {
                bail!("Workspace directory does not exist: {}", ws);
            }
        }

        // 2. Dispatch by mode
        match config.mode {
            ClaudeMode::Pty => Self::spawn_pty(ctx, config, event_tx),
            ClaudeMode::Structured => Self::spawn_structured(ctx, config, event_tx),
        }
    }

    /// Spawns Claude Code in structured NDJSON mode.
    fn spawn_structured(
        ctx: &SessionContext,
        config: ClaudeConfig,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        let mut cmd = Command::new(&config.executable);
        cmd.args(&config.args);

        // Workspace directory pinning
        if let Some(ref ws) = ctx.workspace_path {
            cmd.current_dir(ws);
        }

        // Environment variables
        for (k, v) in &config.envs {
            cmd.env(k, v);
        }

        // Resolve credential ref if available into child environment without leaking
        if let Some(ref cred_ref) = ctx.credential_ref {
            // E.g. "env:ANTHROPIC_API_KEY", direct env var name, or key alias
            let key = if let Some(stripped) = cred_ref.strip_prefix("env:") {
                std::env::var(stripped).unwrap_or_else(|_| cred_ref.clone())
            } else if let Ok(val) = std::env::var(cred_ref) {
                val
            } else if let Ok(val) = std::env::var("ANTHROPIC_API_KEY") {
                val
            } else {
                cred_ref.clone()
            };
            cmd.env("ANTHROPIC_API_KEY", key);
        }

        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().with_context(|| {
            format!("failed to spawn Claude executable '{}'", config.executable)
        })?;

        let child_pid = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to take child stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to take child stderr"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to take child stdin"))?;

        let stdin = tokio::sync::Mutex::new(stdin);
        let stdin = std::sync::Arc::new(stdin);

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<AgentCommand>(64);
        let event_tx_cmd = event_tx.clone();
        let stdin_cmd = stdin.clone();

        // ── Command loop ──────────────────────────────────────────────────────
        let cmd_task = tokio::spawn(async move {
            while let Some(command) = cmd_rx.recv().await {
                match &command {
                    AgentCommand::Pause => {
                        #[cfg(unix)]
                        if let Some(pid) = child_pid {
                            unsafe {
                                libc::kill(pid as i32, libc::SIGSTOP);
                            }
                        }
                        let _ = event_tx_cmd.send(AdapterEvent::Paused).await;
                    }
                    AgentCommand::Resume => {
                        #[cfg(unix)]
                        if let Some(pid) = child_pid {
                            unsafe {
                                libc::kill(pid as i32, libc::SIGCONT);
                            }
                        }
                        let _ = event_tx_cmd.send(AdapterEvent::Resumed).await;
                    }
                    AgentCommand::Stop { .. } => {
                        // Write stop command to stdin first
                        if let Some(line) = format_claude_command(&command) {
                            let mut lock = stdin_cmd.lock().await;
                            let _ = lock.write_all(line.as_bytes()).await;
                            let _ = lock.flush().await;
                        }
                        // Send SIGTERM
                        #[cfg(unix)]
                        if let Some(pid) = child_pid {
                            unsafe {
                                libc::kill(pid as i32, libc::SIGTERM);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        #[cfg(unix)]
                        if let Some(pid) = child_pid {
                            unsafe {
                                libc::kill(pid as i32, libc::SIGKILL);
                            }
                        }
                    }
                    other => {
                        if let Some(line) = format_claude_command(other) {
                            let mut lock = stdin_cmd.lock().await;
                            let _ = lock.write_all(line.as_bytes()).await;
                            let _ = lock.flush().await;
                        }
                    }
                }
            }
        });

        // ── Stdout reader loop ────────────────────────────────────────────────
        let event_tx_stdout = event_tx.clone();
        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            // Emit Ready immediately on successful startup
            let _ = event_tx_stdout.send(AdapterEvent::Ready).await;

            while let Ok(Some(line)) = reader.next_line().await {
                if let Some(event) = parse_claude_event(&line) {
                    if event_tx_stdout.send(event).await.is_err() {
                        break;
                    }
                }
            }
        });

        // ── Stderr reader loop ────────────────────────────────────────────────
        let event_tx_stderr = event_tx.clone();
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let _ = event_tx_stderr
                        .send(AdapterEvent::OutputChunk {
                            text: format!("[stderr] {}", trimmed),
                            confidence: Confidence::High,
                        })
                        .await;
                }
            }
        });

        // ── Child process monitor task ────────────────────────────────────────
        let event_tx_wait = event_tx;
        let monitor_task = tokio::spawn(async move {
            let status = child.wait().await;
            match status {
                Ok(s) if s.success() => {
                    let _ = event_tx_wait
                        .send(AdapterEvent::Completed {
                            summary: Some("Claude completed successfully".into()),
                        })
                        .await;
                }
                Ok(s) => {
                    let code = s.code();
                    let _ = event_tx_wait
                        .send(AdapterEvent::Crashed {
                            exit_code: code,
                            reason: Some(format!("Claude process exited with code {:?}", code)),
                        })
                        .await;
                }
                Err(e) => {
                    let _ = event_tx_wait
                        .send(AdapterEvent::Crashed {
                            exit_code: None,
                            reason: Some(format!("Error waiting for Claude process: {e}")),
                        })
                        .await;
                }
            }
        });

        // Drop guard
        let drop_guard = Box::new(ClaudeDropGuard {
            child_pid,
            cmd_task: cmd_task.abort_handle(),
            stdout_task: stdout_task.abort_handle(),
            stderr_task: stderr_task.abort_handle(),
            monitor_task: monitor_task.abort_handle(),
        });

        Ok(AdapterHandle::with_cmd_tx(drop_guard, cmd_tx))
    }

    /// Spawns Claude Code in PTY fallback mode.
    fn spawn_pty(
        ctx: &SessionContext,
        config: ClaudeConfig,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        let mut pty_config = PtyConfig::new(&config.executable)
            .with_args(config.args)
            .with_pattern(
                r"(?i)(Allow Claude to run|Do you want to run tool|Execute command:)\s*([^\n\r]+)",
                PtyPatternKind::ApprovalRequested {
                    tool_name: Some("tool".into()),
                },
            )?
            .with_pattern(
                r"(?i)(Claude asks:|\?\s*$)",
                PtyPatternKind::QuestionRaised,
            )?
            .with_pattern(
                r"(?i)(rate limit exceeded|429 too many requests|quota exhausted)",
                PtyPatternKind::RateLimitSignal {
                    back_off_secs: Some(30),
                },
            )?
            .with_pattern(
                r"(?i)(Task completed|Execution finished)",
                PtyPatternKind::Completed {
                    summary: Some("Claude finished in PTY mode".into()),
                },
            )?;

        for (k, v) in config.envs {
            pty_config = pty_config.with_env(k, v);
        }

        GenericPtyAdapter::spawn(ctx, pty_config, event_tx)
    }
}

struct ClaudeDropGuard {
    child_pid: Option<u32>,
    cmd_task: tokio::task::AbortHandle,
    stdout_task: tokio::task::AbortHandle,
    stderr_task: tokio::task::AbortHandle,
    monitor_task: tokio::task::AbortHandle,
}

impl Drop for ClaudeDropGuard {
    fn drop(&mut self) {
        self.cmd_task.abort();
        self.stdout_task.abort();
        self.stderr_task.abort();
        self.monitor_task.abort();
        #[cfg(unix)]
        if let Some(pid) = self.child_pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
}

/// Factory for creating Claude Code adapters.
pub struct ClaudeAdapterFactory {
    config: ClaudeConfig,
}

impl ClaudeAdapterFactory {
    pub fn new(config: ClaudeConfig) -> Self {
        Self { config }
    }
}

impl Default for ClaudeAdapterFactory {
    fn default() -> Self {
        Self {
            config: ClaudeConfig::default(),
        }
    }
}

impl crate::adapter::AdapterFactory for ClaudeAdapterFactory {
    fn capabilities(&self, _agent_type: &str) -> ProviderCapabilities {
        ClaudeAdapter::capabilities()
    }

    fn create(
        &mut self,
        ctx: SessionContext,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        ClaudeAdapter::spawn(&ctx, self.config.clone(), event_tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Id;

    #[test]
    fn test_claude_event_parsing_ready() {
        let evt = parse_claude_event(r#"{"type": "ready"}"#).unwrap();
        assert!(matches!(evt, AdapterEvent::Ready));

        let evt2 = parse_claude_event(r#"{"type": "system", "subtype": "init"}"#).unwrap();
        assert!(matches!(evt2, AdapterEvent::Ready));
    }

    #[test]
    fn test_claude_event_parsing_output() {
        let evt = parse_claude_event(r#"{"type": "output", "text": "Analyzing repo..."}"#).unwrap();
        if let AdapterEvent::OutputChunk { text, confidence } = evt {
            assert_eq!(text, "Analyzing repo...");
            assert_eq!(confidence, Confidence::High);
        } else {
            panic!("expected OutputChunk");
        }

        // Assistant with message.content
        let line = r#"{"type": "assistant", "message": {"content": [{"type": "text", "text": "Hello human"}]}}"#;
        let evt2 = parse_claude_event(line).unwrap();
        if let AdapterEvent::OutputChunk { text, confidence } = evt2 {
            assert_eq!(text, "Hello human");
            assert_eq!(confidence, Confidence::High);
        } else {
            panic!("expected OutputChunk");
        }
    }

    #[test]
    fn test_claude_event_parsing_progress() {
        let evt = parse_claude_event(r#"{"type": "progress", "text": "Running tests"}"#).unwrap();
        if let AdapterEvent::OutputChunk { text, confidence } = evt {
            assert!(text.contains("[progress] Running tests"));
            assert_eq!(confidence, Confidence::High);
        } else {
            panic!("expected OutputChunk");
        }
    }

    #[test]
    fn test_claude_event_parsing_question() {
        let evt = parse_claude_event(r#"{"type": "question", "prompt": "Which file?"}"#).unwrap();
        if let AdapterEvent::QuestionRaised { prompt } = evt {
            assert_eq!(prompt, "Which file?");
        } else {
            panic!("expected QuestionRaised");
        }
    }

    #[test]
    fn test_claude_event_parsing_approval() {
        let evt = parse_claude_event(
            r#"{"type": "approval_requested", "tool_name": "Bash", "prompt": "Run git status"}"#,
        )
        .unwrap();
        if let AdapterEvent::ApprovalRequested { tool_name, prompt } = evt {
            assert_eq!(tool_name, "Bash");
            assert_eq!(prompt, "Run git status");
        } else {
            panic!("expected ApprovalRequested");
        }

        // Action/tool_use format
        let evt2 =
            parse_claude_event(r#"{"type": "tool_use", "name": "Edit", "input": {"file": "a.rs"}}"#)
                .unwrap();
        if let AdapterEvent::ApprovalRequested { tool_name, prompt } = evt2 {
            assert_eq!(tool_name, "Edit");
            assert!(prompt.contains("a.rs"));
        } else {
            panic!("expected ApprovalRequested");
        }
    }

    #[test]
    fn test_claude_event_parsing_rate_limit() {
        let evt = parse_claude_event(r#"{"type": "rate_limit", "back_off_secs": 45}"#).unwrap();
        if let AdapterEvent::RateLimitSignal { back_off_secs } = evt {
            assert_eq!(back_off_secs, Some(45));
        } else {
            panic!("expected RateLimitSignal");
        }

        // Error variant with rate limit
        let evt2 = parse_claude_event(
            r#"{"type": "error", "error": {"type": "rate_limit_error"}, "back_off_secs": 30}"#,
        )
        .unwrap();
        assert!(matches!(evt2, AdapterEvent::RateLimitSignal { back_off_secs: Some(30) }));
    }

    #[test]
    fn test_claude_event_parsing_completed() {
        let evt = parse_claude_event(r#"{"type": "completed", "summary": "Done everything"}"#).unwrap();
        if let AdapterEvent::Completed { summary } = evt {
            assert_eq!(summary, Some("Done everything".into()));
        } else {
            panic!("expected Completed");
        }
    }

    #[test]
    fn test_claude_event_parsing_error() {
        let evt = parse_claude_event(r#"{"type": "error", "message": "Syntax error on line 5"}"#).unwrap();
        if let AdapterEvent::OutputChunk { text, confidence } = evt {
            assert!(text.contains("[error] Syntax error"));
            assert_eq!(confidence, Confidence::High);
        } else {
            panic!("expected OutputChunk");
        }
    }

    #[test]
    fn test_claude_event_parsing_malformed_and_unknown() {
        // Plain text banner
        let evt = parse_claude_event("Welcome to Claude Code CLI").unwrap();
        if let AdapterEvent::OutputChunk { text, confidence } = evt {
            assert_eq!(text, "Welcome to Claude Code CLI");
            assert_eq!(confidence, Confidence::Low);
        } else {
            panic!("expected OutputChunk with Low confidence");
        }

        // Empty line
        assert!(parse_claude_event("   ").is_none());
    }

    #[test]
    fn test_claude_command_formatting() {
        let respond_allow = AgentCommand::Respond {
            allow: true,
            response: Some("looks good".into()),
        };
        let formatted = format_claude_command(&respond_allow).unwrap();
        assert!(formatted.contains("\"type\":\"response\""));
        assert!(formatted.contains("\"allow\":true"));
        assert!(formatted.contains("\"response\":\"looks good\""));

        let respond_deny = AgentCommand::Respond {
            allow: false,
            response: None,
        };
        let formatted2 = format_claude_command(&respond_deny).unwrap();
        assert!(formatted2.contains("\"allow\":false"));

        let steer = AgentCommand::Steer {
            message: "Refactor function x".into(),
        };
        let formatted3 = format_claude_command(&steer).unwrap();
        assert!(formatted3.contains("\"type\":\"steer\""));
        assert!(formatted3.contains("Refactor function x"));
    }

    #[tokio::test]
    async fn test_claude_adapter_spawn_and_lifecycle() {
        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "claude test".into(), "claude-code".into());

        // Use sh to simulate Claude emitting structured NDJSON
        let script = r#"
printf '{"type": "ready"}\n'
printf '{"type": "output", "text": "Claude is working"}\n'
printf '{"type": "completed", "summary": "Finished task"}\n'
"#;
        let config = ClaudeConfig::custom("sh", vec!["-c".to_string(), script.to_string()]);

        let _handle = ClaudeAdapter::spawn(&ctx, config, tx).unwrap();

        let mut got_ready = false;
        let mut got_output = false;
        let mut got_completed = false;

        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                match evt {
                    AdapterEvent::Ready => got_ready = true,
                    AdapterEvent::OutputChunk { text, .. } => {
                        if text.contains("Claude is working") {
                            got_output = true;
                        }
                    }
                    AdapterEvent::Completed { .. } => {
                        got_completed = true;
                        break;
                    }
                    _ => {}
                }
            }
        }

        assert!(got_ready, "should emit Ready");
        assert!(got_output, "should emit OutputChunk");
        assert!(got_completed, "should emit Completed");
    }

    #[tokio::test]
    async fn test_claude_workspace_isolation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ws_path = temp_dir.path().to_str().unwrap().to_string();

        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "ws isolation".into(), "claude-code".into())
            .with_workspace(&ws_path);

        let script = "pwd";
        let config = ClaudeConfig::custom("sh", vec!["-c".to_string(), script.to_string()]);

        let _handle = ClaudeAdapter::spawn(&ctx, config, tx).unwrap();

        let mut pwd_output = String::new();
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                match evt {
                    AdapterEvent::OutputChunk { text, .. } => {
                        pwd_output.push_str(&text);
                    }
                    AdapterEvent::Completed { .. } => {
                        break;
                    }
                    _ => {}
                }
            }
        }

        assert!(
            pwd_output.contains(temp_dir.path().file_name().unwrap().to_str().unwrap()),
            "Claude must execute inside workspace directory"
        );
    }
}
