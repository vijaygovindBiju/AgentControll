# Agent Control — Roadmap

This document defines the development phases from Phase 0 (design) through
Phase 8 (stable platform). Phases 1–3 form the **MVP**. Each phase lists its
deliverables and the exit criteria that must be met before the next phase begins.

> **Status:** All phases (Phase 0 through Phase 8) are complete. Agent Control v1.0.0 is released!

---

## MVP boundary

Phases 1, 2, and 3 together form the MVP. The MVP is considered complete when:

- The daemon starts, recovers from restart, and exposes the Control API. ✅
- Sessions can be started, paused, resumed and stopped using the Mock adapter. ✅
- Projects and accounts are registered and the account selection strategy works. ✅
- Interactions reach the inbox, are auto-resolved by policy or pending for human
  approval, and every decision produces an audit entry. ✅
- All MVP behaviour is covered by tests described in
  [`TESTING.md`](TESTING.md). ✅ (124/124 tests passing, 0 warnings).

The MVP is deliberately validated against the Mock adapter only so that the
core is fully deterministic before real agents are connected.

---

## Phase 0 — Design and documentation

**Goal:** Produce a complete implementation blueprint before writing any code.

Deliverables:
- `README.md` — project overview, glossary, document index.
- `docs/PROJECT.md` — problem, scope, boundaries, goals, non-goals, success criteria.
- `docs/ARCHITECTURE.md` — modules, layers, responsibilities.
- `docs/DATA_MODEL.md` — all entities, state machines.
- `docs/DATA_FLOW.md` — all conceptual runtime flows.
- `docs/INTERFACES.md` — Control API and adapter contract (conceptual).
- `docs/SECURITY.md` — threat model and security controls.
- `docs/TESTING.md` — testing strategy and test categories.
- `docs/ROADMAP.md` — this document.
- `docs/DECISIONS.md` — architecture decision log with open decisions.
- `docs/FUTURE.md` — future extensions beyond v1.

Exit criteria:
- All documents above exist and cross-reference each other correctly.
- Open architectural decisions are explicitly recorded as open in `DECISIONS.md`.
- All open decisions that block Phase 1 are identified.

**Status:** ✅ Complete.

---

## Phase 1 — Core daemon and CLI (MVP part 1)

**Goal:** A working daemon with the event store, state machine, session manager
(Mock adapter only), and a functional CLI.

Deliverables:
- Daemon binary: startup, configuration loading, Control API socket.
- Event store: SQLite append-only table, sequence numbers, schema versioning.
- `AgentSession` state machine: all states and transitions enforced.
- Session Manager: create/start/pause/resume/stop sessions via Mock adapter.
- Mock adapter: scripted event sequences, controllable by tests.
- Startup recovery: replay from event store to rebuild session and account states.
- CLI: `session.*` and `events.*` command families.
- Snapshots: save/load snapshot API in the event store (periodic scheduling deferred to Phase 6).

Exit criteria:
- All open ADRs that affect Phase 1 are resolved. ✅ ADR-007 resolved (Rust + Ratatui).
- Daemon starts and accepts CLI commands via the Control API socket. ✅
- Sessions transition through all states in the state machine using the Mock
  adapter; invalid transitions are rejected and logged. ✅
- Daemon restarts and reconstructs all session states from the event log. ✅
- Unit and integration tests pass; coverage per `TESTING.md` targets met. ✅
  (36/36 tests pass; 22 unit, 14 integration.)

**Status:** ✅ Complete — 2026-09-19.

**Implementation notes:**
- `Idle → Stopped` direct transition added (see ADR-011 in `DECISIONS.md`).
- Periodic snapshot scheduling (write a snapshot every N events) is deferred
  to Phase 6; the snapshot save/load infrastructure is in place.
- `session.steer` and `events.query` commands are defined in the CLI but the
  daemon returns `NotFound` for `events.query` (full implementation in Phase 3).
- `WaitingForHuman → RateLimited`, `Starting → Stopping`, `Starting → Crashed`,
  and `Paused → Crashed` transitions exist in the state machine for completeness;
  they are not exercised by Phase 1 tests but are correctly enforced.

---

## Phase 2 — Account manager and project registry (MVP part 2)

**Goal:** First-class accounts and projects; account selection and rotation;
workspace assignment.

