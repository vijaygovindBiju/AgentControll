//! "Start Antigravity Session" form: account, working directory (terminal-style TAB completion),
//! execution mode, permission / access mode, and model.
//!
//! Directory listings and model discovery run on blocking worker threads; the
//! results are delivered through `App::launch_jobs` and applied on the next
//! tick, so the UI never waits on the filesystem or on `agy`.

use std::path::{Path, PathBuf};

use ac_core::{
    agy_launch::{resolve_dir_input, AgyExecutionMode, AgyLaunchOptions, AgyPermissionMode},
    types::Account,
};
use crossterm::event::{KeyCode, KeyEvent};

use crate::{
    app::{App, Modal},
    client::ApiClient,
};

pub const FIELD_ACCOUNT: usize = 0;
pub const FIELD_DIR: usize = 1;
pub const FIELD_EXEC_MODE: usize = 2;
pub const FIELD_PERM_MODE: usize = 3;
pub const FIELD_MODEL: usize = 4;
pub const FIELD_BUTTONS: usize = 5;
pub const FIELDS_COUNT: usize = 6;

/// Open completion popup for the working-directory field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirCompletion {
    /// Input text before the component being completed (e.g. `~/projects/`).
    pub parent: String,
    /// All child directories of `parent`, each with a trailing `/`.
    pub all: Vec<String>,
    pub selected: usize,
}

impl DirCompletion {
    /// Children matching the typed partial name (case-insensitive prefix).
    pub fn matches(&self, partial: &str) -> Vec<&str> {
        let p = partial.to_lowercase();
        self.all
            .iter()
            .map(String::as_str)
            .filter(|d| d.to_lowercase().starts_with(&p))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartSessionForm {
    pub field: usize,
    /// Index into [`agy_accounts`]; `len()` selects "+ Add Antigravity Account".
    pub account_index: usize,
    pub dir_input: String,
    pub completion: Option<DirCompletion>,
    /// A directory listing for this parent is in flight.
    pub listing: Option<String>,
    pub dir_notice: Option<String>,
    pub exec_modes: Vec<AgyExecutionMode>,
    pub exec_mode_index: usize,
    pub perm_modes: Vec<AgyPermissionMode>,
    pub perm_mode_index: usize,
    /// 0 = agy default, otherwise index + 1 into the account's model list.
    pub model_index: usize,
    /// 0 = Start, 1 = Cancel.
    pub button: usize,
    /// Dangerous-mode confirmation is showing; value is the focused button (0 = Cancel, 1 = Continue).
    pub confirm_dangerous: Option<usize>,
    pub error: Option<String>,
    pub cwd: PathBuf,
    pub home: PathBuf,
}

impl StartSessionForm {
    pub fn new(
        cwd: PathBuf,
        home: PathBuf,
        exec_modes: Vec<AgyExecutionMode>,
        perm_modes: Vec<AgyPermissionMode>,
    ) -> Self {
        let mut dir_input = abbreviate_home(&cwd, &home);
        if !dir_input.ends_with('/') {
            dir_input.push('/');
        }
        Self {
            field: FIELD_ACCOUNT,
            account_index: 0,
            dir_input,
            completion: None,
            listing: None,
            dir_notice: None,
            exec_modes: if exec_modes.is_empty() {
                vec![AgyExecutionMode::Default]
            } else {
                exec_modes
            },
            exec_mode_index: 0,
            perm_modes: if perm_modes.is_empty() {
                vec![AgyPermissionMode::Normal]
            } else {
                perm_modes
            },
            perm_mode_index: 0,
            model_index: 0,
            button: 0,
            confirm_dangerous: None,
            error: None,
            cwd,
            home,
        }
    }

    pub fn exec_mode(&self) -> AgyExecutionMode {
        self.exec_modes
            .get(self.exec_mode_index)
            .copied()
            .unwrap_or_default()
    }

    pub fn perm_mode(&self) -> AgyPermissionMode {
        self.perm_modes
            .get(self.perm_mode_index)
            .copied()
            .unwrap_or_default()
    }

    pub fn resolved_dir(&self) -> PathBuf {
        resolve_dir_input(&self.dir_input, &self.cwd, &self.home)
    }
}

/// Background results applied on tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobResult {
    Dirs {
        parent: String,
        from_tab: bool,
        result: Result<Vec<String>, String>,
    },
    Models {
        account_id: String,
        result: Result<Vec<(String, String)>, String>,
    },
}

pub fn is_agy(a: &Account) -> bool {
    matches!(a.provider.as_str(), "agy" | "antigravity")
}

pub fn agy_accounts(app: &App) -> Vec<&Account> {
    app.accounts.iter().filter(|a| is_agy(a)).collect()
}

pub fn abbreviate_home(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// Split directory input into (parent text, partial last component).
pub fn split_input(input: &str) -> (&str, &str) {
    if input == "~" {
        return ("~", "");
    }
    match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    }
}

/// Immediate child directories of `dir` (symlinks to directories included),
/// sorted case-insensitively with hidden directories after the others.
pub fn list_child_dirs(dir: &Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("No such directory: {}", dir.display()),
        std::io::ErrorKind::PermissionDenied => format!("Permission denied: {}", dir.display()),
        _ => format!("Cannot open {}: {e}", dir.display()),
    })?;
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| {
            e.file_type()
                .map(|t| t.is_dir() || (t.is_symlink() && e.path().is_dir()))
                .unwrap_or(false)
        })
        .filter_map(|e| e.file_name().to_str().map(|n| format!("{n}/")))
        .collect();
    out.sort_by_key(|n| (n.starts_with('.'), n.to_lowercase()));
    Ok(out)
}

