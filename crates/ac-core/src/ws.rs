//! Loopback token-protected WebSocket server (Phase 7 / ADR-009).
//!
//! Provides the Control API v1 over WebSocket for external tools (e.g. AgentDesk, AgentMesh).
//! Enforces token authentication and scopes (`read` vs `write`/`admin`).

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, RwLock},
};

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::broadcast,
};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
        Message,
    },
};
use tracing::{debug, error, info};

use crate::{
    account_manager::AccountManagerHandle,
    interaction_hub::InteractionHubHandle,
    ipc::dispatch_request,
    policy_engine::PolicyEngineHandle,
    project_registry::ProjectRegistryHandle,
    session::manager::SessionManagerHandle,
    types::{AgentEvent, ApiRequest, ApiResponse, SubscriptionFilter, TokenScope},
};

/// Thread-safe registry of valid authentication tokens and their assigned scopes.
#[derive(Clone, Default)]
pub struct TokenRegistry {
    tokens: Arc<RwLock<HashMap<String, TokenScope>>>,
}

impl TokenRegistry {
    /// Create an empty token registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder helper to register a token with a scope.
    pub fn with_token(self, token: impl Into<String>, scope: TokenScope) -> Self {
        self.tokens.write().unwrap().insert(token.into(), scope);
        self
    }

    /// Insert or replace a token mapping.
    pub fn insert(&self, token: impl Into<String>, scope: TokenScope) {
        self.tokens.write().unwrap().insert(token.into(), scope);
    }

    /// Validate a token, returning its scope if valid.
    pub fn validate(&self, token: &str) -> Option<TokenScope> {
        self.tokens.read().unwrap().get(token).copied()
    }

    /// Number of registered tokens.
    pub fn len(&self) -> usize {
        self.tokens.read().unwrap().len()
    }

    /// Returns true if no tokens are configured.
    pub fn is_empty(&self) -> bool {
        self.tokens.read().unwrap().is_empty()
    }
}

/// WebSocket server for Control API v1 external integrations.
pub struct WsServer {
    listener: TcpListener,
    session_mgr: SessionManagerHandle,
    account_mgr: Option<AccountManagerHandle>,
    project_registry: Option<ProjectRegistryHandle>,
    interaction_hub: Option<InteractionHubHandle>,
    policy_engine: Option<PolicyEngineHandle>,
    event_tx: broadcast::Sender<AgentEvent>,
    token_registry: TokenRegistry,
}

impl WsServer {
    /// Bind to a TCP address (e.g. `127.0.0.1:4242` or `127.0.0.1:0`).
    pub async fn bind(
        addr: &str,
        session_mgr: SessionManagerHandle,
        event_tx: broadcast::Sender<AgentEvent>,
        token_registry: TokenRegistry,
    ) -> Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        info!("WebSocket server listening on ws://{}", local_addr);
        Ok(Self {
            listener,
            session_mgr,
            account_mgr: None,
            project_registry: None,
            interaction_hub: None,
            policy_engine: None,
            event_tx,
            token_registry,
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

    /// Returns the local bound socket address.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }

    /// Run the WebSocket connection accept loop.
    pub async fn run(self) {
        loop {
            match self.listener.accept().await {
                Ok((stream, peer_addr)) => {
                    debug!("WebSocket: incoming connection from {}", peer_addr);
                    let mgr = self.session_mgr.clone();
                    let evt_tx = self.event_tx.clone();
                    let acct_mgr = self.account_mgr.clone();
                    let proj_reg = self.project_registry.clone();
                    let hub = self.interaction_hub.clone();
                    let policy = self.policy_engine.clone();
                    let registry = self.token_registry.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_ws_connection(
                            stream, mgr, acct_mgr, proj_reg, hub, policy, evt_tx, registry,
                        )
                        .await
                        {
                            debug!("WebSocket connection closed ({peer_addr}): {e}");
                        }
                    });
                }
                Err(e) => {
                    error!("WebSocket accept error: {e}");
                    break;
                }
            }
        }
    }
}

