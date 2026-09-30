# Changelog

All notable changes to Agent Control are documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-09-22

Initial production-ready release of Agent Control — the multi-account supervision and orchestration daemon for coding agents.

### Highlights

- **Seamless Multi-Account Agent Switching**: Run multiple accounts across providers (e.g., Anthropic Claude Code, OpenAI, Generic PTY CLI tools) and switch identities without manual logout/login cycles.
- **Controlled Process Hand-Off**: Versioned session snapshots allow transitioning running agents between accounts with preserved project workspaces and execution metadata.
- **Autonomous Policy Engine & Approval Inbox**: Declarative rule evaluation with a hardcoded never-auto-approve boundary for destructive commands (`rm`, `sudo`, `curl | sh`, `force push`).
- **Unified Transports**: Full parity between local Unix Domain Socket IPC and authenticated WebSocket loopback (`ws://127.0.0.1:4242`).
- **Granular Token Scopes**: Strict capability isolation (`read`, `write`, `admin`) preventing read-only telemetry clients from mutating daemon state.
- **High-Performance Event Sourcing**: Append-only SQLite WAL store achieving >72,000 events/sec batch ingestion and sub-second recovery of 10,000 sessions from 100,000 events.
- **Terminal User Interface (TUI)**: Full-screen interactive dashboard built with Ratatui and Crossterm, featuring live event streaming, modal approval/question answering, and interactive account switching.

### Added

#### Phase 1 — Core Daemon & State Machine
- `AgentSession` state machine enforcing 10 lifecycle states (`Idle`, `Starting`, `Working`, `WaitingForHuman`, `Paused`, `RateLimited`, `Stopping`, `Stopped`, `Failed`, `Crashed`).
- SQLite append-only event store with sequential gap detection and monotonic IDs.
- Deterministic Mock adapter for offline lifecycle testing and crash back-off validation.
- CLI subcommands for session lifecycle management (`session.create`, `session.start`, `session.pause`, `session.resume`, `session.stop`).

#### Phase 2 — Account Manager & Project Registry
- Account registry supporting state transitions (`Active`, `Exhausted`, `Disabled`), tag filtering, and concurrency limits.
- Deterministic least-loaded account selection strategy with tie-breaking on account ID.
- Project and workspace registry supporting `Shared` and `WorktreePerSession` git workspace policies.

#### Phase 3 — Interaction Hub & Policy Engine
- Human-in-the-loop interaction hub managing approval requests, user questions, and notifications.
- Declarative policy rules with priority ordering and condition operators (`Equals`, `Contains`, `StartsWith`, `EndsWith`, `Regex`).
- Absolute `NeverAutoApprove` boundary enforcing mandatory human escalation for destructive filesystem and network operations.
- SQLite-backed audit log capturing all automated and human decisions.

#### Phase 4 — Real Agent Adapters
- Claude Code structured adapter consuming child-process NDJSON streaming events (`control_request`, `progress`, `text`).
- Generic PTY adapter using pseudo-terminal allocation (`portable-pty`) and regex pattern matching to interact with arbitrary CLI agents.
- Composite adapter factory routing sessions to appropriate backends by agent type.

#### Phase 5 — Terminal User Interface (TUI)
- Multi-tab Ratatui dashboard (`Sessions`, `Inbox`, `Accounts`, `Projects`, `Events`).
- Real-time event streaming and UI updating over local Unix domain sockets.
- Modal dialogs for human approval, freeform question replies, and interactive account switching.
- Comprehensive headless rendering test suite using `ratatui::backend::TestBackend`.

#### Phase 6 — Seamless Multi-Account Agent Switching
- Provider-neutral account switching abstraction distinguishing idle re-binding from active session hand-offs.
- Controlled session hand-off lifecycle (`AccountSwitchStarted`, `SessionSnapshotCreated`, `SessionHandOffCompleted`) preserving workspace and project context.
- Provider capabilities model declaring restart requirements and context preservation guarantees.
- Strict credential indirection (`env:<VAR>` or `ref:<ID>`), guaranteeing raw credentials never enter SQLite, events, audit records, errors, snapshots, or TUI displays.

