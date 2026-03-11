use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Cell;

use crate::application::actions::{Action, NavAction};
use crate::application::state::AppState;
use crate::domain::entities::Team;
use crate::adapters::views::{ColumnDef, Keybinding, TableView, ViewKind};

pub struct TeamsTable;

impl TableView for TeamsTable {
    type Item = Team;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("NAME", 25),
            ColumnDef::new("MEMBERS", 10),
            ColumnDef::new("LEAD", 20),
            ColumnDef::new("DESCRIPTION", 45),
        ]
    }

    fn row(item: &Team) -> Vec<Cell<'static>> {
        let active_count = item.members.iter().filter(|m| m.is_active).count();
        let total = item.members.len();
        let members_str = if total > 0 {
            format!("{}/{}", active_count, total)
        } else {
            "0".to_string()
        };

        let lead = item
            .lead_agent_id
            .as_deref()
            .unwrap_or("—")
            .to_string();

        vec![
            Cell::from(item.name.clone()).style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Cell::from(members_str),
            Cell::from(lead),
            Cell::from(item.description.clone()).style(Style::default().fg(Color::DarkGray)),
        ]
    }

    fn items(state: &AppState) -> Vec<&Team> {
        state.store.teams.iter().collect()
    }

    fn on_select(item: &Team) -> Action {
        Action::Nav(NavAction::DrillIn {
            view: ViewKind::TeamDetail,
            context: item.name.clone(),
        })
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
