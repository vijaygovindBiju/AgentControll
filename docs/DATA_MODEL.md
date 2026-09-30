# Agent Control — Data Model

This document defines every persistent and in-memory entity, their fields, the
authoritative `AgentSession` state machine, and the `Account` state machine.

All identifiers are ULID strings unless noted otherwise. Timestamps are UTC
ISO-8601 strings in the event log; in-memory they are monotonic-clock instants.

---

## Entities

### Account

Represents one set of provider credentials with its own quota, rate limits and
concurrency cap.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | Stable identifier |
| `label` | string | Human-readable name, e.g. `"anthropic-personal"` |
| `provider` | string | e.g. `"anthropic"`, `"openai"`, `"devin"`, `"agy"` |
| `agent_types` | string[] | Which adapter types may use this account |
| `credential_ref` | string | Reference into the credential store; never a raw secret |
| `state` | AccountState | Current state (see state machine below) |
| `concurrency_cap` | u8 | Maximum simultaneous sessions on this account |
| `active_session_count` | u8 | Derived: sessions currently in Active/Working/WaitingForHuman states |
| `cooldown_until` | timestamp? | Set when state is `Cooldown`; null otherwise |
| `quota_reset_at` | timestamp? | When the provider's quota window resets; null if unknown |
| `tags` | string[] | Grouping labels used for account selection strategies |
| `created_at` | timestamp | |
| `updated_at` | timestamp | |
| `notes` | string? | Optional human annotation |

#### Account State Machine

```
              ┌────────────────────────────┐
              │           Active           │◀─────────────────────────┐
              │  normal operation          │                          │
              └──────┬──────────┬──────────┘                         │
                     │          │                                     │
          RateLimited │          │ AuthFailed                         │ cooldown_expired
          or Quota    │          │                                     │
          exceeded    │          │                                 ┌──┴──────────┐
                      ▼          ▼                                 │   Cooldown  │
              ┌────────────┐  ┌──────────┐                        │  (timed)    │
              │ RateLimited│  │ Invalid  │                        └─────────────┘
              │ (transient)│  │ (perm)   │                              ▲
              └──────┬─────┘  └──────────┘               cooldown      │
                     │                                    triggered     │
          manual or  │                                                  │
          policy     ▼                                                  │
          action  ┌──────────┐    operator re-enables    ┌─────────────┴───┐
              ──▶ │ Disabled  │◀─────────────────────────│  Exhausted      │
                  │ (manual)  │                           │  (quota=0)      │
                  └───────────┘                           └─────────────────┘
```

| State | Meaning |
|---|---|
| `Active` | Available for new session assignment |
| `RateLimited` | Temporarily receiving 429s; adapter reported rate-limit signal |
| `Cooldown` | Cooling down after exhaustion or repeated rate-limits; timed |
| `Exhausted` | Quota fully consumed; cannot accept new sessions |
| `Invalid` | Credentials rejected (auth failure); requires human intervention |
| `Disabled` | Manually taken out of rotation by the operator |

---

### Project

A registered repository with defaults and policies for agent sessions.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | |
| `name` | string | Human-readable, e.g. `"agent-control"` |
| `repo_path` | path | Absolute path to the repository root |
| `default_agent_type` | string? | e.g. `"claude-code"`, `"codex"` |
| `default_account_tags` | string[] | Account tag filter for session assignment |
| `workspace_policy` | WorkspacePolicy | `Shared` or `WorktreePerSession` |
| `worktree_base_branch` | string? | Base branch for worktree creation (if `WorktreePerSession`) |
| `policies` | Policy[] | Project-level policy rules |
| `created_at` | timestamp | |
| `updated_at` | timestamp | |
| `notes` | string? | |

#### WorkspacePolicy

- `Shared` — all sessions on this project share the project's main checkout.
- `WorktreePerSession` — the Project Registry creates a git worktree on a
  fresh branch for each new session; the branch name is derived from the
  session id and task summary.

---

### Workspace

