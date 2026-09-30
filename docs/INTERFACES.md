# Agent Control — Interfaces

This document describes the Control API (commands and event subscription) and
the Adapter Contract that every agent adapter must satisfy. These are conceptual
specifications for the design phase; precise wire formats will be finalized
during implementation.

For the entities these commands operate on, see [`DATA_MODEL.md`](DATA_MODEL.md).
For runtime flows, see [`DATA_FLOW.md`](DATA_FLOW.md).
For security controls on the API, see [`SECURITY.md`](SECURITY.md).

---

## Control API

### Transport

| Transport | When available | Authentication |
|---|---|---|
| Unix domain socket `$XDG_RUNTIME_DIR/agentcontrol/agentcontrol.sock` (mode 0600) | Always (v1 default) | File system permissions (owner only) |
| Loopback WebSocket `ws://127.0.0.1:4242` (ADR-009) | Config-enabled (`ws_enabled = true`) | Bearer token (`Authorization`), query string (`?token=`), or in-band `auth` command |

- Unix socket is the primary and mandatory transport for CLI and TUI. Strict filesystem permissions `0600` enforce local isolation.
- WebSocket transport enables external consumers (AgentDesk, AgentMesh, scripts) to issue commands and stream events over loopback.
- Scope enforcement: tokens are assigned `read`, `write`, or `admin` scopes. Mutating commands require `write` or `admin` authorization.
- Both transports share the identical request dispatcher (`dispatch_request`), JSON message schema, and versioning guarantee (`v: 1`). See [`API_CHANGELOG.md`](API_CHANGELOG.md) for full details.

### Schema versioning

Every request and response carries a `"v"` field with the schema version
integer. The daemon advertises its current version on connect. If a client
sends a version the daemon does not support, the daemon returns a
`VersionMismatch` error.

```json
{ "v": 1, "id": "req-01", "cmd": "session.start", "params": { ... } }
```

### Request / response model

Commands use a request/response pattern:

```
Client → { "v": 1, "id": "<uuid>", "cmd": "<family>.<action>", "params": { ... } }
Daemon → { "v": 1, "id": "<uuid>", "ok": true, "result": { ... } }
       | { "v": 1, "id": "<uuid>", "ok": false, "error": { "code": "...", "message": "..." } }
```

### Event subscription

```
Client → { "v": 1, "id": "<uuid>", "cmd": "events.subscribe", "params": { <filter> } }
Daemon → { "v": 1, "id": "<uuid>", "ok": true }          (ack)
Daemon → { "v": 1, "event": { <AgentEvent> } }            (stream, one per line)
Daemon → { "v": 1, "event": { <AgentEvent> } }
...
```

Subscription filters (all optional, combinable):

| Filter field | Type | Effect |
|---|---|---|
| `kinds` | string[] | Only events of these `EventKind` values |
| `session_id` | ULID | Only events for this session |
| `project_id` | ULID | Only events for sessions in this project |
| `account_id` | ULID | Only events involving this account |
| `since_seq` | u64 | Replay from this sequence number (catch-up) |

---

## Command families

### session.*

| Command | Params | Description |
|---|---|---|
| `session.create` | `project_id?`, `task_description`, `agent_type?`, `account_tags?`, `account_id?` | Create a session (state = Idle) without starting it |
| `session.start` | `session_id` | Start a created session (Idle → Starting) |
| `session.create_and_start` | same as create | Shorthand: create + start atomically |
| `session.pause` | `session_id` | Pause a Working or WaitingForHuman session |
| `session.resume` | `session_id` | Resume a Paused session |
| `session.stop` | `session_id`, `reason?` | Gracefully stop any non-terminal session |
| `session.steer` | `session_id`, `message` | Send a steering instruction to a running session |
| `session.select_account` | `session_id`, `account_id` | Explicitly bind/rebind an account to an Idle session |
| `session.switch_account` | `session_id`, `target_account_id` | Switch account for an Idle (rebind) or running session (controlled hand-off) |
| `session.snapshot` | `session_id` | Capture a provider-neutral context snapshot |
| `session.handoff` | `session_id`, `target_account_id?` | Initiate a controlled session hand-off |
| `session.list` | `project_id?`, `state?` | List sessions with optional filters |
| `session.get` | `session_id` | Get full session detail |
| `session.transcript` | `session_id`, `limit?`, `offset?` | Get paginated interaction transcript |