Deliverables:
- Account Manager: register accounts, state machine, concurrency tracking.
- Account selection strategy: least-loaded, tag-filtered.
- Cooldown and quota tracking; simulated exhaustion and cooldown flow.
- Project Registry: register projects, workspace policy (`Shared` / `WorktreePerSession`).
- Workspace creation and reclamation (worktrees).
- CLI: `account.*` and `project.*` command families.
- Credential store binding: accounts reference credentials, never store secrets
  in the event log (see [`SECURITY.md`](SECURITY.md)).

Exit criteria:
- Multiple accounts can be registered; the selection strategy picks the correct
  account under load. ✅
- Simulated quota exhaustion triggers cooldown and blocks new session assignment
  to the exhausted account. ✅
- Worktrees are created on session start and reclaimed on session stop/completion. ✅
- Secrets are absent from the event store and all log outputs. ✅
- Tests per `TESTING.md` targets met. ✅ (79/79 tests pass; 49 unit, 14 Phase 1 integration, 16 Phase 2 integration.)

**Status:** ✅ Complete — 2026-09-21.

**Implementation notes:**
- `AccountManager` and `AccountStore` implemented with SQLite persistence and concurrency tracking.
- Account state machine enforced (`Active`, `RateLimited`, `Cooldown`, `Exhausted`, `Invalid`, `Disabled`).
- Account selection strategy handles least-loaded, agent-type compatibility, and tag filtering.
- Account cooldown, rate limiting, and exhaustion tracking prevent dispatching to unavailable accounts.
- `ProjectRegistry` and `ProjectStore` implemented with SQLite persistence and workspace policies (`Shared` and `WorktreePerSession`).
- Worktree workspace path generation and lifecycle reclamation on session stop.
- `credential_ref` indirection ensures raw secrets never enter the event log or SQLite database (see ADR-006).
- Control API IPC handlers for `account.*` and `project.*` implemented with scoped locks to prevent holding guards across `.await`.
- CLI commands for `account` and `project` subcommands implemented in `ac-cli`.
- 16 integration tests in `crates/ac-core/tests/phase2_integration.rs` covering all Phase 2 deliverables.

---

## Phase 3 — Interaction hub and policy engine (MVP part 3)

**Goal:** Complete human ↔ agent interaction loop with declarative policy
auto-approval and full audit trail.

Deliverables:
- Interaction Hub: unified inbox, routing to Policy Engine, pending queue for humans.
- Policy Engine: condition evaluation, `Allow`/`Deny`/`Escalate` outcomes.
- Hard never-auto-approve boundary: classes of action that always escalate.
- Audit log: `AuditEntry` for every policy decision and human action.
- Policy CRUD via CLI.
- TUI inbox view: pending interactions with approve/deny/reply.
- Full transcript per session (human + agent messages, decisions).

Exit criteria:
- Every `ApprovalRequested` and `Question` event produces exactly one `Interaction`. ✅
- Policy rules auto-resolve matching interactions; non-matching interactions
  go to the human inbox. ✅
- Never-auto-approve classes always escalate regardless of rules. ✅
- Every decision (policy or human) produces an `AuditEntry`. ✅
- Audit entries are never deleted or redacted. ✅
- Tests per `TESTING.md` targets met. ✅ (124/124 tests pass; 85 unit, 14 Phase 1 integration, 16 Phase 2 integration, 9 Phase 3 integration).
- **MVP complete**: all three phases passing together against Mock adapter. ✅

**Status:** ✅ Complete — 2026-09-22.

**Implementation notes:**
- `InteractionHub` and `InteractionStore` implemented with SQLite persistence (`interactions` table), unified inbox, routing, pending query, human resolution (`reply`, `dismiss`), and session expiration.
- `PolicyEngine` and `PolicyStore` implemented with SQLite persistence (`policies` and `audit_log` tables).
- ADR-012 deterministic evaluation precedence: Hardcoded Never-Auto-Approve boundary > Deny rules > Allow rules > RequireHuman rules > Default Fallback (`RequireHuman`).
- Hardcoded Never-Auto-Approve boundary (`SECURITY.md` T3): destructive filesystem (`rm`), system commands (`sudo`), network exfiltration (`curl`, `wget`), git force (`push --force`), and package publishing (`publish`) always escalate to human.
- `SessionManager` integration:
  - Adapter events forwarded directly into manager's `tokio::select!` loop.
  - Auto-approved actions allow session to continue uninterrupted in `Working`.
  - Manual approvals and questions escalate session: `Working -> WaitingForHuman`.
  - Human resolution (`reply` or `dismiss`) via `InteractionHub` sends `AgentCommand::Respond` to adapter and resumes `WaitingForHuman -> Working` once all pending interactions are resolved.
  - Session stop automatically transitions pending interactions to `Expired`.