A specific on-disk working directory bound to exactly one session at a time.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | |
| `project_id` | ULID | Parent project |
| `kind` | WorkspaceKind | `Shared` or `Worktree` |
| `path` | path | Absolute path on disk |
| `branch` | string? | Git branch (for `Worktree` kind) |
| `session_id` | ULID? | Currently bound session; null when idle |
| `created_at` | timestamp | |
| `reclaimed_at` | timestamp? | When the worktree was deleted after session completion |

---

### AgentSession

One running (or paused/stopped) instance of an agent, bound to a workspace,
an account and a task.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | Stable identifier |
| `task_description` | string | Full task description given to the agent |
| `agent_type` | string | Adapter type, e.g. `"mock"`, `"claude-code"`, `"agy"`, `"pty"` |
| `state` | SessionState | See state machine below |
| `restart_count` | u32 | Number of automatic restarts performed |
| `project_id` | ULID? | Parent project (None for unassociated sessions) |
| `workspace_id` | ULID? | Bound workspace directory reference |
| `account_id` | ULID? | The account currently in use |
| `predecessor_id` | ULID? | Previous session (for hand-offs) |
| `successor_id` | ULID? | Next session (after hand-off) |
| `context_snapshot_id` | ULID? | Latest snapshot used or produced |
| `created_at` | timestamp | Timestamp of entity creation |
| `updated_at` | timestamp | Last state change timestamp |
| `started_at` | timestamp? | When the adapter process was last launched |
| `stopped_at` | timestamp? | When the session entered a terminal state |
| `launch` | AgyLaunchOptions? | Per-session launch configuration (permission mode, execution mode, model, directory) |

#### AgentSession State Machine

States and their meanings:

| State | Meaning |
|---|---|
| `Idle` | Created but agent process not yet started |
| `Starting` | Adapter process launched; waiting for ready signal |
| `Working` | Agent is actively processing |
| `WaitingForHuman` | Agent has raised a question or approval request; awaiting human or policy reply |
| `Paused` | Execution suspended by human or policy; process is alive but not consuming input |
| `RateLimited` | Account reported rate-limit; session is blocked, not failed |
| `Stopping` | Graceful shutdown in progress |
| `Stopped` | Terminated cleanly |
| `Crashed` | Process exited unexpectedly; eligible for automatic restart |
| `Restarting` | Restart in progress (bounded by restart policy) |
| `HandedOff` | Task migrated to a successor session; this session is permanently done |
| `Failed` | Unrecoverable error; requires human intervention |

```
                         ┌──────────────────────────────────┐
             create      │             Idle                 │
          ─────────────▶ │  (created, process not started)  │
                         └──────┬──────────────┬────────────┘
                                │ start         │ stop (no adapter)
                                ▼               ▼
                         ┌──────────────┐   ┌─────────┐
                         │   Starting   │   │ Stopped │ ◀── (terminal)
                         └──────┬───────┘   └─────────┘
                                │ adapter ready
                    ┌───────────┴────────────────────────────┐
                    │                                        │ start failed
                    ▼                                        ▼
      ┌─────────────────────────┐                  ┌─────────────────┐
  ┌──▶│         Working         │◀──────────┐       │     Failed      │
  │   │   (agent processing)    │           │       └─────────────────┘
  │   └────────────┬────────────┘           │
  │                │ approval/question       │ reply/policy decides
  │   resume       ▼                        │
  │   ┌──────────────────────────┐          │
  │   │     WaitingForHuman      │──────────┘
  │   └──────────────────────────┘
  │
  │  From Working, WaitingForHuman, Paused, RateLimited, Starting:
  │         │ pause       │ rate-limit   │ stop cmd    │ process exit
  │         ▼             ▼             ▼             ▼
  │  ┌────────────┐ ┌───────────┐ ┌──────────┐  ┌──────────┐
  │  │   Paused   │ │RateLimited│ │ Stopping │  │ Crashed  │
  │  └─────┬──────┘ └─────┬─────┘ └────┬─────┘  └────┬─────┘
  │        │ resume        │ cleared    │             │ within restart limit?
  └────────┘               └──────────▶│ Stopped ◀───┘ yes → Restarting──▶ Starting
                                        │ HandedOff       no → Failed
                                        ▼
                                   (terminal)

  hand-off:  Working/Paused ──▶ Stopping ──▶ HandedOff
```

