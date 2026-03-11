use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView, ViewKind};
use crate::application::actions::{Action, NavAction};
use crate::application::state::{AppState, SessionTreeRow};
use crate::domain::entities::SessionStatus;
use crate::infrastructure::tui::theme;

pub struct SessionsTable;

impl SessionsTable {
    pub fn has_items(state: &AppState) -> bool {
        !<Self as TableView>::items(state).is_empty()
    }
}

impl TableView for SessionsTable {
    type Item = SessionTreeRow;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("STATUS", 15),
            ColumnDef::new("SESSION", 12),
            ColumnDef::new("PROJECT", 20),
            ColumnDef::new("SUMMARY", 35),
            ColumnDef::new("AGENTS", 8),
            ColumnDef::new("BRANCH", 10),
        ]
    }

    fn row(item: &SessionTreeRow) -> Vec<Cell<'static>> {
        match item {
            SessionTreeRow::Session(session) => session_row(session),
            SessionTreeRow::Subagent {
                subagent, is_last, ..
            } => subagent_row(subagent, *is_last),
        }
    }

    fn items(state: &AppState) -> Vec<&SessionTreeRow> {
        state.filtered_session_tree()
    }

    fn on_select(item: &SessionTreeRow) -> Action {
        match item {
            SessionTreeRow::Session(session) => Action::Nav(NavAction::DrillIn {
                view: ViewKind::SessionDetail,
                context: session.id.clone(),
            }),
            SessionTreeRow::Subagent { subagent, .. } => Action::Nav(NavAction::DrillIn {
                view: ViewKind::SubagentDetail,
                context: subagent.id.clone(),
            }),
        }
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("Enter", "Attach to session"),
            Keybinding::new("i", "Inspect session details"),
            Keybinding::new("a", "Attach to session"),
            Keybinding::new("c/n", "New Claude session"),
            Keybinding::new("A", "Toggle filter: active / all"),
            Keybinding::new("d", "Close and delete session"),
            Keybinding::new("D", "Close and delete ALL sessions"),
            Keybinding::new(":active", "Show active sessions"),
            Keybinding::new(":all", "Show all sessions"),
        ]
    }

    fn empty_message() -> &'static str {
        "No sessions. Press A to cycle filter, or c to start a new session."
    }
}

fn session_row(item: &crate::domain::entities::Session) -> Vec<Cell<'static>> {
    let (status, status_style) = match item.status {
        SessionStatus::Waiting => (
            "◉ WAITING".to_string(),
            Style::default()
                .fg(theme::STATUS_WAITING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Thinking => (
            "◎ THINKING".to_string(),
            Style::default()
                .fg(theme::STATUS_THINKING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Running => (
            "● RUNNING".to_string(),
            Style::default()
                .fg(theme::STATUS_RUNNING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Starting => (
            "⦿ STARTING".to_string(),
            Style::default()
                .fg(theme::STATUS_STARTING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Prompting => (
            "◉ PROMPTING".to_string(),
            Style::default()
                .fg(theme::STATUS_PROMPTING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Idle => (
            "○ IDLE".to_string(),
            Style::default().fg(theme::STATUS_IDLE),
        ),
    };

    let short_id = if item.id.len() > 8 {
        item.id[..8].to_string()
    } else {
        item.id.clone()
    };

    let display_name = if !item.summary.is_empty() {
        item.summary.clone()
    } else {
        "—".to_string()
    };

    let project_display = item
        .project_path
        .rsplit('/')
        .next()
        .unwrap_or(&item.project_path)
        .to_string();

    let agents = if item.subagent_count > 0 {
        format!("{}", item.subagent_count)
    } else {
        "—".to_string()
    };

    let branch = if item.git_branch.is_empty() {
        "—".to_string()
    } else {
        item.git_branch.clone()
    };

    vec![
        Cell::from(status).style(status_style),
        Cell::from(short_id).style(
            Style::default()
                .fg(theme::CLAUDE_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(project_display).style(Style::default().fg(theme::TEXT)),
        Cell::from(display_name).style(Style::default().fg(theme::TEXT_DIM)),
        Cell::from(agents).style(Style::default().fg(theme::ACCENT)),
        Cell::from(branch).style(Style::default().fg(theme::STATUS_WAITING)),
    ]
}

fn subagent_row(subagent: &crate::domain::entities::Subagent, is_last: bool) -> Vec<Cell<'static>> {
    let tree_char = if is_last { "└─" } else { "├─" };

    let (status_icon, status_label, status_style) = match subagent.status {
        SessionStatus::Waiting => (
            "◉",
            "WAITING",
            Style::default()
                .fg(theme::STATUS_WAITING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Thinking => (
            "◎",
            "THINKING",
            Style::default()
                .fg(theme::STATUS_THINKING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Running => (
            "●",
            "RUNNING",
            Style::default()
                .fg(theme::STATUS_RUNNING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Starting => (
            "⦿",
            "STARTING",
            Style::default()
                .fg(theme::STATUS_STARTING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Prompting => (
            "◉",
            "PROMPTING",
            Style::default()
                .fg(theme::STATUS_PROMPTING)
                .add_modifier(Modifier::BOLD),
        ),
        SessionStatus::Idle => (
            "✓",
            "DONE",
            Style::default()
                .fg(theme::STATUS_RUNNING) // green = done
                .add_modifier(Modifier::BOLD),
        ),
    };

    let agent_style = Style::default().fg(theme::ACCENT);
    let dim = Style::default().fg(theme::TEXT_DIM);

    let short_id = if subagent.id.len() > 8 {
        subagent.id[..8].to_string()
    } else {
        subagent.id.clone()
    };

    let agent_type = if subagent.agent_type.is_empty() {
        "agent".to_string()
    } else {
        subagent.agent_type.clone()
    };

    let summary = if subagent.summary.is_empty() {
        "—".to_string()
    } else {
        subagent.summary.clone()
    };

    vec![
        Cell::from(format!("  {} {} {}", tree_char, status_icon, status_label)).style(status_style),
        Cell::from(short_id).style(agent_style),
        Cell::from(agent_type).style(dim),
        Cell::from(summary).style(dim),
        Cell::from("").style(dim),
        Cell::from("").style(dim),
    ]
}
