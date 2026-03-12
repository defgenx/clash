//! Pure reducer — the core state machine.
//!
//! `reduce(state, action) → Vec<Effect>` is a pure function: it mutates
//! the in-memory state and returns a list of effects for the infrastructure
//! to execute. It performs no IO, no async, no file access.

use crate::adapters::input::parse_command;
use crate::adapters::views::ViewKind;
use crate::application::actions::*;
use crate::application::effects::{CliCommand, Effect};
use crate::application::state::{AppState, ConfirmDialog, InputMode};

/// Pure reducer: takes state + action, returns effects.
/// All state mutation happens here and only here.
pub fn reduce(state: &mut AppState, action: Action) -> Vec<Effect> {
    match action {
        Action::Nav(nav) => reduce_nav(state, nav),
        Action::Table(table) => reduce_table(state, table),
        Action::Team(team) => reduce_team(state, team),
        Action::Task(task) => reduce_task(state, task),
        Action::Agent(agent) => reduce_agent(state, agent),
        Action::Ui(ui) => reduce_ui(state, ui),
        Action::Noop => vec![],
        Action::CliResult {
            success,
            output,
            follow_up,
        } => {
            state.spinner = None;
            if !success {
                state.toast = Some(format!("CLI error: {}", output));
            }
            reduce(state, *follow_up)
        }
    }
}

fn reduce_nav(state: &mut AppState, action: NavAction) -> Vec<Effect> {
    match action {
        NavAction::NavigateTo(view) => {
            // Always push so Esc goes back to the previous view
            state.nav.push(view, None);
            state.table_state.selected = 0;
            state.scroll_state.offset = 0;
            state.filter.clear();
            load_effects_for_view(state, view)
        }
        NavAction::DrillIn { view, context } => {
            let ctx = if context.is_empty() {
                resolve_context(state)
            } else {
                Some(context)
            };

            // Some drill-ins don't need a new context (e.g. SessionDetail → Subagents)
            // — the parent context is already on the nav stack
            let needs_context = matches!(
                view,
                ViewKind::TeamDetail
                    | ViewKind::AgentDetail
                    | ViewKind::TaskDetail
                    | ViewKind::SessionDetail
                    | ViewKind::SubagentDetail
            );

            if let Some(ctx) = ctx {
                state.nav.push(view, Some(ctx));
                state.table_state.selected = 0;
                state.scroll_state.offset = 0;
                state.filter.clear();
                load_effects_for_view(state, view)
            } else if !needs_context {
                // Navigate without context (e.g. into a list view)
                state.nav.push(view, None);
                state.table_state.selected = 0;
                state.scroll_state.offset = 0;
                state.filter.clear();
                load_effects_for_view(state, view)
            } else {
                vec![]
            }
        }
        NavAction::GoBack => {
            state.nav.pop();
            state.table_state.selected = 0;
            state.scroll_state.offset = 0;
            state.filter.clear();
            state.store.conversation_loaded = false;
            vec![]
        }
    }
}

fn reduce_table(state: &mut AppState, action: TableAction) -> Vec<Effect> {
    let item_count = current_item_count(state);

    match action {
        TableAction::Next => {
            if item_count > 0 && state.table_state.selected < item_count - 1 {
                state.table_state.selected += 1;
            }
        }
        TableAction::Prev => {
            if state.table_state.selected > 0 {
                state.table_state.selected -= 1;
            }
        }
        TableAction::First => {
            state.table_state.selected = 0;
        }
        TableAction::Last => {
            if item_count > 0 {
                state.table_state.selected = item_count - 1;
            }
        }
    }
    vec![]
}

fn reduce_team(state: &mut AppState, action: TeamAction) -> Vec<Effect> {
    match action {
        TeamAction::Create { name, description } => {
            vec![
                Effect::ShowSpinner(format!("Creating team '{}'...", name)),
                Effect::RunCli {
                    command: CliCommand::CreateTeam { name, description },
                    on_complete: Action::Team(TeamAction::Refresh),
                },
            ]
        }
        TeamAction::Delete { name } => {
            state.toast = Some(format!("Deleted team '{}'", name));
            if state.current_team() == Some(&name) {
                state.nav.replace(ViewKind::Teams);
            }
            vec![Effect::RemoveTeam { name }, Effect::RefreshAll]
        }
        TeamAction::Refresh => {
            vec![Effect::RefreshAll]
        }
    }
}

