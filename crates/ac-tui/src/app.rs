//! App state and business logic for the TUI.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use ac_core::types::{
    Account, AgentEvent, AgentSession, EventKind, Id, Interaction, InteractionState,
    Project, SessionState,
};

use crate::terminal_buffer::TerminalBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard = 0,
    Sessions = 1,
    Inbox = 2,
    Accounts = 3,
    Projects = 4,
    Activity = 5,
    Agents = 6,
    Settings = 7,
}

impl Tab {
    pub fn from_index(index: usize) -> Self {
        match index {
            0 => Tab::Dashboard,
            1 => Tab::Sessions,
            2 => Tab::Inbox,
            3 => Tab::Accounts,
            4 => Tab::Projects,
            5 => Tab::Activity,
            6 => Tab::Agents,
            7 => Tab::Settings,
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
            Tab::Inbox => "Inbox",
            Tab::Accounts => "Accounts",
            Tab::Projects => "Projects",
            Tab::Activity => "Activity",
            Tab::Agents => "Agents",
            Tab::Settings => "Settings",
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Steer { session_id: Id, input: String },
    Reply { interaction_id: Id, input: String },
    ConfirmStop { session_id: Id },
    FilterActivity { input: String },
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
        task: String,
        active_field: usize,
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
        }
    }

    pub fn next_detail_subtab(&mut self) {
        self.detail_subtab = (self.detail_subtab + 1) % 4;
    }

    pub fn prev_detail_subtab(&mut self) {
        self.detail_subtab = if self.detail_subtab == 0 { 3 } else { self.detail_subtab - 1 };
    }

    pub fn next_sidebar_item(&mut self) {
        self.sidebar_selected = (self.sidebar_selected + 1) % 10;
    }

    pub fn prev_sidebar_item(&mut self) {
        self.sidebar_selected = if self.sidebar_selected == 0 { 9 } else { self.sidebar_selected - 1 };
    }

    pub fn set_status(&mut self, message: impl Into<String>, status_type: StatusType) {
        self.status_message = Some((message.into(), status_type, Instant::now()));
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
            Tab::Activity => 6,
            Tab::Settings => 7,
            _ => self.sidebar_selected,
        };
    }

    pub fn next_tab(&mut self) {
        let next_idx = (self.current_tab.to_index() + 1) % 6;
        self.current_tab = Tab::from_index(next_idx);
        self.session_detail_id = None;
    }

    pub fn prev_tab(&mut self) {
        let prev_idx = if self.current_tab.to_index() == 0 {
            5
        } else {
            self.current_tab.to_index() - 1
        };
        self.current_tab = Tab::from_index(prev_idx);
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
            Tab::Inbox => {
                let count = self.pending_interactions().len();
                if count > 0 {
                    self.selected_interaction = (self.selected_interaction + 1) % count;
                }
            }
            Tab::Accounts => {
                if !self.accounts.is_empty() {
                    self.selected_account = (self.selected_account + 1) % self.accounts.len();
                }
            }
            Tab::Projects => {
                if !self.projects.is_empty() {
                    self.selected_project = (self.selected_project + 1) % self.projects.len();
                }
            }
            Tab::Activity => {
                let count = self.filtered_events().len();
                if count > 0 {
                    self.selected_event = (self.selected_event + 1) % count;
                }
            }
            Tab::Settings => {}
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
                self.selected_agent = if self.selected_agent == 0 { 2 } else { self.selected_agent - 1 };
            }
            Tab::Inbox => {
                let count = self.pending_interactions().len();
                if count > 0 {
                    self.selected_interaction = if self.selected_interaction == 0 {
                        count - 1
                    } else {
                        self.selected_interaction - 1
                    };
                }
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
            Tab::Projects => {
                if !self.projects.is_empty() {
                    self.selected_project = if self.selected_project == 0 {
                        self.projects.len() - 1
                    } else {
                        self.selected_project - 1
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
            Tab::Settings => {}
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

            // 1. Update live terminal buffer
            let term_buf = self
                .session_terminal_buffers
                .entry(session_id.0.clone())
                .or_default();

            match event.kind {
                EventKind::AgentOutputReceived => {
                    let text = event.payload["text"].as_str().unwrap_or("");
                    term_buf.push_str(text);
                }
                EventKind::StateChanged => {
                    let from = event.payload["from"].as_str().unwrap_or("?");
                    let to = event.payload["to"].as_str().unwrap_or("?");
                    term_buf.push_system_line(
                        &format!("  {time_str}  ● State changed: {from} -> {to}"),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Cyan),
                    );
                }
                EventKind::AgentSteeringReceived => {
                    let msg = event.payload["message"].as_str().unwrap_or("");
                    term_buf.push_system_line(
                        &format!("  {time_str}  ➜ User steering: {msg}"),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Green),
                    );
                }
                EventKind::AgentApprovalRequested => {
                    let tool = event.payload["tool_name"].as_str().unwrap_or("tool");
                    term_buf.push_system_line(
                        &format!("  {time_str}  ⚠ Tool approval requested: {tool}"),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                    );
                }
                EventKind::AgentQuestion => {
                    let prompt = event.payload["prompt"].as_str().unwrap_or("");
                    term_buf.push_system_line(
                        &format!("  {time_str}  ❓ Agent question: {prompt}"),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                    );
                }
                EventKind::SessionCrashed => {
                    term_buf.push_system_line(
                        &format!("  {time_str}  ✖ Session crashed"),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Red),
                    );
                }
                EventKind::SessionFailed => {
                    term_buf.push_system_line(
                        &format!("  {time_str}  ✖ Session failed"),
                        ratatui::style::Style::default().fg(ratatui::style::Color::Red),
                    );
                }
                EventKind::SessionStopped => {
                    term_buf.push_system_line(
                        &format!("  {time_str}  ■ Session stopped"),
                        ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray),
                    );
                }
                _ => {}
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
                    transcript.push(format!("[{time_str}] [FAILED] Session entered failed state"));
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
                        if let Some(to_state) = event.payload["to"]
                            .as_str()
                            .and_then(|s| serde_json::from_value::<SessionState>(serde_json::Value::String(s.to_string())).ok())
                        {
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
                    if let Some(interaction) = self.interactions.iter_mut().find(|i| i.id.0 == iid) {
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
}
