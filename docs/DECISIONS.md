# Agent Control — Architecture Decision Log

This document records significant architecture decisions, their rationale, and
their status. Decisions marked **Open** have not been finalized and must be
resolved before the phase that depends on them begins.

Decisions are numbered ADR-001 onwards. New decisions are appended.

---

## ADR-001 — Local-first single daemon

**Status:** Accepted

**Decision:** Agent Control runs as a single daemon process on the developer's
local machine. There is no cloud component required for any v1 feature.

**Rationale:**
- The target user is a single developer on a laptop or workstation, not a team.
- Local-only eliminates network latency, availability dependencies, and the
  complexity of cloud auth for a tool that manages credentials.
- A single daemon means one authoritative source of state — no distributed
  consensus problem.
- Remote and multi-user operation is explicitly future work (see
  [`FUTURE.md`](FUTURE.md)).

**Consequences:**
- Multi-user or multi-machine use requires a future extension.
- The daemon binary must be stable and resilient to crash/restart since there
  is no redundancy.

---

## ADR-002 — SQLite append-only event log as source of truth

**Status:** Accepted

**Decision:** All state changes are persisted as events in an append-only SQLite
table with monotonically increasing sequence numbers. Derived state (in-memory
session and account states) is rebuilt from this log on startup.

**Rationale:**
- Event sourcing provides a complete, replayable audit trail with no separate
  audit infrastructure needed.
- SQLite is universally available, requires no separate server, has excellent
  single-writer performance, and is well-understood.
- Append-only discipline makes the log tamper-evident (gaps are detectable)
  and simplifies the persistence layer.
- Periodic snapshots keep recovery time bounded without sacrificing history.

**Consequences:**
- Event schemas must be versioned; breaking changes require migration on replay.
- `UPDATE`/`DELETE` on the events table must be prohibited at the ORM layer.
- The SQLite file is a critical asset; its permissions must be `0600`.

**Alternatives considered:**
- Postgres: rejected — requires a server process; too heavy for local-only v1.
- Pure in-memory with JSON export: rejected — loses history across restarts.
- LMDB: considered but SQLite has better tooling and developer familiarity.

---

## ADR-003 — Transport-agnostic adapter contract

**Status:** Accepted

**Decision:** The adapter contract is an in-process message channel interface,
not a network protocol. Each adapter is a library that the daemon loads, not a
separate process the daemon calls over HTTP.

**Rationale:**
- Eliminates a network hop and serialization overhead for every agent event.
- Keeps the adapter as thin as possible: it translates I/O, it does not own
  a server.
- External agent processes (Claude Code, Codex) are the things being wrapped;
  they are child processes of the adapter, not the adapter itself.

**Consequences:**
- Adapters are compiled into (or dynamically loaded by) the daemon binary. A
  new adapter requires a daemon rebuild/restart (unless dynamic loading is used;
  see ADR-008).
- The adapter contract must remain stable across versions; breaking changes
  require a version bump.

---

## ADR-004 — Structured adapter preferred, PTY mandatory as fallback

**Status:** Accepted

**Decision:** Every adapter must implement PTY fallback. If the agent exposes
structured output (hooks, NDJSON, API), structured mode is preferred and
produces `High`-confidence events. PTY mode is mandatory to ensure coverage
of agents that offer no structured interface.

**Rationale:**
- Not all coding agents expose structured APIs. PTY is the least-common-denominator.
- Structured mode, where available, is unambiguous and simpler to maintain.
- Dual mode with explicit confidence markers lets operators know when they are
  relying on pattern matching versus structured data.

**Consequences:**
- Every adapter ships with a PTY pattern file even if a structured mode exists.
- Confidence markers must be surfaced in the TUI.
- Pattern files need updating when agent output formats change; this is
  maintenance overhead.

---

## ADR-005 — Authoritative session state machine in the core

**Status:** Accepted

**Decision:** The session state machine lives in the Control Core and is the
single authority for valid state transitions. Adapters propose transitions via
events; the core accepts or rejects them. Adapters cannot directly set session
state.

