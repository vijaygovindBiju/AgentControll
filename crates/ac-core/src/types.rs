//! Core domain types shared across all modules.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use ulid::Ulid;

// ── Identifiers ──────────────────────────────────────────────────────────────

/// Stable ULID-based identifier for any entity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(pub String);

impl Id {
    pub fn new() -> Self {
        Self(Ulid::new().to_string())
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<&str> for Id {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for Id {
    fn from(s: String) -> Self {
        Self(s)
    }
}

// ── Session states ────────────────────────────────────────────────────────────

/// All possible states of an `AgentSession`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Created but agent process not yet started.
    Idle,
    /// Adapter process launched; waiting for ready signal.
    Starting,
    /// Agent is actively processing.
    Working,
    /// Agent raised a question/approval; awaiting human or policy reply.
    WaitingForHuman,
    /// Suspended by human or policy; process alive but not consuming input.
    Paused,
    /// Account reported rate-limit; session blocked, not failed.
    RateLimited,
    /// Graceful shutdown in progress.
    Stopping,
    /// Terminated cleanly.
    Stopped,
    /// Process exited unexpectedly; eligible for automatic restart.
    Crashed,
    /// Restart in progress.
    Restarting,
    /// Task migrated to successor session; permanently done.
    HandedOff,
    /// Unrecoverable error; requires human intervention.
    Failed,
}

impl SessionState {
    /// Returns true when the state is terminal (no further transitions expected).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            SessionState::Stopped
                | SessionState::HandedOff
                | SessionState::Failed
        )
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .unwrap_or_else(|| format!("{:?}", self));
        write!(f, "{}", s)
    }
}

// ── AgentSession ──────────────────────────────────────────────────────────────

/// One running (or paused/stopped) agent session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: Id,
    pub task_description: String,
    pub agent_type: String,
    pub state: SessionState,
    pub restart_count: u32,
    /// Phase 2: bound project (None for sessions created without a project).
    pub project_id: Option<Id>,
    /// Phase 2: bound account (None for sessions created without account selection).
    pub account_id: Option<Id>,
    /// Phase 2: bound workspace (None for sessions created without a project).
    pub workspace_id: Option<Id>,
    /// Phase 6: Predecessor session ID (for hand-offs).
    #[serde(default)]
    pub predecessor_id: Option<Id>,
    /// Phase 6: Successor session ID (after hand-off).
    #[serde(default)]
    pub successor_id: Option<Id>,
    /// Phase 6: Context snapshot ID used or produced.
    #[serde(default)]
    pub context_snapshot_id: Option<Id>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
    /// Per-session launch options (permission mode, model, working directory).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<crate::agy_launch::AgyLaunchOptions>,
}

impl AgentSession {
    pub fn new(id: Id, task_description: String, agent_type: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            task_description,
            agent_type,
            state: SessionState::Idle,
            restart_count: 0,
            project_id: None,
            account_id: None,
            workspace_id: None,
            predecessor_id: None,
            successor_id: None,
            context_snapshot_id: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            stopped_at: None,
            launch: None,
        }
    }
}

// ── AgentEvent ────────────────────────────────────────────────────────────────

/// Kind of an agent event persisted in the event store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    // Session lifecycle
    SessionCreated,
    SessionStartRequested,
    SessionStarted,
    SessionReady,
    StateChanged,
    TransitionRejected,
    SessionPaused,
    SessionResumed,
    SessionStopRequested,
    SessionStopped,
    SessionCrashed,
    SessionRestartAttempted,
    SessionRestartSucceeded,
    SessionFailed,
    SessionHandedOff,
    SessionRemoved,
    // Agent I/O
    AgentOutputReceived,
    AgentQuestion,
    AgentApprovalRequested,
    AgentSteeringReceived,
    // Account events (Phase 2)
    AccountRegistered,
    AccountStateChanged,
    AccountCooldownStarted,
    AccountCooldownExpired,
    AccountExhausted,
    AccountRateLimited,
    AccountRateLimitCleared,
    AccountDisabled,
    AccountEnabled,
    AccountRemoved,
    // Project events (Phase 2)
    ProjectRegistered,
    ProjectUpdated,
    ProjectRemoved,
    // Workspace events (Phase 2)
    WorkspaceCreated,
    WorkspaceReclaimed,
    // Interaction events (Phase 3)
    InteractionCreated,
    InteractionAutoResolved,
    InteractionHumanResolved,
    InteractionExpired,
    InteractionDismissed,
    // Policy events (Phase 3)
    PolicyCreated,
    PolicyUpdated,
    PolicyRemoved,
    PolicyEvaluated,
    // Approval flow events (Phase 3)
    ApprovalRequested,
    ApprovalAutoApproved,
    ApprovalDenied,
    ApprovalHumanDecision,
    // Phase 6: Multi-account switching & hand-off events
    AccountSelected,
    AccountSwitchRequested,
    AccountSwitchStarted,
    AccountSwitchCompleted,
    AccountSwitchFailed,
    SessionSnapshotCreated,
    SessionHandOffStarted,
    SessionHandOffCompleted,
    SessionHandOffFailed,
    // System
    DaemonStarted,
    DaemonShutdownRequested,
    DaemonStopped,
    SnapshotCreated,
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .unwrap_or_else(|| format!("{:?}", self));
        write!(f, "{}", s)
    }
}

