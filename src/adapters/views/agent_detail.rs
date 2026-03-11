use crate::adapters::views::{DetailView, Keybinding, Section};
use crate::application::state::AppState;

pub struct AgentDetailView;

impl DetailView for AgentDetailView {
    fn title(state: &AppState) -> String {
        if let Some(agent_name) = state.nav.current().context.as_deref() {
            format!("Agent: {}", agent_name)
        } else {
            "Agent".to_string()
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

        let status = if member.is_active { "active" } else { "idle" };

        let info = Section::new("Info")
            .row("Name", &member.name)
            .row("Agent ID", &member.agent_id)
            .row("Type", &member.agent_type)
            .row("Model", &member.model)
            .row("Status", status)
            .row("Mode", member.mode.as_deref().unwrap_or("—"))
            .row("Color", &member.color);

        let runtime = Section::new("Runtime")
            .row("CWD", member.cwd.as_deref().unwrap_or("—"))
            .row("Tmux Pane", member.tmux_pane_id.as_deref().unwrap_or("—"))
            .row("Backend", member.backend_type.as_deref().unwrap_or("—"));

        let mut prompt_section = Section::new("Prompt");
        if member.prompt.is_empty() {
            prompt_section = prompt_section.row("", "No prompt configured");
        } else {
            prompt_section = prompt_section.row("", &member.prompt);
        }

        vec![info, runtime, prompt_section]
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("a", "Attach session"),
            Keybinding::new("m", "Send message"),
            Keybinding::new(":inbox", "View inbox"),
            Keybinding::new(":prompts", "View prompt"),
        ]
    }
}
