//! Input adapter — translates keyboard events into application actions.

use crossterm::event::{KeyCode, KeyEvent};

use crate::adapters::format;
use crate::adapters::views::ViewKind;
use crate::application::actions::{
    Action, AgentAction, NavAction, TableAction, TaskAction, UiAction,
};
use crate::application::state::{AppState, InputMode};

/// Map a key event to an action based on current state.
pub fn handle_key(key: KeyEvent, state: &AppState) -> Action {
    // Ctrl+C is NOT bound — it must pass through to Claude when attached.
    // Use 'q' or ':quit' to exit clash.

    // Attached and text-input modes are handled directly in the event loop
    // (app.rs) before reaching this function.
    match &state.input_mode {
        InputMode::Normal => handle_normal_mode(key, state),
        InputMode::Command
        | InputMode::Filter
        | InputMode::NewSession
        | InputMode::NewSessionName
        | InputMode::NewSessionWorktree => handle_input_mode(key),
        InputMode::Confirm => handle_confirm_mode(key, state),
        InputMode::Picker => handle_picker_mode(key),
        InputMode::Attached => Action::Noop,
    }
}

fn handle_normal_mode(key: KeyEvent, state: &AppState) -> Action {
    // Tour overlay intercepts all keys
    if state.tour_step.is_some() {
        return match key.code {
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') | KeyCode::Right => {
                Action::Ui(UiAction::TourNext)
            }
            KeyCode::Esc | KeyCode::Char('q') => Action::Ui(UiAction::TourSkip),
            _ => Action::Noop,
        };
    }

    if state.show_help {
        return match key.code {
            KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
                Action::Ui(UiAction::HideHelp)
            }
            KeyCode::Char('j') | KeyCode::Down => Action::Ui(UiAction::ScrollDown),
            KeyCode::Char('k') | KeyCode::Up => Action::Ui(UiAction::ScrollUp),
            _ => Action::Noop,
        };
    }

    // Check if we're in a detail view (scrollable, not a table)
    let is_detail_view = matches!(
        state.current_view(),
        ViewKind::TeamDetail
            | ViewKind::AgentDetail
            | ViewKind::TaskDetail
            | ViewKind::SessionDetail
            | ViewKind::SubagentDetail
            | ViewKind::Prompts
    );

    match key.code {
        // Navigation — scroll in detail views, select in tables
        KeyCode::Char('j') | KeyCode::Down => {
            if is_detail_view {
                Action::Ui(UiAction::ScrollDown)
            } else {
                Action::Table(TableAction::Next)
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if is_detail_view {
                Action::Ui(UiAction::ScrollUp)
            } else {
                Action::Table(TableAction::Prev)
            }
        }
        KeyCode::Char('g') => Action::Table(TableAction::First),
        KeyCode::Char('G') => Action::Table(TableAction::Last),
        KeyCode::Enter => handle_enter(state),
        KeyCode::Esc => Action::Nav(NavAction::GoBack),

        // Modes
        KeyCode::Char(':') => Action::Ui(UiAction::EnterCommandMode),
        KeyCode::Char('/') => Action::Ui(UiAction::EnterFilterMode),
        KeyCode::Char('?') => Action::Ui(UiAction::ToggleHelp),

        // Context-dependent actions
        KeyCode::Char('c') => handle_create(state),
        KeyCode::Char('d') => handle_delete(state),
        KeyCode::Char('A') => {
            if state.current_view() == ViewKind::Sessions {
                Action::Ui(UiAction::CycleSessionFilter)
            } else {
                Action::Noop
            }
        }
        KeyCode::Char('D') => handle_delete_all(state),
        KeyCode::Char('a') => handle_attach_or_assign(state),
        KeyCode::Char('i') => handle_inspect(state),
        KeyCode::Char('s') => handle_s_key(state),
        KeyCode::Char('m') => handle_message(state),
        KeyCode::Char('t') => handle_t_key(state),
        KeyCode::Char('n') => handle_new_session(state),
        KeyCode::Char('e') => handle_open_in_ide(state),
        KeyCode::Char('o') => handle_attach_new_window(state),
        KeyCode::Char('O') => handle_attach_all_new_windows(state),
        KeyCode::Char('w') => handle_worktree(state),
        KeyCode::Char('r') => Action::Team(crate::application::actions::TeamAction::Refresh),
        KeyCode::Tab => Action::Ui(UiAction::ToggleExpand),

        // Quit
        KeyCode::Char('q') => Action::Ui(UiAction::Quit),

        _ => Action::Noop,
    }
}

