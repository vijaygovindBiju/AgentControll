//! Control API IPC server — Unix domain socket, JSON request/response.
//!
//! Phase 7 additions: shared `dispatch_request`, `is_mutating_cmd`,
//! `SubscriptionFilter` filtering, and `since_seq` catch-up replay.

use anyhow::Result;
use serde_json::json;
use std::path::Path;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::broadcast,
};
use tracing::{debug, error, info, warn};

use crate::{
    account_manager::AccountManagerHandle,
    interaction_hub::InteractionHubHandle,
    policy_engine::PolicyEngineHandle,
    project_registry::ProjectRegistryHandle,
    session::manager::SessionManagerHandle,
    types::{
        Account, AgentEvent, ApiRequest, ApiResponse, Id, InteractionState, Policy,
        PolicyCondition, PolicyDecision, PolicyScope, Project, SubscriptionFilter,
        TokenScope, WorkspacePolicy,
    },
};

pub struct IpcServer {
    listener: UnixListener,
    session_mgr: SessionManagerHandle,
    account_mgr: Option<AccountManagerHandle>,
    project_registry: Option<ProjectRegistryHandle>,
    interaction_hub: Option<InteractionHubHandle>,
    policy_engine: Option<PolicyEngineHandle>,
    event_tx: broadcast::Sender<AgentEvent>,
}

impl IpcServer {
    pub fn bind(
        socket_path: &Path,
        session_mgr: SessionManagerHandle,
        event_tx: broadcast::Sender<AgentEvent>,
    ) -> Result<Self> {
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let listener = UnixListener::bind(socket_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
        }
        info!("IPC server listening on {}", socket_path.display());
        Ok(Self {
            listener,
            session_mgr,
            account_mgr: None,
            project_registry: None,
            interaction_hub: None,
            policy_engine: None,
            event_tx,
        })
    }

    pub fn with_account_manager(mut self, h: AccountManagerHandle) -> Self {
        self.account_mgr = Some(h);
        self
    }

    pub fn with_project_registry(mut self, h: ProjectRegistryHandle) -> Self {
        self.project_registry = Some(h);
        self
    }

    pub fn with_interaction_hub(mut self, h: InteractionHubHandle) -> Self {
        self.interaction_hub = Some(h);
        self
    }

    pub fn with_policy_engine(mut self, h: PolicyEngineHandle) -> Self {
        self.policy_engine = Some(h);
        self
    }

    pub async fn run(self) {
        loop {
            match self.listener.accept().await {
                Ok((stream, _)) => {
                    debug!("IPC: new client connected");
                    let mgr = self.session_mgr.clone();
                    let evt_tx = self.event_tx.clone();
                    let acct_mgr = self.account_mgr.clone();
                    let proj_reg = self.project_registry.clone();
                    let hub = self.interaction_hub.clone();
                    let policy = self.policy_engine.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            handle_connection(stream, mgr, acct_mgr, proj_reg, hub, policy, evt_tx).await
                        {
                            warn!("IPC connection error: {e}");
                        }
                    });
                }
                Err(e) => {
                    error!("IPC accept error: {e}");
                    break;
                }
            }
        }
    }
}

/// Returns true if `cmd` mutates daemon state and requires write authorization.
pub fn is_mutating_cmd(cmd: &str) -> bool {
    matches!(
        cmd,
        "session.create"
            | "session.start"
            | "session.create_and_start"
            | "session.pause"
            | "session.resume"
            | "session.stop"
            | "session.steer"
            | "session.select_account"
            | "session.switch_account"
            | "session.handoff"
            | "account.register"
            | "account.disable"
            | "account.enable"
            | "account.remove"
            | "account.switch"
            | "project.register"
            | "project.remove"
            | "interaction.reply"
            | "interaction.resolve"
            | "interaction.dismiss"
            | "policy.upsert"
            | "policy.create"
            | "policy.update"
            | "policy.remove"
            | "policy.delete"
    )
}

