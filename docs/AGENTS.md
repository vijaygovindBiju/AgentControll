# Supported Agents

Agent Control interfaces with autonomous coding agents through adapter drivers that manage authentication, execution sandboxing, PTY multiplexing, and lifecycle monitoring.

---

## 1. Antigravity (`agy`)

Google Antigravity is a terminal-based autonomous coding agent. Agent Control provides deep integration for Antigravity, including multi-account credential rotation, profile isolation, and launch customization.

### Authentication & Credential Storage
- **Isolated Profiles:** Each Antigravity account is assigned a sandboxed home directory: `~/.config/agentcontrol/profiles/<account-id>/`. The adapter launches the agent with `HOME` set to this directory, preventing token collisions between concurrent accounts. Desktop keyring IPC is neutralized via `DBUS_SESSION_BUS_ADDRESS="disabled:"` so `agy` strictly reads its isolated profile token.
- **Login Flows:**
  - **`agy-browser`**: Spawns local OAuth loopback listener and opens the browser.
  - **`agy-login-link`**: Generates PKCE authorization URL for headless/remote machines; paste redirect URL back into CLI or TUI.
  - **`agy-existing`**: Imports tokens from existing `~/.config/antigravity/` installations.
- **Validation:** Credentials are validated against Google OAuth endpoints prior to session launch.

### Models
Antigravity selects the configured default model (e.g. Gemini 1.5 Pro / Ultra / custom checkpoints) or accepts command-line parameters passed through launch options.

### Execution Modes & Permissions
- **Normal (Default):** Standard permissions where the agent requests interactive confirmation before modifying files or executing destructive commands.
- **Auto-Accept / Autonomous:** Bypasses confirmation prompts for automated pipelines or unattended development loops (only offered if supported by the installed `agy` binary).

### Working Directories
Sessions execute in the project repository directory selected during launch. Agent Control verifies the path exists and has appropriate read/write permissions.

### Limitations
- Requires the `agy` binary to be installed on the host or available in `$PATH`.
- Multi-byte UTF-8 terminal sequences are decoded via `decode_utf8_stream` to avoid glyph corruption across split PTY reads.

---

## 2. Claude Code (`claude-code`)

Anthropic Claude Code is an agentic coding tool that operates directly in the terminal to inspect repositories, execute commands, edit code, and create commits.

### Authentication
- Credentials are read from standard environment variables (`ANTHROPIC_API_KEY`) or Claude CLI session tokens (`~/.claude/`).
- Future releases will isolate Claude CLI configuration directories similarly to Antigravity.

### Execution Modes
- **Interactive PTY:** Full terminal interaction through Agent Control's ANSI virtual screen.
- **Command Dispatch:** Can execute predefined instructions or prompt strings directly from CLI invocations.

### Limitations
- Requires `@anthropic-ai/claude-code` npm package installed (`npm install -g @anthropic-ai/claude-code`).
- Tool calls and permission prompts are rendered directly within the virtual terminal buffer.

---

## 3. Mock Agent (`mock`)

A lightweight built-in testing agent used for integration tests, unit verification, and CI pipelines.
- Simulates incremental stdout, interactive prompts, and error states without external API dependencies.
- Available when running development builds or unit tests.