pub fn longest_common_prefix(strings: &[&str]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let first = strings[0];
    let mut len = 0;
    'outer: for (i, c) in first.char_indices() {
        for s in &strings[1..] {
            if !s[i..].starts_with(c) {
                break 'outer;
            }
        }
        len = i + c.len_utf8();
    }
    first[..len].to_string()
}

fn spawn_job<F>(app: &App, f: F)
where
    F: FnOnce() -> JobResult + Send + 'static,
{
    let jobs = app.launch_jobs.clone();
    tokio::task::spawn_blocking(move || {
        let res = f();
        if let Ok(mut v) = jobs.lock() {
            v.push(res);
        }
    });
}

/// Fetch available models for `account` on a background thread unless already cached.
pub fn request_models(app: &mut App, account: &Account) {
    let aid = account.id.0.clone();
    if app.agy_models.contains_key(&aid) {
        return;
    }
    app.agy_models.insert(aid.clone(), None);
    let id = account.id.clone();
    let cred = account.credential_ref.clone();
    spawn_job(app, move || {
        let res =
            list_models(&id, &cred).map(|ms| ms.into_iter().map(|m| (m.id, m.name)).collect());
        JobResult::Models {
            account_id: aid,
            result: res,
        }
    });
}

#[cfg(unix)]
fn list_models(
    account_id: &ac_core::types::Id,
    cred_ref: &str,
) -> Result<Vec<ac_core::agy_launch::AgyModel>, String> {
    ac_core::agy_launch::list_models(account_id, cred_ref, std::time::Duration::from_secs(6))
}

#[cfg(not(unix))]
fn list_models(
    _: &ac_core::types::Id,
    _: &str,
) -> Result<Vec<ac_core::agy_launch::AgyModel>, String> {
    Err("model listing is only supported on Unix".into())
}

/// Model choices for the selected account: `(model id, label)`, first = agy default.
pub fn model_options(
    app: &App,
    form: &StartSessionForm,
) -> (Vec<(Option<String>, String)>, Option<String>) {
    let mut opts = vec![(None, "Default (agy setting)".to_string())];
    let status = match agy_accounts(app)
        .get(form.account_index)
        .and_then(|a| app.agy_models.get(&a.id.0))
    {
        Some(None) => Some("loading models…".to_string()),
        Some(Some(Err(e))) => Some(format!("model list unavailable: {e}")),
        Some(Some(Ok(models))) => {
            opts.extend(
                models
                    .iter()
                    .map(|(id, name)| (Some(id.clone()), name.clone())),
            );
            None
        }
        None => None,
    };
    (opts, status)
}

