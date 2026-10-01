# Quick Start Guide

Get started with AgentControll in under two minutes.

---

## 1. Launch AgentControll

Start the interactive terminal environment:

```bash
agentcontroll
```

*(Or use shorthand `agy` for Antigravity-focused launcher, or `npx agentcontroll` if running via npm).*

On initial launch, if the background daemon (`agentcontrold`) is not running, AgentControll starts it automatically and opens the dashboard.

---

## 2. Connect an Agent Account

Autonomous agents like Antigravity require authenticated accounts. AgentControll isolates credentials per profile:

1. Press **`a`** to switch to the **Accounts** view, or run:
   ```bash
   agentcontroll account add
   ```
2. Choose your authentication method:
   - **Browser Login (`agy-browser`):** AgentControll prints a secure authorization link and listens on a local loopback port for the OAuth callback.
   - **Handoff Login (`agy-login-link`):** For headless servers or SSH sessions without a browser. Open the printed authorization link on any computer, complete authentication, and paste the redirect URL back into AgentControll.
   - **Existing Credentials (`agy-existing`):** Imports existing credentials from `~/.config/antigravity/`.
3. AgentControll validates the token with Google OAuth endpoints and creates an isolated profile at `~/.config/agentcontrol/profiles/<account-id>/`.


---

## 3. Starting an Agent Session

Press **`n`** from anywhere in the TUI to open the **New Session** modal.

### Step 3.1: Choose the Agent
Select the target agent adapter:
- **`antigravity`**: Google Antigravity autonomous coding agent.
- **`claude-code`**: Anthropic Claude Code terminal agent.

### Step 3.2: Select Working Directory
Use the interactive path input with tab-completion:
- Type relative or absolute paths (e.g. `~/projects/my-api`).
- Press **Tab** for real-time filesystem directory completion.

### Step 3.3: Select Provider Account
Choose from your connected accounts. AgentControll displays account status, quota health, and concurrency limits.

### Step 3.4: Choose Execution Mode
Select the execution model:
- **`Interactive` (Default):** Full two-way interactive PTY session. You can watch the agent run, review diffs, and type input directly.
- **`Autonomous` / `Headless`:** Runs in the background without blocking your terminal. You can attach to it at any time.

### Step 3.5: Configure Permission & Access Mode
Set the autonomy level for tool usage and command execution:
- **`Normal` (Recommended):** Standard safety checks and prompts for sensitive operations.
- **`Auto-Accept` / `YOLO`:** Grants permission for read/write tools and command execution without interactive confirmation prompts.

Press **Enter** on **`[Launch Session]`**. AgentControll spawns the isolated PTY process, sets up environment sandboxing, and displays the real-time agent output.

---

## 4. TUI Navigation & Control

| Key | Action |
|:---|:---|
| **`Tab` / `BackTab`** | Switch between tabs (`Sessions`, `Accounts`, `Projects`, `Policies`, `Audit Log`) |
| **`n`** | Open New Session launcher modal |
| **`Enter`** | Attach to the currently selected session PTY |
| **`Ctrl+]`** | Detach from the active session back to AgentControll dashboard |

| **`p`** | Pause the running agent session (`SIGSTOP`) |
| **`r`** | Resume a paused agent session (`SIGCONT`) |
| **`x` / `Delete`** | Terminate an agent session (`SIGTERM` / `SIGKILL`) |
| **`?`** | Toggle Help & Keybinding reference modal |
| **`q`** | Exit the TUI (all background agent sessions continue executing) |

---

## 5. Next Steps

- Consult [TUI Guide](TUI.md) for full keyboard navigation and multi-session management.
- Read [Configuration Guide](CONFIGURATION.md) to customize ports, paths, and policy rules.
- Review [Architecture Guide](ARCHITECTURE.md) to understand how PTY multiplexing and daemon isolation work.