/// An event persisted in the append-only event store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    /// Monotonically increasing sequence number; assigned by the event store.
    pub seq: u64,
    /// Stable ULID identifier.
    pub id: Id,
    /// Schema version for migration on replay.
    pub version: u16,
    pub kind: EventKind,
    pub session_id: Option<Id>,
    /// Kind-specific data.
    pub payload: serde_json::Value,
    /// Who triggered this event: "human", "system", "adapter:<type>".
    pub triggered_by: String,
    pub timestamp: DateTime<Utc>,
}

impl AgentEvent {
    pub fn new(
        kind: EventKind,
        session_id: Option<Id>,
        payload: serde_json::Value,
        triggered_by: impl Into<String>,
    ) -> Self {
        Self {
            seq: 0, // assigned by the event store on append
            id: Id::new(),
            version: 1,
            kind,
            session_id,
            payload,
            triggered_by: triggered_by.into(),
            timestamp: Utc::now(),
        }
    }
}

// ── Control API wire types ────────────────────────────────────────────────────

/// A command sent over the Control API socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    /// Schema version.
    pub v: u16,
    /// Client-assigned request id (echoed in response).
    pub id: String,
    /// Command name, e.g. `"session.start"`.
    pub cmd: String,
    /// Command-specific parameters.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A response to a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub v: u16,
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl ApiResponse {
    pub fn ok(id: impl Into<String>, result: serde_json::Value) -> Self {
        Self {
            v: 1,
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: impl Into<String>, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            v: 1,
            id: id.into(),
            ok: false,
            result: None,
            error: Some(ApiError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

// ── Control API v1 Auth & Subscription types (Phase 7) ────────────────────────

/// Token authorization scope for WebSocket / external Control API consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenScope {
    /// Read-only access: queries, event subscriptions, status.
    Read,
    /// Write access: session creation, commands, approvals, account/project management.
    Write,
    /// Full administrator access.
    Admin,
}

impl TokenScope {
    /// Returns true if this scope permits mutating actions.
    pub fn can_write(&self) -> bool {
        matches!(self, TokenScope::Write | TokenScope::Admin)
    }
}

/// Advanced subscription filter for events.subscribe (Phase 7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionFilter {
    /// Event kinds to receive (if empty or None, receives all event kinds).
    #[serde(default)]
    pub kinds: Option<Vec<EventKind>>,
    /// Filter events specific to a session.
    #[serde(default)]
    pub session_id: Option<Id>,
    /// Filter events specific to a project.
    #[serde(default)]
    pub project_id: Option<Id>,
    /// Filter events specific to an account.
    #[serde(default)]
    pub account_id: Option<Id>,
    /// Catch-up replay: start replaying from this event sequence number.
    #[serde(default)]
    pub since_seq: Option<u64>,
}

impl SubscriptionFilter {
    /// Checks whether an event satisfies this subscription filter.
    pub fn matches(&self, event: &AgentEvent) -> bool {
        if let Some(kinds) = &self.kinds {
            if !kinds.is_empty() && !kinds.contains(&event.kind) {
                return false;
            }
        }
        if let Some(expected_sid) = &self.session_id {
            if event.session_id.as_ref() != Some(expected_sid) {
                return false;
            }
        }
        if let Some(expected_pid) = &self.project_id {
            let pid_in_payload = event.payload.get("project_id").and_then(|v| v.as_str());
            if pid_in_payload != Some(&expected_pid.0) {
                return false;
            }
        }
        if let Some(expected_aid) = &self.account_id {
            let aid_in_payload = event.payload.get("account_id").and_then(|v| v.as_str())
                .or_else(|| event.payload.get("target_account_id").and_then(|v| v.as_str()));
            if aid_in_payload != Some(&expected_aid.0) {
                return false;
            }
        }
        true
    }
}

// ── Adapter contract types ────────────────────────────────────────────────────

/// Commands sent from the core to an adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentCommand {
    Start {
        session_id: Id,
        task_description: String,
        agent_type: String,
    },
    Stop {
        reason: Option<String>,
    },
    Pause,
    Resume,
    Steer {
        message: String,
    },
    /// Direct raw input bytes/text to agent PTY stdin.
    Input {
        data: String,
    },
    /// Resize PTY terminal dimensions.
    Resize {
        rows: u16,
        cols: u16,
    },
    Snapshot,
    Respond {
        allow: bool,
        response: Option<String>,
    },
    /// Switch account in running adapter dynamically if supported.
    SwitchAccount {
        credential_ref: String,
    },
}

/// How an agent provider handles switching authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountSwitchMode {
    /// Adapter can dynamically change credentials in a running process without restarting.
    Dynamic,
    /// Adapter requires stopping the process, rotating account, and starting a successor session (hand-off).
    RequiresRestart,
    /// Provider does not support switching accounts.
    Unsupported,
}

