# Agent Control — Data Flow

This document describes the conceptual runtime flows of Agent Control. Each
flow is described as an ordered sequence of steps, showing which modules
participate and what events and state changes result.

For the entities mentioned here, see [`DATA_MODEL.md`](DATA_MODEL.md).
For module responsibilities, see [`ARCHITECTURE.md`](ARCHITECTURE.md).
For the command and event schemas, see [`INTERFACES.md`](INTERFACES.md).

---

## 1. Starting an agent session

**Trigger:** Human sends `session.start` command via TUI or CLI.

```
Human (TUI/CLI)
  │ session.start { project_id, task, agent_type?, account_tags? }
  ▼
Control API
  │ dispatches to Session Manager
  ▼
Session Manager
  ├─ validates project exists (Project Registry)
  ├─ selects account (Account Manager: least-loaded, tag-filtered, compatible agent type)
  ├─ resolves or creates workspace (Project Registry: Shared or new worktree)
  ├─ creates AgentSession entity (state = Idle)
  ├─ creates Task entity
  ├─ emits SessionCreated event → Event Bus → Event Store
  ├─ increments account.active_session_count
  │
  ├─ sends SessionStartRequested → Event Bus → Event Store
  ├─ State Machine: Idle → Starting
  ├─ emits StateChanged(Idle→Starting) → Event Store
  │
  ├─ constructs SessionContext (session_id, task_description, agent_type, workspace_path, credential_ref, agent_config)
  ├─ parses optional AgyLaunchOptions (execution_mode, permission_mode, model, working_dir, sandbox)
  ├─ for Antigravity:
  │     prepares isolated profile directory ~/.config/agentcontrol/profiles/<account-id>/
  │     strips ambient Google environment variables (GEMINI_API_KEY, GOOGLE_APPLICATION_CREDENTIALS, etc.)
  │     sets HOME = ~/.config/agentcontrol/profiles/<account-id>/
  ├─ resolves adapter via CompositeAdapterFactory (ClaudeAdapter, GenericPtyAdapter, AntigravityAdapter, MockAdapter)
  ├─ spawns adapter process with:
  │     cwd  = resolved working_dir or workspace.path (validated on-disk directory boundary)
  │     env  = resolved credentials and isolated HOME (never stored in event log or SQLite)
  │     args = task.description, session.id, and validated CLI flags (--mode=..., --dangerously-skip-permissions, etc.)
  │     PTY  = 8KB read buffers, non-blocking try_send, 512-channel queue to prevent deadlocks
  │
  ▼
Adapter process starts
  │ emits AgentReady (or StartFailed after timeout)
  ▼
Session Manager (via Event Bus)
  ├─ on AgentReady:
  │     State Machine: Starting → Working
  │     emits StateChanged(Starting→Working)
  │     emits SessionStarted
  │     returns success to Control API → TUI refreshes
  │
  └─ on StartFailed / timeout:
        State Machine: Starting → Failed
        emits SessionFailed { reason }
        returns error to Control API → TUI shows failure
```

**Error paths:**
- No compatible account available: command rejected immediately with
  `NoAccountAvailable` error.
- Workspace creation fails (worktree conflict): command rejected with
  `WorkspaceError`.
- Adapter process fails to start within the configured timeout: `Failed` state.

---

## 2. Agent output and approval request

**Trigger:** Running agent produces output or requests a tool call.

