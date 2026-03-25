//! Application state — the single source of truth for the UI.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::adapters::views::ViewKind;
use crate::application::actions::Action;
use crate::application::nav::NavigationStack;
use crate::application::store::DataStore;
use crate::domain::entities::{InboxMessage, Session};

/// Phases of the self-update process (displayed in the update overlay).
#[derive(Debug, Clone)]
pub enum UpdatePhase {
    Checking,
    Downloading { version: String },
    Extracting,
    Installing,
    Done { version: String },
    Failed { message: String },
}

/// Serializable snapshot of UI state for persistence across restarts.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UiSnapshot {
    /// Navigation stack entries: [(view_kind_key, optional_context), ...]
    #[serde(default)]
    pub nav_stack: Vec<(String, Option<String>)>,
    /// Selected row index in the current table view.
    #[serde(default)]
    pub selected: usize,
    /// Session filter mode: "active" or "all".
    #[serde(default)]
    pub session_filter: String,
    /// Session IDs with expanded subagent rows.
    #[serde(default)]
    pub expanded_sessions: Vec<String>,
}

/// Session filter mode for the sessions view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFilter {
    /// Only active/running sessions (default).
    Active,
    /// All sessions (active + idle).
    All,
}

impl SessionFilter {
    pub fn next(self) -> Self {
        match self {
            Self::Active => Self::All,
            Self::All => Self::Active,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::All => "all",
        }
    }
}

/// Input mode for the application.
#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    Command,
    Filter,
    Confirm,
    /// Prompting user for the directory for a new session.
    NewSession,
    /// Prompting user for the name of a new session (after directory).
    NewSessionName,
    /// Prompting user whether to start the new session in a worktree (y/n).
    NewSessionWorktree,
    /// Attached to a daemon PTY session — keystrokes go to the session.
    Attached,
    /// Picker dialog is open — j/k navigate, Enter selects, Esc cancels.
    Picker,
}

/// A generic picker dialog — list of items with a callback action.
#[derive(Debug, Clone)]
pub struct PickerDialog {
    pub title: String,
    pub items: Vec<PickerItem>,
    pub selected: usize,
    pub on_select_action: PickerAction,
}

/// An item in a picker dialog.
#[derive(Debug, Clone)]
pub struct PickerItem {
    pub label: String,
    pub description: String,
    /// Opaque value — e.g. `"code"` or `"terminal:nvim"`.
    pub value: String,
}

/// What happens when a picker item is selected.
#[derive(Debug, Clone)]
pub enum PickerAction {
    OpenInIde { project_dir: String },
}

/// Table selection state.
#[derive(Debug, Clone, Default)]
pub struct TableState {
    pub selected: usize,
}

/// Scroll state for detail views.
#[derive(Debug, Clone, Default)]
pub struct ScrollState {
    pub offset: u16,
}

/// A confirmation dialog — simple yes/no.
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub message: String,
    pub on_confirm: Action,
}

