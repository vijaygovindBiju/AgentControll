//! Project Registry — owns the set of registered `Project`s and their `Workspace`s.
//!
//! Responsibilities:
//!  - CRUD for `Project` entities (persisted to SQLite).
//!  - Workspace resolution: for a new session, find or create the correct workspace.
//!    - `Shared` policy: return the project's shared workspace (create if not yet exists).
//!    - `WorktreePerSession` policy: create a new workspace record (actual worktree
//!      creation is the adapter's responsibility in Phase 4; here we track it).
//!  - Workspace reclamation on session stop.

use anyhow::{bail, Result};
use chrono::Utc;
use std::collections::HashMap;
use tracing::info;

use crate::types::{Id, Project, Workspace, WorkspaceKind, WorkspacePolicy};

use rusqlite::{params, Connection};
use std::path::Path;

// ── Persistence ───────────────────────────────────────────────────────────────

pub struct ProjectStore {
    conn: Connection,
}

impl ProjectStore {
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
             CREATE TABLE IF NOT EXISTS projects (
                 id                   TEXT PRIMARY KEY,
                 name                 TEXT NOT NULL,
                 repo_path            TEXT NOT NULL,
                 default_agent_type   TEXT,
                 default_account_tags TEXT NOT NULL DEFAULT '[]',
                 workspace_policy     TEXT NOT NULL DEFAULT '\"shared\"',
                 notes                TEXT,
                 created_at           TEXT NOT NULL,
                 updated_at           TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS workspaces (
                 id             TEXT PRIMARY KEY,
                 project_id     TEXT NOT NULL,
                 kind           TEXT NOT NULL,
                 path           TEXT NOT NULL,
                 branch         TEXT,
                 session_id     TEXT,
                 created_at     TEXT NOT NULL,
                 reclaimed_at   TEXT
             );",
        )?;
        Ok(())
    }

    // Projects

    pub fn insert_project(&self, p: &Project) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects
             (id, name, repo_path, default_agent_type, default_account_tags,
              workspace_policy, notes, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                p.id.0,
                p.name,
                p.repo_path,
                p.default_agent_type,
                serde_json::to_string(&p.default_account_tags)?,
                serde_json::to_string(&p.workspace_policy)?,
                p.notes,
                p.created_at.to_rfc3339(),
                p.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn update_project(&self, p: &Project) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET name=?2, default_agent_type=?3,
             default_account_tags=?4, workspace_policy=?5, notes=?6, updated_at=?7
             WHERE id=?1",
            params![
                p.id.0,
                p.name,
                p.default_agent_type,
                serde_json::to_string(&p.default_account_tags)?,
                serde_json::to_string(&p.workspace_policy)?,
                p.notes,
                p.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn delete_project(&self, id: &Id) -> Result<()> {
        self.conn.execute("DELETE FROM projects WHERE id=?1", params![id.0])?;
        Ok(())
    }

    pub fn load_all_projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, repo_path, default_agent_type, default_account_tags,
                    workspace_policy, notes, created_at, updated_at
             FROM projects ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], row_to_project)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // Workspaces

    pub fn insert_workspace(&self, w: &Workspace) -> Result<()> {
        self.conn.execute(
            "INSERT INTO workspaces
             (id, project_id, kind, path, branch, session_id, created_at, reclaimed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                w.id.0,
                w.project_id.0,
                serde_json::to_string(&w.kind)?,
                w.path,
                w.branch,
                w.session_id.as_ref().map(|id| &id.0),
                w.created_at.to_rfc3339(),
                w.reclaimed_at.map(|d| d.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn update_workspace(&self, w: &Workspace) -> Result<()> {
        self.conn.execute(
            "UPDATE workspaces SET session_id=?2, reclaimed_at=?3 WHERE id=?1",
            params![
                w.id.0,
                w.session_id.as_ref().map(|id| &id.0),
                w.reclaimed_at.map(|d| d.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    pub fn load_workspaces_for_project(&self, project_id: &Id) -> Result<Vec<Workspace>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, kind, path, branch, session_id, created_at, reclaimed_at
             FROM workspaces WHERE project_id=?1 ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![project_id.0], row_to_workspace)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn load_all_workspaces(&self) -> Result<Vec<Workspace>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, kind, path, branch, session_id, created_at, reclaimed_at
             FROM workspaces ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], row_to_workspace)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let id: String = row.get(0)?;
    let name: String = row.get(1)?;
    let repo_path: String = row.get(2)?;
    let default_agent_type: Option<String> = row.get(3)?;
    let tags_str: String = row.get(4)?;
    let policy_str: String = row.get(5)?;
    let notes: Option<String> = row.get(6)?;
    let created_at_str: String = row.get(7)?;
    let updated_at_str: String = row.get(8)?;

    let default_account_tags: Vec<String> =
        serde_json::from_str(&tags_str).unwrap_or_default();
    let workspace_policy: WorkspacePolicy =
        serde_json::from_str(&policy_str).unwrap_or_default();
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    Ok(Project {
        id: Id(id),
        name,
        repo_path,
        default_agent_type,
        default_account_tags,
        workspace_policy,
        notes,
        created_at,
        updated_at,
    })
}

fn row_to_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<Workspace> {
    let id: String = row.get(0)?;
    let project_id: String = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let path: String = row.get(3)?;
    let branch: Option<String> = row.get(4)?;
    let session_id: Option<String> = row.get(5)?;
    let created_at_str: String = row.get(6)?;
    let reclaimed_at_str: Option<String> = row.get(7)?;

    let kind: WorkspaceKind =
        serde_json::from_str(&kind_str).unwrap_or(WorkspaceKind::Shared);
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let reclaimed_at = reclaimed_at_str.and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    });

    Ok(Workspace {
        id: Id(id),
        project_id: Id(project_id),
        kind,
        path,
        branch,
        session_id: session_id.map(Id),
        created_at,
        reclaimed_at,
    })
}

// ── ProjectRegistry ───────────────────────────────────────────────────────────

pub struct ProjectRegistry {
    projects: HashMap<String, Project>,
    /// workspace_id → Workspace (in-memory cache for fast lookup)
    workspaces: HashMap<String, Workspace>,
    store: ProjectStore,
}

impl ProjectRegistry {
    pub fn new(store: ProjectStore) -> Result<Self> {
        let projects_vec = store.load_all_projects()?;
        let workspaces_vec = store.load_all_workspaces()?;

        let mut projects = HashMap::new();
        for p in projects_vec {
            projects.insert(p.id.0.clone(), p);
        }
        let mut workspaces = HashMap::new();
        for w in workspaces_vec {
            workspaces.insert(w.id.0.clone(), w);
        }

        info!(
            "ProjectRegistry loaded {} projects, {} workspaces",
            projects.len(),
            workspaces.len()
        );
        Ok(Self { projects, workspaces, store })
    }

    // ── Project CRUD ──────────────────────────────────────────────────────

    pub fn register(&mut self, project: Project) -> Result<Id> {
        let id = project.id.clone();
        self.store.insert_project(&project)?;
        self.projects.insert(id.0.clone(), project);
        info!("Project registered: {}", id);
        Ok(id)
    }

    pub fn get(&self, id: &Id) -> Option<&Project> {
        self.projects.get(&id.0)
    }

    pub fn list(&self) -> Vec<&Project> {
        let mut v: Vec<&Project> = self.projects.values().collect();
        v.sort_by_key(|p| &p.created_at);
        v
    }

    pub fn update(&mut self, id: &Id, name: Option<String>, default_agent_type: Option<Option<String>>, default_account_tags: Option<Vec<String>>) -> Result<()> {
        let project = self
            .projects
            .get_mut(&id.0)
            .ok_or_else(|| anyhow::anyhow!("Project not found: {}", id))?;
        if let Some(n) = name { project.name = n; }
        if let Some(dat) = default_agent_type { project.default_agent_type = dat; }
        if let Some(tags) = default_account_tags { project.default_account_tags = tags; }
        project.updated_at = Utc::now();
        self.store.update_project(project)?;
        Ok(())
    }

    pub fn remove(&mut self, id: &Id) -> Result<()> {
        // Check for bound workspaces with active sessions
        let has_active = self.workspaces.values()
            .any(|w| &w.project_id == id && w.session_id.is_some() && w.reclaimed_at.is_none());
        if has_active {
            bail!("Cannot remove project {} with active sessions", id);
        }
        self.store.delete_project(id)?;
        self.projects.remove(&id.0);
        info!("Project removed: {}", id);
        Ok(())
    }

    // ── Workspace resolution ──────────────────────────────────────────────

    /// Resolve or create a workspace for `session_id` in `project_id`.
    ///
    /// - `Shared` policy: returns the single shared workspace for this project
    ///   (creating it if it does not yet exist). The workspace is not bound to
    ///   a specific session.
    /// - `WorktreePerSession` policy: creates a new workspace record bound to
    ///   `session_id`. The path is `<repo_path>/.worktrees/<session_id>`.
    ///   Actual git worktree creation is deferred to Phase 4.
    pub fn resolve_workspace(
        &mut self,
        project_id: &Id,
        session_id: &Id,
    ) -> Result<Id> {
        let project = self
            .projects
            .get(&project_id.0)
            .ok_or_else(|| anyhow::anyhow!("Project not found: {}", project_id))?
            .clone();

        match project.workspace_policy {
            WorkspacePolicy::Shared => {
                // Return the existing shared workspace, or create it.
                let existing = self.workspaces.values().find(|w| {
                    &w.project_id == project_id
                        && w.kind == WorkspaceKind::Shared
                        && w.reclaimed_at.is_none()
                });
                if let Some(w) = existing {
                    return Ok(w.id.clone());
                }
                // Create the shared workspace
                let ws = Workspace {
                    id: Id::new(),
                    project_id: project_id.clone(),
                    kind: WorkspaceKind::Shared,
                    path: project.repo_path.clone(),
                    branch: None,
                    session_id: None, // shared — not bound to one session
                    created_at: Utc::now(),
                    reclaimed_at: None,
                };
                let ws_id = ws.id.clone();
                self.store.insert_workspace(&ws)?;
                self.workspaces.insert(ws_id.0.clone(), ws);
                Ok(ws_id)
            }
            WorkspacePolicy::WorktreePerSession => {
                let path = format!(
                    "{}/.worktrees/{}",
                    project.repo_path, session_id.0
                );
                let ws = Workspace {
                    id: Id::new(),
                    project_id: project_id.clone(),
                    kind: WorkspaceKind::Worktree,
                    path,
                    branch: Some(format!("session/{}", &session_id.0[..8])),
                    session_id: Some(session_id.clone()),
                    created_at: Utc::now(),
                    reclaimed_at: None,
                };
                let ws_id = ws.id.clone();
                self.store.insert_workspace(&ws)?;
                self.workspaces.insert(ws_id.0.clone(), ws);
                Ok(ws_id)
            }
        }
    }

    /// Reclaim (logically free) a workspace when a session ends.
    /// For Worktree workspaces, marks the workspace as reclaimed.
    /// For Shared workspaces, this is a no-op (it remains available).
    pub fn reclaim_workspace(&mut self, workspace_id: &Id) -> Result<()> {
        let ws = self
            .workspaces
            .get_mut(&workspace_id.0)
            .ok_or_else(|| anyhow::anyhow!("Workspace not found: {}", workspace_id))?;
        if ws.kind == WorkspaceKind::Worktree {
            ws.reclaimed_at = Some(Utc::now());
            ws.session_id = None;
            self.store.update_workspace(ws)?;
        }
        Ok(())
    }

    /// Get a workspace by its ID.
    pub fn get_workspace(&self, id: &Id) -> Option<&Workspace> {
        self.workspaces.get(&id.0)
    }

    /// List workspaces for a project.
    pub fn workspaces_for(&self, project_id: &Id) -> Vec<&Workspace> {
        let mut v: Vec<&Workspace> = self.workspaces.values()
            .filter(|w| &w.project_id == project_id)
            .collect();
        v.sort_by_key(|w| w.created_at);
        v
    }
}

// ── Shared handle ─────────────────────────────────────────────────────────────

use std::sync::{Arc, Mutex};

/// Thread-safe shared handle to a `ProjectRegistry`.
#[derive(Clone)]
pub struct ProjectRegistryHandle(pub Arc<Mutex<ProjectRegistry>>);

impl ProjectRegistryHandle {
    pub fn new(reg: ProjectRegistry) -> Self {
        Self(Arc::new(Mutex::new(reg)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Project, WorkspacePolicy};

    fn make_project(name: &str, policy: WorkspacePolicy) -> Project {
        Project::new(
            name.into(),
            format!("/tmp/{name}"),
            Some("mock".into()),
            vec![],
            policy,
        )
    }

    fn registry() -> ProjectRegistry {
        let store = ProjectStore::open_in_memory().unwrap();
        ProjectRegistry::new(store).unwrap()
    }

    #[test]
    fn register_and_get() {
        let mut reg = registry();
        let p = make_project("test", WorkspacePolicy::Shared);
        let id = reg.register(p).unwrap();
        assert!(reg.get(&id).is_some());
        assert_eq!(reg.get(&id).unwrap().name, "test");
    }

    #[test]
    fn list_projects() {
        let mut reg = registry();
        reg.register(make_project("p1", WorkspacePolicy::Shared)).unwrap();
        reg.register(make_project("p2", WorkspacePolicy::Shared)).unwrap();
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn remove_project() {
        let mut reg = registry();
        let id = reg.register(make_project("p", WorkspacePolicy::Shared)).unwrap();
        reg.remove(&id).unwrap();
        assert!(reg.get(&id).is_none());
    }

    #[test]
    fn shared_workspace_reused() {
        let mut reg = registry();
        let pid = reg.register(make_project("p", WorkspacePolicy::Shared)).unwrap();
        let s1 = Id::new();
        let s2 = Id::new();
        let w1 = reg.resolve_workspace(&pid, &s1).unwrap();
        let w2 = reg.resolve_workspace(&pid, &s2).unwrap();
        assert_eq!(w1, w2, "Shared policy should return the same workspace");
    }

    #[test]
    fn worktree_policy_creates_unique_workspaces() {
        let mut reg = registry();
        let pid = reg
            .register(make_project("p", WorkspacePolicy::WorktreePerSession))
            .unwrap();
        let s1 = Id::new();
        let s2 = Id::new();
        let w1 = reg.resolve_workspace(&pid, &s1).unwrap();
        let w2 = reg.resolve_workspace(&pid, &s2).unwrap();
        assert_ne!(w1, w2, "WorktreePerSession should create a new workspace per session");
    }

    #[test]
    fn workspace_path_is_under_repo() {
        let mut reg = registry();
        let p = make_project("proj", WorkspacePolicy::Shared);
        let repo_path = p.repo_path.clone();
        let pid = reg.register(p).unwrap();
        let wid = reg.resolve_workspace(&pid, &Id::new()).unwrap();
        let ws = reg.workspaces_for(&pid);
        let ws = ws.iter().find(|w| w.id == wid).unwrap();
        assert!(ws.path.starts_with(&repo_path));
    }

    #[test]
    fn worktrees_for_project() {
        let mut reg = registry();
        let pid = reg
            .register(make_project("p", WorkspacePolicy::WorktreePerSession))
            .unwrap();
        reg.resolve_workspace(&pid, &Id::new()).unwrap();
        reg.resolve_workspace(&pid, &Id::new()).unwrap();
        assert_eq!(reg.workspaces_for(&pid).len(), 2);
    }

    #[test]
    fn reclaim_workspace() {
        let mut reg = registry();
        let pid = reg
            .register(make_project("p", WorkspacePolicy::WorktreePerSession))
            .unwrap();
        let wid = reg.resolve_workspace(&pid, &Id::new()).unwrap();
        reg.reclaim_workspace(&wid).unwrap();
        let ws = reg.workspaces_for(&pid);
        let ws = ws.iter().find(|w| w.id == wid).unwrap();
        assert!(ws.reclaimed_at.is_some());
    }

    #[test]
    fn persistence_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("projects.db");
        let pid;
        {
            let store = ProjectStore::open(&db).unwrap();
            let mut reg = ProjectRegistry::new(store).unwrap();
            pid = reg.register(make_project("persisted", WorkspacePolicy::Shared)).unwrap();
            reg.resolve_workspace(&pid, &Id::new()).unwrap();
        }
        {
            let store = ProjectStore::open(&db).unwrap();
            let reg = ProjectRegistry::new(store).unwrap();
            assert!(reg.get(&pid).is_some());
            assert_eq!(reg.workspaces_for(&pid).len(), 1);
        }
    }
}
