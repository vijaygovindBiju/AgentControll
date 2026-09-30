# Control API Changelog & Stability Guarantee

This document defines the stability commitment, schema versioning policy, and integration specifications for the **Agent Control v1 Control API**.

---

## 1. Stability Commitment & Versioning Policy

### Schema Version Freeze: `v: 1`
- The wire protocol schema version is frozen at `v = 1`.
- Every client request must include `"v": 1`. Any request with an unsupported or missing version returns a structured error:
  ```json
  {
    "v": 1,
    "id": "<req-id>",
    "ok": false,
    "error": {
      "code": "VersionMismatch",
      "message": "Unsupported schema version: 2"
    }
  }
  ```
- **Backward Compatibility**: Fields will not be removed or renamed in v1. Optional fields may be added additively with default values (`#[serde(default)]`).
- **Breaking Changes**: Any breaking change requires a new protocol version increment (e.g. `v: 2`) with an explicit deprecation cycle.

---

## 2. Transports

Agent Control provides two transports sharing the identical request dispatcher and command set:

### A. Unix Domain Socket (Local IPC)
- **Path**: `$XDG_RUNTIME_DIR/agentcontrol/agentcontrol.sock` (or `/tmp/agentcontrol.sock`).
- **Permissions**: Strict filesystem mode `0600` (owner read/write only).
- **Format**: Newline-delimited JSON (`\n`).
- **Intended Clients**: Local CLI (`agentcontrol`), Ratatui TUI (`ac-tui`), system scripts running under the same user UID.

### B. WebSocket Loopback (External Integrations / ADR-009)
- **Address**: `ws://127.0.0.1:4242` (configurable via `ws_bind_addr` in `config.toml`).
- **Loopback Enforcement**: Guaranteed loopback binding (`127.0.0.1`); never bound to external interfaces.
- **Intended Clients**: GUI applications (e.g. **AgentDesk**), orchestrators (e.g. **AgentMesh**), browser devtools, extension bridges.

---

## 3. Authentication & Scopes (WebSocket)

WebSocket endpoints require authentication unless token verification is explicitly left unconfigured.

### Authentication Methods
1. **HTTP Authorization Header**:
   ```http
   GET / HTTP/1.1
   Host: 127.0.0.1:4242
   Upgrade: websocket
   Connection: Upgrade
   Authorization: Bearer <token>
   ```
2. **URL Query Parameter**:
   ```text
   ws://127.0.0.1:4242/?token=<token>
   ```
3. **In-Band WebSocket Handshake Message**:
   ```json
   {
     "v": 1,
     "id": "auth-1",
     "cmd": "auth",
     "params": {
       "token": "<token>"
     }
   }
   ```

### Scopes
| Scope | Permissions | Permitted Commands |
|---|---|---|
| `read` | Read-only observation and querying | `*.list`, `*.get`, `events.query`, `events.subscribe`, `events.unsubscribe`, `account.query_availability`, `project.workspaces`, `policy.test`, `daemon.status` |
| `write` | Session execution and interaction management | All `read` commands + `session.*` (create, start, stop, pause, resume, steer, input, resize, remove, select_account, switch_account, handoff), `interaction.reply`, `interaction.dismiss` |
| `admin` | Full administrator control | All `write` commands + `account.register`, `account.disable`, `account.enable`, `account.remove`, `project.register`, `project.remove`, `policy.upsert`, `policy.remove` |

If a client with `read` scope attempts a mutating command, the server returns:
```json
{
  "v": 1,
  "id": "create-1",
  "ok": false,
  "error": {
    "code": "PermissionDenied",
    "message": "Token scope 'read' does not allow mutating command 'session.create'"
  }
}
```

---

## 4. Advanced Event Subscription & Catch-Up Replay

Clients can subscribe to live daemon events using `events.subscribe`.

### Request Schema
```json
{
  "v": 1,
  "id": "sub-1",
  "cmd": "events.subscribe",
  "params": {
    "filter": {
      "kinds": ["session.started", "session.stopped", "human.interaction.requested"],
      "session_id": "01J7ABCDEF...",
      "project_id": "01J7FEDCBA...",
      "account_id": "01J7112233...",
      "since_seq": 104
    }
  }
}
```

