use crate::adapters::views::{DetailView, Keybinding, Section};
use crate::application::state::AppState;

pub struct PromptsView;

impl DetailView for PromptsView {
    fn title(state: &AppState) -> String {
        if let Some(agent_name) = state.nav.current().context.as_deref() {
            format!("Prompt: {}", agent_name)
        } else {
            "Prompts".to_string()
        }
    }

    fn sections(state: &AppState) -> Vec<Section> {
        let team_name = match state.current_team() {
            Some(n) => n,
            None => return vec![],
        };
        let agent_name = match state.nav.current().context.as_deref() {
            Some(n) => n,
            None => return vec![],
        };

        let team = match state.store.find_team(team_name) {
            Some(t) => t,
            None => return vec![],
        };

        let member = match team.members.iter().find(|m| m.name == agent_name) {
            Some(m) => m,
            None => return vec![Section::new("Error").row("", "Agent not found")],
        };

        let prompt = Section::new("System Prompt").row(
            "",
            if member.prompt.is_empty() {
                "No prompt configured"
            } else {
                &member.prompt
            },
        );

        vec![prompt]
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![Keybinding::new("e", "Edit prompt")]
    }
}