**Rationale:**
- Centralised state machine prevents state divergence between adapter and core.
- Invalid transitions are recorded as `TransitionRejected` events, providing
  diagnostics without corrupting state.
- The state machine is fully testable in isolation using the Mock adapter.

**Consequences:**
- Adapters must not assume their proposed transition was applied; they must
  wait for the core's `StateChanged` event.
- The transition table is the authoritative definition of legal session
  behaviour; changes to it are breaking changes.

---

## ADR-006 — Credential store integration (credential_ref indirection)

**Status:** Accepted — resolved during Phase 2 (2026-09-21)

**Decision:** Account entities hold a `credential_ref` (an indirection identifier such as a key alias, URI, or secret-manager path), never raw secrets. Raw credentials are never written to the SQLite database, event store, or Control API messages. Adapters resolve credentials from their local runtime environment or OS store at session execution time.

**Rationale:**
- Prevents secret leakage into the append-only event log, audit trail, and snapshot storage.
- Keeps the Control Core decoupled from platform-specific secret store APIs (e.g. headless vs desktop keyring).
- Conforms to the zero-secret-in-logs security invariant defined in [`SECURITY.md`](SECURITY.md).

**Consequences:**
- When registering an account via the CLI or IPC, operators pass a `credential_ref` identifier rather than an API key.
- Adapters are responsible for resolving `credential_ref` when launching agent child processes.

---

## ADR-007 — Implementation language and TUI framework

**Status:** Accepted — resolved 2026-09-19 before Phase 1 implementation began

**Decision:** Rust core + Ratatui TUI framework.

- **Core daemon:** Rust
- **CLI:** Rust (same workspace, separate crate)
- **TUI:** Ratatui
- **Persistence:** SQLite via `rusqlite`
- **Architecture:** Local-first single daemon

**Rationale for Rust:**
- Strong PTY and process control libraries.
- Memory safety without GC pauses (important for a supervisor daemon).
- Consistent with sibling projects.
- SQLite binding (`rusqlite`) is mature.
- `tokio` async runtime handles concurrent sessions and socket I/O cleanly.

**Rationale for TUI-first:**
- TUI is accessible from SSH sessions and headless environments.
- Faster to implement and iterate than a web UI.
- Web UI is explicitly future work.

**Alternatives considered:**
- Go: good process control, good SQLite, simpler concurrency model; weaker
  type system; less mature TUI ecosystem.
- Python: rapid development, good tooling; GC pauses; weaker for PTY
  management at scale.
- Dart/Flutter: native GUI with good UX; poor PTY/process control; heavier
  binary size; not TUI-friendly.

---

## ADR-008 — Static vs. dynamic adapter loading

**Status:** Accepted — resolved during Phase 4 implementation (2026-09-22)

**Decision:** Adapters are statically compiled into the daemon binary via the `AdapterFactory` trait and `CompositeAdapterFactory`. Supported adapters (Claude Code, Generic PTY, Mock) are included directly in `ac-core`. Dynamic plugin loading is deferred to future v2 extensions.

**Rationale:**
- Maximum simplicity and reliability: no dynamic linker dependencies, symbol versioning, or fragile C ABI wrappers.
- Subprocess isolation is maintained for the agents themselves: Claude Code and PTY processes are spawned as supervised child processes with strict stdio/PTY handling and workspace isolation.
- Fast startup and zero runtime overhead for adapter instantiation.

**Consequences:**
- Adding a new adapter requires adding a module to `ac-core` and recompiling the daemon.
- `CompositeAdapterFactory` provides unified routing by `agent_type` across all compiled adapters.
- Dynamic plugin/subprocess adapters remain an option for future v2 releases.

---

## ADR-009 — WebSocket loopback transport for external consumers

**Status:** Accepted — resolved during Phase 7 implementation (2026-09-22)

**Decision:** Built directly into the daemon (`agentcontrold`) on a configurable loopback address (default `127.0.0.1:4242`), using `tokio-tungstenite`.

