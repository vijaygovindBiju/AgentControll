//! Policy Engine — deterministic policy evaluation layer.
//!
//! Responsibilities:
//!  - Maintain policy rules (in-memory with SQLite persistence).
//!  - Evaluate incoming approval requests against rules.
//!  - Enforce the hard never-auto-approve boundary.
//!  - Return deterministic `PolicyDecision` results.
//!
//! Precedence (ADR in DECISIONS.md):
//!   1. Never-auto-approve boundary → always RequireHuman
//!   2. Deny rules (highest priority first)
//!   3. Allow rules (highest priority first)
//!   4. Default → RequireHuman (fail-safe)

use anyhow::{bail, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, info};

use crate::types::{AuditEntry, Id, Policy, PolicyCondition, PolicyDecision, PolicyScope};

// ── Never-auto-approve boundary ──────────────────────────────────────────────

/// Tool names / prefixes that are NEVER auto-approved regardless of policy.
/// These always return RequireHuman.
///
/// This is a hard-coded boundary defined in SECURITY.md (T3).
/// Changing it requires a code change and daemon restart.
const NEVER_AUTO_APPROVE_TOOLS: &[&str] = &[
    "rm",
    "git_push_force",
    "git_reset_hard",
    "sudo",
    "chmod",
    "chown",
    "curl",
    "wget",
    "fetch",
    "npm_publish",
    "cargo_publish",
    "pip_upload",
    "git_clean",
    "deploy",
    "publish",
];

/// Prefixes that trigger the never-auto-approve boundary.
const NEVER_AUTO_APPROVE_PREFIXES: &[&str] = &[
    "rm -rf",
    "sudo ",
    "curl ",
    "wget ",
    "git push --force",
    "git push -f",
    "git reset --hard",
    "git clean -f",
    "npm publish",
    "cargo publish",
    "pip upload",
];

/// Check if a tool name falls in the never-auto-approve boundary.
pub fn is_never_auto_approve(tool_name: &str) -> bool {
    let lower = tool_name.to_lowercase();
    // Check exact matches
    if NEVER_AUTO_APPROVE_TOOLS.iter().any(|t| lower == *t) {
        return true;
    }
    // Check prefix matches
    if NEVER_AUTO_APPROVE_PREFIXES
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return true;
    }
    false
}

// ── Policy Store (SQLite) ────────────────────────────────────────────────────

/// Persistent store for Policy entities.
pub struct PolicyStore {
    conn: Connection,
}

impl PolicyStore {
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
             CREATE TABLE IF NOT EXISTS policies (
                 id          TEXT PRIMARY KEY,
                 name        TEXT NOT NULL,
                 scope       TEXT NOT NULL,
                 priority    INTEGER NOT NULL DEFAULT 0,
                 conditions  TEXT NOT NULL DEFAULT '[]',
                 decision    TEXT NOT NULL DEFAULT '\"require_human\"',
                 enabled     INTEGER NOT NULL DEFAULT 1,
                 created_at  TEXT NOT NULL,
                 updated_at  TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS audit_log (
                 id              TEXT PRIMARY KEY,
                 event_seq       INTEGER,
                 interaction_id  TEXT,
                 session_id      TEXT,
                 actor           TEXT NOT NULL,
                 action          TEXT NOT NULL,
                 rationale       TEXT,
                 timestamp       TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_audit_session
                 ON audit_log (session_id) WHERE session_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_audit_interaction
                 ON audit_log (interaction_id) WHERE interaction_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_audit_timestamp
                 ON audit_log (timestamp);",
        )?;
        Ok(())
    }