/// Open the start-session form. Accounts of other providers keep the
/// existing generic launcher.
pub fn open_new_session(app: &mut App) {
    if app
        .accounts
        .get(app.selected_account)
        .is_some_and(|a| !is_agy(a))
    {
        app.active_modal = Some(Modal::NewSession {
            account_index: app.selected_account,
            session_name: String::new(),
            active_field: 0,
        });
        return;
    }
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"));
    let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
    let mut form = StartSessionForm::new(
        cwd,
        home,
        ac_core::agy_launch::supported_execution_modes(),
        ac_core::agy_launch::supported_permission_modes(),
    );

    // Always use the currently selected account from the UI
    // The persisted default_account is only used as a fallback if no account is selected
    let selected_id = app.accounts.get(app.selected_account).map(|a| a.id.clone());
    form.account_index = if let Some(sid) = selected_id {
        agy_accounts(app)
            .iter()
            .position(|a| &a.id == &sid)
            .unwrap_or(0)
    } else if let Some(ref default_acc) = app.user_settings.default_account {
        // Fallback to persisted default if no account is selected
        agy_accounts(app)
            .iter()
            .position(|a| &a.label == default_acc || &a.id.0 == default_acc)
            .unwrap_or(0)
    } else {
        0
    };

    // Apply persisted default working directory if valid
    if let Some(ref default_dir) = app.user_settings.default_working_dir {
        if !default_dir.is_empty() {
            let resolved = resolve_dir_input(default_dir, &form.cwd, &form.home);
            if resolved.is_dir() {
                form.dir_input = abbreviate_home(&resolved, &form.home);
                if !form.dir_input.ends_with('/') {
                    form.dir_input.push('/');
                }
            }
        }
    }

    // Apply persisted default execution mode
    form.exec_mode_index = form
        .exec_modes
        .iter()
        .position(|m| *m == app.user_settings.default_execution_mode)
        .unwrap_or(0);

    // Apply persisted default permission mode
    form.perm_mode_index = form
        .perm_modes
        .iter()
        .position(|m| *m == app.user_settings.default_permission_mode)
        .unwrap_or(0);

    open_form(app, Box::new(form));
}

/// Show the form (again), refreshing the model list for its account.
pub fn open_form(app: &mut App, form: Box<StartSessionForm>) {
    if let Some(a) = agy_accounts(app)
        .get(form.account_index)
        .map(|a| (*a).clone())
    {
        request_models(app, &a);
    }
    app.active_modal = Some(Modal::StartSession(form));
}

fn cycle(i: usize, n: usize, forward: bool) -> usize {
    if n == 0 {
        0
    } else if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    }
}

/// Request a listing of the directory the input's last component lives in.
fn request_listing(app: &App, form: &mut StartSessionForm, from_tab: bool) {
    let parent = split_input(&form.dir_input).0.to_string();
    if form.listing.as_deref() == Some(parent.as_str()) {
        return;
    }
    form.listing = Some(parent.clone());
    let dir = resolve_dir_input(
        if parent.is_empty() { "." } else { &parent },
        &form.cwd,
        &form.home,
    );
    spawn_job(app, move || JobResult::Dirs {
        result: list_child_dirs(&dir),
        parent,
        from_tab,
    });
}

/// Re-filter (or re-list) after the directory input was edited.
fn after_dir_edit(app: &App, form: &mut StartSessionForm) {
    form.dir_notice = None;
    form.error = None;
    let parent = split_input(&form.dir_input).0.to_string();
    match &mut form.completion {
        Some(c) if c.parent == parent => c.selected = 0,
        Some(_) => request_listing(app, form, false),
        None => {}
    }
}

fn apply_completion(form: &mut StartSessionForm, name: &str) {
    let parent = split_input(&form.dir_input).0.to_string();
    let parent = if parent == "~" {
        "~/".to_string()
    } else {
        parent
    };
    form.dir_input = format!("{parent}{name}");
    form.completion = None;
}

/// Apply finished background work. Called on every tick.
pub fn poll_jobs(app: &mut App) {
    let jobs: Vec<JobResult> = app
        .launch_jobs
        .lock()
        .map(|mut v| v.drain(..).collect())
        .unwrap_or_default();
    for job in jobs {
        match job {
            JobResult::Models { account_id, result } => {
                app.agy_models.insert(account_id, Some(result));
            }
            JobResult::Dirs {
                parent,
                from_tab,
                result,
            } => {
                let Some(Modal::StartSession(form)) = app.active_modal.as_mut() else {
                    continue;
                };
                if form.listing.as_deref() == Some(parent.as_str()) {
                    form.listing = None;
                }
                let (cur_parent, partial) = split_input(&form.dir_input);
                if cur_parent != parent {
                    continue; // stale: the user has moved on
                }
                let partial = partial.to_string();
                match result {
                    Err(e) => {
                        form.dir_notice = Some(e);
                        form.completion = None;
                    }
                    Ok(all) => {
                        let c = DirCompletion {
                            parent,
                            all,
                            selected: 0,
                        };
                        let matches: Vec<String> = c
                            .matches(&partial)
                            .into_iter()
                            .map(str::to_string)
                            .collect();
                        if from_tab && matches.len() == 1 {
                            apply_completion(form, &matches[0]);
                        } else if from_tab && matches.is_empty() {
                            form.dir_notice = Some(if partial.is_empty() {
                                "No subdirectories".into()
                            } else {
                                "No matching directories".into()
                            });
                            form.completion = None;
                        } else {
                            let refs: Vec<&str> = matches.iter().map(String::as_str).collect();
                            let lcp = longest_common_prefix(&refs);
                            if from_tab && lcp.len() > partial.len() {
                                let p = form.dir_input.len() - partial.len();
                                form.dir_input.replace_range(p.., &lcp);
                            }
                            form.completion = Some(c);
                        }
                    }
                }
            }
        }
    }
}

