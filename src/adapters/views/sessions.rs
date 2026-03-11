use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::application::actions::{Action, NavAction};
use crate::application::state::AppState;
use crate::domain::entities::{Session, SessionStatus};
use crate::adapters::views::{ColumnDef, Keybinding, TableView, ViewKind};
use crate::infrastructure::tui::theme;

pub struct SessionsTable;

impl SessionsTable {
    pub fn has_items(state: &AppState) -> bool {
        !<Self as TableView>::items(state).is_empty()
    }
}

impl TableView for SessionsTable {
    type Item = Session;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("STATUS", 12),
            ColumnDef::new("SESSION", 12),
            ColumnDef::new("PROJECT", 20),
            ColumnDef::new("SUMMARY", 30),
            ColumnDef::new("MSGS", 5),
            ColumnDef::new("AGENTS", 6),
            ColumnDef::new("BRANCH", 10),
            ColumnDef::new("MODIFIED", 7),
        ]
    }

    fn row(item: &Session) -> Vec<Cell<'static>> {
        let (status, status_style) = match item.status {
            SessionStatus::Waiting => (
                "◉ WAITING".to_string(),
                Style::default().fg(theme::STATUS_WAITING).add_modifier(Modifier::BOLD),
            ),
            SessionStatus::Thinking => (
                "◎ THINKING".to_string(),
                Style::default().fg(theme::STATUS_THINKING).add_modifier(Modifier::BOLD),
            ),
            SessionStatus::Running => (
                "● RUNNING".to_string(),
                Style::default().fg(theme::STATUS_RUNNING).add_modifier(Modifier::BOLD),
            ),
            SessionStatus::Starting => (
                "⦿ STARTING".to_string(),
                Style::default().fg(theme::STATUS_STARTING).add_modifier(Modifier::BOLD),
            ),
            SessionStatus::Prompting => (
                "◉ PROMPTING".to_string(),
                Style::default().fg(theme::STATUS_PROMPTING).add_modifier(Modifier::BOLD),
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

        let msgs = format!("{}", item.message_count);

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
            Cell::from(msgs).style(Style::default().fg(theme::MUTED)),
            Cell::from(agents).style(Style::default().fg(theme::ACCENT)),
            Cell::from(branch).style(Style::default().fg(theme::STATUS_WAITING)),
            Cell::from(item.last_modified.clone())
                .style(Style::default().fg(theme::MUTED)),
        ]
    }

    fn items(state: &AppState) -> Vec<&Session> {
        if state.show_all_sessions {
            state.store.sessions.iter().collect()
        } else {
            state.store.sessions.iter().filter(|s| s.is_running).collect()
        }
    }

    fn on_select(item: &Session) -> Action {
        Action::Nav(NavAction::DrillIn {
            view: ViewKind::SessionDetail,
            context: item.id.clone(),
        })
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("Enter", "Attach to session"),
            Keybinding::new("i", "Inspect session details"),
            Keybinding::new("A", "Toggle all / active sessions"),
            Keybinding::new("c", "New Claude session"),
            Keybinding::new("d", "Delete session"),
            Keybinding::new(":teams", "View teams"),
        ]
    }

    fn empty_message() -> &'static str {
        "No active sessions. Press A to show all, or c to start a new session."
    }
}
