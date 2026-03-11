use crate::adapters::views::{DetailView, Keybinding, Section};
use crate::application::state::AppState;
use crate::domain::entities::SessionStatus;

pub struct SessionDetailView;

impl DetailView for SessionDetailView {
    fn title(state: &AppState) -> String {
        if let Some(session_id) = state.nav.current().context.as_deref() {
            let short = if session_id.len() > 8 {
                &session_id[..8]
            } else {
                session_id
            };
            if let Some(session) = state.store.find_session(session_id) {
                let status_icon = match session.status {
                    SessionStatus::Waiting => "◉",
                    SessionStatus::Thinking => "◎",
                    SessionStatus::Running => "●",
                    SessionStatus::Starting => "⦿",
                    SessionStatus::Prompting => "◉",
                    SessionStatus::Idle => "○",
                };
                format!("{} Session {}", status_icon, short)
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

        let status_display = match session.status {
            SessionStatus::Waiting => "◉ WAITING FOR INPUT",
            SessionStatus::Thinking => "◎ THINKING",
            SessionStatus::Running => "● RUNNING",
            SessionStatus::Starting => "⦿ STARTING",
            SessionStatus::Prompting => "◉ PROMPTING (approval needed)",
            SessionStatus::Idle => "○ IDLE",
        };

        let info = Section::new("Info")
            .row("Session", &session.id)
            .row("Status", status_display)
            .row("Project", &session.project_path)
            .row(
                "Branch",
                if session.git_branch.is_empty() {
                    "—"
                } else {
                    &session.git_branch
                },
            )
            .row("Messages", &session.message_count.to_string())
            .row("Modified", &session.last_modified)
            .row(
                "Summary",
                if session.summary.is_empty() {
                    "—"
                } else {
                    &session.summary
                },
            );

        // Subagents section
        let subagents_for_session: Vec<_> = state
            .store
            .subagents
            .iter()
            .filter(|sa| sa.parent_session_id == session_id)
            .collect();

        let agents_section = if !subagents_for_session.is_empty() {
            let mut section = Section::new("Team");
            for sa in &subagents_for_session {
                let status_icon = match sa.status {
                    SessionStatus::Waiting => "◉",
                    SessionStatus::Thinking => "◎",
                    SessionStatus::Running => "●",
                    SessionStatus::Starting => "⦿",
                    SessionStatus::Prompting => "◉",
                    SessionStatus::Idle => "✓",
                };
                let type_label = if sa.agent_type.is_empty() {
                    "agent".to_string()
                } else {
                    sa.agent_type.clone()
                };
                let short_id = if sa.id.len() > 12 {
                    &sa.id[..12]
                } else {
                    &sa.id
                };
                let desc = if sa.summary.is_empty() {
                    format!("{} [{}] {}", status_icon, type_label, sa.last_modified)
                } else {
                    format!("{} [{}] {}", status_icon, type_label, sa.summary)
                };
                section = section.row(short_id, &desc);
            }
            section
        } else if session.subagent_count > 0 {
            Section::new("Team")
                .row("Count", &session.subagent_count.to_string())
                .row("", "Press Enter to view")
        } else {
            Section::new("Team").row("", "No agents")
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

        vec![info, agents_section, conversation]
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("Enter", "View subagents"),
            Keybinding::new("a", "Attach to session"),
            Keybinding::new("d", "Delete session"),
            Keybinding::new("j/k", "Scroll"),
        ]
    }
}
