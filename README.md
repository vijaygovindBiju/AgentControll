# Agent Control

A local-first control plane for coding agents.

Agent Control is a single daemon that owns the lifecycle of coding-agent sessions
(Claude Code, Codex, Devin CLI, agy, …), the pool of provider accounts they run
under, the projects and workspaces they operate in, and the human ↔ agent
interaction loop (questions, approvals, steering).

> **Status:** All phases (Phases 0–8 + User-Friendly Launcher & Account UX) are complete and fully tested with 100% test pass rate and 0 warnings.

---

## User-Friendly Experience (Quickstart)

Agent Control provides a clean **two-layer CLI architecture**:
1. **Normal-User Layer (`agent-control`, `agy`):** Frictionless daily workflows using friendly account labels (`"Personal Google"`, `"Work Google"`), interactive selectors, and automated browser authentication.
2. **Power-User / Automation Layer (`ac`):** Low-level scriptable subcommands for session supervisors, account registries, policy inspection, and audit event logs.

### 1. Main Interactive Dashboard

Launch the unified terminal dashboard:
```bash
agent-control
```
Displays:
* **AGENTS**: Registered agent types, account counts, and active session counts.
* **ACCOUNTS**: Friendly account labels, associated agent types, and real-time statuses (`Ready`, `Cooldown`, `Exhausted`).
* **SESSIONS**: Active sessions with agent type, account label, project, and lifecycle badge (`Working`, `WaitingForHuman`, `Paused`).
* **Keybindings**: `[Enter] Open`, `[A] Add Account`, `[N] New Agent`, `[S] Sessions`, `[Q] Quit`.

---

### 2. Antigravity Launcher (`agy`)

Launch Antigravity without worrying about low-level IDs or manual login/logout:

```bash
# Launch with a specific account label
agy "Personal Google"

# Or simply run agy:
# - If 1 account: automatically launches
# - If multiple: opens interactive selector with real-time status & cooldowns
# - If 0 accounts: prompts to authenticate via browser
agy
```

Interactive Agent Control view:
```text
╭────────────────────────────────────────────╮
│ Antigravity                                │
│ Account: Personal Google                   │
│ State: Working                             │
├────────────────────────────────────────────┤
│                                            │
│ Agent output transcript...                 │
│                                            │
│ > Inspecting project...                    │
│ > Running tests...                         │
│                                            │
├────────────────────────────────────────────┤
│ [s] Steer  [p] Pause  [r] Resume  [x] Stop│
│ [a] Switch Account   [q] Quit             │
╰────────────────────────────────────────────╯
```

Add a new account via friendly browser authentication:
```bash
agy account add
```

---

Antigravity account management (see [docs/ANTIGRAVITY_ACCOUNTS.md](docs/ANTIGRAVITY_ACCOUNTS.md)):

```bash
agy account add                      # browser sign-in (or import the local agy login)
agy account list                     # names, IDs, status, credential validity
agy "College Google"                 # launch as that account (exact name or account ID)
agy account remove "College Google"  # remove account, credential and profile
```

Each account runs `agy` in its own profile; Agent Control never falls back to the machine's default agy login.

### 3. Power-User / Automation CLI (`ac`)

Keep granular, scriptable control over daemon subsystems:
```bash
ac account list
ac session list
ac session steer <session-id> --instruction "Focus on auth tests"
ac session pause <session-id>
ac session resume <session-id>
ac session stop <session-id>
ac events tail
```

```text
Human (TUI / CLI)
   │  commands                          ▲  events
   ▼                                    │
Control Core ─── Session Manager ─── Agent State Machine ─── Event Store (SQLite)
   │
   ├── Account Manager
   ├── Project Registry
   ├── Interaction Hub ◀──▶ Policy Engine
   └── Event Bus
   │
   ▼  Adapter Contract
Adapters  (Claude Code | Codex | Devin CLI | agy | Generic PTY | Mock)
   │
   ▼
Coding agent processes  (cwd = Workspace, env = Account credentials)
```

External consumers (AgentDesk, AgentMesh, scripts) connect to the Control API
as clients — they never become internal dependencies.

---

## Documentation

| Document | Content |
|---|---|
| [`docs/PROJECT.md`](docs/PROJECT.md) | Problem, target user, scope, boundaries, non-goals, success criteria |
| [`docs/CLI_REFERENCE.md`](docs/CLI_REFERENCE.md) | **Comprehensive CLI Reference Manual** — all commands, subcommands, options, and recipes |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Layers, modules and their responsibilities |
| [`docs/DATA_MODEL.md`](docs/DATA_MODEL.md) | Entities, session state machine, account states |
| [`docs/DATA_FLOW.md`](docs/DATA_FLOW.md) | Conceptual runtime flows |
| [`docs/INTERFACES.md`](docs/INTERFACES.md) | Control API, adapter contract, error model (conceptual) |
| [`docs/SECURITY.md`](docs/SECURITY.md) | Threat model and security controls |
| [`docs/ANTIGRAVITY_ACCOUNTS.md`](docs/ANTIGRAVITY_ACCOUNTS.md) | Antigravity accounts: add, select, switch, remove, profiles, expiry, troubleshooting |
| [`docs/TESTING.md`](docs/TESTING.md) | Testing and validation strategy |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Phases 0–8, MVP definition, exit criteria |
| [`docs/DECISIONS.md`](docs/DECISIONS.md) | Architecture decision log, including open decisions |
| [`docs/FUTURE.md`](docs/FUTURE.md) | Future extensions beyond v1 |

---

## Glossary

| Term | Definition |
|---|---|
| **Agent** | An external coding-agent program (CLI or API-driven). |
| **Adapter** | The Agent Control component that wraps one agent type and translates its I/O into the common event model. |
| **Session** | One running (or paused/stopped) instance of an agent, bound to a workspace, an account and a task. |
| **Account** | One set of provider credentials with its own quota, rate limits and concurrency cap. |
| **Project** | A registered repository with its own default agent type, account group and workspace policy. |
| **Workspace** | A specific on-disk working directory for a session — either the shared project checkout or a dedicated worktree on a branch. |
| **Task** | A human-supplied description of what one session is expected to accomplish; attached to a session and included in hand-offs. |
| **Hand-off** | Moving a session's task and context snapshot to a new session on a different account, preserving continuity. |
| **Interaction** | A question or approval request raised by an agent, and the human's or policy's answer. |
| **Policy** | A declarative rule that the Policy Engine evaluates against an interaction or session condition to produce Allow, Deny, or Escalate. |
| **Audit entry** | An immutable record of every policy decision or human action, written to the event log. |
| **Control API** | The local Unix-socket / loopback WebSocket interface through which all commands are sent and all events are streamed. |
| **Event Bus** | The in-process publish/subscribe backbone; every state change is an event. |
| **Event Store** | The append-only SQLite table that persists every event with a sequence number and version. |
