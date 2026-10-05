# Installation Guide

This guide covers installing, updating, and removing AgentControll on supported platforms.

---

## Supported Systems

| Operating System | Architecture | Support Status | Notes |
|:---|:---|:---|:---|
| **Linux** | `x86_64` (AMD64) | Tier 1 (Verified) | Prebuilt binary (glibc 2.31+, systemd/user session) |
| **Linux** | `aarch64` (ARM64) | Source Build | Compile via `cargo build --release` |
| **macOS** | `x86_64` (Intel) | Supported | Prebuilt binary (macOS 12+) |
| **macOS** | `aarch64` (Apple Silicon) | Supported | Prebuilt binary (macOS 12+, M1/M2/M3/M4) |
| **Windows** | `x86_64` | Via WSL2 | Run inside Ubuntu/Debian on WSL2 |

> [!NOTE]
> **Windows Note:** AgentControll relies on POSIX Unix domain sockets (`0600` permissions) and native terminal signals (`SIGWINCH`, `SIGSTOP`, `SIGCONT`). Windows users can run AgentControll inside **WSL2** (`wsl --install`).

---

## Method 1: NPX / NPM (Recommended)

Run without manual compilation or cloning using `npx`:

```bash
npx agentcontroll
```

Or install globally into your Node environment:

```bash
npm install -g agentcontroll
agentcontroll
```

The runner automatically detects your operating system and architecture, downloads the verified prebuilt release binary, caches it in `~/.cache/agentcontrol/bin/`, and invokes the terminal UI with transparent stdio and signal forwarding.

---

## Method 2: One-Line Shell Installer

Install precompiled binaries directly to `~/.local/bin` (or `/usr/local/bin` if run as root):

```bash
curl -fsSL https://raw.githubusercontent.com/vijaygovindBiju/AgentControll/main/install.sh | sh
```

### Installer Options

You can customize the installation using environment variables:

```bash
# Specify target directory
curl -fsSL https://raw.githubusercontent.com/vijaygovindBiju/AgentControll/main/install.sh | BIN_DIR=/usr/local/bin sh

# Install a specific release version
curl -fsSL https://raw.githubusercontent.com/vijaygovindBiju/AgentControll/main/install.sh | AGENTCONTROL_VERSION=1.0.0 sh
```

---

## Method 3: Prebuilt Release Tarballs

Download precompiled release archives and checksums from the [GitHub Releases](https://github.com/vijaygovindBiju/AgentControll/releases) page:

1. Download the archive for your architecture:
   ```bash
   # Example: Linux x86_64
   curl -LO https://github.com/vijaygovindBiju/AgentControll/releases/download/v1.0.0/agentcontroll-v1.0.0-x86_64-unknown-linux-gnu.tar.gz
   curl -LO https://github.com/vijaygovindBiju/AgentControll/releases/download/v1.0.0/SHA256SUMS
   ```

2. Verify integrity:
   ```bash
   sha256sum --check --ignore-missing SHA256SUMS
   ```

3. Extract binaries:
   ```bash
   tar -xzf agentcontroll-v1.0.0-x86_64-unknown-linux-gnu.tar.gz -C ~/.local/bin/
   ```

4. Verify installation:
   ```bash
   agentcontroll --version
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
   git clone https://github.com/vijaygovindBiju/AgentControll.git
   cd AgentControll
   ```

2. Compile all workspace crates in release mode:
   ```bash
   cargo build --release --workspace
   ```

   > [!TIP]
   > If building on a resource-constrained partition, point cargo target to a physical drive with ample free space:
   > `CARGO_TARGET_DIR=$HOME/.cache/ac-target CARGO_INCREMENTAL=0 cargo build --release --workspace`
   > Avoid redirecting to `/tmp` if it is mounted as a memory-backed `tmpfs`, as linking large binaries can exhaust memory and fail with `signal 7 [Bus error]`.

3. The release binaries will be located in `target/release/`:
   - `agentcontroll`: Primary interactive launcher and CLI
   - `agent-control`: Backward-compatible launcher alias
   - `agentcontrold`: Background daemon and state engine
   - `ac`: Shorthand CLI

4. Install binaries to your PATH:
   ```bash
   cp target/release/{agentcontroll,agent-control,agentcontrold,ac} ~/.local/bin/
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

## Updating AgentControll

- **Via npm:**
   ```bash
   npm update -g agentcontroll
   ```
- **Via shell installer:** Re-run the installer command.
- **Via source:**
   ```bash
   git pull origin main
   cargo build --release --workspace
   cp target/release/{agentcontroll,agent-control,agentcontrold,ac} ~/.local/bin/
   ```

---

## Uninstalling

1. Remove installed binaries:
   ```bash
   rm -f ~/.local/bin/{agentcontroll,agent-control,agentcontrold,ac}
   # If installed via npm:
   npm uninstall -g agentcontroll
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

### `command not found: agentcontroll`
Verify that `~/.local/bin` is in your `PATH` (`echo $PATH`). Restart your terminal after updating your profile.

### `Permission denied` on daemon socket
AgentControll stores its runtime socket at `~/.config/agentcontrol/agentcontrold.sock` with POSIX permissions `0600` (accessible only by your user). If permissions were altered or created by another user (e.g. via `sudo`), remove the stale socket:
```bash
rm -f ~/.config/agentcontrol/agentcontrold.sock
```

### Windows Subsystem for Linux (WSL2)
Ensure you are running inside a WSL2 Linux terminal rather than native Windows Command Prompt or PowerShell:
```powershell
wsl -d Ubuntu
```
Inside WSL2, execute `npx agentcontroll`.

### `ld terminated with signal 7 [Bus error]` or `No space left on device` during build/test
This issue occurs when:
1. The filesystem partition holding the repository or build cache runs out of free disk space, OR
2. `CARGO_TARGET_DIR` is pointed to `/tmp` on Linux systems where `/tmp` is mounted as a memory-backed `tmpfs` (RAM filesystem). Linking large binaries and test suites consumes several gigabytes of target directory space, which rapidly exhausts tmpfs memory and causes the dynamic linker (`rust-lld` or `ld`) to crash with SIGBUS (`signal 7 [Bus error]`) or `No space left on device`.

**Resolution:**
1. Check available disk space and filesystem mount types:
   ```bash
   df -h /tmp $HOME
   ```
2. Clean up any stale build directories consuming `/tmp` memory:
   ```bash
   rm -rf /tmp/ac-target
   ```
3. Set `CARGO_TARGET_DIR` to a directory on a physical partition with at least 5GB of free space (e.g. `~/.cache/ac-target`), and disable incremental compilation to reduce peak storage:
   ```bash
   CARGO_TARGET_DIR=$HOME/.cache/ac-target CARGO_INCREMENTAL=0 cargo build --release --workspace
   ```