```
Adapter (running)
  │ agent writes output / requests tool approval
  ▼
Adapter (structured NDJSON or PTY pattern classifier)
  ├─ structured mode: parses stream-json events (High confidence)
  ├─ PTY mode: matches output chunks against regex patterns (Low confidence)
  ├─ normalizes to AgentOutputReceived event (for regular output)
  │   or AgentApprovalRequested / AgentQuestion event
  ├─ emits event → Event Bus → Event Store
  ▼
For regular output:
  Event Bus → Control API subscription → TUI (session transcript updated)
  (no state change)

For ApprovalRequested or Question:
  ▼
  Interaction Hub
    ├─ creates Interaction entity (state = Pending)
    ├─ emits InteractionCreated → Event Bus → Event Store
    ├─ State Machine: Working → WaitingForHuman
    ├─ emits StateChanged(Working→WaitingForHuman)
    │
    ├─ routes to Policy Engine
    │     evaluates all matching policies in priority order
    │     emits PolicyEvaluated (per policy) → Event Store (AuditEntry)
    │
    ├─ if outcome = Allow:
    │     Interaction.state = AutoResolved, decision = Allow
    │     emits InteractionAutoResolved
    │     State Machine: WaitingForHuman → Working
    │     adapter receives AgentCommand(Respond, allow=true)
    │
    ├─ if outcome = Deny:
    │     Interaction.state = AutoResolved, decision = Deny
    │     emits InteractionAutoResolved
    │     State Machine: WaitingForHuman → Working
    │     adapter receives AgentCommand(Respond, allow=false, reason)
    │
    └─ if outcome = Escalate (or no matching policy):
          Interaction.state = Pending
          TUI inbox updated (pending interaction appears)
          session remains WaitingForHuman until human responds
```

---

## 3. Human approval

**Trigger:** Human responds to a pending interaction in the TUI or CLI.

```
Human (TUI inbox)
  │ interaction.resolve { interaction_id, decision=Allow|Deny, response? }
  ▼
Control API → Interaction Hub
  ├─ validates interaction is Pending and belongs to an active session
  ├─ records decision: Interaction.state = HumanResolved
  ├─ emits InteractionHumanResolved → Event Store
  ├─ creates AuditEntry { actor="human", action=decision, rationale=response }
  ├─ emits AuditEntry → Event Store
  │
  ├─ delivers AgentCommand(Respond, decision, response?) to adapter
  │
  └─ State Machine: WaitingForHuman → Working
        emits StateChanged(WaitingForHuman→Working)
        adapter resumes agent execution
```

**Override flow:** If a human overrides a policy auto-decision (e.g. reopens a
previously auto-approved interaction):

```
  Human overrides → PolicyOverridden event → AuditEntry(actor=human, action=override)
  → same delivery path as above
```

---

## 4. Human steering

**Trigger:** Human sends a steering instruction to a running session.

```
Human (TUI session detail)
  │ session.steer { session_id, message }
  ▼
Control API → Session Manager
  ├─ validates session is in Working or WaitingForHuman state
  ├─ emits AgentSteeringReceived → Event Store
  ├─ delivers AgentCommand(Steer, message) to adapter
  │
  ▼
Adapter
  ├─ injects message into agent's stdin or API
  ├─ emits SteeringAck → Event Bus → Event Store
  ├─ creates Interaction(kind=SteeringAck, state=HumanResolved) for transcript
  └─ session remains in current state (no state transition)
```

---

## 5. Account quota exhaustion

**Trigger:** Adapter receives a quota-exceeded signal from the provider.

```
Adapter
  │ detects quota-exceeded response (HTTP 429 + quota body, or CLI error pattern)
  ▼
Adapter emits RateLimitSignal { account_id, back_off_hint?, quota_reset_at? }
  → Event Bus → Event Store

Account Manager (subscribed to RateLimitSignal)
  ├─ updates Account.state = RateLimited (if transient) or Exhausted (if quota=0)
  ├─ sets Account.cooldown_until (using back_off_hint or default)
  ├─ emits AccountRateLimited or AccountExhausted → Event Store
  ├─ emits AccountStateChanged
  │
  ├─ notifies Session Manager: account is no longer accepting new sessions
  │
  └─ schedules cooldown expiry timer
        on expiry: Account.state = Active
        emits AccountCooldownExpired → Event Store

Session Manager (notified of account state change)
  ├─ transitions affected sessions: Working → RateLimited
  ├─ emits StateChanged(Working→RateLimited) per session
  │
  └─ evaluates hand-off policy:
        if hand-off allowed → initiates flow 6 (hand-off)
        if not allowed → sessions remain RateLimited until cooldown expires
              on cooldown expiry → RateLimited → Working (resumed)
```

---

## 6. Multi-account selection, switching, and controlled hand-off

