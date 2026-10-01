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

## Commands

The package includes the standard suite of commands:

```bash
# Launch the interactive terminal UI
agentcontroll

# Shorthand alias
ac

# Launch or switch Antigravity (agy) agent sessions directly
agy

# Inspect daemon state or start the background control daemon
agentcontrold --help
```

## Supported Systems

- **Linux x86_64** (glibc 2.31+)
- **macOS x86_64** (Intel)
- **macOS aarch64** (Apple Silicon)
- **Windows**: Run natively inside [WSL2](https://learn.microsoft.com/en-us/windows/wsl/install) (`npx agentcontroll`)

## License

MIT © AgentControll contributors. See [LICENSE](https://github.com/vijaygovindBiju/AgentControll/blob/main/LICENSE) for details.

