use crate::adapters::format::{self, or_dash, short_id};
use crate::adapters::views::{DetailView, Keybinding, Section};
use crate::application::state::AppState;

pub struct SessionDetailView;

impl DetailView for SessionDetailView {
    fn title(state: &AppState) -> String {
        if let Some(session_id) = state.nav.current().context.as_deref() {
            let short = short_id(session_id, 8);
            if let Some(session) = state.store.find_session(session_id) {
                let icon = format::status_icon(session.status);
                format!("{} Session {}", icon, short)
            } else {
                format!("Session {}", short)
            }
        } else {
            "Session".to_string()
        }
    }

    fn sections(state: &AppState) -> Vec<Section> {
        let session_id = match state.current_session() {
            Some(id) => id.to_string(),
            None => return vec![],
        };

        let session = match state.store.find_session(&session_id) {
            Some(s) => s,
            None => return vec![Section::new("Error").row("", "Session not found")],
        };

        let info = Section::new("Info")
            .row("Session", &session.id)
            .row("Status", format::status_display(session.status))
            .row("Project", &session.project_path)
            .row("Branch", or_dash(&session.git_branch))
            .row("Messages", &session.message_count.to_string())
            .row("Modified", &session.last_modified)
            .row(
                "Summary",
                or_dash(if session.summary.is_empty() {
                    ""
                } else {
                    &session.summary
                }),
            );

        // Linked Teams section — teams where lead_session_id matches this session
        let linked_teams: Vec<_> = state
            .store
            .teams
            .iter()
            .filter(|t| t.lead_session_id.as_deref() == Some(&session_id))
            .collect();

        let mut linked_teams_section = Section::new("Linked Teams");
        if linked_teams.is_empty() {
            linked_teams_section = linked_teams_section.row("", "No linked teams");
        } else {
            for team in &linked_teams {
                let member_count = team.members.len();
                linked_teams_section =
                    linked_teams_section.row(&team.name, &format!("{} members", member_count));
            }
        }

        // Team Members section — members from linked teams
        let mut sections = vec![info, linked_teams_section];

        if !linked_teams.is_empty() {
            let mut members_section = Section::new("Team Members");
            for team in &linked_teams {
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
                    members_section = members_section.row(
                        &member.name,
                        &format!("{} [{}]{}", status_icon, type_label, model_str),
                    );
                }
            }
            sections.push(members_section);
        }

        // Subagents section
        let subagents_for_session: Vec<_> = state
            .store
            .subagents
            .iter()
            .filter(|sa| sa.parent_session_id == session_id)
            .collect();

        let agents_section = if !subagents_for_session.is_empty() {
            let mut section = Section::new("Subagents");
            for sa in &subagents_for_session {
                let icon = format::status_icon(sa.status);
                let type_label = if sa.agent_type.is_empty() {
                    "agent"
                } else {
                    &sa.agent_type
                };
                let sid = short_id(&sa.id, 12);
                let detail = if sa.summary.is_empty() {
                    &sa.last_modified
                } else {
                    &sa.summary
                };
                let desc = format!("{} [{}] {}", icon, type_label, detail);
                section = section.row(sid, &desc);
            }
            section
        } else if session.subagent_count > 0 {
            Section::new("Subagents")
                .row("Count", &session.subagent_count.to_string())
                .row("", "Press Enter to view")
        } else {
            Section::new("Subagents").row("", "No subagents")
        };

        // Conversation transcript (latest first)
        let mut conversation = Section::new("Conversation");
        if state.store.conversation.is_empty() {
            if state.store.conversation_loaded {
                conversation =
                    conversation.row("", "No messages (session file may have been removed)");
            } else {
                conversation = conversation.row("", "Loading...");
            }
        } else {
            for msg in state.store.conversation.iter().rev() {
                let role_label = if msg.role == "user" { "USER" } else { "CLAUDE" };
                conversation = conversation.row(role_label, &msg.text);
            }
        }

        sections.push(agents_section);
        sections.push(conversation);
        sections
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("Enter", "View subagents"),
            Keybinding::new("a", "Attach to session"),
            Keybinding::new("s", "View subagents"),
            Keybinding::new("m", "View team members"),
            Keybinding::new("t", "View linked team"),
            Keybinding::new("d", "Delete session"),
            Keybinding::new("j/k", "Scroll"),
        ]
    }
}
