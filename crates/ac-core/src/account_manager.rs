//! Account Manager — owns the pool of provider accounts.
//!
//! Responsibilities:
//!  - CRUD for `Account` entities (persisted to SQLite via `AccountStore`).
//!  - Account state machine: Active ↔ RateLimited / Cooldown / Exhausted / Invalid / Disabled.
//!  - Account selection: least-loaded, tag-filtered, agent-type-compatible.
//!  - Cooldown tracking: clear a cooldown when `cooldown_until` has passed.

use anyhow::{bail, Result};
use chrono::Utc;
use std::collections::HashMap;
use tracing::{debug, info, warn};

use crate::types::{Account, AccountAvailability, AccountState, AgentEvent, EventKind, Id};

// ── Persistence (SQLite) ──────────────────────────────────────────────────────

use rusqlite::{params, Connection};
use std::path::Path;

/// Persistent store for Account entities.
pub struct AccountStore {
    conn: Connection,
}

impl AccountStore {
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
             CREATE TABLE IF NOT EXISTS accounts (
                 id                  TEXT PRIMARY KEY,
                 label               TEXT NOT NULL,
                 provider            TEXT NOT NULL,
                 agent_types         TEXT NOT NULL,   -- JSON array
                 credential_ref      TEXT NOT NULL,
                 state               TEXT NOT NULL DEFAULT 'active',
                 concurrency_cap     INTEGER NOT NULL DEFAULT 1,
                 active_session_count INTEGER NOT NULL DEFAULT 0,
                 cooldown_until      TEXT,
                 tags                TEXT NOT NULL DEFAULT '[]', -- JSON array
                 notes               TEXT,
                 created_at          TEXT NOT NULL,
                 updated_at          TEXT NOT NULL
             );",
        )?;
        Ok(())
    }

    pub fn insert(&self, account: &Account) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts
             (id, label, provider, agent_types, credential_ref, state, concurrency_cap,
              active_session_count, cooldown_until, tags, notes, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                account.id.0,
                account.label,
                account.provider,
                serde_json::to_string(&account.agent_types)?,
                account.credential_ref,
                serde_json::to_string(&account.state)?,
                account.concurrency_cap as i64,
                account.active_session_count as i64,
                account.cooldown_until.map(|d| d.to_rfc3339()),
                serde_json::to_string(&account.tags)?,
                account.notes,
                account.created_at.to_rfc3339(),
                account.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn update(&self, account: &Account) -> Result<()> {
        self.conn.execute(
            "UPDATE accounts SET
             label=?2, state=?3, concurrency_cap=?4, active_session_count=?5,
             cooldown_until=?6, tags=?7, notes=?8, updated_at=?9
             WHERE id=?1",
            params![
                account.id.0,
                account.label,
                serde_json::to_string(&account.state)?,
                account.concurrency_cap as i64,
                account.active_session_count as i64,
                account.cooldown_until.map(|d| d.to_rfc3339()),
                serde_json::to_string(&account.tags)?,
                account.notes,
                account.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: &Id) -> Result<()> {
        self.conn.execute("DELETE FROM accounts WHERE id=?1", params![id.0])?;
        Ok(())
    }

    pub fn load_all(&self) -> Result<Vec<Account>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, provider, agent_types, credential_ref, state,
                    concurrency_cap, active_session_count, cooldown_until,
                    tags, notes, created_at, updated_at
             FROM accounts ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], row_to_account)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn exists(&self, id: &Id) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM accounts WHERE id=?1",
            params![id.0],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let id: String = row.get(0)?;
    let label: String = row.get(1)?;
    let provider: String = row.get(2)?;
    let agent_types_str: String = row.get(3)?;
    let credential_ref: String = row.get(4)?;
    let state_str: String = row.get(5)?;
    let concurrency_cap: i64 = row.get(6)?;
    let active_session_count: i64 = row.get(7)?;
    let cooldown_until_str: Option<String> = row.get(8)?;
    let tags_str: String = row.get(9)?;
    let notes: Option<String> = row.get(10)?;
    let created_at_str: String = row.get(11)?;
    let updated_at_str: String = row.get(12)?;

    let agent_types: Vec<String> =
        serde_json::from_str(&agent_types_str).unwrap_or_default();
    let state: AccountState =
        serde_json::from_str(&state_str).unwrap_or(AccountState::Active);
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
    let cooldown_until = cooldown_until_str.and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    });
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    Ok(Account {
        id: Id(id),
        label,
        provider,
        agent_types,
        credential_ref,
        state,
        concurrency_cap: concurrency_cap as u8,
        active_session_count: active_session_count as u8,
        cooldown_until,
        tags,
        notes,
        created_at,
        updated_at,
    })
}

