# Agent Control — Future Extensions

This document catalogs extensions that are explicitly out of scope for Agent
Control v1 but are worth preserving as design constraints so that v1
architecture does not foreclose them.

Nothing in this document is a commitment. Each item should be evaluated
independently when the relevant phase is reached.

---

## F1 — Remote agent execution

**What:** Run agent sessions on remote hosts (cloud VMs, CI runners, home lab
servers) and control them from the local daemon via an encrypted tunnel.

**Why deferred:** v1 is local-only. Secure remote execution requires key
exchange, tunnelling, and host trust management — a significant scope addition.

**Design constraints on v1:**
- The adapter contract is already transport-agnostic (in-process channels in
  v1). A remote adapter would replace the channel with an RPC stub.
- Session state machine and event model do not change for remote sessions.
- Workspace and credential handling would need a remote equivalent.

**Dependency:** Requires ADR-008 (dynamic/subprocess adapter loading) and a
new transport layer.

---

## F2 — Multi-user / team mode

**What:** Multiple developers sharing one Agent Control daemon, each with their
own credential namespace, session isolation, and policy set.

**Why deferred:** v1 targets a single developer. Multi-user requires access
control, user identity, and per-user credential isolation — a substantial change
to the security model.

**Design constraints on v1:**
- Unix socket ownership is per-user in v1. Multi-user would use token-based
  auth on the WebSocket transport.
- The event store would need a `user_id` column on all entities.
- Policy Engine would need per-user policy namespaces.

---

## F3 — Web UI

**What:** A browser-based UI served by the daemon on localhost, as an
alternative to the TUI.

**Why deferred:** TUI-first is the v1 choice (see ADR-007). A web UI requires
a frontend framework, asset bundling, and more complex API auth.

**Design constraints on v1:**
- The Control API (commands + event subscription) is already JSON over
  WebSocket. A web UI is just another client.
- No privileged in-process UI path exists; the TUI already uses the same API.

---

## F4 — Filesystem isolation (sandboxing)

**What:** Enforce that agent processes cannot access files outside their
assigned workspace, using Linux namespaces, seccomp-bpf, or landlock.