fn handle_input_mode(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Enter => Action::Ui(UiAction::SubmitInput(String::new())),
        KeyCode::Esc => Action::Ui(UiAction::ExitInputMode),
        _ => Action::Noop,
    }
}

fn handle_confirm_mode(key: KeyEvent, _state: &AppState) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Action::Ui(UiAction::ConfirmYes),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Action::Ui(UiAction::ConfirmNo),
        _ => Action::Noop,
    }
}

fn handle_picker_mode(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Action::Ui(UiAction::PickerDown),
        KeyCode::Char('k') | KeyCode::Up => Action::Ui(UiAction::PickerUp),
        KeyCode::Enter => Action::Ui(UiAction::PickerSelect),
        KeyCode::Esc => Action::Ui(UiAction::PickerCancel),
        _ => Action::Noop,
    }
}

/// Get the subagent list matching what the Subagents view displays.
/// When there's a session context, returns only that session's subagents;
/// otherwise returns all subagents.
fn subagent_items(state: &AppState) -> Vec<&crate::domain::entities::Subagent> {
    if let Some(session_id) = state.current_session() {
        state
            .store
            .subagents
            .iter()
            .filter(|s| s.parent_session_id == session_id)
            .collect()
    } else {
        state.store.all_subagents.iter().collect()
    }
}

fn handle_enter(state: &AppState) -> Action {
    match state.current_view() {
        // Enter on Sessions = drill into SessionDetail
        ViewKind::Sessions => {
            let items = state.filtered_sessions();
            if let Some(session) = items.get(state.table_state.selected) {
                Action::Nav(NavAction::DrillIn {
                    view: ViewKind::SessionDetail,
                    context: session.id.clone(),
                })
            } else {
                Action::Noop
            }
        }
        // Enter on Subagents = drill into SubagentDetail
        ViewKind::Subagents => {
            let idx = state.table_state.selected;
            let items = subagent_items(state);
            if let Some(sa) = items.get(idx) {
                Action::Nav(NavAction::DrillIn {
                    view: ViewKind::SubagentDetail,
                    context: sa.id.clone(),
                })
            } else {
                Action::Noop
            }
        }
        // TeamDetail → drill into Agents table
        ViewKind::TeamDetail => Action::Nav(NavAction::DrillIn {
            view: ViewKind::Agents,
            context: String::new(),
        }),
        // Other views drill in
        _ => Action::Nav(NavAction::DrillIn {
            view: match state.current_view() {
                ViewKind::Teams => ViewKind::TeamDetail,
                ViewKind::Agents => ViewKind::AgentDetail,
                ViewKind::Tasks => ViewKind::TaskDetail,
                ViewKind::SessionDetail => ViewKind::Subagents,
                _ => return Action::Noop,
            },
            context: String::new(),
        }),
    }
}

fn handle_create(_state: &AppState) -> Action {
    // `c` prompts for the directory before spawning a new Claude session
    Action::Ui(UiAction::EnterNewSessionMode)
}

fn handle_delete(state: &AppState) -> Action {
    match state.current_view() {
        ViewKind::Teams | ViewKind::TeamDetail => {
            if let Some(team) = state.current_team() {
                Action::Ui(UiAction::ShowConfirm {
                    message: format!("Delete team '{}'?", team),
                    on_confirm: Box::new(Action::Team(
                        crate::application::actions::TeamAction::Delete {
                            name: team.to_string(),
                        },
                    )),
                })
            } else {
                Action::Noop
            }
        }
        ViewKind::Sessions => {
            let items = state.filtered_sessions();
            if let Some(session) = items.get(state.table_state.selected) {
                let display = format::session_display_name(session);
                Action::Ui(UiAction::ShowConfirm {
                    message: format!("Drop session '{}'?", display),
                    on_confirm: Box::new(Action::Agent(AgentAction::DropSession {
                        session_id: session.id.clone(),
                    })),
                })
            } else {
                Action::Noop
            }
        }
        ViewKind::SessionDetail => {
            if let Some(session_id) = state.current_session() {
                if let Some(session) = state.store.find_session(session_id) {
                    let display = format::session_display_name(session);
                    Action::Ui(UiAction::ShowConfirm {
                        message: format!("Drop session '{}'?", display),
                        on_confirm: Box::new(Action::Agent(AgentAction::DropSession {
                            session_id: session.id.clone(),
                        })),
                    })
                } else {
                    Action::Noop
                }
            } else {
                Action::Noop
            }
        }
        _ => Action::Noop,
    }
}

