//! Agent Control daemon entry point.
//!
//! Startup sequence:
//!  1. Load configuration.
//!  2. Initialise tracing.
//!  3. Open event store (SQLite).
//!  4. Open account manager and project registry stores.
//!  5. Emit `DaemonStarted` event.
//!  6. Recover in-memory state from event log.
//!  7. Start session manager task.
//!  8. Start IPC server (Unix socket).
//!  9. Wait for SIGTERM / SIGINT and shut down gracefully.

use anyhow::{Context, Result};
use serde_json::json;
use std::path::PathBuf;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

use ac_core::{
    account_manager::{AccountManager, AccountManagerHandle, AccountStore},
    adapter::CompositeAdapterFactory,
    config::Config,
    event_store::EventStore,
    ipc::IpcServer,
    project_registry::{ProjectRegistry, ProjectRegistryHandle, ProjectStore},
    session::manager::{SessionManager, SessionManagerHandle},
    types::{AgentEvent, EventKind},
};

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. Configuration ──────────────────────────────────────────────────
    let config_path = std::env::var("AC_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
                        .join(".config")
                })
                .join("agentcontrol")
                .join("config.toml")
        });

    let cfg = Config::load_or_default(&config_path);

    // ── 2. Tracing ────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("AC_LOG")
                .unwrap_or_else(|_| cfg.log_level.parse().unwrap_or_else(|_| "info".parse().unwrap())),
        )
        .init();

    info!("agentcontrold v{} starting", env!("CARGO_PKG_VERSION"));

    // ── 3. Event store ────────────────────────────────────────────────────
    let mut store = EventStore::open(&cfg.db_path)
        .with_context(|| format!("opening event store at {}", cfg.db_path.display()))?;

    // ── 4. Account manager + project registry ─────────────────────────────
    let accounts_db = cfg.db_path.with_file_name("accounts.db");
    let projects_db = cfg.db_path.with_file_name("projects.db");

    let account_store = AccountStore::open(&accounts_db)
        .with_context(|| format!("opening account store at {}", accounts_db.display()))?;
    let account_mgr = AccountManager::new(account_store)?;
    let acct_handle = AccountManagerHandle::new(account_mgr);

    let project_store = ProjectStore::open(&projects_db)
        .with_context(|| format!("opening project store at {}", projects_db.display()))?;
    let project_reg = ProjectRegistry::new(project_store)?;
    let proj_handle = ProjectRegistryHandle::new(project_reg);

    // ── Phase 3: Policy engine + Interaction hub ──────────────────────────
    let policies_db = cfg.db_path.with_file_name("policies.db");
    let interactions_db = cfg.db_path.with_file_name("interactions.db");

    let policy_store = ac_core::policy_engine::PolicyStore::open(&policies_db)
        .with_context(|| format!("opening policy store at {}", policies_db.display()))?;
    let policy_engine = ac_core::policy_engine::PolicyEngine::new(policy_store)?;

    let interaction_store = ac_core::interaction_hub::InteractionStore::open(&interactions_db)
        .with_context(|| format!("opening interaction store at {}", interactions_db.display()))?;
    let interaction_hub = ac_core::interaction_hub::InteractionHub::new(interaction_store, policy_engine)?;
    let hub_handle = ac_core::interaction_hub::InteractionHubHandle::new(interaction_hub);

    // ── 5. DaemonStarted event ────────────────────────────────────────────
    let mut daemon_started = AgentEvent::new(
        EventKind::DaemonStarted,
        None,
        json!({ "version": env!("CARGO_PKG_VERSION") }),
        "system",
    );
    store.append(&mut daemon_started)?;

    // ── 6. Session manager ────────────────────────────────────────────────
    let (event_tx, _) = broadcast::channel::<AgentEvent>(1024);
    let (cmd_tx, cmd_rx) = mpsc::channel(256);

    // Rebuild account and project managers from their DBs for the session manager
    let account_store2 = AccountStore::open(&accounts_db)?;
    let account_mgr2 = AccountManager::new(account_store2)?;
    let project_store2 = ProjectStore::open(&projects_db)?;
    let project_reg2 = ProjectRegistry::new(project_store2)?;

    let adapter_factory = Box::new(CompositeAdapterFactory::new());

    let mut manager = SessionManager::new(
        store,
        cmd_rx,
        event_tx.clone(),
        cfg.max_restarts,
        adapter_factory,
    )
    .with_account_manager(account_mgr2)
    .with_project_registry(project_reg2)
    .with_interaction_hub(hub_handle.clone());

    manager.recover_from_store()?;

    let mgr_handle = SessionManagerHandle::new(cmd_tx);
    tokio::spawn(async move { manager.run().await });

    // ── 7. IPC server ─────────────────────────────────────────────────────
    let server = IpcServer::bind(&cfg.socket_path, mgr_handle, event_tx.clone())
        .context("binding IPC socket")?
        .with_account_manager(acct_handle)
        .with_project_registry(proj_handle)
        .with_interaction_hub(hub_handle);

    tokio::spawn(async move { server.run().await });

    info!("Daemon ready. Listening on {}", cfg.socket_path.display());

    // ── 8. Shutdown signal ────────────────────────────────────────────────
    match tokio::signal::ctrl_c().await {
        Ok(()) => info!("Received SIGINT, shutting down"),
        Err(e) => warn!("Signal error: {e}"),
    }

    info!("agentcontrold stopped");
    Ok(())
}
