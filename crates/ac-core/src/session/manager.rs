//! Session Manager — creates, supervises and transitions `AgentSession`s.
//!
//! The manager is the only component that calls `state_machine::apply`.
//! It never changes state directly; it always emits the resulting event.

use anyhow::{bail, Result};
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};

use crate::{
    account_manager::{AccountManager, AccountManagerHandle},
    adapter::AdapterHandle,
    event_store::EventStore,
    interaction_hub::{InteractionHubHandle, InteractionSubmissionResult},
    project_registry::{ProjectRegistry, ProjectRegistryHandle},
    session::state_machine,
    types::{
        AccountSwitchMode, AdapterEvent, AgentCommand, AgentEvent, AgentSession, EventKind, Id,
        Interaction, PolicyDecision, SessionSnapshot, SessionState,
    },
};

/// Commands the session manager processes from the API.
#[derive(Debug)]
pub enum SessionCmd {
    Create {
        task_description: String,
        agent_type: String,
        /// Phase 2: optional project to bind this session to.
        project_id: Option<Id>,
        /// Phase 2: explicit account override; if None, auto-select.
        account_id: Option<Id>,
        /// Per-session launch options (permission mode, model, working directory).
        launch: Option<crate::agy_launch::AgyLaunchOptions>,
        reply: tokio::sync::oneshot::Sender<Result<Id>>,
    },
    Start {
        session_id: Id,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    Pause {
        session_id: Id,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    Resume {
        session_id: Id,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    Stop {
        session_id: Id,
        reason: Option<String>,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Remove/delete a session permanently.
    Remove {
        session_id: Id,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    List {
        reply: tokio::sync::oneshot::Sender<Vec<AgentSession>>,
    },
    Get {
        session_id: Id,
        reply: tokio::sync::oneshot::Sender<Option<AgentSession>>,
    },
    /// Phase 3: Human replies to or resolves an interaction.
    RespondInteraction {
        interaction_id: Id,
        decision: Option<PolicyDecision>,
        response: Option<String>,
        actor: Option<String>,
        reply: tokio::sync::oneshot::Sender<Result<Interaction>>,
    },
    /// Phase 3: Human dismisses an interaction without a decision.
    DismissInteraction {
        interaction_id: Id,
        actor: Option<String>,
        reply: tokio::sync::oneshot::Sender<Result<Interaction>>,
    },
    /// Inject a human steering instruction into a session.
    Steer {
        session_id: Id,
        message: String,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Direct PTY input sent to session.
    Input {
        session_id: Id,
        data: String,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Resize PTY terminal dimensions.
    Resize {
        session_id: Id,
        rows: u16,
        cols: u16,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Query historical events from the event store.
    QueryEvents {
        session_id: Option<Id>,
        limit: Option<u64>,
        reply: tokio::sync::oneshot::Sender<Result<Vec<AgentEvent>>>,
    },
    /// Query historical events starting from a specific sequence number (Phase 7 replay).
    QueryEventsSince {
        since_seq: u64,
        session_id: Option<Id>,
        limit: Option<u64>,
        reply: tokio::sync::oneshot::Sender<Result<Vec<AgentEvent>>>,
    },
    /// Phase 6: Explicitly select an account for an Idle session.
    SelectAccount {
        session_id: Id,
        account_id: Id,
        reply: tokio::sync::oneshot::Sender<Result<()>>,
    },
    /// Phase 6: Switch account for a session (Idle rebind or running hand-off/dynamic).
    SwitchAccount {
        session_id: Id,
        target_account_id: Id,
        reply: tokio::sync::oneshot::Sender<Result<Id>>,
    },
    /// Phase 6: Capture a session snapshot.
    Snapshot {
        session_id: Id,
        reply: tokio::sync::oneshot::Sender<Result<SessionSnapshot>>,
    },
    /// Phase 6: Controlled hand-off to another account.
    HandOff {
        session_id: Id,
        target_account_id: Option<Id>,
        reply: tokio::sync::oneshot::Sender<Result<Id>>,
    },
}

/// A running adapter task.
struct LiveAdapter {
    handle: AdapterHandle,
}

/// The session manager — owns all session state and the event store.
pub struct SessionManager {
    sessions: HashMap<String, AgentSession>,
    adapters: HashMap<String, LiveAdapter>,
    store: EventStore,
    cmd_rx: mpsc::Receiver<SessionCmd>,
    adapter_event_tx: mpsc::Sender<(Id, AdapterEvent)>,
    adapter_event_rx: mpsc::Receiver<(Id, AdapterEvent)>,
    /// Broadcast channel for publishing events to the API / TUI.
    event_tx: tokio::sync::broadcast::Sender<AgentEvent>,
    #[allow(dead_code)]
    max_restarts: u32,
    /// Factory for creating adapters (pluggable for tests).
    adapter_factory: Box<dyn crate::adapter::AdapterFactory + Send>,
    /// Phase 2: optional account manager handle (None when running without Phase 2).
    account_mgr: Option<AccountManagerHandle>,
    /// Phase 2: optional project registry handle (None when running without Phase 2).
    project_registry: Option<ProjectRegistryHandle>,
    /// Phase 3: optional interaction hub.
    interaction_hub: Option<InteractionHubHandle>,
}

impl SessionManager {
    pub fn new(
        store: EventStore,
        cmd_rx: mpsc::Receiver<SessionCmd>,
        event_tx: tokio::sync::broadcast::Sender<AgentEvent>,
        max_restarts: u32,
        adapter_factory: Box<dyn crate::adapter::AdapterFactory + Send>,
    ) -> Self {
        let (adapter_event_tx, adapter_event_rx) = mpsc::channel(256);
        Self {
            sessions: HashMap::new(),
            adapters: HashMap::new(),
            store,
            cmd_rx,
            adapter_event_tx,
            adapter_event_rx,
            event_tx,
            max_restarts,
            adapter_factory,
            account_mgr: None,
            project_registry: None,
            interaction_hub: None,
        }
    }

    /// Attach a Phase 2 Account Manager.
    pub fn with_account_manager(mut self, mgr: AccountManager) -> Self {
        self.account_mgr = Some(AccountManagerHandle::new(mgr));
        self
    }

    /// Attach a Phase 2 Account Manager handle (shared with IPC server).
    pub fn with_account_manager_handle(mut self, handle: AccountManagerHandle) -> Self {
        self.account_mgr = Some(handle);
        self
    }

    /// Attach a Phase 2 Project Registry.
    pub fn with_project_registry(mut self, reg: ProjectRegistry) -> Self {
        self.project_registry = Some(ProjectRegistryHandle::new(reg));
        self
    }

    /// Attach a Phase 2 Project Registry handle (shared with IPC server).
    pub fn with_project_registry_handle(mut self, handle: ProjectRegistryHandle) -> Self {
        self.project_registry = Some(handle);
        self
    }

    /// Attach a Phase 3 Interaction Hub.
    pub fn with_interaction_hub(mut self, hub: InteractionHubHandle) -> Self {
        self.interaction_hub = Some(hub);
        self
    }

    /// Replay events from the store to rebuild in-memory state (recovery).
    pub fn recover_from_store(&mut self) -> Result<()> {
        let events = self.store.query(0, None, None)?;
        info!("Recovery: replaying {} events from store", events.len());
        for event in &events {
            self.apply_event_to_state(event);
        }
        // Sessions that were active at shutdown have no live adapter → mark Crashed.
        let orphaned: Vec<Id> = self
            .sessions
            .values()
            .filter(|s| {
                matches!(
                    s.state,
                    SessionState::Starting
                        | SessionState::Working
                        | SessionState::WaitingForHuman
                        | SessionState::Paused
                        | SessionState::RateLimited
                        | SessionState::Stopping
                        | SessionState::Restarting
                )
            })
            .map(|s| s.id.clone())
            .collect();

        for sid in orphaned {
            warn!(
                "Recovery: session {} was orphaned at shutdown → Crashed",
                sid
            );
            self.do_transition(&sid, SessionState::Crashed, "system")?;
        }

        self.reconcile_account_loads();
        info!(
            "Recovery complete. {} sessions loaded.",
            self.sessions.len()
        );
        Ok(())
    }

    /// Reconcile account active session counts with live non-terminal sessions.
    pub fn reconcile_account_loads(&self) {
        if let Some(h) = &self.account_mgr {
            let mut counts: HashMap<Id, u8> = HashMap::new();
            for s in self.sessions.values() {
                if !s.state.is_terminal() {
                    if let Some(aid) = &s.account_id {
                        let entry = counts.entry(aid.clone()).or_insert(0);
                        *entry = entry.saturating_add(1);
                    }
                }
            }
            let _ = h.0.lock().unwrap().reconcile_active_counts(&counts);
        }
    }

    /// Return references to all in-memory sessions (for inspection & testing).
    pub fn sessions(&self) -> Vec<&AgentSession> {
        self.sessions.values().collect()
    }

    /// Apply a single event to in-memory state (used during recovery replay).
    fn apply_event_to_state(&mut self, event: &AgentEvent) {
        match &event.kind {
            EventKind::SessionCreated => {
                let id = event.session_id.clone().unwrap_or_else(Id::new);
                let task = event.payload["task_description"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                let agent_type = event.payload["agent_type"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned();
                let mut session = AgentSession::new(id.clone(), task, agent_type);
                // Restore Phase 2 & 6 fields if present in payload
                session.project_id = event.payload["project_id"].as_str().map(Id::from);
                session.account_id = event.payload["account_id"].as_str().map(Id::from);
                session.workspace_id = event.payload["workspace_id"].as_str().map(Id::from);
                session.predecessor_id = event.payload["predecessor_id"].as_str().map(Id::from);
                session.successor_id = event.payload["successor_id"].as_str().map(Id::from);
                session.context_snapshot_id =
                    event.payload["context_snapshot_id"].as_str().map(Id::from);
                session.launch = serde_json::from_value(event.payload["launch"].clone()).ok();
                self.sessions.insert(id.0, session);
            }
            EventKind::StateChanged => {
                if let Some(sid) = &event.session_id {
                    if let Some(to_str) = event.payload["to"].as_str() {
                        if let Ok(to) = serde_json::from_value::<SessionState>(
                            serde_json::Value::String(to_str.to_owned()),
                        ) {
                            if let Some(session) = self.sessions.get_mut(&sid.0) {
                                state_machine::apply(session, to);
                            }
                        }
                    }
                }
            }
            EventKind::AccountSelected => {
                if let Some(sid) = &event.session_id {
                    if let Some(aid) = event.payload["account_id"].as_str() {
                        if let Some(session) = self.sessions.get_mut(&sid.0) {
                            session.account_id = Some(Id::from(aid));
                        }
                    }
                }
            }
            EventKind::SessionHandedOff => {
                if let Some(sid) = &event.session_id {
                    if let Some(succ) = event.payload["successor_id"].as_str() {
                        if let Some(session) = self.sessions.get_mut(&sid.0) {
                            session.successor_id = Some(Id::from(succ));
                        }
                    }
                }
            }
            EventKind::SessionRemoved => {
                if let Some(sid) = &event.session_id {
                    self.sessions.remove(&sid.0);
                }
            }
            _ => {} // Other events don't affect in-memory session state
        }
    }

    /// Main run loop — processes commands and adapter events.
    pub async fn run(mut self) {
        info!("Session manager started");
        loop {
            tokio::select! {
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(cmd) => self.handle_cmd(cmd).await,
                        None => {
                            info!("Session manager command channel closed, shutting down");
                            break;
                        }
                    }
                }
                Some((session_id, event)) = self.adapter_event_rx.recv() => {
                    self.handle_adapter_event(session_id, event).await;
                }
            }
        }
    }

    async fn handle_cmd(&mut self, cmd: SessionCmd) {
        match cmd {
            SessionCmd::Create {
                task_description,
                agent_type,
                project_id,
                account_id,
                launch,
                reply,
            } => {
                let result =
                    self.cmd_create(task_description, agent_type, project_id, account_id, launch);
                let _ = reply.send(result);
            }
            SessionCmd::Start { session_id, reply } => {
                let result = self.cmd_start(session_id).await;
                let _ = reply.send(result);
            }
            SessionCmd::Pause { session_id, reply } => {
                let result = self.cmd_pause(session_id);
                let _ = reply.send(result);
            }
            SessionCmd::Resume { session_id, reply } => {
                let result = self.cmd_resume(session_id);
                let _ = reply.send(result);
            }
            SessionCmd::Stop {
                session_id,
                reason,
                reply,
            } => {
                let result = self.cmd_stop(session_id, reason).await;
                let _ = reply.send(result);
            }
            SessionCmd::Remove { session_id, reply } => {
                let result = self.cmd_remove(session_id).await;
                let _ = reply.send(result);
            }
            SessionCmd::List { reply } => {
                let sessions: Vec<AgentSession> = self.sessions.values().cloned().collect();
                let _ = reply.send(sessions);
            }
            SessionCmd::Get { session_id, reply } => {
                let session = self.sessions.get(&session_id.0).cloned();
                let _ = reply.send(session);
            }
            SessionCmd::RespondInteraction {
                interaction_id,
                decision,
                response,
                actor,
                reply,
            } => {
                let result = self
                    .cmd_respond_interaction(interaction_id, decision, response, actor)
                    .await;
                let _ = reply.send(result);
            }
            SessionCmd::DismissInteraction {
                interaction_id,
                actor,
                reply,
            } => {
                let result = self.cmd_dismiss_interaction(interaction_id, actor).await;
                let _ = reply.send(result);
            }
            SessionCmd::Steer {
                session_id,
                message,
                reply,
            } => {
                let result = self.cmd_steer(session_id, message);
                let _ = reply.send(result);
            }
            SessionCmd::Input {
                session_id,
                data,
                reply,
            } => {
                let result = self.cmd_input(session_id, data);
                let _ = reply.send(result);
            }
            SessionCmd::Resize {
                session_id,
                rows,
                cols,
                reply,
            } => {
                let result = self.cmd_resize(session_id, rows, cols);
                let _ = reply.send(result);
            }
            SessionCmd::QueryEvents {
                session_id,
                limit,
                reply,
            } => {
                let result = self.cmd_query_events(session_id, limit);
                let _ = reply.send(result);
            }
            SessionCmd::QueryEventsSince {
                since_seq,
                session_id,
                limit,
                reply,
            } => {
                let result = self.store.query(since_seq, session_id.as_ref(), limit);
                let _ = reply.send(result);
            }
            SessionCmd::SelectAccount {
                session_id,
                account_id,
                reply,
            } => {
                let result = self.cmd_select_account(session_id, account_id);
                let _ = reply.send(result);
            }
            SessionCmd::SwitchAccount {
                session_id,
                target_account_id,
                reply,
            } => {
                let result = self.cmd_switch_account(session_id, target_account_id).await;
                let _ = reply.send(result);
            }
            SessionCmd::Snapshot { session_id, reply } => {
                let result = self.cmd_snapshot(session_id);
                let _ = reply.send(result);
            }
            SessionCmd::HandOff {
                session_id,
                target_account_id,
                reply,
            } => {
                let result = self.cmd_handoff(session_id, target_account_id).await;
                let _ = reply.send(result);
            }
        }
    }

    // ── Command implementations ────────────────────────────────────────────

    fn cmd_create(
        &mut self,
        task_description: String,
        agent_type: String,
        project_id: Option<Id>,
        explicit_account_id: Option<Id>,
        launch: Option<crate::agy_launch::AgyLaunchOptions>,
    ) -> Result<Id> {
        if let Some(l) = &launch {
            l.validate()
                .map_err(|e| anyhow::anyhow!("ValidationError: {e}"))?;
        }
        // ── Phase 2: resolve account ──────────────────────────────────────
        let resolved_account_id: Option<Id> = if let Some(aid) = explicit_account_id {
            // Explicit override: validate it exists, supports agent_type, and is available
            let actual_id = if let Some(h) = &self.account_mgr {
                let mgr = h.0.lock().unwrap();
                let acct = mgr.validate_and_select_explicit(&aid, &agent_type)?;
                tracing::info!(
                    "Session creation: explicit account selected: label={}, id={}",
                    acct.label,
                    acct.id.0
                );
                acct.id.clone()
            } else {
                tracing::info!("Session creation: explicit account (no account mgr): {}", aid.0);
                aid.clone()
            };
            Some(actual_id)
        } else if let Some(h) = &self.account_mgr {
            // Auto-select: use project's default tags if a project is specified
            let required_tags: Vec<String> = if let Some(pid) = &project_id {
                if let Some(reg_h) = &self.project_registry {
                    reg_h
                        .0
                        .lock()
                        .unwrap()
                        .get(pid)
                        .map(|p| p.default_account_tags.clone())
                        .unwrap_or_default()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };
            // Expire cooldowns before selection (best-effort)
            let selected_id = h.0.lock().unwrap().select(&agent_type, &required_tags)?;
            tracing::info!(
                "Session creation: auto-selected account id: {}",
                selected_id.0
            );
            Some(selected_id)
        } else {
            tracing::warn!("Session creation: no account manager, account_id will be None");
            None
        };

        // ── Phase 2: resolve workspace ────────────────────────────────────
        let id = Id::new();
        let resolved_workspace_id: Option<Id> = if let Some(pid) = &project_id {
            if let Some(reg_h) = &self.project_registry {
                let mut reg = reg_h.0.lock().unwrap();
                if reg.get(pid).is_none() {
                    bail!("Project not found: {}", pid);
                }
                Some(reg.resolve_workspace(pid, &id)?)
            } else {
                None
            }
        } else {
            None
        };

        // ── Build session ─────────────────────────────────────────────────
        let mut session =
            AgentSession::new(id.clone(), task_description.clone(), agent_type.clone());
        session.project_id = project_id.clone();
        session.account_id = resolved_account_id.clone();
        session.workspace_id = resolved_workspace_id.clone();
        session.launch = launch.clone();
        self.sessions.insert(id.0.clone(), session);

        // ── Increment account load ────────────────────────────────────────
        if let Some(aid) = &resolved_account_id {
            if let Some(h) = &self.account_mgr {
                let _ = h.0.lock().unwrap().increment_sessions(aid);
            }
        }

        self.emit(AgentEvent::new(
            EventKind::SessionCreated,
            Some(id.clone()),
            json!({
                "task_description": task_description,
                "agent_type": agent_type,
                "project_id": project_id,
                "account_id": resolved_account_id,
                "workspace_id": resolved_workspace_id,
                "launch": launch,
            }),
            "human",
        ))?;

        if let Some(aid) = &resolved_account_id {
            self.emit(AgentEvent::new(
                EventKind::AccountSelected,
                Some(id.clone()),
                json!({
                    "session_id": id,
                    "account_id": aid,
                    "previous_account_id": serde_json::Value::Null,
                }),
                "human",
            ))?;
        }

        info!(
            "Session created: {} (project={:?}, account={:?})",
            id, project_id, resolved_account_id
        );
        Ok(id)
    }

    async fn cmd_start(&mut self, session_id: Id) -> Result<()> {
        {
            let session = self.get_session_or_err(&session_id)?;
            if session.state != SessionState::Idle {
                bail!(
                    "Session {} is not Idle (state={})",
                    session_id,
                    session.state
                );
            }
        }

        self.emit(AgentEvent::new(
            EventKind::SessionStartRequested,
            Some(session_id.clone()),
            json!({}),
            "human",
        ))?;

        self.do_transition(&session_id, SessionState::Starting, "human")?;

        // Spawn the adapter
        let session = self.sessions.get(&session_id.0).unwrap();
        let task = session.task_description.clone();
        let agent_type = session.agent_type.clone();
        let workspace_path = if let Some(wid) = &session.workspace_id {
            if let Some(reg_h) = &self.project_registry {
                reg_h
                    .0
                    .lock()
                    .unwrap()
                    .get_workspace(wid)
                    .map(|w| w.path.clone())
            } else {
                None
            }
        } else {
            None
        };
        // A project workspace wins; otherwise the directory chosen at launch.
        let launch = session.launch.clone();
        let workspace_path =
            workspace_path.or_else(|| launch.as_ref().and_then(|l| l.working_dir.clone()));
        let credential_ref = if let Some(aid) = &session.account_id {
            if let Some(h) = &self.account_mgr {
                let cref = h.0.lock()
                    .unwrap()
                    .get(aid)
                    .map(|a| a.credential_ref.clone());
                tracing::info!(
                    "Session start: account_id={}, credential_ref={:?}",
                    aid,
                    cref
                );
                cref
            } else {
                tracing::warn!("Session start: account_id={} but no account manager", aid);
                None
            }
        } else {
            tracing::warn!("Session start: no account_id");
            None
        };
        let account_id = session.account_id.clone();
        let _ = session;

        let ctx = crate::types::SessionContext {
            session_id: session_id.clone(),
            task_description: task,
            agent_type: agent_type.clone(),
            workspace_path,
            credential_ref,
            account_id,
            context_snapshot: None,
            agent_config: launch.and_then(|l| serde_json::to_value(l).ok()),
        };

        let (adapter_event_tx, adapter_event_rx) = mpsc::channel(512);
        let handle = self.adapter_factory.create(ctx, adapter_event_tx)?;

        // Process the first event synchronously (Ready or StartFailed)
        // with a timeout so tests don't hang.
        let first_event = tokio::time::timeout(std::time::Duration::from_secs(5), {
            let mut rx = adapter_event_rx;
            async move {
                let event = rx.recv().await;
                (event, rx)
            }
        })
        .await;

        match first_event {
            Ok((Some(AdapterEvent::Ready), rx)) => {
                self.adapters
                    .insert(session_id.0.clone(), LiveAdapter { handle });
                // Forward subsequent adapter events to session manager
                let session_id_clone = session_id.clone();
                let forward_tx = self.adapter_event_tx.clone();
                tokio::spawn(async move {
                    let mut rx = rx;
                    while let Some(evt) = rx.recv().await {
                        if forward_tx
                            .send((session_id_clone.clone(), evt))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                self.emit(AgentEvent::new(
                    EventKind::SessionReady,
                    Some(session_id.clone()),
                    json!({}),
                    format!("adapter:{}", agent_type),
                ))?;
                self.do_transition(
                    &session_id,
                    SessionState::Working,
                    format!("adapter:{}", agent_type),
                )?;
                self.emit(AgentEvent::new(
                    EventKind::SessionStarted,
                    Some(session_id.clone()),
                    json!({}),
                    format!("adapter:{}", agent_type),
                ))?;
                info!("Session started: {}", session_id);
                Ok(())
            }
            Ok((Some(AdapterEvent::StartFailed { reason }), _)) => {
                self.do_transition(&session_id, SessionState::Failed, "system")?;
                bail!(
                    "Adapter start failed for session {}: {}",
                    session_id,
                    reason
                );
            }
            Ok((None, _)) | Err(_) => {
                self.do_transition(&session_id, SessionState::Failed, "system")?;
                bail!("Adapter start timed out for session {}", session_id);
            }
            Ok((Some(other), _)) => {
                self.do_transition(&session_id, SessionState::Failed, "system")?;
                bail!(
                    "Unexpected first adapter event for session {}: {:?}",
                    session_id,
                    other
                );
            }
        }
    }

    fn cmd_pause(&mut self, session_id: Id) -> Result<()> {
        {
            let session = self.get_session_or_err(&session_id)?;
            if !matches!(
                session.state,
                SessionState::Working | SessionState::WaitingForHuman
            ) {
                bail!(
                    "Session {} cannot be paused from state {}",
                    session_id,
                    session.state
                );
            }
        }
        self.do_transition(&session_id, SessionState::Paused, "human")?;
        self.emit(AgentEvent::new(
            EventKind::SessionPaused,
            Some(session_id.clone()),
            json!({}),
            "human",
        ))?;
        debug!("Session paused: {}", session_id);
        Ok(())
    }

    fn cmd_resume(&mut self, session_id: Id) -> Result<()> {
        {
            let session = self.get_session_or_err(&session_id)?;
            if session.state != SessionState::Paused {
                bail!(
                    "Session {} is not Paused (state={})",
                    session_id,
                    session.state
                );
            }
        }
        self.do_transition(&session_id, SessionState::Working, "human")?;
        self.emit(AgentEvent::new(
            EventKind::SessionResumed,
            Some(session_id.clone()),
            json!({}),
            "human",
        ))?;
        debug!("Session resumed: {}", session_id);
        Ok(())
    }

    async fn cmd_stop(&mut self, session_id: Id, reason: Option<String>) -> Result<()> {
        {
            let session = self.get_session_or_err(&session_id)?;
            if session.state.is_terminal() {
                bail!(
                    "Session {} is already in terminal state {}",
                    session_id,
                    session.state
                );
            }
        }

        self.emit(AgentEvent::new(
            EventKind::SessionStopRequested,
            Some(session_id.clone()),
            json!({ "reason": reason }),
            "human",
        ))?;

        // If session hasn't started yet, skip Stopping and go directly to Stopped
        let is_idle = self
            .sessions
            .get(&session_id.0)
            .map(|s| s.state == SessionState::Idle)
            .unwrap_or(false);

        if !is_idle {
            self.do_transition(&session_id, SessionState::Stopping, "human")?;
        }

        // Terminate adapter if running
        if let Some(mut live) = self.adapters.remove(&session_id.0) {
            let _ = live.handle.stop();
        }

        self.do_transition(&session_id, SessionState::Stopped, "system")?;
        self.emit(AgentEvent::new(
            EventKind::SessionStopped,
            Some(session_id.clone()),
            json!({ "reason": reason }),
            "system",
        ))?;

        // Phase 2: decrement account load and reclaim workspace
        let (account_id, workspace_id) = self
            .sessions
            .get(&session_id.0)
            .map(|s| (s.account_id.clone(), s.workspace_id.clone()))
            .unwrap_or((None, None));
        if let Some(aid) = account_id {
            if let Some(h) = &self.account_mgr {
                let cred_ref = {
                    let mut mgr = h.0.lock().unwrap();
                    let _ = mgr.decrement_sessions(&aid);
                    mgr.get(&aid)
                        .filter(|a| a.provider == crate::agy_auth::PROVIDER)
                        .map(|a| a.credential_ref.clone())
                };
                // Persist any token agy refreshed during this session.
                if let Some(cref) = cred_ref {
                    if let Err(e) = crate::agy_auth::sync_profile_back(&aid, &cref) {
                        warn!(
                            "Antigravity credential sync for account {} failed: {}",
                            aid,
                            e.to_string().replace('\n', " ")
                        );
                    }
                }
            }
        }
        if let Some(wid) = workspace_id {
            if let Some(reg_h) = &self.project_registry {
                let _ = reg_h.0.lock().unwrap().reclaim_workspace(&wid);
            }
        }

        // Phase 3: expire pending interactions for this session
        if let Some(hub) = &self.interaction_hub {
            let expired = {
                let mut h = hub.0.lock().unwrap();
                h.expire_session_interactions(&session_id)?
            };
            for item in expired {
                self.emit(AgentEvent::new(
                    EventKind::InteractionExpired,
                    Some(session_id.clone()),
                    json!({ "interaction_id": item.id }),
                    "system",
                ))?;
            }
        }

        info!("Session stopped: {}", session_id);
        Ok(())
    }

    async fn cmd_remove(&mut self, session_id: Id) -> Result<()> {
        let is_terminal = {
            let session = self.get_session_or_err(&session_id)?;
            session.state.is_terminal()
        };

        if !is_terminal {
            if let Err(e) = self
                .cmd_stop(session_id.clone(), Some("session removed".to_string()))
                .await
            {
                warn!(
                    "cmd_remove: error stopping session {} before removal: {}",
                    session_id, e
                );
            }
        }

        // Ensure live adapter is cleaned up
        if let Some(mut live) = self.adapters.remove(&session_id.0) {
            let _ = live.handle.stop();
        }

        // Expire / dismiss any pending interactions if any remain
        if let Some(hub) = &self.interaction_hub {
            let _ = hub
                .0
                .lock()
                .unwrap()
                .expire_session_interactions(&session_id);
        }

        // Remove from in-memory sessions map
        self.sessions.remove(&session_id.0);

        // Emit SessionRemoved event
        self.emit(AgentEvent::new(
            EventKind::SessionRemoved,
            Some(session_id.clone()),
            json!({ "session_id": session_id }),
            "human",
        ))?;

        self.reconcile_account_loads();

        info!("Session removed: {}", session_id);
        Ok(())
    }

    fn cmd_steer(&mut self, session_id: Id, message: String) -> Result<()> {
        {
            let session = self.get_session_or_err(&session_id)?;
            if session.state != SessionState::Working
                && session.state != SessionState::WaitingForHuman
            {
                bail!(
                    "Session {} is in state {}, must be Working or WaitingForHuman to steer",
                    session_id,
                    session.state
                );
            }
        }

        if let Some(live) = self.adapters.get_mut(&session_id.0) {
            live.handle.send_command(AgentCommand::Steer {
                message: message.clone(),
            })?;
        }

        self.emit(AgentEvent::new(
            EventKind::AgentSteeringReceived,
            Some(session_id.clone()),
            json!({ "message": message }),
            "human",
        ))?;

        info!("Session {} steered with message: {}", session_id, message);
        Ok(())
    }

    fn cmd_input(&mut self, session_id: Id, data: String) -> Result<()> {
        if let Some(live) = self.adapters.get_mut(&session_id.0) {
            debug!(
                "cmd_input: forwarding {} bytes to session {}",
                data.len(),
                session_id
            );
            live.handle.send_command(AgentCommand::Input { data })?;
        } else {
            warn!(
                "cmd_input: no live adapter for session {}; {} input byte(s) dropped",
                session_id,
                data.len()
            );
        }
        Ok(())
    }

    fn cmd_resize(&mut self, session_id: Id, rows: u16, cols: u16) -> Result<()> {
        if let Some(live) = self.adapters.get_mut(&session_id.0) {
            live.handle
                .send_command(AgentCommand::Resize { rows, cols })?;
        }
        Ok(())
    }

    fn cmd_query_events(
        &self,
        session_id: Option<Id>,
        limit: Option<u64>,
    ) -> Result<Vec<AgentEvent>> {
        self.store.query(0, session_id.as_ref(), limit)
    }

    // ── Phase 6: Multi-Account Selection, Switching & Hand-Off ───────────────

    fn cmd_select_account(&mut self, session_id: Id, account_id: Id) -> Result<()> {
        let agent_type = {
            let session = self.get_session_or_err(&session_id)?;
            if session.state != SessionState::Idle {
                bail!(
                    "InvalidState: session {} is in state {}, account can only be selected directly before start (use switch_account for running sessions)",
                    session_id,
                    session.state
                );
            }
            session.agent_type.clone()
        };

        let previous_account_id = {
            let session = self.sessions.get(&session_id.0).unwrap();
            session.account_id.clone()
        };

        let actual_account_id = if let Some(h) = &self.account_mgr {
            let mgr = h.0.lock().unwrap();
            let acct = mgr.validate_and_select_explicit(&account_id, &agent_type)?;
            acct.id.clone()
        } else {
            account_id.clone()
        };

        // Decrement previous account if any
        if let Some(old_aid) = &previous_account_id {
            if let Some(h) = &self.account_mgr {
                let _ = h.0.lock().unwrap().decrement_sessions(old_aid);
            }
        }

        // Increment new account
        if let Some(h) = &self.account_mgr {
            let _ = h.0.lock().unwrap().increment_sessions(&actual_account_id);
        }

        if let Some(session) = self.sessions.get_mut(&session_id.0) {
            session.account_id = Some(actual_account_id.clone());
            session.updated_at = chrono::Utc::now();
        }

        self.emit(AgentEvent::new(
            EventKind::AccountSelected,
            Some(session_id.clone()),
            json!({
                "session_id": session_id,
                "account_id": actual_account_id,
                "previous_account_id": previous_account_id,
            }),
            "human",
        ))?;

        info!("Session {} account selected: {}", session_id, account_id);
        Ok(())
    }

    fn create_snapshot_for_session(
        &self,
        session_id: &Id,
        workspace_path: Option<String>,
    ) -> Result<SessionSnapshot> {
        let session = self.get_session_or_err(session_id)?;
        let mut snapshot = SessionSnapshot::new(
            session.id.clone(),
            session.agent_type.clone(),
            session.task_description.clone(),
            session.state.clone(),
            self.store.max_seq().unwrap_or(0),
        );
        snapshot.project_id = session.project_id.clone();
        snapshot.workspace_id = session.workspace_id.clone();
        snapshot.workspace_path = workspace_path;
        snapshot.summary = Some(format!(
            "Snapshot of session {} ({}) in state {}",
            session.id, session.agent_type, session.state
        ));
        Ok(snapshot)
    }

    fn cmd_snapshot(&mut self, session_id: Id) -> Result<SessionSnapshot> {
        let workspace_path = {
            let session = self.get_session_or_err(&session_id)?;
            if let Some(wid) = &session.workspace_id {
                if let Some(reg_h) = &self.project_registry {
                    reg_h
                        .0
                        .lock()
                        .unwrap()
                        .get_workspace(wid)
                        .map(|w| w.path.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };

        let snapshot = self.create_snapshot_for_session(&session_id, workspace_path)?;

        if let Some(s) = self.sessions.get_mut(&session_id.0) {
            s.context_snapshot_id = Some(snapshot.id.clone());
        }

        self.emit(AgentEvent::new(
            EventKind::SessionSnapshotCreated,
            Some(session_id.clone()),
            json!({
                "snapshot_id": snapshot.id,
                "version": snapshot.version,
                "summary": snapshot.summary,
            }),
            "system",
        ))?;

        info!("Session {} snapshot captured: {}", session_id, snapshot.id);
        Ok(snapshot)
    }

    async fn cmd_switch_account(&mut self, session_id: Id, target_account_id: Id) -> Result<Id> {
        let (state, agent_type, current_account_id) = {
            let session = self.get_session_or_err(&session_id)?;
            if session.state.is_terminal() {
                bail!(
                    "InvalidState: Session {} is in terminal state {}",
                    session_id,
                    session.state
                );
            }
            (
                session.state.clone(),
                session.agent_type.clone(),
                session.account_id.clone(),
            )
        };

        // Path A: If session is Idle, rebind account before launch
        if state == SessionState::Idle {
            self.cmd_select_account(session_id.clone(), target_account_id.clone())?;
            return Ok(session_id);
        }

        // Path B: Running session
        let caps = self.adapter_factory.capabilities(&agent_type);
        if caps.switch_mode == AccountSwitchMode::Unsupported {
            let _ = self.emit(AgentEvent::new(
                EventKind::AccountSwitchFailed,
                Some(session_id.clone()),
                json!({
                    "target_account_id": target_account_id,
                    "reason": "unsupported_capability",
                }),
                "system",
            ));
            bail!(
                "UnsupportedCapability: Provider for agent_type '{}' does not support account switching",
                agent_type
            );
        }

        // Validate target account before emitting request
        let (target_cred, resolved_target_id) = if let Some(mgr_handle) = self.account_mgr.clone() {
            let res = {
                let mgr = mgr_handle.0.lock().unwrap();
                mgr.validate_and_select_explicit(&target_account_id, &agent_type)
                    .map(|a| (a.credential_ref.clone(), a.id.clone()))
            };
            match res {
                Ok(res) => res,
                Err(e) => {
                    let _ = self.emit(AgentEvent::new(
                        EventKind::AccountSwitchFailed,
                        Some(session_id.clone()),
                        json!({
                            "target_account_id": target_account_id,
                            "reason": e.to_string(),
                        }),
                        "system",
                    ));
                    return Err(e);
                }
            }
        } else {
            bail!("AccountNotFound: Account manager not available");
        };
        let target_account_id = resolved_target_id;

        self.emit(AgentEvent::new(
            EventKind::AccountSwitchRequested,
            Some(session_id.clone()),
            json!({
                "target_account_id": target_account_id,
                "current_account_id": current_account_id,
                "switch_mode": caps.switch_mode,
            }),
            "human",
        ))?;

        match caps.switch_mode {
            AccountSwitchMode::Dynamic => {
                self.emit(AgentEvent::new(
                    EventKind::AccountSwitchStarted,
                    Some(session_id.clone()),
                    json!({
                        "target_account_id": target_account_id,
                        "mode": "dynamic",
                    }),
                    "system",
                ))?;

                // Notify live adapter
                if let Some(live) = self.adapters.get_mut(&session_id.0) {
                    let _ = live.handle.send_command(AgentCommand::SwitchAccount {
                        credential_ref: target_cred,
                    });
                }

                // Update counters
                if let Some(old_aid) = &current_account_id {
                    if let Some(mgr) = &self.account_mgr {
                        let _ = mgr.0.lock().unwrap().decrement_sessions(old_aid);
                    }
                }
                if let Some(mgr) = &self.account_mgr {
                    let _ = mgr.0.lock().unwrap().increment_sessions(&target_account_id);
                }

                if let Some(session) = self.sessions.get_mut(&session_id.0) {
                    session.account_id = Some(target_account_id.clone());
                    session.updated_at = chrono::Utc::now();
                }

                self.emit(AgentEvent::new(
                    EventKind::AccountSwitchCompleted,
                    Some(session_id.clone()),
                    json!({
                        "old_account_id": current_account_id,
                        "new_account_id": target_account_id,
                        "mode": "dynamic",
                    }),
                    "system",
                ))?;

                self.emit(AgentEvent::new(
                    EventKind::AccountSelected,
                    Some(session_id.clone()),
                    json!({
                        "session_id": session_id,
                        "account_id": target_account_id,
                        "previous_account_id": current_account_id,
                    }),
                    "system",
                ))?;

                Ok(session_id)
            }
            AccountSwitchMode::RequiresRestart => {
                self.cmd_handoff_internal(session_id, target_account_id)
                    .await
            }
            AccountSwitchMode::Unsupported => unreachable!(),
        }
    }

    async fn cmd_handoff(&mut self, session_id: Id, target_account_id: Option<Id>) -> Result<Id> {
        let (agent_type, current_account_id, project_id) = {
            let session = self.get_session_or_err(&session_id)?;
            if session.state.is_terminal() {
                bail!(
                    "InvalidState: Session {} is already in terminal state {}",
                    session_id,
                    session.state
                );
            }
            (
                session.agent_type.clone(),
                session.account_id.clone(),
                session.project_id.clone(),
            )
        };

        let resolved_target_id = if let Some(tid) = target_account_id {
            tid
        } else if let Some(mgr_handle) = self.account_mgr.clone() {
            // Auto-select an account compatible with agent_type excluding current_account_id
            let tags = if let Some(pid) = &project_id {
                if let Some(reg) = &self.project_registry {
                    reg.0
                        .lock()
                        .unwrap()
                        .get(pid)
                        .map(|p| p.default_account_tags.clone())
                        .unwrap_or_default()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            let mgr = mgr_handle.0.lock().unwrap();
            let candidate = mgr
                .list(None, &tags)
                .into_iter()
                .filter(|a| a.can_accept_session())
                .filter(|a| a.supports_agent_type(&agent_type))
                .filter(|a| Some(&a.id) != current_account_id.as_ref())
                .min_by(|a, b| {
                    a.active_session_count
                        .cmp(&b.active_session_count)
                        .then_with(|| a.label.cmp(&b.label))
                        .then_with(|| a.id.0.cmp(&b.id.0))
                });

            match candidate {
                Some(a) => a.id.clone(),
                None => bail!(
                    "NoAccountAvailable: No alternative active account available for hand-off"
                ),
            }
        } else {
            bail!("AccountNotFound: Account manager not available");
        };

        self.cmd_switch_account(session_id, resolved_target_id)
            .await
    }

    async fn cmd_handoff_internal(
        &mut self,
        predecessor_id: Id,
        target_account_id: Id,
    ) -> Result<Id> {
        // 1. Validate predecessor
        let (
            task_description,
            agent_type,
            project_id,
            old_account_id,
            workspace_id,
            workspace_path,
            launch,
        ) = {
            let session = self.get_session_or_err(&predecessor_id)?;
            if session.state.is_terminal() {
                bail!(
                    "InvalidState: Session {} is already in terminal state {}",
                    predecessor_id,
                    session.state
                );
            }
            let ws_path = if let Some(wid) = &session.workspace_id {
                if let Some(reg) = &self.project_registry {
                    reg.0
                        .lock()
                        .unwrap()
                        .get_workspace(wid)
                        .map(|w| w.path.clone())
                } else {
                    None
                }
            } else {
                None
            };
            (
                session.task_description.clone(),
                session.agent_type.clone(),
                session.project_id.clone(),
                session.account_id.clone(),
                session.workspace_id.clone(),
                ws_path.or_else(|| session.launch.as_ref().and_then(|l| l.working_dir.clone())),
                session.launch.clone(),
            )
        };

        // 2. Validate target account
        let (target_credential_ref, resolved_target_id) =
            if let Some(mgr_handle) = self.account_mgr.clone() {
                let res = {
                    let mgr = mgr_handle.0.lock().unwrap();
                    mgr.validate_and_select_explicit(&target_account_id, &agent_type)
                        .map(|a| (a.credential_ref.clone(), a.id.clone()))
                };
                match res {
                    Ok(res) => res,
                    Err(e) => {
                        let _ = self.emit(AgentEvent::new(
                        EventKind::SessionHandOffFailed,
                        Some(predecessor_id.clone()),
                        json!({ "target_account_id": target_account_id, "reason": e.to_string() }),
                        "system",
                    ));
                        return Err(e);
                    }
                }
            } else {
                bail!("AccountNotFound: Account manager not available");
            };
        let target_account_id = resolved_target_id;

        // 2b. Preflight the successor (e.g. credential validation) BEFORE the
        // predecessor is stopped, so a bad target never leaves the user with
        // no running session and never falls back to another login.
        let probe_ctx = crate::types::SessionContext::new(
            Id::new(),
            task_description.clone(),
            agent_type.clone(),
        )
        .with_account(target_account_id.clone(), target_credential_ref.clone());
        if let Err(e) = self.adapter_factory.preflight(&probe_ctx) {
            let _ = self.emit(AgentEvent::new(
                EventKind::SessionHandOffFailed,
                Some(predecessor_id.clone()),
                json!({ "target_account_id": target_account_id, "reason": "target account credential check failed" }),
                "system",
            ));
            let _ = self.emit(AgentEvent::new(
                EventKind::AccountSwitchFailed,
                Some(predecessor_id.clone()),
                json!({ "target_account_id": target_account_id, "reason": "target account credential check failed" }),
                "system",
            ));
            return Err(e);
        }

        // 3. Emit HandOffStarted & AccountSwitchStarted
        self.emit(AgentEvent::new(
            EventKind::AccountSwitchStarted,
            Some(predecessor_id.clone()),
            json!({
                "target_account_id": target_account_id,
                "predecessor_id": predecessor_id,
            }),
            "system",
        ))?;

        self.emit(AgentEvent::new(
            EventKind::SessionHandOffStarted,
            Some(predecessor_id.clone()),
            json!({
                "target_account_id": target_account_id,
                "predecessor_id": predecessor_id,
            }),
            "system",
        ))?;

        // 4. Capture snapshot
        let snapshot =
            match self.create_snapshot_for_session(&predecessor_id, workspace_path.clone()) {
                Ok(s) => s,
                Err(e) => {
                    let _ = self.emit(AgentEvent::new(
                        EventKind::SessionHandOffFailed,
                        Some(predecessor_id.clone()),
                        json!({ "reason": format!("SnapshotFailed: {e}") }),
                        "system",
                    ));
                    let _ = self.emit(AgentEvent::new(
                        EventKind::AccountSwitchFailed,
                        Some(predecessor_id.clone()),
                        json!({ "reason": format!("SnapshotFailed: {e}") }),
                        "system",
                    ));
                    bail!("SnapshotFailed: {e}");
                }
            };

        self.emit(AgentEvent::new(
            EventKind::SessionSnapshotCreated,
            Some(predecessor_id.clone()),
            json!({
                "snapshot_id": snapshot.id,
                "version": snapshot.version,
                "summary": snapshot.summary,
            }),
            "system",
        ))?;

        let successor_id = Id::new();

        // 5. Safely stop predecessor process
        if let Some(mut live) = self.adapters.remove(&predecessor_id.0) {
            let _ = live.handle.stop();
        }

        // Transition predecessor -> Stopping -> HandedOff
        self.do_transition(&predecessor_id, SessionState::Stopping, "system")?;
        self.do_transition(&predecessor_id, SessionState::HandedOff, "system")?;

        if let Some(s) = self.sessions.get_mut(&predecessor_id.0) {
            s.successor_id = Some(successor_id.clone());
            s.context_snapshot_id = Some(snapshot.id.clone());
        }

        // Decrement predecessor account load
        if let Some(aid) = &old_account_id {
            if let Some(mgr) = &self.account_mgr {
                let _ = mgr.0.lock().unwrap().decrement_sessions(aid);
            }
        }

        self.emit(AgentEvent::new(
            EventKind::SessionHandedOff,
            Some(predecessor_id.clone()),
            json!({
                "predecessor_id": predecessor_id,
                "successor_id": successor_id,
                "target_account_id": target_account_id,
            }),
            "system",
        ))?;

        // 6. Create successor session (Idle)
        let mut successor = AgentSession::new(
            successor_id.clone(),
            task_description.clone(),
            agent_type.clone(),
        );
        successor.project_id = project_id.clone();
        successor.account_id = Some(target_account_id.clone());
        successor.workspace_id = workspace_id.clone(); // Workspace preserved!
        successor.predecessor_id = Some(predecessor_id.clone());
        successor.context_snapshot_id = Some(snapshot.id.clone());
        successor.launch = launch.clone();
        self.sessions.insert(successor_id.0.clone(), successor);

        // Increment target account load
        if let Some(mgr) = &self.account_mgr {
            let _ = mgr.0.lock().unwrap().increment_sessions(&target_account_id);
        }

        self.emit(AgentEvent::new(
            EventKind::SessionCreated,
            Some(successor_id.clone()),
            json!({
                "task_description": task_description,
                "agent_type": agent_type,
                "project_id": project_id,
                "account_id": target_account_id,
                "workspace_id": workspace_id,
                "predecessor_id": predecessor_id,
                "context_snapshot_id": snapshot.id,
                "launch": launch,
            }),
            "system",
        ))?;

        self.emit(AgentEvent::new(
            EventKind::AccountSelected,
            Some(successor_id.clone()),
            json!({
                "session_id": successor_id,
                "account_id": target_account_id,
                "previous_account_id": old_account_id,
            }),
            "system",
        ))?;

        // 7. Start successor session
        self.emit(AgentEvent::new(
            EventKind::SessionStartRequested,
            Some(successor_id.clone()),
            json!({}),
            "system",
        ))?;

        self.do_transition(&successor_id, SessionState::Starting, "system")?;

        let ctx = crate::types::SessionContext {
            session_id: successor_id.clone(),
            task_description: task_description.clone(),
            agent_type: agent_type.clone(),
            workspace_path,
            credential_ref: Some(target_credential_ref),
            account_id: Some(target_account_id.clone()),
            context_snapshot: Some(snapshot.to_context_string()),
            agent_config: launch.and_then(|l| serde_json::to_value(l).ok()),
        };

        let (adapter_event_tx, adapter_event_rx) = mpsc::channel(512);
        let handle = self.adapter_factory.create(ctx, adapter_event_tx)?;

        // Wait for first event (Ready) with 5-second timeout
        let first_event = tokio::time::timeout(std::time::Duration::from_secs(5), {
            let mut rx = adapter_event_rx;
            async move {
                let event = rx.recv().await;
                (event, rx)
            }
        })
        .await;

        match first_event {
            Ok((Some(AdapterEvent::Ready), rx)) => {
                self.adapters
                    .insert(successor_id.0.clone(), LiveAdapter { handle });
                let successor_id_clone = successor_id.clone();
                let forward_tx = self.adapter_event_tx.clone();
                tokio::spawn(async move {
                    let mut rx = rx;
                    while let Some(evt) = rx.recv().await {
                        if forward_tx
                            .send((successor_id_clone.clone(), evt))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });

                self.emit(AgentEvent::new(
                    EventKind::SessionReady,
                    Some(successor_id.clone()),
                    json!({}),
                    format!("adapter:{}", agent_type),
                ))?;
                self.do_transition(
                    &successor_id,
                    SessionState::Working,
                    format!("adapter:{}", agent_type),
                )?;
                self.emit(AgentEvent::new(
                    EventKind::SessionStarted,
                    Some(successor_id.clone()),
                    json!({}),
                    format!("adapter:{}", agent_type),
                ))?;

                self.emit(AgentEvent::new(
                    EventKind::SessionHandOffCompleted,
                    Some(predecessor_id.clone()),
                    json!({
                        "predecessor_session_id": predecessor_id,
                        "successor_session_id": successor_id,
                        "target_account_id": target_account_id,
                        "snapshot_id": snapshot.id,
                    }),
                    "system",
                ))?;

                self.emit(AgentEvent::new(
                    EventKind::AccountSwitchCompleted,
                    Some(predecessor_id.clone()),
                    json!({
                        "old_session_id": predecessor_id,
                        "new_session_id": successor_id,
                        "old_account_id": old_account_id,
                        "new_account_id": target_account_id,
                    }),
                    "system",
                ))?;

                info!(
                    "Session hand-off complete: {} -> {}",
                    predecessor_id, successor_id
                );
                Ok(successor_id)
            }
            Ok((Some(AdapterEvent::StartFailed { reason }), _)) => {
                self.do_transition(&successor_id, SessionState::Failed, "system")?;
                self.emit(AgentEvent::new(
                    EventKind::SessionHandOffFailed,
                    Some(successor_id.clone()),
                    json!({ "reason": reason }),
                    "system",
                ))?;
                self.emit(AgentEvent::new(
                    EventKind::AccountSwitchFailed,
                    Some(successor_id.clone()),
                    json!({ "reason": reason }),
                    "system",
                ))?;
                bail!(
                    "HandOffFailed: Adapter start failed for successor session {}: {}",
                    successor_id,
                    reason
                );
            }
            _ => {
                self.do_transition(&successor_id, SessionState::Failed, "system")?;
                self.emit(AgentEvent::new(
                    EventKind::SessionHandOffFailed,
                    Some(successor_id.clone()),
                    json!({ "reason": "timeout" }),
                    "system",
                ))?;
                self.emit(AgentEvent::new(
                    EventKind::AccountSwitchFailed,
                    Some(successor_id.clone()),
                    json!({ "reason": "timeout" }),
                    "system",
                ))?;
                bail!(
                    "HandOffFailed: Adapter start timed out for successor session {}",
                    successor_id
                );
            }
        }
    }

    // ── Phase 3: Interaction & Adapter event handlers ────────────────────────

    async fn handle_adapter_event(&mut self, session_id: Id, event: AdapterEvent) {
        match &event {
            AdapterEvent::OutputChunk { text, .. } => {
                trace!(
                    "SessionManager: OutputChunk from {} ({} bytes)",
                    session_id,
                    text.len()
                );
            }
            other => {
                debug!(
                    "SessionManager: event from session {}: {:?}",
                    session_id, other
                );
            }
        }
        match event {
            AdapterEvent::Ready => {}
            AdapterEvent::OutputChunk {
                text,
                confidence: _,
            } => {
                let _ = self.emit(AgentEvent::new(
                    EventKind::AgentOutputReceived,
                    Some(session_id),
                    json!({ "text": text }),
                    "adapter",
                ));
            }
            AdapterEvent::ApprovalRequested { tool_name, prompt } => {
                self.handle_approval_requested(session_id, tool_name, prompt)
                    .await;
            }
            AdapterEvent::QuestionRaised { prompt } => {
                self.handle_question_raised(session_id, prompt).await;
            }
            AdapterEvent::RateLimitSignal { back_off_secs } => {
                let _ = self.emit(AgentEvent::new(
                    EventKind::AccountRateLimited,
                    Some(session_id),
                    json!({ "back_off_secs": back_off_secs }),
                    "adapter",
                ));
            }
            AdapterEvent::Paused => {}
            AdapterEvent::Resumed => {}
            AdapterEvent::Completed { summary } => {
                let _ = self.cmd_stop(session_id, summary).await;
            }
            AdapterEvent::Crashed { exit_code, reason } => {
                let _ = self.emit(AgentEvent::new(
                    EventKind::SessionCrashed,
                    Some(session_id.clone()),
                    json!({ "exit_code": exit_code, "reason": reason }),
                    "adapter",
                ));
                let _ = self.do_transition(&session_id, SessionState::Crashed, "adapter");
                self.reconcile_account_loads();
            }
            AdapterEvent::SnapshotProduced { summary } => {
                let _ = self.emit(AgentEvent::new(
                    EventKind::SnapshotCreated,
                    Some(session_id),
                    json!({ "summary": summary }),
                    "adapter",
                ));
            }
            AdapterEvent::StartFailed { reason } => {
                warn!(
                    "StartFailed after start for session {}: {}",
                    session_id, reason
                );
            }
        }
    }

    async fn handle_approval_requested(
        &mut self,
        session_id: Id,
        tool_name: String,
        prompt: String,
    ) {
        let (project_id, agent_type) = if let Some(s) = self.sessions.get(&session_id.0) {
            (s.project_id.clone(), s.agent_type.clone())
        } else {
            return;
        };

        if let Some(hub) = &self.interaction_hub {
            let submission = {
                let mut h = hub.0.lock().unwrap();
                h.submit_approval_request(
                    session_id.clone(),
                    tool_name.clone(),
                    None,
                    prompt.clone(),
                    Some(&agent_type),
                    project_id.as_ref(),
                )
            };

            match submission {
                Ok(InteractionSubmissionResult::AutoResolved {
                    interaction,
                    decision,
                    command,
                }) => {
                    let kind = match decision {
                        PolicyDecision::Allow => EventKind::ApprovalAutoApproved,
                        PolicyDecision::Deny => EventKind::ApprovalDenied,
                        PolicyDecision::RequireHuman => EventKind::ApprovalRequested,
                    };
                    let _ = self.emit(AgentEvent::new(
                        kind,
                        Some(session_id.clone()),
                        json!({
                            "interaction_id": interaction.id,
                            "tool_name": tool_name,
                            "decision": decision,
                            "policy_id": interaction.policy_id,
                        }),
                        "policy",
                    ));

                    if let Some(live) = self.adapters.get_mut(&session_id.0) {
                        let _ = live.handle.send_command(command);
                    }
                }
                Ok(InteractionSubmissionResult::Escalated { interaction }) => {
                    let _ = self.emit(AgentEvent::new(
                        EventKind::ApprovalRequested,
                        Some(session_id.clone()),
                        json!({
                            "interaction_id": interaction.id,
                            "tool_name": tool_name,
                            "prompt": prompt,
                        }),
                        "interaction_hub",
                    ));
                    let _ = self.do_transition(
                        &session_id,
                        SessionState::WaitingForHuman,
                        "interaction_hub",
                    );
                }
                Err(e) => {
                    warn!("Error processing approval request: {e}");
                }
            }
        } else {
            let _ = self.emit(AgentEvent::new(
                EventKind::ApprovalRequested,
                Some(session_id.clone()),
                json!({ "tool_name": tool_name, "prompt": prompt }),
                "adapter",
            ));
            let _ = self.do_transition(&session_id, SessionState::WaitingForHuman, "system");
        }
    }

    async fn handle_question_raised(&mut self, session_id: Id, prompt: String) {
        if let Some(hub) = &self.interaction_hub {
            let submission = {
                let mut h = hub.0.lock().unwrap();
                h.submit_question(session_id.clone(), prompt.clone())
            };

            match submission {
                Ok(InteractionSubmissionResult::Escalated { interaction }) => {
                    let _ = self.emit(AgentEvent::new(
                        EventKind::AgentQuestion,
                        Some(session_id.clone()),
                        json!({
                            "interaction_id": interaction.id,
                            "prompt": prompt,
                        }),
                        "interaction_hub",
                    ));
                    let _ = self.do_transition(
                        &session_id,
                        SessionState::WaitingForHuman,
                        "interaction_hub",
                    );
                }
                _ => {}
            }
        } else {
            let _ = self.emit(AgentEvent::new(
                EventKind::AgentQuestion,
                Some(session_id.clone()),
                json!({ "prompt": prompt }),
                "adapter",
            ));
            let _ = self.do_transition(&session_id, SessionState::WaitingForHuman, "system");
        }
    }

    async fn cmd_respond_interaction(
        &mut self,
        interaction_id: Id,
        decision: Option<PolicyDecision>,
        response: Option<String>,
        actor: Option<String>,
    ) -> Result<Interaction> {
        let hub = self
            .interaction_hub
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Interaction hub not available"))?;

        let resolution = {
            let mut h = hub.0.lock().unwrap();
            h.reply(&interaction_id, decision, response.clone(), actor.clone())?
        };

        let session_id = resolution.interaction.session_id.clone();
        let actor_str = actor.unwrap_or_else(|| "human".into());

        let event_kind = match resolution.interaction.kind {
            crate::types::InteractionKind::ApprovalRequest => EventKind::ApprovalHumanDecision,
            crate::types::InteractionKind::Question => EventKind::InteractionHumanResolved,
        };

        self.emit(AgentEvent::new(
            event_kind,
            Some(session_id.clone()),
            json!({
                "interaction_id": interaction_id,
                "decision": resolution.interaction.decision,
                "response": response,
            }),
            &actor_str,
        ))?;

        // Send command to adapter if one was produced
        if let Some(cmd) = resolution.command {
            if let Some(live) = self.adapters.get_mut(&session_id.0) {
                let _ = live.handle.send_command(cmd);
            }
        }

        // Check if this session has any remaining pending interactions
        let has_pending = {
            let h = hub.0.lock().unwrap();
            !h.list_pending(Some(&session_id)).is_empty()
        };

        if !has_pending {
            if let Some(session) = self.sessions.get(&session_id.0) {
                if session.state == SessionState::WaitingForHuman {
                    self.do_transition(&session_id, SessionState::Working, actor_str)?;
                }
            }
        }

        Ok(resolution.interaction)
    }

    async fn cmd_dismiss_interaction(
        &mut self,
        interaction_id: Id,
        actor: Option<String>,
    ) -> Result<Interaction> {
        let hub = self
            .interaction_hub
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Interaction hub not available"))?;

        let actor_str = actor.unwrap_or_else(|| "human".into());
        let dismissed = {
            let mut h = hub.0.lock().unwrap();
            h.dismiss(&interaction_id, Some(actor_str.clone()))?
        };

        let session_id = dismissed.session_id.clone();

        self.emit(AgentEvent::new(
            EventKind::InteractionDismissed,
            Some(session_id.clone()),
            json!({
                "interaction_id": interaction_id,
            }),
            &actor_str,
        ))?;

        let has_pending = {
            let h = hub.0.lock().unwrap();
            !h.list_pending(Some(&session_id)).is_empty()
        };

        if !has_pending {
            if let Some(session) = self.sessions.get(&session_id.0) {
                if session.state == SessionState::WaitingForHuman {
                    self.do_transition(&session_id, SessionState::Working, actor_str)?;
                }
            }
        }

        Ok(dismissed)
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    fn get_session_or_err(&self, id: &Id) -> Result<&AgentSession> {
        self.sessions
            .get(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", id))
    }

    /// Apply a state machine transition, emit StateChanged or TransitionRejected.
    fn do_transition(
        &mut self,
        session_id: &Id,
        to: SessionState,
        triggered_by: impl Into<String> + Clone,
    ) -> Result<()> {
        let session = self
            .sessions
            .get_mut(&session_id.0)
            .ok_or_else(|| anyhow::anyhow!("Session not found: {}", session_id))?;

        let result = state_machine::apply(session, to.clone());

        match result {
            state_machine::TransitionResult::Changed { from, to: new_to } => {
                self.emit(AgentEvent::new(
                    EventKind::StateChanged,
                    Some(session_id.clone()),
                    json!({
                        "from": from,
                        "to": new_to,
                    }),
                    triggered_by,
                ))?;
                Ok(())
            }
            state_machine::TransitionResult::Rejected { current, attempted } => {
                self.emit(AgentEvent::new(
                    EventKind::TransitionRejected,
                    Some(session_id.clone()),
                    json!({
                        "current": current,
                        "attempted": attempted,
                    }),
                    triggered_by,
                ))?;
                bail!(
                    "Invalid transition for session {}: {:?} → {:?}",
                    session_id,
                    current,
                    attempted
                );
            }
        }
    }

    /// Append event to the store and broadcast it.
    fn emit(&mut self, mut event: AgentEvent) -> Result<()> {
        self.store.append(&mut event)?;
        let _ = self.event_tx.send(event); // ok if no subscribers
        Ok(())
    }
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Cheap cloneable handle to talk to the session manager task.
#[derive(Clone)]
pub struct SessionManagerHandle {
    tx: mpsc::Sender<SessionCmd>,
}

impl SessionManagerHandle {
    pub fn new(tx: mpsc::Sender<SessionCmd>) -> Self {
        Self { tx }
    }

    async fn send_and_wait<T>(
        &self,
        build: impl FnOnce(tokio::sync::oneshot::Sender<T>) -> SessionCmd,
    ) -> Result<T> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(build(reply_tx))
            .await
            .map_err(|_| anyhow::anyhow!("Session manager is not running"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("Session manager dropped the reply"))
    }

    pub async fn create(&self, task: String, agent_type: String) -> Result<Id> {
        self.send_and_wait(|reply| SessionCmd::Create {
            task_description: task,
            agent_type,
            project_id: None,
            account_id: None,
            launch: None,
            reply,
        })
        .await?
    }

    /// Phase 2: create a session with optional project and/or account binding.
    pub async fn create_with_context(
        &self,
        task: String,
        agent_type: String,
        project_id: Option<Id>,
        account_id: Option<Id>,
    ) -> Result<Id> {
        self.create_with_launch(task, agent_type, project_id, account_id, None)
            .await
    }

    /// Create a session with explicit per-session launch options.
    pub async fn create_with_launch(
        &self,
        task: String,
        agent_type: String,
        project_id: Option<Id>,
        account_id: Option<Id>,
        launch: Option<crate::agy_launch::AgyLaunchOptions>,
    ) -> Result<Id> {
        self.send_and_wait(|reply| SessionCmd::Create {
            task_description: task,
            agent_type,
            project_id,
            account_id,
            launch,
            reply,
        })
        .await?
    }

    pub async fn start(&self, session_id: Id) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::Start { session_id, reply })
            .await?
    }

    pub async fn pause(&self, session_id: Id) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::Pause { session_id, reply })
            .await?
    }

    pub async fn resume(&self, session_id: Id) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::Resume { session_id, reply })
            .await?
    }

    pub async fn stop(&self, session_id: Id, reason: Option<String>) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::Stop {
            session_id,
            reason,
            reply,
        })
        .await?
    }

    pub async fn remove(&self, session_id: Id) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::Remove { session_id, reply })
            .await?
    }

    pub async fn list(&self) -> Result<Vec<AgentSession>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(SessionCmd::List { reply: reply_tx })
            .await
            .map_err(|_| anyhow::anyhow!("Session manager is not running"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("Session manager dropped reply"))
    }

    pub async fn get(&self, session_id: Id) -> Result<Option<AgentSession>> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(SessionCmd::Get {
                session_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Session manager is not running"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("Session manager dropped reply"))
    }

    /// Phase 3: Respond to or resolve an interaction.
    pub async fn respond_interaction(
        &self,
        interaction_id: Id,
        decision: Option<PolicyDecision>,
        response: Option<String>,
        actor: Option<String>,
    ) -> Result<Interaction> {
        self.send_and_wait(|reply| SessionCmd::RespondInteraction {
            interaction_id,
            decision,
            response,
            actor,
            reply,
        })
        .await?
    }

    /// Phase 3: Dismiss an interaction without a decision.
    pub async fn dismiss_interaction(
        &self,
        interaction_id: Id,
        actor: Option<String>,
    ) -> Result<Interaction> {
        self.send_and_wait(|reply| SessionCmd::DismissInteraction {
            interaction_id,
            actor,
            reply,
        })
        .await?
    }

    /// Steer a running session by injecting a human instruction.
    pub async fn steer(&self, session_id: Id, message: String) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::Steer {
            session_id,
            message,
            reply,
        })
        .await?
    }

    /// Direct PTY input sent to session stdin.
    pub async fn input(&self, session_id: Id, data: String) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::Input {
            session_id,
            data,
            reply,
        })
        .await?
    }

    /// Resize PTY terminal dimensions.
    pub async fn resize(&self, session_id: Id, rows: u16, cols: u16) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::Resize {
            session_id,
            rows,
            cols,
            reply,
        })
        .await?
    }

    /// Query historical events from the store.
    pub async fn query_events(
        &self,
        session_id: Option<Id>,
        limit: Option<u64>,
    ) -> Result<Vec<AgentEvent>> {
        self.send_and_wait(|reply| SessionCmd::QueryEvents {
            session_id,
            limit,
            reply,
        })
        .await?
    }

    /// Query historical events starting from a specific sequence number (Phase 7 replay).
    pub async fn query_events_since(
        &self,
        since_seq: u64,
        session_id: Option<Id>,
        limit: Option<u64>,
    ) -> Result<Vec<AgentEvent>> {
        self.send_and_wait(|reply| SessionCmd::QueryEventsSince {
            since_seq,
            session_id,
            limit,
            reply,
        })
        .await?
    }

    /// Phase 6: Explicitly select an account for an Idle session.
    pub async fn select_account(&self, session_id: Id, account_id: Id) -> Result<()> {
        self.send_and_wait(|reply| SessionCmd::SelectAccount {
            session_id,
            account_id,
            reply,
        })
        .await?
    }

    /// Phase 6: Switch account for a session (Idle rebind or running hand-off/dynamic).
    pub async fn switch_account(&self, session_id: Id, target_account_id: Id) -> Result<Id> {
        self.send_and_wait(|reply| SessionCmd::SwitchAccount {
            session_id,
            target_account_id,
            reply,
        })
        .await?
    }

    /// Phase 6: Capture a session snapshot.
    pub async fn snapshot(&self, session_id: Id) -> Result<SessionSnapshot> {
        self.send_and_wait(|reply| SessionCmd::Snapshot { session_id, reply })
            .await?
    }

    /// Phase 6: Controlled hand-off to another account.
    pub async fn handoff(&self, session_id: Id, target_account_id: Option<Id>) -> Result<Id> {
        self.send_and_wait(|reply| SessionCmd::HandOff {
            session_id,
            target_account_id,
            reply,
        })
        .await?
    }
}