impl fmt::Display for AccountSwitchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccountSwitchMode::Dynamic => write!(f, "dynamic"),
            AccountSwitchMode::RequiresRestart => write!(f, "requires_restart"),
            AccountSwitchMode::Unsupported => write!(f, "unsupported"),
        }
    }
}

/// Provider-neutral capability model for an agent adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    /// Mode of account switching supported by the adapter.
    pub switch_mode: AccountSwitchMode,
    /// Whether the adapter can resume an existing session context.
    pub supports_session_resume: bool,
    /// Whether the adapter can restore state from a SessionSnapshot.
    pub supports_snapshot_restore: bool,
    /// Whether changing accounts requires restarting the underlying agent process.
    pub requires_process_restart: bool,
}

impl ProviderCapabilities {
    pub fn new(
        switch_mode: AccountSwitchMode,
        supports_session_resume: bool,
        supports_snapshot_restore: bool,
    ) -> Self {
        let requires_process_restart = matches!(switch_mode, AccountSwitchMode::RequiresRestart);
        Self {
            switch_mode,
            supports_session_resume,
            supports_snapshot_restore,
            requires_process_restart,
        }
    }

    pub fn supports_account_switch(&self) -> bool {
        !matches!(self.switch_mode, AccountSwitchMode::Unsupported)
    }
}

pub const SNAPSHOT_SCHEMA_VERSION: u16 = 1;

/// Point-in-time capture of a session's state used for hand-off and recovery.
/// Strictly excludes credentials, environment secrets, tokens, or process memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSnapshot {
    /// Schema version for backward compatibility and format evolution.
    pub version: u16,
    /// Stable ULID snapshot identifier.
    pub id: Id,
    /// Identifier of the session that produced this snapshot.
    pub session_id: Id,
    /// Project identifier (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<Id>,
    /// Workspace identifier (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<Id>,
    /// Working directory path (if preserved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    /// Adapter / agent type string.
    pub agent_type: String,
    /// The original task description.
    pub task_description: String,
    /// Session state when the snapshot was captured.
    pub state: SessionState,
    /// Summary of work done up to this point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Safe excerpt of the interaction transcript.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_excerpt: Option<String>,
    /// Additional safe, non-secret metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Event store sequence number at snapshot time.
    pub event_seq: u64,
}

impl SessionSnapshot {
    pub fn new(
        session_id: Id,
        agent_type: String,
        task_description: String,
        state: SessionState,
        event_seq: u64,
    ) -> Self {
        Self {
            version: 1,
            id: Id::new(),
            session_id,
            project_id: None,
            workspace_id: None,
            workspace_path: None,
            agent_type,
            task_description,
            state,
            summary: None,
            transcript_excerpt: None,
            metadata: HashMap::new(),
            created_at: Utc::now(),
            event_seq,
        }
    }

