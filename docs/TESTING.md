# Agent Control — Testing

This document describes the testing and validation strategy for Agent Control.
It covers test categories, what each category verifies, and the coverage targets
per phase.

For the entities under test, see [`DATA_MODEL.md`](DATA_MODEL.md).
For the flows under test, see [`DATA_FLOW.md`](DATA_FLOW.md).
For the security-specific verification requirements, see
[`SECURITY.md`](SECURITY.md#security-review-checkpoint).

---

## Principles

- **Test behaviour, not internals.** Tests drive the system through the Control
  API or the Adapter Contract, not by calling internal functions directly.
- **Deterministic by design.** The Mock adapter and the injectable clock
  abstraction exist specifically to make every test fully deterministic without
  real timers or real agents.
- **Fast feedback first.** Unit tests run in milliseconds; integration tests in
  seconds; end-to-end (E2E) tests are the outer ring and run less frequently.
- **Recovery is a first-class test target.** Daemon restart recovery is tested
  as rigorously as normal operation.
- **Security controls are tested explicitly.** Each control in
  [`SECURITY.md`](SECURITY.md) has at least one dedicated test.

---

## Test categories

### Unit tests

**What they verify:** Individual modules in isolation, with all dependencies
replaced by test doubles.

Target modules and examples:

| Module | Example test |
|---|---|
| State Machine | All valid transitions accepted; all invalid transitions rejected with `TransitionRejected` event |
| Policy Engine | Condition evaluation for each condition type; never-auto-approve boundary always returns Escalate |
| Event Store | Append + sequential read; sequence-number gaplessness enforced; snapshot bound to correct seq |
| Redaction component | Known secret patterns replaced; unrelated text untouched |
| Account Manager | Selection strategy picks least-loaded account; exhausted account is skipped |
| Cooldown timer | Cooldown expiry fires at the correct injected time |
| Interaction Hub | Routing to policy then to inbox; correct state transitions |

Coverage target: all state machine transitions; all policy condition types;
all event types produced by each module.

### Integration tests

**What they verify:** Multiple modules working together, driven through the
Control API using the Mock adapter and an injected clock.

Key integration scenarios:

1. **Session lifecycle (happy path)**
   Start → Working → Pause → Resume → Stop, using Mock adapter scripted events.
   Verify: correct state at each step; correct events in the event store.

2. **Approval round-trip (auto-resolved)**
   Mock adapter emits `ApprovalRequested` matching an Allow policy.
   Verify: `Working → WaitingForHuman → Working`; `InteractionAutoResolved` event;
   `AuditEntry` produced.

3. **Approval round-trip (human-resolved)**
   Mock adapter emits `ApprovalRequested` with no matching policy.
   Human sends `interaction.resolve`.
   Verify: inbox shows pending interaction; `WaitingForHuman → Working` after resolve;
   `AuditEntry` produced.

4. **Never-auto-approve boundary**
   Mock adapter requests a tool in the never-auto-approve class.
   Verify: outcome is always `Escalate` regardless of any Allow policy.

5. **Account quota exhaustion**
   Mock adapter emits `RateLimitSignal { quota_reset_at }`.
   Verify: account transitions to `Exhausted`; sessions transition to `RateLimited`;
   cooldown timer fires at injected time; account returns to `Active`.

6. **Account hand-off**
   Exhausted account triggers hand-off policy.
   Verify: `ContextSnapshot` created; predecessor reaches `HandedOff`; successor
   session created with the same task and snapshot; successor reaches `Working`.

7. **Daemon recovery (event log replay)**
   Start sessions, then simulate daemon restart.
   Verify: all session states, account states, and pending interactions are
   reconstructed correctly from the event log.

8. **Crash and restart (within limit)**
   Mock adapter emits `Crashed` with restart limit of 3.
   Verify: sessions restart with back-off; `restart_count` increments; reaches
   `Working` on successful restart within limit.

9. **Crash beyond restart limit**
   Mock adapter emits `Crashed` four times.
   Verify: session reaches `Failed` after the fourth crash; no further restart
   attempted.

10. **Event subscription filtering**
    External subscriber registers with a kind filter.
    Verify: only events matching the filter are delivered; unrelated events
    are not delivered.

### End-to-end (E2E) tests

**What they verify:** The full system including a real adapter process, the
daemon, and the TUI or CLI, running on the developer's machine.

E2E tests are introduced in Phase 4. They are slower and require network/
credential access, so they run in a separate suite (not in the default fast
suite).

Key E2E scenarios (Phase 4+):

1. Start a real Claude Code session; verify it reaches `Working`.
2. Send an approval request from a real agent; approve it in the CLI; verify
   the agent continues.
3. Send a steering instruction via CLI; verify the agent acknowledges it in
   the TUI.
4. Simulate rate-limiting by using a test account with minimal quota; verify
   the hand-off flow completes.
5. Kill the daemon while a session is running; restart it; verify recovery.

### Property-based tests

**What they verify:** Invariants that must hold across arbitrary sequences of
events or commands.

Key properties:

- **State machine invariant:** For any sequence of valid inputs, the session
  state machine always ends in a valid state and never produces a gapped
  sequence of `StateChanged` events.
- **Event store invariant:** Sequence numbers are always gapless after any
  sequence of appends.
- **Redaction invariant:** Any payload containing a simulated credential value
  produces a redacted output with no trace of the original value.
- **Policy evaluation determinism:** The same policy set and input always
  produce the same outcome.
- **Cooldown invariant:** A session in `RateLimited` state never transitions to
  `Working` before the account cooldown expires (in injected time).

Property-based tests use an arbitrary input generator seeded from the Mock
adapter's event catalogue.

### Security tests

**What they verify:** Each control in [`SECURITY.md`](SECURITY.md) has a
corresponding test.

| Control | Test |
|---|---|
| T1 — Credential exposure | Scan event store for simulated credential value after session start and stop; assert absent |
| T1 — Redaction | Send a payload containing a known credential pattern; assert output is redacted |
| T3 — Never-auto-approve boundary | Request each never-auto-approve action class with a maximally permissive policy; assert `Escalate` |
| T3 — Policy allowlist | Only operations in the project allowlist produce `Allow`; all others `Escalate` or `Deny` |
| T5 — Append-only store | Attempt `UPDATE`/`DELETE` on events table via the ORM; assert rejected at the layer |
| T5 — Sequence gap detection | Insert a gap manually into the event log; assert daemon refuses to start and reports `CorruptionDetected` |

### Adapter compliance tests

**What they verify:** Any adapter implementation satisfies the Adapter Contract
(see [`INTERFACES.md`](INTERFACES.md#adapter-contract)).

The compliance test suite is run against every adapter, including Mock and
Generic PTY.

Tests:
1. Adapter emits `Ready` within timeout after `start()`.
2. Adapter emits `Crashed` when the underlying process exits unexpectedly.
3. Adapter delivers `AgentCommand(Respond)` to the agent correctly.
4. Adapter emits `RateLimitSignal` when the provider returns a rate-limit
   response pattern.
5. Adapter emits `SnapshotProduced` in response to `AgentCommand(Snapshot)`.
6. `stop()` terminates the process and emits no further events.
7. PTY fallback produces the correct `AdapterEvent` type for each defined
   output pattern with confidence = `Low`.

---

## Test infrastructure

### Mock adapter

A scriptable adapter that emits events from a pre-defined list at configurable
delays. Used for all unit and integration tests. Configuration:

```
MockAdapter::new(script: Vec<(Duration, AdapterEvent)>) -> MockAdapter
```

The script may include pauses (to test timeouts) and error conditions (to test
crash recovery and policy escalation paths).

### Injectable clock

The daemon uses a `Clock` abstraction instead of `std::time::Instant` or
`SystemTime` directly. Tests provide a `FakeClock` that can be advanced manually,
making all timer-dependent behaviour (cooldowns, back-offs, escalation
timeouts) fully deterministic.

### Deterministic ULID generator

Tests provide a deterministic ULID generator to produce stable, predictable
entity IDs, making test assertions readable and reproducible.

---

## Coverage targets per phase

| Phase | Target | Status |
|---|---|---|
| Phase 1 (core + CLI) | All state machine transitions; event store append + replay; startup recovery | ✅ 36/36 tests passing (22 unit, 14 integration) |
| Phase 2 (accounts + projects) | All account state transitions; workspace creation/reclamation; account selection | ✅ 79/79 tests passing (49 unit, 30 integration) |
| Phase 3 (interaction + policy) | All integration scenarios; never-auto-approve boundary; audit log completeness | ✅ 124/124 tests passing (85 unit, 39 integration across 3 suites) |
| Phase 4 (Claude Code adapter) | Adapter compliance suite; E2E happy path; PTY & structured modes; workspace isolation | ✅ 152/152 tests passing (106 unit, 46 integration across 4 suites) |
| Phase 5 (TUI Dashboard + Inbox) | Ratatui TUI headless render tests; keyboard event routing; live socket integration | ✅ 167/167 tests passing (106 unit, 46 core integration, 7 TUI state, 8 API integration) |
| Phase 6 (Seamless Multi-Account Switching) | Explicit/auto account selection; controlled hand-off; snapshots; secret exclusion; TUI switch modal | ✅ 178/178 tests passing (106 unit, 56 core integration across 5 suites, 8 TUI state, 8 API integration) |
| Phase 7 (External API and integrations) | Control API v1 schema freeze; WebSocket loopback; token auth & scopes; filtering & catch-up replay | ✅ 185/185 tests passing (106 unit, 63 core integration across 6 suites, 8 TUI state, 8 API integration) |
| Phase 8 | Hardening; fuzz testing; performance benchmarks; v1.0.0 release | Upcoming |

---

## What is not tested here

- **Human visual aesthetics** — layout styling is validated manually and programmatically via `ratatui::backend::TestBackend` buffer assertions.
- **AgentDesk and AgentMesh integration** — their integration is tested from
  their own test suites against the Control API event stream.
- **Provider behaviour** — rate-limiting, quota resets, and auth failures from
  real providers are simulated via the Mock adapter and the adapter compliance
  suite; we do not test providers directly.