Valid transitions (authoritative — matches `session/state_machine.rs`):

| From | To | Trigger |
|---|---|---|
| `Idle` | `Starting` | `session.start` command |
| `Idle` | `Stopped` | `session.stop` on an unstarted session (no adapter to terminate) |
| `Starting` | `Working` | Adapter ready signal |
| `Starting` | `Failed` | Adapter failed to start |
| `Starting` | `Stopping` | `session.stop` command issued while adapter is still starting |
| `Starting` | `Crashed` | Adapter process exited unexpectedly during start |
| `Working` | `WaitingForHuman` | `ApprovalRequested` or `Question` event |
| `Working` | `Paused` | `session.pause` command |
| `Working` | `RateLimited` | `RateLimitSignal` from adapter |
| `Working` | `Stopping` | `session.stop` command or hand-off initiated |
| `Working` | `Crashed` | Process exited unexpectedly |
| `WaitingForHuman` | `Working` | Human reply or policy decision delivered |
| `WaitingForHuman` | `Paused` | `session.pause` command |
| `WaitingForHuman` | `RateLimited` | `RateLimitSignal` from adapter |
| `WaitingForHuman` | `Stopping` | `session.stop` command |
| `Paused` | `Working` | `session.resume` command |
| `Paused` | `Stopping` | `session.stop` command |
| `Paused` | `Crashed` | Process exited unexpectedly while paused |
| `RateLimited` | `Working` | Rate-limit cleared (account back to Active) |
| `RateLimited` | `Stopping` | `session.stop` command |
| `Stopping` | `Stopped` | Adapter confirmed shutdown |
| `Stopping` | `HandedOff` | Hand-off sequence completed |
| `Crashed` | `Restarting` | Restart policy allows retry |
| `Crashed` | `Failed` | Restart limit exceeded |
| `Restarting` | `Starting` | Restart attempt begins |

**Note on `Idle → Stopped`:** An `Idle` session has no active adapter process.
Routing the stop through `Stopping` would be meaningless — there is nothing to
shut down. The `Idle → Stopped` transition skips `Stopping` and is handled
directly in the Session Manager's `cmd_stop`, which checks for `Idle` state
before deciding whether to emit a `Stopping` transition. See ADR-011 in
[`DECISIONS.md`](DECISIONS.md).

---

### Task

A human-supplied description of what a session is expected to accomplish.
Tasks survive hand-offs.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | |
| `project_id` | ULID | |
| `title` | string | Short summary, used in branch names and TUI |
| `description` | string | Full task description given to the agent |
| `origin_session_id` | ULID? | Session that first received this task |
| `created_at` | timestamp | |
| `closed_at` | timestamp? | Set when task is done or abandoned |

---

### Interaction

A single question or approval request raised by an agent, and its resolution.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | Stable interaction identifier |
| `session_id` | ULID | The session that raised this interaction |
| `kind` | InteractionKind | `Question`, `ApprovalRequest` |
| `state` | InteractionState | `Pending`, `AutoResolved`, `HumanResolved`, `Dismissed`, `Expired` |
| `prompt` | string | The agent's question or approval request text |
| `tool_name` | string? | Tool being requested (for `ApprovalRequest`) |
| `tool_args` | json? | Arguments of the tool call (redacted before storage) |
| `policy_id` | ULID? | Policy that auto-resolved this interaction (if auto-resolved) |
| `decision` | PolicyDecision? | `Allow`, `Deny`, `RequireHuman` |
| `response` | string? | Human text response (for `Question` kind) |
| `resolved_by` | string? | `"policy:<id>"` or `"human:<user>"` |
| `created_at` | timestamp | |
| `resolved_at` | timestamp? | |

