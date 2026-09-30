# Agent Control — Roadmap

This roadmap documents future initiatives, planned enhancements, and architectural extensions scheduled for post-v1.0.0 releases.

---

## Short-Term Roadmap (v1.1.0)

### 1. Reusable Session Templates & Presets
- Pre-configured profiles defining target agent (`agy`, `claude`), account tags, permission policies, and default system instructions.
- Launch standardized sessions with a single command or hotkey.

### 2. Claude Code Multi-Account Isolation
- Extend per-account home directory isolation (`~/.config/agentcontrol/profiles/<account-id>/`) from Antigravity to Claude Code.
- Manage multiple Anthropic API keys and credential profiles concurrently without environment bleed.

### 3. Native Metrics & OpenTelemetry Export
- Expose Prometheus `/metrics` endpoint or OTLP stream over the local daemon.
- Track session durations, crash frequencies, interaction approval rates, and account quota exhaustion.

---

## Medium-Term Roadmap (v1.2.0)

### 4. Kernel-Enforced Filesystem Sandboxing (Linux Landlock / bubblewrap)
- Enforce strict filesystem boundaries ensuring agent child processes cannot access paths outside their designated working directories or account profiles.
- Complement existing policy engine checks with kernel-level security isolation.

### 5. Webhook & External Notification Triggers
- Configurable webhooks to push critical events (`SessionCrashed`, `HumanInterventionRequired`, `QuotaExhausted`) to Discord, Slack, or desktop notifications.

### 6. Dynamic Adapter Plugins
- Allow third-party agent adapters to be registered as external executables communicating over stdin/stdout JSON-RPC, without requiring recompilation of `agentcontrold`.

---

## Long-Term Vision (v2.0.0)

### 7. Remote Agent Execution & Multi-Node Clusters
- Manage agent sessions running across cloud VMs, build servers, or local dev containers from a single local TUI.
- Encrypted mTLS tunnel between the local CLI/TUI and remote `agentcontrold` instances.

### 8. Web Console Companion
- Lightweight localhost web interface for monitoring complex multi-agent graphs and real-time diff inspections in browser tabs.

---

## Proposed Capabilities Matrix

| Initiative | Target Milestone | Complexity | Tracking Reference |
|:---|:---|:---|:---|
| Reusable Session Presets | v1.1.0 | Low | F10 |
| Claude Multi-Account Sandbox | v1.1.0 | Medium | ADR-014 |
| OpenTelemetry Metrics Exporter | v1.1.0 | Medium | F12 |
| Linux Landlock Sandboxing | v1.2.0 | High | F4 |
| Webhook Alert Dispatcher | v1.2.0 | Low | F8 |
| Subprocess Adapter Plugins | v1.2.0 | High | F5 / ADR-008 |
| Remote Cluster Execution | v2.0.0 | High | F1 |
| Web Companion Dashboard | v2.0.0 | Medium | F3 |
