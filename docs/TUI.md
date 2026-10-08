# Terminal User Interface (TUI) & Virtual Terminal Emulator Reference

AgentControll features a high-performance terminal console and virtual terminal emulator built with [`ratatui`](https://crates.io/crates/ratatui) and [`crossterm`](https://crates.io/crates/crossterm). It provides real-time multi-agent supervision, live PTY streaming, interactive steering, account administration, and full-fidelity terminal emulation directly inside your shell.

---

## 1. Top-Level Navigation

Switch between primary supervisor views using number shortcuts or tab cycles:

| Key | Tab View | Description |
|:---|:---|:---|
| **`1`** | **Dashboard** | Unified workspace overview with system telemetry (CPU, RAM, event rate) and active session cards. |
| **`2`** | **Sessions** | Active and historical agent processes with live terminal previews. |
| **`3`** | **Accounts** | Connected provider credentials, quota health, concurrency caps, and cooldown timers. |
| **`4`** | **Activity** | Append-only event stream and audit trail with live filtering and structured detail inspections. |
| **`5`** | **Agents** | Detected agent binaries (`agy`, `claude`, etc.) and system capabilities. |
| **`6`** | **Settings** | Configuration browser and runtime settings. |
| **`Tab`** | Next Tab | Cycles forward through tabs. |
| **`Shift+Tab`** / **`BackTab`** | Previous Tab | Cycles backward through tabs. |

---

## 2. Global Shortcuts

| Key | Action |
|:---|:---|
| **`q`** | Exit AgentControll TUI (daemon and child agent sessions continue executing in background). |
| **`?`** | Open the interactive Help & Keybindings modal. |
| **`r`** | Force refresh state from the `agentcontrold` supervisor daemon. |
| **`Esc`** | Close current modal dialog or cancel active prompt input. |

---

## 3. Session Controls (`Sessions` Tab)

| Key | Action |
|:---|:---|
| **`↑` / `k`** | Navigate to previous session in the list. |
| **`↓` / `j`** | Navigate to next session in the list. |
| **`Enter`** | Attach to the selected session's full-screen virtual terminal. |
| **`n`** or **`g`** | Open the **New Session** launcher modal. |
| **`s`** | Open **Steer Session** prompt to inject instructions into running agent. |
| **`p`** | **Pause** the running agent process (`SIGSTOP`). |
| **`r`** / **`c`** | **Resume** a paused agent process (`SIGCONT`). |
| **`x`** / **`Delete`** | Open confirmation dialog to gracefully stop/terminate the agent session. |
| **`w`** | Open **Switch Account** modal (controlled hand-off with context preservation). |
| **`Ctrl+]`** / **`Ctrl+Q`** | *(When attached)* Detach from terminal view back to supervisor dashboard. |

---

## 4. Built-in Terminal Emulator (`TerminalBuffer`)

AgentControll's terminal emulator (`crates/ac-tui/src/terminal_buffer.rs`) is designed to host demanding interactive CLI tools (including Google Antigravity `agy`, Claude Code, bash, nano, vim, and fzf) with complete visual fidelity.

### Visual & Escape Sequence Capabilities

- **ANSI / TrueColor Graphics:** Supports standard 16 ANSI colors, 256-color palette, and full 24-bit TrueColor (RGB) escape sequences (`\x1b[38;2;R;G;Bm`).
- **Alternate Screen Buffer (`CSI ? 1049 h` / `l`):** Seamlessly handles full-screen programs (vim, nano, htop, less). While in the alternate screen, scrollback history from the normal screen is safely preserved and restored upon exit.
- **PTY Boundary UTF-8 Decoding:** Employs `decode_utf8_stream` to prevent multi-byte UTF-8 glyphs (emojis, complex scripts, Braille spinners) from splitting across PTY read chunks.
- **Trailing Space & Echo Hygiene:** Intelligently prunes blank cells from terminal backspace echoes (`\x08 \x08`), preventing phantom wrapped lines on terminal resize while keeping intentional trailing spaces.

### Cursor Navigation & Arrow Key Integrity

- **Non-Destructive Cursor Navigation:** Left (`\x1b[D` or `\x08` / `cub1`) and Right (`\x1b[C`) arrow keys move the cursor without deleting, overwriting, or truncating text at the end or in the middle of input lines.
- **ANSI Addressing:** Full support for `Home` (`\x1b[H`), `End` (`\x1b[F`), relative jumps, and absolute coordinate addressing (`CSI <col> G`, `CSI <row> d`).
- **Multi-Line Prompt History:** History navigation (`Up` / `Down`) recalls complete multi-line prompts as atomic submissions rather than decomposing them line-by-line.

### Mouse Selection & Edge Autoscrolling

- **Click-and-Drag Text Selection:** Click and drag the mouse across the terminal viewport to highlight and select text.
- **Multi-Click Selection:**
  - Single click: Clear selection and set position.
  - Double click: Select word bounded by whitespace and punctuation.
  - Triple click: Select entire line.
- **Viewport Boundary Autoscroll:** Dragging the cursor above the top border or below the bottom border automatically scrolls the viewport smoothly through scrollback history.
- **Copying Selection:** Copy selected text via system clipboard, OSC 52 escape sequences, `Ctrl+Shift+C`, or right-click.

### Interactive Search & URL Palette

- **Scrollback Regex Search (`/`):**
  - Press **`/`** while in the terminal to open incremental search.
  - Type any regex or plain text string; matching rows are highlighted with bright high-contrast badges.
  - Cycle matches with **`n`** (next match) and **`N`** (previous match).
  - Press **`Esc`** to exit search mode and restore normal cursor position.
- **Interactive URL Picker (`o`):**
  - Press **`o`** to scan the entire terminal scrollback for HTTP/HTTPS web links and OSC 8 hyperlinks.
  - Use **`↑` / `↓`** to navigate links, **`c`** to copy the URL to clipboard, or **`Enter`** to open directly in the system web browser.
- **Visual Mode (`v`):**
  - Press **`v`** to toggle keyboard-driven visual selection mode.
  - Move selection cursor with arrow keys or vi navigation keys (`h`, `j`, `k`, `l`).
  - Press **`y`** to yank selected text to clipboard.

### Operating System Controls (OSC) & Notifications

- **OSC 0 / OSC 2:** Captures dynamic window and tab title updates from child processes.
- **OSC 7:** Tracks current working directory (`file://<hostname>/<path>`) synchronizing workspace state.
- **OSC 8:** Explicit terminal hyperlinks with embedded clickable URL targets.
- **OSC 9 / OSC 777:** Desktop notifications forwarded from agent tasks.
- **DECSCUSR (`CSI <n> q`):** Dynamic cursor shape styling (blinking block, steady block, underline, bar).
- **Bracketed Paste Mode (`CSI ? 2004 h` / `l`):** Guards multi-line pasted commands against accidental execution.

---

## 5. Accounts Management (`Accounts` Tab)

| Key | Action |
|:---|:---|
| **`a`** | Open **Add Account** wizard (supports browser login, handoff link, and token import). |
| **`Ctrl+O`** | Launch the native Google OAuth flow for Antigravity (`agy`). |
| **`d`** / **`Delete`** | Open confirmation dialog to remove selected account and purge credentials. |
| **`v`** | Validate credential against provider API endpoints. |

---

## 6. Interactive Directory Completion

When launching a new session in the **New Session** modal:

1. Navigate to the **Working Directory** field.
2. Begin typing a relative or absolute path (e.g. `~/dev/` or `/var/repo`).
3. Press **`Tab`**:
   - AgentControll scans the filesystem and auto-completes unique directory names.
   - If multiple matching directories exist, pressing `Tab` cycles through candidates.
   - Directory paths are verified before the session is spawned to avoid runtime path errors.
