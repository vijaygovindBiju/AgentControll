//! App state and business logic for the TUI.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use ac_core::types::{
    Account, AgentEvent, AgentSession, EventKind, Id, Interaction, InteractionState, Project,
    SessionState,
};

use crate::prompt_history::PromptHistory;
use crate::terminal_buffer::TerminalBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard = 0,
    Sessions = 1,
    Accounts = 2,
    Activity = 3,
    Agents = 4,
    Settings = 5,
}

impl Tab {
    pub const ALL: [Self; 6] = [
        Tab::Dashboard,
        Tab::Sessions,
        Tab::Accounts,
        Tab::Activity,
        Tab::Agents,
        Tab::Settings,
    ];

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Tab::Dashboard,
            1 => Tab::Sessions,
            2 => Tab::Accounts,
            3 => Tab::Activity,
            4 => Tab::Agents,
            5 => Tab::Settings,
            _ => Tab::Dashboard,
        }
    }

    pub fn to_index(self) -> usize {
        self as usize
    }

    pub fn title(self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Sessions => "Sessions",
            Tab::Accounts => "Accounts",
            Tab::Activity => "Activity",
            Tab::Agents => "Agents",
            Tab::Settings => "Settings",
        }
    }
}

/// Sections inside the Settings configuration view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    General = 0,
    Accounts = 1,
    Projects = 2,
    Agents = 3,
    Models = 4,
    Permissions = 5,
    Authentication = 6,
    Terminal = 7,
    Security = 8,
}