fn handle_delete_all(state: &AppState) -> Action {
    match state.current_view() {
        ViewKind::Sessions => Action::Ui(UiAction::ShowConfirm {
            message: "Drop ALL sessions?".to_string(),
            on_confirm: Box::new(Action::Agent(AgentAction::DropAllSessions)),
        }),
        _ => Action::Noop,
    }
}

/// Resolve the session ID from the current view context.
///
/// Returns `Some(session_id)` for views that have an actionable session
/// (Sessions, SessionDetail, Subagents, SubagentDetail), or `None`.
fn resolve_session_id(state: &AppState) -> Option<String> {
    match state.current_view() {
        ViewKind::Sessions => state
            .filtered_sessions()
            .get(state.table_state.selected)
            .map(|s| s.id.clone()),
        ViewKind::SessionDetail => state.current_session().map(|s| s.to_string()),
        ViewKind::Subagents => {
            let items = subagent_items(state);
            items
                .get(state.table_state.selected)
                .map(|sa| sa.parent_session_id.clone())
        }
        ViewKind::SubagentDetail => state
            .nav
            .current()
            .context
            .as_deref()
            .and_then(|aid| state.store.find_subagent(aid))
            .map(|sa| sa.parent_session_id.clone()),
        _ => None,
    }
}

fn handle_attach_or_assign(state: &AppState) -> Action {
    if let Some(session_id) = resolve_session_id(state) {
        return Action::Agent(AgentAction::Attach { session_id });
    }
    // Non-session views: TeamDetail drills into Agents
    match state.current_view() {
        ViewKind::TeamDetail => Action::Nav(NavAction::DrillIn {
            view: ViewKind::Agents,
            context: String::new(),
        }),
        _ => Action::Noop,
    }
}

fn handle_open_in_ide(state: &AppState) -> Action {
    if let Some(session_id) = resolve_session_id(state) {
        return Action::Agent(AgentAction::OpenInIde { session_id });
    }
    Action::Ui(UiAction::Toast("No session selected".to_string()))
}

fn handle_attach_new_window(state: &AppState) -> Action {
    if let Some(session_id) = resolve_session_id(state) {
        return Action::Agent(AgentAction::AttachNewWindow { session_id });
    }
    Action::Ui(UiAction::Toast("No session selected".to_string()))
}

fn handle_attach_all_new_windows(state: &AppState) -> Action {
    if state.current_view() != ViewKind::Sessions {
        return Action::Noop;
    }
    let count = state
        .filtered_sessions()
        .iter()
        .filter(|s| s.is_running && !state.externally_opened.contains(&s.id))
        .count();
    if count == 0 {
        return Action::Ui(UiAction::Toast(
            "No running sessions (or all already open)".to_string(),
        ));
    }
    Action::Ui(UiAction::ShowConfirm {
        message: format!("Open {} session(s) in new tabs?", count),
        on_confirm: Box::new(Action::Agent(AgentAction::AttachAllNewWindows)),
    })
}