- Control API IPC and CLI commands added for `interaction.*`, `policy.*`, and `audit.*`.
- 9 comprehensive integration tests in `crates/ac-core/tests/phase3_integration.rs` covering all Phase 3 flows and Unix socket IPC roundtrips.

---

## Phase 4 — First real adapter (Claude Code)

**Goal:** Connect a real coding agent; validate the adapter contract end to end.

Deliverables:
- Claude Code adapter: structured mode (hooks/NDJSON) with PTY fallback.
- Output parser: maps agent output to `AgentEvent` types with confidence markers.
- Real approval round-trip: adapter receives `Allow`/`Deny` from Interaction Hub.
- Steering: human sends instructions to a live session.
- TUI terminal attach: passthrough view of the agent's PTY.
- Adapter confidence markers surfaced in the TUI.

Exit criteria:
- A real Claude Code session can be started, paused, resumed and stopped. ✅
- An approval request from Claude Code reaches the inbox and the decision is
  delivered back to the agent correctly. ✅
- A human steering instruction reaches the running agent. ✅
- PTY fallback produces correct events when the structured mode is unavailable. ✅
- Tests cover both structured and PTY modes. ✅

**Status:** ✅ Complete — 2026-09-22.

**Implementation notes:**
- `ClaudeAdapter` and `ClaudeAdapterFactory` implemented in `crates/ac-core/src/adapter/claude.rs`.
- Supports both `ClaudeMode::Structured` (NDJSON stream over stdio) and `ClaudeMode::Pty` (PTY fallback).
- Implemented `parse_claude_event` mapping stream JSON payloads (`ready`, `output`, `progress`, `tool_use`/`approval_requested`, `question`, `rate_limit`, `completed`, `error`) into `AdapterEvent` with confidence levels (`High` for structured, `Low` for raw text).
- Implemented `format_claude_command` for sending decisions (`Respond`), human steering (`Steer`), termination (`Stop`), and context requests (`Snapshot`).
- `GenericPtyAdapter` and `GenericPtyAdapterFactory` implemented in `crates/ac-core/src/adapter/pty.rs`.
  - Built with `portable-pty` for pseudo-terminal allocation and async I/O.
  - Configurable regex pattern matching for interactive CLI agents (`Ready`, `ApprovalRequested`, `QuestionRaised`, `RateLimitSignal`, `Completed`).
  - Native POSIX signal handling (`libc::kill`) for pause (`SIGSTOP`), resume (`SIGCONT`), graceful shutdown (`SIGTERM`), and kill (`SIGKILL`).
  - Background exit status monitor detecting clean process exits and crash signals.
- `CompositeAdapterFactory` introduced in `crates/ac-core/src/adapter/mod.rs` routing `agent_type` (`claude-code`, `generic-pty`, `mock`) to appropriate factories.
- Extended `SessionContext` with `workspace_path` and `credential_ref`, strictly binding workspace directory boundaries on spawn.
- Resolved ADR-008: Statically linked adapters within `ac-core` with trait boundary isolation.
- 7 comprehensive integration tests in `crates/ac-core/tests/phase4_integration.rs` covering structured/PTY lifecycles, workspace isolation, human approval/question loops, and factory dispatch. Total test suite: 152/152 tests passing.

---

## Phase 5 — TUI Dashboard + Inbox

**Goal:** Build the primary human-facing Ratatui TUI for Agent Control over the Control API.

Deliverables:
- Ratatui TUI crate (`crates/ac-tui`) with modular view architecture and clean client separation.
- Main Dashboard: High-level overview of active sessions, pending human interactions, system health, and recent activity.
- Sessions View & Session Detail:
  - Table of sessions with visually distinct state badges (`Starting`, `Idle`, `Working`, `WaitingForHuman`, `Paused`, `RateLimited`, `Completed`, `Failed`, `Stopped`).
  - Detailed drilldown displaying workspace path, bound account, live transcript buffer with color-coded tags, and recent session events.
  - Interactive session control actions: `Start` (s), `Pause` (p), `Resume` (Space), `Stop` (x), `Steer` (t).