impl SettingsSection {
    pub const ALL: [Self; 9] = [
        Self::General,
        Self::Accounts,
        Self::Projects,
        Self::Agents,
        Self::Models,
        Self::Permissions,
        Self::Authentication,
        Self::Terminal,
        Self::Security,
    ];

    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Self::General,
            1 => Self::Accounts,
            2 => Self::Projects,
            3 => Self::Agents,
            4 => Self::Models,
            5 => Self::Permissions,
            6 => Self::Authentication,
            7 => Self::Terminal,
            8 => Self::Security,
            _ => Self::General,
        }
    }

    pub fn to_index(self) -> usize {
        self as usize
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Accounts => "Accounts",
            Self::Projects => "Projects & Workspaces",
            Self::Agents => "Agents",
            Self::Models => "Models",
            Self::Permissions => "Permissions",
            Self::Authentication => "Authentication",
            Self::Terminal => "Terminal",
            Self::Security => "Security",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusType {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchModalStep {
    SelectAccount,
    ConfirmRestart {
        target_account_id: Id,
        target_account_label: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountOption {
    pub account_id: Id,
    pub label: String,
    pub provider: String,
    pub usable: bool,
    pub active_sessions: u32,
    pub concurrency_cap: u8,
    pub reason: Option<String>,
}

/// Steps of the "Add Antigravity Account" dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgyAddStep {
    /// Enter a friendly account name.
    Name {
        error: Option<String>,
    },
    /// Choose Login with Browser (default) or Import.
    Method,
    /// Browser login in progress. `url` is empty until the listener is ready.
    Waiting {
        url: String,
        browser_opened: bool,
        deadline: Instant,
    },
    /// Shareable login link. `paste` holds the redirect address the user pastes
    /// back after signing in on another device; `notice` reports rejections.
    Link {
        url: String,
        deadline: Instant,
        paste: String,
        notice: Option<String>,
    },
    Success {
        account_id: Id,
        email: Option<String>,
    },
    Failed {
        reason: String,
    },
}

/// Progress reported by the background Antigravity login task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgyLoginUpdate {
    Waiting {
        url: String,
        browser_opened: bool,
    },
    /// A login link was generated (nothing was opened locally).
    LinkReady {
        url: String,
    },
    /// A pasted redirect address was rejected; the login keeps waiting.
    PasteRejected {
        reason: String,
    },
    Succeeded {
        account_id: Id,
        email: Option<String>,
    },
    Failed {
        reason: String,
    },
}

/// Handle to the single in-flight Antigravity login. Aborting it drops the
/// callback listener and all temporary OAuth state (state, PKCE verifier).
#[derive(Debug, Clone)]
pub struct AgyLoginTask {
    pub abort: tokio::task::AbortHandle,
    pub updates: std::sync::Arc<std::sync::Mutex<Vec<AgyLoginUpdate>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Steer {
        session_id: Id,
        input: String,
    },
    Reply {
        interaction_id: Id,
        input: String,
    },
    ConfirmStop {
        session_id: Id,
    },
    ConfirmRemoveSession {
        session_id: Id,
    },
    /// Add an Antigravity account (name → browser login / import → result).
    AgyAddAccount {
        label: String,
        step: AgyAddStep,
    },
    /// Confirm removal of an account. `active_sessions > 0` blocks removal.
    ConfirmRemoveAccount {
        account_id: Id,
        label: String,
        provider: String,
        active_sessions: usize,
    },
    FilterActivity {
        input: String,
    },
    Help,
    SwitchAccount {
        session_id: Id,
        agent_type: String,
        current_account_id: Option<Id>,
        current_account_label: String,
        options: Vec<AccountOption>,
        selected_index: usize,
        step: SwitchModalStep,
    },
    AddAccount {
        label: String,
        provider: String,
        auth_method: usize,
        token: String,
        active_field: usize,
    },
    NewSession {
        account_index: usize,
        session_name: String,
        active_field: usize,
    },
    /// Start an Antigravity session: account, permission mode, directory, model.
    StartSession(Box<crate::launch::StartSessionForm>),
    CommandPalette {
        session_id: Id,
        selected_index: usize,
    },
    Approval {
        interaction_id: Id,
        tool_name: Option<String>,
        prompt: String,
    },
    SessionInfo {
        session_id: Id,
    },
    RegisterProject {
        name: String,
        repo_path: String,
        policy_index: usize,
        active_field: usize,
        error: Option<String>,
    },
    ConfirmRemoveProject {
        project_id: Id,
        name: String,
    },
    SetDefaultWorkingDir {
        input: String,
        error: Option<String>,
    },
    UrlPicker {
        session_id: Id,
        urls: Vec<String>,
        selected_index: usize,
    },
}

#[derive(Debug, Clone)]
pub struct App {
    pub current_tab: Tab,
    pub session_detail_id: Option<Id>,

    // Data collections
    pub sessions: Vec<AgentSession>,
    pub selected_session: usize,

    pub interactions: Vec<Interaction>,
    pub selected_interaction: usize,

    pub accounts: Vec<Account>,
    pub selected_account: usize,

    pub projects: Vec<Project>,
    pub selected_project: usize,

    pub events: Vec<AgentEvent>,
    pub selected_event: usize,

    // Live session transcripts (session_id -> lines of transcript)
    pub session_transcripts: HashMap<String, Vec<String>>,
    // Live bounded virtual terminal buffers for session screens
    pub session_terminal_buffers: HashMap<String, TerminalBuffer>,
    // Command and prompt history per session
    pub session_prompt_histories: HashMap<String, PromptHistory>,
    // Transient active prompt drafts per session
    pub session_prompt_buffers: HashMap<String, String>,
    // Tracks if completion popup was explicitly dismissed for the current prompt (e.g. by Esc/Tab/Enter)
    pub session_completion_dismissed: HashMap<String, bool>,
    // Global / shared prompt history for general input
    pub global_prompt_history: PromptHistory,

    // Activity filter
    pub activity_filter: Option<String>,

    // Active modal dialog
    pub active_modal: Option<Modal>,

    // Status / Notification bar
    pub status_message: Option<(String, StatusType, Instant)>,
    pub daemon_connected: bool,

    // Should quit
    pub should_quit: bool,

    // Professional btop-style UI state
    pub sidebar_selected: usize,
    pub detail_subtab: usize,
    pub quick_launch_selected: usize,
    pub selected_agent: usize,
    pub start_time: Instant,

    /// Results from background tasks (e.g. browser login), drained on tick.
    pub background_notices: std::sync::Arc<std::sync::Mutex<Vec<(String, StatusType)>>>,

    /// The in-flight Antigravity browser login, if any (at most one).
    pub agy_login: Option<AgyLoginTask>,
    /// Paste channel of an in-flight login-link login (redirect address handoff).
    pub agy_login_paste: Option<tokio::sync::mpsc::Sender<String>>,
    /// Selected option in the "Add Antigravity Account" method list.
    pub agy_add_method: usize,
    /// Start-session form to return to after adding an account from it.
    pub resume_start_session: Option<Box<crate::launch::StartSessionForm>>,
    /// Results of background launch-form work (directory listings, models).
    pub launch_jobs: std::sync::Arc<std::sync::Mutex<Vec<crate::launch::JobResult>>>,
    /// Models per account id, as reported by `agy models` (None = loading).
    pub agy_models: HashMap<String, Option<Result<Vec<(String, String)>, String>>>,

    // Settings state & persistence
    pub user_settings: ac_core::settings::UserSettings,
    pub settings_section_index: usize,
    pub settings_focus_panel: bool,
    pub settings_general_item: usize,
    pub settings_account_selected: usize,
    pub settings_project_selected: usize,
    pub settings_model_selected: usize,

    // Mouse selection tracking
    pub mouse_selection: MouseSelectionState,
}

/// Tracks mouse cursor drag and multi-click selection state.
#[derive(Debug, Clone, Default)]
pub struct MouseSelectionState {
    pub is_dragging: bool,
    pub last_click_time: Option<Instant>,
    pub last_click_pos: (u16, u16),
    pub click_count: usize,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            current_tab: Tab::Dashboard,
            session_detail_id: None,
            sessions: Vec::new(),
            selected_session: 0,
            interactions: Vec::new(),
            selected_interaction: 0,
            accounts: Vec::new(),
            selected_account: 0,
            projects: Vec::new(),
            selected_project: 0,
            events: Vec::new(),
            selected_event: 0,
            session_transcripts: HashMap::new(),
            session_terminal_buffers: HashMap::new(),
            session_prompt_histories: HashMap::new(),
            session_prompt_buffers: HashMap::new(),
            session_completion_dismissed: HashMap::new(),
            global_prompt_history: PromptHistory::new(),
            activity_filter: None,
            active_modal: None,
            status_message: None,
            daemon_connected: false,
            should_quit: false,
            sidebar_selected: 0,
            detail_subtab: 0,
            quick_launch_selected: 0,
            selected_agent: 0,
            start_time: Instant::now(),
            background_notices: Default::default(),
            agy_login: None,
            agy_login_paste: None,
            agy_add_method: 0,
            resume_start_session: None,
            launch_jobs: Default::default(),
            agy_models: HashMap::new(),
            user_settings: ac_core::settings::UserSettings::load(),
            settings_section_index: 0,
            settings_focus_panel: false,
            settings_general_item: 0,
            settings_account_selected: 0,
            settings_project_selected: 0,
            settings_model_selected: 0,
            mouse_selection: MouseSelectionState::default(),
        }
    }

    pub fn current_settings_section(&self) -> SettingsSection {
        SettingsSection::from_index(self.settings_section_index)
    }

    pub fn next_detail_subtab(&mut self) {
        self.detail_subtab = (self.detail_subtab + 1) % 4;
    }

    pub fn prev_detail_subtab(&mut self) {
        self.detail_subtab = if self.detail_subtab == 0 {
            3
        } else {
            self.detail_subtab - 1
        };
    }

    pub fn next_sidebar_item(&mut self) {
        self.sidebar_selected = (self.sidebar_selected + 1) % 10;
    }

    pub fn prev_sidebar_item(&mut self) {
        self.sidebar_selected = if self.sidebar_selected == 0 {
            9
        } else {
            self.sidebar_selected - 1
        };
    }

    pub fn set_status(&mut self, message: impl Into<String>, status_type: StatusType) {
        self.status_message = Some((message.into(), status_type, Instant::now()));
    }

    /// Drain background notices into the status bar. Returns true if any arrived.
    pub fn drain_background_notices(&mut self) -> bool {
        let notices: Vec<_> = self
            .background_notices
            .lock()
            .map(|mut v| v.drain(..).collect())
            .unwrap_or_default();
        let any = !notices.is_empty();
        for (msg, kind) in notices {
            self.set_status(msg, kind);
        }
        any
    }

    /// Drain terminal notifications (OSC 9 / OSC 777) from all session buffers into status messages. Returns true if any arrived.
    pub fn drain_terminal_notifications(&mut self) -> bool {
        let mut notifications = Vec::new();
        for (session_id, buf) in &mut self.session_terminal_buffers {
            let notifs = buf.take_notifications();
            for notif in notifs {
                notifications.push((session_id.clone(), notif));
            }
        }
        let any = !notifications.is_empty();
        for (session_id, notif) in notifications {
            let prefix = if session_id.len() >= 8 {
                &session_id[..8]
            } else {
                &session_id
            };
            let text = match notif.title {
                Some(title) => format!("[{prefix}] {title}: {}", notif.body),
                None => format!("[{prefix}] Notification: {}", notif.body),
            };
            self.set_status(text, StatusType::Info);
        }
        any
    }

    pub fn clear_expired_status(&mut self) {
        if let Some((_, _, time)) = self.status_message {
            if time.elapsed() > Duration::from_secs(6) {
                self.status_message = None;
            }
        }
    }

    // ── Navigation ──────────────────────────────────────────────────────────

    pub fn set_tab(&mut self, tab: Tab) {
        self.current_tab = tab;
        self.session_detail_id = None;
        self.sidebar_selected = match tab {
            Tab::Dashboard => 0,
            Tab::Agents => 1,
            Tab::Accounts => 2,
            Tab::Sessions => 3,
            Tab::Activity => 4,
            Tab::Settings => 7,
        };
    }

    pub fn next_tab(&mut self) {
        let next_idx = (self.current_tab.to_index() + 1) % Tab::ALL.len();
        self.set_tab(Tab::from_index(next_idx));
        self.session_detail_id = None;
    }

    pub fn prev_tab(&mut self) {
        let count = Tab::ALL.len();
        let prev_idx = (self.current_tab.to_index() + count - 1) % count;
        self.set_tab(Tab::from_index(prev_idx));
        self.session_detail_id = None;
    }

    pub fn next_row(&mut self) {
        match self.current_tab {
            Tab::Dashboard | Tab::Sessions => {
                if !self.sessions.is_empty() {
                    self.selected_session = (self.selected_session + 1) % self.sessions.len();
                }
            }
            Tab::Agents => {
                self.selected_agent = (self.selected_agent + 1) % 3;
            }
            Tab::Accounts => {
                if !self.accounts.is_empty() {
                    self.selected_account = (self.selected_account + 1) % self.accounts.len();
                }
            }
            Tab::Activity => {
                let count = self.filtered_events().len();
                if count > 0 {
                    self.selected_event = (self.selected_event + 1) % count;
                }
            }
            Tab::Settings => {
                if self.settings_focus_panel {
                    match self.current_settings_section() {
                        SettingsSection::General => {
                            self.settings_general_item = (self.settings_general_item + 1) % 6;
                        }
                        SettingsSection::Accounts => {
                            if !self.accounts.is_empty() {
                                self.settings_account_selected =
                                    (self.settings_account_selected + 1) % self.accounts.len();
                                self.selected_account = self.settings_account_selected;
                            }
                        }
                        SettingsSection::Projects => {
                            if !self.projects.is_empty() {
                                self.settings_project_selected =
                                    (self.settings_project_selected + 1) % self.projects.len();
                                self.selected_project = self.settings_project_selected;
                            }
                        }
                        SettingsSection::Agents => {
                            self.selected_agent = (self.selected_agent + 1) % 3;
                        }
                        SettingsSection::Models => {
                            self.settings_model_selected =
                                self.settings_model_selected.saturating_add(1);
                        }
                        _ => {}
                    }
                } else {
                    self.settings_section_index =
                        (self.settings_section_index + 1) % SettingsSection::ALL.len();
                }
            }
        }
    }

    pub fn prev_row(&mut self) {
        match self.current_tab {
            Tab::Dashboard | Tab::Sessions => {
                if !self.sessions.is_empty() {
                    self.selected_session = if self.selected_session == 0 {
                        self.sessions.len() - 1
                    } else {
                        self.selected_session - 1
                    };
                }
            }
            Tab::Agents => {
                self.selected_agent = if self.selected_agent == 0 {
                    2
                } else {
                    self.selected_agent - 1
                };
            }
            Tab::Accounts => {
                if !self.accounts.is_empty() {
                    self.selected_account = if self.selected_account == 0 {
                        self.accounts.len() - 1
                    } else {
                        self.selected_account - 1
                    };
                }
            }
            Tab::Activity => {
                let count = self.filtered_events().len();
                if count > 0 {
                    self.selected_event = if self.selected_event == 0 {
                        count - 1
                    } else {
                        self.selected_event - 1
                    };
                }
            }
            Tab::Settings => {
                if self.settings_focus_panel {
                    match self.current_settings_section() {
                        SettingsSection::General => {
                            self.settings_general_item = if self.settings_general_item == 0 {
                                5
                            } else {
                                self.settings_general_item - 1
                            };
                        }
                        SettingsSection::Accounts => {
                            if !self.accounts.is_empty() {
                                self.settings_account_selected =
                                    if self.settings_account_selected == 0 {
                                        self.accounts.len() - 1
                                    } else {
                                        self.settings_account_selected - 1
                                    };
                                self.selected_account = self.settings_account_selected;
                            }
                        }
                        SettingsSection::Projects => {
                            if !self.projects.is_empty() {
                                self.settings_project_selected =
                                    if self.settings_project_selected == 0 {
                                        self.projects.len() - 1
                                    } else {
                                        self.settings_project_selected - 1
                                    };
                                self.selected_project = self.settings_project_selected;
                            }
                        }
                        SettingsSection::Agents => {
                            self.selected_agent = if self.selected_agent == 0 {
                                2
                            } else {
                                self.selected_agent - 1
                            };
                        }
                        SettingsSection::Models => {
                            self.settings_model_selected =
                                self.settings_model_selected.saturating_sub(1);
                        }
                        _ => {}
                    }
                } else {
                    let total = SettingsSection::ALL.len();
                    self.settings_section_index = if self.settings_section_index == 0 {
                        total - 1
                    } else {
                        self.settings_section_index - 1
                    };
                }
            }
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        let delta = page_size.max(1);
        match self.current_tab {
            Tab::Dashboard | Tab::Sessions => {
                if !self.sessions.is_empty() {
                    self.selected_session =
                        (self.selected_session + delta).min(self.sessions.len() - 1);
                }
            }
            Tab::Accounts => {
                if !self.accounts.is_empty() {
                    self.selected_account =
                        (self.selected_account + delta).min(self.accounts.len() - 1);
                }
            }
            Tab::Activity => {
                let count = self.filtered_events().len();
                if count > 0 {
                    self.selected_event = (self.selected_event + delta).min(count - 1);
                }
            }
            Tab::Settings => {
                if self.settings_focus_panel {
                    match self.current_settings_section() {
                        SettingsSection::Accounts => {
                            if !self.accounts.is_empty() {
                                self.settings_account_selected = (self.settings_account_selected
                                    + delta)
                                    .min(self.accounts.len() - 1);
                                self.selected_account = self.settings_account_selected;
                            }
                        }
                        SettingsSection::Projects => {
                            if !self.projects.is_empty() {
                                self.settings_project_selected = (self.settings_project_selected
                                    + delta)
                                    .min(self.projects.len() - 1);
                                self.selected_project = self.settings_project_selected;
                            }
                        }
                        _ => self.next_row(),
                    }
                } else {
                    self.next_row();
                }
            }
            _ => self.next_row(),
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        let delta = page_size.max(1);
        match self.current_tab {
            Tab::Dashboard | Tab::Sessions => {
                if !self.sessions.is_empty() {
                    self.selected_session = self.selected_session.saturating_sub(delta);
                }
            }
            Tab::Accounts => {
                if !self.accounts.is_empty() {
                    self.selected_account = self.selected_account.saturating_sub(delta);
                }
            }
            Tab::Activity => {
                let count = self.filtered_events().len();
                if count > 0 {
                    self.selected_event = self.selected_event.saturating_sub(delta);
                }
            }
            Tab::Settings => {
                if self.settings_focus_panel {
                    match self.current_settings_section() {
                        SettingsSection::Accounts => {
                            if !self.accounts.is_empty() {
                                self.settings_account_selected =
                                    self.settings_account_selected.saturating_sub(delta);
                                self.selected_account = self.settings_account_selected;
                            }
                        }
                        SettingsSection::Projects => {
                            if !self.projects.is_empty() {
                                self.settings_project_selected =
                                    self.settings_project_selected.saturating_sub(delta);
                                self.selected_project = self.settings_project_selected;
                            }
                        }
                        _ => self.prev_row(),
                    }
                } else {
                    self.prev_row();
                }
            }
            _ => self.prev_row(),
        }
    }

    pub fn first_row(&mut self) {
        match self.current_tab {
            Tab::Dashboard | Tab::Sessions => self.selected_session = 0,
            Tab::Agents => self.selected_agent = 0,
            Tab::Accounts => self.selected_account = 0,
            Tab::Activity => self.selected_event = 0,
            Tab::Settings => {
                if self.settings_focus_panel {
                    match self.current_settings_section() {
                        SettingsSection::General => self.settings_general_item = 0,
                        SettingsSection::Accounts => {
                            self.settings_account_selected = 0;
                            self.selected_account = 0;
                        }
                        SettingsSection::Projects => {
                            self.settings_project_selected = 0;
                            self.selected_project = 0;
                        }
                        SettingsSection::Agents => self.selected_agent = 0,
                        SettingsSection::Models => self.settings_model_selected = 0,
                        _ => {}
                    }
                } else {
                    self.settings_section_index = 0;
                }
            }
        }
    }

    pub fn last_row(&mut self) {
        match self.current_tab {
            Tab::Dashboard | Tab::Sessions => {
                if !self.sessions.is_empty() {
                    self.selected_session = self.sessions.len() - 1;
                }
            }
            Tab::Agents => self.selected_agent = 2,
            Tab::Accounts => {
                if !self.accounts.is_empty() {
                    self.selected_account = self.accounts.len() - 1;
                }
            }
            Tab::Activity => {
                let count = self.filtered_events().len();
                if count > 0 {
                    self.selected_event = count - 1;
                }
            }
            Tab::Settings => {
                if self.settings_focus_panel {
                    match self.current_settings_section() {
                        SettingsSection::General => self.settings_general_item = 5,
                        SettingsSection::Accounts => {
                            if !self.accounts.is_empty() {
                                self.settings_account_selected = self.accounts.len() - 1;
                                self.selected_account = self.settings_account_selected;
                            }
                        }
                        SettingsSection::Projects => {
                            if !self.projects.is_empty() {
                                self.settings_project_selected = self.projects.len() - 1;
                                self.selected_project = self.settings_project_selected;
                            }
                        }
                        SettingsSection::Agents => self.selected_agent = 2,
                        _ => {}
                    }
                } else {
                    self.settings_section_index = SettingsSection::ALL.len().saturating_sub(1);
                }
            }
        }
    }

    pub fn clamp_selections(&mut self) {
        if self.sessions.is_empty() {
            self.selected_session = 0;
        } else if self.selected_session >= self.sessions.len() {
            self.selected_session = self.sessions.len() - 1;
        }

        if self.accounts.is_empty() {
            self.selected_account = 0;
            self.settings_account_selected = 0;
        } else {
            if self.selected_account >= self.accounts.len() {
                self.selected_account = self.accounts.len() - 1;
            }
            if self.settings_account_selected >= self.accounts.len() {
                self.settings_account_selected = self.accounts.len() - 1;
            }
        }

        if self.projects.is_empty() {
            self.selected_project = 0;
            self.settings_project_selected = 0;
        } else {
            if self.selected_project >= self.projects.len() {
                self.selected_project = self.projects.len() - 1;
            }
            if self.settings_project_selected >= self.projects.len() {
                self.settings_project_selected = self.projects.len() - 1;
            }
        }

        let filtered_events_len = self.filtered_events().len();
        if filtered_events_len == 0 {
            self.selected_event = 0;
        } else if self.selected_event >= filtered_events_len {
            self.selected_event = filtered_events_len - 1;
        }

        let pending_interactions_len = self.pending_interactions().len();
        if pending_interactions_len == 0 {
            self.selected_interaction = 0;
        } else if self.selected_interaction >= pending_interactions_len {
            self.selected_interaction = pending_interactions_len - 1;
        }
    }

    pub fn open_selected_session_detail(&mut self) {
        if let Some(session) = self.sessions.get(self.selected_session) {
            self.session_detail_id = Some(session.id.clone());
        }
    }

    pub fn close_session_detail(&mut self) {
        self.session_detail_id = None;
    }

    pub fn remove_session(&mut self, session_id: &str) {
        self.sessions.retain(|s| s.id.0 != session_id);
        self.session_terminal_buffers.remove(session_id);
        self.session_transcripts.remove(session_id);
        self.session_prompt_histories.remove(session_id);
        self.session_prompt_buffers.remove(session_id);
        if let Some(detail_id) = &self.session_detail_id {
            if detail_id.0 == session_id {
                self.session_detail_id = None;
            }
        }
        self.clamp_selections();
    }

    /// Access or lazily create mutable prompt history for a session.
    pub fn prompt_history_for_session_mut(&mut self, session_id: &str) -> &mut PromptHistory {
        self.session_prompt_histories
            .entry(session_id.to_string())
            .or_default()
    }

    /// Access prompt history for a session (read-only).
    pub fn prompt_history_for_session(&self, session_id: &str) -> Option<&PromptHistory> {
        self.session_prompt_histories.get(session_id)
    }

    /// Current unsubmitted draft prompt buffer for a session.
    pub fn session_prompt_buffer(&self, session_id: &str) -> &str {
        self.session_prompt_buffers
            .get(session_id)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    /// Update the draft prompt buffer for a session.
    pub fn set_session_prompt_buffer(&mut self, session_id: &str, buf: String) {
        self.session_prompt_buffers
            .insert(session_id.to_string(), buf);
    }

    /// Clear the draft prompt buffer for a session.
    pub fn clear_session_prompt_buffer(&mut self, session_id: &str) {
        self.session_prompt_buffers.remove(session_id);
    }

    /// Returns true if slash-command completion popup is currently open and active for the session.
    pub fn is_session_completion_open(&self, session_id: &str) -> bool {
        if self
            .session_completion_dismissed
            .get(session_id)
            .copied()
            .unwrap_or(false)
        {
            return false;
        }
        if let Some(buf) = self.session_terminal_buffers.get(session_id) {
            buf.is_completion_open()
        } else {
            false
        }
    }

    /// Mark completion menu as dismissed (e.g. on Esc, Tab, Enter) or active again (e.g. typing).
    pub fn set_session_completion_dismissed(&mut self, session_id: &str, dismissed: bool) {
        if dismissed {
            self.session_completion_dismissed
                .insert(session_id.to_string(), true);
        } else {
            self.session_completion_dismissed.remove(session_id);
        }
    }

    /// Attempt to read the current prompt line directly from the terminal buffer.
    pub fn session_terminal_prompt(&self, session_id: &str) -> Option<String> {
        let buf = self.session_terminal_buffers.get(session_id)?;
        if buf.cursor_row < buf.lines.len() {
            let line = buf.lines[buf.cursor_row].to_plain_string();
            if let Some(stripped) = line.strip_prefix("> ") {
                let p = stripped.trim_end().to_string();
                if !p.is_empty() {
                    return Some(p);
                }
            } else if let Some(stripped) = line.strip_prefix('>') {
                let p = stripped.trim_start().trim_end().to_string();
                if !p.is_empty() {
                    return Some(p);
                }
            }
        }
        None
    }

    // ── Queries / Filtered Views ────────────────────────────────────────────

    pub fn selected_session(&self) -> Option<&AgentSession> {
        self.sessions.get(self.selected_session)
    }

    pub fn selected_account(&self) -> Option<&Account> {
        self.accounts.get(self.selected_account)
    }

    pub fn selected_session_or_detail(&self) -> Option<&AgentSession> {
        if let Some(detail_id) = &self.session_detail_id {
            self.sessions.iter().find(|s| &s.id == detail_id)
        } else {
            self.selected_session()
        }
    }

    pub fn account_label(&self, account_id: Option<&Id>) -> String {
        match account_id {
            Some(aid) => self
                .accounts
                .iter()
                .find(|a| &a.id == aid)
                .map(|a| a.label.clone())
                .unwrap_or_else(|| aid.0.clone()),
            None => "None".to_string(),
        }
    }

    pub fn pending_interactions(&self) -> Vec<&Interaction> {
        self.interactions
            .iter()
            .filter(|i| i.state == InteractionState::Pending)
            .collect()
    }

    pub fn selected_pending_interaction(&self) -> Option<&Interaction> {
        let pending = self.pending_interactions();
        pending.get(self.selected_interaction).copied()
    }

    pub fn filtered_events(&self) -> Vec<&AgentEvent> {
        if let Some(filter) = &self.activity_filter {
            let filter_lower = filter.to_lowercase();
            self.events
                .iter()
                .filter(|e| {
                    e.kind.to_string().to_lowercase().contains(&filter_lower)
                        || e.session_id
                            .as_ref()
                            .map(|s| s.0.to_lowercase().contains(&filter_lower))
                            .unwrap_or(false)
                        || e.triggered_by.to_lowercase().contains(&filter_lower)
                })
                .collect()
        } else {
            self.events.iter().collect()
        }
    }

    // ── Real-Time Event Processing ──────────────────────────────────────────

    pub fn apply_event(&mut self, event: AgentEvent) {
        // Append to transcript and terminal buffer if associated with a session
        if let Some(session_id) = &event.session_id {
            let time_str = event.timestamp.format("%H:%M:%S").to_string();

            // 1. Update live terminal buffer with pure agent output
            let term_buf = self
                .session_terminal_buffers
                .entry(session_id.0.clone())
                .or_default();

            if let EventKind::AgentOutputReceived = event.kind {
                let text = event.payload["text"].as_str().unwrap_or("");
                term_buf.push_str(text);
            }

            // 2. Also keep legacy transcript for simple queries/tests
            let transcript = self
                .session_transcripts
                .entry(session_id.0.clone())
                .or_default();

            match event.kind {
                EventKind::AgentOutputReceived => {
                    let text = event.payload["text"].as_str().unwrap_or("");
                    transcript.push(format!("[{time_str}] [OUT] {text}"));
                }
                EventKind::StateChanged => {
                    let from = event.payload["from"].as_str().unwrap_or("?");
                    let to = event.payload["to"].as_str().unwrap_or("?");
                    transcript.push(format!("[{time_str}] [STATE] {from} -> {to}"));
                }
                EventKind::AgentSteeringReceived => {
                    let msg = event.payload["message"].as_str().unwrap_or("");
                    transcript.push(format!("[{time_str}] [STEER] {msg}"));
                }
                EventKind::AgentApprovalRequested => {
                    let tool = event.payload["tool_name"].as_str().unwrap_or("tool");
                    transcript.push(format!("[{time_str}] [APPROVAL_REQ] {tool}"));
                }
                EventKind::AgentQuestion => {
                    let prompt = event.payload["prompt"].as_str().unwrap_or("");
                    transcript.push(format!("[{time_str}] [QUESTION] {prompt}"));
                }
                EventKind::SessionCrashed => {
                    transcript.push(format!("[{time_str}] [CRASHED] Session process crashed"));
                }
                EventKind::SessionFailed => {
                    transcript.push(format!(
                        "[{time_str}] [FAILED] Session entered failed state"
                    ));
                }
                EventKind::SessionStopped => {
                    transcript.push(format!("[{time_str}] [STOPPED] Session stopped"));
                }
                _ => {
                    transcript.push(format!("[{time_str}] [{}]", event.kind));
                }
            }

            // Keep transcript bounded
            if transcript.len() > 1000 {
                transcript.drain(0..200);
            }

            // Update session state in-place if matching
            if let Some(session) = self.sessions.iter_mut().find(|s| &s.id == session_id) {
                session.updated_at = event.timestamp;
                match event.kind {
                    EventKind::StateChanged => {
                        if let Some(to_state) = event.payload["to"].as_str().and_then(|s| {
                            serde_json::from_value::<SessionState>(serde_json::Value::String(
                                s.to_string(),
                            ))
                            .ok()
                        }) {
                            session.state = to_state;
                        }
                    }
                    EventKind::SessionStarted | EventKind::SessionReady => {
                        session.state = SessionState::Working;
                    }
                    EventKind::SessionPaused => {
                        session.state = SessionState::Paused;
                    }
                    EventKind::SessionResumed => {
                        session.state = SessionState::Working;
                    }
                    EventKind::SessionStopped => {
                        session.state = SessionState::Stopped;
                    }
                    EventKind::SessionCrashed => {
                        session.state = SessionState::Crashed;
                    }
                    EventKind::SessionFailed => {
                        session.state = SessionState::Failed;
                    }
                    EventKind::AccountSelected => {
                        if let Some(aid) = event.payload["account_id"].as_str() {
                            session.account_id = Some(Id::from(aid));
                        }
                    }
                    EventKind::AccountSwitchCompleted => {
                        if let Some(aid) = event.payload["target_account_id"].as_str() {
                            session.account_id = Some(Id::from(aid));
                        }
                    }
                    EventKind::SessionHandOffCompleted | EventKind::SessionHandedOff => {
                        session.state = SessionState::HandedOff;
                        let succ = event.payload["successor_session_id"]
                            .as_str()
                            .or_else(|| event.payload["successor_id"].as_str());
                        if let Some(s) = succ {
                            session.successor_id = Some(Id::from(s));
                        }
                    }
                    _ => {}
                }
            }
        }

        // Handle interaction events
        match event.kind {
            EventKind::InteractionAutoResolved
            | EventKind::InteractionHumanResolved
            | EventKind::InteractionDismissed
            | EventKind::InteractionExpired => {
                if let Some(iid) = event.payload["interaction_id"].as_str() {
                    if let Some(interaction) = self.interactions.iter_mut().find(|i| i.id.0 == iid)
                    {
                        interaction.state = match event.kind {
                            EventKind::InteractionAutoResolved => InteractionState::AutoResolved,
                            EventKind::InteractionHumanResolved => InteractionState::HumanResolved,
                            EventKind::InteractionDismissed => InteractionState::Dismissed,
                            EventKind::InteractionExpired => InteractionState::Expired,
                            _ => interaction.state.clone(),
                        };
                    }
                }
            }
            _ => {}
        }

        // Handle session removal
        if event.kind == EventKind::SessionRemoved {
            if let Some(session_id) = &event.session_id {
                self.remove_session(&session_id.0);
            }
        }

        // If approval is requested for the active session, present the approval modal overlay
        if (event.kind == EventKind::ApprovalRequested
            || event.kind == EventKind::AgentApprovalRequested)
            && event.session_id.is_some()
            && event.session_id.as_ref() == self.session_detail_id.as_ref()
        {
            let interaction_id = event
                .payload
                .get("interaction_id")
                .and_then(|v| v.as_str())
                .map(Id::from)
                .unwrap_or_else(|| Id::from("unknown"));
            let tool_name = event
                .payload
                .get("tool_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let prompt = event
                .payload
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("Action requires human approval")
                .to_string();

            self.active_modal = Some(Modal::Approval {
                interaction_id,
                tool_name,
                prompt,
            });
        }

        // Prepend event to events list
        self.events.insert(0, event);
        if self.events.len() > 1000 {
            self.events.truncate(1000);
        }
    }

    pub fn scroll_session_terminal_up(&mut self, session_id: &str, delta: usize) {
        if let Some(buf) = self.session_terminal_buffers.get_mut(session_id) {
            buf.scroll_up(delta);
        }
    }

    pub fn scroll_session_terminal_down(&mut self, session_id: &str, delta: usize) {
        if let Some(buf) = self.session_terminal_buffers.get_mut(session_id) {
            buf.scroll_down(delta);
        }
    }

    pub fn scroll_session_terminal_top(&mut self, session_id: &str) {
        if let Some(buf) = self.session_terminal_buffers.get_mut(session_id) {
            buf.scroll_to_top();
        }
    }

    pub fn scroll_session_terminal_bottom(&mut self, session_id: &str) {
        if let Some(buf) = self.session_terminal_buffers.get_mut(session_id) {
            buf.scroll_to_bottom();
        }
    }

    pub fn clear_session_terminal(&mut self, session_id: &str) {
        if let Some(buf) = self.session_terminal_buffers.get_mut(session_id) {
            buf.clear();
        }
    }

    /// Keep every terminal grid the same size as the PTYs (`rows` × `cols`).
    pub fn resize_session_terminals(&mut self, rows: u16, cols: u16) {
        for buf in self.session_terminal_buffers.values_mut() {
            buf.resize(rows as usize, cols as usize);
        }
    }

    /// Whether the session's program currently owns the alternate screen.
    pub fn session_in_alt_screen(&self, session_id: &str) -> bool {
        self.session_terminal_buffers
            .get(session_id)
            .is_some_and(|b| b.in_alt_screen())
    }
}