/// Validate the working directory, returning its absolute path.
fn validate_dir(form: &StartSessionForm) -> Result<PathBuf, String> {
    let p = form.resolved_dir();
    if !p.is_dir() {
        return Err(format!("Working directory does not exist: {}", p.display()));
    }
    std::fs::read_dir(&p)
        .map_err(|_| format!("Working directory is not accessible: {}", p.display()))?;
    Ok(p)
}

/// Handle a key for the start-session form (which has been taken out of `app`).
pub async fn handle_key(
    app: &mut App,
    client: &ApiClient,
    mut form: Box<StartSessionForm>,
    key: KeyEvent,
) {
    // Dangerous-mode confirmation: Cancel is focused by default (0 = Cancel, 1 = Continue).
    if let Some(btn) = form.confirm_dangerous {
        match key.code {
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => {
                form.confirm_dangerous = Some(1 - btn)
            }
            KeyCode::Enter if btn == 1 => return launch(app, client, form).await,
            KeyCode::Enter | KeyCode::Esc => form.confirm_dangerous = None,
            _ => {}
        }
        app.active_modal = Some(Modal::StartSession(form));
        return;
    }

    // Completion popup navigation.
    if form.field == FIELD_DIR {
        if let Some(c) = form.completion.as_mut() {
            let n = c.matches(split_input(&form.dir_input).1).len();
            match key.code {
                KeyCode::Down | KeyCode::Tab => {
                    c.selected = cycle(c.selected, n, true);
                    app.active_modal = Some(Modal::StartSession(form));
                    return;
                }
                KeyCode::Up | KeyCode::BackTab => {
                    c.selected = cycle(c.selected, n, false);
                    app.active_modal = Some(Modal::StartSession(form));
                    return;
                }
                KeyCode::Enter => {
                    let pick = c
                        .matches(split_input(&form.dir_input).1)
                        .get(c.selected)
                        .map(|s| s.to_string());
                    match pick {
                        Some(name) => apply_completion(&mut form, &name),
                        None => form.completion = None,
                    }
                    app.active_modal = Some(Modal::StartSession(form));
                    return;
                }
                KeyCode::Esc => {
                    form.completion = None;
                    app.active_modal = Some(Modal::StartSession(form));
                    return;
                }
                _ => {}
            }
        }
    }

    let n_accounts = agy_accounts(app).len();
    let (models, _) = model_options(app, &form);
    if key
        .modifiers
        .contains(crossterm::event::KeyModifiers::CONTROL)
    {
        if matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')) {
            app.active_modal = None;
            return;
        }
        if matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V')) {
            if let Some(text) = crate::clipboard::paste() {
                handle_paste(app, form, &text);
                return;
            }
        }
    }
    match key.code {
        KeyCode::Esc => {
            app.active_modal = None;
            return;
        }
        KeyCode::Up | KeyCode::BackTab => form.field = cycle(form.field, FIELDS_COUNT, false),
        KeyCode::Down => form.field = cycle(form.field, FIELDS_COUNT, true),
        KeyCode::Tab if form.field == FIELD_DIR => {
            form.dir_notice = None;
            request_listing(app, &mut form, true);
        }
        KeyCode::Tab => form.field = cycle(form.field, FIELDS_COUNT, true),
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if form.field != FIELD_DIR => {
            let fwd = key.code != KeyCode::Left;
            match form.field {
                FIELD_ACCOUNT => {
                    form.account_index = cycle(form.account_index, n_accounts + 1, fwd);
                    form.model_index = 0;
                    form.error = None;
                    if let Some(a) = agy_accounts(app)
                        .get(form.account_index)
                        .map(|a| (*a).clone())
                    {
                        request_models(app, &a);
                    }
                }
                FIELD_EXEC_MODE => {
                    form.exec_mode_index = cycle(form.exec_mode_index, form.exec_modes.len(), fwd)
                }
                FIELD_PERM_MODE => {
                    form.perm_mode_index = cycle(form.perm_mode_index, form.perm_modes.len(), fwd)
                }
                FIELD_MODEL => form.model_index = cycle(form.model_index, models.len(), fwd),
                FIELD_BUTTONS => form.button = 1 - form.button,
                _ => {}
            }
        }
        KeyCode::Backspace if form.field == FIELD_DIR => {
            form.dir_input.pop();
            after_dir_edit(app, &mut form);
        }
        KeyCode::Delete if form.field == FIELD_DIR => {
            form.dir_input.pop();
            after_dir_edit(app, &mut form);
        }
        KeyCode::Char(c) if form.field == FIELD_DIR => {
            form.dir_input.push(c);
            after_dir_edit(app, &mut form);
        }
        KeyCode::Enter => match form.field {
            FIELD_ACCOUNT if form.account_index >= n_accounts => {
                app.resume_start_session = Some(form);
                crate::event::open_agy_add_account(app);
                return;
            }
            FIELD_DIR => match validate_dir(&form) {
                Ok(p) => {
                    form.dir_input = abbreviate_home(&p, &form.home);
                    form.field = FIELD_EXEC_MODE;
                }
                Err(e) => form.dir_notice = Some(e),
            },
            FIELD_BUTTONS if form.button == 1 => {
                app.active_modal = None;
                return;
            }
            _ => return try_start(app, client, form).await,
        },
        _ => {}
    }
    app.active_modal = Some(Modal::StartSession(form));
}