/// Unified command dispatcher shared between Unix socket IPC and WebSocket servers.
pub async fn dispatch_request(
    req: &ApiRequest,
    session_mgr: &SessionManagerHandle,
    account_mgr: &Option<AccountManagerHandle>,
    project_registry: &Option<ProjectRegistryHandle>,
    interaction_hub: &Option<InteractionHubHandle>,
    policy_engine: &Option<PolicyEngineHandle>,
    token_scope: Option<TokenScope>,
) -> ApiResponse {
    if req.v != 1 {
        return ApiResponse::err(
            &req.id,
            "VersionMismatch",
            format!("Unsupported schema version: {}", req.v),
        );
    }

    if let Some(scope) = token_scope {
        if !scope.can_write() && is_mutating_cmd(&req.cmd) {
            return ApiResponse::err(
                &req.id,
                "PermissionDenied",
                format!("Token scope 'read' does not allow mutating command '{}'", req.cmd),
            );
        }
    }

    let family = req.cmd.split('.').next().unwrap_or("");
    match family {
        "session" => handle_session_cmd(req, session_mgr).await,
        "account" => handle_account_cmd(req, account_mgr, session_mgr).await,
        "project" => handle_project_cmd(req, project_registry).await,
        "interaction" => handle_interaction_cmd(req, session_mgr, interaction_hub).await,
        "policy" => handle_policy_cmd(req, policy_engine, interaction_hub).await,
        "audit" => handle_audit_cmd(req, policy_engine, interaction_hub).await,
        "events" => {
            if req.cmd == "events.query" {
                let sid = req.params["session_id"].as_str().map(Id::from);
                let limit = req.params["limit"].as_u64();
                match session_mgr.query_events(sid, limit).await {
                    Ok(events) => ApiResponse::ok(&req.id, json!({ "events": events })),
                    Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                }
            } else if req.cmd == "events.subscribe" {
                ApiResponse::ok(&req.id, json!({}))
            } else {
                ApiResponse::err(&req.id, "NotFound", format!("Unknown command: {}", req.cmd))
            }
        }
        "daemon" => ApiResponse::ok(
            &req.id,
            json!({ "version": env!("CARGO_PKG_VERSION"), "status": "running" }),
        ),
        _ => ApiResponse::err(&req.id, "NotFound", format!("Unknown command: {}", req.cmd)),
    }
}