fn reduce_task(state: &mut AppState, action: TaskAction) -> Vec<Effect> {
    match action {
        TaskAction::Create {
            team,
            subject,
            description,
        } => {
            let id = format!("{}", chrono::Utc::now().timestamp_millis());
            let task = crate::domain::entities::Task {
                id,
                subject,
                description,
                ..Default::default()
            };
            state.toast = Some("Task created".to_string());
            vec![Effect::PersistTask { team, task }, Effect::RefreshAll]
        }
        TaskAction::UpdateStatus {
            team,
            task_id,
            status,
        } => {
            if let Some(tasks) = state.store.tasks.get(&team) {
                if let Some(task) = tasks.iter().find(|t| t.id == task_id) {
                    let mut updated = task.clone();
                    updated.status = status;
                    return vec![
                        Effect::PersistTask {
                            team,
                            task: updated,
                        },
                        Effect::RefreshAll,
                    ];
                }
            }
            vec![]
        }
        TaskAction::CycleStatus { team, task_id } => {
            if let Some(tasks) = state.store.tasks.get(&team) {
                if let Some(task) = tasks.iter().find(|t| t.id == task_id) {
                    let new_status = task.status.next();
                    state.toast = Some(format!("Status → {}", new_status));
                    return reduce_task(
                        state,
                        TaskAction::UpdateStatus {
                            team,
                            task_id,
                            status: new_status,
                        },
                    );
                }
            }
            vec![]
        }
    }
}

fn reduce_agent(state: &mut AppState, action: AgentAction) -> Vec<Effect> {
    match action {
        AgentAction::Attach { session_id } => {
            // Attach inline via daemon — vt100 terminal emulator renders in TUI
            state.input_mode = InputMode::Attached;
            state.attached_session = Some(session_id.clone());

            state.scroll_state.offset = 0;
            vec![Effect::DaemonAttach { session_id }]
        }
        AgentAction::SpawnSession { cwd, name } => {
            // Create a new daemon-managed session and attach inline.
            // Use a UUID as the internal session ID for the daemon PTY.
            // We don't pass --session-id to Claude — let it manage its own.
            let session_id = uuid::Uuid::now_v7().to_string();
            state.input_mode = InputMode::Attached;
            state.attached_session = Some(session_id.clone());

            // Default name: "Clash-N" where N is based on current session count.
            let session_name = name.unwrap_or_else(|| {
                let n = state.store.sessions.len() + 1;
                format!("Clash-{}", n)
            });

            state.scroll_state.offset = 0;
            vec![
                Effect::DaemonCreateSession {
                    session_id: session_id.clone(),
                    args: vec![],
                    cwd,
                    name: Some(session_name),
                },
                Effect::DaemonAttach { session_id },
            ]
        }
        AgentAction::TerminateAndDelete {
            project,
            session_id,
        } => {
            state.toast = Some("Terminating session...".to_string());
            state.nav.pop();
            let mut effects = vec![
                Effect::DaemonKill {
                    session_id: session_id.clone(),
                },
                Effect::TerminateProcess {
                    session_id: session_id.clone(),
                },
            ];
            if !project.is_empty() {
                effects.push(Effect::DeleteSession {
                    project,
                    session_id,
                });
            }
            effects.push(Effect::RefreshSessions);
            effects
        }
        AgentAction::DeleteSession {
            project,
            session_id,
        } => {
            state.toast = Some("Session files deleted".to_string());
            state.nav.pop();
            let mut effects = vec![Effect::DaemonKill {
                session_id: session_id.clone(),
            }];
            if !project.is_empty() {
                effects.push(Effect::DeleteSession {
                    project,
                    session_id,
                });
            }
            effects.push(Effect::RefreshSessions);
            effects
        }
        AgentAction::TerminateAndDeleteAllSessions => {
            state.toast = Some("Terminating all sessions...".to_string());
            vec![
                Effect::DaemonKillAll,
                Effect::TerminateAllProcesses,
                Effect::DeleteAllSessions,
                Effect::RefreshSessions,
            ]
        }
        AgentAction::DeleteAllSessions => {
            state.toast = Some("All session files deleted".to_string());
            vec![
                Effect::DaemonKillAll,
                Effect::DeleteAllSessions,
                Effect::RefreshSessions,
            ]
        }
    }
}