async fn handle_ws_connection(
    stream: TcpStream,
    session_mgr: SessionManagerHandle,
    account_mgr: Option<AccountManagerHandle>,
    project_registry: Option<ProjectRegistryHandle>,
    interaction_hub: Option<InteractionHubHandle>,
    policy_engine: Option<PolicyEngineHandle>,
    event_tx: broadcast::Sender<AgentEvent>,
    token_registry: TokenRegistry,
) -> Result<()> {
    let auth_scope_cell = Arc::new(std::sync::Mutex::new(None));
    let cell_clone = auth_scope_cell.clone();
    let reg_clone = token_registry.clone();

    // Accept WebSocket handshake with token extraction from HTTP headers / query string
    let ws_stream = match accept_hdr_async(stream, move |req: &Request, resp: Response| {
        // 1. Check Authorization: Bearer <token>
        if let Some(auth_hdr) = req.headers().get("authorization").and_then(|v| v.to_str().ok()) {
            if let Some(token) = auth_hdr
                .strip_prefix("Bearer ")
                .or_else(|| auth_hdr.strip_prefix("bearer "))
            {
                match reg_clone.validate(token.trim()) {
                    Some(scope) => {
                        *cell_clone.lock().unwrap() = Some(scope);
                        return Ok(resp);
                    }
                    None => {
                        let mut err = ErrorResponse::new(Some("Invalid authorization token".into()));
                        *err.status_mut() = StatusCode::UNAUTHORIZED;
                        return Err(err);
                    }
                }
            }
        }

        // 2. Check query string: ?token=<token>
        if let Some(query) = req.uri().query() {
            for param in query.split('&') {
                if let Some(token) = param.strip_prefix("token=") {
                    match reg_clone.validate(token) {
                        Some(scope) => {
                            *cell_clone.lock().unwrap() = Some(scope);
                            return Ok(resp);
                        }
                        None => {
                            let mut err = ErrorResponse::new(Some("Invalid authorization token".into()));
                            *err.status_mut() = StatusCode::UNAUTHORIZED;
                            return Err(err);
                        }
                    }
                }
            }
        }

        // Allow handshake to complete; client can authenticate via in-band "auth" command.
        Ok(resp)
    })
    .await
    {
        Ok(ws) => ws,
        Err(e) => {
            debug!("WebSocket handshake rejected or failed: {e}");
            return Ok(());
        }
    };

    let (mut ws_sink, mut ws_rx) = ws_stream.split();

    let mut current_scope = {
        let guard = auth_scope_cell.lock().unwrap();
        *guard
    };

    // If no tokens are configured at all, grant Admin scope by default.
    if current_scope.is_none() && token_registry.is_empty() {
        current_scope = Some(TokenScope::Admin);
    }

    let mut sub_rx: Option<broadcast::Receiver<AgentEvent>> = None;
    let mut current_filter: Option<SubscriptionFilter> = None;

    loop {
        tokio::select! {
            // Forward subscribed events to WebSocket client
            evt_res = async {
                match &mut sub_rx {
                    Some(rx) => rx.recv().await.ok(),
                    None => futures_util::future::pending().await,
                }
            } => {
                if let Some(event) = evt_res {
                    if let Some(filter) = &current_filter {
                        if filter.matches(&event) {
                            let text = serde_json::to_string(&json!({"v": 1, "event": event}))?;
                            if ws_sink.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }

            // Receive messages from WebSocket client
            msg_opt = ws_rx.next() => {
                let msg = match msg_opt {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        debug!("WebSocket read error: {e}");
                        break;
                    }
                    None => break,
                };

                match msg {
                    Message::Text(text) => {
                        if text.trim().is_empty() {
                            continue;
                        }

                        let req: ApiRequest = match serde_json::from_str(&text) {
                            Ok(r) => r,
                            Err(e) => {
                                let resp = ApiResponse::err("?", "ValidationError", format!("Invalid JSON: {e}"));
                                let line = serde_json::to_string(&resp)?;
                                let _ = ws_sink.send(Message::Text(line.into())).await;
                                continue;
                            }
                        };

                        if req.v != 1 {
                            let resp = ApiResponse::err(
                                &req.id,
                                "VersionMismatch",
                                format!("Unsupported schema version: {}", req.v),
                            );
                            let line = serde_json::to_string(&resp)?;
                            let _ = ws_sink.send(Message::Text(line.into())).await;
                            continue;
                        }

                        // In-band authentication handling
                        if req.cmd == "auth" || req.cmd == "authenticate" {
                            let token = req.params["token"].as_str().unwrap_or("");
                            match token_registry.validate(token) {
                                Some(scope) => {
                                    current_scope = Some(scope);
                                    let resp = ApiResponse::ok(&req.id, json!({ "scope": scope }));
                                    let line = serde_json::to_string(&resp)?;
                                    let _ = ws_sink.send(Message::Text(line.into())).await;
                                }
                                None => {
                                    let resp = ApiResponse::err(&req.id, "Unauthorized", "Invalid authentication token");
                                    let line = serde_json::to_string(&resp)?;
                                    let _ = ws_sink.send(Message::Text(line.into())).await;
                                }
                            }
                            continue;
                        }

                        // Verify that client is authenticated before proceeding
                        if current_scope.is_none() {
                            let resp = ApiResponse::err(&req.id, "Unauthorized", "Authentication required");
                            let line = serde_json::to_string(&resp)?;
                            let _ = ws_sink.send(Message::Text(line.into())).await;
                            continue;
                        }

                        // Subscription command handling
                        if req.cmd == "events.subscribe" {
                            let filter: SubscriptionFilter = if let Some(f) = req.params.get("filter") {
                                serde_json::from_value(f.clone()).unwrap_or_default()
                            } else {
                                serde_json::from_value(req.params.clone()).unwrap_or_default()
                            };

                            let ok_resp = ApiResponse::ok(&req.id, json!({}));
                            let resp_line = serde_json::to_string(&ok_resp)?;
                            if ws_sink.send(Message::Text(resp_line.into())).await.is_err() {
                                break;
                            }

                            // Historical catch-up replay if since_seq is provided
                            if let Some(since) = filter.since_seq {
                                if let Ok(historical) = session_mgr
                                    .query_events_since(since, filter.session_id.clone(), None)
                                    .await
                                {
                                    for event in historical {
                                        if filter.matches(&event) {
                                            let text = serde_json::to_string(&json!({"v": 1, "event": event}))?;
                                            if ws_sink.send(Message::Text(text.into())).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }

                            sub_rx = Some(event_tx.subscribe());
                            current_filter = Some(filter);
                            continue;
                        }

                        if req.cmd == "events.unsubscribe" {
                            sub_rx = None;
                            current_filter = None;
                            let ok_resp = ApiResponse::ok(&req.id, json!({}));
                            let resp_line = serde_json::to_string(&ok_resp)?;
                            let _ = ws_sink.send(Message::Text(resp_line.into())).await;
                            continue;
                        }

                        // Standard request dispatch
                        let resp = dispatch_request(
                            &req,
                            &session_mgr,
                            &account_mgr,
                            &project_registry,
                            &interaction_hub,
                            &policy_engine,
                            current_scope,
                        )
                        .await;

                        let line = serde_json::to_string(&resp)?;
                        if ws_sink.send(Message::Text(line.into())).await.is_err() {
                            break;
                        }
                    }
                    Message::Ping(data) => {
                        if ws_sink.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
