use crate::application::state::AppState;
use crate::adapters::views::{DetailView, Keybinding, Section};

pub struct TeamDetailView;

impl DetailView for TeamDetailView {
    fn title(state: &AppState) -> String {
        state
            .current_team()
            .map(|t| format!("Team: {}", t))
            .unwrap_or_else(|| "Team".to_string())
    }

    fn sections(state: &AppState) -> Vec<Section> {
        let team_name = match state.current_team() {
            Some(n) => n,
            None => return vec![],
        };

        let team = match state.store.find_team(team_name) {
            Some(t) => t,
            None => return vec![Section::new("Error").row("", "Team not found")],
        };

        let mut info = Section::new("Info")
            .row("Name", &team.name)
            .row("Description", &team.description);

        if let Some(ref created) = team.created_at {
            info = info.row("Created", &created.to_string());
        }
        if let Some(ref lead) = team.lead_agent_id {
            info = info.row("Lead Agent", lead);
        }
        if let Some(ref session) = team.lead_session_id {
            info = info.row("Lead Session", session);
        }

        let active = team.members.iter().filter(|m| m.is_active).count();
        let total = team.members.len();
        let members = Section::new("Members")
            .row("Total", &total.to_string())
            .row("Active", &active.to_string());

        let task_count = state.store.get_tasks(team_name).len();
        let tasks = Section::new("Tasks")
            .row("Total", &task_count.to_string());

        vec![info, members, tasks]
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("d", "Delete team"),
            Keybinding::new(":agents", "View agents"),
            Keybinding::new(":tasks", "View tasks"),
            Keybinding::new(":inbox", "View inbox"),
        ]
    }
}
