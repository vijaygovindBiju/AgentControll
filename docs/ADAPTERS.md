# Adapter Architecture & Development

Adapters are the integration bridges that connect external coding agent CLIs to Agent Control's event-driven state machine, PTY multiplexer, and permission model.

---

## 1. Architecture Overview

Agent Control communicates with agents via an asynchronous, event-driven contract:

```
┌─────────────────────────────────────────────────────────────┐
│                    SessionManager                           │
└──────────────┬───────────────────────────────▲──────────────┘
               │ SessionContext                │ AdapterEvent
               ▼                               │ (Started, Output,
┌──────────────────────────────────────────┐   │  Prompt, Stopped)
│          AdapterFactory                  │   │
│  (Composite / Pty / Claude / Mock)       │   │
└──────────────┬───────────────────────────┴───┘
               │ spawns
               ▼
┌──────────────────────────────────────────┐
│              AdapterHandle               │
│      - Drop guard / process killer       │
│      - Command channel (AgentCommand)    │
└──────────────────────────────────────────┘
```

### Key Components

- **`AdapterFactory`**: Trait defining how to query agent capabilities, perform preflight validation, and spawn active sessions.
- **`AdapterHandle`**: RAII handle holding process control channels and cleanup drop guards. When an `AdapterHandle` is dropped or receives `Stop`, it safely shuts down the agent child process.
- **`SessionContext`**: Carries metadata required to spawn the agent, including `session_id`, `project_dir`, `account_id`, `agent_config`, and `launch_options`.
- **`AdapterEvent`**: Asynchronous events emitted by the adapter back to the session manager:
  - `Started`: Agent process spawned and PID assigned.
  - `Output(Bytes)`: Terminal bytes or structured text emitted by agent.
  - `Prompt(InteractionId, PromptData)`: Agent is awaiting user input or permission confirmation.
  - `Stopped(ExitStatus)`: Agent completed execution.
  - `Failed(Error)`: Fatal execution or runtime error.

---

## 2. Built-in Adapters

### `PtyAdapter` (`crates/ac-core/src/adapter/pty.rs`)
- Manages pseudo-terminal (PTY) allocation using `portable-pty`.
- Spawns agents like Antigravity (`agy`) inside an isolated PTY slave.
- Reads raw terminal streams and processes them through `decode_utf8_stream` to ensure multi-byte UTF-8 glyphs are never corrupted when split across read chunks.
- Forwards terminal window resizes (`SIGWINCH` / `set_size`) dynamically.

### `ClaudeAdapter` (`crates/ac-core/src/adapter/claude.rs`)
- Integrates with Anthropic's Claude Code CLI.
- Handles standard environment variables, prompt injection, and process monitoring.

### `MockAdapter` (`crates/ac-core/src/adapter/mock.rs`)
- Synthetic agent used for unit and integration testing without external dependencies.
- Can simulate delays, simulated user prompts, crashes, and rapid output bursts.

---

## 3. How to Add a New Agent Adapter

To add support for a new coding agent (e.g. `codex-cli` or `devin`):

### Step 1: Implement the `AdapterFactory` Trait

In `crates/ac-core/src/adapter/<agent_name>.rs`:

```rust
use anyhow::Result;
use tokio::sync::mpsc;
use crate::types::{AdapterEvent, ProviderCapabilities, SessionContext};
use crate::adapter::{AdapterFactory, AdapterHandle};

pub struct CustomAgentFactory;

impl AdapterFactory for CustomAgentFactory {
    fn capabilities(&self, agent_type: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_pty: true,
            supports_auto_accept: true,
            models: vec!["default".to_string()],
        }
    }

    fn preflight(&self, ctx: &SessionContext) -> Result<()> {
        // Validate credentials, binary paths, or project workspace
        Ok(())
    }

    fn create(
        &mut self,
        ctx: SessionContext,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        // 1. Prepare child process command and environment
        // 2. Allocate PTY or pipe channels
        // 3. Spawn background reader task to send AdapterEvent::Output
        // 4. Return AdapterHandle with drop guard
        todo!()
    }
}
```

### Step 2: Register in `CompositeAdapterFactory`

In `crates/ac-core/src/adapter/mod.rs`, add your new factory to the routing match in `CompositeAdapterFactory::create()`:

```rust
match ctx.agent_type.as_str() {
    "antigravity" | "agy" => self.pty_factory.create(ctx, event_tx),
    "claude-code" => self.claude_factory.create(ctx, event_tx),
    "custom-agent" => self.custom_factory.create(ctx, event_tx),
    _ => Err(anyhow::anyhow!("Unknown agent type: {}", ctx.agent_type)),
}
```

### Step 3: Add End-to-End Tests

Add an integration test in `crates/ac-cli/tests/` to verify session startup, output streaming, and termination under real or mock environments.