**Trigger:** Human selects or switches an account via TUI (`[w]`), CLI (`ac session switch-account`), or Control API (`session.switch_account` / `session.handoff`).

### 6a. Account selection before session start (Idle rebind)

Preferred path when selecting account before agent launch:

```text
Human (TUI / CLI / API)
  │ session.select_account { session_id, account_id }
  ▼
Session Manager
  ├─ verifies session is in Idle state
  ├─ validates account: AccountManager::validate_and_select_explicit
  │     (existence, agent type compatibility, active state, concurrency cap)
  ├─ decrements previous account active_session_count (if previously assigned)
  ├─ increments target account active_session_count
  ├─ updates session.account_id = target_account_id
  ├─ emits AccountSelected { session_id, account_id, previous_account_id }
  └─ returns success (session remains in Idle state ready to launch)
```

### 6b. Switching a running session (Controlled session hand-off)

Triggered on a running session when the provider adapter requires process restart (`AccountSwitchMode::RequiresRestart`):

```text
Human / Policy / API
  │ session.switch_account { session_id, target_account_id }
  ▼
Session Manager
  ├─ inspects adapter capabilities: adapter_factory.capabilities(agent_type)
  │     (Claude Code and CLI adapters declare AccountSwitchMode::RequiresRestart)
  │
  ├─ validates target account (AccountManager::validate_and_select_explicit)
  │     if invalid (not found, incompatible, exhausted) → emits AccountSwitchFailed and returns structured error
  │
  ├─ emits AccountSwitchRequested { session_id, target_account_id, switch_mode }
  │
  ├─ 1. Snapshot capture:
  │     ├─ creates SessionSnapshot (v1) with task description, seq progress, and workspace path
  │     ├─ strictly excludes raw secrets, API tokens, and process memory
  │     └─ emits SessionSnapshotCreated { snapshot_id, version, summary }
  │
  ├─ 2. Stop predecessor process:
  │     ├─ signals live adapter to terminate process cleanly
  │     ├─ State Machine: Working/Paused → Stopping → HandedOff (terminal)
  │     ├─ sets predecessor.successor_id and context_snapshot_id
  │     ├─ decrements predecessor account active_session_count
  │     └─ emits SessionHandedOff { predecessor_id, successor_id, target_account_id }
  │
  ├─ 3. Create successor session:
  │     ├─ initializes new AgentSession (Idle) inheriting workspace_id, project_id, task_description
  │     ├─ sets successor.predecessor_id and context_snapshot_id
  │     ├─ increments target account active_session_count
  │     ├─ emits SessionCreated (successor)
  │     └─ emits AccountSelected (successor)
  │
  └─ 4. Launch successor session:
        ├─ spawns adapter with target credential_ref and restored context snapshot
        ├─ State Machine: Starting → Working
        ├─ emits SessionReady, SessionStarted
        ├─ emits SessionHandOffCompleted { predecessor_id, successor_id, target_account_id }
        └─ emits AccountSwitchCompleted { old_session_id, new_session_id, new_account_id }
```

**Context preservation:**
- **Preserved:** Filesystem workspace (disk changes, uncommitted edits), project configuration, task prompt, audit lineage.
- **Not preserved:** In-memory agent process state (terminal scrollback, process stack). The TUI explicitly presents a confirmation modal detailing preserved vs lost context prior to executing the switch.

---

## 7. Agent crash and recovery

### 7a. Adapter process crash (runtime)

**Trigger:** Adapter process exits unexpectedly while session is active.

```
Session Manager (supervising adapter process)
  │ detects process exit (non-zero exit or SIGKILL)
  ▼
  ├─ emits SessionCrashed { session_id, exit_code?, reason } → Event Store
  ├─ State Machine: Working/WaitingForHuman/Paused → Crashed
  ├─ emits StateChanged(→Crashed)
  │
  ├─ evaluates restart policy:
  │     if session.restart_count < max_restarts and policy allows:
  │       State Machine: Crashed → Restarting
  │       emits SessionRestartAttempted { attempt = restart_count + 1 }
  │       increments session.restart_count
  │       waits back-off delay (exponential: 1s, 2s, 4s … up to cap)
  │       follows flow 1 from "spawns adapter process" step
  │       on success: emits SessionRestartSucceeded → Working
  │
  └─ if restart_count >= max_restarts or policy denies restart:
        State Machine: Crashed → Failed
        emits SessionFailed { reason = "restart_limit_exceeded" }
        TUI shows session as Failed; human must intervene
```