fn reduce_ui(state: &mut AppState, action: UiAction) -> Vec<Effect> {
    match action {
        UiAction::HideHelp => {
            state.show_help = false;
            state.help_scroll = 0;
            vec![]
        }
        UiAction::ToggleHelp => {
            state.show_help = !state.show_help;
            if state.show_help {
                state.help_scroll = 0;
            }
            vec![]
        }
        UiAction::RequestUpdate => {
            state.toast = Some("Updating clash...".to_string());
            vec![Effect::PerformUpdate]
        }
        UiAction::StartTour => {
            state.tour_step = Some(0);
            vec![]
        }
        UiAction::TourNext => {
            if let Some(step) = state.tour_step {
                let total = crate::infrastructure::tui::widgets::tour::TOUR_STEPS.len();
                if step + 1 < total {
                    state.tour_step = Some(step + 1);
                } else {
                    state.tour_step = None;
                }
            }
            vec![]
        }
        UiAction::TourSkip => {
            state.tour_step = None;
            vec![]
        }
        UiAction::ShowConfirm {
            message,
            on_confirm,
        } => {
            state.confirm_dialog = Some(ConfirmDialog::Simple {
                message,
                on_confirm: *on_confirm,
            });
            state.input_mode = InputMode::Confirm;
            vec![]
        }
        UiAction::ShowDeleteConfirm {
            message,
            on_terminate,
            on_files_only,
        } => {
            state.confirm_dialog = Some(ConfirmDialog::Delete {
                message,
                on_terminate: *on_terminate,
                on_files_only: *on_files_only,
            });
            state.input_mode = InputMode::Confirm;
            vec![]
        }
        UiAction::ConfirmYes | UiAction::ConfirmTerminate => {
            state.input_mode = InputMode::Normal;
            let action = match state.confirm_dialog.take() {
                Some(ConfirmDialog::Simple { on_confirm, .. }) => Some(on_confirm),
                Some(ConfirmDialog::Delete { on_terminate, .. }) => Some(on_terminate),
                None => None,
            };
            if let Some(action) = action {
                return reduce(state, action);
            }
            vec![]
        }
        UiAction::ConfirmFilesOnly => {
            state.input_mode = InputMode::Normal;
            let action = match state.confirm_dialog.take() {
                Some(ConfirmDialog::Delete { on_files_only, .. }) => Some(on_files_only),
                _ => None,
            };
            if let Some(action) = action {
                return reduce(state, action);
            }
            vec![]
        }
        UiAction::ConfirmNo => {
            state.input_mode = InputMode::Normal;
            state.confirm_dialog = None;
            vec![]
        }
        UiAction::Toast(msg) => {
            state.toast = Some(msg);
            vec![]
        }
        UiAction::EnterCommandMode => {
            state.input_mode = InputMode::Command;
            state.input_buffer.clear();
            state.input_cursor = 0;
            vec![]
        }
        UiAction::EnterFilterMode => {
            state.input_mode = InputMode::Filter;
            state.input_buffer.clear();
            state.input_cursor = 0;
            vec![]
        }
        UiAction::EnterNewSessionMode => {
            state.input_mode = InputMode::NewSession;
            state.input_buffer = state.default_cwd.clone();
            state.input_cursor = state.input_buffer.len();
            vec![]
        }
        UiAction::ExitInputMode => {
            state.input_mode = InputMode::Normal;
            state.input_buffer.clear();
            state.input_cursor = 0;
            vec![]
        }
        UiAction::SubmitInput(text) => {
            let input = if text.is_empty() {
                state.input_buffer.clone()
            } else {
                text
            };
            let mode = state.input_mode.clone();
            state.input_mode = InputMode::Normal;
            state.input_buffer.clear();
            state.input_cursor = 0;

            match mode {
                InputMode::Command => {
                    let action = parse_command(&input);
                    reduce(state, action)
                }
                InputMode::Filter => {
                    state.filter = input;
                    vec![]
                }
                InputMode::NewSession => {
                    let cwd = input.trim().to_string();
                    let cwd = if cwd.is_empty() {
                        state.default_cwd.clone()
                    } else {
                        cwd
                    };
                    reduce(
                        state,
                        Action::Agent(AgentAction::SpawnSession { cwd, name: None }),
                    )
                }
                _ => vec![],
            }
        }
        UiAction::ScrollDown => {
            if state.show_help {
                state.help_scroll = state.help_scroll.saturating_add(1);
            } else {
                state.scroll_state.offset = state.scroll_state.offset.saturating_add(3);
            }
            vec![]
        }
        UiAction::ScrollUp => {
            if state.show_help {
                state.help_scroll = state.help_scroll.saturating_sub(1);
            } else {
                state.scroll_state.offset = state.scroll_state.offset.saturating_sub(3);
            }
            vec![]
        }
        UiAction::DetachSession => {
            let effects = if let Some(session_id) = state.attached_session.take() {
                vec![Effect::DaemonDetach { session_id }]
            } else {
                vec![]
            };
            state.input_mode = InputMode::Normal;
            state.toast = Some("Detached — session continues in background".to_string());
            effects
        }
        UiAction::SessionExited { session_id } => {
            if state.attached_session.as_deref() == Some(&session_id) {
                state.attached_session = None;
                state.input_mode = InputMode::Normal;
                state.toast = Some("Session exited".to_string());
            }
            vec![Effect::RefreshSessions]
        }
        UiAction::ToggleExpand => {
            if state.current_view() == ViewKind::Sessions {
                let sessions = state.filtered_sessions();
                if let Some(session) = sessions.get(state.table_state.selected) {
                    let id = session.id.clone();
                    if state.expanded_sessions.contains(&id) {
                        state.expanded_sessions.remove(&id);
                    } else {
                        state.expanded_sessions.insert(id);
                    }
                }
            }
            vec![]
        }
        UiAction::CycleSessionFilter => {
            state.session_filter = state.session_filter.next();
            state.table_state.selected = 0;
            state.toast = Some(format!("Showing {} sessions", state.session_filter.label()));
            vec![]
        }
        UiAction::SetSessionFilter(filter) => {
            state.session_filter = filter;
            state.table_state.selected = 0;
            state.toast = Some(format!("Showing {} sessions", state.session_filter.label()));
            // Navigate to Sessions view if not already there
            if state.current_view() != ViewKind::Sessions {
                state.nav.push(ViewKind::Sessions, None);
                state.filter.clear();
                return vec![Effect::RefreshSessions];
            }
            vec![]
        }
        UiAction::InputEdit(edit) => {
            use crate::application::actions::ui::InputEdit;
            match edit {
                InputEdit::InsertChar(c) => {
                    let pos = state.input_cursor;
                    state.input_buffer.insert(pos, c);
                    state.input_cursor += 1;
                    if state.input_mode == InputMode::Filter {
                        state.filter = state.input_buffer.clone();
                        state.table_state.selected = 0;
                    }
                }
                InputEdit::Backspace => {
                    if state.input_cursor > 0 {
                        state.input_buffer.remove(state.input_cursor - 1);
                        state.input_cursor -= 1;
                        if state.input_mode == InputMode::Filter {
                            state.filter = state.input_buffer.clone();
                            state.table_state.selected = 0;
                        }
                    }
                }
                InputEdit::Delete => {
                    if state.input_cursor < state.input_buffer.len() {
                        state.input_buffer.remove(state.input_cursor);
                        if state.input_mode == InputMode::Filter {
                            state.filter = state.input_buffer.clone();
                            state.table_state.selected = 0;
                        }
                    }
                }
                InputEdit::CursorLeft => {
                    state.input_cursor = state.input_cursor.saturating_sub(1);
                }
                InputEdit::CursorRight => {
                    if state.input_cursor < state.input_buffer.len() {
                        state.input_cursor += 1;
                    }
                }
                InputEdit::CursorHome => {
                    state.input_cursor = 0;
                }
                InputEdit::CursorEnd => {
                    state.input_cursor = state.input_buffer.len();
                }
            }
            vec![]
        }
        UiAction::Tick => {
            state.tick = state.tick.wrapping_add(1);
            if state.toast.is_some() && state.tick.is_multiple_of(300) {
                state.toast = None;
            }
            vec![]
        }
        UiAction::Quit => {
            vec![Effect::Quit]
        }
    }
}