fn handle_s_key(state: &AppState) -> Action {
    match state.current_view() {
        // On Sessions list, `s` stashes/unstashes the selected session
        ViewKind::Sessions => {
            let items = state.filtered_sessions();
            if let Some(session) = items.get(state.table_state.selected) {
                Action::Agent(AgentAction::StashSession {
                    session_id: session.id.clone(),
                })
            } else {
                Action::Noop
            }
        }
        // On SessionDetail, `s` drills into Subagents
        ViewKind::SessionDetail => Action::Nav(NavAction::DrillIn {
            view: ViewKind::Subagents,
            context: String::new(),
        }),
        // On TeamDetail, `s` views lead session
        ViewKind::TeamDetail => {
            if let Some(team_name) = state.current_team() {
                if let Some(team) = state.store.find_team(team_name) {
                    if let Some(ref session_id) = team.lead_session_id {
                        return Action::Nav(NavAction::DrillIn {
                            view: ViewKind::SessionDetail,
                            context: session_id.clone(),
                        });
                    }
                }
            }
            Action::Noop
        }
        // On Tasks, `s` cycles task status
        ViewKind::Tasks => {
            if let Some(team) = state.current_team() {
                let tasks = state.store.get_tasks(team);
                if let Some(task) = tasks.get(state.table_state.selected) {
                    return Action::Task(TaskAction::CycleStatus {
                        team: team.to_string(),
                        task_id: task.id.clone(),
                    });
                }
            }
            Action::Noop
        }
        _ => Action::Noop,
    }
}

fn handle_t_key(state: &AppState) -> Action {
    match state.current_view() {
        // On SessionDetail, `t` drills into linked team (or falls back to Teams list)
        ViewKind::SessionDetail => {
            if let Some(session_id) = state.current_session() {
                // Find a team whose lead_session_id matches this session
                if let Some(team) = state
                    .store
                    .teams
                    .iter()
                    .find(|t| t.lead_session_id.as_deref() == Some(session_id))
                {
                    return Action::Nav(NavAction::DrillIn {
                        view: ViewKind::TeamDetail,
                        context: team.name.clone(),
                    });
                }
            }
            Action::Nav(NavAction::NavigateTo(ViewKind::Teams))
        }
        // On TeamDetail, `t` drills into Tasks
        ViewKind::TeamDetail => Action::Nav(NavAction::DrillIn {
            view: ViewKind::Tasks,
            context: String::new(),
        }),
        _ => Action::Noop,
    }
}

fn handle_message(state: &AppState) -> Action {
    match state.current_view() {
        ViewKind::Agents | ViewKind::AgentDetail | ViewKind::Inbox => {
            Action::Ui(UiAction::EnterCommandMode)
        }
        ViewKind::SessionDetail => {
            if let Some(session_id) = state.current_session() {
                if let Some(team) = state
                    .store
                    .teams
                    .iter()
                    .find(|t| t.lead_session_id.as_deref() == Some(session_id))
                {
                    return Action::Nav(NavAction::DrillIn {
                        view: ViewKind::Agents,
                        context: team.name.clone(),
                    });
                }
            }
            // No linked team — show all agents
            Action::Nav(NavAction::NavigateTo(ViewKind::Agents))
        }
        _ => Action::Noop,
    }
}

fn handle_inspect(state: &AppState) -> Action {
    match state.current_view() {
        ViewKind::Sessions => {
            let items = state.filtered_sessions();
            if let Some(session) = items.get(state.table_state.selected) {
                Action::Nav(NavAction::DrillIn {
                    view: ViewKind::SessionDetail,
                    context: session.id.clone(),
                })
            } else {
                Action::Noop
            }
        }
        ViewKind::Subagents => Action::Nav(NavAction::DrillIn {
            view: ViewKind::SubagentDetail,
            context: String::new(),
        }),
        ViewKind::Teams => {
            if let Some(team) = state.store.teams.get(state.table_state.selected) {
                Action::Nav(NavAction::DrillIn {
                    view: ViewKind::TeamDetail,
                    context: team.name.clone(),
                })
            } else {
                Action::Noop
            }
        }
        _ => Action::Noop,
    }
}

fn handle_new_session(state: &AppState) -> Action {
    match state.current_view() {
        ViewKind::Sessions | ViewKind::SessionDetail => Action::Ui(UiAction::EnterNewSessionMode),
        _ => Action::Noop,
    }
}

