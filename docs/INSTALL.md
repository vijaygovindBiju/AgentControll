# Installation Guide

This guide covers installing, updating, and removing Agent Control on supported platforms.

---

## Supported Systems

| Operating System | Architecture | Support Status | Notes |
|:---|:---|:---|:---|
| **Linux** | `x86_64` (AMD64) | Tier 1 (Verified) | glibc 2.31+, systemd/user session |
| **Linux** | `aarch64` (ARM64) | Tier 1 (Verified) | glibc 2.31+ (Raspberry Pi 4/5, AWS Graviton) |
| **macOS** | `x86_64` (Intel) | Supported | macOS 12+ |
| **macOS** | `aarch64` (Apple Silicon) | Supported | macOS 12+ (M1/M2/M3/M4) |
| **Windows** | `x86_64` | Via WSL2 | Run inside Ubuntu/Debian on WSL2 |

> [!NOTE]
> **Windows Note:** Agent Control relies on POSIX Unix domain sockets (`0600` permissions) and native terminal signals (`SIGWINCH`, `SIGSTOP`, `SIGCONT`). Windows users can run Agent Control inside **WSL2** (`wsl --install`).

---

## Method 1: NPX / NPM (Recommended)

Run without manual compilation or cloning using `npx`:

```bash
npx agent-control
```

Or install globally into your Node environment:

```bash
npm install -g agent-control
agent-control
```

The runner automatically detects your operating system and architecture, downloads the verified prebuilt release binary, caches it in `~/.cache/agentcontrol/bin/`, and invokes the terminal UI with transparent stdio and signal forwarding.

---

## Method 2: One-Line Shell Installer

Install precompiled binaries directly to `~/.local/bin` (or `/usr/local/bin` if run as root):

```bash
curl -fsSL https://raw.githubusercontent.com/agentcontrol/agentcontrol/main/install.sh | sh
```

### Installer Options

You can customize the installation using environment variables:

```bash
# Specify target directory
curl -fsSL https://raw.githubusercontent.com/agentcontrol/agentcontrol/main/install.sh | BIN_DIR=/usr/local/bin sh

# Install a specific release version
curl -fsSL https://raw.githubusercontent.com/agentcontrol/agentcontrol/main/install.sh | AGENTCONTROL_VERSION=1.0.0 sh
```

---

## Method 3: Prebuilt Release Tarballs

Download precompiled release archives and checksums from the [GitHub Releases](https://github.com/agentcontrol/agentcontrol/releases) page:

1. Download the archive for your architecture:
   ```bash
   # Example: Linux x86_64
   curl -LO https://github.com/agentcontrol/agentcontrol/releases/download/v1.0.0/agent-control-v1.0.0-x86_64-unknown-linux-gnu.tar.gz
   curl -LO https://github.com/agentcontrol/agentcontrol/releases/download/v1.0.0/SHA256SUMS
   ```

2. Verify integrity:
   ```bash
   sha256sum --check --ignore-missing SHA256SUMS
   ```

3. Extract binaries:
   ```bash
   tar -xzf agent-control-v1.0.0-x86_64-unknown-linux-gnu.tar.gz -C ~/.local/bin/
   ```

4. Verify installation:
   ```bash
   agent-control --version
   ```

---

## Method 4: Building from Source

### Prerequisites

- **Rust toolchain:** 1.75.0 or later (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- **C compiler & utilities:** `build-essential` (Ubuntu/Debian), `base-devel` (Arch), or Xcode command-line tools (macOS)
- **Git**

### Build Steps

1. Clone the repository:
   ```bash
   git clone https://github.com/agentcontrol/agentcontrol.git
   cd agentcontrol
   ```

2. Compile all workspace crates in release mode:
   ```bash
   cargo build --release --workspace
   ```

   > [!TIP]
   > If building on a resource-constrained partition, point cargo target to temporary storage:
   > `CARGO_TARGET_DIR=/tmp/ac-target CARGO_INCREMENTAL=0 cargo build --release --workspace`

3. The release binaries will be located in `target/release/`:
   - `agent-control`: Primary interactive launcher and CLI
   - `agentcontrold`: Background daemon and state engine
   - `agy`: Direct Antigravity session launcher
   - `ac`: Shorthand CLI

4. Install binaries to your PATH:
   ```bash
   cp target/release/{agent-control,agentcontrold,agy,ac} ~/.local/bin/
   ```

---

## Verifying PATH

Ensure `~/.local/bin` is in your shell's `PATH`:

```bash
echo $PATH | grep -q "$HOME/.local/bin" && echo "PATH is configured" || echo "PATH needs configuration"
```

If missing, add this line to your `~/.bashrc`, `~/.zshrc`, or shell profile:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Then reload your shell:

```bash
source ~/.bashrc  # or source ~/.zshrc
```

---

## Updating Agent Control

- **Via npm:**
  ```bash
  npm update -g agent-control
  ```
- **Via shell installer:** Re-run the installer command.
- **Via source:**
  ```bash
  git pull origin main
  cargo build --release --workspace
  cp target/release/{agent-control,agentcontrold,agy,ac} ~/.local/bin/
  ```

---

## Uninstalling

1. Remove installed binaries:
   ```bash
   rm -f ~/.local/bin/{agent-control,agentcontrold,agy,ac}
   # If installed via npm:
   npm uninstall -g agent-control
   ```

2. Clean binary cache:
   ```bash
   rm -rf ~/.cache/agentcontrol
   ```

3. *(Optional)* Remove configuration, databases, and account profiles:
   ```bash
   rm -rf ~/.config/agentcontrol
   ```

---

## Troubleshooting

### `command not found: agent-control`
Verify that `~/.local/bin` is in your `PATH` (`echo $PATH`). Restart your terminal after updating your profile.

### `Permission denied` on daemon socket
Agent Control stores its runtime socket at `~/.config/agentcontrol/agentcontrold.sock` with POSIX permissions `0600` (accessible only by your user). If permissions were altered or created by another user (e.g. via `sudo`), remove the stale socket:
```bash
rm -f ~/.config/agentcontrol/agentcontrold.sock
```

### Windows Subsystem for Linux (WSL2)
Ensure you are running inside a WSL2 Linux terminal rather than native Windows Command Prompt or PowerShell:
```powershell
wsl -d Ubuntu
```
Inside WSL2, execute `npx agent-control`.