**Why deferred:** Kernel-level sandboxing increases implementation complexity
and portability concerns. v1 relies on policy-level path checks instead (see
[`SECURITY.md`](SECURITY.md#t6--workspace-boundary-violation)).

**Design constraints on v1:**
- The workspace path is tracked per session; sandboxing would extend this to
  a kernel enforcement boundary.
- Adapters must be designed so that the agent process is spawned as a separate
  PID that can be placed in a restricted namespace.

---

## F5 — Dynamic adapter loading (plugin system)

**What:** Load adapter plugins at runtime without recompiling the daemon, so
that new agent types can be added by dropping a shared library or subprocess
config file.

**Why deferred:** Static linking is simpler and safer for v1 (see ADR-008).

**Design constraints on v1:**
- The adapter contract (see [`INTERFACES.md`](INTERFACES.md#adapter-contract))
  is already defined as an interface. A plugin system would require ABI
  stability guarantees and security review of the plugin isolation model.

---

## F6 — Cost accounting and budget enforcement

**What:** Track token usage and API cost per session, project, and account;
enforce budget limits that stop or pause sessions before exceeding a configured
spend.

**Why deferred:** Quota tracking (rate-limit / exhaustion signals) is in v1.
Cost accounting requires per-provider pricing data, token counting, and a
budget enforcement engine — more scope than v1 warrants.

**Design constraints on v1:**
- `Account` already tracks `quota_reset_at` and concurrency; a cost counter
  could be added to the same entity.
- `RateLimitSignal` could be extended to carry cost metadata.

---

## F7 — Cryptographic audit log signing

**What:** Hash-chain or digitally sign every event record so that any
modification to the audit trail is cryptographically detectable, even by the
process owner.

**Why deferred:** v1 uses sequence-number gap detection only (see ADR-010).
Cryptographic signing adds write overhead and complexity.

**Design constraints on v1:**
- Event records have a stable schema and a monotonic sequence number. Adding
  a `prev_hash` column and a `hash` column per record is a backwards-compatible
  schema extension.

---

## F8 — AgentDesk integration (attention and notifications)

**What:** AgentDesk is a separate project that classifies agent events by
attention priority and sends mobile/desktop notifications.

**Design constraints already met in v1:**
- Control API exposes a stable event subscription endpoint.
- AgentDesk subscribes as an external consumer (see
  [`INTERFACES.md`](INTERFACES.md#external-consumer-integration) and
  [`DATA_FLOW.md`](DATA_FLOW.md#8-external-event-consumers)).
- No Agent Control code changes are needed for AgentDesk to integrate.

**What AgentDesk needs from this doc:**
- The event schema (specifically `StateChanged`, `InteractionCreated`,
  `SessionFailed`) — defined in [`DATA_MODEL.md`](DATA_MODEL.md#agentevent).
- The subscription API — defined in [`INTERFACES.md`](INTERFACES.md#event-subscription).

---

## F9 — AgentMesh integration (multi-agent coordination)

**What:** AgentMesh is a separate project that decomposes tasks into
sub-tasks and coordinates multiple agent sessions to execute a plan.

**Design constraints already met in v1:**
- Control API allows external clients to issue `session.start`, `session.steer`
  commands.
- AgentMesh subscribes to session events and issues commands as a client (see
  [`INTERFACES.md`](INTERFACES.md#external-consumer-integration)).
- No Agent Control code changes are needed for AgentMesh to integrate.

**What AgentMesh needs from this doc:**
- Session start and steer command schemas — in [`INTERFACES.md`](INTERFACES.md).
- Session state events — in [`DATA_MODEL.md`](DATA_MODEL.md).
- Hand-off flow — in [`DATA_FLOW.md`](DATA_FLOW.md#6-account-and-session-hand-off).

---

## F10 — Session templates and saved configurations

**What:** Reusable session templates that pre-configure agent type, account
tags, policy set, and initial task prompt, so a developer can start a
particular kind of session with a single command.

**Why deferred:** Simple enough to add post-MVP; not required for the core
control plane to work.

---

## F11 — Continuous session mode (long-running autonomous sessions)

**What:** Sessions that run indefinitely with automatic task re-queuing:
when the agent completes a task, it is immediately given the next task from
a queue without human-initiated restart.

**Why deferred:** Requires a task queue entity and a task routing policy.
v1 sessions are single-task; the human decides when to start a successor.

**Design constraints on v1:**
- `Task` entity is already separate from `AgentSession`.
- Session completion (`Stopped` state) could trigger a policy evaluation
  that starts a successor with the next task.

---

## F12 — Metrics and observability export

**What:** Export session metrics (session count, state distribution, approval
rate, hand-off frequency) to Prometheus or a structured log sink.

**Why deferred:** The event stream already contains all the data needed;
v1 exposes it via the subscription API. A metrics exporter is a thin
client on top of that stream.

---

## F13 — Mobile companion app

**What:** A mobile app that receives notifications and allows basic session
control (pause, approve, steer) from a phone.

**Why deferred:** This is the core mission of AgentDesk, not Agent Control.
Agent Control's contribution is the event stream and command API. AgentDesk
owns the notification and mobile UX layer.

---

## Summary

| ID | Extension | Blocking constraint |
|---|---|---|
| F1 | Remote agent execution | ADR-008; new transport layer |
| F2 | Multi-user / team mode | Auth model change; event store schema |
| F3 | Web UI | Control API already supports it as a client |
| F4 | Filesystem sandboxing | Adapter process spawn model |
| F5 | Dynamic adapter plugins | ABI stability; security review |
| F6 | Cost accounting | Provider pricing data; budget enforcement |
| F7 | Cryptographic audit signing | Event schema extension; ADR-010 |
| F8 | AgentDesk integration | No v1 changes needed |
| F9 | AgentMesh integration | No v1 changes needed |
| F10 | Session templates | Post-MVP convenience |
| F11 | Continuous session mode | Task queue entity |
| F12 | Metrics export | Event stream client |
| F13 | Mobile companion | AgentDesk responsibility |
