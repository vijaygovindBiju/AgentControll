# Agent Control — Security

This document describes the threat model for Agent Control v1, the security
controls that address each threat, and the hard boundaries that no policy or
configuration can override.

---

## Scope and assumptions

Agent Control is a **local, single-user daemon** running on a developer's
laptop or workstation. The threat model reflects this context:

- The primary user is the process owner; they are trusted.
- Other processes on the same machine are partially trusted (same user) or
  untrusted (other users, daemons, network services).
- The daemon is not exposed to the network in v1 (loopback only; see
  [`INTERFACES.md`](INTERFACES.md#transport)).
- Agent processes (adapters) are untrusted by design; their output is treated
  as untrusted data.
- Provider credentials are highly sensitive secrets that must never leave the
  credential store in plaintext.

---

## Threat model

### T1 — Credential exposure

**Description:** Provider API keys or OAuth tokens leak into the event log,
the audit log, the TUI, or any log file.

**Impact:** An attacker with read access to any of these outputs gains access
to the provider account.

**Controls:**

- Account entities store a `credential_ref` (a key name or path in the
  credential store), never the raw secret value.
- The credential store is OS-provided or a locked file; Agent Control reads
  credentials at session-start time only, into process memory, and does not
  forward them to the event log.
- A redaction component scans every event payload before it is persisted to
  the event store or published to the Control API. Redaction rules match
  known secret patterns (bearer tokens, API key formats) and replace them
  with `[REDACTED]`.
- The redaction component is applied uniformly: event store writes, event
  subscriptions (TUI, external consumers), and the CLI `events.query` output
  all pass through it.
- Session environment variables (carrying credentials) are never captured in
  `AgentOutputReceived` events.

### Phase 6: Multi-Account Switching & Session Snapshot Security

- **Credential Indirection (`credential_ref`):**
  - Account records store only opaque references (e.g. `"ref:claude-account-b"`).
  - Raw authentication tokens, passwords, and API keys are never stored in `Account`, `AgentSession`, `SessionSnapshot`, or SQLite tables.
  - TUI tables and detail views display only account labels and provider identities.
- **Provider Capability Boundary:**
  - Adapters report their runtime capabilities truthfully via `ProviderCapabilities`.
  - Process-based coding agents (Claude Code, CLI agents) declare `AccountSwitchMode::RequiresRestart`. Agent Control performs controlled session hand-off rather than attempting unsafe in-memory credential injection.
- **Session Snapshot Secret Exclusion:**
  - `SessionSnapshot` captures only safe task metadata, workspace directory path, and event sequence progress.
  - Raw credentials, environment variables, authorization headers, and arbitrary process heap/memory are strictly excluded from serialization.
  - Automated integration tests (`test_session_snapshot_serialization_and_zero_secret_leakage`) verify that serialized snapshot JSON contains no API tokens or secret substrings.
- **Auditable Lineage Without Secret Propagation:**
  - Predecessor and successor sessions are permanently distinct entities linked by ULIDs (`predecessor_id`, `successor_id`).
  - Switching events (`AccountSelected`, `AccountSwitchRequested`, `SessionHandOffStarted`, `SessionHandOffCompleted`) audit who requested the switch and which accounts were involved without ever including credential values.

### Phase 7: External API Surface & Token Scopes Security

- **Strict Loopback Binding Guarantee:**
  - The WebSocket Control API binds exclusively to `127.0.0.1` (loopback). It is never exposed on external IP interfaces (`0.0.0.0`), preventing remote network access.
- **Granular Token Scopes (`read`, `write`, `admin`):**
  - Read-only consumers (e.g. AgentDesk in observation mode) are strictly prevented from issuing mutating commands (`session.create`, `session.start`, `session.stop`, `account.remove`, `interaction.reply`, etc.).
  - Mutating commands attempted by `read`-scoped tokens are blocked immediately by `dispatch_request` with a structured `PermissionDenied` error before reaching managers or adapters.
- **Authentication Handshake Flexibility with Zero Credential Leakage:**
  - Supports HTTP `Authorization: Bearer`, URL query string `?token=...`, or in-band `auth` message.
  - Invalid tokens return HTTP 401 Unauthorized or structured `Unauthorized` error without leaking timing or internal system information.
- **Zero Raw Secrets in WebSocket Frames:**
  - Broadcast event streams (`events.subscribe`) and query results contain only sanitized metadata, event sequences, and interaction records. No child process credentials or auth secrets are serialized.

---

### T2 — Unauthorised command issuance

**Description:** A process on the same machine (malware, compromised tool)
sends commands to the Control API socket to start, stop, or steer sessions,
or to read the event history.

**Impact:** Unauthorised control of agent sessions; exfiltration of task
context and interaction history.

**Controls:**

- The Unix domain socket is created with mode `0600`, owned by the daemon's
  user. Only processes running as the same user can connect.
- The loopback WebSocket transport, if enabled, requires a bearer token. The
  token is generated at daemon start, stored in a file with mode `0600`, and
  rotated on restart.
- Token scopes limit external consumers to the minimum required access (e.g.
  AgentDesk gets a read-only token; AgentMesh gets a write-scoped token for
  specific command families only).

**Residual risk:** A process running as the same user (e.g. a compromised
extension) can connect to the Unix socket. This is inherent to local-only
architecture; multi-user isolation is a future extension (see
[`FUTURE.md`](FUTURE.md)).

---

### T3 — Agent-driven privilege escalation via approval bypass

**Description:** A rogue or compromised agent constructs approval requests that
a policy auto-approves for destructive or exfiltrating actions (e.g. `rm -rf`,
`curl <external>`, credential access).

**Impact:** Data destruction, credential theft, or exfiltration, all without
human awareness.

**Controls (human approval boundary):**

The Policy Engine enforces a hard, never-overridable boundary for the following
action classes. These always return `Escalate` regardless of any policy rule:

| Action class | Examples |
|---|---|
| Filesystem destructive operations | `rm -rf`, `git clean -f`, bulk deletes outside workspace |
| Credential access | Reading files containing known secret patterns, env dumps |
| Network exfiltration | `curl`/`wget`/`fetch` to non-allowlisted external hosts |
| Git force operations | `git push --force`, `git reset --hard` on remote branches |
| Package publish | `npm publish`, `cargo publish`, `pip upload` |
| System-level commands | `sudo`, `chmod`, `chown`, package manager installs |
| Code execution in CI/CD | Triggering production pipelines |

The list of never-auto-approve classes is defined in configuration but the
configuration is not user-accessible at runtime (it requires a daemon restart
to change, ensuring no policy rule can dynamically widen the boundary).

Auto-approve (`Allow` outcome) is only permitted for:
- Read-only filesystem operations within the session's workspace.
- Git operations that do not modify remote state (status, diff, log, checkout).
- Operations explicitly allowlisted in the project's policy.

**Controls (structural):**

- Tool names and arguments in `ApprovalRequested` events are validated against
  a schema before Policy Engine evaluation; malformed requests are rejected.
- The Policy Engine evaluates conditions against normalized, structured data,
  not raw agent text, to resist prompt injection in policy evaluation.

**Residual risk:** The allowlist may be over-wide for some projects. Operators
should audit project policies before enabling auto-approval.

---

### T4 — Prompt injection via agent output

**Description:** An agent's output (from a web page, file, or external API)
contains instructions that, when displayed in the TUI or forwarded to another
system, cause unintended behaviour.

**Impact:** Misleading TUI display; injection into steering messages forwarded
to other sessions (if AgentMesh integration exists).

**Controls:**

- `AgentOutputReceived` events are stored verbatim in the event store but are
  rendered in the TUI as plain text (not interpreted as markup or commands).
- Steering messages from external consumers (AgentMesh) are treated as
  untrusted data; the Interaction Hub does not evaluate them for policy
  conditions, only forwards them to the adapter.
- The TUI must not execute any content received from agent output as terminal
  escape sequences capable of altering the terminal state in dangerous ways
  (OSC, title-set, clipboard-write sequences should be stripped or escaped).

**Residual risk:** Full prompt-injection prevention depends on adapter
implementation quality and TUI rendering choices; flag for review in Phase 4.

---

### T5 — Event log tampering

**Description:** An attacker (or a bug) modifies or deletes rows in the SQLite
event store, corrupting history or concealing actions.

**Impact:** Loss of audit trail; incorrect state reconstruction on recovery.

**Controls:**

- The event store is append-only by design. The daemon opens SQLite in WAL
  mode and never issues `UPDATE` or `DELETE` on the events or audit tables.
- The SQLite file is in the user's data directory with mode `0600`.
- Sequence numbers are gapless; a gap detected during recovery or replay is
  reported as a `CorruptionDetected` event and the daemon refuses to start
  until the operator acknowledges.
- Periodic checksums on the event log are recorded in a separate table to
  detect offline tampering (design detail TBD; record as open decision).

**Residual risk:** SQLite-level tampering by the process owner cannot be
prevented in v1. Cryptographic event signing is a future extension (see
[`FUTURE.md`](FUTURE.md)).

---

### T6 — Workspace boundary violation

**Description:** An agent operates outside its assigned workspace (reading or
writing files in other projects, the home directory, or system paths).

**Impact:** Cross-project data leakage; unintended modifications to unrelated
files.

**Controls:**

- The adapter pre-validates that `SessionContext.workspace_path` exists on disk and is a valid directory prior to spawning the child process; missing or invalid paths cause immediate startup failure (`AdapterError::ConfigError`).
- The adapter strictly pins child process `cwd` to `workspace.path` (via `Command::current_dir` or `portable_pty::CommandBuilder::cwd`).
- Approval policies include `touched_paths` conditions; accesses to paths
  outside the workspace prefix trigger policy evaluation and, by default,
  `Escalate`.
- The never-auto-approve list includes destructive operations outside the
  workspace path.

**Residual risk:** Agent Control cannot enforce filesystem isolation without
OS-level sandboxing (namespaces, seccomp). Filesystem-level isolation is a
future extension (see [`FUTURE.md`](FUTURE.md)).

---

## Human approval boundary (summary)

The human approval boundary is the set of action classes that must always reach
a human and can never be auto-approved by any policy. It is the principal
safety guarantee of Agent Control.

```
┌──────────────────────────────────────────────────────────────────┐
│   Agent action request arrives at Interaction Hub                │
│                                                                  │
│   Policy Engine evaluates …                                      │
│                                                                  │
│   ┌────────────────────────────────────────────────────────┐     │
│   │   NEVER-AUTO-APPROVE BOUNDARY (always Escalate)        │     │
│   │                                                        │     │
│   │  • Destructive filesystem ops                          │     │
│   │  • Credential access                                   │     │
│   │  • Network exfiltration                                │     │
│   │  • Force git pushes / history rewrites                 │     │
│   │  • Package publish                                     │     │
│   │  • System commands (sudo, chmod, …)                    │     │
│   │  • CI/CD production triggers                           │     │
│   └────────────────────────────────────────────────────────┘     │
│                                                                  │
│   Actions not in the above list may be Allow or Deny by policy.  │
│   Actions not matched by any policy are Escalate by default.     │
└──────────────────────────────────────────────────────────────────┘
```

The boundary is not configurable at runtime. Changing it requires editing the
daemon configuration file and restarting.

---

## Credential store integration

Agent Control does not implement its own credential store in v1. It integrates
with:

- **Linux**: `libsecret` / GNOME Keyring or `kwallet` (KDE), with a plaintext
  locked file fallback in `$XDG_DATA_HOME/agentcontrol/credentials` (mode
  0600).
- The credential store integration is an open decision (see ADR-006 in
  [`DECISIONS.md`](DECISIONS.md)).

Credentials are:
- Looked up by `credential_ref` at session-start time.
- Held in process memory only for the lifetime of the adapter spawn call.
- Never serialized to disk by Agent Control itself.
- Never transmitted over the Control API.

---

## Security review checkpoint

A security review against this threat model is a Phase 8 exit criterion (see
[`ROADMAP.md`](ROADMAP.md#phase-8--hardening-and-v1-release)). The review
must specifically verify:

1. No credential value appears in the event store (automated scan).
2. The never-auto-approve boundary is enforced by code path, not by policy
   configuration.
3. The Unix socket is created with the correct mode on all supported platforms.
4. The redaction component covers all known credential formats for v1 adapters.
5. No terminal escape injection is possible through agent output in the TUI.
