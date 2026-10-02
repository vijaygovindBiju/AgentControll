# Agent Control — Project

## Problem

A developer running several coding agents at once — across several
repositories and several provider accounts — has no single place to:

- see which agent is doing what, in which repository, under which account;
- pause, resume or stop an agent;
- answer an agent's questions and approve or deny its tool calls;
- hand a task from one agent or account to another when a quota runs out;
- review afterwards what an agent did and who approved what.

Each agent CLI has its own terminal, its own login, its own approval prompt and
its own idea of "state". Coordination happens in the developer's head.

## Target user

A single developer (or a very small team sharing one machine) who runs one to
roughly ten coding-agent sessions in parallel on a laptop or workstation,
with more than one provider account available, and who wants to *control*
those sessions rather than merely watch them.

## Proposed solution

**Agent Control** is a local-first control plane: one daemon that owns

1. **Sessions** — start, attach, pause, resume, stop, supervise, restart.
2. **Accounts** — credential pool, quota and rate-limit tracking, cooldowns,
   concurrency caps, rotation and hand-off.
3. **Projects and workspaces** — registry of repositories, default agent and
   account per project, worktree/branch policy, per-project rules.
4. **Human interaction** — a single inbox for agent questions and approval
   requests, replies, steering instructions, and a declarative policy engine
   that auto-decides only within a configured allowlist.
5. **State and history** — an authoritative session state machine and an
   append-only event log that can be replayed and audited.
6. **A public interface** — a local Control API (commands + event stream) used
   by the TUI and CLI and available to external tools.

## Core concept

```text
Human (TUI / CLI)
   │  commands                       ▲ events
   ▼                                 │
Control Core ── Session Manager ── State Machine ── Event Store (SQLite)
   │  Account Manager · Project Registry · Interaction Hub · Policy Engine
   ▼
Adapters (Claude Code | Codex | Devin CLI | agy | Generic PTY | Mock)
   ▼
Coding agent processes, each pinned to a workspace and an account
```

The `agy` adapter refers to the external Antigravity executable; AgentControll
does not install, package, or replace that command. Antigravity account
authentication and account switching remain available through the AgentControll
TUI.

## Principles

- **Control, not observation.** Every piece of state exists so that a human or
  a policy can act on it.
- **One authoritative state per session.** Adapters *propose* transitions via
  events; the core decides.
- **Local-first.** No cloud component is required for any v1 feature.
- **Accounts are resources, not configuration.** They are first-class,
  rotatable, and decoupled from sessions.
- **Humans own destructive decisions.** Policies can widen auto-approval only
  inside a configured allowlist; some action classes can never be auto-approved.
- **Everything is an event.** State transitions, decisions and commands are
  appended to a replayable log.
- **Adapters are thin.** Agent-specific knowledge stays in the adapter layer;
  the core never imports agent-specific logic.

## Boundaries

Agent Control lives next to two related but separate projects. The
responsibilities are split as follows and must not be merged.

| Concern | Owner |
|---|---|
| Launching, supervising, pausing, resuming, stopping agent processes/sessions | **Agent Control** |
| Account/credential pool, quota tracking, cooldowns, rotation, hand-off | **Agent Control** |
| Project registry, workspace/worktree assignment | **Agent Control** |
| Human ↔ agent conversation, approvals, steering, audit | **Agent Control** |
| Authoritative session state machine and event log | **Agent Control** |
| Attention classification/scoring, progressive disclosure, phone notifications | **AgentDesk** |
| LLM-driven task decomposition, dependency graphs, multi-agent planning and coordination | **AgentMesh** |

Rules:

- Agent Control does **not** depend on AgentDesk or AgentMesh, at build time or
  at runtime.
- AgentDesk and AgentMesh may later integrate as **clients** of the Control API
  (see [`INTERFACES.md`](INTERFACES.md) and
  [`DATA_FLOW.md`](DATA_FLOW.md#8-external-event-consumers)). They never
  become dependencies of the core.
- Agent Control does not rank events for attention and does not plan tasks. If
  a feature requires either, it belongs to the other project.

## Goals

- Run, pause, resume and stop several agent sessions of different agent types
  from one interface.
- Never lose a question or approval request from any session; surface each
  exactly once in the inbox.
- Survive daemon restarts and agent crashes with the session history intact.
- Hand a task from an exhausted account to a fresh one with its context
  preserved, without human re-typing.
- Provide an adapter seam that lets a new agent type be added without changing
  the core.
- Expose a stable, versioned event/command interface for external consumers.

## Non-goals (v1)

- Cloud or multi-user/multi-tenant operation.
- LLM-based task planning or decomposition (AgentMesh).
- Attention scoring or mobile/phone UI (AgentDesk).
- Running agents on remote hosts (documented as future work).
- Web UI (future work; TUI and CLI first).
- Cost accounting beyond what is needed for quota tracking.
- Being a full remote terminal; terminal attach is a convenience, not the
  primary interface.

## MVP

Phases 1–3 of [`ROADMAP.md`](ROADMAP.md) form the MVP:

```text
Core daemon (event store, state machine, session manager) + Mock adapter + CLI
   → projects and account pool with selection and cooldown
   → interaction hub, policy engine, approval round-trip, audit log
```

The MVP is deliberately validated against the Mock adapter so that the core's
behaviour is fully deterministic before any real agent is connected (Phase 4).

## Success criteria

The project is successful when, with at least two real adapters connected:

1. Three or more sessions across two or more projects and two or more accounts
   can be started, paused, resumed and stopped from the TUI, and their state is
   always consistent with the state machine in
   [`DATA_MODEL.md`](DATA_MODEL.md#agentsession-state-machine).
2. Every `ApprovalRequested` and `Question` event produces exactly one
   `Interaction`, and every interaction is resolved by a human or an explicitly
   matching policy, with an `AuditEntry` for each decision.
3. A simulated quota exhaustion triggers an account cooldown and — when the
   policy allows — a hand-off to another account with the task and context
   snapshot preserved.
4. Killing the daemon and restarting it reconstructs all session states from
   the event log; killing an agent process is detected and handled by the
   restart policy.
5. A secret placed in an agent's environment never appears in the event store,
   the audit log or the TUI.
6. A normal user can launch Antigravity with a single friendly command (`agy "Personal Google"` or `agy`), switch accounts transparently, and authenticate via browser OAuth without manual token management or shell scripting.
7. Full-screen ANSI virtual terminal emulation allows interactive terminal agents (such as `agy`) to render cleanly with alternate screen buffers, forward `Esc` directly to the agent PTY, detach using `Ctrl+Q`, and stream without back-pressure deadlocks.
8. A centralized Settings view unifies configuration, account management, and project management with keyboard navigation and persistence to `~/.config/agentcontrol/settings.json`.
9. All of the above is covered by tests described in
   [`TESTING.md`](TESTING.md).
