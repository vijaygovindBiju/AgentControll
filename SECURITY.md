# Security Policy

Agent Control takes the security and privacy of user credentials, codebases, and local execution environments seriously. This document describes our security policy, vulnerability reporting procedures, credential isolation model, and bug reporting guidance.

---

## Supported Versions

Security updates are actively maintained for the following versions:

| Version | Supported          |
|---------|--------------------|
| 1.0.x   | :white_check_mark: |
| < 1.0.0 | :x:                |

---

## Reporting a Security Vulnerability

If you discover a security vulnerability or potential exploit in Agent Control:

1. **Do NOT open a public GitHub issue.** Public issues disclose vulnerabilities before patches can be developed and distributed.
2. **Report Privately via GitHub Security Advisories:**
   - Navigate to the **Security** tab of the repository on GitHub.
   - Click **Report a vulnerability** to open a private disclosure thread.
3. **Alternative Contact:** If GitHub Security Advisories are unavailable, email `security@agentcontrol.dev` (or the repository maintainers) with:
   - Description of the vulnerability.
   - Minimal reproduction steps or proof-of-concept.
   - Affected components, binaries, or operating system environments.
4. **Response Timeline:** Maintainers will acknowledge receipt within **48 hours** and provide periodic status updates on remediation and coordinated release schedules.

---

## Security & Permission Model

Agent Control operates as a local supervisor daemon managing third-party coding agents. It enforces multiple defense-in-depth boundaries:

### 1. Local-First Isolation
- **IPC Domain Socket:** Bound to `$XDG_RUNTIME_DIR/agentcontrol/agentcontrol.sock` or `/tmp/agentcontrol-<uid>.sock` with strict filesystem permissions (`0600`), owned by the current user. Other local users cannot connect to or read from the socket.
- **WebSocket Loopback (`ws://127.0.0.1:4242`):** Bound strictly to `127.0.0.1` (loopback interface only). It is never bound to `0.0.0.0` or external network adapters.
- **WebSocket Permission Scopes:** Incoming WebSocket connections are authenticated via tokens with granular capability scopes:
  - `read`: Read-only queries, status telemetry, and event subscription. Mutating commands are rejected with structured `PermissionDenied` errors.
  - `write`: Session lifecycle execution, interaction replies, and steering.
  - `admin`: Account registration, removal, and policy modification.

### 2. Credential Storage & Isolation
- **Zero Raw Secrets in Databases:** SQLite event store tables, audit logs, and session snapshots store only opaque credential references (e.g. `ref:antigravity:<id>`), never plaintext tokens or passwords.
- **Protected File Storage:** Credential files are stored under `~/.config/agentcontrol/credentials/` with strict `0600` permissions (directory `0700`).
- **Profile Redirection:** Antigravity runs with `HOME` set to an isolated profile directory (`~/.config/agentcontrol/profiles/<account-id>/`). Default machine credentials under `~/.gemini` are never used as a fallback.
- **Environment Stripping:** When spawning agent processes, ambient credential environment variables (`GEMINI_API_KEY`, `GOOGLE_API_KEY`, `GOOGLE_APPLICATION_CREDENTIALS`, etc.) are explicitly stripped from the process environment.
- **In-Memory Transport:** Credentials are held in process memory only for the duration needed to spawn the child process or perform token exchange.

### 3. Never-Auto-Approve Boundary
- High-risk operations (`sudo`, `rm`, `curl`, `wget`, `git push --force`, `publish`) cannot be auto-approved by declarative user policies.
- Even if a user defines an `Allow` rule matching these commands, the Policy Engine unconditionally overrides it and escalates the request to a human operator (`RequireHuman`).

### 4. Redaction Engine
- All event payloads pass through an automated redaction filter before being written to SQLite or published over the event stream. Common token formats (bearer tokens, Google OAuth codes, API keys) are replaced with `[REDACTED]`.

---

## What NOT to Include in Bug Reports

When opening bug reports or submitting diagnostic logs, ensure you protect your privacy:

> [!CAUTION]
> **Never include the following in GitHub issues or public forums:**
> - Contents of `~/.config/agentcontrol/credentials/` or `~/.gemini/`
> - Full redirect URLs containing `code=...` query parameters
> - OAuth client secret files (`antigravity-oauth-client.json`)
> - Proprietary source code or confidential repository paths
> - Unredacted process environment variables (`env` or `export`)

Agent Control error messages and diagnostic outputs are designed to sanitize tokens and output byte counts or account labels instead of secrets. If you notice any secret leakage in an error message or log line, report it immediately as a vulnerability.