#### InteractionKind

- `Question` — agent is asking for information or clarification.
- `ApprovalRequest` — agent wants to execute a tool/action and needs sign-off.

#### InteractionState

- `Pending` — awaiting policy evaluation or human response.
- `AutoResolved` — resolved by a policy rule without human involvement.
- `HumanResolved` — resolved by explicit human input.
- `Dismissed` — dismissed by human without executing the action.
- `Expired` — the session ended before the interaction was resolved.

---

### AgentEvent

Every state change, signal and significant occurrence is recorded as an
`AgentEvent` in the append-only event store.

| Field | Type | Description |
|---|---|---|
| `seq` | u64 | Monotonically increasing, gapless sequence number |
| `id` | ULID | Stable event identifier |
| `version` | u16 | Schema version, for migration on replay |
| `kind` | EventKind | See event kind catalogue below |
| `session_id` | ULID? | Null for non-session events (account, project, system) |
| `account_id` | ULID? | |
| `project_id` | ULID? | |
| `payload` | json | Kind-specific data; subject to redaction rules |
| `triggered_by` | string | `"human"`, `"policy:<id>"`, `"adapter:<type>"`, `"system"` |
| `timestamp` | timestamp | Wall-clock UTC |

#### Event Kind Catalogue

Session lifecycle:
- `SessionCreated`, `SessionStartRequested`, `SessionStarted`, `SessionReady`
- `SessionPaused`, `SessionResumed`
- `SessionStopRequested`, `SessionStopped`
- `SessionCrashed`, `SessionRestartAttempted`, `SessionRestartSucceeded`
- `SessionFailed`, `SessionHandedOff`
- `StateChanged` — generic transition record (from → to, reason)
- `TransitionRejected` — invalid transition attempt, for diagnostics

Agent I/O:
- `AgentOutputReceived` — raw or structured output chunk from adapter
- `AgentQuestion` — agent raised a question
- `AgentApprovalRequested` — agent requested a tool call approval
- `AgentSteeringReceived` — adapter acknowledged a steering instruction

Account events:
- `AccountRegistered`, `AccountStateChanged`, `AccountCooldownStarted`
- `AccountCooldownExpired`, `AccountExhausted`, `AccountInvalidated`
- `AccountRateLimited`, `AccountRateLimitCleared`
- `RateLimitSignal` — rate-limit signal from adapter (carries back-off hint)

Interaction events:
- `InteractionCreated`, `InteractionAutoResolved`, `InteractionHumanResolved`
- `InteractionExpired`, `InteractionDismissed`

Policy events:
- `PolicyCreated`, `PolicyUpdated`, `PolicyRemoved`, `PolicyEvaluated`
- `PolicyOverridden` — human overrode a policy decision

Approval flow events:
- `ApprovalRequested`, `ApprovalAutoApproved`, `ApprovalDenied`, `ApprovalHumanDecision`

System events:
- `DaemonStarted`, `DaemonShutdownRequested`, `DaemonStopped`
- `SnapshotCreated` — derived-state snapshot bound to a sequence number

---

### Policy

A declarative rule evaluated by the Policy Engine against interactions and
session conditions.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | Unique policy identifier |
| `name` | string | Human-readable label |
| `scope` | PolicyScope | `Global`, `Project(id)` |
| `priority` | i32 | Higher priority evaluated first |
| `conditions` | PolicyCondition[] | All must match for the policy rule to apply (conjunction) |
| `decision` | PolicyDecision | `Allow`, `Deny`, `RequireHuman` |
| `enabled` | boolean | Active or inactive |
| `created_at` | timestamp | Creation timestamp |
| `updated_at` | timestamp | Last update timestamp |

#### Condition types

- `ToolNameEquals(string)` — matches tool name exactly.
- `ToolNamePrefix(string)` — matches tool name by prefix (e.g. `"git:"`, `"read:"`).
- `AgentTypeEquals(string)` — matches agent type (e.g. `"claude-code"`, `"mock"`).
- `ToolNameIn(string[])` — matches any tool name in the specified list.