/// Resolve the context string for drill-in based on current selection.
fn resolve_context(state: &AppState) -> Option<String> {
    let idx = state.table_state.selected;
    match state.current_view() {
        ViewKind::Teams => state.store.teams.get(idx).map(|t| t.name.clone()),
        ViewKind::Tasks => {
            if let Some(team) = state.current_team() {
                state.store.get_tasks(team).get(idx).map(|t| t.id.clone())
            } else {
                None
            }
        }
        ViewKind::Agents => state.store.all_members.get(idx).map(|m| m.name.clone()),
        ViewKind::Sessions => {
            let items = state.filtered_sessions();
            items.get(idx).map(|s| s.id.clone())
        }
        ViewKind::Subagents => state.store.all_subagents.get(idx).map(|s| s.id.clone()),
        _ => None,
    }
}

/// Get the count of items in the current table view.
fn current_item_count(state: &AppState) -> usize {
    match state.current_view() {
        ViewKind::Teams => state.store.teams.len(),
        ViewKind::Tasks => {
            if let Some(team) = state.current_team() {
                state.store.get_tasks(team).len()
            } else {
                0
            }
        }
        ViewKind::Agents => state.store.all_members.len(),
        ViewKind::Inbox => state.inbox_messages.len(),
        ViewKind::Sessions => state.filtered_sessions().len(),
        ViewKind::Subagents => state.store.all_subagents.len(),
        _ => 0,
    }
}

