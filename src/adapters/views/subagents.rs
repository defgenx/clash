use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView, ViewKind};
use crate::application::actions::{Action, NavAction};
use crate::application::state::AppState;
use crate::domain::entities::Subagent;
use crate::infrastructure::tui::theme;

pub struct SubagentsTable;

impl TableView for SubagentsTable {
    type Item = Subagent;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("AGENT", 25),
            ColumnDef::new("TYPE", 15),
            ColumnDef::new("MODIFIED", 15),
            ColumnDef::new("SUMMARY", 45),
        ]
    }

    fn row(item: &Subagent) -> Vec<Cell<'static>> {
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
            Cell::from(display_id).style(
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(agent_type).style(Style::default().fg(theme::STATUS_WAITING)),
            Cell::from(item.last_modified.clone()).style(Style::default().fg(theme::MUTED)),
            Cell::from(summary).style(Style::default().fg(theme::TEXT_DIM)),
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
