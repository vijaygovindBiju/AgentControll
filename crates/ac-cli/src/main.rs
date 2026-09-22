//! `ac` — Agent Control CLI
//!
//! Communicates with the daemon via the Unix domain socket.
//! Each command sends a JSON request and prints the JSON response.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

use ac_core::types::{ApiRequest, ApiResponse};

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "ac",
    version,
    about = "Agent Control CLI",
    long_about = "Control Agent Control daemon sessions and events from the command line."
)]
struct Cli {
    /// Path to the daemon Unix socket.
    #[arg(
        long,
        env = "AC_SOCKET",
        default_value_os_t = ac_core::config::Config::default().socket_path
    )]
    socket: PathBuf,

    /// Output raw JSON responses (default: pretty-printed).
    #[arg(long)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Session management commands.
    #[command(subcommand)]
    Session(SessionCmd),
    /// Account management commands.
    #[command(subcommand)]
    Account(AccountCmd),
    /// Project management commands.
    #[command(subcommand)]
    Project(ProjectCmd),
    /// Interaction management commands (Phase 3).
    #[command(subcommand)]
    Interaction(InteractionCmd),
    /// Policy management commands (Phase 3).
    #[command(subcommand)]
    Policy(PolicyCmd),
    /// Audit log commands (Phase 3).
    #[command(subcommand)]
    Audit(AuditCmd),
    /// Event log commands.
    #[command(subcommand)]
    Events(EventsCmd),
    /// Daemon status.
    Status,
    /// Launch the interactive Ratatui TUI dashboard.
    Tui,
    /// Launch the interactive Ratatui TUI dashboard (alias for tui).
    Dashboard,
}

#[derive(Subcommand)]
enum SessionCmd {
    /// Create a new session (without starting it).
    Create {
        /// Task description for the agent.
        #[arg(short, long)]
        task: String,
        /// Agent type (default: mock).
        #[arg(short, long, default_value = "mock")]
        agent_type: String,
        /// Optional project ID.
        #[arg(long)]
        project_id: Option<String>,
        /// Optional explicit account ID.
        #[arg(long)]
        account_id: Option<String>,
    },
    /// Start a session (transition Idle → Starting → Working).
    Start {
        /// Session ID.
        session_id: String,
    },
    /// Create and immediately start a session.
    Run {
        /// Task description for the agent.
        #[arg(short, long)]
        task: String,
        /// Agent type (default: mock).
        #[arg(short, long, default_value = "mock")]
        agent_type: String,
        /// Optional project ID.
        #[arg(long)]
        project_id: Option<String>,
        /// Optional explicit account ID.
        #[arg(long)]
        account_id: Option<String>,
    },
    /// Pause a running session.
    Pause {
        /// Session ID.
        session_id: String,
    },
    /// Resume a paused session.
    Resume {
        /// Session ID.
        session_id: String,
    },
    /// Stop a session.
    Stop {
        /// Session ID.
        session_id: String,
        /// Optional reason for stopping.
        #[arg(short, long)]
        reason: Option<String>,
    },
    /// List all sessions.
    List,
    /// Get details for a specific session.
    Get {
        /// Session ID.
        session_id: String,
    },
    /// Steer a running session by injecting a human instruction.
    Steer {
        /// Session ID.
        session_id: String,
        /// Message/instruction to inject.
        #[arg(short, long)]
        message: String,
    },
    /// Explicitly select an account for an Idle session.
    SelectAccount {
        /// Session ID.
        session_id: String,
        /// Account ID.
        account_id: String,
    },
    /// Switch account for a session (Idle rebind or running hand-off/dynamic).
    SwitchAccount {
        /// Session ID.
        session_id: String,
        /// Target account ID.
        account_id: String,
    },
    /// Capture a safe session snapshot.
    Snapshot {
        /// Session ID.
        session_id: String,
    },
    /// Controlled session hand-off to another account.
    Handoff {
        /// Session ID.
        session_id: String,
        /// Optional target account ID.
        #[arg(short, long)]
        account_id: Option<String>,
    },
}