### 7b. Daemon restart (recovery)

**Trigger:** Daemon process is killed or restarts (e.g. system reboot, update).

```
Daemon starts
  │
  ▼
  1. Load configuration
  2. Initialise Event Store connection (SQLite)
  3. Check for latest snapshot
     ├─ load snapshot (account states, session states, interaction states)
     │   bound to snapshot.event_seq
     └─ or start from empty state if no snapshot exists

  4. Replay events from snapshot.event_seq + 1 to end of log
     ├─ each event is re-applied to the in-memory state machines
     ├─ AccountStateChanged → rebuild Account states
     ├─ StateChanged → rebuild AgentSession states
     ├─ InteractionCreated/Resolved → rebuild pending interaction queue
     └─ (already-terminal events like Stopped/Failed are replayed as no-ops
         for the state machine; history is preserved)

  5. Detect orphaned sessions
     ├─ sessions that were Working/Starting/RateLimited at shutdown time
     │   have no live adapter process → mark as Crashed
     └─ apply restart policy (flow 7a from Crashed state)

  6. Accept commands via Control API
     └─ TUI reconnects and refreshes from current state
```

---

## 8. External event consumers & WebSocket Control API (Phase 7)

**Trigger:** External process (AgentDesk, AgentMesh, script) connects via WebSocket to `ws://127.0.0.1:4242`.

```
External consumer (AgentDesk, AgentMesh, automation scripts)
  │ WebSocket connect: ws://127.0.0.1:4242 (with Authorization: Bearer <token> or ?token=<token>)
  ▼
WsServer (tokio-tungstenite)
  ├─ validates token against TokenRegistry → resolves TokenScope (Read, Write, Admin)
  ├─ on invalid token: returns HTTP 401 Unauthorized
  ├─ on missing token: completes handshake in unauthenticated state; awaits in-band 'auth' command
  │
  ├─ client sends: events.subscribe { filter: { kinds?, session_id?, project_id?, account_id?, since_seq? } }
  ├─ returns acknowledgement: { v: 1, id: "...", ok: true, result: {} }
  │
  ├─ [Historical Catch-Up Replay]
  │     if since_seq is provided:
  │       queries SQLite event store for events with seq >= since_seq
  │       filters and streams matching events immediately: { v: 1, event: { ... } }
  │
  ├─ [Live Event Streaming]
  │     subscribes to Event Bus broadcast channel
  │     filters live events against SubscriptionFilter
  │     streams matching events concurrently: { v: 1, event: { ... } }
  │
  └─ [Bidirectional Command Execution]
        client sends: { v: 1, id: "...", cmd: "...", params: { ... } }
        validates TokenScope:
          if TokenScope::Read and is_mutating_cmd(cmd) → returns structured PermissionDenied error
          otherwise → dispatches to dispatch_request(...)
        returns response frame: { v: 1, id: "...", ok: true/false, result/error: { ... } }
```

**AgentDesk (Desktop GUI)**:
Connects using `read` or `write` scope, subscribes to interaction and state events with `since_seq` catch-up, displays user prompts for approval requests, and replies with human decisions.

**AgentMesh (Multi-Agent Fabric)**:
Connects using `admin` or `write` scope to orchestrate multi-agent sessions, monitor pool load, and dynamically steer agent workflows without storing provider credentials.

Both integrations are documented in [`API_CHANGELOG.md`](API_CHANGELOG.md) and [`INTERFACES.md`](INTERFACES.md).

---

## 9. TUI event subscription and real-time updates

**Trigger:** The TUI client launches (`ac tui`) and connects to the Control Core.