    /// Formats a safe summary string suitable for passing into a successor session's context.
    pub fn to_context_string(&self) -> String {
        let mut out = format!(
            "Session Hand-Off Context [Snapshot v{}, predecessor={}]:\nTask: {}\n",
            self.version, self.session_id, self.task_description
        );
        if let Some(s) = &self.summary {
            out.push_str(&format!("Summary: {}\n", s));
        }
        if let Some(t) = &self.transcript_excerpt {
            out.push_str(&format!("Recent transcript:\n{}\n", t));
        }
        out
    }
}

/// Events sent from an adapter to the core.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdapterEvent {
    Ready,
    OutputChunk {
        text: String,
        confidence: Confidence,
    },
    ApprovalRequested {
        tool_name: String,
        prompt: String,
    },
    QuestionRaised {
        prompt: String,
    },
    RateLimitSignal {
        back_off_secs: Option<u64>,
    },
    Paused,
    Resumed,
    Completed {
        summary: Option<String>,
    },
    Crashed {
        exit_code: Option<i32>,
        reason: Option<String>,
    },
    SnapshotProduced {
        summary: String,
    },
    StartFailed {
        reason: String,
    },
}

/// Confidence level of an adapter event (structured vs PTY pattern match).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Low,
}

/// Context provided to an adapter when starting a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionContext {
    pub session_id: Id,
    pub task_description: String,
    pub agent_type: String,
    pub workspace_path: Option<String>,
    pub credential_ref: Option<String>,
    /// Account selected for this session (used for per-account agent profiles).
    #[serde(default)]
    pub account_id: Option<Id>,
    pub context_snapshot: Option<String>,
    pub agent_config: Option<serde_json::Value>,
}

impl SessionContext {
    pub fn new(session_id: Id, task_description: String, agent_type: String) -> Self {
        Self {
            session_id,
            task_description,
            agent_type,
            workspace_path: None,
            credential_ref: None,
            account_id: None,
            context_snapshot: None,
            agent_config: None,
        }
    }

    pub fn with_workspace(mut self, path: impl Into<String>) -> Self {
        self.workspace_path = Some(path.into());
        self
    }

    pub fn with_credential_ref(mut self, cred: impl Into<String>) -> Self {
        self.credential_ref = Some(cred.into());
        self
    }

    pub fn with_account(mut self, account_id: Id, cred: impl Into<String>) -> Self {
        self.account_id = Some(account_id);
        self.credential_ref = Some(cred.into());
        self
    }
}

// ── Phase 2: Account ──────────────────────────────────────────────────────────

/// All possible states of an `Account`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    /// Available for new session assignment.
    Active,
    /// Temporarily receiving rate-limit responses; timed back-off.
    RateLimited,
    /// Cooling down after exhaustion or repeated rate-limits; timed.
    Cooldown,
    /// Quota fully consumed; cannot accept new sessions.
    Exhausted,
    /// Credentials rejected; requires human intervention.
    Invalid,
    /// Manually taken out of rotation.
    Disabled,
}

impl AccountState {
    /// Returns true when the account can accept a new session assignment.
    pub fn is_available(&self) -> bool {
        matches!(self, AccountState::Active)
    }
}

impl fmt::Display for AccountState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .unwrap_or_else(|| format!("{:?}", self));
        write!(f, "{}", s)
    }
}

/// One set of provider credentials with its own quota, rate limits and
/// concurrency cap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Id,
    /// Human-readable name, e.g. `"anthropic-personal"`.
    pub label: String,
    /// Provider identifier, e.g. `"anthropic"`, `"openai"`.
    pub provider: String,
    /// Which agent adapter types may use this account.
    pub agent_types: Vec<String>,
    /// Reference into the credential store; never a raw secret.
    pub credential_ref: String,
    pub state: AccountState,
    /// Maximum simultaneous sessions on this account.
    pub concurrency_cap: u8,
    /// Current number of active (non-terminal) sessions on this account.
    pub active_session_count: u8,
    /// Set when state is Cooldown; null otherwise.
    pub cooldown_until: Option<DateTime<Utc>>,
    /// Grouping labels used for account selection strategies.
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Account {
    pub fn new(
        label: String,
        provider: String,
        agent_types: Vec<String>,
        credential_ref: String,
        concurrency_cap: u8,
        tags: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Id::new(),
            label,
            provider,
            agent_types,
            credential_ref,
            state: AccountState::Active,
            concurrency_cap,
            active_session_count: 0,
            cooldown_until: None,
            tags,
            notes: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Returns true when this account can be selected for a new session,
    /// considering both state and concurrency cap.
    pub fn can_accept_session(&self) -> bool {
        self.state.is_available() && self.active_session_count < self.concurrency_cap
    }

    /// Returns true when this account supports the given agent type.
    pub fn supports_agent_type(&self, agent_type: &str) -> bool {
        self.agent_types.is_empty() || self.agent_types.iter().any(|t| t == agent_type)
    }
}

/// Account availability status descriptor for querying, API, and UI display.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountAvailability {
    pub account_id: Id,
    pub label: String,
    pub provider: String,
    pub agent_types: Vec<String>,
    pub state: AccountState,
    pub active_session_count: u8,
    pub concurrency_cap: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_until: Option<DateTime<Utc>>,
    pub is_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ── Phase 2: Project ──────────────────────────────────────────────────────────

/// Policy for how workspaces are created within a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePolicy {
    /// All sessions share the project's main checkout.
    #[default]
    Shared,
    /// A git worktree on a fresh branch is created per session.
    WorktreePerSession,
}