/// Account management subcommands.
#[derive(Subcommand)]
enum AccountCmd {
    /// Register a new provider account.
    Register {
        #[arg(short, long)]
        label: String,
        #[arg(short, long)]
        provider: String,
        /// Comma-separated list of agent types this account supports.
        #[arg(long, default_value = "mock")]
        agent_types: String,
        /// Reference key in the credential store (not a raw secret).
        #[arg(long, default_value = "ref:none")]
        credential_ref: String,
        /// Maximum concurrent sessions.
        #[arg(long, default_value = "2")]
        concurrency_cap: u8,
        /// Comma-separated tags for grouping/filtering.
        #[arg(long, default_value = "")]
        tags: String,
    },
    /// List all accounts.
    List,
    /// Query account availability for an agent type and optional tags.
    Availability {
        /// Agent type (e.g. mock, claude, pty).
        #[arg(short, long)]
        agent_type: Option<String>,
        /// Comma-separated tags.
        #[arg(long, default_value = "")]
        tags: String,
    },
    /// Get details for an account.
    Get { account_id: String },
    /// Disable an account (take it out of rotation).
    Disable { account_id: String },
    /// Re-enable a disabled account.
    Enable { account_id: String },
    /// Remove an account (only if idle).
    Remove { account_id: String },
}

/// Project management subcommands.
#[derive(Subcommand)]
enum ProjectCmd {
    /// Register a new project.
    Register {
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        repo_path: String,
        /// Default agent type for sessions in this project.
        #[arg(long)]
        default_agent_type: Option<String>,
        /// Comma-separated default account tags.
        #[arg(long, default_value = "")]
        default_account_tags: String,
        /// Workspace policy: shared or worktree_per_session.
        #[arg(long, default_value = "shared")]
        workspace_policy: String,
    },
    /// List all projects.
    List,
    /// Get details for a project.
    Get { project_id: String },
    /// Remove a project (only if no active sessions).
    Remove { project_id: String },
    /// List workspaces for a project.
    Workspaces { project_id: String },
}

/// Interaction management subcommands (Phase 3).
#[derive(Subcommand)]
enum InteractionCmd {
    /// List pending interactions awaiting decision or input.
    ListPending {
        /// Optional session ID filter.
        #[arg(short, long)]
        session_id: Option<String>,
    },
    /// List interactions with optional filters.
    List {
        /// Optional session ID filter.
        #[arg(short, long)]
        session_id: Option<String>,
        /// Optional state filter (pending, auto_resolved, human_resolved, dismissed, expired).
        #[arg(long)]
        state: Option<String>,
    },
    /// Get details of a specific interaction.
    Get {
        /// Interaction ID.
        interaction_id: String,
    },
    /// Reply to an interaction (question or approval).
    Reply {
        /// Interaction ID.
        interaction_id: String,
        /// Decision for approval requests: "allow" or "deny".
        #[arg(short, long)]
        decision: Option<String>,
        /// Text response for questions or rationale.
        #[arg(short, long)]
        response: Option<String>,
        /// Actor identity (default: human).
        #[arg(long)]
        actor: Option<String>,
    },
    /// Approve an interaction directly.
    Approve {
        /// Interaction ID.
        interaction_id: String,
        /// Optional response note.
        #[arg(short, long)]
        note: Option<String>,
    },
    /// Deny an interaction directly.
    Deny {
        /// Interaction ID.
        interaction_id: String,
        /// Optional denial reason.
        #[arg(short, long)]
        reason: Option<String>,
    },
    /// Dismiss an interaction without a decision.
    Dismiss {
        /// Interaction ID.
        interaction_id: String,
        /// Actor identity.
        #[arg(long)]
        actor: Option<String>,
    },
}

