//! Adapter contract and built-in adapters.
//!
//! An adapter wraps one agent type and translates its I/O into `AdapterEvent`s.
//! In Phase 1 only the `MockAdapter` is implemented.

pub mod claude;
pub mod mock;
pub mod pty;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::warn;

use crate::types::{AdapterEvent, Id, ProviderCapabilities, SessionContext};

/// Handle to a running adapter instance. Dropping it terminates the adapter.
pub struct AdapterHandle {
    pub(crate) _drop_guard: Box<dyn std::any::Any + Send>,
    pub(crate) cmd_tx: Option<tokio::sync::mpsc::Sender<crate::types::AgentCommand>>,
}

impl AdapterHandle {
    pub fn new(drop_guard: Box<dyn std::any::Any + Send>) -> Self {
        Self {
            _drop_guard: drop_guard,
            cmd_tx: None,
        }
    }

    pub fn with_cmd_tx(
        drop_guard: Box<dyn std::any::Any + Send>,
        cmd_tx: tokio::sync::mpsc::Sender<crate::types::AgentCommand>,
    ) -> Self {
        Self {
            _drop_guard: drop_guard,
            cmd_tx: Some(cmd_tx),
        }
    }

    /// Send a command to the adapter.
    pub fn send_command(&mut self, cmd: crate::types::AgentCommand) -> Result<()> {
        if let Some(tx) = &self.cmd_tx {
            let _ = tx.try_send(cmd);
        }
        Ok(())
    }

    /// Request the adapter to stop. The adapter may not stop immediately.
    pub fn stop(&mut self) {
        // Drop guard will clean up when this handle is dropped;
        // or a Stop command can be sent via cmd_tx.
    }
}

/// Factory for creating adapter instances.
///
/// Injected into the `SessionManager` so that tests can provide a mock factory,
/// or production can provide multi-agent dispatch (Claude Code, Generic PTY, Mock).
pub trait AdapterFactory {
    /// Return capabilities for a given agent type.
    fn capabilities(&self, agent_type: &str) -> ProviderCapabilities;

    fn create(
        &mut self,
        ctx: SessionContext,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle>;

    /// Validate that a session can be launched (e.g. credentials are usable)
    /// without spawning anything. Used before destructive steps such as
    /// stopping a predecessor during hand-off.
    fn preflight(&self, _ctx: &SessionContext) -> Result<()> {
        Ok(())
    }

    fn create_simple(
        &mut self,
        session_id: Id,
        task_description: String,
        agent_type: String,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        self.create(
            SessionContext::new(session_id, task_description, agent_type),
            event_tx,
        )
    }
}

/// Default composite adapter factory that routes according to `agent_type`.
pub struct CompositeAdapterFactory {
    pub claude_config: claude::ClaudeConfig,
    pub pty_config: pty::PtyConfig,
    pub mock_factory: Option<mock::MockAdapterFactory>,
}

impl CompositeAdapterFactory {
    pub fn new() -> Self {
        Self {
            claude_config: claude::ClaudeConfig::default(),
            pty_config: pty::PtyConfig::default(),
            mock_factory: None,
        }
    }

    pub fn with_claude_config(mut self, cfg: claude::ClaudeConfig) -> Self {
        self.claude_config = cfg;
        self
    }

    pub fn with_pty_config(mut self, cfg: pty::PtyConfig) -> Self {
        self.pty_config = cfg;
        self
    }

    pub fn with_mock_factory(mut self, factory: mock::MockAdapterFactory) -> Self {
        self.mock_factory = Some(factory);
        self
    }
}

impl Default for CompositeAdapterFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl AdapterFactory for CompositeAdapterFactory {
    fn preflight(&self, ctx: &SessionContext) -> Result<()> {
        match ctx.agent_type.as_str() {
            "agy" | "antigravity" => Self::preflight_agy(ctx),
            _ => Ok(()),
        }
    }

    fn capabilities(&self, agent_type: &str) -> ProviderCapabilities {
        match agent_type {
            "claude-code" | "claude" => claude::ClaudeAdapter::capabilities(),
            "generic-pty" | "pty" | "shell" | "bash" | "zsh" | "agy" | "antigravity" => {
                pty::GenericPtyAdapter::capabilities()
            }
            "mock" => {
                if let Some(ref mock) = self.mock_factory {
                    mock.capabilities(agent_type)
                } else {
                    ProviderCapabilities::new(
                        crate::types::AccountSwitchMode::RequiresRestart,
                        true,
                        true,
                    )
                }
            }
            _ => pty::GenericPtyAdapter::capabilities(),
        }
    }