- Interaction Inbox:
  - Table of pending interactions (questions and approval requests) displaying session, prompt, tool details, and time elapsed.
  - Action hotkeys: `Approve` (a), `Deny` (d), `Reply` (r), and `Dismiss` (x).
- Accounts & Projects Views:
  - Accounts: Live state badges, provider, active session load against concurrency caps, and cooldown countdowns.
  - Projects: Registered repositories, default agent types, workspace policies, and active workspace count.
- Activity Stream: Real-time event log with interactive filtering by event kind or session ID.
- Modal Dialog System: Centered popups for human steering input, interaction replies, stop confirmations, event filters, and keyboard shortcut help (`?`).
- Control API Client (`ApiClient`):
  - Async Unix domain socket client with graceful disconnect detection and automatic reconnection attempts.
  - Background event stream subscription (`events.subscribe`) updating in-memory UI state reactively.
  - Added `session.steer` and `events.query` support in Control Core IPC.
- CLI Integration: `ac tui` and `ac dashboard` subcommands in `crates/ac-cli`.

Exit criteria:
- Main dashboard and all 6 tabs render cleanly on standard terminal dimensions. ✅
- Visually distinguishable states with standardized color badges across all views. ✅
- Real-time updates delivered via `events.subscribe` stream without requiring manual polling. ✅
- Approvals, denials, replies, and steering routed successfully to running sessions. ✅
- Resilient error handling: socket disconnects and stale interactions display alerts without crashing the TUI. ✅
- Headless test coverage (`ratatui::backend::TestBackend`) and end-to-end API integration tests. ✅ (167/167 tests passing across the workspace; 15 Phase 5 tests).
- 0 compiler warnings (`cargo check --workspace --all-targets`). ✅

**Status:** ✅ Complete — 2026-09-22.

**Implementation notes:**
- Decoupled architecture: `ac-tui` connects to the daemon purely over the local Unix socket (`/tmp/agentcontrold.sock` or `XDG_RUNTIME_DIR`), with zero direct core memory access.
- Non-blocking event loop using `tokio::select!` interleaves terminal key events (`crossterm::event::EventStream`), live API events (`events.subscribe`), and periodic 2-second polling to ensure state freshness.
- Status message banner with automated 4-second expiration for non-intrusive operational feedback.
- Headless testing verified through 7 unit tests in `crates/ac-tui/tests/ui_state_tests.rs` and 8 live server integration tests in `crates/ac-tui/tests/api_integration_tests.rs`.

---

## Phase 6 — Seamless Multi-Account Agent Switching

**Goal:** Allow users to manage multiple authenticated coding-agent accounts and select or switch which account a coding-agent session uses without repeated manual logouts and logins. Abstract provider-specific auth mechanisms behind the account/adapter boundary.

Deliverables:
- **Clean Concept Separation:**
  - `Account`: Persistent provider identity with metadata, label, concurrency caps, tags, and indirect `credential_ref` (zero secrets).
  - `Credential`: External secret referenced indirectly via ADR-006 tokens (`ref:<id>`).
  - `AgentSession`: Running coding agent instance bound to an account identity, maintaining predecessor/successor lineage across switches.
  - `ProviderAdapter`: Boundary translating account references into child-process auth injection.
- **Provider Capability Model:**
  - `ProviderCapabilities` and `AccountSwitchMode`: `Dynamic`, `RequiresRestart`, `Unsupported`.
  - Implemented honest capability reporting on `ClaudeAdapter` (`RequiresRestart`), `GenericPtyAdapter` (`RequiresRestart`), and `MockAdapterFactory`.
- **Deterministic Account Selection:**
  - Explicit selection via `validate_and_select_explicit`: Validates existence, compatibility, usability, and concurrency cap. Returns structured errors (`AccountNotFound`, `IncompatibleAccount`, `AccountUnavailable`, `ConcurrencyLimitReached`).
  - Automatic selection: Filters compatible, active accounts, excludes exhausted/cooldown accounts, and breaks ties deterministically (`active_session_count` → `label` → `id`).
  - Availability querying via `query_availability`.
- **Account Switching Abstraction:**
  - *Path A (Idle session):* Atomic rebinding before start (`session.select_account` / `session.switch_account`).
  - *Path B (Running session):* Controlled Session Hand-off when provider requires restart.