/// Policy management subcommands (Phase 3).
#[derive(Subcommand)]
enum PolicyCmd {
    /// List policy rules.
    List {
        /// Optional scope filter ("global" or "project:<id>").
        #[arg(short, long)]
        scope: Option<String>,
    },
    /// Get details of a specific policy rule.
    Get {
        /// Policy ID.
        policy_id: String,
    },
    /// Create or update a policy rule.
    Upsert {
        /// Policy ID (if updating existing).
        #[arg(long)]
        policy_id: Option<String>,
        /// Policy rule name.
        #[arg(short, long)]
        name: String,
        /// Scope: "global" or "project:<id>".
        #[arg(short, long, default_value = "global")]
        scope: String,
        /// Priority (higher evaluates first).
        #[arg(short, long, default_value = "0")]
        priority: i32,
        /// Decision: "allow", "deny", or "require_human".
        #[arg(short, long)]
        decision: String,
        /// JSON array of conditions (e.g. '[{"tool_name_equals":"read_file"}]').
        #[arg(short, long, default_value = "[]")]
        conditions: String,
        /// Whether this policy is disabled.
        #[arg(long)]
        disabled: bool,
    },
    /// Remove a policy rule.
    Remove {
        /// Policy ID.
        policy_id: String,
    },
    /// Test policy evaluation without applying it.
    Test {
        /// Tool name to test.
        #[arg(short, long)]
        tool_name: Option<String>,
        /// Agent type to test.
        #[arg(short, long)]
        agent_type: Option<String>,
        /// Project ID to test.
        #[arg(short, long)]
        project_id: Option<String>,
    },
}

/// Audit log subcommands (Phase 3).
#[derive(Subcommand)]
enum AuditCmd {
    /// List audit log entries.
    List {
        /// Optional session ID filter.
        #[arg(short, long)]
        session_id: Option<String>,
        /// Maximum number of entries to return.
        #[arg(short, long, default_value = "50")]
        limit: u64,
    },
}

