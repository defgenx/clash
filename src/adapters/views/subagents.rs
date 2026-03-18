use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::format::{self, or_dash, short_id, truncate};
use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::Subagent;
use crate::infrastructure::tui::theme;

pub struct SubagentsTable;

fn subagent_texts(item: &Subagent, tick: usize) -> Vec<String> {
    let (status, _) = format::status_cell(item.status, tick);
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
        status,
        display_id,
        short_session,
        agent_type.to_string(),
        summary.to_string(),
        item.last_modified.clone(),
    ]
}

impl TableView for SubagentsTable {
    type Item = Subagent;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::flex("STATUS", 6, 14),
            ColumnDef::flex("AGENT", 5, 20),
            ColumnDef::flex("SESSION", 5, 12),
            ColumnDef::flex("TYPE", 4, 10),
            ColumnDef::new("SUMMARY", 40),
            ColumnDef::flex("MODIFIED", 8, 18),
        ]
    }

    fn row_texts(item: &Subagent, tick: usize) -> Vec<String> {
        subagent_texts(item, tick)
    }

    fn row(item: &Subagent, tick: usize) -> Vec<Cell<'static>> {
        let texts = subagent_texts(item, tick);
        let (_, status_style) = format::status_cell(item.status, tick);

        vec![
            Cell::from(texts[0].clone()).style(status_style),
            Cell::from(texts[1].clone()).style(
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(texts[2].clone()).style(
                Style::default()
                    .fg(theme::CLAUDE_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(texts[3].clone()).style(Style::default().fg(theme::STATUS_WAITING)),
            Cell::from(texts[4].clone()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(texts[5].clone()).style(Style::default().fg(theme::MUTED)),
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
