use crate::adapters::format::{self, or_dash, short_id};
use crate::adapters::views::{DetailView, Keybinding, Section};
use crate::application::state::AppState;

pub struct SessionDetailView;

impl DetailView for SessionDetailView {
    fn title(state: &AppState) -> String {
        if let Some(session_id) = state.nav.current().context.as_deref() {
            let short = short_id(session_id, 8);
            if let Some(session) = state.store.find_session(session_id) {
                let icon = format::status_icon(session.status, state.tick);
                let label = session.name.as_deref().unwrap_or(short);
                format!("{} Session {}", icon, label)
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

        let mut info = Section::new("Info")
            .row("Session", &session.id)
            .row("Name", session.name.as_deref().unwrap_or("—"))
            .row(
                "Status",
                &format::status_display(session.status, state.tick),
            )
            .row("Path", &session.project_path);
        if let Some(ref src) = session.source_branch {
            info = info
                .row("Branch", or_dash(src))
                .row("Worktree Branch", or_dash(&session.git_branch));
        } else {
            info = info.row("Branch", or_dash(&session.git_branch));
        }
        info = info
            .row(
                "Worktree",
                &match &session.worktree {
                    Some(name) => {
                        format::worktree_display(name, session.worktree_project.as_deref())
                    }
                    None => "no".to_string(),
                },
            )
            .row("Modified", &session.last_modified);
        info = info.row(
            "Summary",
            or_dash(if session.summary.is_empty() {
                ""
            } else {
                &session.summary
            }),
        );

        // Repo Config section (lazy-loaded)
        let repo_config_section = session.repo_config.as_ref().and_then(|config| {
            let has_content = !config.mcp_servers.is_empty()
                || !config.custom_commands.is_empty()
                || !config.agent_definitions.is_empty()
                || !config.setup_scripts.is_empty();
            if !has_content {
                return None;
            }
            let mut rc = Section::new("Repo Config");
            if !config.mcp_servers.is_empty() {
                rc = rc.row("MCP Servers", &config.mcp_servers.join(", "));
            }
            if !config.custom_commands.is_empty() {
                rc = rc.row("Commands", &config.custom_commands.join(", "));
            }
            if !config.agent_definitions.is_empty() {
                rc = rc.row("Agents", &config.agent_definitions.join(", "));
            }
            if !config.setup_scripts.is_empty() {
                rc = rc.row("Setup", &config.setup_scripts.join(", "));
            }
            Some(rc)
        });

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
        let mut sections = vec![info];
        if let Some(rc) = repo_config_section {
            sections.push(rc);
        }
        sections.push(linked_teams_section);

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
                let icon = format::status_icon(sa.status, state.tick);
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
                conversation = conversation.with_loading();
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
            Keybinding::new("p", "View diff"),
            Keybinding::new("e", "Open in IDE"),
            Keybinding::new("o", "Open in new window"),
            Keybinding::new("s", "View subagents"),
            Keybinding::new("m", "View team members"),
            Keybinding::new("t", "View linked team"),
            Keybinding::new("w", "Open in git worktree"),
            Keybinding::new("d", "Drop session"),
            Keybinding::new("j/k", "Scroll"),
        ]
    }
}
