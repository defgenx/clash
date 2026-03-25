use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::format as fmt;
use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::Member;
use crate::infrastructure::tui::theme;

fn agent_texts(item: &Member) -> Vec<String> {
    let status = if item.is_active { "active" } else { "idle" };
    let team_display = if item.team_name.is_empty() {
        "—".to_string()
    } else {
        item.team_name.clone()
    };
    let worktree = item.cwd.as_deref().and_then(fmt::detect_worktree);
    let worktree_display = match &worktree {
        Some(name) => format!("⊟ {}", name),
        None => "—".to_string(),
    };
    vec![
        item.name.clone(),
        team_display,
        item.agent_type.clone(),
        item.model.clone(),
        status.to_string(),
        item.mode.as_deref().unwrap_or("—").to_string(),
        item.cwd.as_deref().unwrap_or("—").to_string(),
        worktree_display,
    ]
}

pub struct AgentsTable;

impl TableView for AgentsTable {
    type Item = Member;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::flex("NAME", 4, 25),
            ColumnDef::flex("TEAM", 4, 20),
            ColumnDef::flex("TYPE", 4, 12),
            ColumnDef::flex("MODEL", 5, 15),
            ColumnDef::flex("STATUS", 4, 10),
            ColumnDef::flex("MODE", 4, 10),
            ColumnDef::new("CWD", 30),
            ColumnDef::flex("WORKTREE", 4, 20),
        ]
    }

    fn row_texts(item: &Member, _tick: usize) -> Vec<String> {
        agent_texts(item)
    }

    fn row(item: &Member, _tick: usize) -> Vec<Cell<'static>> {
        let texts = agent_texts(item);
        let status_color = if item.is_active {
            theme::STATUS_RUNNING
        } else {
            theme::STATUS_IDLE
        };

        vec![
            Cell::from(texts[0].clone()).style(theme::name_style()),
            Cell::from(texts[1].clone()).style(Style::default().fg(theme::BRANCH_COLOR)),
            Cell::from(texts[2].clone()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(texts[3].clone()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(texts[4].clone()).style(
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(texts[5].clone()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(texts[6].clone()).style(Style::default().fg(theme::PATH_COLOR)),
            Cell::from(texts[7].clone()).style(Style::default().fg(theme::ACCENT)),
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
