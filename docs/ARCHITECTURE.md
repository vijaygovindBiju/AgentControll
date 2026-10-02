# Agent Control — Architecture

## Overview

Agent Control is a single local daemon (the **Control Core**) sitting between
human interfaces above and agent adapters below. All state changes flow through
the core; all persistence is an append-only event log in SQLite.

```text
┌──────────────────── Human Interfaces ────────────────────┐
│     TUI (primary)      │     CLI      │   Web UI (future) │
└────────────┬───────────┴──────┬───────┴───────────────────┘
             │   Control API — local Unix socket / loopback WebSocket, JSON
┌────────────▼──────────────────▼───────────────────────────┐
│                     Control Core (daemon)                  │
│                                                            │
│  ┌──────────────────┐   ┌──────────────────────────────┐   │
│  │ Session Manager  │──▶│   Agent Session State Machine │   │
│  └──────────────────┘   └──────────────────────────────┘   │
│  ┌──────────────────┐   ┌──────────────────┐               │
│  │ Account Manager  │   │ Project Registry │               │
│  └──────────────────┘   └──────────────────┘               │
│  ┌──────────────────┐   ┌──────────────────┐               │
│  │ Interaction Hub  │◀─▶│  Policy Engine   │               │
│  └──────────────────┘   └──────────────────┘               │
│  ┌────────────────────────────────────────────────────┐    │
│  │   Event Bus  ──▶  Event Store (SQLite, append-only) │    │
│  └────────────────────────────────────────────────────┘    │
└────────────────────────────┬───────────────────────────────┘
                             │   Adapter Contract (transport-agnostic)
┌────────────────────────────▼───────────────────────────────┐
│  Adapters: Claude Code │ Codex │ Devin CLI │ agy │          │
│            Generic PTY │ Mock                               │
└────────────────────────────┬───────────────────────────────┘
                             │   child processes / local APIs
                     Coding agent processes
             (cwd pinned to a Workspace, env carrying one Account)
```

External consumers (for example AgentDesk or AgentMesh, in the future) connect
to the same Control API as the TUI and CLI. They are clients, never
dependencies.

## Layers

| Layer | Contains | Depends on |
|---|---|---|
| Human interface | TUI, CLI, future Web UI | Control API only |
| Control API | Command dispatch, event subscription, auth, versioned JSON schema | Core |
| Control Core | All modules listed below | Adapter contract, Event Store |
| Adapter layer | One adapter per agent type + Mock + Generic PTY | Adapter contract, the agent's CLI/API |
| Persistence | SQLite event log, snapshots, credential store binding | — |

Dependency direction is strictly top-down. The core never imports UI code or
agent-specific code.

## Modules and responsibilities

### Control Core / daemon

