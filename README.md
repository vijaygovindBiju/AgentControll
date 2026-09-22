# Agent Control

A local-first control plane for coding agents.

Agent Control is a single daemon that owns the lifecycle of coding-agent sessions
(Claude Code, Codex, Devin CLI, agy, …), the pool of provider accounts they run
under, the projects and workspaces they operate in, and the human ↔ agent
interaction loop (questions, approvals, steering).

> **Status:** Phase 0 (Design), Phase 1 (Core daemon & CLI), and Phase 2 (Account Manager & Project Registry) are complete. Phase 3 (Interaction Hub & Policy Engine) is next.

---

## What it does

- Start, attach, pause, resume, stop and supervise agent sessions.
- Keep one authoritative state per session (`Idle`, `Working`,
  `WaitingForHuman`, `RateLimited`, …).
- Manage a pool of provider accounts: quota signals, cooldowns, concurrency
  caps, rotation, and hand-off of a session from one account to another.
- Register projects and bind sessions to isolated workspaces (worktrees).
- Provide one inbox for everything an agent asks a human, with declarative
  policies for safe auto-approval and a hard human-approval boundary for
  destructive actions.
- Persist every event to an append-only SQLite log and expose a local Control
  API (commands + event subscription) for the TUI, CLI and external consumers.

---

## What it deliberately does not do

| Concern | Owner |
|---|---|
| Attention scoring, mobile summaries, phone notifications | **AgentDesk** (separate project) |
| LLM-driven task decomposition, multi-agent planning and coordination | **AgentMesh** (separate project) |

Agent Control has no dependency on either. Both can integrate later as clients
of the public Control API. See [`docs/PROJECT.md`](docs/PROJECT.md#boundaries).

---

## Architecture at a glance

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
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Layers, modules and their responsibilities |
| [`docs/DATA_MODEL.md`](docs/DATA_MODEL.md) | Entities, session state machine, account states |
| [`docs/DATA_FLOW.md`](docs/DATA_FLOW.md) | Conceptual runtime flows |
| [`docs/INTERFACES.md`](docs/INTERFACES.md) | Control API, adapter contract, error model (conceptual) |
| [`docs/SECURITY.md`](docs/SECURITY.md) | Threat model and security controls |
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
