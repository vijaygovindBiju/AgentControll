# AGENTS.md

## Verification

```bash
cargo check --workspace --all-targets   # must produce 0 warnings
cargo test --workspace
```

- The project lives on a nearly full partition. If builds fail with "No space left on device", build elsewhere:
  `CARGO_TARGET_DIR=/tmp/ac-target CARGO_INCREMENTAL=0 cargo test --workspace`
- Dependencies can be added offline from the local registry cache: `cargo add <crate>@<ver> -p <crate> --offline`.

## Antigravity (agy) accounts

- All agy auth, credential storage, validation, profile and removal logic lives in `crates/ac-core/src/agy_auth.rs`. The CLI (`crates/ac-cli/src/auth.rs`, `launcher.rs`) and the TUI (`crates/ac-tui/src/event.rs`) are only front ends; do not duplicate the logic there.
- `agy` must never start without a selected account and a validated credential. The adapter (`adapter/mod.rs`) prepares `~/.config/agentcontrol/profiles/<account-id>/` and sets `HOME` to it.
- End-to-end tests with a fake `agy` binary: `crates/ac-cli/tests/agy_hardening_tests.rs`. Tests that change env vars use a global lock.
- User guide: `docs/ANTIGRAVITY_ACCOUNTS.md`.
- Per-session launch options (permission mode, model, working directory) live in `crates/ac-core/src/agy_launch.rs`; they travel as `session.create` → `launch`, are stored on `AgentSession.launch` and reach the adapter via `SessionContext.agent_config`. Only modes the installed agy advertises are offered; `Normal` (no flag) is the default. The TUI form is `crates/ac-tui/src/launch.rs` + `views/start_session.rs`.
- The login-link flow uses `agy_auth::finish_login_with_handoff` (loopback callback or pasted redirect address; same state + PKCE login).

## TUI terminal emulation

- `crates/ac-tui/src/terminal_buffer.rs` emulates the PTY screen. Full-screen programs use the alternate screen (fixed rows×cols grid); the grid is kept at the PTY size (`App::resize_session_terminals`). Regression fixture of real agy output: `crates/ac-tui/tests/fixtures/agy_startup_100x30.raw`.
- PTY output is decoded with `decode_utf8_stream` (pty.rs) so multi-byte glyphs split across reads are never turned into U+FFFD.