- Process that hosts every module below; one instance per machine/user.
- Owns startup recovery: replays the event log to rebuild in-memory state
  before accepting commands (see
  [`DATA_FLOW.md`](DATA_FLOW.md#7-agent-crash-and-recovery)).
- Owns graceful shutdown: pauses or stops sessions according to policy, flushes
  the event store.
- Owns configuration loading (paths, socket location, default policies).

### Session Manager

- Creates, attaches to, pauses, resumes, stops and restarts `AgentSession`s.
- Resolves the workspace (via the Project Registry) and the account (via the
  Account Manager) before spawning an adapter.
- Supervises adapter processes: liveness, exit detection, restart policy
  (bounded attempts with back-off).
- Performs session snapshots and controlled hand-off (captures snapshot, stops
  predecessor, decrements predecessor account load, spawns linked successor, increments target account load).
- Never changes state directly: it emits events; the State Machine applies them.

### Multi-Account Selection & Switching Subsystem

- **Account Selection**:
  - Explicit selection via `validate_and_select_explicit`: Validates account existence, compatibility with `agent_type`, active status, and concurrency limits before binding.
  - Automatic selection: Evaluates project tag filters and selects the least-loaded compatible account (`active_session_count`), breaking ties deterministically by label, then ID.
  - Availability querying via `query_availability`: Provides clients and the TUI with live availability status and explanatory reasons.
- **Provider Capability Model**:
  - `ProviderCapabilities` reports `switch_mode`: `Dynamic`, `RequiresRestart`, or `Unsupported`.
  - Child-process CLI agents (Claude Code, Generic PTY) honestly declare `RequiresRestart`.
- **Account Switching & Controlled Hand-Off**:
  - *Idle Rebind*: In-place rebinding of `account_id` before agent launch.
  - *Controlled Session Hand-off*: When switching a running agent requiring restart:
    1. Predecessor state snapshot captured (`SessionSnapshot`).
    2. Old agent process stopped gracefully.
    3. Predecessor transitions to `HandedOff`.
    4. Predecessor account concurrency counter decremented.
    5. Successor session created in `Idle` state, inheriting workspace, project, and task description.
    6. Successor session linked via `predecessor_id` and `successor_id`.
    7. Target account concurrency counter incremented.
    8. Successor session launched under the target account with restored snapshot context.
- **Session Snapshot Model**:
  - Versioned schema (`SessionSnapshot`, version 1) capturing safe execution state, task descriptions, and workspace paths.
  - Strict secret exclusion: Raw tokens, API keys, and environment credentials are never serialized.

### Agent Session State Machine

- Holds the single authoritative state per session (states and transitions in
  [`DATA_MODEL.md`](DATA_MODEL.md#agentsession-state-machine)).
- Consumes proposed transitions from adapters, humans, policies and the system;
  accepts or rejects them against the transition table.
- Every accepted transition becomes a `StateChanged` event with `reason`,
  `timestamp` and `triggered_by`.
- Rejected transitions are recorded as `TransitionRejected` events for
  diagnostics; they never mutate state.

### Account Manager

- Maintains the pool of `Account`s (API key, OAuth token, or CLI login
  profile) and their states (`Active`, `Cooldown`, `Exhausted`, `Invalid`,
  `Disabled`).
- Tracks quota and rate-limit signals reported by adapters, per-account
  concurrency caps, and cooldown timers.
- Selects an account for a new session according to project defaults, agent
  type compatibility, availability and a selection strategy (least-loaded by
  default).
- Marks accounts on `RateLimited`/`AuthFailed` events and drives rotation
  decisions together with the Policy Engine.
- Never stores secrets in the event log; it holds only references to the
  credential store (see [`SECURITY.md`](SECURITY.md)).

### Project Registry

- Registry of `Project`s: repository path, default agent type, default account
  or account group, worktree/branch policy, per-project `Policy` set.
- Creates and reclaims `Workspace`s (shared checkout or worktree-per-session
  on a named branch).
- Enforces that a session's working directory is always inside one workspace.

### Interaction Hub

- Unified inbox of `Interaction`s: questions, approval requests, and their
  answers.
- Routes an incoming `ApprovalRequested`/`Question` event first to the Policy
  Engine; if no policy decides, the interaction becomes pending for a human.
- Routes human replies and steering instructions back to the adapter as
  `AgentCommand`s.
- Maintains the per-session transcript (human messages, agent messages,
  decisions) used by the TUI and by session snapshots.

### Policy Engine

- Evaluates declarative `Policy` rules against interactions and session
  conditions.
- Rule outcomes: `Allow`, `Deny`, `Escalate` (to human), plus session-level
  actions such as `PauseSession`, `StopSession`, `RotateAccount`.
- Inputs: tool name, command text, touched paths, project, agent type, account
  state, session runtime, budget counters.
- Hard boundary: action classes marked *never-auto-approve* (see
  [`SECURITY.md`](SECURITY.md#human-approval-boundary)) always return
  `Escalate` regardless of rules.
- Every evaluation produces an `AuditEntry`.

### Event Bus + SQLite Store

- **Event Bus**: in-process publish/subscribe of `AgentEvent`s and core events
  to modules, the Control API subscription endpoint, and the TUI.
- **Event Store**: append-only SQLite table of all events with monotonically
  increasing sequence numbers; source of truth for recovery and replay.
- Periodic **snapshots** of derived state (session states, account states)
  bound to a sequence number to shorten recovery.
- Redaction filter is applied before an event is persisted or published.

### Control API

- Local transport: Unix domain socket (mode 0600) by default; loopback
  WebSocket optionally, token-protected.
- JSON request/response for commands and a streaming subscription for events;
  versioned schema.
- Command families: `session.*`, `account.*`, `project.*`, `interaction.*`,
  `policy.*`, `events.*` — defined in [`INTERFACES.md`](INTERFACES.md).
- The only entry point for humans and external tools; the TUI uses it too, so
  there is no privileged in-process UI path.

### Agent Adapter layer

- One adapter per agent type, all implementing the same
  [adapter contract](INTERFACES.md#adapter-contract).
- Two operating modes:
  - **Structured** (preferred): the agent exposes hooks, NDJSON streams, or an
    API from which events can be derived unambiguously.
  - **PTY fallback** (mandatory for coverage): the adapter drives the agent in
    a pseudo-terminal and classifies output with agent-specific patterns into
    the same event types, with a lower confidence marker.
- Normalizes agent I/O into `AgentEvent`s and accepts `AgentCommand`s
  (`Respond`, `Steer`, `Pause`, `Resume`, `Stop`, `Snapshot`).
- **Claude Code adapter** (`ClaudeAdapter`):
  - **Structured mode** (primary): Spawns Claude Code with `--output-format stream-json --input-format stream-json` over stdin/stdout, parsing structured NDJSON events (`ready`, `output`, `progress`, `tool_use`/`approval_requested`, `question`, `rate_limit`, `completed`, `error`) into `AdapterEvent` with `Confidence::High`. Formats `AgentCommand` into JSON commands on stdin.
  - **PTY mode** (fallback): Runs Claude Code in a pseudo-terminal (`portable-pty`) parsing interactive terminal output.
- **Generic PTY adapter** (`GenericPtyAdapter`):
  - Cross-platform pseudo-terminal allocation via `portable-pty`.
  - Configurable regex matching for interactive CLI agents (`Ready`, `ApprovalRequested`, `QuestionRaised`, `RateLimitSignal`, `Completed`) marked with `Confidence::Low`.
  - Native POSIX signal handling (`libc::kill`) for pause (`SIGSTOP`), resume (`SIGCONT`), graceful shutdown (`SIGTERM`), and kill (`SIGKILL`).
  - Background process monitor detecting clean exit codes and abnormal crash termination.
- **Antigravity adapter** (`CompositeAdapterFactory` support for `"agy"` / `"antigravity"`):
  - Resolves the external Antigravity binary via `ANTIGRAVITY_BIN`, `AGY_BIN`, `~/.local/bin/agy`, or `~/.gemini/antigravity-cli/bin/agy`, avoiding recursive self-invocation. AgentControll does not install or own the `agy` command.
  - **Account Profile Isolation**: Prepares an isolated profile directory at `~/.config/agentcontrol/profiles/<account-id>/` and sets `HOME` to it. The profile contains private tokens and config symlinks, completely isolated from ambient `~/.gemini` defaults.
  - **Ambient Credential Stripping**: Strips ambient Google environment variables (`GEMINI_API_KEY`, `GOOGLE_API_KEY`, `GOOGLE_APPLICATION_CREDENTIALS`, `GOOGLE_GENAI_USE_VERTEXAI`, `GOOGLE_CLOUD_ACCESS_TOKEN`) so sessions run strictly under the bound account.
  - **Per-Session Launch Options**: Consumes `AgyLaunchOptions` (`execution_mode`, `permission_mode`, `model`, `working_dir`, `sandbox`), validating values against advertised options of the installed binary.
  - **Anti-Deadlock PTY Streaming**: Employs non-blocking `try_send` dispatch, 8KB burst read buffers, and 512-item per-session event queues to prevent reader thread deadlocks during large output bursts.
- **Composite Adapter Factory** (`CompositeAdapterFactory`):
  - Routes `agent_type` string (`claude-code`/`claude`, `generic-pty`/`pty`, `mock`, `agy`/`antigravity`) to the designated factory implementation.
  - Passes immutable `SessionContext` (containing `workspace_path`, `credential_ref`, `task_description`, and `agent_config`) to instantiate adapters with strict workspace isolation.
- **Mock adapter**: scripted event sequences for deterministic unit and integration tests.

### Human Interface Layer (`crates/ac-cli` & `crates/ac-tui`)

The human interface is split into two complementary layers:

1. **Normal-User Layer (`agentcontroll`):**
   - **`agentcontroll`**: Launches the main Ratatui interactive dashboard providing high-level overviews of `AGENTS`, `ACCOUNTS`, and `SESSIONS` with direct keyboard actions (`[Enter] Open`, `[A] Add Account`, `[N] New Agent`, `[S] Sessions`, `[Q] Quit`). Aliased as `agent-control`.

   - **TUI account and session controls**: Add, select, and remove Antigravity accounts; resolve account availability and launch sessions with the selected account.
   - **Interactive Session Controller**: Ratatui-based live session screen with streaming transcripts, state badges, and hotkeys (`[s]` Steer, `[p]` Pause, `[r]` Resume, `[x]` Stop, `[a]` Switch Account, `[Ctrl+Q]` Detach).

2. **Power-User / Automation Layer (`ac`):**
   - Headless scriptable CLI providing low-level access to all subsystems: `ac session ...`, `ac account ...`, `ac project ...`, `ac interaction ...`, `ac policy ...`, `ac audit ...`, `ac events ...`.
   - Used by CI pipelines, test runners, and automated scripts.

### TUI Architecture (`crates/ac-tui`)

The Ratatui TUI is the primary operational dashboard for human supervision of agents:
- **Independent client layer**: Implemented in `crates/ac-tui` and built purely atop the Control API Unix socket via `ApiClient`. It holds no direct references to internal daemon storage or runtime structs.
- **Asynchronous reactive event loop**: Runs on Tokio using `tokio::select!` across:
  - Terminal key events (`crossterm::event::EventStream`)
  - Real-time daemon event stream (`events.subscribe`)
  - Periodic background polling tick (recovering connection and reconciling in-memory caches)
- **Streamlined 6-Tab Navigation**:
  - `1: Dashboard`: System summary cards, AGENTS breakdown, ACCOUNTS overview, SESSIONS inventory with lifecycle badges, inbox alert banner, and quick-launch triggers.
  - `2: Sessions`: Full session inventory with task summaries, lifecycle badges, and direct action triggers (`Space` run/resume, `w` switch account, `s` steer, `p` pause, `x` stop).
  - `3: Accounts`: Real-time account pool status, provider info, active session counts against concurrency limits, cooldown timers, and account deletion (`d`).
  - `4: Activity`: Global event log viewer with real-time streaming, kind/session filters (`/`), and payload inspection.
  - `5: Agents`: Supported agent types, registered accounts, active sessions, and new session launch modal.
  - `6: Settings`: Central configuration hub with 9 keyboard-navigable sections (General, Accounts, Projects & Workspaces, Agents, Models, Permissions, Authentication, Terminal, Security). Project and account management is fully embedded here with direct add, remove, and default inspection actions. Persisted to `~/.config/agentcontrol/settings.json`.
- **Dedicated Agent Terminal & Virtual Emulation (`terminal_buffer.rs`)**:
  - Full-screen ANSI virtual terminal buffer tracking cursor coordinates, scroll regions, line wraps, colors, and bold/italic styles.
  - Full alternate screen buffer support (`1049h`/`1049l`) ensuring full-screen interactive programs (like `agy`) render with complete layout fidelity.
  - Multi-byte UTF-8 stream decoder (`decode_utf8_stream`) ensuring UTF-8 glyphs split across chunk boundaries are never corrupted into `U+FFFD`.
  - **Transparent Key Forwarding**: `Esc` is forwarded directly to the agent PTY (enabling vi/nano modes, dialog dismissals, and agy prompt interactions), while `Ctrl+Q` cleanly detaches back to the dashboard.
- **Modal System**: Popups for session creation with Tab directory completion (`n`), textual steering input (`s`), account switching with confirmation (`w`), interaction responses, destructive action confirmation, and contextual shortcuts help (`?`).
- **Headless testability**: Decoupled UI state (`App`) and render functions allow headless testing with `ratatui::backend::TestBackend`.

## Cross-cutting concerns

- **Identity**: every entity has a stable ULID-style identifier; events
  reference entities by identifier only. Friendly labels are human-facing aliases mapped to internal IDs.
- **Time**: the core uses a single clock abstraction so that tests can drive
  time deterministically (cooldowns, timeouts, escalations).
- **Redaction**: one shared redaction component used by the Event Store,
  Control API and TUI. Credential references (`ref:<id>`) are separated from secret payload tokens.
- **Versioning**: event and command schemas carry a version; the store keeps
  the version with each event to allow migration on replay.