### account.*

| Command | Params | Description |
|---|---|---|
| `account.register` | `label`, `provider`, `agent_types`, `credential_ref`, `concurrency_cap`, `tags?` | Register a new account |
| `account.list` | `state?`, `tags?` | List accounts |
| `account.get` | `account_id` | Get account detail |
| `account.query_availability` | `agent_type?`, `tags?` | Query accounts with live availability status and capacity diagnostics |
| `account.switch` | `session_id`, `target_account_id` | Convenience alias for `session.switch_account` |
| `account.disable` | `account_id`, `reason?` | Manually disable an account (Disabled state) |
| `account.enable` | `account_id` | Re-enable a disabled account |
| `account.update` | `account_id`, `label?`, `concurrency_cap?`, `tags?` | Update mutable fields |
| `account.remove` | `account_id` | Remove an account (only if no active sessions) |

### project.*

| Command | Params | Description |
|---|---|---|
| `project.register` | `name`, `repo_path`, `default_agent_type?`, `default_account_tags?`, `workspace_policy` | Register a project |
| `project.list` | — | List all projects |
| `project.get` | `project_id` | Get project detail |
| `project.update` | `project_id`, `default_agent_type?`, `default_account_tags?`, `workspace_policy?` | Update project settings |
| `project.remove` | `project_id` | Remove project (only if no active sessions) |
| `project.workspaces` | `project_id` | List workspaces for a project |

### interaction.*

| Command | Params | Description |
|---|---|---|
| `interaction.list_pending` | `session_id?` | List pending interactions awaiting human review |
| `interaction.list` | `session_id?` | List all interactions (optionally filtered by session) |
| `interaction.get` | `id` | Get interaction detail by ID |
| `interaction.reply` / `interaction.resolve` | `id`, `approved?`, `allow?`, `response?` | Resolve a pending interaction (approve/deny or reply) |
| `interaction.dismiss` | `id` | Dismiss a pending interaction |

### policy.*

| Command | Params | Description |
|---|---|---|
| `policy.list` | `scope?` | List registered policy rules (optionally filtered by scope) |
| `policy.get` | `id` | Get policy detail by ID |
| `policy.upsert` / `policy.create` | `policy` or `name`, `scope`, `priority`, `conditions`, `decision` | Create or update a policy rule |
| `policy.remove` / `policy.delete` | `id` | Remove a policy rule by ID |
| `policy.test` | `tool_name`, `agent_type?`, `scope?` | Dry-run policy evaluation against input |

### audit.*

| Command | Params | Description |
|---|---|---|
| `audit.list` / `audit.query` | `session_id?`, `interaction_id?`, `limit?` | Query immutable audit log entries |

### events.*

| Command | Params | Description |
|---|---|---|
| `events.subscribe` | `<filter>` | Open a subscription stream (see above) |
| `events.query` | `<filter>`, `limit?`, `offset?` | Query historical events from the store |
| `events.get` | `seq` | Get a single event by sequence number |

### daemon.*

| Command | Params | Description |
|---|---|---|
| `daemon.status` | — | Daemon version, uptime, event store seq, session counts |
| `daemon.shutdown` | `reason?` | Request graceful daemon shutdown |

---

## Error model

All errors follow a common structure:

```json
{
  "v": 1,
  "id": "<request-id>",
  "ok": false,
  "error": {
    "code": "NoAccountAvailable",
    "message": "No account with the required tags is available for assignment",
    "detail": { "required_tags": ["anthropic"], "available_accounts": 0 }
  }
}
```

