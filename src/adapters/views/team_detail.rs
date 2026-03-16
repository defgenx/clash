use crate::adapters::format::{self, short_id};
use crate::adapters::views::{DetailView, Keybinding, Section};
use crate::application::state::AppState;

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
            let sid = short_id(session, 8);
            info = info.row("Lead Session", &format!("{} (press 's' to view)", sid));
        }

        let mut members_section = Section::new("Members");
        if team.members.is_empty() {
            members_section = members_section.row("", "No members");
        } else {
            for member in &team.members {
                let status_icon = if member.is_active { "●" } else { "○" };
                let type_label = if member.agent_type.is_empty() {
                    "agent"
                } else {
                    &member.agent_type
                };
                let model_str = if member.model.is_empty() {
                    String::new()
                } else {
                    format!(" {}", member.model)
                };
                let worktree_str = member
                    .cwd
                    .as_deref()
                    .and_then(format::detect_worktree)
                    .map(|w| format!(" ⊟ {}", w))
                    .unwrap_or_default();
                members_section = members_section.row(
                    &member.name,
                    &format!(
                        "{} [{}]{}{}",
                        status_icon, type_label, model_str, worktree_str
                    ),
                );
            }
            members_section = members_section.row("", "Press Enter to view agents");
        }

        let task_count = state.store.get_tasks(team_name).len();
        let mut tasks = Section::new("Tasks").row("Total", &task_count.to_string());
        if task_count > 0 {
            tasks = tasks.row("", "Press 't' to view");
        }

        vec![info, members_section, tasks]
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("Enter", "View agents"),
            Keybinding::new("a", "View agents"),
            Keybinding::new("s", "View lead session"),
            Keybinding::new("t", "View tasks"),
            Keybinding::new("d", "Delete team"),
            Keybinding::new("j/k", "Scroll"),
        ]
    }
}