- **Controlled Session Hand-off:**
  - Predecessor session snapshots safe context, terminates cleanly, and transitions to `SessionState::HandedOff`.
  - Decrements predecessor account load; increments successor account load.
  - Spawns successor session inheriting workspace path, project identity, and task metadata.
  - Establishes bi-directional lineage tracking (`predecessor_id` and `successor_id`).
- **Session Snapshot Model:**
  - `SessionSnapshot`: Versioned schema (`SNAPSHOT_SCHEMA_VERSION = 1`), capturing safe metadata, sequence progress, workspace location, and task summary.
  - Strict secret exclusion: Raw tokens, API keys, and credentials are never serialized.
- **TUI Multi-Account Switching:**
  - Accounts view displaying live load, availability, provider, and concurrency caps.
  - Session detail view displaying bound account and predecessor/successor lineage badges.
  - Two-step interactive switch modal (`Modal::SwitchAccount` with `[w]` hotkey): Account selection table → Restart confirmation prompt highlighting preserved vs lost context.
- **Control API / IPC & CLI:**
  - Commands: `session.select_account`, `session.switch_account`, `session.snapshot`, `session.handoff`, `account.query_availability`.
  - CLI: `ac session select-account`, `ac session switch-account`, `ac session snapshot`, `ac session handoff`, `ac account availability`, `--account-id` flag for `create` and `run`.
- **Audit & Events:**
  - Emits `AccountSelected`, `AccountSwitchRequested`, `AccountSwitchStarted`, `AccountSwitchCompleted`, `AccountSwitchFailed`, `SessionSnapshotCreated`, `SessionHandOffStarted`, `SessionHandOffCompleted`, `SessionHandOffFailed`.
- **Integration & UI Test Suite:**
  - 10 integration tests in `crates/ac-core/tests/phase6_integration.rs`.
  - UI modal and progression tests in `crates/ac-tui/tests/ui_state_tests.rs`.
  - Total test suite: 178/178 tests passing across workspace, 0 compiler warnings.

Exit criteria:
- Explicit and automatic account selection works deterministically. ✅
- Controlled hand-off creates linked successor session and updates load counters. ✅
- Claude Code and CLI adapters honestly report `RequiresRestart`. ✅
- Zero raw secrets in SQLite, events, audit log, TUI, errors, or session snapshots. ✅
- Ratatui TUI allows interactive account switching with confirmation. ✅
- 100% test pass rate (178/178 tests) with 0 compiler warnings. ✅

**Status:** ✅ Complete — 2026-09-22.

---

## Phase 7 — External API and integrations

**Goal:** Stabilise the Control API for external consumers; deliver AgentDesk
and AgentMesh integration points.

Deliverables:
- Control API schema v1: frozen, versioned, documented in `docs/API_CHANGELOG.md`.
- WebSocket loopback endpoint (`tokio-tungstenite`) built directly into the daemon (`ws://127.0.0.1:4242` / ADR-009).
- Token authorization registry (`TokenRegistry`) with granular permission scopes (`read`, `write`, `admin`).
- Advanced subscription filtering (`SubscriptionFilter`) by event kinds, `session_id`, `project_id`, and `account_id`.
- Historical catch-up replay via `since_seq` on both Unix socket IPC and WebSocket.
- Shared request dispatcher (`dispatch_request`) ensuring complete parity between Unix socket IPC and WebSocket transports.
- Integration guidance for AgentDesk (desktop GUI) and AgentMesh (orchestrator) published in `INTERFACES.md`, `DATA_FLOW.md`, and `API_CHANGELOG.md`.
- Comprehensive Phase 7 integration test suite in `crates/ac-core/tests/phase7_integration.rs`.

Exit criteria:
- A script/client can subscribe to the event stream over WebSocket or IPC and receive events without access to daemon internals. ✅
- The schema version (`v: 1`) is enforced on every request; a version mismatch returns structured error `VersionMismatch`. ✅
- Scope enforcement: mutating commands are rejected with `PermissionDenied` for `read` tokens. ✅
- AgentDesk and AgentMesh integration paths are documented in `INTERFACES.md`, `DATA_FLOW.md`, and `API_CHANGELOG.md`. ✅
- 100% test pass rate (185/185 tests) with 0 compiler warnings. ✅