**Rationale:**
- A single unified daemon binary avoids operational overhead, inter-process connection management, and synchronization latency.
- Strict loopback binding (`127.0.0.1`) guarantees no remote exposure on host interfaces.
- Token authentication (`TokenRegistry`) enforces scoped authorization (`read`, `write`, `admin`) for local consumers (AgentDesk GUI, AgentMesh multi-agent fabric).
- Shared request dispatching (`dispatch_request`) ensures complete functional and validation parity between Unix domain socket IPC and WebSocket transports.

**Consequences:**
- External consumers connect via `ws://127.0.0.1:4242` with bearer token or query string `?token=...`.
- Replay catch-up and granular subscription filtering are supported on both transports.
- No separate gateway binary required.

---

## ADR-010 — Event log checksum / integrity verification

**Status:** Open — informational, no phase blocker

**Decision pending:** Should the event log include periodic checksums or
signatures to detect offline tampering?

**Context:** Sequence-number gaps are already detectable. The question is
whether to go further with cryptographic integrity (e.g. hash-chaining each
event record).

**Options:**

| Option | Pros | Cons |
|---|---|---|
| Sequence-number gap detection only | Already implemented as part of ADR-002; simple | Does not detect value tampering (only deletion) |
| Hash-chained records | Detects any record modification | 10–20% write overhead; complexity in migration |
| Periodic checkpoint signatures | Lightweight; operator-verifiable | Gaps between checkpoints are not covered |

**Note:** Cryptographic signing of the audit log is listed as a future extension
in [`FUTURE.md`](FUTURE.md). The baseline (gap detection) is sufficient for
v1.

---

## ADR-011 — `Idle → Stopped` direct transition (no Stopping intermediate)

**Status:** Accepted — resolved during Phase 1 implementation (2026-09-19)

**Decision:** The `Idle → Stopped` transition is valid and bypasses `Stopping`.
When a session is stopped before it has been started, the Session Manager
transitions it directly from `Idle` to `Stopped` without emitting a `Stopping`
transition.

**Rationale:**
- An `Idle` session has no active adapter process. There is nothing to shut
  down gracefully.
- Routing through `Stopping` would imply "waiting for a process to exit" when
  no process exists. This is semantically incorrect and would clutter the event
  log with a meaningless `StateChanged(Idle → Stopping)` event.
- Stopping an unstarted session is a valid operator action (e.g., cancelling a
  session that was queued but never launched).

**Implementation:**
- `is_valid_transition(Idle, Stopped)` returns `true` in `state_machine.rs`.
- `SessionManager::cmd_stop` checks if the current state is `Idle` before
  deciding whether to emit the `Stopping` intermediate transition:
  - If `Idle`: emits `SessionStopRequested`, then transitions directly to
    `Stopped`.
  - Otherwise: emits `SessionStopRequested`, transitions to `Stopping`, then
    to `Stopped`.

**Consequences:**
- The `Stopping` state remains meaningful: it only appears when a running
- adapter process is being shut down.
- The transition table in [`DATA_MODEL.md`](DATA_MODEL.md) documents this
  explicitly.

---

## ADR-012 — Policy Evaluation Precedence and Determinism

**Status:** Accepted — resolved during Phase 3 implementation (2026-09-22)

**Decision:** The Policy Engine evaluates actions deterministically using a strict precedence order:
1. **Never-auto-approve boundary**: Hardcoded security checks (e.g. `rm`, `sudo`, `curl`, `wget`, `git push --force`, `publish`) ALWAYS evaluate to `RequireHuman`. No user rule (Allow or Deny) can override this boundary.
2. **Matching Deny rules**: Highest priority first.
3. **Matching Allow rules**: Highest priority first.
4. **Matching RequireHuman rules**: Highest priority first.
5. **Fail-safe fallback**: If no rule matches, default to `RequireHuman`.

**Rationale:**
- AI agent systems must have fail-safe, defense-in-depth security (SECURITY.md T3).
- Hard boundaries prevent malicious or accidental wildcard auto-approvals from compromising host security or destroying data.
- Deny rules overriding Allow rules prevents accidental bypasses when multiple policies match.
- Fallback to `RequireHuman` ensures any unexpected or unconfigured tool invocation prompts the operator rather than executing silently.