impl fmt::Display for WorkspacePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspacePolicy::Shared => write!(f, "shared"),
            WorkspacePolicy::WorktreePerSession => write!(f, "worktree_per_session"),
        }
    }
}

/// A registered repository with defaults and policies for agent sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Id,
    /// Human-readable name, e.g. `"agent-control"`.
    pub name: String,
    /// Absolute path to the repository root on disk.
    pub repo_path: String,
    /// Default agent type for sessions in this project.
    pub default_agent_type: Option<String>,
    /// Account tag filter for session assignment.
    pub default_account_tags: Vec<String>,
    pub workspace_policy: WorkspacePolicy,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    pub fn new(
        name: String,
        repo_path: String,
        default_agent_type: Option<String>,
        default_account_tags: Vec<String>,
        workspace_policy: WorkspacePolicy,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Id::new(),
            name,
            repo_path,
            default_agent_type,
            default_account_tags,
            workspace_policy,
            notes: None,
            created_at: now,
            updated_at: now,
        }
    }
}

// ── Phase 2: Workspace ────────────────────────────────────────────────────────

/// How a workspace was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// Shared project checkout.
    Shared,
    /// Dedicated git worktree for one session.
    Worktree,
}

/// A specific on-disk working directory bound to a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Id,
    pub project_id: Id,
    pub kind: WorkspaceKind,
    /// Absolute path on disk.
    pub path: String,
    /// Git branch (for Worktree kind).
    pub branch: Option<String>,
    /// Currently bound session; None when idle.
    pub session_id: Option<Id>,
    pub created_at: DateTime<Utc>,
    pub reclaimed_at: Option<DateTime<Utc>>,
}

// ── Phase 3: Interaction ──────────────────────────────────────────────────────

/// Kind of interaction between agent and human.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    /// Agent is asking for information/clarification.
    Question,
    /// Agent requesting approval to execute a tool/action.
    ApprovalRequest,
}

impl fmt::Display for InteractionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InteractionKind::Question => write!(f, "question"),
            InteractionKind::ApprovalRequest => write!(f, "approval_request"),
        }
    }
}

/// State of an interaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionState {
    /// Awaiting policy evaluation or human input.
    Pending,
    /// Resolved by policy without human involvement.
    AutoResolved,
    /// Resolved by explicit human input.
    HumanResolved,
    /// Dismissed by human without a decision.
    Dismissed,
    /// Session ended before interaction was resolved.
    Expired,
}

impl fmt::Display for InteractionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .unwrap_or_else(|| format!("{:?}", self));
        write!(f, "{}", s)
    }
}

/// Policy decision result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    /// Action is allowed (auto-approved).
    Allow,
    /// Action is denied.
    Deny,
    /// Requires human decision.
    RequireHuman,
}

impl fmt::Display for PolicyDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_owned()))
            .unwrap_or_else(|| format!("{:?}", self));
        write!(f, "{}", s)
    }
}

