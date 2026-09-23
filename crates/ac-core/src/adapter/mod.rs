//! Adapter contract and built-in adapters.
//!
//! An adapter wraps one agent type and translates its I/O into `AdapterEvent`s.
//! In Phase 1 only the `MockAdapter` is implemented.

pub mod mock;
pub mod pty;
pub mod claude;

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
    fn capabilities(&self, agent_type: &str) -> ProviderCapabilities {
        match agent_type {
            "claude-code" | "claude" => claude::ClaudeAdapter::capabilities(),
            "generic-pty" | "pty" | "agy" | "antigravity" => pty::GenericPtyAdapter::capabilities(),
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
            "agy" | "antigravity" => {
                let mut agy_cfg = self.pty_config.clone();
                agy_cfg.program = resolve_real_antigravity_bin();
                let task_trimmed = ctx.task_description.trim();
                if !task_trimmed.is_empty() {
                    agy_cfg.args.push("-i".to_string());
                    agy_cfg.args.push(task_trimmed.to_string());
                }
                pty::GenericPtyAdapter::spawn(&ctx, agy_cfg, event_tx)
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
                warn!("Unknown agent_type '{}', falling back to Generic PTY", other);
                let mut fallback_cfg = self.pty_config.clone();
                fallback_cfg.program = other.to_string();
                pty::GenericPtyAdapter::spawn(&ctx, fallback_cfg, event_tx)
            }
        }
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