**Consequences:**
- Rule evaluation is 100% deterministic (no LLMs in the critical policy evaluation path).
- Auto-approvals are restricted to safe, explicitly whitelisted actions that do not trigger the never-auto-approve boundary.
- All evaluations, decisions, overrides, and resolutions are recorded in the immutable audit log table.

---

## ADR-013 — Multi-Account Selection, Switching, and Controlled Session Hand-Off

**Status:** Accepted — resolved during Phase 6 implementation (2026-09-22)

**Decision:**
1. **Honest Provider Capabilities:** Coding agents running as subprocesses (such as Claude Code and interactive CLI agents) cannot safely change authenticated identities in-memory while running. Adapters report `ProviderCapabilities { switch_mode: AccountSwitchMode::RequiresRestart, .. }`. Agent Control performs controlled session hand-off rather than faking in-process credential swaps.
2. **Concept Separation:**
   - `Account`: Provider identity, metadata, concurrency caps, tags, and indirect `credential_ref`. Contains zero raw credentials.
   - `Credential`: External secret referenced indirectly via ADR-006 tokens (`ref:<id>`). Never enters persistence, events, TUI, audit logs, or session snapshots.
   - `AgentSession`: Running instance bound to an account. Maintains lineage (`predecessor_id`, `successor_id`, `context_snapshot_id`).
   - `ProviderAdapter`: Boundary responsible for resolving `credential_ref` and injecting it into the spawned child process.
3. **Deterministic Selection:** Explicit selection validates existence, compatibility, state, and concurrency cap. Automatic selection filters active, compatible accounts and tie-breaks deterministically by least load (`active_session_count`), then label, then ID. No LLM in the critical selection path.
4. **Switching Paths:**
   - *Idle Session:* In-place rebinding of `account_id` and load tracking before agent launch.
   - *Running Session:* Controlled session hand-off. The predecessor session captures a snapshot, terminates cleanly, transitions to `SessionState::HandedOff`, and decrements its account load. A successor session is created inheriting workspace path, project, and task description, linked via `predecessor_id`/`successor_id`, and started under the target account.
5. **Session Snapshot Model:** Versioned provider-neutral snapshot (`SessionSnapshot`, version 1) capturing safe execution state, task descriptions, and workspace paths while strictly excluding credentials, environment secrets, and arbitrary process memory.

**Rationale:**
- Faking continuous in-process authentication changes when the underlying CLI cannot support it leads to silent failures, token corruption, and security boundary violations.
- Controlled session hand-off provides transparent, auditable transitions with explicit lineage.
- Strict secret exclusion in snapshot serialization guarantees that state hand-offs cannot leak authentication tokens into SQLite or audit logs.

**Consequences:**
- The predecessor and successor sessions remain distinct entities with linked IDs, enabling full traceability in the audit log.
- Workspace disk state is preserved across hand-offs; uncommitted file edits remain intact.
- The TUI clearly warns users that in-memory process context cannot be preserved before confirming restarts.

---

## Open decisions summary

| ADR | Decision | Blocking phase |
|---|---|---|
| ~~ADR-006~~ | ~~Credential store integration~~ | ~~Phase 2~~ — **Resolved: credential_ref indirection** |
| ~~ADR-007~~ | ~~Implementation language and TUI framework~~ | ~~Phase 1~~ — **Resolved: Rust + Ratatui** |
| ~~ADR-008~~ | ~~Static vs. dynamic adapter loading~~ | ~~Phase 4~~ — **Resolved: static linking** |
| ADR-009 | WebSocket transport architecture | Phase 7 |
| ADR-010 | Event log integrity verification | No blocker |
| ~~ADR-011~~ | ~~`Idle → Stopped` direct transition~~ | ~~Phase 1~~ — **Resolved: accepted** |
| ~~ADR-012~~ | ~~Policy evaluation precedence & determinism~~ | ~~Phase 3~~ — **Resolved: accepted** |
| ~~ADR-013~~ | ~~Multi-account switching & controlled hand-off~~ | ~~Phase 6~~ — **Resolved: accepted** |

All open decisions must be recorded as resolved in this document before
implementation of their blocking phase begins.