#[derive(Subcommand)]
enum EventsCmd {
    /// Query events from the event log (not yet implemented in daemon).
    Query {
        /// Filter by session ID.
        #[arg(long)]
        session_id: Option<String>,
        /// Maximum number of events to return.
        #[arg(long, default_value = "50")]
        limit: u64,
    },
    /// Stream live events from the daemon.
    Subscribe,
    /// Verify sequence integrity and continuity of the event store (Phase 8).
    Verify {
        /// Optional path to SQLite database (defaults to daemon config path).
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// Replay and validate event log against the state machine (Phase 8).
    Replay {
        /// Optional path to SQLite database (defaults to daemon config path).
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if matches!(cli.command, Commands::Tui | Commands::Dashboard) {
        return ac_tui::run_tui(cli.socket).await;
    }

    if let Commands::Events(EventsCmd::Verify { ref db }) = cli.command {
        let db_path = match db {
            Some(p) => p.clone(),
            None => ac_core::config::Config::default().db_path,
        };
        let store = ac_core::event_store::EventStore::open(&db_path)?;
        let report = store.verify_integrity()?;
        if cli.json {
            println!("{}", serde_json::to_string(&report)?);
        } else {
            println!("Event Store Integrity Report:");
            println!("  Database:        {}", db_path.display());
            println!("  Schema Version:  {}", report.schema_version);
            println!("  Total Events:    {}", report.total_events);
            println!("  Max Sequence:    {}", report.max_seq);
            println!("  Gaps Detected:   {}", report.gap_count);
            println!("  Status:          {}", if report.is_valid { "VALID" } else { "CORRUPTED" });
        }
        if !report.is_valid {
            std::process::exit(1);
        }
        return Ok(());
    }

    if let Commands::Events(EventsCmd::Replay { ref db }) = cli.command {
        let db_path = match db {
            Some(p) => p.clone(),
            None => ac_core::config::Config::default().db_path,
        };
        let start = std::time::Instant::now();
        let store = ac_core::event_store::EventStore::open(&db_path)?;
        let total_events = store.max_seq()?;
        let (event_tx, _) = tokio::sync::broadcast::channel(16);
        let (_cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(16);
        let adapter_factory = Box::new(ac_core::adapter::CompositeAdapterFactory::new());
        let mut mgr = ac_core::session::manager::SessionManager::new(store, cmd_rx, event_tx, 3, adapter_factory);
        mgr.recover_from_store()?;
        let elapsed = start.elapsed();
        let sessions = mgr.sessions();
        if cli.json {
            println!("{}", serde_json::to_string(&json!({
                "events_replayed": total_events,
                "sessions_reconstructed": sessions.len(),
                "duration_ms": elapsed.as_millis(),
                "success": true
            }))?);
        } else {
            println!("Event Store Replay Report:");
            println!("  Database:               {}", db_path.display());
            println!("  Events Replayed:        {}", total_events);
            println!("  Sessions Reconstructed: {}", sessions.len());
            println!("  Duration:               {:?}", elapsed);
            println!("  Status:                 SUCCESS");
        }
        return Ok(());
    }

    let (cmd, params) = build_request(&cli.command)?;

    if cmd == "events.subscribe" {
        // Streaming: print each event line as it arrives
        subscribe_events(&cli.socket).await?;
        return Ok(());
    }

    let resp = send_command(&cli.socket, &cmd, params).await?;

    if cli.json {
        println!("{}", serde_json::to_string(&resp)?);
    } else {
        print_response(&resp);
    }

    if !resp.ok {
        std::process::exit(1);
    }

    Ok(())
}

fn build_request(commands: &Commands) -> Result<(String, serde_json::Value)> {
    let (cmd, params) = match commands {
        Commands::Status => ("daemon.status".to_owned(), json!({})),
        Commands::Tui | Commands::Dashboard => unreachable!(),

        Commands::Session(s) => match s {
            SessionCmd::Create { task, agent_type, project_id, account_id } => (
                "session.create".to_owned(),
                json!({
                    "task_description": task,
                    "agent_type": agent_type,
                    "project_id": project_id,
                    "account_id": account_id,
                }),
            ),
            SessionCmd::Start { session_id } => (
                "session.start".to_owned(),
                json!({ "session_id": session_id }),
            ),
            SessionCmd::Run { task, agent_type, project_id, account_id } => (
                "session.create_and_start".to_owned(),
                json!({
                    "task_description": task,
                    "agent_type": agent_type,
                    "project_id": project_id,
                    "account_id": account_id,
                }),
            ),
            SessionCmd::Pause { session_id } => (
                "session.pause".to_owned(),
                json!({ "session_id": session_id }),
            ),
            SessionCmd::Resume { session_id } => (
                "session.resume".to_owned(),
                json!({ "session_id": session_id }),
            ),
            SessionCmd::Stop { session_id, reason } => (
                "session.stop".to_owned(),
                json!({ "session_id": session_id, "reason": reason }),
            ),
            SessionCmd::List => ("session.list".to_owned(), json!({})),
            SessionCmd::Get { session_id } => (
                "session.get".to_owned(),
                json!({ "session_id": session_id }),
            ),
            SessionCmd::Steer { session_id, message } => (
                "session.steer".to_owned(),
                json!({ "session_id": session_id, "message": message }),
            ),
            SessionCmd::SelectAccount { session_id, account_id } => (
                "session.select_account".to_owned(),
                json!({ "session_id": session_id, "account_id": account_id }),
            ),
            SessionCmd::SwitchAccount { session_id, account_id } => (
                "session.switch_account".to_owned(),
                json!({ "session_id": session_id, "target_account_id": account_id }),
            ),
            SessionCmd::Snapshot { session_id } => (
                "session.snapshot".to_owned(),
                json!({ "session_id": session_id }),
            ),
            SessionCmd::Handoff { session_id, account_id } => (
                "session.handoff".to_owned(),
                json!({ "session_id": session_id, "target_account_id": account_id }),
            ),
        },

        Commands::Account(a) => match a {
            AccountCmd::Register {
                label, provider, agent_types, credential_ref, concurrency_cap, tags,
            } => {
                let agent_types_vec: Vec<&str> = agent_types.split(',').map(str::trim).collect();
                let tags_vec: Vec<&str> = if tags.is_empty() {
                    vec![]
                } else {
                    tags.split(',').map(str::trim).collect()
                };
                (
                    "account.register".to_owned(),
                    json!({
                        "label": label,
                        "provider": provider,
                        "agent_types": agent_types_vec,
                        "credential_ref": credential_ref,
                        "concurrency_cap": concurrency_cap,
                        "tags": tags_vec,
                    }),
                )
            }
            AccountCmd::List => ("account.list".to_owned(), json!({})),
            AccountCmd::Availability { agent_type, tags } => {
                let tags_vec: Vec<&str> = if tags.is_empty() {
                    vec![]
                } else {
                    tags.split(',').map(str::trim).collect()
                };
                (
                    "account.query_availability".to_owned(),
                    json!({
                        "agent_type": agent_type,
                        "tags": tags_vec,
                    }),
                )
            }
            AccountCmd::Get { account_id } => (
                "account.get".to_owned(),
                json!({ "account_id": account_id }),
            ),
            AccountCmd::Disable { account_id } => (
                "account.disable".to_owned(),
                json!({ "account_id": account_id }),
            ),
            AccountCmd::Enable { account_id } => (
                "account.enable".to_owned(),
                json!({ "account_id": account_id }),
            ),
            AccountCmd::Remove { account_id } => (
                "account.remove".to_owned(),
                json!({ "account_id": account_id }),
            ),
        },

        Commands::Project(p) => match p {
            ProjectCmd::Register {
                name, repo_path, default_agent_type, default_account_tags, workspace_policy,
            } => {
                let tags_vec: Vec<&str> = if default_account_tags.is_empty() {
                    vec![]
                } else {
                    default_account_tags.split(',').map(str::trim).collect()
                };
                (
                    "project.register".to_owned(),
                    json!({
                        "name": name,
                        "repo_path": repo_path,
                        "default_agent_type": default_agent_type,
                        "default_account_tags": tags_vec,
                        "workspace_policy": workspace_policy,
                    }),
                )
            }
            ProjectCmd::List => ("project.list".to_owned(), json!({})),
            ProjectCmd::Get { project_id } => (
                "project.get".to_owned(),
                json!({ "project_id": project_id }),
            ),
            ProjectCmd::Remove { project_id } => (
                "project.remove".to_owned(),
                json!({ "project_id": project_id }),
            ),
            ProjectCmd::Workspaces { project_id } => (
                "project.workspaces".to_owned(),
                json!({ "project_id": project_id }),
            ),
        },

        Commands::Interaction(i) => match i {
            InteractionCmd::ListPending { session_id } => (
                "interaction.list_pending".to_owned(),
                json!({ "session_id": session_id }),
            ),
            InteractionCmd::List { session_id, state } => (
                "interaction.list".to_owned(),
                json!({ "session_id": session_id, "state": state }),
            ),
            InteractionCmd::Get { interaction_id } => (
                "interaction.get".to_owned(),
                json!({ "interaction_id": interaction_id }),
            ),
            InteractionCmd::Reply { interaction_id, decision, response, actor } => (
                "interaction.reply".to_owned(),
                json!({
                    "interaction_id": interaction_id,
                    "decision": decision,
                    "response": response,
                    "actor": actor,
                }),
            ),
            InteractionCmd::Approve { interaction_id, note } => (
                "interaction.reply".to_owned(),
                json!({
                    "interaction_id": interaction_id,
                    "decision": "allow",
                    "response": note,
                }),
            ),
            InteractionCmd::Deny { interaction_id, reason } => (
                "interaction.reply".to_owned(),
                json!({
                    "interaction_id": interaction_id,
                    "decision": "deny",
                    "response": reason,
                }),
            ),
            InteractionCmd::Dismiss { interaction_id, actor } => (
                "interaction.dismiss".to_owned(),
                json!({
                    "interaction_id": interaction_id,
                    "actor": actor,
                }),
            ),
        },

        Commands::Policy(p) => match p {
            PolicyCmd::List { scope } => (
                "policy.list".to_owned(),
                json!({ "scope": scope }),
            ),
            PolicyCmd::Get { policy_id } => (
                "policy.get".to_owned(),
                json!({ "policy_id": policy_id }),
            ),
            PolicyCmd::Upsert {
                policy_id,
                name,
                scope,
                priority,
                decision,
                conditions,
                disabled,
            } => {
                let conditions_val: serde_json::Value =
                    serde_json::from_str(conditions).unwrap_or(json!([]));
                (
                    "policy.upsert".to_owned(),
                    json!({
                        "policy_id": policy_id,
                        "name": name,
                        "scope": scope,
                        "priority": priority,
                        "decision": decision,
                        "conditions": conditions_val,
                        "enabled": !disabled,
                    }),
                )
            }
            PolicyCmd::Remove { policy_id } => (
                "policy.remove".to_owned(),
                json!({ "policy_id": policy_id }),
            ),
            PolicyCmd::Test { tool_name, agent_type, project_id } => (
                "policy.test".to_owned(),
                json!({
                    "tool_name": tool_name,
                    "agent_type": agent_type,
                    "project_id": project_id,
                }),
            ),
        },

        Commands::Audit(a) => match a {
            AuditCmd::List { session_id, limit } => (
                "audit.list".to_owned(),
                json!({ "session_id": session_id, "limit": limit }),
            ),
        },

        Commands::Events(e) => match e {
            EventsCmd::Query { session_id, limit } => (
                "events.query".to_owned(),
                json!({ "session_id": session_id, "limit": limit }),
            ),
            EventsCmd::Subscribe => ("events.subscribe".to_owned(), json!({})),
            EventsCmd::Verify { .. } | EventsCmd::Replay { .. } => unreachable!(),
        },
    };
    Ok((cmd, params))
}

// ── Socket helpers ────────────────────────────────────────────────────────────

async fn send_command(
    socket_path: &PathBuf,
    cmd: &str,
    params: serde_json::Value,
) -> Result<ApiResponse> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connecting to daemon socket: {}", socket_path.display()))?;

    let (read_half, mut write_half) = stream.into_split();

    let req = ApiRequest {
        v: 1,
        id: "cli-1".to_owned(),
        cmd: cmd.to_owned(),
        params,
    };

    let line = serde_json::to_string(&req)? + "\n";
    write_half.write_all(line.as_bytes()).await?;

    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();
    let response_line = lines
        .next_line()
        .await?
        .context("Daemon closed connection without responding")?;

    let resp: ApiResponse = serde_json::from_str(&response_line)
        .context("Failed to parse daemon response")?;
    Ok(resp)
}

async fn subscribe_events(socket_path: &PathBuf) -> Result<()> {
    let stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connecting to {}", socket_path.display()))?;

    let (read_half, mut write_half) = stream.into_split();

    let req = ApiRequest {
        v: 1,
        id: "cli-sub".to_owned(),
        cmd: "events.subscribe".to_owned(),
        params: json!({}),
    };
    let line = serde_json::to_string(&req)? + "\n";
    write_half.write_all(line.as_bytes()).await?;

    println!("Subscribed to event stream (Ctrl-C to stop)...");

    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        // Pretty-print events
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            println!("{line}");
        }
    }
    Ok(())
}