    pub fn insert_policy(&self, policy: &Policy) -> Result<()> {
        self.conn.execute(
            "INSERT INTO policies (id, name, scope, priority, conditions, decision, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                policy.id.0,
                policy.name,
                serde_json::to_string(&policy.scope)?,
                policy.priority,
                serde_json::to_string(&policy.conditions)?,
                serde_json::to_string(&policy.decision)?,
                policy.enabled as i64,
                policy.created_at.to_rfc3339(),
                policy.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn update_policy(&self, policy: &Policy) -> Result<()> {
        self.conn.execute(
            "UPDATE policies SET name=?2, scope=?3, priority=?4, conditions=?5, decision=?6, enabled=?7, updated_at=?8
             WHERE id=?1",
            params![
                policy.id.0,
                policy.name,
                serde_json::to_string(&policy.scope)?,
                policy.priority,
                serde_json::to_string(&policy.conditions)?,
                serde_json::to_string(&policy.decision)?,
                policy.enabled as i64,
                policy.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_policy(&self, id: &Id) -> Result<bool> {
        let rows = self
            .conn
            .execute("DELETE FROM policies WHERE id=?1", params![id.0])?;
        Ok(rows > 0)
    }

    pub fn load_all_policies(&self) -> Result<Vec<Policy>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, scope, priority, conditions, decision, enabled, created_at, updated_at
             FROM policies ORDER BY priority DESC, created_at",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let scope_str: String = row.get(2)?;
                let priority: i32 = row.get(3)?;
                let conditions_str: String = row.get(4)?;
                let decision_str: String = row.get(5)?;
                let enabled: i64 = row.get(6)?;
                let created_at_str: String = row.get(7)?;
                let updated_at_str: String = row.get(8)?;

                let scope: PolicyScope =
                    serde_json::from_str(&scope_str).unwrap_or(PolicyScope::Global);
                let conditions: Vec<PolicyCondition> =
                    serde_json::from_str(&conditions_str).unwrap_or_default();
                let decision: PolicyDecision =
                    serde_json::from_str(&decision_str).unwrap_or(PolicyDecision::RequireHuman);
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());
                let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(Policy {
                    id: Id(id),
                    name,
                    scope,
                    priority,
                    conditions,
                    decision,
                    enabled: enabled != 0,
                    created_at,
                    updated_at,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Append an audit entry. This is append-only; never deleted or updated.
    pub fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit_log (id, event_seq, interaction_id, session_id, actor, action, rationale, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.id.0,
                entry.event_seq.map(|s| s as i64),
                entry.interaction_id.as_ref().map(|i| &i.0),
                entry.session_id.as_ref().map(|i| &i.0),
                entry.actor,
                entry.action,
                entry.rationale,
                entry.timestamp.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Query audit entries, optionally filtered by session.
    pub fn query_audit(
        &self,
        session_id: Option<&Id>,
        limit: Option<u64>,
    ) -> Result<Vec<AuditEntry>> {
        let limit_clause = limit.map(|l| format!("LIMIT {l}")).unwrap_or_default();

        if let Some(sid) = session_id {
            let sql = format!(
                "SELECT id, event_seq, interaction_id, session_id, actor, action, rationale, timestamp
                 FROM audit_log WHERE session_id = ?1 ORDER BY timestamp DESC {limit_clause}"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![sid.0], row_to_audit)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let sql = format!(
                "SELECT id, event_seq, interaction_id, session_id, actor, action, rationale, timestamp
                 FROM audit_log ORDER BY timestamp DESC {limit_clause}"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt
                .query_map([], row_to_audit)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }
}

fn row_to_audit(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    let id: String = row.get(0)?;
    let event_seq: Option<i64> = row.get(1)?;
    let interaction_id: Option<String> = row.get(2)?;
    let session_id: Option<String> = row.get(3)?;
    let actor: String = row.get(4)?;
    let action: String = row.get(5)?;
    let rationale: Option<String> = row.get(6)?;
    let timestamp_str: String = row.get(7)?;

    let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    Ok(AuditEntry {
        id: Id(id),
        event_seq: event_seq.map(|s| s as u64),
        interaction_id: interaction_id.map(Id),
        session_id: session_id.map(Id),
        actor,
        action,
        rationale,
        timestamp,
    })
}

// ── Policy Engine ────────────────────────────────────────────────────────────

/// The result of evaluating a policy against an action.
#[derive(Debug, Clone)]
pub struct PolicyEvaluation {
    /// The decision: Allow, Deny, or RequireHuman.
    pub decision: PolicyDecision,
    /// The policy that matched (if any).
    pub matched_policy: Option<Policy>,
    /// Why this decision was made.
    pub reason: String,
}

/// The policy engine — evaluates policies and enforces the never-auto-approve boundary.
pub struct PolicyEngine {
    policies: HashMap<String, Policy>,
    store: PolicyStore,
}

impl PolicyEngine {
    pub fn new(store: PolicyStore) -> Result<Self> {
        let policies_vec = store.load_all_policies()?;
        let mut policies = HashMap::new();
        for p in policies_vec {
            policies.insert(p.id.0.clone(), p);
        }
        info!("PolicyEngine loaded {} policies", policies.len());
        Ok(Self { policies, store })
    }

    // ── CRUD ──────────────────────────────────────────────────────────────

    /// Create a new policy rule.
    pub fn create_policy(&mut self, policy: Policy) -> Result<Id> {
        let id = policy.id.clone();
        self.store.insert_policy(&policy)?;
        self.policies.insert(id.0.clone(), policy);
        info!("Policy created: {}", id);
        Ok(id)
    }

    /// Get a policy by id.
    pub fn get_policy(&self, id: &Id) -> Option<&Policy> {
        self.policies.get(&id.0)
    }

    /// List all policies, optionally filtered by scope.
    pub fn list_policies(&self, scope: Option<&PolicyScope>) -> Vec<&Policy> {
        let mut policies: Vec<&Policy> = self
            .policies
            .values()
            .filter(|p| {
                if let Some(s) = scope {
                    &p.scope == s
                } else {
                    true
                }
            })
            .collect();
        // Sort by priority descending, then by name
        policies.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.name.cmp(&b.name)));
        policies
    }

    /// Update a policy (replace all fields except id and created_at).
    pub fn update_policy(
        &mut self,
        id: &Id,
        name: Option<String>,
        priority: Option<i32>,
        conditions: Option<Vec<PolicyCondition>>,
        decision: Option<PolicyDecision>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let policy = self
            .policies
            .get_mut(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Policy not found: {}", id))?;
        if let Some(n) = name {
            policy.name = n;
        }
        if let Some(p) = priority {
            policy.priority = p;
        }
        if let Some(c) = conditions {
            policy.conditions = c;
        }
        if let Some(d) = decision {
            policy.decision = d;
        }
        if let Some(e) = enabled {
            policy.enabled = e;
        }
        policy.updated_at = Utc::now();
        self.store.update_policy(policy)?;
        Ok(())
    }

    /// Remove a policy by id.
    pub fn remove_policy(&mut self, id: &Id) -> Result<()> {
        if !self.policies.contains_key(&id.0) {
            bail!("Policy not found: {}", id);
        }
        self.store.delete_policy(id)?;
        self.policies.remove(&id.0);
        info!("Policy removed: {}", id);
        Ok(())
    }

    // ── Evaluation ────────────────────────────────────────────────────────

    /// Evaluate an action against all policies.
    ///
    /// Precedence:
    ///   1. Never-auto-approve boundary → RequireHuman (cannot be overridden)
    ///   2. Deny rules (highest priority first among matching)
    ///   3. Allow rules (highest priority first among matching)
    ///   4. Default → RequireHuman
    ///
    /// This method is deterministic: same inputs always produce same output.
    pub fn evaluate(
        &self,
        tool_name: Option<&str>,
        agent_type: Option<&str>,
        _project_id: Option<&Id>,
    ) -> PolicyEvaluation {
        // Step 1: Check the never-auto-approve boundary
        if let Some(tn) = tool_name {
            if is_never_auto_approve(tn) {
                debug!(
                    "Policy eval: tool '{}' matches never-auto-approve boundary → RequireHuman",
                    tn
                );
                return PolicyEvaluation {
                    decision: PolicyDecision::RequireHuman,
                    matched_policy: None,
                    reason: format!(
                        "Tool '{}' is in the never-auto-approve boundary (SECURITY.md T3)",
                        tn
                    ),
                };
            }
        }

        // Step 2: Collect matching enabled policies, sorted by priority descending
        let mut matching: Vec<&Policy> = self
            .policies
            .values()
            .filter(|p| p.enabled)
            .filter(|p| {
                // All conditions must match (conjunction)
                if p.conditions.is_empty() {
                    // A policy with no conditions matches everything
                    true
                } else {
                    p.conditions
                        .iter()
                        .all(|c| c.matches(tool_name, agent_type))
                }
            })
            .collect();

        // Sort by priority descending (higher = evaluated first)
        matching.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Step 3: Apply precedence — Deny first, then Allow
        // First pass: find highest-priority Deny
        for policy in &matching {
            if policy.decision == PolicyDecision::Deny {
                debug!(
                    "Policy eval: matched Deny rule '{}' (priority={}) → Deny",
                    policy.name, policy.priority
                );
                return PolicyEvaluation {
                    decision: PolicyDecision::Deny,
                    matched_policy: Some((*policy).clone()),
                    reason: format!(
                        "Denied by policy '{}' (priority={})",
                        policy.name, policy.priority
                    ),
                };
            }
        }

        // Second pass: find highest-priority Allow
        for policy in &matching {
            if policy.decision == PolicyDecision::Allow {
                debug!(
                    "Policy eval: matched Allow rule '{}' (priority={}) → Allow",
                    policy.name, policy.priority
                );
                return PolicyEvaluation {
                    decision: PolicyDecision::Allow,
                    matched_policy: Some((*policy).clone()),
                    reason: format!(
                        "Allowed by policy '{}' (priority={})",
                        policy.name, policy.priority
                    ),
                };
            }
        }

        // Third pass: find RequireHuman rules (explicit)
        for policy in &matching {
            if policy.decision == PolicyDecision::RequireHuman {
                return PolicyEvaluation {
                    decision: PolicyDecision::RequireHuman,
                    matched_policy: Some((*policy).clone()),
                    reason: format!(
                        "Human required by policy '{}' (priority={})",
                        policy.name, policy.priority
                    ),
                };
            }
        }

        // Step 4: Default — no matching policy → RequireHuman (fail-safe)
        debug!("Policy eval: no matching policy → RequireHuman (default)");
        PolicyEvaluation {
            decision: PolicyDecision::RequireHuman,
            matched_policy: None,
            reason: "No matching policy; defaulting to RequireHuman (fail-safe)".to_owned(),
        }
    }

    /// Test a policy evaluation without persisting anything (for policy.test).
    pub fn test_evaluate(
        &self,
        tool_name: Option<&str>,
        agent_type: Option<&str>,
        project_id: Option<&Id>,
    ) -> PolicyEvaluation {
        self.evaluate(tool_name, agent_type, project_id)
    }

    // ── Audit ─────────────────────────────────────────────────────────────

    /// Record an audit entry.
    pub fn record_audit(&self, entry: &AuditEntry) -> Result<()> {
        self.store.append_audit(entry)?;
        Ok(())
    }

    /// Query audit entries.
    pub fn query_audit(
        &self,
        session_id: Option<&Id>,
        limit: Option<u64>,
    ) -> Result<Vec<AuditEntry>> {
        self.store.query_audit(session_id, limit)
    }
}

// ── Shared handle ─────────────────────────────────────────────────────────────

use std::sync::{Arc, Mutex};

/// Thread-safe shared handle to a `PolicyEngine`.
#[derive(Clone)]
pub struct PolicyEngineHandle(pub Arc<Mutex<PolicyEngine>>);

impl PolicyEngineHandle {
    pub fn new(engine: PolicyEngine) -> Self {
        Self(Arc::new(Mutex::new(engine)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PolicyCondition, PolicyDecision, PolicyScope};

    fn engine() -> PolicyEngine {
        let store = PolicyStore::open_in_memory().unwrap();
        PolicyEngine::new(store).unwrap()
    }

    // ── Never-auto-approve boundary ─────────────────────────────────────

    #[test]
    fn never_auto_approve_rm() {
        assert!(is_never_auto_approve("rm"));
        assert!(is_never_auto_approve("rm -rf /"));
    }

    #[test]
    fn never_auto_approve_sudo() {
        assert!(is_never_auto_approve("sudo"));
        assert!(is_never_auto_approve("sudo rm -rf /"));
    }

    #[test]
    fn never_auto_approve_curl() {
        assert!(is_never_auto_approve("curl"));
        assert!(is_never_auto_approve("curl https://evil.com"));
    }

    #[test]
    fn never_auto_approve_git_force() {
        assert!(is_never_auto_approve("git_push_force"));
        assert!(is_never_auto_approve("git push --force"));
    }

    #[test]
    fn never_auto_approve_publish() {
        assert!(is_never_auto_approve("npm_publish"));
        assert!(is_never_auto_approve("cargo_publish"));
        assert!(is_never_auto_approve("cargo publish"));
    }

    #[test]
    fn safe_tool_not_blocked() {
        assert!(!is_never_auto_approve("read_file"));
        assert!(!is_never_auto_approve("git_status"));
        assert!(!is_never_auto_approve("list_files"));
    }

    // ── Policy CRUD ─────────────────────────────────────────────────────

    #[test]
    fn create_and_get_policy() {
        let mut eng = engine();
        let policy = Policy::new(
            "allow-read".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNamePrefix("read_".into())],
            PolicyDecision::Allow,
        );
        let id = eng.create_policy(policy).unwrap();
        let loaded = eng.get_policy(&id).unwrap();
        assert_eq!(loaded.name, "allow-read");
        assert_eq!(loaded.decision, PolicyDecision::Allow);
    }

    #[test]
    fn list_policies() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "p1".into(),
            PolicyScope::Global,
            10,
            vec![],
            PolicyDecision::Allow,
        ))
        .unwrap();
        eng.create_policy(Policy::new(
            "p2".into(),
            PolicyScope::Global,
            20,
            vec![],
            PolicyDecision::Deny,
        ))
        .unwrap();
        let list = eng.list_policies(None);
        assert_eq!(list.len(), 2);
        // Higher priority first
        assert_eq!(list[0].name, "p2");
    }

    #[test]
    fn update_policy() {
        let mut eng = engine();
        let id = eng
            .create_policy(Policy::new(
                "old-name".into(),
                PolicyScope::Global,
                10,
                vec![],
                PolicyDecision::Allow,
            ))
            .unwrap();
        eng.update_policy(&id, Some("new-name".into()), None, None, None, None)
            .unwrap();
        assert_eq!(eng.get_policy(&id).unwrap().name, "new-name");
    }

    #[test]
    fn remove_policy() {
        let mut eng = engine();
        let id = eng
            .create_policy(Policy::new(
                "temp".into(),
                PolicyScope::Global,
                10,
                vec![],
                PolicyDecision::Allow,
            ))
            .unwrap();
        eng.remove_policy(&id).unwrap();
        assert!(eng.get_policy(&id).is_none());
    }

    #[test]
    fn remove_nonexistent_policy_fails() {
        let mut eng = engine();
        assert!(eng.remove_policy(&Id::from("nonexistent")).is_err());
    }

    // ── Policy evaluation ───────────────────────────────────────────────

    #[test]
    fn eval_allow_rule() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "allow-read".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNamePrefix("read_".into())],
            PolicyDecision::Allow,
        ))
        .unwrap();

        let result = eng.evaluate(Some("read_file"), None, None);
        assert_eq!(result.decision, PolicyDecision::Allow);
        assert!(result.matched_policy.is_some());
    }

    #[test]
    fn eval_deny_rule() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "deny-write".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNamePrefix("write_".into())],
            PolicyDecision::Deny,
        ))
        .unwrap();

        let result = eng.evaluate(Some("write_file"), None, None);
        assert_eq!(result.decision, PolicyDecision::Deny);
    }

    #[test]
    fn eval_require_human_rule() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "human-deploy".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNameEquals("deploy_staging".into())],
            PolicyDecision::RequireHuman,
        ))
        .unwrap();

        let result = eng.evaluate(Some("deploy_staging"), None, None);
        assert_eq!(result.decision, PolicyDecision::RequireHuman);
    }

    #[test]
    fn eval_deny_takes_precedence_over_allow() {
        let mut eng = engine();
        // Both match "write_file"
        eng.create_policy(Policy::new(
            "allow-all".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNamePrefix("write_".into())],
            PolicyDecision::Allow,
        ))
        .unwrap();
        eng.create_policy(Policy::new(
            "deny-write".into(),
            PolicyScope::Global,
            5, // Lower priority, but Deny takes precedence over Allow
            vec![PolicyCondition::ToolNamePrefix("write_".into())],
            PolicyDecision::Deny,
        ))
        .unwrap();

        let result = eng.evaluate(Some("write_file"), None, None);
        assert_eq!(result.decision, PolicyDecision::Deny);
    }

    #[test]
    fn eval_never_auto_approve_overrides_allow() {
        let mut eng = engine();
        // Create a very permissive allow-all policy
        eng.create_policy(Policy::new(
            "allow-everything".into(),
            PolicyScope::Global,
            1000,   // Very high priority
            vec![], // Matches everything
            PolicyDecision::Allow,
        ))
        .unwrap();

        // Even with this policy, never-auto-approve tools must RequireHuman
        let result = eng.evaluate(Some("rm"), None, None);
        assert_eq!(result.decision, PolicyDecision::RequireHuman);
        assert!(result.reason.contains("never-auto-approve"));

        let result = eng.evaluate(Some("sudo"), None, None);
        assert_eq!(result.decision, PolicyDecision::RequireHuman);

        let result = eng.evaluate(Some("curl"), None, None);
        assert_eq!(result.decision, PolicyDecision::RequireHuman);
    }

    #[test]
    fn eval_no_matching_policy_defaults_to_require_human() {
        let eng = engine();
        let result = eng.evaluate(Some("unknown_tool"), None, None);
        assert_eq!(result.decision, PolicyDecision::RequireHuman);
        assert!(result.reason.contains("default"));
    }

    #[test]
    fn eval_disabled_policy_ignored() {
        let mut eng = engine();
        let id = eng
            .create_policy(Policy::new(
                "allow-read".into(),
                PolicyScope::Global,
                10,
                vec![PolicyCondition::ToolNamePrefix("read_".into())],
                PolicyDecision::Allow,
            ))
            .unwrap();
        eng.update_policy(&id, None, None, None, None, Some(false))
            .unwrap();

        let result = eng.evaluate(Some("read_file"), None, None);
        assert_eq!(result.decision, PolicyDecision::RequireHuman); // disabled, so no match
    }

    #[test]
    fn eval_determinism() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "allow-read".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNamePrefix("read_".into())],
            PolicyDecision::Allow,
        ))
        .unwrap();
        eng.create_policy(Policy::new(
            "deny-write".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNamePrefix("write_".into())],
            PolicyDecision::Deny,
        ))
        .unwrap();

        // Same inputs → same outputs (run 10 times)
        for _ in 0..10 {
            let r1 = eng.evaluate(Some("read_file"), None, None);
            assert_eq!(r1.decision, PolicyDecision::Allow);

            let r2 = eng.evaluate(Some("write_file"), None, None);
            assert_eq!(r2.decision, PolicyDecision::Deny);

            let r3 = eng.evaluate(Some("unknown"), None, None);
            assert_eq!(r3.decision, PolicyDecision::RequireHuman);
        }
    }

    #[test]
    fn eval_condition_tool_name_equals() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "exact-match".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNameEquals("specific_tool".into())],
            PolicyDecision::Allow,
        ))
        .unwrap();

        let r1 = eng.evaluate(Some("specific_tool"), None, None);
        assert_eq!(r1.decision, PolicyDecision::Allow);

        let r2 = eng.evaluate(Some("specific_tool_extended"), None, None);
        assert_eq!(r2.decision, PolicyDecision::RequireHuman); // Not an exact match
    }

    #[test]
    fn eval_condition_tool_name_in() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "tool-list".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::ToolNameIn(vec![
                "tool_a".into(),
                "tool_b".into(),
            ])],
            PolicyDecision::Allow,
        ))
        .unwrap();

        assert_eq!(
            eng.evaluate(Some("tool_a"), None, None).decision,
            PolicyDecision::Allow
        );
        assert_eq!(
            eng.evaluate(Some("tool_b"), None, None).decision,
            PolicyDecision::Allow
        );
        assert_eq!(
            eng.evaluate(Some("tool_c"), None, None).decision,
            PolicyDecision::RequireHuman
        );
    }

    #[test]
    fn eval_condition_agent_type() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "mock-only".into(),
            PolicyScope::Global,
            10,
            vec![PolicyCondition::AgentTypeEquals("mock".into())],
            PolicyDecision::Allow,
        ))
        .unwrap();

        let r1 = eng.evaluate(Some("any_tool"), Some("mock"), None);
        assert_eq!(r1.decision, PolicyDecision::Allow);

        let r2 = eng.evaluate(Some("any_tool"), Some("claude"), None);
        assert_eq!(r2.decision, PolicyDecision::RequireHuman);
    }

    #[test]
    fn eval_multiple_conditions_conjunction() {
        let mut eng = engine();
        eng.create_policy(Policy::new(
            "specific".into(),
            PolicyScope::Global,
            10,
            vec![
                PolicyCondition::ToolNamePrefix("read_".into()),
                PolicyCondition::AgentTypeEquals("mock".into()),
            ],
            PolicyDecision::Allow,
        ))
        .unwrap();

        // Both conditions match
        assert_eq!(
            eng.evaluate(Some("read_file"), Some("mock"), None).decision,
            PolicyDecision::Allow
        );
        // Only one condition matches
        assert_eq!(
            eng.evaluate(Some("read_file"), Some("claude"), None)
                .decision,
            PolicyDecision::RequireHuman
        );
        // Neither condition matches
        assert_eq!(
            eng.evaluate(Some("write_file"), Some("claude"), None)
                .decision,
            PolicyDecision::RequireHuman
        );
    }

    // ── Audit ───────────────────────────────────────────────────────────

    #[test]
    fn audit_entry_recorded() {
        let eng = engine();
        let entry = AuditEntry::new(
            Some(Id::from("int-1")),
            Some(Id::from("sess-1")),
            "policy:p1".into(),
            "allow".into(),
            Some("Allowed by policy 'allow-read'".into()),
        );
        eng.record_audit(&entry).unwrap();

        let entries = eng.query_audit(Some(&Id::from("sess-1")), None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, "policy:p1");
        assert_eq!(entries[0].action, "allow");
    }

    #[test]
    fn audit_entries_never_contain_secrets() {
        let eng = engine();
        let entry = AuditEntry::new(
            Some(Id::from("int-1")),
            Some(Id::from("sess-1")),
            "human:operator".into(),
            "allow".into(),
            Some("Approved the action".into()),
        );
        eng.record_audit(&entry).unwrap();

        let entries = eng.query_audit(None, None).unwrap();
        // Verify no secret-like patterns in audit data
        for e in &entries {
            assert!(!e.actor.contains("sk-"));
            assert!(!e.actor.contains("api_key"));
            assert!(!e.action.contains("sk-"));
        }
    }

    // ── Policy persistence ──────────────────────────────────────────────

    #[test]
    fn policy_persistence_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("policies.db");
        let id;
        {
            let store = PolicyStore::open(&db).unwrap();
            let mut eng = PolicyEngine::new(store).unwrap();
            id = eng
                .create_policy(Policy::new(
                    "persistent".into(),
                    PolicyScope::Global,
                    42,
                    vec![PolicyCondition::ToolNameEquals("test".into())],
                    PolicyDecision::Allow,
                ))
                .unwrap();
        }
        {
            let store = PolicyStore::open(&db).unwrap();
            let eng = PolicyEngine::new(store).unwrap();
            let p = eng.get_policy(&id).unwrap();
            assert_eq!(p.name, "persistent");
            assert_eq!(p.priority, 42);
        }
    }
}