### Response & Streaming
1. **Acknowledgement**:
   ```json
   {
     "v": 1,
     "id": "sub-1",
     "ok": true,
     "result": {}
   }
   ```
2. **Historical Replay**:
   If `since_seq` is provided, the daemon replays all matching events from SQLite with sequence number `seq >= since_seq` before streaming live events.
3. **Live Event Stream**:
   ```json
   {
     "v": 1,
     "event": {
       "id": "01J7...",
       "seq": 105,
       "timestamp": "2026-09-22T18:00:00Z",
       "kind": "session.started",
       "session_id": "01J7ABCDEF...",
       "payload": { ... },
       "source": "adapter:mock"
     }
   }
   ```
4. **Unsubscribe**:
   Send `{"v": 1, "id": "unsub-1", "cmd": "events.unsubscribe"}` to stop event delivery while keeping the connection open for command execution.

---

## 5. Standard Error Codes Catalog

| Code | HTTP / WS Equivalent | Description |
|---|---|---|
| `ValidationError` | 400 Bad Request | Request JSON is invalid or missing required parameters |
| `VersionMismatch` | 400 Bad Request | Protocol version `v` is not 1 |
| `Unauthorized` | 401 Unauthorized | Missing or invalid authentication token |
| `PermissionDenied` | 403 Forbidden | Token scope does not permit the requested command |
| `NotFound` | 404 Not Found | Referenced session, account, project, or policy not found |
| `InvalidState` | 409 Conflict | Action not permitted in current session or account state |
| `AccountNotFound` | 404 Not Found | Specified account does not exist |
| `IncompatibleAccount` | 400 Bad Request | Account does not support the requested agent type or tags |
| `AccountUnavailable` | 409 Conflict | Account is disabled or rate-limited |
| `ConcurrencyLimitReached` | 429 Too Many Requests | Account has reached its active session cap |
| `NoAccountAvailable` | 409 Conflict | Automatic account selection found no compatible available account |
| `UnsupportedCapability` | 400 Bad Request | Underlying adapter does not support requested capability |
| `SnapshotFailed` | 500 Internal Error | Failed to capture safe session snapshot |
| `HandOffFailed` | 500 Internal Error | Session hand-off or process restart failed |
| `StaleSession` | 409 Conflict | Session was modified concurrently or state has drifted |
| `InternalError` | 500 Internal Error | Unexpected daemon failure |

---

## 6. Integration Guidance

### For AgentDesk (Desktop GUI)
1. **Connection**: Connect via WebSocket to `ws://127.0.0.1:4242/?token=<user_token>` using `read` or `write` scope.
2. **Initial Sync**:
   - Issue `session.list`, `account.list`, `project.list`, `interaction.list_pending`.
   - Issue `events.subscribe` with `since_seq: <last_known_seq>` to resume seamless event processing.
3. **Interactive Modals**:
   - Listen for `human.interaction.requested` events.
   - Present approval/question dialog to user.
   - Reply via `interaction.reply` with `{"decision": "allow", "actor": "human:agentdesk"}`.
4. **Account Switching**:
   - Query available accounts for a session via `account.query_availability`.
   - Issue `session.switch_account` with target account ID; the daemon automatically handles idle rebinding or controlled hand-off.

### For AgentMesh (Multi-Agent Fabric / Orchestrator)
1. **Connection**: Connect via WebSocket using `admin` or `write` scope.
2. **Project & Account Provisioning**:
   - Ensure target project is registered via `project.register`.
   - Register pools of authenticated accounts via `account.register` with tags (e.g. `["claude-mesh", "tier-1"]`).
3. **Session Orchestration**:
   - Create sessions bound to specific projects with automatic least-loaded account selection: `session.create_and_start`.
   - Steer running sessions dynamically using `session.steer`.
   - Inspect snapshot state without secret exposure using `session.snapshot`.