// ── Output formatting ─────────────────────────────────────────────────────────

fn print_response(resp: &ApiResponse) {
    if resp.ok {
        if let Some(result) = &resp.result {
            // Sessions list
            if let Some(sessions) = result.get("sessions").and_then(|s| s.as_array()) {
                if sessions.is_empty() { println!("No sessions."); return; }
                println!("{:<26}  {:<12}  {:<20}  {}", "SESSION ID", "STATE", "AGENT", "TASK");
                println!("{}", "-".repeat(80));
                for s in sessions {
                    println!(
                        "{:<26}  {:<12}  {:<20}  {}",
                        s["id"].as_str().unwrap_or("?"),
                        s["state"].as_str().unwrap_or("?"),
                        s["agent_type"].as_str().unwrap_or("?"),
                        s["task_description"].as_str().unwrap_or("?"),
                    );
                }
                return;
            }
            // Accounts list
            if let Some(accounts) = result.get("accounts").and_then(|a| a.as_array()) {
                if accounts.is_empty() { println!("No accounts."); return; }
                println!("{:<26}  {:<12}  {:<14}  {:<4}  {}", "ACCOUNT ID", "STATE", "PROVIDER", "CAP", "LABEL");
                println!("{}", "-".repeat(80));
                for a in accounts {
                    println!(
                        "{:<26}  {:<12}  {:<14}  {:<4}  {}",
                        a["id"].as_str().or_else(|| a["account_id"].as_str()).unwrap_or("?"),
                        a["state"].as_str().unwrap_or("?"),
                        a["provider"].as_str().unwrap_or("?"),
                        a["concurrency_cap"].as_u64().unwrap_or(0),
                        a["label"].as_str().unwrap_or("?"),
                    );
                }
                return;
            }
            // Projects list
            if let Some(projects) = result.get("projects").and_then(|p| p.as_array()) {
                if projects.is_empty() { println!("No projects."); return; }
                println!("{:<26}  {:<20}  {}", "PROJECT ID", "NAME", "REPO");
                println!("{}", "-".repeat(80));
                for p in projects {
                    println!(
                        "{:<26}  {:<20}  {}",
                        p["id"].as_str().unwrap_or("?"),
                        p["name"].as_str().unwrap_or("?"),
                        p["repo_path"].as_str().unwrap_or("?"),
                    );
                }
                return;
            }
            // Interactions list
            if let Some(interactions) = result.get("interactions").and_then(|i| i.as_array()) {
                if interactions.is_empty() { println!("No interactions."); return; }
                println!("{:<26}  {:<26}  {:<16}  {:<14}  {}", "INTERACTION ID", "SESSION ID", "KIND", "STATE", "DETAILS");
                println!("{}", "-".repeat(100));
                for i in interactions {
                    let details = i["tool_name"].as_str().or_else(|| i["prompt"].as_str()).unwrap_or("");
                    println!(
                        "{:<26}  {:<26}  {:<16}  {:<14}  {}",
                        i["id"].as_str().unwrap_or("?"),
                        i["session_id"].as_str().unwrap_or("?"),
                        i["kind"].as_str().unwrap_or("?"),
                        i["state"].as_str().unwrap_or("?"),
                        details,
                    );
                }
                return;
            }
            // Policies list
            if let Some(policies) = result.get("policies").and_then(|p| p.as_array()) {
                if policies.is_empty() { println!("No policies."); return; }
                println!("{:<26}  {:<8}  {:<12}  {:<14}  {}", "POLICY ID", "PRIORITY", "DECISION", "SCOPE", "NAME");
                println!("{}", "-".repeat(80));
                for p in policies {
                    println!(
                        "{:<26}  {:<8}  {:<12}  {:<14}  {}",
                        p["id"].as_str().unwrap_or("?"),
                        p["priority"].as_i64().unwrap_or(0),
                        p["decision"].as_str().unwrap_or("?"),
                        p["scope"].as_str().unwrap_or("?"),
                        p["name"].as_str().unwrap_or("?"),
                    );
                }
                return;
            }
            // Audit entries list
            if let Some(entries) = result.get("audit_entries").and_then(|e| e.as_array()) {
                if entries.is_empty() { println!("No audit entries."); return; }
                println!("{:<24}  {:<20}  {:<14}  {}", "TIMESTAMP", "ACTOR", "ACTION", "RATIONALE");
                println!("{}", "-".repeat(80));
                for e in entries {
                    println!(
                        "{:<24}  {:<20}  {:<14}  {}",
                        e["timestamp"].as_str().unwrap_or("?"),
                        e["actor"].as_str().unwrap_or("?"),
                        e["action"].as_str().unwrap_or("?"),
                        e["rationale"].as_str().unwrap_or(""),
                    );
                }
                return;
            }
            println!("{}", serde_json::to_string_pretty(result).unwrap_or_default());
        } else {
            println!("OK");
        }
    } else if let Some(err) = &resp.error {
        eprintln!("Error [{}]: {}", err.code, err.message);
    }
}