async fn try_start(app: &mut App, client: &ApiClient, mut form: Box<StartSessionForm>) {
    form.error = if form.account_index >= agy_accounts(app).len() {
        Some("Select an Antigravity account (or add one first).".into())
    } else {
        validate_dir(&form).err()
    };
    if form.error.is_none() && form.perm_mode().is_dangerous() {
        form.confirm_dangerous = Some(0); // Default to Cancel (0)
    }
    if form.error.is_some() || form.confirm_dangerous.is_some() {
        app.active_modal = Some(Modal::StartSession(form));
        return;
    }
    launch(app, client, form).await
}

/// Final launch options for the form (directory already validated).
pub fn launch_options(app: &App, form: &StartSessionForm) -> Result<AgyLaunchOptions, String> {
    let dir = validate_dir(form)?;
    let (models, _) = model_options(app, form);
    let (initial_cols, initial_rows) = match crossterm::terminal::size() {
        Ok((cols, rows)) => (Some(cols.max(1)), Some(rows.saturating_sub(2).max(1))),
        Err(_) => (None, None),
    };
    Ok(AgyLaunchOptions {
        execution_mode: form.exec_mode(),
        permission_mode: form.perm_mode(),
        sandbox: false,
        model: models.get(form.model_index).and_then(|(id, _)| id.clone()),
        working_dir: Some(dir.display().to_string()),
        initial_rows,
        initial_cols,
    })
}

async fn launch(app: &mut App, client: &ApiClient, mut form: Box<StartSessionForm>) {
    let account = agy_accounts(app)
        .get(form.account_index)
        .map(|a| a.id.clone());
    let (Some(account), Ok(opts)) = (account, launch_options(app, &form)) else {
        form.confirm_dangerous = None;
        form.error = Some("The selected account or directory is no longer valid.".into());
        app.active_modal = Some(Modal::StartSession(form));
        return;
    };
    app.active_modal = None;
    crate::event::launch_session_and_open(
        app,
        client,
        "[interactive] session",
        "agy",
        Some(&account),
        Some(&opts),
    )
    .await;
}

/// Handle paste into the start-session form.
pub fn handle_paste(app: &mut App, mut form: Box<StartSessionForm>, text: &str) {
    if form.field == FIELD_DIR {
        let cleaned: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
        form.dir_input.push_str(&cleaned);
        after_dir_edit(app, &mut form);
    }
    app.active_modal = Some(Modal::StartSession(form));
}
