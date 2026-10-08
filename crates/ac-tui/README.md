# ac-tui: Built-in Terminal Emulator & Supervisor Console

`ac-tui` is the terminal interface and virtual terminal emulator crate of **AgentControll**. Built on [`ratatui`](https://crates.io/crates/ratatui) and [`crossterm`](https://crates.io/crates/crossterm), it provides an interactive control plane for monitoring, supervising, steering, and interacting with coding agents (such as Google Antigravity `agy`, Claude Code, and generic shells) inside full-fidelity PTY sessions.

---

## Architecture Overview

```text
 ┌─────────────────────────────────────────────────────────────┐
 │                         ac-tui App                          │
 │  ┌───────────────────────────────────────────────────────┐  │
 │  │ Dashboard │ Sessions │ Accounts │ Activity │ Settings │  │
 │  └──────────────────────────┬────────────────────────────┘  │
 │                             ▼                               │
 │         Active Session Full-Screen Terminal View            │
 │  ┌───────────────────────────────────────────────────────┐  │
 │  │  TerminalBuffer (crates/ac-tui/src/terminal_buffer.rs)│  │
 │  │  • Normal & Alternate Screens (DECCOLM / 1049h / 1049l)│ │
 │  │  • Multi-byte UTF-8 stream decoder (decode_utf8_stream) │
 │  │  • Truecolor (24-bit RGB) & 256-color cell grid       │  │
 │  │  • Non-destructive cursor navigation (cub1 / CSI D)   │  │
 │  │  • Mouse selection, drag autoscroll, & OSC 52 yank    │  │
 │  │  • Incremental scrollback regex search ('/')          │  │
 │  │  • Interactive URL extractor & browser picker ('o')   │  │
 │  │  • DECSCUSR cursor shapes, OSC 7/8/9/777 tracking     │  │
 │  └──────────────────────────┬────────────────────────────┘  │
 └─────────────────────────────┼───────────────────────────────┘
                               │ IPC (Unix Domain Socket / WS)
                               ▼
 ┌─────────────────────────────────────────────────────────────┐
 │                agentcontrold Daemon & PTY                   │
 └─────────────────────────────────────────────────────────────┘
```

---

## Built-in Terminal Emulator Features

### 1. Full-Fidelity VT100 / xterm-256color & TrueColor Engine
- **Dedicated Virtual Buffer:** Implemented in `crates/ac-tui/src/terminal_buffer.rs`, emulating a full virtual screen grid with style attributes, hyperlinks, and scrolling margins.
- **Alternate Screen Buffer (`CSI ? 1049 h` / `l`):** Dedicated buffer switching for full-screen curses/TUI tools (like `vim`, `nano`, `htop`, or `less`). Returning to the main screen restores the complete scrollback history intact.
- **Distraction-Free Full-Screen Mode (`Ctrl+F`):** Toggles 100% edge-to-edge terminal mode without headers or footers. Pressing `Ctrl+F` again restores the normal layout.
- **PTY Boundary Decoding (`decode_utf8_stream`):** Multi-byte UTF-8 streams split across raw PTY read chunks are buffered and decoded without emitting replacement glyphs (`U+FFFD`), preserving complex Unicode, emojis, and Braille spinners (`⡿`, `⢿`, `⣻`).
- **Trailing Echo Hygiene:** Intelligently prunes blank cells from terminal backspace echoes (`\x08 \x08`), preventing phantom wrapped lines upon terminal resize while strictly preserving prompt spacing.

### 2. Cursor Navigation & Arrow Key Integrity
- **Non-Destructive Movement (`cub1` / `\x08`):** Arrow key navigation (`Left` / `Right`) moves the cursor without deleting, overwriting, or truncating text at the end or middle of input lines.
- **ANSI Navigation (`CSI A/B/C/D`, `Home`, `End`):** Full support for standard ANSI cursor movement and absolute coordinate addressing (`CSI <col> G`, `CSI <row> d`).
- **Multi-Line Prompt History:** History navigation (`Up` / `Down`) tracks and recalls multi-line draft submissions as complete units rather than decomposing them line-by-line.

### 3. Mouse Selection & Edge Autoscrolling
- **Click-and-Drag Text Selection:** Click and drag the mouse across the terminal grid to select text across rows and columns with high-contrast highlighted cells.
- **Multi-Click Granularity:**
  - Single click: Clear selection and position cursor.
  - Double click: Select entire word bounded by whitespace and punctuation.
  - Triple click: Select entire line.
- **Boundary Autoscroll:** Dragging the mouse past the top or bottom viewport edge automatically scrolls the terminal scrollback history smoothly.
- **Clipboard Integration:** Selected regions can be copied via system clipboard, OSC 52 escape sequences, or right-click.

### 4. Interactive Search & URL Navigation
- **Scrollback Regex Search (`/`):** Instant search modal across all historical lines. Displays live match counts, highlights matches directly in the terminal buffer, and allows cycling with `n` (next) and `N` (previous).
- **Interactive URL Picker (`o`):** Scans the buffer for standard web URLs (`http://`, `https://`) and explicit OSC 8 hyperlinks. Lists all detected URLs in an interactive menu to copy (`c`) or open in the default web browser (`Enter`).
- **Visual Mode (`v`):** Keyboard-driven visual selection mode for selecting text without a mouse.

### 5. Advanced OSC Protocol Support
- **OSC 0 / OSC 2:** Dynamic window and terminal title updates from child processes.
- **OSC 7:** Working directory tracking (`file://<hostname>/<path>`) synchronizing workspace location.
- **OSC 8:** Explicit terminal hyperlinks with embedded clickable target URLs.
- **OSC 9 / OSC 777:** Task completion notifications forwarded from agent tools.
- **DECSCUSR (`CSI <n> q`):** Dynamic cursor shape changes (blinking/steady block, underline, bar).
- **Bracketed Paste Mode (`CSI ? 2004 h` / `l`):** Guards multi-line pasted commands from premature shell execution.

---

## Keyboard Shortcuts in Terminal View

| Key | Context | Action |
|:---|:---|:---|
| **`Esc`** | Attached PTY | Passed directly through to the child agent process. |
| **`Ctrl+]`** or **`Ctrl+Q`** | Attached PTY | Detach from terminal view back to supervisor dashboard. |
| **`Ctrl+F`** | Attached PTY | Toggle full-screen distraction-free terminal (hides header/footer). |
| **`Left` / `Right`** | Terminal Input | Move cursor backward/forward without deleting text. |
| **`Up` / `Down`** | Terminal Input | Navigate multi-line prompt submission history. |
| **`Shift+Up` / `Shift+Down`** | Attached PTY | Scroll viewport up/down through scrollback history. |
| **`PageUp` / `PageDown`** | Attached PTY | Page up/down through scrollback history. |
| **`/`** | Attached PTY | Open incremental scrollback search modal. |
| **`o`** | Attached PTY | Open interactive URL picker modal. |
| **`v`** | Attached PTY | Toggle keyboard visual selection mode. |
| **`Ctrl+Shift+C`** | Selection | Copy active text selection to system clipboard. |

---

## Running Verification & Tests

To check and run all unit, integration, and audit tests for `ac-tui`:

```bash
# Verify compilation and lints (0 warnings required)
cargo check -p ac-tui --all-targets

# Run all unit and integration tests
cargo test -p ac-tui

# Run comprehensive TUI audit tests
cargo test -p ac-tui --test tui_audit_tests
```