/// One interaction between an agent and a human.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    pub id: Id,
    pub session_id: Id,
    pub kind: InteractionKind,
    pub state: InteractionState,
    /// The prompt or question from the agent.
    pub prompt: String,
    /// Tool name (for approval requests).
    pub tool_name: Option<String>,
    /// Tool arguments (for approval requests); redacted before storage.
    pub tool_args: Option<serde_json::Value>,
    /// Policy that resolved this interaction (if auto-resolved).
    pub policy_id: Option<Id>,
    /// Decision made.
    pub decision: Option<PolicyDecision>,
    /// Human text response (for questions).
    pub response: Option<String>,
    /// Who resolved: "policy:<id>" or "human:<user>".
    pub resolved_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl Interaction {
    pub fn new_approval_request(
        session_id: Id,
        tool_name: String,
        tool_args: Option<serde_json::Value>,
        prompt: String,
    ) -> Self {
        Self {
            id: Id::new(),
            session_id,
            kind: InteractionKind::ApprovalRequest,
            state: InteractionState::Pending,
            prompt,
            tool_name: Some(tool_name),
            tool_args,
            policy_id: None,
            decision: None,
            response: None,
            resolved_by: None,
            created_at: Utc::now(),
            resolved_at: None,
        }
    }

    pub fn new_question(session_id: Id, prompt: String) -> Self {
        Self {
            id: Id::new(),
            session_id,
            kind: InteractionKind::Question,
            state: InteractionState::Pending,
            prompt,
            tool_name: None,
            tool_args: None,
            policy_id: None,
            decision: None,
            response: None,
            resolved_by: None,
            created_at: Utc::now(),
            resolved_at: None,
        }
    }

    pub fn is_pending(&self) -> bool {
        self.state == InteractionState::Pending
    }
}

// ── Phase 3: Policy ──────────────────────────────────────────────────────────

/// Scope at which a policy applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyScope {
    /// Applies to all sessions.
    Global,
    /// Applies to sessions in a specific project.
    Project(Id),
}

impl fmt::Display for PolicyScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PolicyScope::Global => write!(f, "global"),
            PolicyScope::Project(id) => write!(f, "project:{}", id),
        }
    }
}

/// A condition that must match for a policy rule to apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyCondition {
    /// Matches a specific tool name (exact match).
    ToolNameEquals(String),
    /// Matches tool names starting with a prefix.
    ToolNamePrefix(String),
    /// Matches a specific agent type.
    AgentTypeEquals(String),
    /// Matches any tool name in a list.
    ToolNameIn(Vec<String>),
}

impl PolicyCondition {
    /// Evaluate whether this condition matches the given context.
    pub fn matches(&self, tool_name: Option<&str>, agent_type: Option<&str>) -> bool {
        match self {
            PolicyCondition::ToolNameEquals(name) => {
                tool_name.map(|t| t == name).unwrap_or(false)
            }
            PolicyCondition::ToolNamePrefix(prefix) => {
                tool_name.map(|t| t.starts_with(prefix)).unwrap_or(false)
            }
            PolicyCondition::AgentTypeEquals(at) => {
                agent_type.map(|a| a == at).unwrap_or(false)
            }
            PolicyCondition::ToolNameIn(names) => {
                tool_name.map(|t| names.iter().any(|n| n == t)).unwrap_or(false)
            }
        }
    }
}

/// A declarative policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: Id,
    /// Human-readable name.
    pub name: String,
    /// Scope: global or project-specific.
    pub scope: PolicyScope,
    /// Higher priority is evaluated first.
    pub priority: i32,
    /// All conditions must match (conjunction).
    pub conditions: Vec<PolicyCondition>,
    /// Outcome if all conditions match.
    pub decision: PolicyDecision,
    /// Whether this policy is active.
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Policy {
    pub fn new(
        name: String,
        scope: PolicyScope,
        priority: i32,
        conditions: Vec<PolicyCondition>,
        decision: PolicyDecision,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Id::new(),
            name,
            scope,
            priority,
            conditions,
            decision,
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }
}

// ── Phase 3: Audit Entry ──────────────────────────────────────────────────────

/// Immutable audit record for policy decisions and human actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Id,
    /// Related event sequence number.
    pub event_seq: Option<u64>,
    /// Related interaction.
    pub interaction_id: Option<Id>,
    /// Related session.
    pub session_id: Option<Id>,
    /// Who made the decision: "policy:<id>" or "human:<user>" or "system".
    pub actor: String,
    /// The action taken: "allow", "deny", "require_human", "dismiss".
    pub action: String,
    /// Human note or policy name/rule summary.
    pub rationale: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl AuditEntry {
    pub fn new(
        interaction_id: Option<Id>,
        session_id: Option<Id>,
        actor: String,
        action: String,
        rationale: Option<String>,
    ) -> Self {
        Self {
            id: Id::new(),
            event_seq: None,
            interaction_id,
            session_id,
            actor,
            action,
            rationale,
            timestamp: Utc::now(),
        }
    }
}