async fn handle_connection(
    stream: UnixStream,
    session_mgr: SessionManagerHandle,
    account_mgr: Option<AccountManagerHandle>,
    project_registry: Option<ProjectRegistryHandle>,
    interaction_hub: Option<InteractionHubHandle>,
    policy_engine: Option<PolicyEngineHandle>,
    event_tx: broadcast::Sender<AgentEvent>,
) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let req: ApiRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = ApiResponse::err("?", "ValidationError", format!("Invalid JSON: {e}"));
                send_line(&mut write_half, &resp).await?;
                continue;
            }
        };

        if req.v != 1 {
            let resp = ApiResponse::err(
                &req.id,
                "VersionMismatch",
                format!("Unsupported schema version: {}", req.v),
            );
            send_line(&mut write_half, &resp).await?;
            continue;
        }

        if req.cmd == "events.subscribe" {
            let filter: SubscriptionFilter = if let Some(f) = req.params.get("filter") {
                serde_json::from_value(f.clone()).unwrap_or_default()
            } else {
                serde_json::from_value(req.params.clone()).unwrap_or_default()
            };

            // Acknowledge subscription
            send_line(&mut write_half, &ApiResponse::ok(&req.id, json!({}))).await?;

            // Historical catch-up replay if since_seq is provided
            if let Some(since) = filter.since_seq {
                if let Ok(historical) = session_mgr.query_events_since(since, filter.session_id.clone(), None).await {
                    for event in historical {
                        if filter.matches(&event) {
                            let line = serde_json::to_string(&json!({"v": 1, "event": event}))?;
                            if write_half.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                }
            }

            // Live event streaming loop
            let mut rx = event_tx.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if filter.matches(&event) {
                            let line = serde_json::to_string(&json!({"v": 1, "event": event}))?;
                            if write_half.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("IPC subscriber lagged by {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            return Ok(());
        }

        let resp = dispatch_request(
            &req,
            &session_mgr,
            &account_mgr,
            &project_registry,
            &interaction_hub,
            &policy_engine,
            None,
        )
        .await;

        send_line(&mut write_half, &resp).await?;
    }
    Ok(())
}

// ── Session commands ──────────────────────────────────────────────────────────

pub async fn handle_session_cmd(
    req: &ApiRequest,
    mgr: &SessionManagerHandle,
) -> ApiResponse {
    match req.cmd.as_str() {
        "session.create" => {
            let task = req.params["task_description"].as_str().unwrap_or("").to_owned();
            let agent_type = req.params["agent_type"].as_str().unwrap_or("agy").to_owned();
            let project_id = req.params["project_id"].as_str().map(Id::from);
            let account_id = req.params["account_id"].as_str().map(Id::from);
            match mgr.create_with_context(task, agent_type, project_id, account_id).await {
                Ok(id) => ApiResponse::ok(&req.id, json!({"session_id": id})),
                Err(e) => ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
            }
        }
        "session.start" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            match mgr.start(sid).await {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "session.create_and_start" => {
            let task = req.params["task_description"].as_str().unwrap_or("").to_owned();
            let agent_type = req.params["agent_type"].as_str().unwrap_or("agy").to_owned();
            let project_id = req.params["project_id"].as_str().map(Id::from);
            let account_id = req.params["account_id"].as_str().map(Id::from);
            let id = match mgr.create_with_context(task, agent_type, project_id, account_id).await {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
            };
            match mgr.start(id.clone()).await {
                Ok(()) => ApiResponse::ok(&req.id, json!({"session_id": id})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "session.pause" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            match mgr.pause(sid).await {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "session.resume" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            match mgr.resume(sid).await {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "session.stop" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let reason = req.params["reason"].as_str().map(|s| s.to_owned());
            match mgr.stop(sid, reason).await {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "session.list" => {
            match mgr.list().await {
                Ok(sessions) => ApiResponse::ok(&req.id, json!({"sessions": sessions})),
                Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
            }
        }
        "session.get" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            match mgr.get(sid).await {
                Ok(Some(s)) => match serde_json::to_value(&s) {
                    Ok(v) => ApiResponse::ok(&req.id, v),
                    Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                },
                Ok(None) => ApiResponse::err(&req.id, "NotFound", "Session not found"),
                Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
            }
        }
        "session.steer" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let message = req.params["message"].as_str().unwrap_or("").to_owned();
            match mgr.steer(sid, message).await {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "session.select_account" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let aid = match get_id(&req.params, "account_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            match mgr.select_account(sid, aid).await {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
            }
        }
        "session.switch_account" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let target_aid = match get_id(&req.params, "target_account_id")
                .or_else(|_| get_id(&req.params, "account_id")) {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            match mgr.switch_account(sid, target_aid).await {
                Ok(active_id) => ApiResponse::ok(&req.id, json!({
                    "active_session_id": active_id,
                    "session_id": active_id,
                })),
                Err(e) => ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
            }
        }
        "session.snapshot" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            match mgr.snapshot(sid).await {
                Ok(snapshot) => ApiResponse::ok(&req.id, json!({
                    "snapshot": snapshot,
                })),
                Err(e) => ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
            }
        }
        "session.handoff" => {
            let sid = match get_id(&req.params, "session_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let target_aid = req.params["target_account_id"].as_str().map(Id::from)
                .or_else(|| req.params["account_id"].as_str().map(Id::from));
            match mgr.handoff(sid, target_aid).await {
                Ok(successor_id) => ApiResponse::ok(&req.id, json!({
                    "successor_session_id": successor_id,
                    "session_id": successor_id,
                })),
                Err(e) => ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
            }
        }
        other => ApiResponse::err(&req.id, "NotFound", format!("Unknown command: {other}")),
    }
}

// ── Account commands ──────────────────────────────────────────────────────────

pub async fn handle_account_cmd(
    req: &ApiRequest,
    mgr_opt: &Option<AccountManagerHandle>,
    session_mgr: &SessionManagerHandle,
) -> ApiResponse {
    let mgr_handle = match mgr_opt {
        Some(h) => h,
        None => return ApiResponse::err(&req.id, "InternalError", "Account manager not available"),
    };

    if req.cmd == "account.switch" {
        let sid = match get_id(&req.params, "session_id") {
            Ok(id) => id,
            Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
        };
        let target_aid = match get_id(&req.params, "target_account_id")
            .or_else(|_| get_id(&req.params, "account_id")) {
            Ok(id) => id,
            Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
        };
        return match session_mgr.switch_account(sid, target_aid).await {
            Ok(active_id) => ApiResponse::ok(&req.id, json!({"active_session_id": active_id, "session_id": active_id})),
            Err(e) => ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
        };
    }

    match req.cmd.as_str() {
        "account.register" => {
            let label = req.params["label"].as_str().unwrap_or("").to_owned();
            let provider = req.params["provider"].as_str().unwrap_or("").to_owned();
            let agent_types: Vec<String> = req.params["agent_types"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let credential_ref = req.params["credential_ref"].as_str().unwrap_or("").to_owned();
            let concurrency_cap = req.params["concurrency_cap"].as_u64().unwrap_or(2) as u8;
            let tags: Vec<String> = req.params["tags"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let account = Account::new(label, provider, agent_types, credential_ref, concurrency_cap, tags);
            match mgr_handle.0.lock().unwrap().register(account) {
                Ok(id) => ApiResponse::ok(&req.id, json!({"account_id": id})),
                Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
            }
        }
        "account.list" => {
            let list: Vec<_> = {
                let mgr = mgr_handle.0.lock().unwrap();
                mgr.list(None, &[]).into_iter().cloned().collect()
            };
            ApiResponse::ok(&req.id, json!({"accounts": list}))
        }
        "account.availability" | "account.query_availability" => {
            let agent_type = req.params["agent_type"].as_str();
            let tags: Vec<String> = req.params["tags"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let list = {
                let mgr = mgr_handle.0.lock().unwrap();
                mgr.query_availability(agent_type, &tags)
            };
            ApiResponse::ok(&req.id, json!({"accounts": list}))
        }
        "account.get" => {
            let aid = match get_id(&req.params, "account_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let result = {
                let mgr = mgr_handle.0.lock().unwrap();
                mgr.get(&aid).and_then(|a| serde_json::to_value(a).ok())
            };
            match result {
                Some(v) => ApiResponse::ok(&req.id, v),
                None => ApiResponse::err(&req.id, "NotFound", "Account not found"),
            }
        }
        "account.disable" => {
            let aid = match get_id(&req.params, "account_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let result = mgr_handle.0.lock().unwrap().disable(&aid);
            match result {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "account.enable" => {
            let aid = match get_id(&req.params, "account_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let result = mgr_handle.0.lock().unwrap().enable(&aid);
            match result {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "account.remove" => {
            let aid = match get_id(&req.params, "account_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let result = mgr_handle.0.lock().unwrap().remove(&aid);
            match result {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        other => ApiResponse::err(&req.id, "NotFound", format!("Unknown command: {other}")),
    }
}

// ── Project commands ──────────────────────────────────────────────────────────

pub async fn handle_project_cmd(
    req: &ApiRequest,
    reg_opt: &Option<ProjectRegistryHandle>,
) -> ApiResponse {
    let reg_handle = match reg_opt {
        Some(h) => h,
        None => return ApiResponse::err(&req.id, "InternalError", "Project registry not available"),
    };

    match req.cmd.as_str() {
        "project.register" => {
            let name = req.params["name"].as_str().unwrap_or("").to_owned();
            let repo_path = req.params["repo_path"].as_str().unwrap_or("").to_owned();
            let default_agent_type = req.params["default_agent_type"].as_str().map(String::from);
            let default_account_tags: Vec<String> = req.params["default_account_tags"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let workspace_policy = match req.params["workspace_policy"].as_str() {
                Some("worktree_per_session") => WorkspacePolicy::WorktreePerSession,
                _ => WorkspacePolicy::Shared,
            };
            let project = Project::new(name, repo_path, default_agent_type, default_account_tags, workspace_policy);
            match reg_handle.0.lock().unwrap().register(project) {
                Ok(id) => ApiResponse::ok(&req.id, json!({"project_id": id})),
                Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
            }
        }
        "project.list" => {
            let list: Vec<_> = {
                let reg = reg_handle.0.lock().unwrap();
                reg.list().into_iter().cloned().collect()
            };
            ApiResponse::ok(&req.id, json!({"projects": list}))
        }
        "project.get" => {
            let pid = match get_id(&req.params, "project_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let result = {
                let reg = reg_handle.0.lock().unwrap();
                reg.get(&pid).and_then(|p| serde_json::to_value(p).ok())
            };
            match result {
                Some(v) => ApiResponse::ok(&req.id, v),
                None => ApiResponse::err(&req.id, "NotFound", "Project not found"),
            }
        }
        "project.remove" => {
            let pid = match get_id(&req.params, "project_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let result = reg_handle.0.lock().unwrap().remove(&pid);
            match result {
                Ok(()) => ApiResponse::ok(&req.id, json!({})),
                Err(e) => ApiResponse::err(&req.id, "InvalidState", e.to_string()),
            }
        }
        "project.workspaces" => {
            let pid = match get_id(&req.params, "project_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let list: Vec<_> = {
                let reg = reg_handle.0.lock().unwrap();
                reg.workspaces_for(&pid).into_iter().cloned().collect()
            };
            ApiResponse::ok(&req.id, json!({"workspaces": list}))
        }
        other => ApiResponse::err(&req.id, "NotFound", format!("Unknown command: {other}")),
    }
}

// ── Interaction commands ────────────────────────────────────────────────────

pub async fn handle_interaction_cmd(
    req: &ApiRequest,
    session_mgr: &SessionManagerHandle,
    hub_opt: &Option<InteractionHubHandle>,
) -> ApiResponse {
    let hub_handle = match hub_opt {
        Some(h) => h,
        None => return ApiResponse::err(&req.id, "InternalError", "Interaction hub not available"),
    };

    match req.cmd.as_str() {
        "interaction.list_pending" => {
            let session_id = req.params["session_id"].as_str().map(Id::from);
            let list = {
                let hub = hub_handle.0.lock().unwrap();
                let pending = hub.list_pending(session_id.as_ref());
                serde_json::to_value(&pending).unwrap_or(json!([]))
            };
            ApiResponse::ok(&req.id, json!({ "interactions": list }))
        }
        "interaction.list" => {
            let session_id = req.params["session_id"].as_str().map(Id::from);
            let state_filter: Option<InteractionState> = req.params["state"]
                .as_str()
                .and_then(|s| serde_json::from_value(json!(s)).ok());
            let list = {
                let hub = hub_handle.0.lock().unwrap();
                let items = hub.list(session_id.as_ref(), state_filter.as_ref());
                serde_json::to_value(&items).unwrap_or(json!([]))
            };
            ApiResponse::ok(&req.id, json!({ "interactions": list }))
        }
        "interaction.get" => {
            let iid = match get_id(&req.params, "interaction_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let found = {
                let hub = hub_handle.0.lock().unwrap();
                hub.get(&iid).cloned()
            };
            match found {
                Some(i) => match serde_json::to_value(i) {
                    Ok(v) => ApiResponse::ok(&req.id, v),
                    Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                },
                None => ApiResponse::err(&req.id, "NotFound", "Interaction not found"),
            }
        }
        "interaction.reply" | "interaction.resolve" => {
            let iid = match get_id(&req.params, "interaction_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let decision: Option<PolicyDecision> = req.params["decision"]
                .as_str()
                .and_then(|d| match d {
                    "allow" => Some(PolicyDecision::Allow),
                    "deny" => Some(PolicyDecision::Deny),
                    "require_human" => Some(PolicyDecision::RequireHuman),
                    _ => serde_json::from_str(d).ok(),
                });
            let response = req.params["response"].as_str().map(|s| s.to_owned());
            let actor = req.params["actor"].as_str().map(|s| s.to_owned());

            match session_mgr.respond_interaction(iid, decision, response, actor).await {
                Ok(interaction) => match serde_json::to_value(interaction) {
                    Ok(v) => ApiResponse::ok(&req.id, v),
                    Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                },
                Err(e) => ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
            }
        }
        "interaction.dismiss" => {
            let iid = match get_id(&req.params, "interaction_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let actor = req.params["actor"].as_str().map(|s| s.to_owned());

            match session_mgr.dismiss_interaction(iid, actor).await {
                Ok(interaction) => match serde_json::to_value(interaction) {
                    Ok(v) => ApiResponse::ok(&req.id, v),
                    Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                },
                Err(e) => ApiResponse::err(&req.id, classify_error(&e), e.to_string()),
            }
        }
        other => ApiResponse::err(&req.id, "NotFound", format!("Unknown command: {other}")),
    }
}

// ── Policy & Audit helpers ──────────────────────────────────────────────────

fn with_engine_mut<R>(
    engine_opt: &Option<PolicyEngineHandle>,
    hub_opt: &Option<InteractionHubHandle>,
    f: impl FnOnce(&mut crate::policy_engine::PolicyEngine) -> R,
) -> Option<R> {
    if let Some(h) = engine_opt {
        let mut g = h.0.lock().unwrap();
        Some(f(&mut g))
    } else if let Some(h) = hub_opt {
        let mut g = h.0.lock().unwrap();
        Some(f(g.policy_engine_mut()))
    } else {
        None
    }
}

fn with_engine<R>(
    engine_opt: &Option<PolicyEngineHandle>,
    hub_opt: &Option<InteractionHubHandle>,
    f: impl FnOnce(&crate::policy_engine::PolicyEngine) -> R,
) -> Option<R> {
    if let Some(h) = engine_opt {
        let g = h.0.lock().unwrap();
        Some(f(&g))
    } else if let Some(h) = hub_opt {
        let g = h.0.lock().unwrap();
        Some(f(g.policy_engine()))
    } else {
        None
    }
}

// ── Policy commands ─────────────────────────────────────────────────────────

pub async fn handle_policy_cmd(
    req: &ApiRequest,
    engine_opt: &Option<PolicyEngineHandle>,
    hub_opt: &Option<InteractionHubHandle>,
) -> ApiResponse {
    match req.cmd.as_str() {
        "policy.list" => {
            let scope_filter: Option<PolicyScope> = req.params["scope"].as_str().and_then(|s| {
                if s == "global" {
                    Some(PolicyScope::Global)
                } else if let Some(proj_id) = s.strip_prefix("project:") {
                    Some(PolicyScope::Project(Id::from(proj_id)))
                } else {
                    None
                }
            });
            let result = with_engine(engine_opt, hub_opt, |engine| {
                engine
                    .list_policies(scope_filter.as_ref())
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            });
            match result {
                Some(policies) => ApiResponse::ok(&req.id, json!({ "policies": policies })),
                None => ApiResponse::err(&req.id, "InternalError", "Policy engine not available"),
            }
        }
        "policy.get" => {
            let pid = match get_id(&req.params, "policy_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let result = with_engine(engine_opt, hub_opt, |engine| {
                engine.get_policy(&pid).cloned()
            });
            match result {
                Some(Some(p)) => match serde_json::to_value(p) {
                    Ok(v) => ApiResponse::ok(&req.id, v),
                    Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                },
                Some(None) => ApiResponse::err(&req.id, "NotFound", "Policy not found"),
                None => ApiResponse::err(&req.id, "InternalError", "Policy engine not available"),
            }
        }
        "policy.upsert" | "policy.create" | "policy.update" => {
            let policy_id_opt = req.params["policy_id"].as_str().map(Id::from);

            let result = with_engine_mut(engine_opt, hub_opt, |engine| {
                if let Some(pid) = policy_id_opt {
                    if engine.get_policy(&pid).is_some() {
                        // Update existing
                        let name = req.params["name"].as_str().map(|s| s.to_owned());
                        let priority = req.params["priority"].as_i64().map(|p| p as i32);
                        let conditions: Option<Vec<PolicyCondition>> = req.params["conditions"]
                            .as_array()
                            .and_then(|arr| serde_json::from_value(json!(arr)).ok());
                        let decision: Option<PolicyDecision> = req.params["decision"]
                            .as_str()
                            .and_then(|d| match d {
                                "allow" => Some(PolicyDecision::Allow),
                                "deny" => Some(PolicyDecision::Deny),
                                "require_human" => Some(PolicyDecision::RequireHuman),
                                _ => serde_json::from_str(d).ok(),
                            })
                            .or_else(|| {
                                req.params["outcome"].as_str().and_then(|d| match d {
                                    "allow" => Some(PolicyDecision::Allow),
                                    "deny" => Some(PolicyDecision::Deny),
                                    "require_human" => Some(PolicyDecision::RequireHuman),
                                    _ => serde_json::from_str(d).ok(),
                                })
                            });
                        let enabled = req.params["enabled"].as_bool();

                        match engine.update_policy(&pid, name, priority, conditions, decision, enabled) {
                            Ok(()) => {
                                let updated = engine.get_policy(&pid).cloned().unwrap();
                                ApiResponse::ok(&req.id, serde_json::to_value(updated).unwrap())
                            }
                            Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                        }
                    } else {
                        // Insert with explicit ID
                        let name = req.params["name"].as_str().unwrap_or("unnamed").to_owned();
                        let scope: PolicyScope = req.params["scope"]
                            .as_str()
                            .and_then(|s| {
                                if s == "global" {
                                    Some(PolicyScope::Global)
                                } else if let Some(proj_id) = s.strip_prefix("project:") {
                                    Some(PolicyScope::Project(Id::from(proj_id)))
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(PolicyScope::Global);
                        let priority = req.params["priority"].as_i64().unwrap_or(0) as i32;
                        let conditions: Vec<PolicyCondition> = req.params["conditions"]
                            .as_array()
                            .and_then(|arr| serde_json::from_value(json!(arr)).ok())
                            .unwrap_or_default();
                        let decision: PolicyDecision = req.params["decision"]
                            .as_str()
                            .and_then(|d| match d {
                                "allow" => Some(PolicyDecision::Allow),
                                "deny" => Some(PolicyDecision::Deny),
                                "require_human" => Some(PolicyDecision::RequireHuman),
                                _ => serde_json::from_str(d).ok(),
                            })
                            .or_else(|| {
                                req.params["outcome"].as_str().and_then(|d| match d {
                                    "allow" => Some(PolicyDecision::Allow),
                                    "deny" => Some(PolicyDecision::Deny),
                                    "require_human" => Some(PolicyDecision::RequireHuman),
                                    _ => serde_json::from_str(d).ok(),
                                })
                            })
                            .unwrap_or(PolicyDecision::RequireHuman);

                        let mut policy = Policy::new(name, scope, priority, conditions, decision);
                        policy.id = pid.clone();
                        if let Some(en) = req.params["enabled"].as_bool() {
                            policy.enabled = en;
                        }
                        match engine.create_policy(policy) {
                            Ok(id) => ApiResponse::ok(&req.id, json!({ "policy_id": id })),
                            Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                        }
                    }
                } else {
                    // Create new with generated ID
                    let name = req.params["name"].as_str().unwrap_or("unnamed").to_owned();
                    let scope: PolicyScope = req.params["scope"]
                        .as_str()
                        .and_then(|s| {
                            if s == "global" {
                                Some(PolicyScope::Global)
                            } else if let Some(proj_id) = s.strip_prefix("project:") {
                                Some(PolicyScope::Project(Id::from(proj_id)))
                            } else {
                                None
                            }
                        })
                        .unwrap_or(PolicyScope::Global);
                    let priority = req.params["priority"].as_i64().unwrap_or(0) as i32;
                    let conditions: Vec<PolicyCondition> = req.params["conditions"]
                        .as_array()
                        .and_then(|arr| serde_json::from_value(json!(arr)).ok())
                        .unwrap_or_default();
                    let decision: PolicyDecision = req.params["decision"]
                        .as_str()
                        .and_then(|d| match d {
                            "allow" => Some(PolicyDecision::Allow),
                            "deny" => Some(PolicyDecision::Deny),
                            "require_human" => Some(PolicyDecision::RequireHuman),
                            _ => serde_json::from_str(d).ok(),
                        })
                        .or_else(|| {
                            req.params["outcome"].as_str().and_then(|d| match d {
                                "allow" => Some(PolicyDecision::Allow),
                                "deny" => Some(PolicyDecision::Deny),
                                "require_human" => Some(PolicyDecision::RequireHuman),
                                _ => serde_json::from_str(d).ok(),
                            })
                        })
                        .unwrap_or(PolicyDecision::RequireHuman);

                    let mut policy = Policy::new(name, scope, priority, conditions, decision);
                    if let Some(en) = req.params["enabled"].as_bool() {
                        policy.enabled = en;
                    }
                    match engine.create_policy(policy) {
                        Ok(id) => ApiResponse::ok(&req.id, json!({ "policy_id": id })),
                        Err(e) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                    }
                }
            });

            match result {
                Some(resp) => resp,
                None => ApiResponse::err(&req.id, "InternalError", "Policy engine not available"),
            }
        }
        "policy.remove" | "policy.delete" => {
            let pid = match get_id(&req.params, "policy_id") {
                Ok(id) => id,
                Err(e) => return ApiResponse::err(&req.id, "ValidationError", e.to_string()),
            };
            let result = with_engine_mut(engine_opt, hub_opt, |engine| engine.remove_policy(&pid));
            match result {
                Some(Ok(())) => ApiResponse::ok(&req.id, json!({})),
                Some(Err(e)) => ApiResponse::err(&req.id, "NotFound", e.to_string()),
                None => ApiResponse::err(&req.id, "InternalError", "Policy engine not available"),
            }
        }
        "policy.test" => {
            let tool_name = req.params["tool_name"].as_str();
            let agent_type = req.params["agent_type"].as_str();
            let project_id = req.params["project_id"].as_str().map(Id::from);

            let result = with_engine(engine_opt, hub_opt, |engine| {
                engine.test_evaluate(tool_name, agent_type, project_id.as_ref())
            });
            match result {
                Some(eval) => ApiResponse::ok(
                    &req.id,
                    json!({
                        "decision": eval.decision,
                        "matched_policy": eval.matched_policy,
                        "reason": eval.reason,
                    }),
                ),
                None => ApiResponse::err(&req.id, "InternalError", "Policy engine not available"),
            }
        }
        other => ApiResponse::err(&req.id, "NotFound", format!("Unknown command: {other}")),
    }
}

// ── Audit commands ──────────────────────────────────────────────────────────

pub async fn handle_audit_cmd(
    req: &ApiRequest,
    engine_opt: &Option<PolicyEngineHandle>,
    hub_opt: &Option<InteractionHubHandle>,
) -> ApiResponse {
    match req.cmd.as_str() {
        "audit.list" | "audit.query" => {
            let session_id = req.params["session_id"].as_str().map(Id::from);
            let limit = req.params["limit"].as_u64();
            let result = with_engine(engine_opt, hub_opt, |engine| {
                engine.query_audit(session_id.as_ref(), limit)
            });
            match result {
                Some(Ok(entries)) => ApiResponse::ok(&req.id, json!({ "audit_entries": entries })),
                Some(Err(e)) => ApiResponse::err(&req.id, "InternalError", e.to_string()),
                None => ApiResponse::err(&req.id, "InternalError", "Policy engine not available"),
            }
        }
        other => ApiResponse::err(&req.id, "NotFound", format!("Unknown command: {other}")),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn send_line(
    write: &mut tokio::net::unix::OwnedWriteHalf,
    resp: &ApiResponse,
) -> Result<()> {
    let line = serde_json::to_string(resp)?;
    write.write_all(format!("{line}\n").as_bytes()).await?;
    Ok(())
}

fn get_id(params: &serde_json::Value, field: &str) -> Result<Id> {
    params[field]
        .as_str()
        .map(Id::from)
        .ok_or_else(|| anyhow::anyhow!("Missing required param: {field}"))
}

/// Map an error message to a standard error code.
fn classify_error(e: &anyhow::Error) -> &'static str {
    let msg = e.to_string();
    if msg.contains("AccountNotFound") {
        "AccountNotFound"
    } else if msg.contains("IncompatibleAccount") {
        "IncompatibleAccount"
    } else if msg.contains("AccountUnavailable") {
        "AccountUnavailable"
    } else if msg.contains("ConcurrencyLimitReached") {
        "ConcurrencyLimitReached"
    } else if msg.contains("NoAccountAvailable") {
        "NoAccountAvailable"
    } else if msg.contains("UnsupportedCapability") {
        "UnsupportedCapability"
    } else if msg.contains("SnapshotFailed") {
        "SnapshotFailed"
    } else if msg.contains("HandOffFailed") {
        "HandOffFailed"
    } else if msg.contains("StaleSession") {
        "StaleSession"
    } else if msg.contains("not found") || msg.contains("Not found") {
        "NotFound"
    } else if msg.contains("not available") || msg.contains("InvalidState") {
        "InvalidState"
    } else {
        "InternalError"
    }
}