#### PolicyDecision

- `Allow` — proceed with tool execution without human involvement.
- `Deny` — reject the action; inform the agent.
- `RequireHuman` — send to the human inbox as a pending interaction.

#### Evaluation Precedence (ADR-012)

1. **Never-auto-approve boundary** (hardcoded security rules): Destructive operations (`rm`, `sudo`, `curl`, `wget`, `git push --force`, `publish`) ALWAYS evaluate to `RequireHuman`. Cannot be overridden by any policy rule.
2. **Matching Deny rules**: Highest priority first.
3. **Matching Allow rules**: Highest priority first.
4. **Matching RequireHuman rules**: Highest priority first.
5. **Default fallback**: `RequireHuman`.

---

### AuditEntry

An immutable record of every policy decision, evaluation, and human action.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | Unique audit record identifier |
| `event_seq` | u64? | Associated event store sequence number |
| `interaction_id` | ULID? | Associated interaction identifier |
| `session_id` | ULID? | Associated session identifier |
| `actor` | string | `"policy:<id>"`, `"human:<user>"`, or `"system"` |
| `action` | string | `"allow"`, `"deny"`, `"require_human"`, `"dismiss"`, `"resolved"` |
| `rationale` | string? | Human note or policy evaluation summary |
| `timestamp` | timestamp | UTC instant |

Audit entries are stored in the SQLite `audit_log` table. They are append-only,
never deleted, and never contain raw secrets.

Audit entries are stored in the same SQLite database as events but in a
separate, never-deleted table. They are never redacted.

---

## Context Snapshot

A point-in-time capture of a session's state used to seed a successor session
during hand-off. Not a primary entity but important for the hand-off flow.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | |
| `session_id` | ULID | Session that produced this snapshot |
| `task_id` | ULID | |
| `summary` | string | Agent-produced or human-edited summary of work done |
| `transcript_excerpt` | string? | Relevant portion of the interaction transcript |
| `workspace_branch` | string? | Branch or commit the work is on |
| `created_at` | timestamp | |
| `event_seq` | u64 | Event store sequence at which this snapshot was taken |

---

### SessionContext

Execution context passed from `SessionManager` into an `AgentAdapter` during initialization.

| Field | Type | Description |
|---|---|---|
| `session_id` | ULID | Stable session identifier |
| `task_description` | string | Full task prompt / description |
| `agent_type` | string | Target adapter type (e.g. `"claude-code"`, `"generic-pty"`, `"mock"`) |
| `workspace_path` | path | Absolute working directory bound to this session |
| `credential_ref` | string? | Reference to credential for environment resolution (never secret text) |
| `context_snapshot` | string? | Context snapshot if resuming/handed-off |
| `agent_config` | json? | Optional adapter-specific configuration parameters |

---

### Confidence

Confidence classification applied to output and parsed events emitted by adapters.

| Level | Description |
|---|---|
| `High` | Unambiguous event parsed from structured protocols (e.g. Claude Code stream-json NDJSON). |
| `Low` | Event inferred through heuristic terminal pattern matching or raw text chunking (e.g. PTY regex). |

---

### ProviderCapabilities & AccountSwitchMode

Defines provider-specific runtime switching and session context capabilities.

| Field | Type | Description |
|---|---|---|
| `switch_mode` | AccountSwitchMode | `Dynamic`, `RequiresRestart`, `Unsupported` |
| `supports_session_resume` | boolean | Whether the provider CLI/API supports resuming prior task sessions |
| `supports_snapshot_restore` | boolean | Whether context snapshots can be injected on launch |

#### AccountSwitchMode

- `Dynamic`: Provider supports updating authentication in-place without process interruption.
- `RequiresRestart`: Provider requires controlled process termination, predecessor snapshotting, and spawning a successor session with the new credentials.
- `Unsupported`: Provider does not permit account switching.

