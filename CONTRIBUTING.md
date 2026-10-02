# Contributing to AgentControll

Thank you for your interest in contributing to AgentControll! We welcome bug reports, documentation improvements, and pull requests.

This guide provides everything you need to set up your local development environment, build the project, run tests, and submit contributions.

---

## Development Prerequisites

Before contributing, ensure you have the following installed:

1. **Operating System:** Linux (x86_64, aarch64) or macOS (Darwin). Windows users should use WSL2.
2. **Rust Toolchain:** Stable Rust 1.80+ with `cargo`, `rustc`, `rustfmt`, and `clippy`.
   ```bash
   rustup update stable
   rustup component add rustfmt clippy
   ```
3. **C Toolchain & Libraries:** Standard C compiler (`gcc` or `clang`), `make`, and standard headers (`libc`).
4. **Node.js (Optional):** Node.js 18+ and `npm` (required only if testing or updating the npm installer package under `packages/agent-control`).

---

## Repository Setup & Workspace Layout

Clone the repository:

```bash
git clone https://github.com/vijaygovindBiju/AgentControll.git
cd AgentControll
```

The workspace is organized into three Rust crates:
- **`crates/ac-core`**: Core supervisor daemon library and `agentcontrold` binary (event store, state machine, account/project registries, interaction hub, policy engine, adapters, WebSocket/IPC servers).
- **`crates/ac-cli`**: Command-line interfaces and launchers (`agentcontroll`, `ac`, `agentcontrold`).
- **`crates/ac-tui`**: Terminal user interface library and `ac-tui` binary (Ratatui views, ANSI virtual screen buffer, event loop).
- **`packages/agent-control`**: Cross-platform npm package wrapper for `npx agentcontroll`.


---

## Build Commands

Compile the workspace in debug mode:

```bash
cargo build --workspace
```

Compile release binaries:

```bash
cargo build --release
```

> [!TIP]
> If building on a machine with limited disk space, redirect the build target directory to avoid out-of-space errors:
> ```bash
> CARGO_TARGET_DIR=/tmp/ac-target CARGO_INCREMENTAL=0 cargo build --workspace
> ```

---

## Verification & Quality Checks

All contributions must pass automated verification checks before merging.

### 1. Code Formatting

Check formatting:
```bash
cargo fmt --check
```

Format code automatically:
```bash
cargo fmt
```

### 2. Workspace Compilation

Verify that all crates compile without warnings:
```bash
cargo check --workspace --all-targets
```
> [!IMPORTANT]
> AgentControll enforces a **zero warnings policy**. `cargo check --workspace --all-targets` must produce `0` compiler warnings.

### 3. Running Tests

Run the full workspace test suite (over 250 unit and integration tests):
```bash
cargo test --workspace
```

Run specific test suites:
```bash
# Core integration tests
cargo test -p ac-core --test integration

# Antigravity E2E hardening tests (runs with fake agy binary fixture)
cargo test -p ac-cli --test agy_hardening_tests

# TUI virtual terminal buffer tests
cargo test -p ac-tui terminal_buffer
```

### 4. Linting

Run Clippy to inspect code quality:
```bash
cargo clippy --workspace --all-targets
```

---

## Contribution Workflow

1. **Check Existing Issues:** Before starting work on major changes, search existing GitHub Issues and Pull Requests to avoid duplicate efforts.
2. **Create a Feature Branch:**
   ```bash
   git checkout -b feat/my-improvement
   ```
3. **Commit Guidelines:**
   - Write clear, concise commit messages following standard conventional commit guidelines:
     - `feat:` for new capabilities.
     - `fix:` for bug fixes.
     - `docs:` for documentation updates.
     - `test:` for test additions or improvements.
     - `refactor:` for code restructuring without behavioral changes.
4. **Preserve Behavioral Rules:**
   - Do NOT modify adapter behavior, state machine transitions, or OAuth contracts unless explicitly coordinating a versioned API update.
   - Do NOT commit machine-specific paths (e.g. `/home/<user>`, `/media/...`).
   - Never commit API keys, tokens, or credential files.
5. **Push and Open a Pull Request:**
   - Push your branch to your GitHub fork:
     ```bash
     git push origin feat/my-improvement
     ```
   - Open a Pull Request targeting the `main` branch.
   - Fill out the PR template describing what changed and why, linking any relevant issue numbers.

---

## Pull Request Expectations

Every PR is reviewed against the following criteria:
- **CI Status:** GitHub Actions CI must pass completely (`cargo fmt --check`, `cargo check --workspace --all-targets`, `cargo test --workspace`).
- **Zero Regressions:** Existing tests must not be deleted or weakened.
- **Documentation:** If command syntax, configuration options, or UI controls change, update corresponding documents in `docs/` and `README.md`.
- **Security:** Code that touches process spawning, credentials, or IPC sockets must adhere to the security boundaries defined in `SECURITY.md`.

---

## Issue Reporting Guidance

When reporting an issue:
1. Check that you are running the latest version of AgentControll (`agentcontroll --version`).
2. Provide your operating system distribution (`uname -a`, `cat /etc/os-release`).
3. Describe the expected versus actual behavior.
4. Provide minimal reproduction steps.
5. Review the log output and ensure **no secrets or credentials** are included before submitting.