/// Return effects needed to load data for a view.
fn load_effects_for_view(state: &AppState, view: ViewKind) -> Vec<Effect> {
    match view {
        ViewKind::Teams => vec![Effect::RefreshAll],
        ViewKind::Sessions => vec![Effect::RefreshSessions],
        ViewKind::Tasks => {
            if let Some(team) = state.current_team() {
                vec![Effect::RefreshTeamTasks {
                    team: team.to_string(),
                }]
            } else {
                vec![]
            }
        }
        ViewKind::SessionDetail => {
            let mut effects = vec![Effect::RefreshAll];
            if let Some(session_id) = state.current_session() {
                if let Some(session) = state.store.find_session(session_id) {
                    effects.push(Effect::RefreshSubagents {
                        project: session.project.clone(),
                        session_id: session.id.clone(),
                    });
                    effects.push(Effect::LoadConversation {
                        project: session.project.clone(),
                        session_id: session.id.clone(),
                    });
                }
            }
            effects
        }
        ViewKind::Subagents => {
            // If we have a session context, refresh that session's subagents;
            // otherwise show all subagents (already loaded).
            if let Some(session_id) = state.current_session() {
                if let Some(session) = state.store.find_session(session_id) {
                    return vec![Effect::RefreshSubagents {
                        project: session.project.clone(),
                        session_id: session.id.clone(),
                    }];
                }
            }
            vec![Effect::RefreshSessions]
        }
        ViewKind::Agents => {
            vec![Effect::RefreshAll]
        }
        ViewKind::SubagentDetail => {
            // Load the subagent's conversation
            if let Some(agent_id) = state.nav.current().context.as_deref() {
                if let Some(sa) = state.store.find_subagent(agent_id) {
                    // Find the parent session to get the project
                    let parent_session_id = sa.parent_session_id.clone();
                    let project = sa.project.clone();
                    let agent_id = sa.id.clone();
                    return vec![Effect::LoadSubagentConversation {
                        project,
                        session_id: parent_session_id,
                        agent_id,
                    }];
                }
            }
            vec![]
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::state::AppState;

    fn test_state() -> AppState {
        AppState::new()
    }

    #[test]
    fn test_reduce_noop() {
        let mut state = test_state();
        let effects = reduce(&mut state, Action::Noop);
        assert!(effects.is_empty());
    }

    #[test]
    fn test_reduce_quit() {
        let mut state = test_state();
        let effects = reduce(&mut state, Action::Ui(UiAction::Quit));
        assert!(matches!(effects.first(), Some(Effect::Quit)));
    }

    #[test]
    fn test_reduce_nav_to() {
        let mut state = test_state();
        let _ = reduce(
            &mut state,
            Action::Nav(NavAction::NavigateTo(ViewKind::Tasks)),
        );
        assert_eq!(state.current_view(), ViewKind::Tasks);
    }

    #[test]
    fn test_reduce_table_select() {
        let mut state = test_state();
        // Default view is Sessions (default filter: Active), add 3 running sessions
        state.store.sessions = vec![
            crate::domain::entities::Session {
                id: "s1".to_string(),
                is_running: true,
                ..Default::default()
            },
            crate::domain::entities::Session {
                id: "s2".to_string(),
                is_running: true,
                ..Default::default()
            },
            crate::domain::entities::Session {
                id: "s3".to_string(),
                is_running: true,
                ..Default::default()
            },
        ];
        reduce(&mut state, Action::Table(TableAction::Next));
        assert_eq!(state.table_state.selected, 1);
        reduce(&mut state, Action::Table(TableAction::Next));
        assert_eq!(state.table_state.selected, 2);
        reduce(&mut state, Action::Table(TableAction::Next));
        assert_eq!(state.table_state.selected, 2); // can't go past end

        reduce(&mut state, Action::Table(TableAction::Prev));
        assert_eq!(state.table_state.selected, 1);
    }

    #[test]
    fn test_reduce_toggle_help() {
        let mut state = test_state();
        assert!(!state.show_help);
        reduce(&mut state, Action::Ui(UiAction::ToggleHelp));
        assert!(state.show_help);
        reduce(&mut state, Action::Ui(UiAction::ToggleHelp));
        assert!(!state.show_help);
    }

    #[test]
    fn test_reduce_toast() {
        let mut state = test_state();
        reduce(&mut state, Action::Ui(UiAction::Toast("hello".to_string())));
        assert_eq!(state.toast.as_deref(), Some("hello"));
    }

    #[test]
    fn test_reduce_command_mode() {
        let mut state = test_state();
        reduce(&mut state, Action::Ui(UiAction::EnterCommandMode));
        assert!(matches!(state.input_mode, InputMode::Command));
        reduce(&mut state, Action::Ui(UiAction::ExitInputMode));
        assert!(matches!(state.input_mode, InputMode::Normal));
    }

    #[test]
    fn test_reduce_confirm_flow() {
        let mut state = test_state();
        reduce(
            &mut state,
            Action::Ui(UiAction::ShowConfirm {
                message: "Are you sure?".to_string(),
                on_confirm: Box::new(Action::Ui(UiAction::Toast("confirmed!".to_string()))),
            }),
        );
        assert!(state.confirm_dialog.is_some());
        assert!(matches!(state.input_mode, InputMode::Confirm));

        reduce(&mut state, Action::Ui(UiAction::ConfirmYes));
        assert_eq!(state.toast.as_deref(), Some("confirmed!"));
        assert!(state.confirm_dialog.is_none());
    }

    #[test]
    fn test_reduce_go_back() {
        let mut state = test_state();
        reduce(
            &mut state,
            Action::Nav(NavAction::NavigateTo(ViewKind::Tasks)),
        );
        reduce(&mut state, Action::Nav(NavAction::GoBack));
    }

    #[test]
    fn test_reduce_team_create() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Team(TeamAction::Create {
                name: "test".to_string(),
                description: "A test".to_string(),
            }),
        );
        assert!(effects.iter().any(|e| matches!(e, Effect::ShowSpinner(_))));
        assert!(effects.iter().any(|e| matches!(e, Effect::RunCli { .. })));
    }

    #[test]
    fn test_reduce_filter_mode() {
        let mut state = test_state();
        reduce(&mut state, Action::Ui(UiAction::EnterFilterMode));
        assert!(matches!(state.input_mode, InputMode::Filter));

        state.input_buffer = "test".to_string();
        reduce(&mut state, Action::Ui(UiAction::SubmitInput(String::new())));
        assert_eq!(state.filter, "test");
    }

    #[test]
    fn test_reduce_task_create_produces_persist_effect() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Task(TaskAction::Create {
                team: "my-team".to_string(),
                subject: "Test task".to_string(),
                description: "Description".to_string(),
            }),
        );
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::PersistTask { .. })));
        assert!(effects.iter().any(|e| matches!(e, Effect::RefreshAll)));
    }

    #[test]
    fn test_reduce_team_delete_produces_remove_effect() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Team(TeamAction::Delete {
                name: "old-team".to_string(),
            }),
        );
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::RemoveTeam { .. })));
    }
}
