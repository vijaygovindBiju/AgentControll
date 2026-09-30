//! Interaction Hub — central layer for agent ↔ human interactions.
//!
//! Responsibilities:
//!  - Track pending and historical interactions (questions and approval requests).
//!  - Route incoming adapter interactions (`ApprovalRequested`, `QuestionRaised`) through PolicyEngine.
//!  - Auto-resolve approved or denied requests immediately, or escalate to Pending for human review.
//!  - Route human responses / approvals back to sessions and running adapters.
//!  - Ensure audit logging for all policy and human decisions.
//!  - Persist interactions across restarts in SQLite.

use anyhow::{bail, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::info;

use crate::policy_engine::{PolicyEngine, PolicyEvaluation};
use crate::types::{
    AgentCommand, AuditEntry, Id, Interaction, InteractionKind, InteractionState, PolicyDecision,
};

// ── Persistence (SQLite) ──────────────────────────────────────────────────────

/// Persistent store for Interaction entities.
pub struct InteractionStore {
    conn: Connection,
}

impl InteractionStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let conn = Connection::open(path)?;
        let s = Self { conn };
        s.init_schema()?;
        Ok(s)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let s = Self { conn };
        s.init_schema()?;
        Ok(s)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS interactions (
                 id          TEXT PRIMARY KEY,
                 session_id  TEXT NOT NULL,
                 kind        TEXT NOT NULL,
                 state       TEXT NOT NULL,
                 prompt      TEXT NOT NULL,
                 tool_name   TEXT,
                 tool_args   TEXT,
                 policy_id   TEXT,
                 decision    TEXT,
                 response    TEXT,
                 resolved_by TEXT,
                 created_at  TEXT NOT NULL,
                 resolved_at TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_interactions_session
                 ON interactions (session_id);
             CREATE INDEX IF NOT EXISTS idx_interactions_state
                 ON interactions (state);
             CREATE INDEX IF NOT EXISTS idx_interactions_created
                 ON interactions (created_at);",
        )?;
        Ok(())
    }

    pub fn insert(&self, interaction: &Interaction) -> Result<()> {
        self.conn.execute(
            "INSERT INTO interactions
             (id, session_id, kind, state, prompt, tool_name, tool_args,
              policy_id, decision, response, resolved_by, created_at, resolved_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                interaction.id.0,
                interaction.session_id.0,
                serde_json::to_string(&interaction.kind)?,
                serde_json::to_string(&interaction.state)?,
                interaction.prompt,
                interaction.tool_name,
                interaction
                    .tool_args
                    .as_ref()
                    .map(|v| serde_json::to_string(v))
                    .transpose()?,
                interaction.policy_id.as_ref().map(|p| &p.0),
                interaction
                    .decision
                    .as_ref()
                    .map(|d| serde_json::to_string(d))
                    .transpose()?,
                interaction.response,
                interaction.resolved_by,
                interaction.created_at.to_rfc3339(),
                interaction.resolved_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn update(&self, interaction: &Interaction) -> Result<()> {
        self.conn.execute(
            "UPDATE interactions SET
             state = ?2, policy_id = ?3, decision = ?4, response = ?5,
             resolved_by = ?6, resolved_at = ?7
             WHERE id = ?1",
            params![
                interaction.id.0,
                serde_json::to_string(&interaction.state)?,
                interaction.policy_id.as_ref().map(|p| &p.0),
                interaction
                    .decision
                    .as_ref()
                    .map(|d| serde_json::to_string(d))
                    .transpose()?,
                interaction.response,
                interaction.resolved_by,
                interaction.resolved_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn load_all(&self) -> Result<Vec<Interaction>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, kind, state, prompt, tool_name, tool_args,
                    policy_id, decision, response, resolved_by, created_at, resolved_at
             FROM interactions ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], row_to_interaction)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

fn row_to_interaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<Interaction> {
    let id: String = row.get(0)?;
    let session_id: String = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let state_str: String = row.get(3)?;
    let prompt: String = row.get(4)?;
    let tool_name: Option<String> = row.get(5)?;
    let tool_args_str: Option<String> = row.get(6)?;
    let policy_id: Option<String> = row.get(7)?;
    let decision_str: Option<String> = row.get(8)?;
    let response: Option<String> = row.get(9)?;
    let resolved_by: Option<String> = row.get(10)?;
    let created_at_str: String = row.get(11)?;
    let resolved_at_str: Option<String> = row.get(12)?;

    let kind: InteractionKind =
        serde_json::from_str(&kind_str).unwrap_or(InteractionKind::ApprovalRequest);
    let state: InteractionState =
        serde_json::from_str(&state_str).unwrap_or(InteractionState::Pending);
    let tool_args: Option<serde_json::Value> =
        tool_args_str.and_then(|s| serde_json::from_str(&s).ok());
    let decision: Option<PolicyDecision> = decision_str.and_then(|s| serde_json::from_str(&s).ok());
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let resolved_at = resolved_at_str.and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    });

    Ok(Interaction {
        id: Id(id),
        session_id: Id(session_id),
        kind,
        state,
        prompt,
        tool_name,
        tool_args,
        policy_id: policy_id.map(Id),
        decision,
        response,
        resolved_by,
        created_at,
        resolved_at,
    })
}

// ── Interaction Hub ──────────────────────────────────────────────────────────

/// Outcome when an approval or question is submitted to the Interaction Hub.
#[derive(Debug, Clone)]
pub enum InteractionSubmissionResult {
    /// Resolved immediately by policy.
    AutoResolved {
        interaction: Interaction,
        decision: PolicyDecision,
        command: AgentCommand,
    },
    /// Requires human intervention. Stored as Pending.
    Escalated { interaction: Interaction },
}

/// Result of human interaction resolution.
#[derive(Debug, Clone)]
pub struct HumanResolutionResult {
    pub interaction: Interaction,
    pub command: Option<AgentCommand>,
}

/// Central Interaction Hub managing interactions and routing to policy/session.
pub struct InteractionHub {
    interactions: HashMap<String, Interaction>,
    store: InteractionStore,
    policy_engine: PolicyEngine,
}

impl InteractionHub {
    pub fn new(store: InteractionStore, policy_engine: PolicyEngine) -> Result<Self> {
        let loaded = store.load_all()?;
        let mut map = HashMap::new();
        for item in loaded {
            map.insert(item.id.0.clone(), item);
        }
        info!("InteractionHub initialized with {} interactions", map.len());
        Ok(Self {
            interactions: map,
            store,
            policy_engine,
        })
    }

    /// Access the policy engine directly (read-only or test evaluation).
    pub fn policy_engine(&self) -> &PolicyEngine {
        &self.policy_engine
    }

    /// Mutably access the policy engine.
    pub fn policy_engine_mut(&mut self) -> &mut PolicyEngine {
        &mut self.policy_engine
    }

    // ── Queries ───────────────────────────────────────────────────────────

    pub fn get(&self, id: &Id) -> Option<&Interaction> {
        self.interactions.get(&id.0)
    }

    pub fn list(
        &self,
        session_id: Option<&Id>,
        state_filter: Option<&InteractionState>,
    ) -> Vec<&Interaction> {
        let mut list: Vec<&Interaction> = self
            .interactions
            .values()
            .filter(|i| {
                if let Some(sid) = session_id {
                    if &i.session_id != sid {
                        return false;
                    }
                }
                if let Some(s) = state_filter {
                    if &i.state != s {
                        return false;
                    }
                }
                true
            })
            .collect();
        list.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        list
    }

    pub fn list_pending(&self, session_id: Option<&Id>) -> Vec<&Interaction> {
        self.list(session_id, Some(&InteractionState::Pending))
    }

    // ── Inbound from Agent Adapter ────────────────────────────────────────

    /// Submit an approval request from an agent session.
    pub fn submit_approval_request(
        &mut self,
        session_id: Id,
        tool_name: String,
        tool_args: Option<serde_json::Value>,
        prompt: String,
        agent_type: Option<&str>,
        project_id: Option<&Id>,
    ) -> Result<InteractionSubmissionResult> {
        let mut interaction = Interaction::new_approval_request(
            session_id.clone(),
            tool_name.clone(),
            tool_args,
            prompt,
        );

        // Evaluate with Policy Engine
        let evaluation: PolicyEvaluation =
            self.policy_engine
                .evaluate(Some(&tool_name), agent_type, project_id);

        match evaluation.decision {
            PolicyDecision::Allow => {
                let now = Utc::now();
                let policy_id = evaluation.matched_policy.as_ref().map(|p| p.id.clone());
                let resolved_by = policy_id
                    .as_ref()
                    .map(|p| format!("policy:{}", p.0))
                    .unwrap_or_else(|| "policy:system".into());

                interaction.state = InteractionState::AutoResolved;
                interaction.decision = Some(PolicyDecision::Allow);
                interaction.policy_id = policy_id;
                interaction.resolved_by = Some(resolved_by.clone());
                interaction.resolved_at = Some(now);

                self.store.insert(&interaction)?;
                self.interactions
                    .insert(interaction.id.0.clone(), interaction.clone());

                // Record audit log entry
                let audit = AuditEntry::new(
                    Some(interaction.id.clone()),
                    Some(session_id),
                    resolved_by,
                    "allow".into(),
                    Some(evaluation.reason),
                );
                self.policy_engine.record_audit(&audit)?;

                let cmd = AgentCommand::Respond {
                    allow: true,
                    response: None,
                };
                Ok(InteractionSubmissionResult::AutoResolved {
                    interaction,
                    decision: PolicyDecision::Allow,
                    command: cmd,
                })
            }
            PolicyDecision::Deny => {
                let now = Utc::now();
                let policy_id = evaluation.matched_policy.as_ref().map(|p| p.id.clone());
                let resolved_by = policy_id
                    .as_ref()
                    .map(|p| format!("policy:{}", p.0))
                    .unwrap_or_else(|| "policy:system".into());

                interaction.state = InteractionState::AutoResolved;
                interaction.decision = Some(PolicyDecision::Deny);
                interaction.policy_id = policy_id;
                interaction.resolved_by = Some(resolved_by.clone());
                interaction.resolved_at = Some(now);

                self.store.insert(&interaction)?;
                self.interactions
                    .insert(interaction.id.0.clone(), interaction.clone());

                // Record audit log entry
                let audit = AuditEntry::new(
                    Some(interaction.id.clone()),
                    Some(session_id),
                    resolved_by,
                    "deny".into(),
                    Some(evaluation.reason),
                );
                self.policy_engine.record_audit(&audit)?;

                let cmd = AgentCommand::Respond {
                    allow: false,
                    response: Some("Action denied by policy".into()),
                };
                Ok(InteractionSubmissionResult::AutoResolved {
                    interaction,
                    decision: PolicyDecision::Deny,
                    command: cmd,
                })
            }
            PolicyDecision::RequireHuman => {
                // Must be reviewed by human
                self.store.insert(&interaction)?;
                self.interactions
                    .insert(interaction.id.0.clone(), interaction.clone());

                // Record audit entry for policy evaluation requiring human
                let audit = AuditEntry::new(
                    Some(interaction.id.clone()),
                    Some(session_id),
                    "policy:evaluator".into(),
                    "require_human".into(),
                    Some(evaluation.reason),
                );
                self.policy_engine.record_audit(&audit)?;

                Ok(InteractionSubmissionResult::Escalated { interaction })
            }
        }
    }

    /// Submit a question from an agent session. Questions always require human response.
    pub fn submit_question(
        &mut self,
        session_id: Id,
        prompt: String,
    ) -> Result<InteractionSubmissionResult> {
        let interaction = Interaction::new_question(session_id.clone(), prompt);
        self.store.insert(&interaction)?;
        self.interactions
            .insert(interaction.id.0.clone(), interaction.clone());

        let audit = AuditEntry::new(
            Some(interaction.id.clone()),
            Some(session_id),
            "system".into(),
            "question_pending".into(),
            Some("Agent raised question requiring human input".into()),
        );
        self.policy_engine.record_audit(&audit)?;

        Ok(InteractionSubmissionResult::Escalated { interaction })
    }

    // ── Inbound from Human Operator ───────────────────────────────────────

    /// Reply to a pending interaction (answer question, or approve/deny request).
    pub fn reply(
        &mut self,
        interaction_id: &Id,
        decision: Option<PolicyDecision>,
        response: Option<String>,
        actor: Option<String>,
    ) -> Result<HumanResolutionResult> {
        let (session_id, kind) = {
            let interaction = self
                .interactions
                .get_mut(&interaction_id.0)
                .ok_or_else(|| anyhow::anyhow!("Interaction not found: {}", interaction_id))?;

            if interaction.state != InteractionState::Pending {
                bail!(
                    "Cannot reply to interaction {} in state {}",
                    interaction_id,
                    interaction.state
                );
            }

            let human_actor = actor.unwrap_or_else(|| "human".into());
            let now = Utc::now();

            interaction.state = InteractionState::HumanResolved;
            interaction.resolved_by = Some(human_actor.clone());
            interaction.resolved_at = Some(now);
            interaction.response = response.clone();

            let decided = match interaction.kind {
                InteractionKind::ApprovalRequest => {
                    let d = decision.unwrap_or(PolicyDecision::Allow);
                    interaction.decision = Some(d.clone());
                    d
                }
                InteractionKind::Question => {
                    interaction.decision = Some(PolicyDecision::Allow);
                    PolicyDecision::Allow
                }
            };

            let _ = decided; // stored in interaction
            (interaction.session_id.clone(), interaction.kind.clone())
        };

        let interaction = self.interactions.get(&interaction_id.0).unwrap().clone();
        self.store.update(&interaction)?;

        // Audit entry
        let action_str = match &interaction.decision {
            Some(PolicyDecision::Allow) => "allow",
            Some(PolicyDecision::Deny) => "deny",
            _ => "reply",
        };
        let audit = AuditEntry::new(
            Some(interaction.id.clone()),
            Some(session_id),
            interaction
                .resolved_by
                .clone()
                .unwrap_or_else(|| "human".into()),
            action_str.into(),
            response.clone(),
        );
        self.policy_engine.record_audit(&audit)?;

        // Build corresponding AgentCommand to send to the adapter
        let command = match kind {
            InteractionKind::ApprovalRequest => {
                let allow = interaction.decision == Some(PolicyDecision::Allow);
                Some(AgentCommand::Respond {
                    allow,
                    response: response.or_else(|| {
                        if !allow {
                            Some("Denied by human operator".into())
                        } else {
                            None
                        }
                    }),
                })
            }
            InteractionKind::Question => Some(AgentCommand::Respond {
                allow: true,
                response,
            }),
        };

        info!(
            "Interaction {} resolved by human: {:?}",
            interaction.id, interaction.decision
        );
        Ok(HumanResolutionResult {
            interaction,
            command,
        })
    }

    /// Dismiss a pending interaction.
    pub fn dismiss(&mut self, interaction_id: &Id, actor: Option<String>) -> Result<Interaction> {
        let session_id = {
            let interaction = self
                .interactions
                .get_mut(&interaction_id.0)
                .ok_or_else(|| anyhow::anyhow!("Interaction not found: {}", interaction_id))?;

            if interaction.state != InteractionState::Pending {
                bail!(
                    "Cannot dismiss interaction {} in state {}",
                    interaction_id,
                    interaction.state
                );
            }

            let human_actor = actor.unwrap_or_else(|| "human".into());
            interaction.state = InteractionState::Dismissed;
            interaction.resolved_by = Some(human_actor);
            interaction.resolved_at = Some(Utc::now());
            interaction.session_id.clone()
        };

        let interaction = self.interactions.get(&interaction_id.0).unwrap().clone();
        self.store.update(&interaction)?;

        let audit = AuditEntry::new(
            Some(interaction.id.clone()),
            Some(session_id),
            interaction
                .resolved_by
                .clone()
                .unwrap_or_else(|| "human".into()),
            "dismiss".into(),
            Some("Interaction dismissed without response".into()),
        );
        self.policy_engine.record_audit(&audit)?;

        info!("Interaction {} dismissed", interaction.id);
        Ok(interaction)
    }

    /// Expire all pending interactions for a session when the session terminates or stops.
    pub fn expire_session_interactions(&mut self, session_id: &Id) -> Result<Vec<Interaction>> {
        let now = Utc::now();
        let mut expired = Vec::new();

        for interaction in self.interactions.values_mut() {
            if &interaction.session_id == session_id
                && interaction.state == InteractionState::Pending
            {
                interaction.state = InteractionState::Expired;
                interaction.resolved_by = Some("system".into());
                interaction.resolved_at = Some(now);
                expired.push(interaction.clone());
            }
        }

        for item in &expired {
            self.store.update(item)?;
            let audit = AuditEntry::new(
                Some(item.id.clone()),
                Some(session_id.clone()),
                "system".into(),
                "expire".into(),
                Some("Session terminated with unresolved interaction".into()),
            );
            self.policy_engine.record_audit(&audit)?;
        }

        Ok(expired)
    }
}

// ── Shared handle ─────────────────────────────────────────────────────────────

/// Thread-safe shared handle to an `InteractionHub`.
#[derive(Clone)]
pub struct InteractionHubHandle(pub Arc<Mutex<InteractionHub>>);

impl InteractionHubHandle {
    pub fn new(hub: InteractionHub) -> Self {
        Self(Arc::new(Mutex::new(hub)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy_engine::{PolicyEngine, PolicyStore};
    use crate::types::{Policy, PolicyCondition, PolicyDecision, PolicyScope};

    fn make_hub() -> InteractionHub {
        let policy_store = PolicyStore::open_in_memory().unwrap();
        let policy_engine = PolicyEngine::new(policy_store).unwrap();
        let interaction_store = InteractionStore::open_in_memory().unwrap();
        InteractionHub::new(interaction_store, policy_engine).unwrap()
    }

    #[test]
    fn submit_and_get_approval_request_pending() {
        let mut hub = make_hub();
        let session_id = Id::new();
        let res = hub
            .submit_approval_request(
                session_id.clone(),
                "bash".into(),
                None,
                "Run bash script".into(),
                Some("mock"),
                None,
            )
            .unwrap();

        match res {
            InteractionSubmissionResult::Escalated { interaction } => {
                assert_eq!(interaction.state, InteractionState::Pending);
                assert_eq!(interaction.session_id, session_id);
                assert_eq!(interaction.tool_name, Some("bash".into()));
                let retrieved = hub.get(&interaction.id).unwrap();
                assert_eq!(retrieved.id, interaction.id);
            }
            _ => panic!("Expected Escalated result"),
        }
    }

    #[test]
    fn submit_approval_auto_approved() {
        let mut hub = make_hub();
        // Add an allow policy for read_file
        hub.policy_engine_mut()
            .create_policy(Policy::new(
                "allow-read".into(),
                PolicyScope::Global,
                10,
                vec![PolicyCondition::ToolNameEquals("read_file".into())],
                PolicyDecision::Allow,
            ))
            .unwrap();

        let session_id = Id::new();
        let res = hub
            .submit_approval_request(
                session_id.clone(),
                "read_file".into(),
                None,
                "Read a file".into(),
                Some("mock"),
                None,
            )
            .unwrap();

        match res {
            InteractionSubmissionResult::AutoResolved {
                interaction,
                decision,
                command,
            } => {
                assert_eq!(interaction.state, InteractionState::AutoResolved);
                assert_eq!(decision, PolicyDecision::Allow);
                assert!(matches!(command, AgentCommand::Respond { allow: true, .. }));
            }
            _ => panic!("Expected AutoResolved"),
        }
    }

    #[test]
    fn submit_approval_denied_by_policy() {
        let mut hub = make_hub();
        hub.policy_engine_mut()
            .create_policy(Policy::new(
                "deny-drop".into(),
                PolicyScope::Global,
                10,
                vec![PolicyCondition::ToolNameEquals("drop_database".into())],
                PolicyDecision::Deny,
            ))
            .unwrap();

        let session_id = Id::new();
        let res = hub
            .submit_approval_request(
                session_id,
                "drop_database".into(),
                None,
                "Drop database".into(),
                Some("mock"),
                None,
            )
            .unwrap();

        match res {
            InteractionSubmissionResult::AutoResolved {
                interaction,
                decision,
                command,
            } => {
                assert_eq!(interaction.state, InteractionState::AutoResolved);
                assert_eq!(decision, PolicyDecision::Deny);
                assert!(matches!(
                    command,
                    AgentCommand::Respond { allow: false, .. }
                ));
            }
            _ => panic!("Expected AutoResolved Deny"),
        }
    }

    #[test]
    fn submit_question_always_escalates() {
        let mut hub = make_hub();
        let session_id = Id::new();
        let res = hub
            .submit_question(session_id.clone(), "What is your name?".into())
            .unwrap();

        match res {
            InteractionSubmissionResult::Escalated { interaction } => {
                assert_eq!(interaction.kind, InteractionKind::Question);
                assert_eq!(interaction.state, InteractionState::Pending);
            }
            _ => panic!("Expected Escalated"),
        }
    }

    #[test]
    fn reply_to_pending_interaction() {
        let mut hub = make_hub();
        let session_id = Id::new();
        let sub = hub
            .submit_approval_request(
                session_id,
                "write_file".into(),
                None,
                "Write to file".into(),
                Some("mock"),
                None,
            )
            .unwrap();

        let interaction_id = match sub {
            InteractionSubmissionResult::Escalated { interaction } => interaction.id,
            _ => panic!("Expected Escalated"),
        };

        let res = hub
            .reply(
                &interaction_id,
                Some(PolicyDecision::Allow),
                Some("Approved write".into()),
                Some("operator".into()),
            )
            .unwrap();

        assert_eq!(res.interaction.state, InteractionState::HumanResolved);
        assert_eq!(res.interaction.decision, Some(PolicyDecision::Allow));
        assert_eq!(res.interaction.resolved_by, Some("operator".into()));
        assert!(matches!(
            res.command,
            Some(AgentCommand::Respond {
                allow: true,
                response: Some(_)
            })
        ));
    }

    #[test]
    fn duplicate_reply_fails() {
        let mut hub = make_hub();
        let session_id = Id::new();
        let sub = hub
            .submit_approval_request(
                session_id,
                "write_file".into(),
                None,
                "Write to file".into(),
                Some("mock"),
                None,
            )
            .unwrap();

        let id = match sub {
            InteractionSubmissionResult::Escalated { interaction } => interaction.id,
            _ => panic!("Expected Escalated"),
        };

        hub.reply(&id, Some(PolicyDecision::Allow), None, None)
            .unwrap();
        assert!(
            hub.reply(&id, Some(PolicyDecision::Allow), None, None)
                .is_err(),
            "Replying to an already resolved interaction must fail"
        );
    }

    #[test]
    fn dismiss_pending_interaction() {
        let mut hub = make_hub();
        let session_id = Id::new();
        let sub = hub
            .submit_question(session_id, "Clarification?".into())
            .unwrap();

        let id = match sub {
            InteractionSubmissionResult::Escalated { interaction } => interaction.id,
            _ => panic!("Expected Escalated"),
        };

        let dismissed = hub.dismiss(&id, Some("admin".into())).unwrap();
        assert_eq!(dismissed.state, InteractionState::Dismissed);
        assert_eq!(dismissed.resolved_by, Some("admin".into()));
    }

    #[test]
    fn list_pending_interactions() {
        let mut hub = make_hub();
        let s1 = Id::new();
        let s2 = Id::new();

        hub.submit_question(s1.clone(), "Q1".into()).unwrap();
        hub.submit_question(s1.clone(), "Q2".into()).unwrap();
        hub.submit_question(s2.clone(), "Q3".into()).unwrap();

        assert_eq!(hub.list_pending(None).len(), 3);
        assert_eq!(hub.list_pending(Some(&s1)).len(), 2);
        assert_eq!(hub.list_pending(Some(&s2)).len(), 1);
    }

    #[test]
    fn expire_session_interactions() {
        let mut hub = make_hub();
        let s1 = Id::new();
        hub.submit_question(s1.clone(), "Q1".into()).unwrap();
        hub.submit_question(s1.clone(), "Q2".into()).unwrap();

        let expired = hub.expire_session_interactions(&s1).unwrap();
        assert_eq!(expired.len(), 2);
        assert_eq!(hub.list_pending(Some(&s1)).len(), 0);
    }

    #[test]
    fn persistence_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("interactions.db");
        let id;

        {
            let store = InteractionStore::open(&db_path).unwrap();
            let pstore = PolicyStore::open_in_memory().unwrap();
            let pengine = PolicyEngine::new(pstore).unwrap();
            let mut hub = InteractionHub::new(store, pengine).unwrap();

            let sub = hub.submit_question(Id::new(), "Persist me".into()).unwrap();
            id = match sub {
                InteractionSubmissionResult::Escalated { interaction } => interaction.id,
                _ => panic!("Expected Escalated"),
            };
        }

        {
            let store = InteractionStore::open(&db_path).unwrap();
            let pstore = PolicyStore::open_in_memory().unwrap();
            let pengine = PolicyEngine::new(pstore).unwrap();
            let hub = InteractionHub::new(store, pengine).unwrap();

            let loaded = hub.get(&id).unwrap();
            assert_eq!(loaded.prompt, "Persist me");
            assert_eq!(loaded.state, InteractionState::Pending);
        }
    }
}
