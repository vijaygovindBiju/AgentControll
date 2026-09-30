# Configuration Guide

Agent Control operates with zero required configuration out of the box, using sensible defaults following the XDG Base Directory specification. For custom deployments, behavior can be customized using a TOML configuration file and environment variables.

---

## Configuration File Location

The background daemon (`agentcontrold`) searches for its configuration file in the following order:

1. **`$AC_CONFIG`** environment variable (if set).
2. **`$XDG_CONFIG_HOME/agentcontrol/config.toml`** (typically `~/.config/agentcontrol/config.toml`).
3. If no file exists, internal defaults are used automatically.

### Example `config.toml`

```toml
# Path to SQLite event store
db_path = "~/.local/share/agentcontrol/events.db"

# Path to Unix domain socket for IPC communication
socket_path = "/run/user/1000/agentcontrol.sock"

# Maximum restart attempts before marking a crashed session as Failed
max_restarts = 3

# Base exponential backoff delay (in milliseconds) for session restarts
restart_backoff_base_ms = 1000

# Daemon logging level: "error", "warn", "info", "debug", "trace"
log_level = "info"

# WebSocket server toggle (loopback only)
ws_enabled = true

# Loopback bind address for WebSocket API
ws_bind_addr = "127.0.0.1:4242"

# Optional path to token file for WebSocket authentication (<token>:<scope>)
# ws_auth_token_path = "~/.config/agentcontrol/ws_tokens.txt"
```

---

## Settings Reference

| Key | Type | Default | Description |
|:---|:---|:---|:---|
| `db_path` | String (Path) | `~/.local/share/agentcontrol/events.db` | SQLite event store database file path. |
| `socket_path` | String (Path) | `$XDG_RUNTIME_DIR/agentcontrol.sock` | POSIX Unix domain socket used for IPC between CLI/TUI and daemon. |
| `max_restarts` | Integer | `3` | Number of restart attempts before a crashing agent enters `Failed` state. |
| `restart_backoff_base_ms` | Integer | `1000` | Starting delay in ms for exponential backoff on crashes. |
| `log_level` | String | `"info"` | Tracing level filter (`error`, `warn`, `info`, `debug`, `trace`). |
| `ws_enabled` | Boolean | `true` | Enables loopback WebSocket server for real-time telemetry streaming. |
| `ws_bind_addr` | String | `"127.0.0.1:4242"` | IP and port for the WebSocket server (strictly loopback `127.0.0.1`). |
| `ws_auth_token_path` | String (Path) | `None` | Path to optional file containing `<token>:<scope>` pairs for token auth. |

---

## Environment Variables

| Variable | Default | Purpose |
|:---|:---|:---|
| `AC_CONFIG` | None | Overrides the path to `config.toml`. |
| `AC_LOG` | Config `log_level` | Tracing filter string for daemon output (e.g. `debug,ac_core::pty=trace`). |
| `RUST_LOG` | `"info"` | Fallback tracing filter if `AC_LOG` is unset. |
| `AGENTCONTROL_BIN_DIR` | None | Override directory path to look for prebuilt binaries before downloading. |
| `XDG_CONFIG_HOME` | `~/.config` | Base path for configuration files and profile credentials. |
| `XDG_DATA_HOME` | `~/.local/share` | Base path for persistent SQLite event databases. |
| `XDG_RUNTIME_DIR` | `/tmp` | Directory for POSIX domain sockets (`mode 0600`). |

---

## Precedence Order

When resolving configuration options, Agent Control applies precedence in the following descending order:

1. **CLI Flags** (e.g., `--socket <path>`, `--log-level <level>`)
2. **Environment Variables** (e.g., `AC_LOG`, `AC_CONFIG`)
3. **Configuration File** (`config.toml`)
4. **Built-in Defaults**
