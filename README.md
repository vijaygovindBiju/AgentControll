# Agent Control

A local-first control plane for coding agents.

> **Status:** Version 1.0.0 released. Fully verified with zero compiler warnings and 100% test pass rate across all workspace test suites.

---

## Table of Contents

1. [What is Agent Control?](#what-is-agent-control)
2. [What Problem It Solves](#what-problem-it-solves)
3. [Main Features](#main-features)
4. [Supported Agents](#supported-agents)
5. [Architecture Overview](#architecture-overview)
6. [Installation](#installation)
7. [Quick Start](#quick-start)
8. [Configuration](#configuration)
9. [Accounts & Authentication](#accounts--authentication)
10. [Working Directories & Projects](#working-directories--projects)
11. [Execution Modes](#execution-modes)
12. [Permission & Access Controls](#permission--access-controls)
13. [TUI Controls](#tui-controls)
14. [Security](#security)
15. [Troubleshooting](#troubleshooting)
16. [Development](#development)
17. [Testing](#testing)
18. [Roadmap](#roadmap)
19. [Contributing](#contributing)
20. [License](#license)

---

## What is Agent Control?

**Agent Control** is a local supervisor daemon, interactive terminal interface, and automation CLI that manages the complete lifecycle of coding agents—such as **Google Antigravity (`agy`)**, **Claude Code**, **Codex**, and **Devin CLI**.

It coordinates multiple agent processes across different repositories and accounts, supervises interactive PTY streams, isolates credentials per profile, automates permissions, and maintains an append-only event log persisted in SQLite.

---

## What Problem It Solves

Developers running coding agents across various projects, teams, and accounts encounter significant friction:
- **Terminal Fragmentation:** Running multiple agents in disconnected terminal tabs leads to lost context, untracked tool invocations, and unmonitored processes.
- **Credential Collisions:** Managing multiple provider accounts (e.g., personal vs. work Google/Gemini accounts) in a shared home directory causes token overwrites and accidental cross-billing.
- **Quota Exhaustion:** When an agent encounters provider rate limits or token exhaustion, work halts without a mechanism to capture a context snapshot and rotate to an alternate account.
- **Audit & Safety Blind Spots:** Autonomous agents executing shell commands require consistent guardrails and a tamper-evident audit trail of tool decisions and approvals.

Agent Control solves this by operating as a unified local supervisor: it pools accounts, isolates execution workspaces, provides interactive human-in-the-loop steering, and enforces declarative security policies.

---

## Main Features

- **One-Command Launching:** Run directly via `npx agent-control` or the standalone shell installer without manually installing Rust or Cargo.
- **Full-Screen Virtual ANSI Terminal:**
  - Dedicated virtual terminal buffer (`crates/ac-tui/src/terminal_buffer.rs`) with 256-color, truecolor, scrollback history, and alternate screen (`1049h`/`1049l`) emulation.
  - Transparent key forwarding: `Esc` passes directly to agent processes (for nano, vi, fzf, and CLI prompts), while `Ctrl+]` or `Ctrl+Q` safely detaches back to the supervisor dashboard.
  - Multi-byte UTF-8 stream decoding (`decode_utf8_stream`) prevents glyph splitting and character corruption across PTY chunks.
- **Multi-Account Pooling & Controlled Hand-Off:**
  - Maintain multiple provider accounts with concurrency caps, cooldown tracking, and quota health status.
  - Seamlessly hand off tasks: capture context snapshots and switch accounts without re-entering prompts.
- **Decoupled Execution & Permission Modes:**
  - Independently configure execution modes (`Default`, `Accept Edits`, `Plan`) and safety levels (`Normal`, `Dangerously Skip Permissions` with required confirmation modal).
- **Interactive Directory Completion:**
  - Shell-style `Tab` completion for arbitrary filesystem paths with live filtering in the session launcher.
- **Profile & Credential Isolation:**
  - Every account runs in a dedicated profile directory (`~/.config/agentcontrol/profiles/<account-id>/`), setting `HOME` to the profile and stripping ambient credentials from the environment.
- **Dual Transport Control API:**
  - POSIX Unix domain socket IPC (`0600` permissions) for local processes.
  - Loopback WebSocket (`127.0.0.1:4242`) with token-based scopes (`read`, `write`, `admin`) for external tooling.
- **Policy Engine & Audit Trail:**
  - Declarative policy rules with a hardcoded never-auto-approve boundary (`sudo`, `rm`, `curl`, `git push --force`).
  - Append-only SQLite event store recording every state change, approval, and command.

---

## Supported Agents

| Agent | Adapter | Protocol / Transport | Capabilities |
|:---|:---|:---|:---|
| **Google Antigravity (`agy`)** | `PtyAdapter` | Native PTY with Profile Isolation | Multi-account OAuth PKCE, login-link handoff, execution modes (`accept-edits`, `plan`), model flags, permission bypass. |
| **Claude Code** | `ClaudeAdapter` | Structured NDJSON / PTY fallback | `--output-format stream-json`, tool approvals, rate-limit back-off, terminal interaction. |
| **Generic CLI Agent** | `PtyAdapter` | Pseudo-Terminal (`portable-pty`) | Regex pattern matching for prompts; POSIX signal controls (`SIGSTOP`, `SIGCONT`, `SIGTERM`, `SIGKILL`). |
| **Codex / Devin CLI** | `PtyAdapter` | Pseudo-Terminal | Interactive session supervision, ANSI terminal streaming, and workspace binding. |
| **Mock Agent** | `MockAdapter` | In-Memory Event Simulation | Deterministic testing of state machines, crashes, and policy escalations without external APIs. |

For detailed adapter architecture, see [docs/AGENTS.md](docs/AGENTS.md) and [docs/ADAPTERS.md](docs/ADAPTERS.md).

---

## Architecture Overview

```text
┌──────────────────────────────── Human Interfaces ────────────────────────────────┐
│      Ratatui TUI (`agent-control` / `ac-tui`)     │      CLI (`ac`, `agy`)       │
└────────────────────────────────────────┬─────────────────────────────────────────┘
                                         │ Control API (Unix Socket 0600 / Loopback WS)
┌────────────────────────────────────────▼─────────────────────────────────────────┐
│                              Control Core (Daemon)                               │
│                                                                                  │
│  ┌──────────────────┐    ┌───────────────────────────────────────────────────┐   │
│  │ Session Manager  │───▶│            Agent Session State Machine            │   │
│  └────────┬─────────┘    └───────────────────────────────────────────────────┘   │
│           │                                                                      │
│  ┌────────┴─────────┐    ┌──────────────────┐    ┌───────────────────────────┐   │
│  │ Account Manager  │    │ Project Registry │    │ Interaction Hub ◀──▶      │   │
│  └──────────────────┘    └──────────────────┘    │ Policy Engine             │   │
│                                                  └───────────────────────────┘   │
│  ┌────────────────────────────────────────────────────────────────────────────┐  │
│  │                  Event Bus  ───▶  Event Store (SQLite)                     │  │
│  └────────────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────┬─────────────────────────────────────────┘
                                         │ Adapter Contract (In-Process Channels)
┌────────────────────────────────────────▼─────────────────────────────────────────┐
│ Adapters: Google Antigravity (agy) │ Claude Code │ Generic PTY │ Mock             │
└────────────────────────────────────────┬─────────────────────────────────────────┘
                                         │ Child Processes (PTY / stdio pipes)
┌────────────────────────────────────────▼─────────────────────────────────────────┐
│                            Supervised Coding Agents                              │
│      (cwd = Workspace Directory, HOME = Account Profile, Clean Environment)      │
└──────────────────────────────────────────────────────────────────────────────────┘
```

For in-depth specifications, see:
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): Component responsibilities, threading, and lifecycle.
- [docs/DATA_FLOW.md](docs/DATA_FLOW.md): Step-by-step sequence diagrams for core workflows.
- [docs/INTERFACES.md](docs/INTERFACES.md): Wire protocols, command schemas, and adapter contracts.

---

## Installation

### Method 1: NPX (No Rust or compilation required)

Run immediately using `npx`:

```bash
npx agent-control
```

Or install globally:

```bash
npm install -g agent-control
agent-control
```

### Method 2: One-Line Shell Installer

Install precompiled binaries directly to `~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/agentcontrol/agentcontrol/main/install.sh | sh
```

### Method 3: Prebuilt Tarballs

Download precompiled tarballs and checksums from [GitHub Releases](https://github.com/agentcontrol/agentcontrol/releases):

```bash
curl -LO https://github.com/agentcontrol/agentcontrol/releases/download/v1.0.0/agent-control-v1.0.0-x86_64-unknown-linux-gnu.tar.gz
tar -xzf agent-control-v1.0.0-x86_64-unknown-linux-gnu.tar.gz -C ~/.local/bin/
```

### Method 4: Building from Source

```bash
git clone https://github.com/agentcontrol/agentcontrol.git
cd agentcontrol
cargo build --release --workspace
cp target/release/{agent-control,agentcontrold,agy,ac} ~/.local/bin/
```

For complete platform details and options, see [docs/INSTALL.md](docs/INSTALL.md).

---

## Quick Start

### 1. Launch the Supervisor Dashboard

```bash
agent-control
```

The daemon starts automatically in the background if not already active.

### 2. Connect an Antigravity Account

Press **`a`** in the TUI, or run:

```bash
agy account add --name "Personal Google"
```

- **Browser Login:** Opens your browser to complete Google OAuth sign-in.
- **Handoff Login:** Generates a login URL to complete on any machine; paste the redirect URL back into Agent Control.
- **Import Local Token:** Automatically imports existing tokens from `~/.gemini/antigravity-cli/`.

### 3. Launch an Agent Session

Press **`n`** from the TUI to open the launcher modal:
1. Select target agent (`antigravity`, `claude-code`).
2. Type working directory with **`Tab`** completion.
3. Select account, execution mode, and permission boundary.
4. Press **Enter** on `[Launch Session]`.

For complete walkthrough, see [docs/QUICKSTART.md](docs/QUICKSTART.md).

---

## Configuration

Agent Control works out of the box with zero required configuration. To customize defaults, create `~/.config/agentcontrol/config.toml`:

```toml
# Path to SQLite database
db_path = "~/.local/share/agentcontrol/events.db"

# Path to Unix domain socket
socket_path = "/run/user/1000/agentcontrol.sock"

# Maximum restart attempts before marking a crashed session as Failed
max_restarts = 3

# Daemon logging level
log_level = "info"

# WebSocket server (strictly loopback)
ws_enabled = true
ws_bind_addr = "127.0.0.1:4242"
```

See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for full configuration reference and environment variables.

---

## Accounts & Authentication

Agent Control treats provider accounts as first-class, rotatable resources. All authentication logic is centralized in `crates/ac-core/src/agy_auth.rs`.

### Account Management Commands

```bash
agy account list                      # View accounts, cooldowns, and quotas
agy account remove "Work Google"      # Delete account and purge isolated profile
```

### Isolation Model

- **Zero Secret Exposure:** Credentials are stored in `0600` files at `~/.config/agentcontrol/credentials/agy_<id>.json`. The database and logs record only opaque references (`ref:antigravity:<id>`).
- **Profile Redirection:** Antigravity runs with `HOME=~/.config/agentcontrol/profiles/<account-id>/`. Symlinks are created only for non-sensitive configurations (`.gitconfig`, `.ssh/`).
- **Controlled Hand-Off:** When an account hits quota exhaustion, Agent Control captures a safe task snapshot, gracefully terminates the old process, and starts a successor session under an alternate account.

See [docs/ANTIGRAVITY_ACCOUNTS.md](docs/ANTIGRAVITY_ACCOUNTS.md) for full account documentation.

---

## Working Directories & Projects

Sessions can execute in standalone folders or registered projects:

### Tab Directory Completion
In the launch modal (`n`), typing in the **Working Directory** input and pressing **`Tab`** automatically expands paths and cycles through matching directories.

### Project Registry (`ac project`)
```bash
# Register repository with shared directory execution
ac project register --name api-service --repo /path/to/repo --workspace-policy shared

# Register repository with isolated Git worktree per session
ac project register --name core-lib --repo /path/to/repo --workspace-policy worktree-per-session --base-branch main
```

---

## Execution Modes

Configured per-session or globally in Settings:
- **`Default`:** Standard interactive agent loop with user prompts.
- **`Accept Edits` (`--mode=accept-edits`):** Automatically accepts file edits while prompting before running shell commands.
- **`Plan` (`--mode=plan`):** Read-only codebase exploration and plan generation without file mutations.

---

## Permission & Access Controls

- **`Normal` (Default):** Standard safety boundaries; requests confirmation for tool calls and terminal executions.
- **`Dangerously Skip Permissions` (`--dangerously-skip-permissions`):** Auto-approves tool and command executions. In the TUI, selecting this mode requires explicit confirmation through a red-bordered confirmation dialog.

---

## TUI Controls

| Key | Tab View | Action |
|:---|:---|:---|
| **`1`–`6`** | Top Bar | Jump to tab: Dashboard (`1`), Sessions (`2`), Accounts (`3`), Activity (`4`), Agents (`5`), Settings (`6`). |
| **`Tab` / `Shift+Tab`** | Top Bar | Cycle forward / backward through tabs. |
| **`Enter`** | Sessions | Attach to the full-screen virtual terminal of the selected session. |
| **`Esc`** | Attached PTY | Passed directly through to the agent child process. |
| **`Ctrl+]`** / **`Ctrl+Q`** | Attached PTY | Detach from the session terminal back to the dashboard. |
| **`n`** | Sessions / Agents | Open **Start Session Modal** (directory completion, account, mode). |
| **`s`** | Sessions | Open **Steer Modal** to inject instructions into running agent. |
| **`p`** | Sessions | **Pause** running agent process (`SIGSTOP`). |
| **`r`** / **`c`** | Sessions | **Resume** paused agent process (`SIGCONT`). |
| **`x`** / **`Delete`** | Sessions | **Stop** agent process gracefully. |
| **`w`** | Sessions | Open **Switch Account Modal** (controlled hand-off). |
| **`?`** | Global | Toggle Help & Keybindings modal. |
| **`q`** | Global | Exit TUI application (sessions continue running in background). |

For complete keybindings, see [docs/TUI.md](docs/TUI.md).

---

## Security

1. **Local-Only Communication:** Unix socket is created with strict `0600` permissions. The WebSocket binds exclusively to `127.0.0.1` and requires token-based authentication.
2. **Zero Credential Leakage:** Tokens are stored in `0600` files; events and database entries record only opaque references (`ref:<provider>:<id>`). Payloads pass through a redaction filter before storage or publishing.
3. **Never-Auto-Approve Boundary:** High-risk actions (`sudo`, `rm`, `curl`, `git push --force`) cannot be auto-approved by policy rules; they unconditionally escalate to human review (`RequireHuman`).
4. **Clean Process Environment:** Ambient Google and cloud environment variables are stripped before launching agent processes.

For security policies and vulnerability reporting, see [SECURITY.md](SECURITY.md) and [docs/SECURITY.md](docs/SECURITY.md).

---

## Troubleshooting

| Issue | Cause | Resolution |
|:---|:---|:---|
| `Daemon: Disconnected` | Background daemon is not running. | Run `agent-control` (starts it automatically) or `agentcontrold &`. |
| `Permission denied` on socket | Stale socket owned by different user. | Remove stale socket: `rm -f ~/.config/agentcontrol/agentcontrold.sock`. |
| `No Antigravity credential saved` | Credential file deleted or expired. | Run `agy account add --name "<name>"`. |
| `Windows unsupported` | Windows requires POSIX sockets and PTY. | Run inside WSL2 (`wsl --install`), then run `npx agent-control`. |
| `Session hanging` | Legacy PTY back-pressure deadlock. | Fixed in v1.0.0 via 8KB burst buffers, non-blocking `try_send`, and 512-item queues. |

See [docs/INSTALL.md#troubleshooting](docs/INSTALL.md#troubleshooting) and [docs/ANTIGRAVITY_ACCOUNTS.md](docs/ANTIGRAVITY_ACCOUNTS.md) for more details.

---

## Development

The workspace is organized into three Rust crates:
- [crates/ac-core](crates/ac-core): Core supervisor daemon and `agentcontrold` binary (event store, session manager, state machine, accounts, projects, policy engine, adapters, WebSocket/IPC servers).
- [crates/ac-cli](crates/ac-cli): Command-line interfaces and launchers (`agent-control`, `agy`, `ac`).
- [crates/ac-tui](crates/ac-tui): Terminal UI library and `ac-tui` binary (Ratatui views, ANSI virtual screen buffer, event loop).

### Building

```bash
# Check workspace with 0 compiler warnings
cargo check --workspace --all-targets

# Run release build
cargo build --release --workspace
```

For contribution guidelines, see [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Testing

Agent Control maintains an extensive test suite covering unit logic, integration flows, and end-to-end launcher scenarios:

```bash
# Run all workspace unit and integration tests
cargo test --workspace

# Run Antigravity hardening tests (fake binary E2E)
cargo test -p ac-cli --test agy_hardening_tests

# Run TUI virtual terminal buffer tests
cargo test -p ac-tui terminal_buffer
```

For test principles and categories, see [docs/TESTING.md](docs/TESTING.md).

---

## Roadmap

- [x] **v1.0.0 Core Platform:** Complete supervisor daemon, SQLite event store, PTY terminal emulation, and multi-account hand-off.
- [ ] **v1.1.0 Presets & Claude Sandbox:** Reusable session presets, Claude Code multi-account profile isolation, and OpenTelemetry metrics export.
- [ ] **v1.2.0 Linux Sandboxing:** Kernel-enforced filesystem sandboxing via Linux Landlock / bubblewrap, and webhook notification triggers.
- [ ] **v2.0.0 Distributed Clusters:** Remote agent execution over encrypted mTLS tunnels and localhost web companion.

See [docs/ROADMAP.md](docs/ROADMAP.md) for the complete roadmap.

---

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) for information on our zero-warning policy, test requirements, and pull request workflow.

---

## License

Agent Control is open source software licensed under the [MIT License](LICENSE).
