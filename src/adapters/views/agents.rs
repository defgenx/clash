use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::Member;
use crate::infrastructure::fs::backend::FsBackend;

pub struct AgentsTable;

impl TableView for AgentsTable {
    type Item = Member;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("NAME", 17),
            ColumnDef::new("TEAM", 13),
            ColumnDef::new("TYPE", 10),
            ColumnDef::new("MODEL", 10),
            ColumnDef::new("STATUS", 8),
            ColumnDef::new("MODE", 8),
            ColumnDef::new("CWD", 22),
            ColumnDef::new("WORKTREE", 12),
        ]
    }

    fn row(item: &Member) -> Vec<Cell<'static>> {
        let status = if item.is_active { "active" } else { "idle" };
        let status_color = if item.is_active {
            Color::Green
        } else {
            Color::DarkGray
        };

        let team_display = if item.team_name.is_empty() {
            "—".to_string()
        } else {
            item.team_name.clone()
        };

        let worktree = item.cwd.as_deref().and_then(FsBackend::detect_worktree);
        let worktree_display = match &worktree {
            Some(name) => format!("⊟ {}", name),
            None => "—".to_string(),
        };

        vec![
            Cell::from(item.name.clone()).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(team_display).style(Style::default().fg(Color::Yellow)),
            Cell::from(item.agent_type.clone()),
            Cell::from(item.model.clone()),
            Cell::from(status.to_string()).style(Style::default().fg(status_color)),
            Cell::from(item.mode.as_deref().unwrap_or("—").to_string()),
            Cell::from(item.cwd.as_deref().unwrap_or("—").to_string())
                .style(Style::default().fg(Color::DarkGray)),
            Cell::from(worktree_display).style(Style::default().fg(Color::Cyan)),
        ]
    }

    fn items(state: &AppState) -> Vec<&Member> {
        // If we have a team context, show only that team's members;
        // otherwise show all members across all teams.
        if let Some(team_name) = state.current_team() {
            if let Some(team) = state.store.find_team(team_name) {
                return team.members.iter().collect();
            }
        }
        state.store.all_members.iter().collect()
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("a", "Attach to agent"),
            Keybinding::new("m", "Send message"),
            Keybinding::new("Enter", "View agent"),
        ]
    }

    fn empty_message() -> &'static str {
        "No agents found."
    }
}
