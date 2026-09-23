//! API Client for communicating with the Agent Control daemon over Unix socket.

use std::path::{Path, PathBuf};
use serde_json::json;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::mpsc,
};
use tracing::{debug, warn};

use ac_core::types::{
    Account, AccountAvailability, AgentEvent, AgentSession, ApiRequest, ApiResponse, Id,
    Interaction, PolicyDecision, Project, SessionSnapshot,
};

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("Daemon unavailable at {0}: {1}")]
    DaemonUnavailable(PathBuf, String),

    #[error("Connection closed by daemon")]
    ConnectionClosed,

    #[error("API error [{code}]: {message}")]
    ApiError { code: String, message: String },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct ApiClient {
    socket_path: PathBuf,
}

impl ApiClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Check if the daemon is currently reachable.
    pub async fn check_daemon(&self) -> bool {
        UnixStream::connect(&self.socket_path).await.is_ok()
    }

    /// Send an API command and return the parsed `ApiResponse`.
    pub async fn send_request(
        &self,
        cmd: &str,
        params: serde_json::Value,
    ) -> Result<ApiResponse, ClientError> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| ClientError::DaemonUnavailable(self.socket_path.clone(), e.to_string()))?;

        let (read_half, mut write_half) = stream.into_split();

        let req = ApiRequest {
            v: 1,
            id: format!("tui-{}", ulid::Ulid::new()),
            cmd: cmd.to_owned(),
            params,
        };

        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(b'\n');
        write_half.write_all(&req_bytes).await?;

        let mut lines = BufReader::new(read_half).lines();
        let line = lines
            .next_line()
            .await?
            .ok_or(ClientError::ConnectionClosed)?;

        let resp: ApiResponse = serde_json::from_str(&line)?;
        if !resp.ok {
            if let Some(err) = resp.error {
                return Err(ClientError::ApiError {
                    code: err.code,
                    message: err.message,
                });
            }
        }
        Ok(resp)
    }

    /// Subscribe to live daemon events via streaming connection.
    /// Returns a receiver for incoming `AgentEvent`s.
    pub async fn subscribe_events(&self) -> Result<mpsc::Receiver<AgentEvent>, ClientError> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| ClientError::DaemonUnavailable(self.socket_path.clone(), e.to_string()))?;

        let (read_half, mut write_half) = stream.into_split();

        let req = ApiRequest {
            v: 1,
            id: format!("tui-sub-{}", ulid::Ulid::new()),
            cmd: "events.subscribe".to_owned(),
            params: json!({}),
        };

        let mut req_bytes = serde_json::to_vec(&req)?;
        req_bytes.push(b'\n');
        write_half.write_all(&req_bytes).await?;

        let (tx, rx) = mpsc::channel(512);

        tokio::spawn(async move {
            let mut lines = BufReader::new(read_half).lines();
            // First line is subscription ack
            if let Ok(Some(first_line)) = lines.next_line().await {
                debug!("Subscription ack received: {}", first_line);
            }

            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                    if let Some(event_val) = val.get("event") {
                        if let Ok(event) = serde_json::from_value::<AgentEvent>(event_val.clone()) {
                            if tx.send(event).await.is_err() {
                                break;
                            }
                        } else {
                            warn!("Failed to deserialize event: {}", line);
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    // ── Session APIs ────────────────────────────────────────────────────────

    pub async fn list_sessions(&self) -> Result<Vec<AgentSession>, ClientError> {
        let resp = self.send_request("session.list", json!({})).await?;
        let sessions = resp
            .result
            .and_then(|r| r.get("sessions").cloned())
            .map(serde_json::from_value::<Vec<AgentSession>>)
            .transpose()?
            .unwrap_or_default();
        Ok(sessions)
    }

    pub async fn get_session(&self, id: &Id) -> Result<Option<AgentSession>, ClientError> {
        let resp = self
            .send_request("session.get", json!({ "session_id": id }))
            .await?;
        if let Some(r) = resp.result {
            let session = serde_json::from_value::<AgentSession>(r)?;
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    pub async fn create_session(
        &self,
        task: &str,
        agent_type: &str,
        project_id: Option<&Id>,
        account_id: Option<&Id>,
    ) -> Result<Id, ClientError> {
        let resp = self
            .send_request(
                "session.create",
                json!({
                    "task_description": task,
                    "agent_type": agent_type,
                    "project_id": project_id,
                    "account_id": account_id,
                }),
            )
            .await?;
        let sid = resp
            .result
            .and_then(|r| r.get("session_id").and_then(|v| v.as_str().map(Id::from)))
            .ok_or_else(|| ClientError::ApiError {
                code: "InvalidResponse".into(),
                message: "Missing session_id in response".into(),
            })?;
        Ok(sid)
    }

    pub async fn start_session(&self, id: &Id) -> Result<(), ClientError> {
        self.send_request("session.start", json!({ "session_id": id }))
            .await?;
        Ok(())
    }

    pub async fn pause_session(&self, id: &Id) -> Result<(), ClientError> {
        self.send_request("session.pause", json!({ "session_id": id }))
            .await?;
        Ok(())
    }

    pub async fn resume_session(&self, id: &Id) -> Result<(), ClientError> {
        self.send_request("session.resume", json!({ "session_id": id }))
            .await?;
        Ok(())
    }

    pub async fn stop_session(&self, id: &Id, reason: Option<&str>) -> Result<(), ClientError> {
        self.send_request(
            "session.stop",
            json!({
                "session_id": id,
                "reason": reason,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn steer_session(&self, id: &Id, message: &str) -> Result<(), ClientError> {
        self.send_request(
            "session.steer",
            json!({
                "session_id": id,
                "message": message,
            }),
        )
        .await?;
        Ok(())
    }

    // ── Interaction APIs ────────────────────────────────────────────────────

    pub async fn list_interactions(
        &self,
        session_id: Option<&Id>,
        pending_only: bool,
    ) -> Result<Vec<Interaction>, ClientError> {
        let resp = self
            .send_request(
                "interaction.list",
                json!({
                    "session_id": session_id,
                    "pending_only": pending_only,
                }),
            )
            .await?;
        let list = resp
            .result
            .and_then(|r| r.get("interactions").cloned())
            .map(serde_json::from_value::<Vec<Interaction>>)
            .transpose()?
            .unwrap_or_default();
        Ok(list)
    }

    pub async fn resolve_interaction(
        &self,
        id: &Id,
        decision: Option<PolicyDecision>,
        response: Option<&str>,
    ) -> Result<Interaction, ClientError> {
        let resp = self
            .send_request(
                "interaction.resolve",
                json!({
                    "interaction_id": id,
                    "decision": decision,
                    "response": response,
                }),
            )
            .await?;
        let interaction = resp
            .result
            .map(|r| {
                let val = r.get("interaction").cloned().unwrap_or(r);
                serde_json::from_value::<Interaction>(val)
            })
            .transpose()?
            .ok_or_else(|| ClientError::ApiError {
                code: "InvalidResponse".into(),
                message: "Missing interaction in resolve response".into(),
            })?;
        Ok(interaction)
    }

    pub async fn dismiss_interaction(&self, id: &Id) -> Result<Interaction, ClientError> {
        let resp = self
            .send_request(
                "interaction.dismiss",
                json!({
                    "interaction_id": id,
                }),
            )
            .await?;
        let interaction = resp
            .result
            .map(|r| {
                let val = r.get("interaction").cloned().unwrap_or(r);
                serde_json::from_value::<Interaction>(val)
            })
            .transpose()?
            .ok_or_else(|| ClientError::ApiError {
                code: "InvalidResponse".into(),
                message: "Missing interaction in dismiss response".into(),
            })?;
        Ok(interaction)
    }

    // ── Account & Project APIs ──────────────────────────────────────────────

    pub async fn list_accounts(&self) -> Result<Vec<Account>, ClientError> {
        let resp = self.send_request("account.list", json!({})).await?;
        let accounts = resp
            .result
            .and_then(|r| r.get("accounts").cloned())
            .map(serde_json::from_value::<Vec<Account>>)
            .transpose()?
            .unwrap_or_default();
        Ok(accounts)
    }

    pub async fn register_account(
        &self,
        label: &str,
        provider: &str,
        agent_types: &[&str],
        credential_ref: &str,
        concurrency_cap: u8,
        tags: &[&str],
    ) -> Result<Id, ClientError> {
        let resp = self
            .send_request(
                "account.register",
                json!({
                    "label": label,
                    "provider": provider,
                    "agent_types": agent_types,
                    "credential_ref": credential_ref,
                    "concurrency_cap": concurrency_cap,
                    "tags": tags,
                }),
            )
            .await?;
        let id_val = resp
            .result
            .as_ref()
            .and_then(|r| r.get("account_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| ClientError::ApiError {
                code: "Protocol".into(),
                message: "missing account_id in response".into(),
            })?;
        Ok(Id::from(id_val))
    }

    pub async fn remove_account(&self, account_id: &Id) -> Result<(), ClientError> {
        self.send_request(
            "account.remove",
            json!({
                "account_id": account_id.0,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn list_projects(&self) -> Result<Vec<Project>, ClientError> {
        let resp = self.send_request("project.list", json!({})).await?;
        let projects = resp
            .result
            .and_then(|r| r.get("projects").cloned())
            .map(serde_json::from_value::<Vec<Project>>)
            .transpose()?
            .unwrap_or_default();
        Ok(projects)
    }

    // ── Event APIs ──────────────────────────────────────────────────────────

    pub async fn query_events(
        &self,
        session_id: Option<&Id>,
        limit: Option<u64>,
    ) -> Result<Vec<AgentEvent>, ClientError> {
        let resp = self
            .send_request(
                "events.query",
                json!({
                    "session_id": session_id,
                    "limit": limit,
                }),
            )
            .await?;
        let events = resp
            .result
            .and_then(|r| r.get("events").cloned())
            .map(serde_json::from_value::<Vec<AgentEvent>>)
            .transpose()?
            .unwrap_or_default();
        Ok(events)
    }

    // ── Phase 6: Account Selection & Switching APIs ──────────────────────────

    pub async fn select_account(&self, session_id: &Id, account_id: &Id) -> Result<(), ClientError> {
        self.send_request(
            "session.select_account",
            json!({
                "session_id": session_id,
                "account_id": account_id,
            }),
        )
        .await?;
        Ok(())
    }

    pub async fn switch_account(&self, session_id: &Id, target_account_id: &Id) -> Result<Id, ClientError> {
        let resp = self
            .send_request(
                "session.switch_account",
                json!({
                    "session_id": session_id,
                    "target_account_id": target_account_id,
                }),
            )
            .await?;
        let active_id = resp
            .result
            .and_then(|r| {
                r.get("active_session_id")
                    .or_else(|| r.get("session_id"))
                    .and_then(|v| v.as_str().map(Id::from))
            })
            .unwrap_or_else(|| session_id.clone());
        Ok(active_id)
    }

    pub async fn get_snapshot(&self, session_id: &Id) -> Result<SessionSnapshot, ClientError> {
        let resp = self
            .send_request(
                "session.snapshot",
                json!({
                    "session_id": session_id,
                }),
            )
            .await?;
        let snapshot = resp
            .result
            .and_then(|r| r.get("snapshot").cloned())
            .map(serde_json::from_value::<SessionSnapshot>)
            .transpose()?
            .ok_or_else(|| ClientError::ApiError {
                code: "InvalidResponse".into(),
                message: "Missing snapshot in response".into(),
            })?;
        Ok(snapshot)
    }

    pub async fn query_account_availability(
        &self,
        agent_type: Option<&str>,
        tags: &[String],
    ) -> Result<Vec<AccountAvailability>, ClientError> {
        let resp = self
            .send_request(
                "account.query_availability",
                json!({
                    "agent_type": agent_type,
                    "tags": tags,
                }),
            )
            .await?;
        let list = resp
            .result
            .and_then(|r| r.get("accounts").cloned())
            .map(serde_json::from_value::<Vec<AccountAvailability>>)
            .transpose()?
            .unwrap_or_default();
        Ok(list)
    }

    pub async fn handoff(
        &self,
        session_id: &Id,
        target_account_id: Option<&Id>,
    ) -> Result<Id, ClientError> {
        let resp = self
            .send_request(
                "session.handoff",
                json!({
                    "session_id": session_id,
                    "target_account_id": target_account_id,
                }),
            )
            .await?;
        let successor_id = resp
            .result
            .and_then(|r| {
                r.get("successor_session_id")
                    .or_else(|| r.get("session_id"))
                    .and_then(|v| v.as_str().map(Id::from))
            })
            .unwrap_or_else(|| session_id.clone());
        Ok(successor_id)
    }
}
