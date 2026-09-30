# Terminal User Interface (TUI) Reference

Agent Control features a high-performance terminal console built with `ratatui` and `crossterm`. It provides real-time multi-agent supervision, live PTY streaming, interactive steering, and account administration directly from the terminal.

---

## 1. Top-Level Navigation

You can switch between views using number shortcuts or tab cycles:

| Key | Tab View | Description |
|:---|:---|:---|
| **`1`** | **Dashboard** | Unified workspace overview with quick stats and recent activity. |
| **`2`** | **Sessions** | Active and historical agent processes with live terminal previews. |
| **`3`** | **Accounts** | Connected provider credentials, health, and concurrency limits. |
| **`4`** | **Activity** | Live event stream and audit trail with search and filtering. |
| **`5`** | **Agents** | Detected agent binaries (`agy`, `claude`, etc.) and capabilities. |
| **`6`** | **Settings** | Configuration browser and runtime settings. |
| **`Tab`** | Next Tab | Cycles forward through the tabs. |
| **`Shift+Tab`** / **`BackTab`** | Previous Tab | Cycles backward through the tabs. |

---

## 2. Global Shortcuts

| Key | Action |
|:---|:---|
| **`q`** | Exit Agent Control TUI (background daemon and agent sessions keep running). |
| **`?`** | Open the interactive Help modal. |
| **`r`** | Manually force refresh state from the `agentcontrold` daemon. |
| **`Esc`** | Close current modal dialog or cancel active input. |

---

## 3. Session Controls (`Sessions` Tab)

| Key | Action |
|:---|:---|
| **`↑` / `k`** | Navigate to the previous session in the list. |
| **`↓` / `j`** | Navigate to the next session in the list. |
| **`Enter`** | Attach to the active session's full-screen virtual terminal. |
| **`n`** or **`g`** | Open the **New Session** launcher modal. |
| **`s`** | Open **Steer Session** prompt to send instructions to the running agent. |
| **`p`** | **Pause** the running agent process (`SIGSTOP`). |
| **`r`** / **`c`** | **Resume** a paused agent process (`SIGCONT`). |
| **`x`** / **`Delete`** | Open confirmation to stop/kill the selected agent session. |
| **`w`** | Switch account for the selected session. |
| **`Ctrl+]`** | *(When attached)* Detach from terminal back to the Agent Control dashboard. |

---

## 4. Accounts Management (`Accounts` Tab)

| Key | Action |
|:---|:---|
| **`a`** | Open **Add Account** wizard (supports browser login, handoff link, and token import). |
| **`Ctrl+O`** | Launch the native Google OAuth flow for Antigravity (`agy`). |
| **`d`** / **`Delete`** | Remove selected account and purge credentials. |
| **`v`** | Validate credential against provider API endpoints. |

---

## 5. Interactive Directory Completion

When launching a new session in the **New Session** modal:

1. Navigate to the **Working Directory** field.
2. Begin typing a relative or absolute path (e.g. `~/dev/` or `/var/repo`).
3. Press **`Tab`**:
   - Agent Control scans the filesystem and auto-completes unique directory names.
   - If multiple matching directories exist, pressing `Tab` cycles through candidates.
   - Directory paths are verified before the session is spawned to avoid runtime path errors.
