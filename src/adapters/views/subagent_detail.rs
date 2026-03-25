use crate::adapters::format;
use crate::adapters::views::{DetailView, Keybinding, Section};
use crate::application::state::AppState;

pub struct SubagentDetailView;

impl DetailView for SubagentDetailView {
    fn title(state: &AppState) -> String {
        if let Some(agent_id) = state.nav.current().context.as_deref() {
            let short = format::short_id(agent_id, 12);
            if let Some(sa) = state.store.find_subagent(agent_id) {
                let type_label = if sa.agent_type.is_empty() {
                    "agent"
                } else {
                    &sa.agent_type
                };
                format!("◈ {} [{}]", short, type_label)
            } else {
                format!("Subagent {}", short)
            }
        } else {
            "Subagent".to_string()
        }
    }

    fn sections(state: &AppState) -> Vec<Section> {
        let agent_id = match state.nav.current().context.as_deref() {
            Some(id) => id,
            None => return vec![],
        };

        let subagent = match state.store.find_subagent(agent_id) {
            Some(s) => s,
            None => return vec![Section::new("Error").row("", "Subagent not found")],
        };

        let type_label = if subagent.agent_type.is_empty() {
            "unknown"
        } else {
            &subagent.agent_type
        };

        let status_str = format!("{}", subagent.status);

        let info = Section::new("Info")
            .row("Status", &status_str)
            .row("Agent ID", &subagent.id)
            .row("Type", type_label)
            .row("Parent", &subagent.parent_session_id)
            .row("Project", &subagent.file_path)
            .row("Modified", &subagent.last_modified);

        let summary = if subagent.summary.is_empty() {
            Section::new("Summary").row("", "No summary available")
        } else {
            Section::new("Summary").row("", &subagent.summary)
        };

        // Conversation transcript (latest first)
        let mut conversation = Section::new("Conversation");
        if state.store.conversation.is_empty() {
            conversation = conversation.row("", "Loading...");
        } else {
            for msg in state.store.conversation.iter().rev() {
                let role_label = if msg.role == "user" { "USER" } else { "CLAUDE" };
                conversation = conversation.row(role_label, &msg.text);
            }
        }

        vec![info, summary, conversation]
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("a", "Attach to parent session"),
            Keybinding::new("e", "Open in IDE"),
            Keybinding::new("Esc", "Go back"),
        ]
    }
}