### Standard error codes

| Code | Meaning |
|---|---|
| `NotFound` | Entity does not exist |
| `InvalidState` | Command not valid in the entity's current state |
| `InvalidTransition` | State machine rejected the requested transition |
| `NoAccountAvailable` | No compatible, available account for session assignment |
| `AccountNotFound` | Specified account does not exist |
| `IncompatibleAccount` | Account does not support the requested agent type |
| `AccountUnavailable` | Account is not active (cooldown, rate-limited, disabled, invalid) |
| `ConcurrencyLimitReached` | Account active session count is at its concurrency cap |
| `UnsupportedCapability` | Provider does not support requested capability (e.g. dynamic account switch) |
| `SnapshotFailed` | Session snapshot generation failed |
| `StaleSession` | Target session is terminal or cannot be mutated |
| `WorkspaceError` | Workspace creation or reclamation failed |
| `AdapterStartFailed` | Adapter process failed to start |
| `PermissionDenied` | Token lacks required scope (WebSocket auth) |
| `VersionMismatch` | Client schema version not supported |
| `ValidationError` | Request params failed validation |
| `InternalError` | Unexpected internal error (includes a correlation id for log lookup) |

---

## Adapter Contract

All agent adapters must implement this contract. The contract is
transport-agnostic: the Session Manager communicates with adapters via an
in-process message channel (not over the network).

### Adapter lifecycle interface

```rust
AdapterFactory::capabilities(&self, agent_type: &str) -> ProviderCapabilities
AdapterFactory::create(ctx: SessionContext, event_tx: mpsc::Sender<AdapterEvent>) -> Result<Box<dyn AgentAdapter>, CoreError>

Adapter::start(&mut self) -> Result<(), AdapterError>
Adapter::stop(&mut self, reason: Option<String>) -> Result<(), AdapterError>
Adapter::pause(&mut self) -> Result<(), AdapterError>
Adapter::resume(&mut self) -> Result<(), AdapterError>
Adapter::send_command(&mut self, cmd: AgentCommand) -> Result<(), AdapterError>
Adapter::snapshot(&mut self) -> Result<ContextSnapshot, AdapterError>

events: channel<AdapterEvent>   (adapter publishes events via event_tx; core subscribes)
```

### SessionContext (provided to adapter on start)

| Field | Description |
|---|---|
| `session_id` | ULID of the session |
| `workspace_path` | Absolute path for cwd |
| `credential_ref` | Reference to look up credentials from the credential store |
| `task_description` | The task string to give the agent |
| `context_snapshot` | Optional prior snapshot (for hand-off continuation) |
| `agent_config` | Adapter-specific configuration (timeouts, model, flags) |

### AgentCommand (core → adapter)

| Variant | Fields | Description |
|---|---|---|
| `Respond` | `allow: bool`, `response?: string` | Reply to an approval request or question |
| `Steer` | `message: string` | Inject a steering instruction |
| `SwitchAccount` | `credential_ref: string` | Notify running adapter of dynamic credential update (if supported) |
| `Pause` | — | Suspend agent execution |
| `Resume` | — | Resume agent execution |
| `Stop` | `reason?: string` | Graceful shutdown |
| `Snapshot` | — | Request the adapter to produce a context snapshot |

### AdapterEvent (adapter → core)

| Variant | Key fields | Description |
|---|---|---|
| `Ready` | — | Adapter and agent are ready |
| `OutputChunk` | `text`, `confidence` | Raw or structured output chunk |
| `ApprovalRequested` | `tool_name`, `tool_args`, `prompt` | Agent wants to execute a tool |
| `QuestionRaised` | `prompt` | Agent is asking for information |
| `SteeringAcknowledged` | — | Agent received a steering instruction |
| `RateLimitSignal` | `back_off_hint?`, `quota_reset_at?` | Provider quota/rate-limit signal |
| `Paused` | — | Agent execution suspended |
| `Resumed` | — | Agent execution resumed |
| `Completed` | `summary?` | Agent considers the task done |
| `Crashed` | `exit_code?`, `stderr_tail?` | Process exited unexpectedly |
| `SnapshotProduced` | `summary`, `transcript_excerpt?` | Response to `Snapshot` command |

