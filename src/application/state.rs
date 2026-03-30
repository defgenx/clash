//! Application state — the single source of truth for the UI.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::adapters::views::ViewKind;
use crate::application::actions::Action;
use crate::application::nav::NavigationStack;
use crate::application::store::DataStore;
use crate::domain::entities::{InboxMessage, Preset, Session, SessionSection};

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
    /// Section filter mode: "all", "active", "pending", "done", or "fail".
    #[serde(default)]
    pub section_filter: String,
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
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::All => "all",
        }
    }
}

/// Section filter — narrows the sessions view to a single section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectionFilter {
    #[default]
    All,
    Active,
    Done,
    Fail,
}

impl SectionFilter {
    /// Cycle to the next filter. Skips `Fail` when session filter is `:active`
    /// (since Fail sessions are already hidden in that mode).
    pub fn next(self, session_filter: SessionFilter) -> Self {
        match session_filter {
            SessionFilter::Active => match self {
                Self::All => Self::Active,
                Self::Active => Self::Done,
                Self::Done | Self::Fail => Self::All,
            },
            SessionFilter::All => match self {
                Self::All => Self::Active,
                Self::Active => Self::Done,
                Self::Done => Self::Fail,
                Self::Fail => Self::All,
            },
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Active => "Active",
            Self::Done => "Done",
            Self::Fail => "Fail",
        }
    }

