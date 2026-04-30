//! View definitions — traits and implementations for each screen.
//!
//! Views are adapters: they translate application state into UI-renderable structures.

pub mod agent_detail;
pub mod agents;
pub mod diff;
pub mod inbox;
pub mod prompts;
pub mod session_detail;
pub mod sessions;
pub mod subagent_detail;
pub mod subagents;
pub mod task_detail;
pub mod tasks;
pub mod team_detail;
pub mod teams;

use ratatui::widgets::Cell;

use crate::application::state::AppState;
use crate::domain::entities::SessionSource;

/// Single-character glyph for a session's `source` — drawn as a row
/// prefix in the sessions list and reused in the help/tour legends.
///
/// Centralized here so the visual character is defined exactly once
/// (mirrors the `worktree_display_from_cwd` precedent in CLAUDE.md).
/// Returns the empty string for `Daemon` so daemon-managed rows have
/// no prefix at all — the visual default.
pub fn source_glyph(source: SessionSource) -> &'static str {
    match source {
        SessionSource::Daemon => "",
        SessionSource::External => "\u{229e} ", // ⊞
        SessionSource::Wild => "\u{1f33f} ",    // 🌿
        SessionSource::Unknown => "",
    }
}

/// Full-word label for a session's `source`, for the help/tour legend
/// and any future status-bar surface that wants prose. Empty for the
/// default cases that have no glyph.
#[allow(dead_code)] // wired up by status-bar hint in PR 1 — Step 7 (a-key flow)
pub fn source_label(source: SessionSource) -> &'static str {
    match source {
        SessionSource::Daemon => "",
        SessionSource::External => "external pane",
        SessionSource::Wild => "wild claude (outside daemon)",
        SessionSource::Unknown => "",
    }
}

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
    Diff,
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
            Self::Diff => "Diff",
            Self::Subagents => "Subagents",
            Self::SubagentDetail => "Subagent",
        }
    }

    /// Machine-readable key for serialization (used in UI state persistence).
    pub fn key(&self) -> &'static str {
        match self {
            Self::Teams => "teams",
            Self::TeamDetail => "team_detail",
            Self::Agents => "agents",
            Self::AgentDetail => "agent_detail",
            Self::Tasks => "tasks",
            Self::TaskDetail => "task_detail",
            Self::Inbox => "inbox",
            Self::Prompts => "prompts",
            Self::Sessions => "sessions",
            Self::SessionDetail => "session_detail",
            Self::Diff => "diff",
            Self::Subagents => "subagents",
            Self::SubagentDetail => "subagent_detail",
        }
    }

    /// Parse from a serialized key string.
    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "teams" => Some(Self::Teams),
            "team_detail" => Some(Self::TeamDetail),
            "agents" => Some(Self::Agents),
            "agent_detail" => Some(Self::AgentDetail),
            "tasks" => Some(Self::Tasks),
            "task_detail" => Some(Self::TaskDetail),
            "inbox" => Some(Self::Inbox),
            "prompts" => Some(Self::Prompts),
            "sessions" => Some(Self::Sessions),
            "session_detail" => Some(Self::SessionDetail),
            "diff" => Some(Self::Diff),
            "subagents" => Some(Self::Subagents),
            "subagent_detail" => Some(Self::SubagentDetail),
            _ => None,
        }
    }
}

/// How a column should be sized.
#[derive(Debug, Clone, Copy)]
pub enum ColumnSizing {
    /// Fixed percentage of table width (legacy behavior).
    Pct(u16),
    /// Fit to content: uses measured content width, clamped to `min..=max`.
    /// Remaining space is distributed proportionally among Flex columns.
    Flex { min: u16, max: u16 },
}

/// Column definition for table views.
#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub sizing: ColumnSizing,
}

impl ColumnDef {
    /// Create a fixed-percentage column.
    pub fn new(name: &str, width_pct: u16) -> Self {
        Self {
            name: name.to_string(),
            sizing: ColumnSizing::Pct(width_pct),
        }
    }

    /// Create a flex column that sizes to content within `min..=max` chars.
    pub fn flex(name: &str, min: u16, max: u16) -> Self {
        Self {
            name: name.to_string(),
            sizing: ColumnSizing::Flex { min, max },
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
    /// When true, the section shows a loading spinner instead of rows.
    pub loading: bool,
}

impl Section {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            rows: Vec::new(),
            loading: false,
        }
    }

    pub fn row(mut self, label: &str, value: &str) -> Self {
        self.rows.push((label.to_string(), value.to_string()));
        self
    }

    /// Mark this section as loading — the detail widget will render a spinner.
    pub fn with_loading(mut self) -> Self {
        self.loading = true;
        self
    }
}

/// Trait for views that render as a table.
pub trait TableView {
    type Item;

    fn columns() -> Vec<ColumnDef>;
    fn row(item: &Self::Item, tick: usize) -> Vec<Cell<'static>>;
    fn items(state: &AppState) -> Vec<&Self::Item>;
    fn context_keybindings() -> Vec<Keybinding>;
    fn empty_message() -> &'static str {
        "No items"
    }

    /// Return plain-text values for width measurement. Override if your columns
    /// use Flex sizing. Default returns empty strings (no measurement).
    fn row_texts(item: &Self::Item, tick: usize) -> Vec<String> {
        let cells = Self::row(item, tick);
        // Fallback: return header-width placeholders so Flex columns use min width.
        cells.iter().map(|_| String::new()).collect()
    }
}

/// Trait for views that render as a detail/info panel.
pub trait DetailView {
    fn title(state: &AppState) -> String;
    fn sections(state: &AppState) -> Vec<Section>;
    fn context_keybindings() -> Vec<Keybinding>;
}
