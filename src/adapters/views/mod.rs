//! View definitions — traits and implementations for each screen.
//!
//! Views are adapters: they translate application state into UI-renderable structures.

pub mod agents;
pub mod agent_detail;
pub mod inbox;
pub mod prompts;
pub mod session_detail;
pub mod sessions;
pub mod subagent_detail;
pub mod subagents;
pub mod tasks;
pub mod task_detail;
pub mod teams;
pub mod team_detail;

use ratatui::widgets::Cell;

use crate::application::actions::Action;
use crate::application::state::AppState;

/// All view kinds in the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Teams,
    TeamDetail,
    Agents,
    AgentDetail,
    Tasks,
    TaskDetail,
    Inbox,
    Prompts,
    Sessions,
    SessionDetail,
    Subagents,
    SubagentDetail,
}

impl ViewKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Teams => "Teams",
            Self::TeamDetail => "Team",
            Self::Agents => "Agents",
            Self::AgentDetail => "Agent",
            Self::Tasks => "Tasks",
            Self::TaskDetail => "Task",
            Self::Inbox => "Inbox",
            Self::Prompts => "Prompts",
            Self::Sessions => "Sessions",
            Self::SessionDetail => "Session",
            Self::Subagents => "Subagents",
            Self::SubagentDetail => "Subagent",
        }
    }
}

/// Column definition for table views.
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub width_pct: u16,
}

impl ColumnDef {
    pub fn new(name: &str, width_pct: u16) -> Self {
        Self {
            name: name.to_string(),
            width_pct,
        }
    }
}

/// A keybinding hint shown in help overlay.
#[derive(Debug, Clone)]
pub struct Keybinding {
    pub key: String,
    pub description: String,
}

impl Keybinding {
    pub fn new(key: &str, desc: &str) -> Self {
        Self {
            key: key.to_string(),
            description: desc.to_string(),
        }
    }
}

/// A section in detail views.
#[derive(Debug, Clone)]
pub struct Section {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

impl Section {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            rows: Vec::new(),
        }
    }

    pub fn row(mut self, label: &str, value: &str) -> Self {
        self.rows.push((label.to_string(), value.to_string()));
        self
    }
}

/// Trait for views that render as a table.
#[allow(dead_code)]
pub trait TableView {
    type Item;

    fn columns() -> Vec<ColumnDef>;
    fn row(item: &Self::Item) -> Vec<Cell<'static>>;
    fn items(state: &AppState) -> Vec<&Self::Item>;
    fn on_select(item: &Self::Item) -> Action;
    fn context_keybindings() -> Vec<Keybinding>;
    fn empty_message() -> &'static str {
        "No items"
    }
}

/// Trait for views that render as a detail/info panel.
pub trait DetailView {
    fn title(state: &AppState) -> String;
    fn sections(state: &AppState) -> Vec<Section>;
    fn context_keybindings() -> Vec<Keybinding>;
}