fn handle_worktree(state: &AppState) -> Action {
    match state.current_view() {
        ViewKind::Sessions => {
            let items = state.filtered_sessions();
            if let Some(session) = items.get(state.table_state.selected) {
                Action::Agent(AgentAction::SpawnInWorktree {
                    session_id: session.id.clone(),
                })
            } else {
                Action::Noop
            }
        }
        ViewKind::SessionDetail => {
            if let Some(id) = state.current_session() {
                Action::Agent(AgentAction::SpawnInWorktree {
                    session_id: id.to_string(),
                })
            } else {
                Action::Noop
            }
        }
        _ => Action::Noop,
    }
}

/// Parse a command string (from `:` mode).
pub fn parse_command(cmd: &str) -> Action {
    let cmd = cmd.trim();

    // Handle "delete team <name>" / "remove team <name>"
    if let Some(rest) = cmd
        .strip_prefix("delete team ")
        .or_else(|| cmd.strip_prefix("remove team "))
    {
        let name = rest.trim();
        if !name.is_empty() {
            return Action::Ui(UiAction::ShowConfirm {
                message: format!("Delete team '{}'?", name),
                on_confirm: Box::new(Action::Team(
                    crate::application::actions::TeamAction::Delete {
                        name: name.to_string(),
                    },
                )),
            });
        }
        return Action::Ui(UiAction::Toast("Usage: delete team <name>".to_string()));
    }

    // Handle "create team <name>" and "create task <team> <subject>"
    if let Some(rest) = cmd.strip_prefix("create team ") {
        let name = rest.trim();
        if !name.is_empty() {
            return Action::Team(crate::application::actions::TeamAction::Create {
                name: name.to_string(),
                description: String::new(),
            });
        }
        return Action::Ui(UiAction::Toast("Usage: create team <name>".to_string()));
    }
    if let Some(rest) = cmd.strip_prefix("create task ") {
        let parts: Vec<&str> = rest.splitn(2, ' ').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Action::Task(crate::application::actions::TaskAction::Create {
                team: parts[0].to_string(),
                subject: parts[1].to_string(),
                description: String::new(),
            });
        }
        return Action::Ui(UiAction::Toast(
            "Usage: create task <team> <subject>".to_string(),
        ));
    }

    // Handle "new [path]" to spawn a session in a specific directory
    if cmd == "new" || cmd.starts_with("new ") {
        let path = cmd.strip_prefix("new").unwrap().trim();
        if path.is_empty() {
            // No path — prompt for it
            return Action::Ui(UiAction::EnterNewSessionMode);
        }
        return Action::Agent(crate::application::actions::AgentAction::SpawnSession {
            cwd: path.to_string(),
            name: None,
        });
    }

    match cmd {
        "teams" | "team" => Action::Nav(NavAction::NavigateTo(ViewKind::Teams)),
        "agents" | "agent" => Action::Nav(NavAction::NavigateTo(ViewKind::Agents)),
        "tasks" | "task" => Action::Nav(NavAction::NavigateTo(ViewKind::Tasks)),
        "inbox" => Action::Nav(NavAction::NavigateTo(ViewKind::Inbox)),
        "prompts" | "prompt" => Action::Nav(NavAction::NavigateTo(ViewKind::Prompts)),
        "sessions" | "session" => Action::Nav(NavAction::NavigateTo(ViewKind::Sessions)),
        "active" => Action::Ui(UiAction::SetSessionFilter(
            crate::application::state::SessionFilter::Active,
        )),
        "all" => Action::Ui(UiAction::SetSessionFilter(
            crate::application::state::SessionFilter::All,
        )),
        "subagents" | "subagent" => Action::Nav(NavAction::NavigateTo(ViewKind::Subagents)),
        "tour" | "guide" => Action::Ui(UiAction::StartTour),
        "update" | "upgrade" => Action::Ui(UiAction::RequestUpdate),
        "quit" | "q" => Action::Ui(UiAction::Quit),
        _ => Action::Ui(UiAction::Toast(format!("Unknown command: {}", cmd))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_teams() {
        match parse_command("teams") {
            Action::Nav(NavAction::NavigateTo(ViewKind::Teams)) => {}
            _ => panic!("Expected NavigateTo Teams"),
        }
    }

    #[test]
    fn test_parse_command_quit() {
        match parse_command("q") {
            Action::Ui(UiAction::Quit) => {}
            _ => panic!("Expected Quit"),
        }
    }

    #[test]
    fn test_parse_command_unknown() {
        match parse_command("foobar") {
            Action::Ui(UiAction::Toast(_)) => {}
            _ => panic!("Expected Toast for unknown command"),
        }
    }

    #[test]
    fn test_w_key_on_sessions_view() {
        let mut state = AppState::new();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        }];
        state.table_state.selected = 0;
        // Default view is Sessions
        let key = KeyEvent::new(KeyCode::Char('w'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Agent(AgentAction::SpawnInWorktree { session_id }) => {
                assert_eq!(session_id, "s1");
            }
            other => panic!("Expected SpawnInWorktree, got {:?}", other),
        }
    }

    #[test]
    fn test_w_key_on_teams_view_is_noop() {
        let mut state = AppState::new();
        state.nav.push(ViewKind::Teams, None);
        let key = KeyEvent::new(KeyCode::Char('w'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Noop => {}
            other => panic!("Expected Noop on Teams view, got {:?}", other),
        }
    }

    #[test]
    fn test_o_key_on_sessions_view_attaches_new_window() {
        let mut state = AppState::new();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        }];
        state.table_state.selected = 0;
        let key = KeyEvent::new(KeyCode::Char('o'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Agent(AgentAction::AttachNewWindow { session_id }) => {
                assert_eq!(session_id, "s1");
            }
            other => panic!("Expected AttachNewWindow, got {:?}", other),
        }
    }

    #[test]
    fn test_o_key_on_session_detail_attaches_new_window() {
        let mut state = AppState::new();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        }];
        state.session_filter = crate::application::state::SessionFilter::All;
        state
            .nav
            .push(ViewKind::SessionDetail, Some("s1".to_string()));
        let key = KeyEvent::new(KeyCode::Char('o'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Agent(AgentAction::AttachNewWindow { session_id }) => {
                assert_eq!(session_id, "s1");
            }
            other => panic!("Expected AttachNewWindow, got {:?}", other),
        }
    }

    #[test]
    fn test_shift_o_on_sessions_shows_confirm() {
        let mut state = AppState::new();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        }];
        let key = KeyEvent::new(KeyCode::Char('O'), crossterm::event::KeyModifiers::SHIFT);
        match handle_key(key, &state) {
            Action::Ui(UiAction::ShowConfirm {
                message,
                on_confirm,
            }) => {
                assert!(message.contains("1 session"));
                assert!(matches!(
                    *on_confirm,
                    Action::Agent(AgentAction::AttachAllNewWindows)
                ));
            }
            other => panic!("Expected ShowConfirm, got {:?}", other),
        }
    }

    #[test]
    fn test_shift_o_no_running_sessions_toasts() {
        let mut state = AppState::new();
        state.session_filter = crate::application::state::SessionFilter::All;
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "idle1".to_string(),
            is_running: false,
            status: crate::domain::entities::SessionStatus::Idle,
            ..Default::default()
        }];
        let key = KeyEvent::new(KeyCode::Char('O'), crossterm::event::KeyModifiers::SHIFT);
        match handle_key(key, &state) {
            Action::Ui(UiAction::Toast(msg)) => {
                assert!(msg.contains("No running"));
            }
            other => panic!("Expected Toast, got {:?}", other),
        }
    }

    #[test]
    fn test_shift_o_on_session_detail_is_noop() {
        let mut state = AppState::new();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        }];
        state.session_filter = crate::application::state::SessionFilter::All;
        state
            .nav
            .push(ViewKind::SessionDetail, Some("s1".to_string()));
        let key = KeyEvent::new(KeyCode::Char('O'), crossterm::event::KeyModifiers::SHIFT);
        match handle_key(key, &state) {
            Action::Noop => {}
            other => panic!("Expected Noop on SessionDetail, got {:?}", other),
        }
    }

    #[test]
    fn test_i_on_teams_drills_to_detail() {
        let mut state = AppState::new();
        state.nav.push(ViewKind::Teams, None);
        state.store.teams = vec![crate::domain::entities::Team {
            name: "my-team".to_string(),
            ..Default::default()
        }];
        state.table_state.selected = 0;
        let key = KeyEvent::new(KeyCode::Char('i'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Nav(NavAction::DrillIn { view, context }) => {
                assert_eq!(view, ViewKind::TeamDetail);
                assert_eq!(context, "my-team");
            }
            other => panic!("Expected DrillIn to TeamDetail, got {:?}", other),
        }
    }

    #[test]
    fn test_e_key_sessions_view() {
        let mut state = AppState::new();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        }];
        state.table_state.selected = 0;
        let key = KeyEvent::new(KeyCode::Char('e'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Agent(AgentAction::OpenInIde { session_id }) => {
                assert_eq!(session_id, "s1");
            }
            other => panic!("Expected OpenInIde, got {:?}", other),
        }
    }

    #[test]
    fn test_e_key_non_session_view_toasts() {
        let mut state = AppState::new();
        state
            .nav
            .push(crate::adapters::views::ViewKind::Teams, None);
        let key = KeyEvent::new(KeyCode::Char('e'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Ui(UiAction::Toast(msg)) => {
                assert!(msg.contains("No session selected"));
            }
            other => panic!("Expected Toast, got {:?}", other),
        }
    }

    #[test]
    fn test_picker_mode_keys() {
        let key_j = KeyEvent::new(KeyCode::Char('j'), crossterm::event::KeyModifiers::NONE);
        match handle_picker_mode(key_j) {
            Action::Ui(UiAction::PickerDown) => {}
            other => panic!("Expected PickerDown, got {:?}", other),
        }

        let key_k = KeyEvent::new(KeyCode::Char('k'), crossterm::event::KeyModifiers::NONE);
        match handle_picker_mode(key_k) {
            Action::Ui(UiAction::PickerUp) => {}
            other => panic!("Expected PickerUp, got {:?}", other),
        }

        let key_enter = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        match handle_picker_mode(key_enter) {
            Action::Ui(UiAction::PickerSelect) => {}
            other => panic!("Expected PickerSelect, got {:?}", other),
        }

        let key_esc = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        match handle_picker_mode(key_esc) {
            Action::Ui(UiAction::PickerCancel) => {}
            other => panic!("Expected PickerCancel, got {:?}", other),
        }
    }

    #[test]
    fn test_o_key_on_teams_view_toasts() {
        let mut state = AppState::new();
        state
            .nav
            .push(crate::adapters::views::ViewKind::Teams, None);
        let key = KeyEvent::new(KeyCode::Char('o'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Ui(UiAction::Toast(msg)) => {
                assert!(msg.contains("No session selected"));
            }
            other => panic!("Expected Toast, got {:?}", other),
        }
    }

    #[test]
    fn test_delete_session_shows_name() {
        let mut state = AppState::new();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "abcdef1234567890".to_string(),
            name: Some("my-session".to_string()),
            is_running: true,
            ..Default::default()
        }];
        state.table_state.selected = 0;
        let key = KeyEvent::new(KeyCode::Char('d'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Ui(UiAction::ShowConfirm { message, .. }) => {
                assert!(
                    message.contains("my-session"),
                    "Expected name in confirm: {}",
                    message
                );
            }
            other => panic!("Expected ShowConfirm, got {:?}", other),
        }
    }

    #[test]
    fn test_delete_session_detail_shows_name() {
        let mut state = AppState::new();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "abcdef1234567890".to_string(),
            name: Some("detail-session".to_string()),
            is_running: true,
            ..Default::default()
        }];
        state.session_filter = crate::application::state::SessionFilter::All;
        state.nav.push(
            ViewKind::SessionDetail,
            Some("abcdef1234567890".to_string()),
        );
        let key = KeyEvent::new(KeyCode::Char('d'), crossterm::event::KeyModifiers::NONE);
        match handle_key(key, &state) {
            Action::Ui(UiAction::ShowConfirm { message, .. }) => {
                assert!(
                    message.contains("detail-session"),
                    "Expected name in confirm: {}",
                    message
                );
            }
            other => panic!("Expected ShowConfirm, got {:?}", other),
        }
    }
}