### Core Event Bus Events (AgentEvent)

Published to subscribers and written to the append-only SQLite event store:
- **Session Lifecycle:** `SessionCreated`, `SessionStartRequested`, `SessionReady`, `SessionStarted`, `SessionPaused`, `SessionResumed`, `SessionStopping`, `SessionStopped`, `SessionCrashed`, `SessionRestarting`, `SessionFailed`, `SessionHandedOff`.
- **Interactions & Approvals:** `InteractionCreated`, `InteractionAutoResolved`, `InteractionHumanResolved`, `InteractionDismissed`, `AgentSteeringReceived`.
- **Accounts & Switching (Phase 6):**
  - `AccountSelected`: Emitted when an account is explicitly or automatically assigned to a session.
  - `AccountSwitchRequested`: Operator or policy requested an account switch for a session.
  - `AccountSwitchStarted`: Account switch execution has begun.
  - `AccountSwitchCompleted`: Account switch completed successfully.
  - `AccountSwitchFailed`: Account switch rejected or failed with structured error.
  - `SessionSnapshotCreated`: Session snapshot captured for hand-off or recovery.
  - `SessionHandOffStarted`: Hand-off process to target account started.
  - `SessionHandOffCompleted`: Predecessor stopped and linked successor launched.
  - `SessionHandOffFailed`: Hand-off failed during snapshot or startup.

### Confidence marker

`OutputChunk` events carry a `confidence` field: `High` (structured mode,
unambiguous) or `Low` (PTY pattern match, may be approximate). The TUI
surfaces this so operators can assess adapter quality.

### Operating modes

Every adapter must support at least one mode:

- **Structured mode** (preferred): the adapter communicates with the agent
  via hooks, NDJSON output, or an API; events are derived unambiguously.
- **PTY fallback** (mandatory for coverage): the adapter drives the agent
  in a pseudo-terminal and classifies output with a configurable pattern set.

When both modes are available, structured mode is used; PTY is the fallback.
The mode in use is recorded in the `SessionStarted` event.

### Mock adapter

The Mock adapter satisfies the contract with scripted event sequences. It
accepts a script (list of events with optional delays) at creation time. Used
exclusively for testing and validation (Phases 1–3).

### Generic PTY adapter

A reusable PTY adapter that accepts a configuration file mapping regex patterns
to event types. Can be used for agents that lack a dedicated adapter, as a
stopgap pending a dedicated implementation.

---

## External consumer integration

External consumers (AgentDesk, AgentMesh, scripts) are clients of the Control
API. They are never compiled into the daemon.

### AgentDesk integration

AgentDesk wants to know about session state changes and pending interactions in
order to build attention scores and send notifications.

Recommended integration:
1. Connect via WebSocket (loopback) with a read-only token.
2. Subscribe with `kinds: ["StateChanged", "InteractionCreated", "SessionFailed"]`.
3. Optionally subscribe to `AccountStateChanged` for account health reporting.
4. AgentDesk does not issue commands; it is read-only.

### AgentMesh integration

AgentMesh wants to start sessions, steer running sessions, and react to
session completion events in order to coordinate multi-agent plans.

Recommended integration:
1. Connect via WebSocket with a write-scoped token.
2. Subscribe to `SessionStopped`, `SessionHandedOff`, `SessionFailed` to
   track plan progress.
3. Issue `session.create_and_start` and `session.steer` commands as needed.
4. AgentMesh must never issue `session.handoff` or `account.*` commands; those
   are reserved for the operator.

**Both integrations are optional.** Agent Control does not know or care whether
AgentDesk or AgentMesh are connected. If neither is connected, Agent Control
operates normally.