// ── AccountManager ────────────────────────────────────────────────────────────

/// In-memory account pool with persistence.
pub struct AccountManager {
    accounts: HashMap<String, Account>,
    store: AccountStore,
}

impl AccountManager {
    pub fn new(store: AccountStore) -> Result<Self> {
        let accounts_vec = store.load_all()?;
        let mut accounts = HashMap::new();
        for a in accounts_vec {
            accounts.insert(a.id.0.clone(), a);
        }
        info!("AccountManager loaded {} accounts", accounts.len());
        Ok(Self { accounts, store })
    }

    // ── CRUD ──────────────────────────────────────────────────────────────

    /// Register a new account.
    pub fn register(&mut self, account: Account) -> Result<Id> {
        let id = account.id.clone();
        self.store.insert(&account)?;
        self.accounts.insert(id.0.clone(), account);
        info!("Account registered: {}", id);
        Ok(id)
    }

    /// Get a reference to an account by id.
    pub fn get(&self, id: &Id) -> Option<&Account> {
        self.accounts.get(&id.0)
    }

    /// List all accounts, optionally filtered by state and/or tags.
    pub fn list(&self, state_filter: Option<&AccountState>, tag_filter: &[String]) -> Vec<&Account> {
        self.accounts
            .values()
            .filter(|a| {
                if let Some(s) = state_filter {
                    if &a.state != s {
                        return false;
                    }
                }
                if !tag_filter.is_empty()
                    && !tag_filter.iter().any(|t| a.tags.contains(t))
                {
                    return false;
                }
                true
            })
            .collect()
    }

    /// Find all accounts matching a friendly label (case-insensitive).
    pub fn find_by_label(&self, label: &str) -> Vec<&Account> {
        let needle = label.trim();
        self.accounts
            .values()
            .filter(|a| a.label.eq_ignore_ascii_case(needle))
            .collect()
    }

    /// List all accounts that support a given agent type.
    pub fn accounts_for_agent_type(&self, agent_type: &str) -> Vec<&Account> {
        self.accounts
            .values()
            .filter(|a| a.supports_agent_type(agent_type))
            .collect()
    }