/// Main application state — everything the reducer and renderer need.
pub struct AppState {
    pub nav: NavigationStack,
    pub store: DataStore,
    pub table_state: TableState,
    pub scroll_state: ScrollState,
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub input_cursor: usize,
    pub filter: String,
    pub show_help: bool,
    pub help_scroll: u16,
    pub spinner: Option<String>,
    pub toast: Option<String>,
    pub confirm_dialog: Option<ConfirmDialog>,
    pub picker_dialog: Option<PickerDialog>,
    pub tick: usize,
    pub inbox_messages: Vec<InboxMessage>,
    pub session_filter: SessionFilter,
    /// Currently attached daemon session ID.
    pub attached_session: Option<String>,
    /// Sessions with expanded subagent rows in the Sessions table.
    pub expanded_sessions: HashSet<String>,
    /// Default working directory for new sessions (where clash was started).
    pub default_cwd: String,
    /// Pending CWD for new session (set during the two-step creation flow).
    pub pending_session_cwd: Option<String>,
    /// Whether the pending new session should use a worktree.
    pub pending_session_worktree: bool,
    /// Guided tour state: Some(step_index) when active, None when inactive.
    pub tour_step: Option<usize>,
    /// vt100 screen for inline terminal rendering when attached to a session.
    pub terminal_screen: Option<vt100::Parser>,
    /// Sessions currently open in external panes/tabs/windows.
    /// Tracked in-memory only — cleared on restart.
    pub externally_opened: HashSet<String>,
    /// Debug mode flag — enables verbose logging.
    pub debug_mode: bool,
    /// Self-update progress (shown as an overlay when active).
    pub update_progress: Option<UpdatePhase>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            nav: NavigationStack::new(),
            store: DataStore::new(),
            table_state: TableState::default(),
            scroll_state: ScrollState::default(),
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            input_cursor: 0,
            filter: String::new(),
            show_help: false,
            help_scroll: 0,
            spinner: None,
            toast: None,
            confirm_dialog: None,
            picker_dialog: None,
            tick: 0,
            inbox_messages: Vec::new(),
            session_filter: SessionFilter::Active,
            attached_session: None,
            expanded_sessions: HashSet::new(),
            default_cwd: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            pending_session_cwd: None,
            pending_session_worktree: false,
            tour_step: None,
            terminal_screen: None,
            externally_opened: HashSet::new(),
            debug_mode: false,
            update_progress: None,
        }
    }

    pub fn current_view(&self) -> ViewKind {
        self.nav.current().view
    }

    pub fn current_team(&self) -> Option<&str> {
        self.nav.current_team()
    }

    pub fn current_session(&self) -> Option<&str> {
        self.nav.current_session()
    }

    /// Capture current UI state for persistence.
    pub fn snapshot(&self) -> UiSnapshot {
        UiSnapshot {
            nav_stack: self
                .nav
                .entries()
                .iter()
                .map(|entry| (entry.view.key().to_string(), entry.context.clone()))
                .collect(),
            selected: self.table_state.selected,
            session_filter: self.session_filter.label().to_string(),
            expanded_sessions: self.expanded_sessions.iter().cloned().collect(),
        }
    }

    /// Restore UI state from a snapshot (best-effort — stale/invalid entries are skipped).
    pub fn restore(&mut self, snapshot: UiSnapshot) {
        // Restore session filter
        self.session_filter = match snapshot.session_filter.as_str() {
            "all" => SessionFilter::All,
            _ => SessionFilter::Active,
        };

        // Restore selected row
        self.table_state.selected = snapshot.selected;

        // Restore expanded sessions
        self.expanded_sessions = snapshot.expanded_sessions.into_iter().collect();

        // Restore navigation stack
        if !snapshot.nav_stack.is_empty() {
            let mut valid_entries = Vec::new();
            for (key, context) in &snapshot.nav_stack {
                if let Some(view) = ViewKind::from_key(key) {
                    valid_entries.push((view, context.clone()));
                } else {
                    break; // Stop at first invalid entry
                }
            }
            if !valid_entries.is_empty() {
                self.nav.restore_from(valid_entries);
            }
        }
    }

    /// Get filtered sessions based on the current session filter and text filter.
    pub fn filtered_sessions(&self) -> Vec<&Session> {
        let status_filtered: Vec<&Session> = match self.session_filter {
            SessionFilter::All => self.store.sessions.iter().collect(),
            SessionFilter::Active => self
                .store
                .sessions
                .iter()
                .filter(|s| {
                    s.is_running || s.status == crate::domain::entities::SessionStatus::Errored
                })
                .collect(),
        };

        if self.filter.is_empty() {
            return status_filtered;
        }

        status_filtered
            .into_iter()
            .filter(|s| s.matches_filter(&self.filter))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::Session;

    #[test]
    fn test_filtered_sessions_default_active() {
        let mut state = AppState::new();
        state.store.sessions = vec![
            Session {
                id: "s1".to_string(),
                is_running: true,
                ..Default::default()
            },
            Session {
                id: "s2".to_string(),
                is_running: false,
                ..Default::default()
            },
        ];
        // Default filter is Active — only running sessions shown
        assert_eq!(state.filtered_sessions().len(), 1);
        assert_eq!(state.filtered_sessions()[0].id, "s1");
    }

    #[test]
    fn test_filtered_sessions_active_only() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::Active;
        state.store.sessions = vec![
            Session {
                id: "s1".to_string(),
                is_running: true,
                ..Default::default()
            },
            Session {
                id: "s2".to_string(),
                is_running: false,
                ..Default::default()
            },
        ];
        let filtered = state.filtered_sessions();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "s1");
    }

    #[test]
    fn test_filtered_sessions_with_text_filter() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::All;
        state.store.sessions = vec![
            Session {
                id: "s1".to_string(),
                summary: "Fix login".to_string(),
                ..Default::default()
            },
            Session {
                id: "s2".to_string(),
                summary: "Add tests".to_string(),
                ..Default::default()
            },
        ];
        state.filter = "login".to_string();
        let filtered = state.filtered_sessions();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "s1");
    }
}
