# agentcontroll

**Local-first control plane for coding agents (Antigravity, Claude Code, Codex, Devin CLI)**

`agentcontroll` manages, sandboxes, switches between, and monitors autonomous coding agents inside full-fidelity terminal sessions with hardware-isolated account profiles and zero-configuration launch flows.

## Quick Start

You can run AgentControll without manual compilation using `npx`:

```bash
npx agentcontroll
```

Or install it globally:

```bash
npm install -g agentcontroll
agentcontroll
```

On first execution, the runner detects your host operating system and CPU architecture, downloads the matching prebuilt release binary, and invokes the terminal interface with transparent signal and stdio forwarding.

## Built-in Virtual Terminal Emulator

AgentControll features a built-in virtual terminal engine (`TerminalBuffer`) specifically optimized for interactive AI agent workflows:

- **Full-Fidelity PTY Emulation:** ANSI 16-color, 256-color, 24-bit TrueColor, alternate screen (`1049h`/`1049l`), and stream-safe UTF-8 multi-byte decoding (preserving complex Unicode, Braille spinners, and emojis).
- **Distraction-Free Full Screen (`Ctrl+F`):** Instantly maximizes the terminal to 100% of the display area with zero headers or footers; pressing `Ctrl+F` again restores the normal layout.
- **Non-Destructive Cursor Navigation:** Full arrow key (`Left`, `Right`, `Up`, `Down`, `Home`, `End`) navigation without erasing, deleting, or corrupting input characters.
- **Mouse Selection & Autoscroll:** Click and drag to highlight and select text, double-click for words, triple-click for lines, with automatic edge scrolling when dragging beyond viewport boundaries.
- **Scrollback Regex Search (`/`):** Instant search modal across all historical lines with live match highlighting and `n` / `N` cycling.
- **Interactive URL Picker (`o`):** Automatically extracts web links and OSC 8 hyperlinks into a searchable menu to open (`Enter`) or copy (`c`).
- **Visual Mode (`v`) & OSC 52 Clipboard:** Keyboard-driven visual text selection and yank (`y`) to system clipboard, plus native handling of OSC 52 ANSI clipboard escapes.
- **Customizable Keybindings:** Remap any supervisor hotkey interactively via Settings (`6`) or `~/.config/agentcontrol/settings.json`.
- **Multi-Line Prompt History:** Recalls multi-line prompt drafts as atomic units.
- **Transparent Signal Passing:** `Esc` is forwarded directly to the child agent (for vi, nano, fzf, CLI prompts), while `Ctrl+]` or `Ctrl+Q` safely detaches back to the supervisor dashboard.

## Commands

The package includes the standard suite of commands:

```bash
# Launch the interactive terminal UI
agentcontroll

# Shorthand alias
ac

# Diagnose installation health, credentials, and account switching isolation
agentcontroll doctor
# or
ac doctor

# Deep diagnostics (scans for orphan profiles and unreferenced credentials)
agentcontroll doctor --deep

# Machine-readable JSON output
agentcontroll doctor --json

# Inspect daemon state or start the background control daemon
agentcontrold --help
```

> **Note on Antigravity (`agy`):** AgentControll manages external Antigravity sessions and credentials, but does **not** install, wrap, or replace the external Google Antigravity CLI (`agy`). The real `agy` executable must be installed separately on your system.

## Supported Systems

- **Linux x86_64** (glibc 2.31+)
- **macOS x86_64** (Intel)
- **macOS aarch64** (Apple Silicon)
- **Windows**: Run natively inside [WSL2](https://learn.microsoft.com/en-us/windows/wsl/install) (`npx agentcontroll`)

## License

MIT © AgentControll contributors. See [LICENSE](https://github.com/vijaygovindBiju/AgentControll/blob/main/LICENSE) for details.