    /// Update mutable display fields.
    pub fn update_label(&mut self, id: &Id, label: String) -> Result<()> {
        let account = self
            .accounts
            .get_mut(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
        account.label = label;
        account.updated_at = Utc::now();
        self.store.update(account)?;
        Ok(())
    }

    /// Update concurrency cap.
    pub fn update_concurrency_cap(&mut self, id: &Id, cap: u8) -> Result<()> {
        let account = self
            .accounts
            .get_mut(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
        account.concurrency_cap = cap;
        account.updated_at = Utc::now();
        self.store.update(account)?;
        Ok(())
    }

    /// Remove an account. Only allowed if no active sessions.
    pub fn remove(&mut self, id: &Id) -> Result<()> {
        let account = self
            .accounts
            .get(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
        if account.active_session_count > 0 {
            bail!("Cannot remove account {} with {} active session(s)", id, account.active_session_count);
        }
        self.store.delete(id)?;
        self.accounts.remove(&id.0);
        info!("Account removed: {}", id);
        Ok(())
    }

    // ── State transitions ─────────────────────────────────────────────────

    /// Disable an account manually.
    pub fn disable(&mut self, id: &Id) -> Result<()> {
        self.set_state(id, AccountState::Disabled)?;
        info!("Account disabled: {}", id);
        Ok(())
    }

    /// Re-enable a disabled account.
    pub fn enable(&mut self, id: &Id) -> Result<()> {
        let account = self
            .accounts
            .get(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
        if !matches!(account.state, AccountState::Disabled) {
            bail!("Account {} is not Disabled (state={})", id, account.state);
        }
        self.set_state(id, AccountState::Active)?;
        info!("Account enabled: {}", id);
        Ok(())
    }

    /// Apply a rate-limit signal; transitions Active → RateLimited.
    /// Optionally sets a cooldown_until timestamp.
    pub fn apply_rate_limit(
        &mut self,
        id: &Id,
        cooldown_until: Option<chrono::DateTime<Utc>>,
    ) -> Result<()> {
        {
            let account = self
                .accounts
                .get_mut(&id.0)
                .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
            account.state = AccountState::RateLimited;
            account.cooldown_until = cooldown_until;
            account.updated_at = Utc::now();
        }
        let account = self.accounts.get(&id.0).unwrap();
        self.store.update(account)?;
        warn!("Account {} rate-limited until {:?}", id, cooldown_until);
        Ok(())
    }

    /// Mark account as exhausted (quota = 0).
    pub fn mark_exhausted(&mut self, id: &Id) -> Result<()> {
        self.set_state(id, AccountState::Exhausted)?;
        warn!("Account {} exhausted", id);
        Ok(())
    }

    /// Start a cooldown period (transitions to Cooldown state).
    pub fn start_cooldown(
        &mut self,
        id: &Id,
        cooldown_until: chrono::DateTime<Utc>,
    ) -> Result<()> {
        {
            let account = self
                .accounts
                .get_mut(&id.0)
                .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
            account.state = AccountState::Cooldown;
            account.cooldown_until = Some(cooldown_until);
            account.updated_at = Utc::now();
        }
        let account = self.accounts.get(&id.0).unwrap();
        self.store.update(account)?;
        info!("Account {} in cooldown until {}", id, cooldown_until);
        Ok(())
    }

    /// Expire a cooldown: if `cooldown_until` has passed, transitions back to Active.
    /// Returns true if the cooldown was cleared.
    pub fn maybe_expire_cooldown(&mut self, id: &Id) -> Result<bool> {
        let (should_clear, _) = {
            let account = self
                .accounts
                .get(&id.0)
                .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
            let in_cooldown = matches!(
                account.state,
                AccountState::Cooldown | AccountState::RateLimited
            );
            let expired = account
                .cooldown_until
                .map(|t| Utc::now() >= t)
                .unwrap_or(false);
            (in_cooldown && expired, account.id.clone())
        };
        if should_clear {
            self.set_state(id, AccountState::Active)?;
            {
                let account = self.accounts.get_mut(&id.0).unwrap();
                account.cooldown_until = None;
                account.updated_at = Utc::now();
            }
            let account = self.accounts.get(&id.0).unwrap();
            self.store.update(account)?;
            info!("Account {} cooldown expired → Active", id);
            return Ok(true);
        }
        Ok(false)
    }

    // ── Session tracking ──────────────────────────────────────────────────

    /// Increment active_session_count when a session is assigned.
    pub fn increment_sessions(&mut self, id: &Id) -> Result<()> {
        let account = self
            .accounts
            .get_mut(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
        account.active_session_count = account.active_session_count.saturating_add(1);
        account.updated_at = Utc::now();
        self.store.update(account)?;
        debug!("Account {} sessions: {}", id, account.active_session_count);
        Ok(())
    }

    /// Decrement active_session_count when a session terminates.
    pub fn decrement_sessions(&mut self, id: &Id) -> Result<()> {
        let account = self
            .accounts
            .get_mut(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
        account.active_session_count = account.active_session_count.saturating_sub(1);
        account.updated_at = Utc::now();
        self.store.update(account)?;
        debug!("Account {} sessions: {}", id, account.active_session_count);
        Ok(())
    }

    // ── Account selection ─────────────────────────────────────────────────

    /// Explicit selection: validate that an account exists, is active, supports
    /// the agent type, and has capacity under its concurrency limit.
    /// Returns structured errors when requirements are not satisfied.
    pub fn validate_and_select_explicit(
        &self,
        account_id: &Id,
        agent_type: &str,
    ) -> Result<&Account> {
        let account = self
            .accounts
            .get(&account_id.0)
            .or_else(|| self.accounts.values().find(|a| a.label.eq_ignore_ascii_case(&account_id.0)))
            .ok_or_else(|| {
                anyhow::anyhow!("AccountNotFound: account {} not found", account_id)
            })?;

        if !account.supports_agent_type(agent_type) {
            bail!(
                "IncompatibleAccount: account '{}' ({}) does not support agent_type '{}'",
                account.label,
                account.id,
                agent_type
            );
        }

        if !account.state.is_available() {
            bail!(
                "AccountUnavailable: account '{}' ({}) is in state '{}'",
                account.label,
                account.id,
                account.state
            );
        }

        if account.active_session_count >= account.concurrency_cap {
            bail!(
                "ConcurrencyLimitReached: account '{}' ({}) is at capacity ({}/{})",
                account.label,
                account.id,
                account.active_session_count,
                account.concurrency_cap
            );
        }

        Ok(account)
    }

    /// Select the best available account for a new session.
    ///
    /// Selection criteria (in order):
    /// 1. State must be Active.
    /// 2. Must have capacity (`active_session_count < concurrency_cap`).
    /// 3. Must support the required `agent_type`.
    /// 4. Must match at least one tag in `required_tags` (if non-empty).
    /// 5. Among qualifying accounts, pick the one with the fewest active sessions
    ///    (least-loaded), with a stable tie-breaker for deterministic selection.
    ///
    /// Returns `Err` with `NoAccountAvailable` message when no account qualifies.
    pub fn select(
        &self,
        agent_type: &str,
        required_tags: &[String],
    ) -> Result<Id> {
        let best = self
            .accounts
            .values()
            .filter(|a| a.can_accept_session())
            .filter(|a| a.supports_agent_type(agent_type))
            .filter(|a| {
                required_tags.is_empty()
                    || required_tags.iter().any(|t| a.tags.contains(t))
            })
            .min_by(|a, b| {
                a.active_session_count
                    .cmp(&b.active_session_count)
                    .then_with(|| a.label.cmp(&b.label))
                    .then_with(|| a.id.0.cmp(&b.id.0))
            });

        match best {
            Some(a) => {
                debug!("Selected account {} (load={}/{})", a.id, a.active_session_count, a.concurrency_cap);
                Ok(a.id.clone())
            }
            None => bail!(
                "NoAccountAvailable: no active account supports agent_type='{}' with tags={:?}",
                agent_type,
                required_tags
            ),
        }
    }

    /// Query availability descriptors for all registered accounts against optional
    /// agent_type and tags filters.
    pub fn query_availability(
        &self,
        agent_type: Option<&str>,
        tags: &[String],
    ) -> Vec<AccountAvailability> {
        let mut results = Vec::new();
        for a in self.accounts.values() {
            let mut reason = None;
            if !a.state.is_available() {
                reason = Some(format!("Account is {}", a.state));
            } else if a.active_session_count >= a.concurrency_cap {
                reason = Some(format!("At concurrency limit ({}/{})", a.active_session_count, a.concurrency_cap));
            } else if let Some(at) = agent_type {
                if !a.supports_agent_type(at) {
                    reason = Some(format!("Does not support agent_type '{}'", at));
                }
            } else if !tags.is_empty() && !tags.iter().any(|t| a.tags.contains(t)) {
                reason = Some(format!("Missing required tags: {:?}", tags));
            }

            let is_available = reason.is_none();
            results.push(AccountAvailability {
                account_id: a.id.clone(),
                label: a.label.clone(),
                provider: a.provider.clone(),
                agent_types: a.agent_types.clone(),
                state: a.state.clone(),
                active_session_count: a.active_session_count,
                concurrency_cap: a.concurrency_cap,
                cooldown_until: a.cooldown_until,
                is_available,
                reason,
            });
        }
        // Sort deterministically by label
        results.sort_by(|a, b| a.label.cmp(&b.label));
        results
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    fn set_state(&mut self, id: &Id, state: AccountState) -> Result<()> {
        let account = self
            .accounts
            .get_mut(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Account not found: {}", id))?;
        account.state = state;
        account.updated_at = Utc::now();
        self.store.update(account)?;
        Ok(())
    }

    /// Build a StateChanged event for an account transition.
    pub fn make_state_event(id: &Id, new_state: &AccountState) -> AgentEvent {
        AgentEvent::new(
            EventKind::AccountStateChanged,
            None,
            serde_json::json!({
                "account_id": id,
                "state": new_state,
            }),
            "system",
        )
    }
}

// ── Shared handle ─────────────────────────────────────────────────────────────

use std::sync::{Arc, Mutex};

/// Thread-safe shared handle to an `AccountManager`.
/// Allows the IPC server and session manager to share one manager instance.
#[derive(Clone)]
pub struct AccountManagerHandle(pub Arc<Mutex<AccountManager>>);

impl AccountManagerHandle {
    pub fn new(mgr: AccountManager) -> Self {
        Self(Arc::new(Mutex::new(mgr)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Account;
    use chrono::Duration;

    fn make_account(label: &str, tags: Vec<&str>) -> Account {
        Account::new(
            label.into(),
            "anthropic".into(),
            vec!["mock".into()],
            "ref:test".into(),
            2,
            tags.into_iter().map(String::from).collect(),
        )
    }

    fn manager() -> AccountManager {
        let store = AccountStore::open_in_memory().unwrap();
        AccountManager::new(store).unwrap()
    }

    #[test]
    fn register_and_get() {
        let mut mgr = manager();
        let acct = make_account("test", vec!["a"]);
        let id = mgr.register(acct).unwrap();
        let loaded = mgr.get(&id).unwrap();
        assert_eq!(loaded.label, "test");
        assert_eq!(loaded.state, AccountState::Active);
    }

    #[test]
    fn list_all() {
        let mut mgr = manager();
        mgr.register(make_account("a1", vec![])).unwrap();
        mgr.register(make_account("a2", vec![])).unwrap();
        assert_eq!(mgr.list(None, &[]).len(), 2);
    }

    #[test]
    fn list_filter_by_state() {
        let mut mgr = manager();
        let id1 = mgr.register(make_account("active", vec![])).unwrap();
        let _id2 = mgr.register(make_account("disabled", vec![])).unwrap();
        mgr.disable(&_id2).unwrap();
        let active = mgr.list(Some(&AccountState::Active), &[]);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, id1);
    }

    #[test]
    fn list_filter_by_tag() {
        let mut mgr = manager();
        mgr.register(make_account("tagged", vec!["anthropic"])).unwrap();
        mgr.register(make_account("other", vec!["openai"])).unwrap();
        let filtered = mgr.list(None, &["anthropic".to_string()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].label, "tagged");
    }

    #[test]
    fn disable_and_enable() {
        let mut mgr = manager();
        let id = mgr.register(make_account("a", vec![])).unwrap();
        mgr.disable(&id).unwrap();
        assert_eq!(mgr.get(&id).unwrap().state, AccountState::Disabled);
        mgr.enable(&id).unwrap();
        assert_eq!(mgr.get(&id).unwrap().state, AccountState::Active);
    }

    #[test]
    fn enable_non_disabled_fails() {
        let mut mgr = manager();
        let id = mgr.register(make_account("a", vec![])).unwrap();
        assert!(mgr.enable(&id).is_err(), "enabling an already-active account should fail");
    }

    #[test]
    fn select_least_loaded() {
        let mut mgr = manager();
        let id1 = mgr.register(make_account("heavy", vec![])).unwrap();
        let id2 = mgr.register(make_account("light", vec![])).unwrap();
        // Give id1 one active session
        mgr.increment_sessions(&id1).unwrap();
        let selected = mgr.select("mock", &[]).unwrap();
        assert_eq!(selected, id2, "Should pick the least-loaded account");
    }

    #[test]
    fn select_respects_agent_type() {
        let mut mgr = manager();
        let _id_wrong = mgr.register(Account::new(
            "wrong-type".into(),
            "anthropic".into(),
            vec!["codex".into()],
            "ref".into(),
            2,
            vec![],
        )).unwrap();
        let id_ok = mgr.register(Account::new(
            "right-type".into(),
            "anthropic".into(),
            vec!["mock".into()],
            "ref".into(),
            2,
            vec![],
        )).unwrap();
        let selected = mgr.select("mock", &[]).unwrap();
        assert_eq!(selected, id_ok);
    }

    #[test]
    fn select_respects_tags() {
        let mut mgr = manager();
        mgr.register(make_account("wrong-tag", vec!["openai"])).unwrap();
        let id_ok = mgr.register(make_account("right-tag", vec!["anthropic"])).unwrap();
        let selected = mgr.select("mock", &["anthropic".to_string()]).unwrap();
        assert_eq!(selected, id_ok);
    }

    #[test]
    fn select_rejects_disabled() {
        let mut mgr = manager();
        let id = mgr.register(make_account("a", vec![])).unwrap();
        mgr.disable(&id).unwrap();
        assert!(mgr.select("mock", &[]).is_err());
    }

    #[test]
    fn select_rejects_at_capacity() {
        let mut mgr = manager();
        let mut a = make_account("full", vec![]);
        a.concurrency_cap = 1;
        let id = mgr.register(a).unwrap();
        mgr.increment_sessions(&id).unwrap();
        assert!(mgr.select("mock", &[]).is_err(), "Full account should not be selected");
    }

    #[test]
    fn no_accounts_returns_error() {
        let mgr = manager();
        assert!(mgr.select("mock", &[]).is_err());
    }

    #[test]
    fn cooldown_expiry() {
        let mut mgr = manager();
        let id = mgr.register(make_account("a", vec![])).unwrap();
        // Set cooldown in the past
        let past = Utc::now() - Duration::seconds(10);
        mgr.start_cooldown(&id, past).unwrap();
        assert_eq!(mgr.get(&id).unwrap().state, AccountState::Cooldown);
        let cleared = mgr.maybe_expire_cooldown(&id).unwrap();
        assert!(cleared);
        assert_eq!(mgr.get(&id).unwrap().state, AccountState::Active);
    }

    #[test]
    fn cooldown_not_expired_stays() {
        let mut mgr = manager();
        let id = mgr.register(make_account("a", vec![])).unwrap();
        let future = Utc::now() + Duration::seconds(3600);
        mgr.start_cooldown(&id, future).unwrap();
        let cleared = mgr.maybe_expire_cooldown(&id).unwrap();
        assert!(!cleared);
        assert_eq!(mgr.get(&id).unwrap().state, AccountState::Cooldown);
    }

    #[test]
    fn rate_limit_signal() {
        let mut mgr = manager();
        let id = mgr.register(make_account("a", vec![])).unwrap();
        mgr.apply_rate_limit(&id, None).unwrap();
        assert_eq!(mgr.get(&id).unwrap().state, AccountState::RateLimited);
    }

    #[test]
    fn remove_account_with_active_sessions_fails() {
        let mut mgr = manager();
        let id = mgr.register(make_account("a", vec![])).unwrap();
        mgr.increment_sessions(&id).unwrap();
        assert!(mgr.remove(&id).is_err());
    }

    #[test]
    fn remove_idle_account_succeeds() {
        let mut mgr = manager();
        let id = mgr.register(make_account("a", vec![])).unwrap();
        mgr.remove(&id).unwrap();
        assert!(mgr.get(&id).is_none());
    }

    #[test]
    fn persistence_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("accounts.db");
        let id;
        {
            let store = AccountStore::open(&db).unwrap();
            let mut mgr = AccountManager::new(store).unwrap();
            id = mgr.register(make_account("persisted", vec!["tag1"])).unwrap();
        }
        {
            let store = AccountStore::open(&db).unwrap();
            let mgr = AccountManager::new(store).unwrap();
            let a = mgr.get(&id).unwrap();
            assert_eq!(a.label, "persisted");
            assert!(a.tags.contains(&"tag1".to_string()));
        }
    }
}
