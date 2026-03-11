//! Application state — the single source of truth for the UI.

use crate::adapters::views::ViewKind;
use crate::application::actions::Action;
use crate::application::nav::NavigationStack;
use crate::domain::entities::{InboxMessage, Session, Subagent};
use crate::infrastructure::fs::store::DataStore;

/// A row in the sessions tree view — either a top-level session or a nested subagent.
#[derive(Debug, Clone)]
pub enum SessionTreeRow {
    Session(Session),
    Subagent {
        subagent: Subagent,
        /// Whether this is the last subagent under its parent (for `└─` vs `├─`).
        is_last: bool,
    },
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
    /// Attached to a daemon PTY session — keystrokes go to the session.
    Attached,
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
    pub spinner: Option<String>,
    pub toast: Option<String>,
    pub confirm_message: Option<String>,
    pub confirm_action: Option<Action>,
    pub tick: usize,
    pub inbox_messages: Vec<InboxMessage>,
    pub session_filter: SessionFilter,
    /// Currently attached daemon session ID.
    pub attached_session: Option<String>,
    /// Flat tree of sessions interleaved with their subagents.
    pub session_tree: Vec<SessionTreeRow>,
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
            spinner: None,
            toast: None,
            confirm_message: None,
            confirm_action: None,
            tick: 0,
            inbox_messages: Vec::new(),
            session_filter: SessionFilter::Active,
            attached_session: None,
            session_tree: Vec::new(),
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

    /// Rebuild the flat session tree from sessions + subagents_by_session.
    pub fn rebuild_session_tree(&mut self) {
        let mut tree = Vec::new();
        for session in &self.store.sessions {
            tree.push(SessionTreeRow::Session(session.clone()));
            if let Some(subagents) = self.store.subagents_by_session.get(&session.id) {
                let count = subagents.len();
                for (i, sa) in subagents.iter().enumerate() {
                    tree.push(SessionTreeRow::Subagent {
                        subagent: sa.clone(),
                        is_last: i == count - 1,
                    });
                }
            }
        }
        self.session_tree = tree;
    }

    /// Get filtered session tree items based on the current session filter
    /// and the text filter (`/` search).
    pub fn filtered_session_tree(&self) -> Vec<&SessionTreeRow> {
        let filter_lower = self.filter.to_lowercase();

        let status_filtered: Vec<&SessionTreeRow> = match self.session_filter {
            SessionFilter::All => self.session_tree.iter().collect(),
            SessionFilter::Active => {
                let running_ids: std::collections::HashSet<&str> = self
                    .store
                    .sessions
                    .iter()
                    .filter(|s| s.is_running)
                    .map(|s| s.id.as_str())
                    .collect();

                self.session_tree
                    .iter()
                    .filter(|row| match row {
                        SessionTreeRow::Session(s) => s.is_running,
                        SessionTreeRow::Subagent { subagent, .. } => {
                            subagent.is_running
                                && running_ids.contains(subagent.parent_session_id.as_str())
                        }
                    })
                    .collect()
            }
        };

        // Apply text filter if set
        if filter_lower.is_empty() {
            return status_filtered;
        }

        // First pass: find which parent session IDs match the filter
        let matching_session_ids: std::collections::HashSet<&str> = status_filtered
            .iter()
            .filter_map(|row| match row {
                SessionTreeRow::Session(s) if Self::row_matches_filter(row, &filter_lower) => {
                    Some(s.id.as_str())
                }
                _ => None,
            })
            .collect();

        // Second pass: include matching sessions + their subagents (parent must be shown)
        status_filtered
            .into_iter()
            .filter(|row| match row {
                SessionTreeRow::Session(_) => Self::row_matches_filter(row, &filter_lower),
                SessionTreeRow::Subagent { subagent, .. } => {
                    matching_session_ids.contains(subagent.parent_session_id.as_str())
                }
            })
            .collect()
    }

    /// Check if a session tree row matches a text filter (case-insensitive).
    fn row_matches_filter(row: &SessionTreeRow, filter: &str) -> bool {
        match row {
            SessionTreeRow::Session(s) => {
                s.id.to_lowercase().contains(filter)
                    || s.summary.to_lowercase().contains(filter)
                    || s.project_path.to_lowercase().contains(filter)
                    || s.git_branch.to_lowercase().contains(filter)
                    || s.first_prompt.to_lowercase().contains(filter)
            }
            SessionTreeRow::Subagent { subagent, .. } => {
                subagent.id.to_lowercase().contains(filter)
                    || subagent.summary.to_lowercase().contains(filter)
                    || subagent.agent_type.to_lowercase().contains(filter)
                    || subagent.file_path.to_lowercase().contains(filter)
            }
        }
    }
}