**Status:** ✅ Complete — 2026-09-22.

---

## Phase 8 — Hardening and v1 release

**Goal:** Production-quality stability, security audit, documentation complete.

Deliverables:
- Security review against the threat model in [`SECURITY.md`](SECURITY.md) (`crates/ac-core/tests/security_audit_tests.rs`).
- Fuzz testing of the Control API and adapter output parsers (`crates/ac-core/tests/fuzz_robustness_tests.rs`).
- Full test suite passing across all workspace crates (195/195 tests, 0 compiler warnings).
- Performance benchmarks: batch event store write throughput (>72,000 events/sec), recovery time from a 100k-event log in ~808 ms (`crates/ac-core/tests/benchmarks.rs`).
- Migration & offline diagnostic tooling: `EventStore::verify_integrity`, `ac event verify`, and `ac event replay`.
- Final documentation review: all cross-references valid, no stale sections.
- CHANGELOG for v1.0.0 and workspace version bump to 1.0.0.

Exit criteria:
- All tests pass with no known critical or high-severity issues. ✅ (195/195 tests pass across unit, integration, property, security, and benchmark suites).
- Security review findings addressed or explicitly accepted with rationale. ✅ (Automated security audit suite verifies T1–T5).
- Recovery from a 100k-event log completes in under 5 seconds on the target hardware. ✅ (Reconstructed 10,000 sessions in ~808 ms).
- All documentation cross-references resolve. ✅

**Status:** ✅ Complete — 2026-09-22. Version 1.0.0 released.

---

## Phase UX — User-Friendly Agent Launcher & Account UX

**Goal:** Provide normal users with a seamless, zero-friction experience for multi-account agent workflows without exposing low-level identifiers or commands.

Deliverables:
- `agy "Personal Google"` launcher: Direct launch with friendly account label, availability checks, concurrency limit protection, and interactive TUI session controls.
- `agy` interactive account selector: Auto-launch if 1 account exists; interactive selector with real-time status and cooldowns if multiple accounts; browser login prompt if zero accounts.
- `agy account add`: Automated OAuth loopback browser flow with manual code fallback and secure `0600` credential file persistence (`ref:antigravity:<id>`).
- `agent-control` command: Unified interactive dashboard with `AGENTS`, `ACCOUNTS`, `SESSIONS`, and global action shortcuts (`[Enter] Open`, `[A] Add Account`, `[N] New Agent`, `[S] Sessions`, `[Q] Quit`).
- Controlled session account switching (`[a]` key) with explicit confirmation for `RequiresRestart` adapters.
- 100% preservation of low-level `ac` CLI power-user commands.
- Comprehensive integration test suite in `crates/ac-cli/tests/launcher_and_account_ux_tests.rs`.

Exit criteria:
- `agy "Account Label"` connects via IPC, starts/attaches session, and prevents silent fallbacks on failure. ✅
- `agy` auto-launches single account or provides interactive selector. ✅
- Browser OAuth authentication persists credential tokens without leaking secrets to SQLite or audit logs. ✅
- All workspace tests pass (204/204 tests) with zero compiler warnings. ✅

**Status:** ✅ Complete — 2026-09-23.

---

## Summary table

| Phase | Label | MVP? | Key deliverable |
|---|---|---|---|
| 0 | Design and documentation | — | Complete blueprint |
| 1 | Core daemon and CLI | ✓ | Event store, state machine, Mock adapter |
| 2 | Account manager and project registry | ✓ | Account pool, workspace assignment |
| 3 | Interaction hub and policy engine | ✓ | Inbox, policy, audit log |
| 4 | Real agent adapters | — | Claude Code & Generic PTY adapters |
| 5 | TUI Dashboard + Inbox | — | Ratatui TUI, live streams, modal dialogs |
| 6 | Seamless Multi-Account Agent Switching | — | Account selection/switching, hand-off, snapshots |
| 7 | External API and integrations | — | Stable API v1, WebSocket, Scopes, Replay |
| 8 | Hardening and v1 release | — | Security, perf, release |
| UX | User-Friendly Agent Launcher & Account UX | — | `agent-control`, `agy "Label"`, OAuth, TUI |

---

## Open items

- Cryptographic signing of audit log: see ADR-010 in [`DECISIONS.md`](DECISIONS.md) and [`FUTURE.md`](FUTURE.md).

