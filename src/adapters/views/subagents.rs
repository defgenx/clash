use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView, ViewKind};
use crate::application::actions::{Action, NavAction};
use crate::application::state::AppState;
use crate::domain::entities::{SessionStatus, Subagent};
use crate::infrastructure::tui::theme;

pub struct SubagentsTable;

impl TableView for SubagentsTable {
    type Item = Subagent;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("STATUS", 15),
            ColumnDef::new("AGENT", 20),
            ColumnDef::new("TYPE", 12),
            ColumnDef::new("SUMMARY", 38),
            ColumnDef::new("MODIFIED", 15),
        ]
    }

    fn row(item: &Subagent) -> Vec<Cell<'static>> {
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
                "✓ DONE".to_string(),
                Style::default()
                    .fg(theme::STATUS_RUNNING) // green = done
                    .add_modifier(Modifier::BOLD),
            ),
        };

        let display_id = if item.id.len() > 20 {
            format!("{}...", &item.id[..17])
        } else {
            item.id.clone()
        };

        let agent_type = if item.agent_type.is_empty() {
            "agent".to_string()
        } else {
            item.agent_type.clone()
        };

        let summary = if item.summary.is_empty() {
            "—".to_string()
        } else {
            item.summary.clone()
        };

        vec![
            Cell::from(status).style(status_style),
            Cell::from(display_id).style(
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(agent_type).style(Style::default().fg(theme::STATUS_WAITING)),
            Cell::from(summary).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(item.last_modified.clone()).style(Style::default().fg(theme::MUTED)),
        ]
    }

    fn items(state: &AppState) -> Vec<&Subagent> {
        state.store.subagents.iter().collect()
    }

    fn on_select(item: &Subagent) -> Action {
        Action::Nav(NavAction::DrillIn {
            view: ViewKind::SubagentDetail,
            context: item.id.clone(),
        })
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("Enter", "View subagent details"),
            Keybinding::new("a", "Attach to parent session"),
        ]
    }

    fn empty_message() -> &'static str {
        "No subagents found for this session."
    }
}
