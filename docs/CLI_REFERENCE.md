# Agent Control CLI Reference Manual

This document provides a comprehensive reference for all command-line functions, binaries, subcommands, arguments, and workflow recipes in **Agent Control**.

---

## Table of Contents

1. [Binary Architecture Overview](#1-binary-architecture-overview)
2. [Global Flags & Environment Variables](#2-global-flags--environment-variables)
3. [Session Management (`ac session`)](#3-session-management-ac-session)
4. [Account Management (`ac account` & `ac login`)](#4-account-management-ac-account--ac-login)
5. [Project & Workspace Management (`ac project`)](#5-project--workspace-management-ac-project)
6. [Human Interaction & Approvals (`ac interaction`)](#6-human-interaction--approvals-ac-interaction)
7. [Policy Engine (`ac policy`)](#7-policy-engine-ac-policy)
8. [Audit Log Inspection (`ac audit`)](#8-audit-log-inspection-ac-audit)
9. [Event Log, Streaming & Integrity (`ac events`)](#9-event-log-streaming--integrity-ac-events)
10. [Daemon Status & Health (`ac status`)](#10-daemon-status--health-ac-status)
11. [Antigravity Launcher CLI (`agy`)](#11-antigravity-launcher-cli-agy)
12. [Interactive Terminal Dashboards (`agent-control` & `ac-tui`)](#12-interactive-terminal-dashboards-agent-control--ac-tui)
13. [JSON Scripting & Automation Examples](#13-json-scripting--automation-examples)
14. [End-to-End Practical Recipes](#14-end-to-end-practical-recipes)

---

## 1. Binary Architecture Overview

Agent Control provides a suite of complementary binaries:

| Binary | Role | Typical Use Case |
|---|---|---|
| `ac` | **Core Control Plane CLI** | Automation, scripting, fine-grained lifecycle, account configuration, policy and audit management. |
| `agy` | **Antigravity Launcher Shim** | Frictionless launching of Google Antigravity agents by account label (`agy "test1"`), browser OAuth, and account switching. |
| `agent-control` | **Unified Interactive Launcher** | One-command interactive interface launching the full supervisor dashboard. |
| `ac-tui` | **Ratatui Terminal Dashboard** | Full-screen interactive dashboard with live transcripts, approval inbox, account switching, and session control. |
| `agentcontrold` | **Supervisor Daemon** | Long-running background daemon managing sessions, accounts, policies, SQLite event store, and IPC/WebSocket servers. |

---

## 2. Global Flags & Environment Variables

These flags apply to `ac` and all its subcommands:

```bash
ac [OPTIONS] <COMMAND>
```

### Options:
* `--socket <PATH>`  
  Path to the daemon Unix domain socket.  
  *Default:* `/run/user/<UID>/agentcontrol.sock` (or `/tmp/agentcontrol-<UID>.sock`).
* `--json`  
  Output raw machine-readable JSON responses instead of formatted tables. Ideal for `jq` scripts, CI/CD, and pipelines.
* `-h, --help`  
  Print help information for any command or subcommand.
* `-V, --version`  
  Print version information.

### Environment Variables:
* `AC_SOCKET`: Overrides the default daemon socket path.
* `RUST_LOG`: Controls logging verbosity (`info`, `debug`, `trace`, `warn`, `error`).

---

## 3. Session Management (`ac session`)

The `ac session` command family manages the full lifecycle of coding agent sessions across all states: `Idle`, `Starting`, `Working`, `Paused`, `WaitingHuman`, `Stopped`, `Failed`, and `Crashed`.

```bash
ac session <COMMAND>
```

### 3.1. `ac session run`
Create and immediately start an agent session in a single operation.

```bash
ac session run [OPTIONS] --task <TASK>
```

* **Options:**
  * `-t, --task <TASK>` *(Required)*: The natural language instruction or prompt for the agent.
  * `-a, --agent-type <AGENT_TYPE>`: Agent adapter to use (`agy`, `claude`, `pty`). *Default: `agy`*.
  * `--account-id <ACCOUNT>`: Friendly account label (e.g. `test1`, `"Personal Google"`) or ULID.
  * `--project-id <PROJECT_ID>`: Optional project ID to bind workspace and policies.
* **Examples:**
  ```bash
  # Launch an Antigravity agent using account 'test1'
  ac session run --agent-type agy --account-id test1 --task "Refactor auth middleware to handle JWT expiration"

  # Launch a generic PTY runner
  ac session run --agent-type pty --task "cargo test --workspace"
  ```

### 3.2. `ac session create`
Create a session in `Idle` state without immediately starting it. Allows inspecting or binding accounts/workspaces before launch.

```bash
ac session create [OPTIONS] --task <TASK>
```

* **Options:** Same options as `run`.
* **Output:** JSON containing the allocated `session_id`.
* **Example:**
  ```bash
  ac session create --agent-type agy --account-id test1 --task "Run database migration benchmark"
  # Output: { "session_id": "01M3606GMP78KT564TS8R21EEZ" }
  ```

### 3.3. `ac session start`
Transition a session from `Idle` → `Starting` → `Working`.

```bash
ac session start <SESSION_ID>
```

* **Example:**
  ```bash
  ac session start 01M3606GMP78KT564TS8R21EEZ
  ```

### 3.4. `ac session pause`
Temporarily freeze execution of a running (`Working`) agent process.

```bash
ac session pause <SESSION_ID>
```

### 3.5. `ac session resume`
Resume execution of a `Paused` agent session back to `Working`.

```bash
ac session resume <SESSION_ID>
```

### 3.6. `ac session stop`
Terminate an active session and transition it to the terminal `Stopped` state. Replaces active sessions and decrements account load.

```bash
ac session stop [OPTIONS] <SESSION_ID>
```

* **Options:**
  * `-r, --reason <REASON>`: Optional explanatory reason for stopping the session.
* **Example:**
  ```bash
  ac session stop 01M3606GMP78KT564TS8R21EEZ --reason "User requested cancellation"
  ```

### 3.7. `ac session list`
Display a table of all recorded sessions, their current states, agent types, and task summaries.

```bash
ac session list
```

* **JSON Format:**
  ```bash
  ac --json session list | jq '.[] | {id: .id, state: .state, account: .account_id}'
  ```

### 3.8. `ac session get`
Retrieve full metadata for a specific session including timestamps, attached accounts, workspaces, and restart counts.

```bash
ac session get <SESSION_ID>
```

### 3.9. `ac session steer`
Inject a human steer instruction or prompt refinement into a running agent session without stopping or restarting it.

```bash
ac session steer <SESSION_ID> --message <MESSAGE>
```

* **Options:**
  * `-m, --message <MESSAGE>` *(Required)*: Instruction to send directly to the agent.
* **Example:**
  ```bash
  ac session steer 01M360EXKRTG2GTZB70RNBBD93 -m "Skip integration tests and run only unit tests for faster feedback"
  ```

### 3.10. `ac session select-account`
Explicitly bind an account to an `Idle` session prior to starting. Supports friendly labels (`test1`) or ULIDs.

```bash
ac session select-account <SESSION_ID> <ACCOUNT_ID_OR_LABEL>
```

### 3.11. `ac session switch-account`
Switch the account used by a session. If the session is `Idle`, it performs an instant rebind. If `Working`, it coordinates a dynamic switch or hand-off.

```bash
ac session switch-account <SESSION_ID> <TARGET_ACCOUNT_ID_OR_LABEL>
```

### 3.12. `ac session snapshot`
Capture an immutable, safe context snapshot of a session (file tree, diffs, git status, zero-secret token scrub).

```bash
ac session snapshot <SESSION_ID>
```

### 3.13. `ac session handoff`
Execute a controlled hand-off to another provider account, persisting workspace state and spawning a successor session.

```bash
ac session handoff [OPTIONS] <SESSION_ID>
```

* **Options:**
  * `-a, --account-id <ACCOUNT>`: Target account. If omitted, the daemon automatically selects the least-loaded available account.

---

## 4. Account Management (`ac account` & `ac login`)

Agent Control tracks provider accounts (Google Antigravity, Claude Code, Generic PTY), concurrency capacities, cooldown periods, and rate limits.

```bash
ac account <COMMAND>
```

### 4.1. `ac account list`
List all configured accounts, active sessions, concurrency capacities, states (`active`, `cooldown`, `rate_limited`, `exhausted`, `disabled`), and friendly labels.

```bash
ac account list
```

### 4.2. `ac login` (and `ac account login` / `ac account add`)
Log in and register a new account (e.g., Google OAuth for Antigravity).

```bash
ac login [OPTIONS]
```

* **Options:**
  * `-n, --name <NAME>`: Friendly label for the account (e.g. `test1`, `"Work Account"`).
  * `--cli`: Terminal-only authentication mode without opening a web browser.
* **Modes:**
  * **Browser OAuth Mode (default):** Spawns Google OAuth sign-in in your browser and listens on loopback (`http://localhost:54321/oauth2callback`). Automatically completes token exchange and saves credentials securely (`0600` permissions under `~/.config/agentcontrol/credentials/`).
  * **CLI Mode (`--cli`):** Displays the Google sign-in URL. Open the URL on any device, copy the authorization code, and paste it into the terminal.

### 4.3. `ac account register`
Register custom or generic provider accounts with manual credentials.

```bash
ac account register [OPTIONS] --label <LABEL> --provider <PROVIDER>
```

* **Options:**
  * `-l, --label <LABEL>`: Friendly name.
  * `-p, --provider <PROVIDER>`: Provider identifier (`agy`, `claude`, `pty`).
  * `-a, --agent-types <TYPES>`: Comma-separated supported agents (default: same as provider).
  * `-c, --credential-ref <REF>`: Path or reference to stored credential file.
  * `--cap <N>`: Maximum concurrent sessions allowed for this account (*default: 1*).
  * `-t, --tags <TAGS>`: Comma-separated tags (e.g. `gpu,high-quota,fast`).

### 4.4. `ac account availability`
Query whether valid, non-cooldown accounts are available for an agent type.

```bash
ac account availability --agent-type <AGENT_TYPE> [--tags <TAGS>]
```

### 4.5. `ac account disable` and `ac account enable`
Manually take an account out of rotation or re-enable it.

```bash
# Disable an account (e.g. during billing cycle changes)
ac account disable <ACCOUNT_ID_OR_LABEL>

# Re-enable an account
ac account enable <ACCOUNT_ID_OR_LABEL>
```

### 4.6. `ac account remove`
Delete an account from the database. Allowed only when the account has 0 active sessions.

```bash
ac account remove <ACCOUNT_ID_OR_LABEL>
```

---

## 5. Project & Workspace Management (`ac project`)

Projects define code repositories, workspace isolation policies, and account routing rules.

```bash
ac project <COMMAND>
```

### 5.1. `ac project register`
Register a project repository with Agent Control.

```bash
ac project register [OPTIONS] --name <NAME> --path <PATH>
```

* **Options:**
  * `-n, --name <NAME>`: Friendly project name.
  * `-p, --path <PATH>`: Absolute filesystem path to the project root.
  * `--policy <POLICY>`: Workspace isolation policy:
    * `shared`: Agents operate directly in the repository directory.
    * `worktree`: Agents receive dedicated git worktrees isolated per session.
  * `-t, --default-tags <TAGS>`: Comma-separated account tags required for this project.

### 5.2. `ac project list`
List all registered projects, repository paths, and workspace policies.

```bash
ac project list
```

### 5.3. `ac project workspaces`
List all active workspaces allocated for a specific project.

```bash
ac project workspaces <PROJECT_ID>
```

### 5.4. `ac project remove`
Deregister a project. Allowed only if no active sessions are bound to it.

```bash
ac project remove <PROJECT_ID>
```

---

## 6. Human Interaction & Approvals (`ac interaction`)

Coding agents frequently encounter safety boundaries (bash executions, file modifications, git pushes) or need clarification. The Interaction Hub intercepts these and pauses the agent until human resolution.

```bash
ac interaction <COMMAND>
```

### 6.1. `ac interaction list-pending`
List all interactions currently waiting for human input.

```bash
ac interaction list-pending
```

### 6.2. `ac interaction approve`
Approve an intercepted tool call or permission request immediately.

```bash
ac interaction approve [OPTIONS] <INTERACTION_ID>
```

* **Options:**
  * `-r, --response <TEXT>`: Optional feedback string passed back to the agent.
  * `-a, --actor <NAME>`: Operator identifier (*default: `human`*).
* **Example:**
  ```bash
  ac interaction approve 01M360H89... --response "Proceed with running migrations"
  ```

### 6.3. `ac interaction deny`
Deny an intercepted tool execution. The agent is resumed with an explicit denial notice and can choose an alternative approach.

```bash
ac interaction deny [OPTIONS] <INTERACTION_ID>
```

* **Options:**
  * `-r, --response <REASON>`: Reason for denial.

### 6.4. `ac interaction reply`
Respond to a question posed by an agent or deliver a policy decision with custom input.

```bash
ac interaction reply [OPTIONS] --response <TEXT> <INTERACTION_ID>
```

* **Options:**
  * `-r, --response <TEXT>` *(Required)*: Answer to the question.
  * `-d, --decision <DECISION>`: `allow`, `deny`, or `custom`.

### 6.5. `ac interaction dismiss`
Dismiss a stale interaction without recording a decision.

```bash
ac interaction dismiss <INTERACTION_ID>
```

---

## 7. Policy Engine (`ac policy`)

Configure automated rules that allow, deny, or escalate agent tool invocations without requiring manual human clicks every time.

```bash
ac policy <COMMAND>
```

### 7.1. `ac policy list`
List all installed policy rules in priority order.

```bash
ac policy list
```

### 7.2. `ac policy upsert`
Create or update an automated policy rule.

```bash
ac policy upsert [OPTIONS] --name <NAME> --pattern <PATTERN> --decision <DECISION>
```

* **Options:**
  * `-n, --name <NAME>`: Rule name.
  * `-p, --pattern <REGEX>`: Regex matching tool calls or commands (e.g. `^git status.*`).
  * `-d, --decision <DECISION>`: `allow`, `deny`, or `require_approval`.
  * `-t, --tool-name <TOOL>`: Specific tool name (e.g. `bash`, `file_edit`).
  * `-a, --agent-type <AGENT>`: Target agent type (`agy`, `claude`, etc.).
  * `--priority <N>`: Evaluation priority (higher numbers evaluated first).
* **Example:**
  ```bash
  # Automatically allow read-only git status commands
  ac policy upsert --name "Allow Git Status" --pattern "^git (status|diff|log)" --decision allow --priority 100
  ```

### 7.3. `ac policy test`
Simulate policy evaluation on a sample command before committing rules.

```bash
ac policy test --action "git push origin main --force" --tool-name "bash"
```

---

## 8. Audit Log Inspection (`ac audit`)

Agent Control maintains an append-only audit trail recording every state change, human decision, account switch, and tool invocation.

```bash
ac audit <COMMAND>
```

### 8.1. `ac audit list`
Inspect recent audit entries.

```bash
ac audit list [OPTIONS]
```

* **Options:**
  * `-s, --session-id <SESSION_ID>`: Filter events for a specific session.
  * `-l, --limit <N>`: Maximum records to return (*default: 50*).
* **Example:**
  ```bash
  ac audit list --session-id 01M360EXKRTG2GTZB70RNBBD93 --limit 20
  ```

---

## 9. Event Log, Streaming & Integrity (`ac events`)

Direct interface to the SQLite event store and live event distribution bus.

```bash
ac events <COMMAND>
```

### 9.1. `ac events subscribe`
Stream live events from the daemon in real-time.

```bash
ac events subscribe
```

### 9.2. `ac events verify`
Cryptographically verify sequence number continuity and event hash integrity in the database.

```bash
ac events verify [--db <PATH>]
```

### 9.3. `ac events replay`
Replay the entire recorded history of events from the beginning of time against the state machine to verify zero divergence.

```bash
ac events replay [--db <PATH>]
```

---

## 10. Daemon Status & Health (`ac status`)

Check if the Agent Control background supervisor daemon is running and healthy.

```bash
ac status
```

* **Sample Output:**
  ```json
  {
    "status": "running",
    "version": "1.0.0"
  }
  ```

---

## 11. Antigravity Launcher CLI (`agy`)

The `agy` command is the daily driver for Google Antigravity developers. It provides seamless account switching, automatic local token import, and browser OAuth.

```bash
agy [ACCOUNT_LABEL] [COMMAND]
```

### Usage Patterns:

1. **Launch with Specific Account:**
   ```bash
   agy "test1"
   # or
   agy "Personal Google"
   ```
2. **Interactive Selector:**
   ```bash
   agy
   ```
   If multiple accounts exist, `agy` renders an interactive selector displaying cooldown times and session counts.
3. **Add Account:**
   ```bash
   # Browser OAuth login:
   agy account add --name "College Google"

   # Pure terminal login:
   agy account add --name "Work Google" --cli
   ```
4. **List Antigravity Accounts:**
   ```bash
   agy account list
   ```

---

## 12. Interactive Terminal Dashboards (`agent-control` & `ac-tui`)

Launch the unified Ratatui TUI dashboard:

```bash
agent-control
# or
ac tui
# or
ac-tui
```

### TUI Navigation & Tabs:
* **`1`**: Dashboard (Overview of sessions, accounts, and inbox alerts)
* **`2`**: Sessions (All running and past sessions)
* **`3`**: Inbox (Pending human approvals and questions)
* **`4`**: Accounts (Manage accounts, add new via OAuth, check cooldowns)
* **`5`**: Projects (Repositories and workspace policies)
* **`6`**: Activity (Live event log and filter)
* **`Tab` / `BackTab`**: Cycle between tabs
* **`q`**: Quit TUI

### Session Actions (in Sessions Tab):
* **`n`**: **New Session Modal** (type a prompt and press Enter to launch)
* **`Space`**: **Smart Run / Resume**:
  * If `Idle`: Starts the session.
  * If `Paused`: Resumes the session.
  * If `Stopped`, `Failed`, or `Crashed`: Automatically re-runs the task and starts a fresh session.
* **`Enter` or `t`**: Open Full Transcript & Session Details.
* **`w`**: **Switch Account** (modal to rebind or hand off to another account).
* **`s`**: **Steer** (inject instruction into the running agent).
* **`p`**: **Pause** session.
* **`x`**: **Stop** session.
* **`r`**: Refresh data from daemon.

---

## 13. JSON Scripting & Automation Examples

By adding `--json`, all `ac` commands output structured JSON, making shell automation easy with `jq`.

### Get all currently active session IDs:
```bash
ac --json session list | jq -r '.[] | select(.state == "working") | .id'
```

### Stop all sessions running under a specific account:
```bash
for sid in $(ac --json session list | jq -r '.[] | select(.account_id == "01M35ZSYYCT1W74CP2ZHWTWAGF") | .id'); do
    ac session stop "$sid" --reason "Batch cleanup"
done
```

### Automated health check script:
```bash
if ac status 2>&1 | grep -q '"status": "running"'; then
    echo "✓ Agent Control Daemon is healthy"
else
    echo "✗ Daemon is offline. Starting agentcontrold..."
    agentcontrold &
fi
```

---

## 14. End-to-End Practical Recipes

### Recipe 1: Authenticate Google Account & Start Coding
```bash
# 1. Login with Google OAuth (browser opens automatically)
ac login --name test1

# 2. Verify account is active
ac account list

# 3. Launch an autonomous task
ac session run --agent-type agy --account-id test1 --task "Find and fix broken unit tests in crates/ac-core"

# 4. View live sessions
ac session list
```

### Recipe 2: Steer an Agent and Approve Tool Calls
```bash
# 1. Check if agent requires input
ac interaction list-pending

# 2. View details of pending prompt
ac interaction get <INTERACTION_ID>

# 3. Approve tool execution
ac interaction approve <INTERACTION_ID> --response "Approved to run cargo test"

# 4. Inject mid-flight steer guidance
ac session steer <SESSION_ID> --message "Ensure backwards compatibility with Phase 2 schemas"
```

### Recipe 3: Switch Accounts Mid-Flight (Quota Hand-off)
```bash
# If your personal account reaches rate limit:
ac session switch-account <SESSION_ID> "Claude Work Account"
```

---

*(Agent Control version 1.0.0. Licensed under MIT.)*