```
TUI Client (crates/ac-tui)
  │ connects to Unix socket: /tmp/agentcontrold.sock
  ├─ queries initial state: session.list, account.list, project.list, interaction.list, events.query
  │
  ├─ opens persistent event stream:
  │     events.subscribe { kinds: [] }
  ▼
Control API (IPC server)
  ├─ registers broadcast subscriber channel
  ├─ sends acknowledgement: { ok: true }
  │
  ▼ (continuous background loop)
Event Bus (Core)
  │ emits AgentEvent (e.g. StateChanged, OutputChunk, InteractionCreated, etc.)
  ▼
Control API
  │ serializes event as NDJSON line: { v: 1, event: { ... } }
  ▼
TUI Client (ApiClient)
  ├─ on StateChanged: updates in-memory session.state, refreshes state badge color
  ├─ on OutputChunk: decodes multibyte UTF-8 stream without glyph loss, updates ANSI virtual terminal buffer (terminal_buffer.rs) tracking cursor, scroll regions, colors, and alternate screen
  ├─ on InteractionCreated: triggers alert banner and inserts pending prompt
  ├─ on Interaction*Resolved: marks interaction resolved, updates session state
  ├─ on AgentSteeringReceived: appends steering note to session transcript buffer
  ├─ on key event:
  │     if in session terminal: Esc forwarded directly to agent PTY; Ctrl+Q detaches to dashboard
  │     otherwise: routes to active tab / modal
  └─ renders updated frame to terminal without requiring manual user refresh
```

---

## 10. Normal-user Antigravity launch & account setup (`agy`)

**Trigger:** User runs `agy "Personal Google"` or `agy`.

```
User Terminal
  │ agy "Personal Google"
  ▼
Launcher Client (crates/ac-cli)
  ├─ checks / starts daemon process (setsid detached)
  ├─ connects to Control API IPC socket
  ├─ queries accounts for agent_type "agy" | "antigravity"
  │
  ├─ [Explicit Label Path]:
  │     finds account matching label "Personal Google"
  │     checks status:
  │       - if Incompatible: display clear message and exit
  │       - if Cooldown/Exhausted/Over-limit: explain reason, prompt to pick alternative
  │       - if Ready: proceed to session start
  │
  ├─ [Interactive / Auto Path (agy without argument)]:
  │     - 0 accounts: prompt to login with browser
  │     - 1 account: auto-select "Using account: <label>"
  │     - >1 accounts: render interactive selector menu
  │
  ├─ [Account Addition Flow (agy account add)]:
  │     prompt friendly label
  │     bind ephemeral loopback listener 127.0.0.1:0
  │     open system browser to OAuth authorization URL
  │     await callback or manual authorization code / pasted redirect fallback
  │     save credentials to ~/.config/agentcontrol/credentials/agy_<id>.json (mode 0600)
  │     register account with credential_ref = "ref:antigravity:<id>"
  │
  ├─ [Profile Isolation & Launch]:
  │     prepares ~/.config/agentcontrol/profiles/<account-id>/ with private token
  │     strips ambient Google environment variables
  │     passes validated launch options (mode, permissions, model, working dir)
  ├─ creates & starts session via session.create & session.start
  └─ opens Interactive Session Controller (Ratatui ANSI terminal, [Esc] PTY pass-through, [Ctrl+Q] Detach)
```

---

## Flow summary

| # | Flow | Key state changes |
|---|---|---|
| 1 | Start session | `Idle → Starting → Working` |
| 2 | Agent output / approval request | `Working → WaitingForHuman` |
| 3 | Human approval | `WaitingForHuman → Working` |
| 4 | Human steering | No state change |
| 5 | Account quota exhaustion | `Working → RateLimited`; account `Active → Exhausted` |
| 6 | Account hand-off | `Working → HandedOff`; new session `Idle → Working` |
| 7a | Agent crash (runtime) | `Working → Crashed → Restarting → Working` or `Failed` |
| 7b | Daemon restart (recovery) | Event log replay → state reconstruction |
| 8 | External event consumers | Event subscription, no state changes in core |
| 9 | TUI live updates | Real-time terminal re-rendering driven by `events.subscribe` |
| 10 | Antigravity Launcher UX | `find_by_label → availability_check → session.start → interactive attach` |

