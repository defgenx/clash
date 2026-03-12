use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::format::{self, or_dash, short_id, truncate};
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
            ColumnDef::new("STATUS", 15),
            ColumnDef::new("AGENT", 18),
            ColumnDef::new("SESSION", 10),
            ColumnDef::new("TYPE", 10),
            ColumnDef::new("SUMMARY", 32),
            ColumnDef::new("MODIFIED", 15),
        ]
    }

    fn row(item: &Subagent) -> Vec<Cell<'static>> {
        let (status, status_style) = format::status_cell(item.status);
        let display_id = truncate(&item.id, 18, "…");
        let short_session = short_id(&item.parent_session_id, 8).to_string();
        let agent_type = or_dash(if item.agent_type.is_empty() {
            ""
        } else {
            &item.agent_type
        });
        let agent_type = if agent_type == "—" {
            "agent"
        } else {
            agent_type
        };
        let summary = or_dash(if item.summary.is_empty() {
            ""
        } else {
            &item.summary
        });

        vec![
            Cell::from(status).style(status_style),
            Cell::from(display_id).style(
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(short_session).style(
                Style::default()
                    .fg(theme::CLAUDE_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(agent_type.to_string()).style(Style::default().fg(theme::STATUS_WAITING)),
            Cell::from(summary.to_string()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(item.last_modified.clone()).style(Style::default().fg(theme::MUTED)),
        ]
    }

    fn items(state: &AppState) -> Vec<&Subagent> {
        // If we have a session context, show only that session's subagents;
        // otherwise show all subagents across all sessions.
        if let Some(session_id) = state.current_session() {
            state
                .store
                .subagents
                .iter()
                .filter(|s| s.parent_session_id == session_id)
                .collect()
        } else {
            state.store.all_subagents.iter().collect()
        }
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
        "No subagents found."
    }
}