#### Phase 7 — External API v1 & WebSocket Loopback
- Frozen Control API wire schema v1 with version mismatch rejection.
- Embedded authenticated WebSocket loopback server with `Authorization: Bearer <TOKEN>` or `x-agent-control-token` header parsing.
- Granular token authorization scopes (`read`, `write`, `admin`).
- Advanced subscription filtering by event kinds, `session_id`, `project_id`, and `account_id` with historical catch-up replay (`since_seq`).

#### Phase 8 — Hardening, Security Audit & Migration Tooling
- Automated security audit suite verifying threat models T1–T5 in `docs/SECURITY.md`:
  - T1: Zero credential leakage in snapshots, database columns, event queries, and API responses.
  - T2: Socket permissions (`0600`) and read-only token mutation prevention.
  - T3: Tamper-proof never-auto-approve boundary.
  - T4: Workspace directory traversal and escape containment.
  - T5: Malicious shell and SQL injection immunity in steer messages and task descriptions.
- Robustness and fuzz test suite verifying wire parsers against deeply nested JSON (500 levels), 2MB oversized payloads, malformed NDJSON, ANSI escapes, and ReDoS attacks.
- High-throughput batch ingestion (`EventStore::append_batch`) yielding >72,000 events/second.
- Performance benchmark demonstrating recovery of 10,000 sessions from 100,000 events in ~808 milliseconds (beating the < 5.0-second threshold).
- Diagnostic integrity and replay CLI subcommands (`ac event verify` and `ac event replay`).

#### User Experience Layer & Multi-Account UX
- Two-layer CLI architecture: `agent-control` and `agy` for normal developers, `ac` for power-user scripting and automation.
- `agy "Account Label"` direct launcher with automatic daemon spawning, friendly account label resolution, and interactive session attachment.
- Automatic interactive account selector for `agy` with cooldown tracking and concurrency limit displays.
- Automated OAuth 2.0 PKCE loopback authentication on `127.0.0.1:0` (`agy account add`) with manual code and pasted-redirect callback fallbacks.
- Secure `0600` credential file persistence (`~/.config/agentcontrol/credentials/agy_<id>.json`) referenced solely by `ref:antigravity:<id>`.

#### Post-v1 Hardening & Interactive Terminal Ergonomics
- Dedicated Agent Terminal view with full ANSI virtual screen buffer ([`terminal_buffer.rs`](file:///media/pirate/Shared/currently%20working/AgentControll/crates/ac-tui/src/terminal_buffer.rs)) supporting alternate screen buffers (`1049h`/`1049l`), cursor positioning, scroll regions, and truecolor ANSI rendering.
- Multi-byte UTF-8 stream decoder (`decode_utf8_stream`) ensuring UTF-8 glyphs split across chunk boundaries are never corrupted into `U+FFFD`.
- Anti-deadlock PTY architecture: non-blocking `try_send` dispatch, 8KB burst read buffers, and 512-item per-session event queues eliminating reader thread deadlocks during large generation bursts.
- Transparent key forwarding: `Esc` is forwarded directly to the agent PTY (for vi/nano modes, dialog dismissals, and agy prompt interactions), while `Ctrl+Q` cleanly detaches back to the supervisor dashboard.
- Streamlined 6-tab navigation (`1: Dashboard`, `2: Sessions`, `3: Accounts`, `4: Activity`, `5: Agents`, `6: Settings`).
- Centralized Settings hub with 9 keyboard-navigable sections (General, Accounts, Projects & Workspaces, Agents, Models, Permissions, Authentication, Terminal, Security), persistent to `~/.config/agentcontrol/settings.json`.
- Decoupled Antigravity execution modes (`Default`, `Accept Edits`, `Plan`) from permission boundaries (`Normal`, `Dangerously Skip Permissions` with a strict confirmation modal).
- Interactive session launch modal (`n`) with shell-style `Tab` directory path completion and interactive filtering.
- Isolated account profiles (`~/.config/agentcontrol/profiles/<account-id>/`) with `HOME` redirection and ambient Google credential stripping.
