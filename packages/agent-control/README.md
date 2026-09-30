# agent-control

**Local-first control plane for coding agents (Antigravity, Claude Code, Codex, Devin CLI)**

`agent-control` manages, sandboxes, switches between, and monitors autonomous coding agents inside full-fidelity terminal sessions with hardware-isolated account profiles and zero-configuration launch flows.

## Quick Start

You can run Agent Control without manual compilation using `npx`:

```bash
npx agent-control
```

Or install it globally:

```bash
npm install -g agent-control
agent-control
```

On first execution, the runner detects your host operating system and CPU architecture, downloads the matching prebuilt release binary, and invokes the terminal interface with transparent signal and stdio forwarding.

## Shorthand Commands

The package includes aliases for common launcher flows:

```bash
# Launch the interactive terminal UI
agent-control

# Short alias
ac

# Launch or switch Antigravity (agy) agent sessions directly
agy

# Inspect daemon state or start the background control daemon
agentcontrold --help
```

## Supported Systems

- **Linux x86_64** (glibc 2.31+)
- **Linux aarch64** (ARM64)
- **macOS x86_64** (Intel)
- **macOS aarch64** (Apple Silicon)
- **Windows**: Run natively inside [WSL2](https://learn.microsoft.com/en-us/windows/wsl/install) (`npx agent-control`)

## License

MIT © Agent Control contributors. See [LICENSE](https://github.com/agentcontrol/agentcontrol/blob/main/LICENSE) for details.