---

### AccountAvailability

Diagnostic and selection status projection for accounts against an agent type and filter tags.

| Field | Type | Description |
|---|---|---|
| `account_id` | ULID | Stable account identifier |
| `label` | string | Human-readable account label |
| `provider` | string | Provider identity (e.g. `"anthropic"`, `"openai"`) |
| `agent_types` | string[] | Compatible agent types |
| `state` | AccountState | `Active`, `Cooldown`, `RateLimited`, `Exhausted`, `Invalid`, `Disabled` |
| `concurrency_cap` | u8 | Concurrency limit |
| `active_session_count` | u8 | Current active load |
| `is_available` | boolean | Whether the account is currently usable for a new session |
| `reason` | string? | Diagnostic reason if unavailable (e.g. `"concurrency limit reached (2/2)"`) |

---

### SessionSnapshot

Provider-neutral capture of session context used for hand-off and recovery. Strictly excludes all secrets and raw credentials.

| Field | Type | Description |
|---|---|---|
| `id` | ULID | Snapshot unique identifier |
| `session_id` | ULID | Source session identifier |
| `agent_type` | string | Agent type (e.g. `"claude-code"`) |
| `task_description` | string | Task description / prompt |
| `state` | SessionState | Session state at time of capture |
| `event_seq` | u64 | Event store sequence number |
| `created_at` | timestamp | Timestamp of capture |
| `version` | u32 | Schema version (`SNAPSHOT_SCHEMA_VERSION = 1`) |
| `project_id` | ULID? | Bound project identifier |
| `workspace_id` | ULID? | Bound workspace identifier |
| `workspace_path` | path? | Absolute filesystem path of workspace |
| `summary` | string? | Summary of session progress |
| `metadata` | map<string, string> | Safe key-value metadata (strictly non-sensitive) |

---

### AgyLaunchOptions

Per-session launch options controlling agent execution, permission boundaries, model selection, and working directory.

| Field | Type | Description |
|---|---|---|
| `execution_mode` | AgyExecutionMode | Execution workflow: `Default` (standard), `AcceptEdits` (`--mode=accept-edits`), `Plan` (`--mode=plan`) |
| `permission_mode` | AgyPermissionMode | Permission boundary: `Normal` (standard approval prompt) or `DangerouslySkipPermissions` (`--dangerously-skip-permissions`) |
| `sandbox` | boolean | Whether to pass `--sandbox` |
| `model` | string? | Model ID (e.g. `gemini-3.8-flash-medium`); None for provider default |
| `working_dir` | path? | Absolute filesystem working directory for the session |

---

### UserSettings

Global user preferences and defaults persisted to `~/.config/agentcontrol/settings.json`.

| Field | Type | Description |
|---|---|---|
| `default_agent` | string | Default agent type (e.g. `"Antigravity"`) |
| `default_account` | string? | Default account label or ID for new sessions |
| `default_project` | string? | Default project name or ID |
| `default_working_dir` | path? | Default working directory |
| `default_execution_mode` | AgyExecutionMode | Default AGY execution mode |
| `default_permission_mode` | AgyPermissionMode | Default AGY permission mode |
| `default_model` | string? | Default model ID |
| `terminal_theme` | string | Active TUI terminal theme (`"Default"`, `"Dark"`, `"High-Contrast"`) |
| `terminal_scrollback` | usize | Virtual terminal buffer line capacity (default: 5000) |
| `terminal_cursor` | string | Terminal cursor style (`"Block"`, `"Bar"`, `"Underline"`) |

---

## Cross-references

- State machine transitions are enforced by the Session Manager and State
  Machine module described in [`ARCHITECTURE.md`](ARCHITECTURE.md).
- The flows that produce and consume these entities are in
  [`DATA_FLOW.md`](DATA_FLOW.md).
- The command and event schema used over the wire is in
  [`INTERFACES.md`](INTERFACES.md).
- Security controls for credentials and event redaction are in
  [`SECURITY.md`](SECURITY.md).
