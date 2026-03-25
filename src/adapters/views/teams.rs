use ratatui::style::Style;
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::Team;
use crate::infrastructure::tui::theme;

pub struct TeamsTable;

fn team_texts(item: &Team) -> Vec<String> {
    let active_count = item.members.iter().filter(|m| m.is_active).count();
    let total = item.members.len();
    let members_str = if total > 0 {
        format!("{}/{}", active_count, total)
    } else {
        "0".to_string()
    };
    let lead = item.lead_agent_id.as_deref().unwrap_or("—").to_string();
    vec![
        item.name.clone(),
        members_str,
        lead,
        item.description.clone(),
    ]
}

impl TableView for TeamsTable {
    type Item = Team;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::flex("NAME", 4, 30),
            ColumnDef::flex("MEMBERS", 4, 10),
            ColumnDef::flex("LEAD", 4, 25),
            ColumnDef::new("DESCRIPTION", 50),
        ]
    }

    fn row_texts(item: &Team, _tick: usize) -> Vec<String> {
        team_texts(item)
    }

    fn row(item: &Team, _tick: usize) -> Vec<Cell<'static>> {
        let texts = team_texts(item);

        vec![
            Cell::from(texts[0].clone()).style(theme::name_style()),
            Cell::from(texts[1].clone()).style(Style::default().fg(theme::COUNT_COLOR)),
            Cell::from(texts[2].clone()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(texts[3].clone()).style(Style::default().fg(theme::DESCRIPTION_COLOR)),
        ]
    }

    fn items(state: &AppState) -> Vec<&Team> {
        state.store.teams.iter().collect()
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("c", "Create team"),
            Keybinding::new("d", "Delete team"),
            Keybinding::new("Enter", "View team"),
        ]
    }

    fn empty_message() -> &'static str {
        "No teams found. Press 'c' to create one, or create a team with Claude Code."
    }
}