    fn create(
        &mut self,
        ctx: SessionContext,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        match ctx.agent_type.as_str() {
            "claude-code" | "claude" => {
                claude::ClaudeAdapter::spawn(&ctx, self.claude_config.clone(), event_tx)
            }
            "generic-pty" | "pty" => {
                pty::GenericPtyAdapter::spawn(&ctx, self.pty_config.clone(), event_tx)
            }
            "shell" | "bash" | "zsh" => {
                let mut shell_cfg = self.pty_config.clone();
                let user_shell = if ctx.agent_type == "bash" {
                    "/bin/bash".to_string()
                } else if ctx.agent_type == "zsh" {
                    "/bin/zsh".to_string()
                } else {
                    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
                };
                shell_cfg.program = user_shell;
                if !ctx.task_description.is_empty()
                    && ctx.task_description != "shell"
                    && ctx.task_description != "Interactive Shell"
                {
                    shell_cfg.args = vec!["-c".to_string(), ctx.task_description.clone()];
                }
                pty::GenericPtyAdapter::spawn(&ctx, shell_cfg, event_tx)
            }
            "agy" | "antigravity" => {
                let mut agy_cfg = self.pty_config.clone();
                agy_cfg.program = resolve_real_antigravity_bin();
                tracing::info!(
                    "Launching agy for session {} with account_id={:?}, credential_ref={:?}",
                    ctx.session_id,
                    ctx.account_id,
                    ctx.credential_ref
                );
                agy_cfg.envs.extend(agy_account_env(&ctx)?);
                agy_cfg.env_remove.extend(
                    crate::agy_auth::AMBIENT_AUTH_ENV
                        .iter()
                        .map(|s| s.to_string()),
                );
                tracing::info!(
                    "Removed ambient auth env vars: {:?}",
                    crate::agy_auth::AMBIENT_AUTH_ENV
                );
                agy_cfg.args.extend(
                    crate::agy_launch::AgyLaunchOptions::from_agent_config(
                        ctx.agent_config.as_ref(),
                    )
                    .args(),
                );
                agy_cfg.args.extend(agy_task_args(&ctx.task_description));
                pty::GenericPtyAdapter::spawn(&ctx, agy_cfg, event_tx).map_err(|e| {
                    crate::agy_auth::AgyAuthError::LaunchFailure {
                        label: ctx
                            .account_id
                            .as_ref()
                            .map(|a| a.0.clone())
                            .unwrap_or_default(),
                        reason: e.to_string(),
                    }
                    .into()
                })
            }
            "mock" => {
                if let Some(ref mut mock) = self.mock_factory {
                    mock.create(ctx, event_tx)
                } else {
                    let mut default_mock = mock::MockAdapterFactory::always(mock::default_script());
                    default_mock.create(ctx, event_tx)
                }
            }
            other => {
                // Fallback to Generic PTY with the given agent_type as command name
                warn!(
                    "Unknown agent_type '{}', falling back to Generic PTY",
                    other
                );
                let mut fallback_cfg = self.pty_config.clone();
                fallback_cfg.program = other.to_string();
                pty::GenericPtyAdapter::spawn(&ctx, fallback_cfg, event_tx)
            }
        }
    }
}

impl CompositeAdapterFactory {
    fn preflight_agy(ctx: &SessionContext) -> Result<()> {
        let (Some(aid), Some(cref)) = (&ctx.account_id, &ctx.credential_ref) else {
            return Err(crate::agy_auth::AgyAuthError::NoAccountSelected.into());
        };
        crate::agy_auth::validate_credential(cref, &aid.0)?;
        Ok(())
    }
}

/// Environment for an `agy` process: the selected account's isolated profile.
/// There is deliberately no path that launches `agy` without an account.
fn agy_account_env(ctx: &SessionContext) -> Result<std::collections::HashMap<String, String>> {
    let (Some(aid), Some(cref)) = (&ctx.account_id, &ctx.credential_ref) else {
        return Err(crate::agy_auth::AgyAuthError::NoAccountSelected.into());
    };
    #[cfg(unix)]
    {
        Ok(crate::agy_auth::prepare_profile(aid, cref)?)
    }
    #[cfg(not(unix))]
    {
        let _ = (aid, cref);
        anyhow::bail!("Antigravity account profiles are only supported on Unix")
    }
}

/// Arguments derived from the session task. Interactive sessions start a plain
/// `agy` terminal; only a real, explicit task becomes an initial prompt.
pub fn agy_task_args(task_description: &str) -> Vec<String> {
    let t = task_description.trim();
    if t.is_empty() || t.starts_with("[interactive]") {
        vec![]
    } else {
        vec!["-i".to_string(), t.to_string()]
    }
}

/// Helper to resolve the real Antigravity binary on disk without self-recursion.
pub fn resolve_real_antigravity_bin() -> String {
    if let Ok(bin) = std::env::var("ANTIGRAVITY_BIN").or_else(|_| std::env::var("AGY_BIN")) {
        if !bin.trim().is_empty() {
            return bin;
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        let p1 = std::path::PathBuf::from(&home).join(".local/bin/agy");
        if p1.is_file() {
            return p1.to_string_lossy().to_string();
        }
        let p2 = std::path::PathBuf::from(&home).join(".gemini/antigravity-cli/bin/agy");
        if p2.is_file() {
            return p2.to_string_lossy().to_string();
        }
    }

    "agy".to_string()
}