---

## TUI / CLI as a first-party client

The TUI and CLI are first-party clients of the Control API and use the same
Unix socket transport as any external consumer. There is no privileged
in-process path. This means:

- All TUI state is derived from the event stream.
- CLI commands are identical in structure to API commands.
- Integration tests can drive the full system through the API without a GUI.

### TUI Keyboard Navigation and Shortcuts

The TUI provides a complete keyboard-driven navigation model with contextual hints in the footer:

| Key | Context | Action |
|---|---|---|
| `Tab` / `BackTab` | Global | Cycle forward / backward through top navigation tabs |
| `1` .. `6` | Global | Jump directly to tab (1=Dashboard, 2=Sessions, 3=Inbox, 4=Accounts, 5=Projects, 6=Activity) |
| `↑` / `k`, `↓` / `j` | List Views | Move selection up / down |
| `Enter` | Sessions list | Open Session Detail drilldown view |
| `Ctrl+Q` | Session Terminal | Detach / exit active session terminal view back to dashboard |
| `Esc` | Modals / Terminal | Dismiss active modal dialog (forwarded to PTY in session terminal) |
| `q` | Global | Quit TUI application (or close active modal if open) |
| `r` | Global | Force manual data refresh from Control API |
| `?` | Global | Toggle keyboard shortcuts modal overlay |

#### View-Specific Hotkeys

- **Session Terminal View**:
  - `Ctrl+Q` : Detach from session terminal view back to dashboard / sessions
  - `Ctrl+P` / `F1` : Open Agent Control Command Palette
  - `Esc` : Forwarded to agent PTY (cancels input or closes modal dialogs like Quota in agy)
  - `PgUp` / `PgDn` : Scroll terminal history
- **Sessions List**:
  - `s` : Start selected session (`Idle -> Starting`)
  - `p` : Pause selected session (`Working / WaitingForHuman -> Paused`)
  - `Space` : Resume selected session (`Paused -> Working / WaitingForHuman`)
  - `x` : Open Stop Confirmation modal to gracefully stop session
  - `t` : Open Steer modal to submit human steering instruction
- **Interaction Inbox**:
  - `a` : Approve selected approval request (`Allow` decision)
  - `d` : Deny selected approval request (`Deny` decision)
  - `r` : Open textual Reply modal to provide guidance or answer agent questions
  - `x` : Dismiss interaction without decision
- **Activity Stream**:
  - `/` : Open Event Filter modal (filter stream by event kind or session ID)
  - `c` : Clear active filter to view all events

---

## User-Facing Launchers & CLI Layer

Agent Control provides three CLI binaries:

### 1. `agy` — Antigravity Agent Launcher
```bash
# Launch by friendly account label
agy "Personal Google"

# Auto-detect or interactive account selector
agy

# Add account via browser OAuth
agy account add

# List registered Antigravity accounts
agy account list
```
- **Error Transparency**: Displays human-friendly explanations for cooldowns, quota limits, and concurrency caps. Never silently substitutes another account if the requested account is unavailable.
- **Interactive Session Controller**: Hotkeys `[s]` Steer, `[p]` Pause, `[r]` Resume, `[x]` Stop, `[a]` Switch Account, `[q]` Quit.

### 2. `agent-control` — Main Interactive Control Plane
```bash
agent-control
```
- Launches the unified Ratatui interactive dashboard showing AGENTS summary, ACCOUNTS statuses, and SESSIONS inventory.
- Global navigation shortcuts: `[Enter]` Open, `[A]` Add Account, `[N]` New Agent, `[S]` Sessions, `[Q]` Quit.

### 3. `ac` — Power-User / Automation CLI
```bash
ac session <list|start|pause|resume|stop|steer|create>
ac account <list|register|cooldown|reset>
ac project <list|register>
ac interaction <list|respond|dismiss>
ac policy <list|add|remove>
ac audit <list>
ac events <query|tail|verify|replay>
ac tui
```