    fn matches_section(self, section: SessionSection) -> bool {
        match self {
            Self::All => true,
            Self::Active => section == SessionSection::Active,
            Self::Done => section == SessionSection::Done,
            Self::Fail => section == SessionSection::Fail,
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
    SelectPreset { project_dir: String },
}

/// Pending session creation state — replaces scattered fields.
#[derive(Debug, Clone)]
pub struct PendingSession {
    pub cwd: String,
    pub name: Option<String>,
    pub worktree: bool,
    pub preset: Option<Preset>,
}

/// Diff viewer state — transient UI state, not domain data.
#[derive(Debug, Clone, Default)]
pub struct DiffState {
    /// Parsed diff lines (parsed once on load, widget maps to styled Lines per frame).
    pub lines: Vec<DiffLine>,
    /// true = diff has been loaded at least once.
    pub loaded: bool,
    /// true = git diff subprocess is in flight (prevents concurrent spawns).
    pub loading: bool,
    /// Which session this diff belongs to.
    pub session_id: Option<String>,
    /// Extracted file boundaries from the parsed diff lines.
    pub files: Vec<DiffFile>,
    /// Currently selected file index in the file list panel.
    pub selected_file: usize,
    /// Scroll offset within the selected file's diff (independent per file).
    pub file_scroll: u16,
}

/// A file entry extracted from parsed diff output — tracks line boundaries and change counts.
#[derive(Debug, Clone)]
pub struct DiffFile {
    /// File path, e.g. "src/main.rs".
    pub path: String,
    /// Index into `DiffState.lines` where this file's diff starts.
    pub start_line: usize,
    /// Exclusive end index into `DiffState.lines`.
    pub end_line: usize,
    /// Count of `DiffLineKind::Add` lines in this file.
    pub additions: usize,
    /// Count of `DiffLineKind::Remove` lines in this file.
    pub deletions: usize,
}

/// A single line from git diff output, classified by kind.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

/// Classification of a git diff line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLineKind {
    Add,
    Remove,
    Hunk,
    Meta,
    FilePath,
    Context,
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
    pub section_filter: SectionFilter,
    /// Currently attached daemon session ID.
    pub attached_session: Option<String>,
    /// Sessions with expanded subagent rows in the Sessions table.
    pub expanded_sessions: HashSet<String>,
    /// Default working directory for new sessions (where clash was started).
    pub default_cwd: String,
    /// Pending session creation state (replaces old cwd/worktree fields).
    pub pending_session: Option<PendingSession>,
    /// Guided tour state: Some(step_index) when active, None when inactive.
    pub tour_step: Option<usize>,
    /// vt100 screen for inline terminal rendering when attached to a session.
    pub terminal_screen: Option<vt100::Parser>,
    /// Sessions currently open in external panes/tabs/windows.
    /// Tracked in-memory only — cleared on restart.
    pub externally_opened: HashSet<String>,
    /// Diff viewer state (transient — not persisted).
    pub diff: DiffState,
    /// Debug mode flag — enables verbose logging.
    pub debug_mode: bool,
    /// Self-update progress (shown as an overlay when active).
    pub update_progress: Option<UpdatePhase>,
    /// Toast to show after the current spinner clears.
    pub pending_toast: Option<String>,
    /// Graceful shutdown: Some(start_tick) when stashing sessions before quit.
    pub shutting_down: Option<usize>,
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
            section_filter: SectionFilter::default(),
            attached_session: None,
            expanded_sessions: HashSet::new(),
            default_cwd: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            pending_session: None,
            tour_step: None,
            terminal_screen: None,
            externally_opened: HashSet::new(),
            diff: DiffState::default(),
            debug_mode: false,
            update_progress: None,
            pending_toast: None,
            shutting_down: None,
        }
    }

    pub fn current_view(&self) -> ViewKind {
        self.nav.current().view
    }

    /// Returns `true` when the UI has active animations that need periodic redraws
    /// (spinners, animated status icons for running sessions, update overlay, etc.).
    pub fn needs_animation(&self) -> bool {
        use crate::domain::entities::SessionStatus;
        self.spinner.is_some()
            || self.toast.is_some()
            || self.update_progress.is_some()
            || self.shutting_down.is_some()
            || self.store.sessions.iter().any(|s| {
                matches!(
                    s.status,
                    SessionStatus::Prompting
                        | SessionStatus::Thinking
                        | SessionStatus::Running
                        | SessionStatus::Starting
                )
            })
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
            section_filter: self.section_filter.label().to_string(),
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

        // Restore section filter
        self.section_filter = match snapshot.section_filter.as_str() {
            "active" | "Active" => SectionFilter::Active,
            "done" | "Done" => SectionFilter::Done,
            "fail" | "Fail" => SectionFilter::Fail,
            _ => SectionFilter::All,
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

    /// Get filtered sessions based on the current session filter, section filter, and text filter.
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

        // Apply section filter
        let section_filtered: Vec<&Session> = if self.section_filter == SectionFilter::All {
            status_filtered
        } else {
            status_filtered
                .into_iter()
                .filter(|s| self.section_filter.matches_section(s.status.section()))
                .collect()
        };

        if self.filter.is_empty() {
            return section_filtered;
        }

        section_filtered
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

    // ── Section ordering tests ──────────────────────────────────

    use crate::domain::entities::SessionStatus;

    /// Helper to create a session with a given name and status.
    fn make_session(id: &str, name: Option<&str>, status: SessionStatus) -> Session {
        Session {
            id: id.to_string(),
            name: name.map(|n| n.to_string()),
            status,
            is_running: !matches!(status, SessionStatus::Stashed),
            ..Default::default()
        }
    }

    /// Sort sessions using the same logic as DataStore::sort_sessions().
    fn sort_sessions_for_test(state: &mut AppState) {
        state.store.sort_sessions();
    }

    #[test]
    fn test_ordering_three_sections() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::All;
        state.store.sessions = vec![
            make_session("s1", Some("alpha"), SessionStatus::Waiting),
            make_session("s2", Some("beta"), SessionStatus::Running),
            make_session("s3", Some("gamma"), SessionStatus::Stashed),
            make_session("s4", Some("delta"), SessionStatus::Thinking),
            make_session("s5", Some("echo"), SessionStatus::Prompting),
            make_session("s6", Some("foxtrot"), SessionStatus::Errored),
        ];
        sort_sessions_for_test(&mut state);
        let filtered = state.filtered_sessions();
        // Active (Running, Thinking, Prompting, Waiting), Done (Idle), Fail (Errored)
        assert_eq!(filtered[0].name.as_deref(), Some("alpha")); // Active
        assert_eq!(filtered[1].name.as_deref(), Some("beta")); // Active
        assert_eq!(filtered[2].name.as_deref(), Some("delta")); // Active
        assert_eq!(filtered[3].name.as_deref(), Some("echo")); // Active
        assert_eq!(filtered[4].name.as_deref(), Some("gamma")); // Done
        assert_eq!(filtered[5].name.as_deref(), Some("foxtrot")); // Fail
    }

    #[test]
    fn test_ordering_alphabetical_within_section() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::All;
        state.store.sessions = vec![
            make_session("s1", Some("zebra"), SessionStatus::Running),
            make_session("s2", Some("apple"), SessionStatus::Thinking),
            make_session("s3", Some("mango"), SessionStatus::Running),
        ];
        sort_sessions_for_test(&mut state);
        let filtered = state.filtered_sessions();
        // All Busy, sorted alphabetically: apple, mango, zebra
        assert_eq!(filtered[0].name.as_deref(), Some("apple"));
        assert_eq!(filtered[1].name.as_deref(), Some("mango"));
        assert_eq!(filtered[2].name.as_deref(), Some("zebra"));
    }

    #[test]
    fn test_ordering_unnamed_sessions_by_id() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::All;
        state.store.sessions = vec![
            make_session("zzz-unnamed", None, SessionStatus::Running),
            make_session("aaa-unnamed", None, SessionStatus::Running),
            make_session("mmm-unnamed", None, SessionStatus::Running),
        ];
        sort_sessions_for_test(&mut state);
        let filtered = state.filtered_sessions();
        // Unnamed sessions sort by ID (alphabetically)
        assert_eq!(filtered[0].id, "aaa-unnamed");
        assert_eq!(filtered[1].id, "mmm-unnamed");
        assert_eq!(filtered[2].id, "zzz-unnamed");
    }

    #[test]
    fn test_ordering_stable_across_status_change() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::All;
        // Both running — sorted by name
        state.store.sessions = vec![
            make_session("s1", Some("alpha"), SessionStatus::Running),
            make_session("s2", Some("beta"), SessionStatus::Running),
        ];
        sort_sessions_for_test(&mut state);
        assert_eq!(state.filtered_sessions()[0].name.as_deref(), Some("alpha"));
        assert_eq!(state.filtered_sessions()[1].name.as_deref(), Some("beta"));

        // alpha transitions to Thinking — still in Active section, same order
        state.store.sessions[0].status = SessionStatus::Thinking;
        sort_sessions_for_test(&mut state);
        assert_eq!(state.filtered_sessions()[0].name.as_deref(), Some("alpha"));
        assert_eq!(state.filtered_sessions()[1].name.as_deref(), Some("beta"));
    }

    // ── SectionFilter tests ──────────────────────────────────

    #[test]
    fn test_section_filter_active_only() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::All;
        state.section_filter = SectionFilter::Active;
        state.store.sessions = vec![
            make_session("s1", Some("alpha"), SessionStatus::Running),
            make_session("s2", Some("beta"), SessionStatus::Waiting),
            make_session("s3", Some("gamma"), SessionStatus::Thinking),
            make_session("s4", Some("delta"), SessionStatus::Errored),
        ];
        let filtered = state.filtered_sessions();
        // Running, Waiting, Thinking are all Active now
        assert_eq!(filtered.len(), 3);
        assert_eq!(filtered[0].name.as_deref(), Some("alpha"));
        assert_eq!(filtered[1].name.as_deref(), Some("beta"));
        assert_eq!(filtered[2].name.as_deref(), Some("gamma"));
    }

    #[test]
    fn test_section_filter_done_only() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::All;
        state.section_filter = SectionFilter::Done;
        state.store.sessions = vec![
            make_session("s1", Some("alpha"), SessionStatus::Running),
            make_session("s2", Some("beta"), SessionStatus::Waiting),
            make_session("s3", Some("gamma"), SessionStatus::Stashed),
            make_session("s4", Some("delta"), SessionStatus::Prompting),
        ];
        let filtered = state.filtered_sessions();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name.as_deref(), Some("gamma"));
    }

    #[test]
    fn test_section_filter_fail_only() {
        let mut state = AppState::new();
        state.session_filter = SessionFilter::All;
        state.section_filter = SectionFilter::Fail;
        state.store.sessions = vec![
            make_session("s1", Some("alpha"), SessionStatus::Running),
            make_session("s2", Some("beta"), SessionStatus::Errored),
            make_session("s3", Some("gamma"), SessionStatus::Stashed),
        ];
        let filtered = state.filtered_sessions();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name.as_deref(), Some("beta"));
    }

    #[test]
    fn test_section_filter_cycle_skips_fail_in_active_mode() {
        // In :active mode, cycling should skip Fail
        let f = SectionFilter::All;
        let active_mode = SessionFilter::Active;
        let f = f.next(active_mode); // All -> Active
        assert_eq!(f, SectionFilter::Active);
        let f = f.next(active_mode); // Active -> Done
        assert_eq!(f, SectionFilter::Done);
        let f = f.next(active_mode); // Done -> All (skips Fail)
        assert_eq!(f, SectionFilter::All);
    }

    #[test]
    fn test_section_filter_cycle_includes_fail_in_all_mode() {
        let f = SectionFilter::All;
        let all_mode = SessionFilter::All;
        let f = f.next(all_mode); // All -> Active
        assert_eq!(f, SectionFilter::Active);
        let f = f.next(all_mode); // Active -> Done
        assert_eq!(f, SectionFilter::Done);
        let f = f.next(all_mode); // Done -> Fail
        assert_eq!(f, SectionFilter::Fail);
        let f = f.next(all_mode); // Fail -> All
        assert_eq!(f, SectionFilter::All);
    }

    #[test]
    fn test_section_filter_snapshot_roundtrip() {
        let mut state = AppState::new();
        state.section_filter = SectionFilter::Done;
        let snapshot = state.snapshot();
        assert_eq!(snapshot.section_filter, "Done");

        let mut state2 = AppState::new();
        state2.restore(snapshot);
        assert_eq!(state2.section_filter, SectionFilter::Done);
    }
}
