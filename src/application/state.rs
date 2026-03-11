//! Application state — the single source of truth for the UI.

use crate::adapters::views::ViewKind;
use crate::application::actions::Action;
use crate::application::nav::NavigationStack;
use crate::domain::entities::InboxMessage;
use crate::infrastructure::fs::store::DataStore;

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
    pub filter: String,
    pub show_help: bool,
    pub spinner: Option<String>,
    pub toast: Option<String>,
    pub confirm_message: Option<String>,
    pub confirm_action: Option<Action>,
    pub tick: usize,
    pub inbox_messages: Vec<InboxMessage>,
    pub show_all_sessions: bool,
    /// Currently attached daemon session ID.
    pub attached_session: Option<String>,
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
            filter: String::new(),
            show_help: false,
            spinner: None,
            toast: None,
            confirm_message: None,
            confirm_action: None,
            tick: 0,
            inbox_messages: Vec::new(),
            show_all_sessions: false,
            attached_session: None,
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
}
