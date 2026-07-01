//! Pure reducer — the core state machine.
//!
//! `reduce(state, action) → Vec<Effect>` is a pure function: it mutates
//! the in-memory state and returns a list of effects for the infrastructure
//! to execute. It performs no IO, no async, no file access.

use crate::adapters::input::parse_command;
use crate::adapters::views::ViewKind;
use crate::application::actions::*;
use crate::application::effects::Effect;
use crate::application::state::{visible_scratch_indices, AppState, ConfirmDialog, InputMode};
use crate::domain::entities::ScratchNote;

/// Derive a human-meaningful default session name when the user didn't supply
/// one. Returns a short `clash-<id>` tag (first 8 chars of the session id) —
/// a stable, recognizable default that survives stash/restart. Users rename
/// from there when they want something meaningful; the prefilled `clash-*`
/// is just a unique placeholder, never the literal `"session"`.
///
/// Pure and frontend-agnostic so the TUI's new-session flow and the GUI's
/// `create_new_session` derive identical defaults (one core, two frontends).
pub fn default_session_name(_cwd: &str, session_id: &str) -> String {
    format!("clash-{}", &session_id[..session_id.len().min(8)])
}

/// Pure reducer: takes state + action, returns effects.
/// All state mutation happens here and only here.
pub fn reduce(state: &mut AppState, action: Action) -> Vec<Effect> {
    match action {
        Action::Nav(nav) => reduce_nav(state, nav),
        Action::Table(table) => reduce_table(state, table),
        Action::Team(team) => reduce_team(state, team),
        Action::Task(task) => reduce_task(state, task),
        Action::Scratch(scratch) => reduce_scratch(state, scratch),
        Action::Agent(agent) => reduce_agent(state, agent),
        Action::Ui(ui) => reduce_ui(state, ui),
        Action::Noop => vec![],
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
                    | ViewKind::Diff
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
            // Reset diff state when leaving Diff view
            if state.current_view() == ViewKind::Diff {
                state.diff = crate::application::state::DiffState::default();
            }
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
            state.toast = Some(format!("Created team '{}'", name));
            vec![Effect::CreateTeam { name, description }]
        }
        TeamAction::Delete { name } => {
            state.toast = Some(format!("Deleted team '{}'", name));
            if state.current_team() == Some(&name) {
                // Pop back to the Teams list (preserving the stack so Esc
                // still returns to Sessions). Don't use replace() — that
                // clears the entire stack and makes Teams the root.
                while state.current_view() != ViewKind::Teams {
                    if !state.nav.pop() {
                        break;
                    }
                }
            }
            state.table_state.selected = 0;
            vec![Effect::RemoveTeam { name }, Effect::RefreshAll]
        }
        TeamAction::Refresh => {
            vec![Effect::RefreshAll]
        }
        TeamAction::SetDescription { name, description } => mutate_team(state, &name, |team| {
            team.set_description(&description);
            Ok("Description updated".to_string())
        }),
        TeamAction::AddMember {
            team,
            name,
            agent_type,
            model,
        } => mutate_team(state, &team, |t| {
            t.add_member(&name, &agent_type, &model)?;
            Ok(format!("Added member '{}'", name.trim()))
        }),
        TeamAction::RemoveMember { team, member } => mutate_team(state, &team, |t| {
            t.remove_member(&member)?;
            Ok(format!("Removed member '{}'", member))
        }),
        TeamAction::SetMemberModel { member, model } => {
            let Some(team) = resolve_team_name(state) else {
                state.toast = Some("No team selected".to_string());
                return vec![];
            };
            mutate_team(state, &team, |t| {
                t.set_member_model(&member, &model)?;
                Ok(format!(
                    "Model for '{}' → {}",
                    member,
                    if model.is_empty() { "inherit" } else { &model }
                ))
            })
        }
    }
}

/// Clone a team out of the store, apply a pure mutation, and emit the
/// persistence effect. The store copy is updated optimistically so the UI
/// reflects the change before the next refresh lands.
fn mutate_team(
    state: &mut AppState,
    name: &str,
    change: impl FnOnce(&mut crate::domain::entities::Team) -> std::result::Result<String, String>,
) -> Vec<Effect> {
    let Some(team) = state.store.teams.iter_mut().find(|t| t.name == name) else {
        state.toast = Some(format!("Team '{}' not found", name));
        return vec![];
    };
    let mut updated = team.clone();
    match change(&mut updated) {
        Ok(message) => {
            *team = updated.clone();
            state.toast = Some(message);
            vec![Effect::UpdateTeam { team: updated }, Effect::RefreshAll]
        }
        Err(e) => {
            state.toast = Some(e);
            vec![]
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

/// The scratch entry currently under the table selection, accounting for the
/// collapsed-folder filtering (selection indexes the *visible* tree rows).
fn selected_scratch_note(state: &AppState) -> Option<&ScratchNote> {
    let visible = visible_scratch_indices(&state.store.scratch_notes, &state.expanded_scratch_dirs);
    visible
        .get(state.table_state.selected)
        .and_then(|&i| state.store.scratch_notes.get(i))
}

/// Folder a newly created entry should land in, from the current selection:
/// inside the selected folder, alongside the selected file, else the root.
fn scratch_target_parent(state: &AppState) -> String {
    match selected_scratch_note(state) {
        Some(n) if n.is_dir => n.id.clone(),
        Some(n) => n.parent.clone(),
        None => String::new(),
    }
}

/// Candidate destination folders for moving the scratch entry `note`, as
/// picker items. Includes the root and every folder, minus the entry itself,
/// its descendants (can't move a folder into its own subtree), and its current
/// parent (a no-op move). The item `value` is the destination's relative path
/// (`""` = root); folders keep tree order, root sorts first.
fn scratch_move_targets(
    notes: &[ScratchNote],
    note: &ScratchNote,
) -> Vec<crate::application::state::PickerItem> {
    let self_prefix = format!("{}/", note.id);
    let mut items = Vec::new();
    // Root is a valid target unless the entry already lives there.
    if !note.parent.is_empty() {
        items.push(crate::application::state::PickerItem {
            label: "/ (root)".to_string(),
            description: "Move to the top level".to_string(),
            value: String::new(),
        });
    }
    for folder in notes.iter().filter(|n| n.is_dir) {
        // Skip the entry itself and anything inside it (cycle), and its
        // current parent (moving there would change nothing).
        if folder.id == note.id || folder.id.starts_with(&self_prefix) || folder.id == note.parent {
            continue;
        }
        items.push(crate::application::state::PickerItem {
            label: format!("{}/", folder.id),
            description: String::new(),
            value: folder.id.clone(),
        });
    }
    items
}

/// Expand `dir` and every ancestor folder so a freshly created descendant is
/// visible in the tree. No-op for the root (`""`).
fn expand_scratch_ancestors(state: &mut AppState, dir: &str) {
    let mut acc = String::new();
    for seg in dir.split('/').filter(|s| !s.is_empty()) {
        if acc.is_empty() {
            acc = seg.to_string();
        } else {
            acc = format!("{}/{}", acc, seg);
        }
        state.expanded_scratch_dirs.insert(acc.clone());
    }
}

/// Clamp the table selection to the number of currently-visible scratch rows.
fn clamp_scratch_selection(state: &mut AppState) {
    let count =
        visible_scratch_indices(&state.store.scratch_notes, &state.expanded_scratch_dirs).len();
    if count == 0 {
        state.table_state.selected = 0;
    } else if state.table_state.selected >= count {
        state.table_state.selected = count - 1;
    }
}

fn reduce_scratch(state: &mut AppState, action: ScratchAction) -> Vec<Effect> {
    match action {
        ScratchAction::Create { parent, title } => {
            let title = title.trim().to_string();
            if title.is_empty() {
                state.toast = Some("Scratch title cannot be empty".to_string());
                return vec![];
            }
            // Make the destination visible so the new note shows after refresh.
            expand_scratch_ancestors(state, &parent);
            state.toast = Some(format!("Created scratch '{}'", title));
            vec![
                Effect::CreateScratchNote { parent, title },
                Effect::RefreshScratchNotes,
            ]
        }
        ScratchAction::CreateDir { parent, name } => {
            let name = name.trim().to_string();
            if name.is_empty() {
                state.toast = Some("Folder name cannot be empty".to_string());
                return vec![];
            }
            expand_scratch_ancestors(state, &parent);
            // Expand the new folder itself so it's ready to receive items.
            let new_id = if parent.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", parent, name)
            };
            state.expanded_scratch_dirs.insert(new_id);
            state.toast = Some(format!("Created folder '{}'", name));
            vec![
                Effect::CreateScratchDir { parent, name },
                Effect::RefreshScratchNotes,
            ]
        }
        ScratchAction::Rename { id, new_name } => {
            let new_name = new_name.trim().to_string();
            if new_name.is_empty() {
                state.toast = Some("Name cannot be empty".to_string());
                return vec![];
            }
            state.toast = Some("Renamed".to_string());
            vec![
                Effect::RenameScratch { id, new_name },
                Effect::RefreshScratchNotes,
            ]
        }
        ScratchAction::Move { id, new_parent } => {
            // Expand the destination (and its ancestors) so the moved entry is
            // visible after the refresh rebuilds the tree. The actual move and
            // its cycle/collision checks happen in the backend.
            expand_scratch_ancestors(state, &new_parent);
            if !new_parent.is_empty() {
                state.expanded_scratch_dirs.insert(new_parent.clone());
            }
            let dest = if new_parent.is_empty() {
                "root".to_string()
            } else {
                new_parent.clone()
            };
            state.toast = Some(format!("Moved to {}", dest));
            vec![
                Effect::MoveScratch { id, new_parent },
                Effect::RefreshScratchNotes,
            ]
        }
        ScratchAction::Delete { id } => {
            // Drop the entry (and, for a folder, its whole subtree) from the
            // in-memory list immediately so rows disappear before the next
            // refresh lands; drop any expansion state and clamp the selection.
            let prefix = format!("{}/", id);
            state
                .store
                .scratch_notes
                .retain(|n| n.id != id && !n.id.starts_with(&prefix));
            state
                .expanded_scratch_dirs
                .retain(|d| *d != id && !d.starts_with(&prefix));
            clamp_scratch_selection(state);
            state.toast = Some("Deleted".to_string());
            vec![
                Effect::DeleteScratchNote { id },
                Effect::RefreshScratchNotes,
            ]
        }
        ScratchAction::OpenInEditor { id } => {
            // A folder toggles; a file resolves its path, then reuses the
            // IDE/editor picker — terminal editors (vim/emacs/nano/…) open in a
            // pane, GUI IDEs launch detached (same as "open project in IDE").
            let found = state
                .store
                .scratch_notes
                .iter()
                .find(|n| n.id == id)
                .map(|n| (n.is_dir, n.path.clone()));
            match found {
                Some((true, _)) => reduce_scratch(state, ScratchAction::ToggleDir { id }),
                Some((false, path)) if !path.is_empty() => {
                    vec![Effect::DetectEditors { path }]
                }
                Some((false, _)) => {
                    state.toast = Some("Note has no file path".to_string());
                    vec![]
                }
                None => {
                    state.toast = Some("Note not found".to_string());
                    vec![]
                }
            }
        }
        ScratchAction::ToggleDir { id } => {
            if !state.expanded_scratch_dirs.remove(&id) {
                state.expanded_scratch_dirs.insert(id);
            }
            clamp_scratch_selection(state);
            vec![]
        }
    }
}

fn reduce_agent(state: &mut AppState, action: AgentAction) -> Vec<Effect> {
    match action {
        AgentAction::Attach { session_id } => {
            // Auto-unstash if the session is stashed — attaching implicitly
            // means the user wants it running. Write the status file so the
            // refresh pipeline's hook_says_idle check doesn't block the
            // daemon from updating to Running.
            let was_stashed = mark_session_starting(state, &session_id);

            // Attach via daemon — leaves alternate screen for direct passthrough
            state.input_mode = InputMode::Attached;
            state.attached_session = Some(session_id.clone());
            state.spinner = Some("Attaching...".to_string());

            state.scroll_state.offset = 0;
            let mut effects = vec![];
            if was_stashed {
                effects.push(Effect::MarkSessionStarting {
                    session_id: session_id.clone(),
                });
            }
            effects.push(Effect::DaemonAttach {
                session_id,
                args: vec![],
                cwd: None,
                name: None,
            });
            effects
        }
        AgentAction::SpawnSession { cwd, name } => {
            let session_id = uuid::Uuid::now_v7().to_string();
            state.input_mode = InputMode::Attached;
            state.attached_session = Some(session_id.clone());

            let session_name = name.unwrap_or_else(|| default_session_name(&cwd, &session_id));

            // Check for preset with setup scripts
            let pending_preset = state.pending_session.take().and_then(|p| p.preset);
            let setup_scripts = pending_preset
                .as_ref()
                .map(|p| p.setup.clone())
                .unwrap_or_default();

            state.spinner = Some(format!("Starting session {}...", session_name));
            state.scroll_state.offset = 0;
            let mut effects = vec![
                Effect::RegisterSession {
                    session_id: session_id.clone(),
                    name: session_name.clone(),
                    cwd: cwd.clone(),
                    source_branch: None,
                },
                Effect::DaemonAttach {
                    session_id: session_id.clone(),
                    args: vec!["--session-id".to_string(), session_id.clone()],
                    cwd: Some(cwd.clone()),
                    name: Some(session_name),
                },
            ];
            if !setup_scripts.is_empty() {
                effects.push(Effect::RunSetupScripts {
                    session_id,
                    scripts: setup_scripts,
                    cwd,
                });
            }
            effects
        }
        AgentAction::DropSession { session_id } => {
            // Check if the session has a preset with teardown scripts
            let session = state.store.find_session(&session_id).cloned();
            if let Some(ref s) = session {
                if let Some(ref preset_name) = s.preset_name {
                    if let Some(preset) =
                        state.store.presets.iter().find(|p| &p.name == preset_name)
                    {
                        if !preset.teardown.is_empty() {
                            let cwd = s
                                .cwd
                                .as_deref()
                                .or(Some(&s.project_path))
                                .filter(|p| !p.is_empty())
                                .unwrap_or(".")
                                .to_string();
                            state.spinner = Some("Running teardown...".to_string());
                            return vec![Effect::RunTeardownScripts {
                                scripts: preset.teardown.clone(),
                                cwd,
                                on_complete: Action::Agent(AgentAction::DropSessionAfterTeardown {
                                    session_id,
                                }),
                            }];
                        }
                    }
                }
            }

            // No teardown — proceed with immediate drop
            state.spinner = Some("Dropping session...".to_string());
            let worktree = session.as_ref().and_then(|s| s.worktree.clone());
            // For wild/synthetic rows the only signal that actually
            // terminates the bare claude is its PID — the existing
            // session-id-keyed pgrep finds nothing for `wild-pid-<pid>`.
            let wild_pid = session.as_ref().and_then(|s| s.wild_pid);
            // Remove from store immediately so the UI doesn't show a stale entry
            state.store.sessions.retain(|s| s.id != session_id);
            clamp_session_selection(state);
            state.nav.pop();
            vec![
                Effect::UnregisterSession {
                    session_id: session_id.clone(),
                },
                Effect::MarkSessionIdle {
                    session_id: session_id.clone(),
                },
                Effect::DaemonKill {
                    session_id: session_id.clone(),
                },
                Effect::TerminateProcess {
                    session_id: session_id.clone(),
                    worktree,
                    wild_pid,
                },
                Effect::RefreshSessions,
            ]
        }
        AgentAction::DropAllSessions => {
            state.spinner = Some("Dropping all sessions...".to_string());
            state.store.sessions.clear();
            state.table_state.selected = 0;
            vec![
                Effect::ClearSessionRegistry,
                Effect::DaemonKillAll,
                Effect::TerminateAllProcesses,
                Effect::RefreshSessions,
            ]
        }
        AgentAction::SpawnInWorktree { session_id } => {
            let session = state.store.find_session(&session_id).cloned();
            match session {
                Some(s) if s.worktree.is_some() => {
                    // Already in a worktree — just attach
                    reduce_agent(state, AgentAction::Attach { session_id })
                }
                Some(s) if s.project_path.is_empty() => {
                    state.toast = Some("Session has no project path".to_string());
                    vec![]
                }
                Some(_s) => {
                    let new_session_id = uuid::Uuid::now_v7().to_string();
                    let short = &new_session_id[..8];
                    let name = format!("wt-{}", short);
                    state.input_mode = InputMode::Attached;
                    state.attached_session = Some(new_session_id.clone());
                    state.spinner = Some(format!("Creating worktree {}...", name));
                    state.scroll_state.offset = 0;
                    vec![Effect::CreateWorktreeAndAttach {
                        source_session_id: Some(session_id),
                        cwd: None,
                        new_session_id,
                        name,
                    }]
                }
                None => {
                    state.toast = Some("Session not found".to_string());
                    vec![]
                }
            }
        }
        AgentAction::StashSession { session_id } => {
            let session = state.store.find_session(&session_id).cloned();
            match session {
                Some(s)
                    if s.status == crate::domain::entities::SessionStatus::Stashed
                        && !s.is_running =>
                {
                    // Unstash: restart in the background (don't attach)
                    mark_session_starting(state, &session_id);
                    state.spinner = Some("Starting session...".to_string());
                    state.pending_toast = Some("Session starting...".to_string());
                    vec![
                        Effect::MarkSessionStarting {
                            session_id: session_id.clone(),
                        },
                        Effect::DaemonStart {
                            session_id,
                            args: vec![],
                            cwd: None,
                            name: None,
                        },
                        Effect::RefreshSessions,
                    ]
                }
                Some(s) => {
                    // Stash: kill daemon PTY, terminate process, mark idle
                    let worktree = s.worktree.clone();
                    let wild_pid = s.wild_pid;
                    // Update in-memory state immediately so the UI reflects the change
                    if let Some(session) =
                        state.store.sessions.iter_mut().find(|x| x.id == session_id)
                    {
                        session.status = crate::domain::entities::SessionStatus::Stashed;
                        session.is_running = false;
                    }
                    clamp_session_selection(state);
                    state.spinner = Some("Stashing session...".to_string());
                    state.pending_toast = Some("Session stashed".to_string());
                    vec![
                        Effect::DaemonKill {
                            session_id: session_id.clone(),
                        },
                        Effect::TerminateProcess {
                            session_id: session_id.clone(),
                            worktree,
                            wild_pid,
                        },
                        Effect::MarkSessionIdle { session_id },
                        Effect::RefreshSessions,
                    ]
                }
                None => {
                    state.toast = Some("Session not found".to_string());
                    vec![]
                }
            }
        }
        AgentAction::StashAllSessions => {
            let running: Vec<_> = state
                .store
                .sessions
                .iter()
                .filter(|s| s.is_running)
                .map(|s| s.id.clone())
                .collect();
            let idle: Vec<_> = state
                .store
                .sessions
                .iter()
                .filter(|s| {
                    s.status == crate::domain::entities::SessionStatus::Stashed && !s.is_running
                })
                .map(|s| s.id.clone())
                .collect();

            if !running.is_empty() {
                // Stash all running sessions
                for session in state.store.sessions.iter_mut().filter(|s| s.is_running) {
                    session.status = crate::domain::entities::SessionStatus::Stashed;
                    session.is_running = false;
                }
                clamp_session_selection(state);
                state.spinner = Some(format!(
                    "Stashing {} session{}...",
                    running.len(),
                    if running.len() == 1 { "" } else { "s" }
                ));
                state.pending_toast = Some(format!(
                    "{} session{} stashed",
                    running.len(),
                    if running.len() == 1 { "" } else { "s" }
                ));
                // Capture all IDs (running + already stashed) so the marker
                // protects every session if the app crashes before status
                // files are re-written.
                let session_ids: Vec<String> =
                    state.store.sessions.iter().map(|s| s.id.clone()).collect();
                vec![
                    Effect::WriteQuitStash { session_ids },
                    Effect::DaemonKillAll,
                    Effect::TerminateAllProcesses,
                    Effect::MarkAllSessionsIdle,
                    Effect::RefreshSessions,
                ]
            } else if !idle.is_empty() {
                // Unstash all idle sessions
                for id in &idle {
                    mark_session_starting(state, id);
                }
                let mut effects = Vec::new();
                for id in &idle {
                    effects.push(Effect::MarkSessionStarting {
                        session_id: id.clone(),
                    });
                    effects.push(Effect::DaemonStart {
                        session_id: id.clone(),
                        args: vec![],
                        cwd: None,
                        name: None,
                    });
                }
                effects.push(Effect::RefreshSessions);
                state.spinner = Some(format!(
                    "Starting {} session{}...",
                    idle.len(),
                    if idle.len() == 1 { "" } else { "s" }
                ));
                state.pending_toast = Some(format!(
                    "{} session{} starting...",
                    idle.len(),
                    if idle.len() == 1 { "" } else { "s" }
                ));
                effects
            } else {
                state.toast = Some("No sessions to stash or unstash".to_string());
                vec![]
            }
        }
        AgentAction::SpawnSessionInWorktree { cwd, name } => {
            let new_session_id = uuid::Uuid::now_v7().to_string();
            let short = &new_session_id[..8];
            let session_name = name.unwrap_or_else(|| format!("wt-{}", short));
            state.input_mode = InputMode::Attached;
            state.attached_session = Some(new_session_id.clone());
            state.spinner = Some(format!("Creating worktree {}...", session_name));
            state.scroll_state.offset = 0;
            vec![Effect::CreateWorktreeAndAttach {
                source_session_id: None,
                cwd: Some(cwd),
                new_session_id,
                name: session_name,
            }]
        }
        AgentAction::TakeoverWild { session_id } => {
            // Re-validate with a fresh PID — the wild scan keeps
            // `wild_pid` current, so the PID read here may be newer
            // (and is never staler) than the one shown in the confirm
            // dialog. If the row's source changed since the confirm
            // opened (e.g. the daemon picked it up), refuse: takeover
            // would be a no-op or worse.
            let session = state.store.find_session(&session_id).cloned();
            match session {
                Some(s) => match s.takeover_pid() {
                    Ok(pid) => {
                        // Kill the outside process, then chain the
                        // regular attach flow — `DaemonAttach` resumes
                        // the session under the daemon (`--resume`).
                        let mut effects = vec![Effect::KillWildProcess { pid }];
                        effects.extend(reduce_agent(state, AgentAction::Attach { session_id }));
                        effects
                    }
                    Err(reason) => {
                        state.toast = Some(reason.to_string());
                        vec![]
                    }
                },
                None => {
                    state.toast = Some("Session not found".to_string());
                    vec![]
                }
            }
        }
        AgentAction::OpenInIde { session_id } => {
            // Resolve session ID: use the explicit ID, or fall back to nav context
            let resolved_id = if session_id.is_empty() {
                state.current_session().map(|s| s.to_string())
            } else {
                Some(session_id)
            };
            let session = resolved_id.and_then(|id| state.store.find_session(&id).cloned());
            match session {
                Some(s) => {
                    let project_dir = s
                        .cwd
                        .as_deref()
                        .filter(|c| !c.is_empty())
                        .or(Some(s.project_path.as_str()).filter(|p| !p.is_empty()))
                        .unwrap_or("")
                        .to_string();
                    if project_dir.is_empty() {
                        state.toast = Some("No project directory for this session".to_string());
                        vec![]
                    } else {
                        vec![Effect::DetectIdes { project_dir }]
                    }
                }
                None => {
                    state.toast = Some("Session not found".to_string());
                    vec![]
                }
            }
        }
        AgentAction::AttachNewWindow { session_id } => {
            if state.externally_opened.contains(&session_id) {
                state.toast = Some("Session already open externally".to_string());
                vec![]
            } else {
                vec![Effect::AttachInNewWindow { session_id }]
            }
        }
        AgentAction::RenameSession { session_id, name } => {
            // Resolve session ID: if empty, use the current session from nav context
            let resolved_id = if session_id.is_empty() {
                match state.current_session() {
                    Some(id) => id.to_string(),
                    None => {
                        state.toast =
                            Some("No session selected (use from session detail)".to_string());
                        return vec![];
                    }
                }
            } else {
                session_id
            };
            // Update in-memory session name and re-sort so the list reflects the
            // new alphabetical position immediately (merge_sessions won't detect a
            // change if the incoming name already matches, skipping its re-sort).
            if let Some(session) = state
                .store
                .sessions
                .iter_mut()
                .find(|s| s.id == resolved_id)
            {
                session.name = Some(name.clone());
            }
            state.store.sort_sessions();
            state.toast = Some(format!("Renamed to '{}'", name));
            vec![Effect::RenameSession {
                session_id: resolved_id,
                name,
            }]
        }
        AgentAction::SpawnSessionFromPreset { preset_name } => {
            // Look up preset in cache
            let exists = state.store.presets.iter().any(|p| p.name == preset_name);
            if exists {
                return handle_preset_selection(state, &preset_name, &state.default_cwd.clone());
            }
            // Not found — toast available presets
            let available: Vec<String> =
                state.store.presets.iter().map(|p| p.name.clone()).collect();
            if available.is_empty() {
                state.toast = Some(format!(
                    "Unknown preset '{}'. No presets loaded.",
                    preset_name
                ));
            } else {
                state.toast = Some(format!(
                    "Unknown preset '{}'. Available: {}",
                    preset_name,
                    available.join(", ")
                ));
            }
            vec![]
        }
        AgentAction::DropSessionAfterTeardown { session_id } => {
            // Two-phase drop: teardown has completed — now do the actual kill/unregister
            state.spinner = Some("Dropping session...".to_string());
            let session = state.store.find_session(&session_id);
            let worktree = session.and_then(|s| s.worktree.clone());
            let wild_pid = session.and_then(|s| s.wild_pid);
            state.store.sessions.retain(|s| s.id != session_id);
            clamp_session_selection(state);
            state.nav.pop();
            vec![
                Effect::UnregisterSession {
                    session_id: session_id.clone(),
                },
                Effect::MarkSessionIdle {
                    session_id: session_id.clone(),
                },
                Effect::DaemonKill {
                    session_id: session_id.clone(),
                },
                Effect::TerminateProcess {
                    session_id: session_id.clone(),
                    worktree,
                    wild_pid,
                },
                Effect::RefreshSessions,
            ]
        }
        AgentAction::AttachAllNewWindows => {
            let ids: Vec<String> = state
                .filtered_sessions()
                .iter()
                .filter(|s| s.is_running && !state.externally_opened.contains(&s.id))
                .map(|s| s.id.clone())
                .collect();
            if ids.is_empty() {
                state.toast = Some("No running sessions (or all already open)".to_string());
                vec![]
            } else {
                vec![Effect::AttachBatchInNewWindows { session_ids: ids }]
            }
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
            state.spinner = Some("Updating clash...".to_string());
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
            state.confirm_dialog = Some(ConfirmDialog {
                message,
                on_confirm: *on_confirm,
            });
            state.input_mode = InputMode::Confirm;
            vec![]
        }
        UiAction::ConfirmYes => {
            state.input_mode = InputMode::Normal;
            if let Some(dialog) = state.confirm_dialog.take() {
                return reduce(state, dialog.on_confirm);
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
            state.input = tui_input::Input::default();
            vec![]
        }
        UiAction::EnterFilterMode => {
            state.input_mode = InputMode::Filter;
            state.input = tui_input::Input::default();
            vec![]
        }
        UiAction::EnterNewSessionMode => {
            if !state.store.presets.is_empty() {
                // Show preset picker
                let mut items: Vec<crate::application::state::PickerItem> = state
                    .store
                    .presets
                    .iter()
                    .map(|p| crate::application::state::PickerItem {
                        label: p.name.clone(),
                        description: if p.description.is_empty() {
                            match p.source {
                                crate::domain::entities::PresetSource::Global => {
                                    "(global)".to_string()
                                }
                                crate::domain::entities::PresetSource::Superset => {
                                    "From .superset/config.json".to_string()
                                }
                                _ => String::new(),
                            }
                        } else {
                            p.description.clone()
                        },
                        value: p.name.clone(),
                    })
                    .collect();
                items.push(crate::application::state::PickerItem {
                    label: "No preset".to_string(),
                    description: "Manual setup".to_string(),
                    value: String::new(),
                });
                return reduce(
                    state,
                    Action::Ui(UiAction::ShowPicker {
                        title: "Select Preset".to_string(),
                        items,
                        on_select: crate::application::state::PickerAction::SelectPreset {
                            project_dir: state.default_cwd.clone(),
                        },
                    }),
                );
            }
            // No presets — enter manual 3-step flow
            state.input_mode = InputMode::NewSession;
            state.input = tui_input::Input::new(state.default_cwd.clone());
            vec![]
        }
        UiAction::EnterNewScratchMode => {
            state.scratch_op_target = Some(scratch_target_parent(state));
            state.input_mode = InputMode::NewScratchTitle;
            state.input = tui_input::Input::default();
            vec![]
        }
        UiAction::EnterNewScratchDirMode => {
            state.scratch_op_target = Some(scratch_target_parent(state));
            state.input_mode = InputMode::NewScratchDir;
            state.input = tui_input::Input::default();
            vec![]
        }
        UiAction::EnterRenameScratchMode => {
            // Pre-fill with the entry's current name (folder name or file name).
            let info = selected_scratch_note(state).map(|n| {
                let current = std::path::Path::new(&n.id)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| n.title.clone());
                (n.id.clone(), current)
            });
            match info {
                Some((id, current)) => {
                    state.scratch_op_target = Some(id);
                    state.input_mode = InputMode::RenameScratch;
                    state.input = tui_input::Input::new(current);
                }
                None => state.toast = Some("Nothing selected".to_string()),
            }
            vec![]
        }
        UiAction::EnterMoveScratchMode => {
            let Some(note) = selected_scratch_note(state).cloned() else {
                state.toast = Some("Nothing selected".to_string());
                return vec![];
            };
            let items = scratch_move_targets(&state.store.scratch_notes, &note);
            if items.is_empty() {
                state.toast = Some(format!("Nowhere to move '{}'", note.title));
                return vec![];
            }
            state.picker_dialog = Some(crate::application::state::PickerDialog {
                title: format!("Move '{}' to…", note.title),
                items,
                selected: 0,
                on_select_action: crate::application::state::PickerAction::MoveScratch {
                    id: note.id.clone(),
                },
            });
            state.input_mode = InputMode::Picker;
            vec![]
        }
        UiAction::EnterCopyScratchPathMode => {
            let Some(note) = selected_scratch_note(state).cloned() else {
                state.toast = Some("Nothing selected".to_string());
                return vec![];
            };
            // File name with extension (folder name for folders), from the
            // absolute path; fall back to the display title.
            let file_name = std::path::Path::new(&note.path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| note.title.clone());
            let mut items = Vec::new();
            if !note.path.is_empty() {
                items.push(crate::application::state::PickerItem {
                    label: "Absolute path".to_string(),
                    description: note.path.clone(),
                    value: note.path.clone(),
                });
            }
            items.push(crate::application::state::PickerItem {
                label: "Relative path (from scratch root)".to_string(),
                description: note.id.clone(),
                value: note.id.clone(),
            });
            items.push(crate::application::state::PickerItem {
                label: "File name".to_string(),
                description: file_name.clone(),
                value: file_name,
            });
            state.picker_dialog = Some(crate::application::state::PickerDialog {
                title: format!("Copy path of '{}'", note.title),
                items,
                selected: 0,
                on_select_action: crate::application::state::PickerAction::CopyToClipboard,
            });
            state.input_mode = InputMode::Picker;
            vec![]
        }
        UiAction::EditTeamDescription => {
            let Some(name) = resolve_team_name(state) else {
                state.toast = Some("No team selected".to_string());
                return vec![];
            };
            let description = state
                .store
                .teams
                .iter()
                .find(|t| t.name == name)
                .map(|t| t.description.clone())
                .unwrap_or_default();
            state.pending_team_edit = Some(name);
            state.input_mode = InputMode::TeamDescription;
            state.input = tui_input::Input::new(description);
            vec![]
        }
        UiAction::AddTeamMember => {
            let Some(team) = resolve_team_name(state) else {
                state.toast = Some("No team selected".to_string());
                return vec![];
            };
            state.pending_member = Some(crate::application::state::PendingMember {
                team,
                name: String::new(),
                agent_type: String::new(),
            });
            state.input_mode = InputMode::NewMemberName;
            state.input = tui_input::Input::default();
            vec![]
        }
        UiAction::RemoveTeamMember => {
            let Some(name) = resolve_team_name(state) else {
                state.toast = Some("No team selected".to_string());
                return vec![];
            };
            let items: Vec<crate::application::state::PickerItem> = state
                .store
                .teams
                .iter()
                .find(|t| t.name == name)
                .map(|t| {
                    t.members
                        .iter()
                        .map(|m| crate::application::state::PickerItem {
                            label: m.name.clone(),
                            description: format!(
                                "{} · {}",
                                m.agent_type,
                                if m.model.is_empty() {
                                    "inherit"
                                } else {
                                    m.model.as_str()
                                }
                            ),
                            value: m.name.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if items.is_empty() {
                state.toast = Some(format!("Team '{}' has no members", name));
                return vec![];
            }
            state.picker_dialog = Some(crate::application::state::PickerDialog {
                title: format!("Remove member from '{}'", name),
                items,
                selected: 0,
                on_select_action: crate::application::state::PickerAction::RemoveTeamMember {
                    team: name,
                },
            });
            state.input_mode = InputMode::Picker;
            vec![]
        }
        UiAction::ExitInputMode => {
            state.input_mode = InputMode::Normal;
            state.input = tui_input::Input::default();
            state.pending_session = None;
            state.pending_team_edit = None;
            state.pending_member = None;
            state.scratch_op_target = None;
            vec![]
        }
        UiAction::SubmitInput(text) => {
            let input = if text.is_empty() {
                state.input.value().to_string()
            } else {
                text
            };
            let mode = state.input_mode.clone();
            state.input_mode = InputMode::Normal;
            state.input = tui_input::Input::default();

            match mode {
                InputMode::Command => {
                    let action = parse_command(&input);
                    reduce(state, action)
                }
                InputMode::Filter => {
                    state.filter = input;
                    // The filter may have shrunk the list past the cursor —
                    // land on the first match instead of an out-of-range
                    // index (which renders as a stuck/header-row highlight).
                    state.table_state.selected = 0;
                    vec![]
                }
                InputMode::NewSession => {
                    let cwd = input.trim().to_string();
                    let cwd = if cwd.is_empty() {
                        state.default_cwd.clone()
                    } else {
                        cwd
                    };
                    let uid = uuid::Uuid::now_v7().to_string();
                    let default_name = default_session_name(&cwd, &uid);
                    state.pending_session = Some(crate::application::state::PendingSession {
                        cwd,
                        name: None,
                        worktree: false,
                        preset: None,
                    });
                    state.input_mode = InputMode::NewSessionName;
                    state.input = tui_input::Input::new(default_name);
                    vec![]
                }
                InputMode::NewSessionName => {
                    let name_input = input.trim().to_string();
                    if let Some(ref mut pending) = state.pending_session {
                        pending.name = if name_input.is_empty() {
                            None
                        } else {
                            Some(name_input)
                        };
                    }
                    state.input_mode = InputMode::NewSessionWorktree;
                    state.input = tui_input::Input::new("n".to_string());
                    vec![]
                }
                InputMode::TeamDescription => {
                    let Some(name) = state.pending_team_edit.take() else {
                        return vec![];
                    };
                    reduce(
                        state,
                        Action::Team(TeamAction::SetDescription {
                            name,
                            description: input,
                        }),
                    )
                }
                InputMode::NewScratchTitle => {
                    let parent = state.scratch_op_target.take().unwrap_or_default();
                    reduce(
                        state,
                        Action::Scratch(ScratchAction::Create {
                            parent,
                            title: input,
                        }),
                    )
                }
                InputMode::NewScratchDir => {
                    let parent = state.scratch_op_target.take().unwrap_or_default();
                    reduce(
                        state,
                        Action::Scratch(ScratchAction::CreateDir {
                            parent,
                            name: input,
                        }),
                    )
                }
                InputMode::RenameScratch => {
                    let Some(id) = state.scratch_op_target.take() else {
                        return vec![];
                    };
                    reduce(
                        state,
                        Action::Scratch(ScratchAction::Rename {
                            id,
                            new_name: input,
                        }),
                    )
                }
                InputMode::NewMemberName => {
                    let name = input.trim().to_string();
                    if name.is_empty() {
                        state.toast = Some("Member name cannot be empty".to_string());
                        state.pending_member = None;
                        return vec![];
                    }
                    if let Some(ref mut pending) = state.pending_member {
                        pending.name = name;
                    }
                    state.input_mode = InputMode::NewMemberType;
                    state.input = tui_input::Input::new("general-purpose".to_string());
                    vec![]
                }
                InputMode::NewMemberType => {
                    if let Some(ref mut pending) = state.pending_member {
                        pending.agent_type = input.trim().to_string();
                    }
                    state.input_mode = InputMode::NewMemberModel;
                    state.input = tui_input::Input::default();
                    vec![]
                }
                InputMode::NewMemberModel => {
                    let Some(pending) = state.pending_member.take() else {
                        return vec![];
                    };
                    reduce(
                        state,
                        Action::Team(TeamAction::AddMember {
                            team: pending.team,
                            name: pending.name,
                            agent_type: pending.agent_type,
                            model: input.trim().to_string(),
                        }),
                    )
                }
                InputMode::NewSessionWorktree => {
                    let wants_worktree = input.trim().eq_ignore_ascii_case("y");
                    let pending = state.pending_session.take();
                    let (cwd, name) = match pending {
                        Some(p) => (p.cwd, p.name),
                        None => (state.default_cwd.clone(), None),
                    };
                    if wants_worktree {
                        reduce(
                            state,
                            Action::Agent(AgentAction::SpawnSessionInWorktree { cwd, name }),
                        )
                    } else {
                        reduce(
                            state,
                            Action::Agent(AgentAction::SpawnSession { cwd, name }),
                        )
                    }
                }
                _ => vec![],
            }
        }
        UiAction::ScrollDown => {
            if state.show_help {
                state.help_scroll = state.help_scroll.saturating_add(1);
            } else if state.current_view() == ViewKind::Diff {
                state.diff.file_scroll = state.diff.file_scroll.saturating_add(3);
            } else {
                state.scroll_state.offset = state.scroll_state.offset.saturating_add(3);
            }
            vec![]
        }
        UiAction::ScrollUp => {
            if state.show_help {
                state.help_scroll = state.help_scroll.saturating_sub(1);
            } else if state.current_view() == ViewKind::Diff {
                state.diff.file_scroll = state.diff.file_scroll.saturating_sub(3);
            } else {
                state.scroll_state.offset = state.scroll_state.offset.saturating_sub(3);
            }
            vec![]
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
        UiAction::CycleSectionFilter => {
            state.section_filter = state.section_filter.next(state.session_filter);
            state.table_state.selected = 0;
            if state.section_filter == crate::application::state::SectionFilter::All {
                state.toast = Some("Showing all sections".to_string());
            } else if state.filtered_sessions().is_empty() {
                state.toast = Some(format!(
                    "No {} sessions — press A to cycle",
                    state.section_filter.label()
                ));
            } else {
                state.toast = Some(format!(
                    "Showing {} sessions only",
                    state.section_filter.label()
                ));
            }
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
        UiAction::ShowPicker {
            title,
            items,
            on_select,
        } => {
            if items.is_empty() {
                state.toast = Some("No IDEs detected".to_string());
                return vec![];
            }
            if items.len() == 1 {
                // Single item — skip picker, emit effect directly
                let item = &items[0];
                state.toast = Some(format!("Opening in {}...", item.label));
                return emit_picker_effect(&on_select, &item.value);
            }
            state.picker_dialog = Some(crate::application::state::PickerDialog {
                title,
                items,
                selected: 0,
                on_select_action: on_select,
            });
            state.input_mode = InputMode::Picker;
            vec![]
        }
        UiAction::PickerUp => {
            if let Some(ref mut picker) = state.picker_dialog {
                picker.selected = picker.selected.saturating_sub(1);
            }
            vec![]
        }
        UiAction::PickerDown => {
            if let Some(ref mut picker) = state.picker_dialog {
                if picker.selected + 1 < picker.items.len() {
                    picker.selected += 1;
                }
            }
            vec![]
        }
        UiAction::PickerSelect => {
            if let Some(picker) = state.picker_dialog.take() {
                state.input_mode = InputMode::Normal;
                if let Some(item) = picker.items.get(picker.selected) {
                    // Handle preset picker specially (needs state access)
                    if let crate::application::state::PickerAction::SelectPreset {
                        ref project_dir,
                    } = picker.on_select_action
                    {
                        return handle_preset_selection(state, &item.value, project_dir);
                    }
                    // Member removal re-enters the reducer (needs state access)
                    if let crate::application::state::PickerAction::RemoveTeamMember { ref team } =
                        picker.on_select_action
                    {
                        let team = team.clone();
                        let member = item.value.clone();
                        return reduce(
                            state,
                            Action::Team(TeamAction::RemoveMember { team, member }),
                        );
                    }
                    // Scratch move re-enters the reducer (needs state access);
                    // the picked item's value is the destination parent path.
                    if let crate::application::state::PickerAction::MoveScratch { ref id } =
                        picker.on_select_action
                    {
                        let id = id.clone();
                        let new_parent = item.value.clone();
                        return reduce(
                            state,
                            Action::Scratch(ScratchAction::Move { id, new_parent }),
                        );
                    }
                    // Copy-path emits a clipboard effect; toast names the format
                    // (item label) rather than the generic "Opening in …".
                    if matches!(
                        picker.on_select_action,
                        crate::application::state::PickerAction::CopyToClipboard
                    ) {
                        state.toast = Some(format!("Copied {}", item.label.to_lowercase()));
                        return vec![Effect::CopyToClipboard {
                            text: item.value.clone(),
                        }];
                    }
                    state.toast = Some(format!("Opening in {}...", item.label));
                    return emit_picker_effect(&picker.on_select_action, &item.value);
                }
            }
            vec![]
        }
        UiAction::PickerCancel => {
            state.picker_dialog = None;
            state.input_mode = InputMode::Normal;
            vec![]
        }
        UiAction::RefreshDiff => {
            if let Some(ref session_id) = state.diff.session_id.clone() {
                if !state.diff.loading {
                    state.diff.loading = true;
                    return vec![Effect::LoadDiff {
                        session_id: session_id.clone(),
                    }];
                }
            }
            vec![]
        }
        UiAction::DiffNextFile => {
            if !state.diff.files.is_empty() {
                let max = state.diff.files.len() - 1;
                state.diff.selected_file = (state.diff.selected_file + 1).min(max);
                state.diff.file_scroll = 0;
            }
            vec![]
        }
        UiAction::DiffPrevFile => {
            state.diff.selected_file = state.diff.selected_file.saturating_sub(1);
            state.diff.file_scroll = 0;
            vec![]
        }
        UiAction::Tick => {
            state.tick = state.tick.wrapping_add(1);
            if state.toast.is_some() && state.tick.is_multiple_of(300) {
                state.toast = None;
            }
            // Auto-refresh diff every ~3s (12 ticks at 250ms) when session is active
            let mut effects = check_shutdown(state);
            if state.current_view() == ViewKind::Diff
                && !state.diff.loading
                && state.tick.is_multiple_of(12)
            {
                if let Some(ref session_id) = state.diff.session_id.clone() {
                    let is_active = state
                        .store
                        .find_session(session_id)
                        .map(|s| s.is_running)
                        .unwrap_or(false);
                    if is_active {
                        state.diff.loading = true;
                        effects.push(Effect::LoadDiff {
                            session_id: session_id.clone(),
                        });
                    }
                }
            }
            effects
        }
        UiAction::Quit => {
            let running = state.store.sessions.iter().filter(|s| s.is_running).count();
            let message = if running > 0 {
                format!(
                    "Quit? {} running session{} will be stashed.",
                    running,
                    if running == 1 { "" } else { "s" }
                )
            } else {
                "Are you sure you want to quit?".to_string()
            };
            state.confirm_dialog = Some(crate::application::state::ConfirmDialog {
                message,
                on_confirm: Action::Ui(UiAction::QuitConfirmed),
            });
            state.input_mode = InputMode::Confirm;
            vec![]
        }
        UiAction::QuitConfirmed => {
            let running = state.store.sessions.iter().filter(|s| s.is_running).count();
            if running == 0 {
                vec![Effect::Quit]
            } else {
                state.shutting_down = Some(state.tick);
                state.confirm_dialog = None;
                state.input_mode = InputMode::Normal;
                state.spinner = Some(shutdown_spinner_msg(running));
                // ⚠️ Effect ordering matters: WriteQuitStash MUST run before
                // DaemonKillAll. Killing daemon sessions triggers SessionExited
                // events → refresh → sessions removed from store. Capturing IDs
                // here (in the reducer, before any effects execute) guarantees
                // the stash marker contains all sessions.
                let session_ids: Vec<String> =
                    state.store.sessions.iter().map(|s| s.id.clone()).collect();
                vec![
                    Effect::WriteQuitStash { session_ids },
                    Effect::MarkAllSessionsIdle,
                    Effect::DaemonKillAll,
                    Effect::TerminateAllProcesses,
                ]
            }
        }
        UiAction::ForceQuit => vec![Effect::Quit],
    }
}

/// Emit the appropriate effect for a picker selection.
/// Handle preset picker selection — either start from preset or fall through to manual.
fn handle_preset_selection(
    state: &mut AppState,
    preset_name: &str,
    project_dir: &str,
) -> Vec<Effect> {
    if preset_name.is_empty() {
        // "No preset" selected — fall through to manual flow
        state.input_mode = InputMode::NewSession;
        state.input = tui_input::Input::new(state.default_cwd.clone());
        return vec![];
    }

    let preset = state
        .store
        .presets
        .iter()
        .find(|p| p.name == preset_name)
        .cloned();

    if let Some(preset) = preset {
        // Resolve directory relative to project_dir
        let cwd = if preset.directory.is_empty() || preset.directory == "." {
            project_dir.to_string()
        } else if std::path::Path::new(&preset.directory).is_absolute() {
            preset.directory.clone()
        } else {
            std::path::Path::new(project_dir)
                .join(&preset.directory)
                .to_string_lossy()
                .to_string()
        };

        let uid = uuid::Uuid::now_v7().to_string();
        let default_name = format!("{}-{}", preset.name, &uid[..8]);

        // If worktree is specified, skip the prompt
        if let Some(wants_worktree) = preset.worktree {
            let name = Some(default_name);
            state.pending_session = Some(crate::application::state::PendingSession {
                cwd: cwd.clone(),
                name: name.clone(),
                worktree: wants_worktree,
                preset: Some(preset),
            });
            let pending = state.pending_session.take().unwrap();
            if pending.worktree {
                return reduce(
                    state,
                    Action::Agent(AgentAction::SpawnSessionInWorktree {
                        cwd: pending.cwd,
                        name: pending.name,
                    }),
                );
            } else {
                return reduce(
                    state,
                    Action::Agent(AgentAction::SpawnSession {
                        cwd: pending.cwd,
                        name: pending.name,
                    }),
                );
            }
        }

        // Store preset in pending and go to name step
        state.pending_session = Some(crate::application::state::PendingSession {
            cwd,
            name: None,
            worktree: false,
            preset: Some(preset),
        });
        state.input_mode = InputMode::NewSessionName;
        state.input = tui_input::Input::new(default_name);
        vec![]
    } else {
        state.toast = Some(format!("Unknown preset '{}'", preset_name));
        vec![]
    }
}

fn emit_picker_effect(
    action: &crate::application::state::PickerAction,
    value: &str,
) -> Vec<Effect> {
    match action {
        crate::application::state::PickerAction::OpenInIde { project_dir } => {
            let terminal_prefix = crate::infrastructure::ide::TERMINAL_VALUE_PREFIX;
            let (command, terminal) = if let Some(cmd) = value.strip_prefix(terminal_prefix) {
                (cmd.to_string(), true)
            } else {
                (value.to_string(), false)
            };
            vec![Effect::OpenIde {
                command,
                project_dir: project_dir.clone(),
                terminal,
            }]
        }
        // SelectPreset, RemoveTeamMember, MoveScratch, and CopyToClipboard are
        // handled directly in PickerSelect before calling this function
        crate::application::state::PickerAction::SelectPreset { .. } => vec![],
        crate::application::state::PickerAction::RemoveTeamMember { .. } => vec![],
        crate::application::state::PickerAction::MoveScratch { .. } => vec![],
        crate::application::state::PickerAction::CopyToClipboard => vec![],
    }
}

/// Resolve the team the user is acting on: the drilled-in team
/// (TeamDetail and deeper) or the selected row in the Teams table.
fn resolve_team_name(state: &AppState) -> Option<String> {
    state.current_team().map(|s| s.to_string()).or_else(|| {
        if state.current_view() == ViewKind::Teams {
            state
                .store
                .teams
                .get(state.table_state.selected)
                .map(|t| t.name.clone())
        } else {
            None
        }
    })
}

/// Clamp the table selection after the filtered session list may have shrunk
/// (session removed, stashed, or moved out of the active section filter).
fn clamp_session_selection(state: &mut AppState) {
    let count = state.filtered_sessions().len();
    if count > 0 && state.table_state.selected >= count {
        state.table_state.selected = count - 1;
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
        ViewKind::Scratch => {
            visible_scratch_indices(&state.store.scratch_notes, &state.expanded_scratch_dirs).len()
        }
        ViewKind::Sessions => state.filtered_sessions().len(),
        ViewKind::Subagents => state.store.all_subagents.len(),
        _ => 0,
    }
}

/// Return effects needed to load data for a view.
fn load_effects_for_view(state: &mut AppState, view: ViewKind) -> Vec<Effect> {
    match view {
        ViewKind::Teams => vec![Effect::RefreshAll],
        ViewKind::Scratch => vec![Effect::RefreshScratchNotes],
        ViewKind::Sessions => vec![
            Effect::RefreshSessions,
            Effect::LoadPresets {
                project_dir: state.default_cwd.clone(),
            },
        ],
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
                // Clone fields out of the borrow before mutating state
                let session_info = state
                    .store
                    .find_session(session_id)
                    .map(|s| (s.project.clone(), s.id.clone()));
                if let Some((project, sid)) = session_info {
                    // Reset conversation state so the view shows "Loading..."
                    state.store.conversation_loaded = false;
                    state.store.conversation.clear();
                    effects.push(Effect::RefreshSubagents {
                        project: project.clone(),
                        session_id: sid.clone(),
                    });
                    effects.push(Effect::LoadConversation {
                        project,
                        session_id: sid.clone(),
                    });
                    effects.push(Effect::LoadRepoConfig { session_id: sid });
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
                // Clone fields out of the borrow before mutating state
                let sa_info = state.store.find_subagent(agent_id).map(|sa| {
                    (
                        sa.parent_session_id.clone(),
                        sa.project.clone(),
                        sa.id.clone(),
                    )
                });
                if let Some((parent_session_id, project, agent_id)) = sa_info {
                    // Reset conversation state so the view shows "Loading..."
                    state.store.conversation_loaded = false;
                    state.store.conversation.clear();
                    return vec![Effect::LoadSubagentConversation {
                        project,
                        session_id: parent_session_id,
                        agent_id,
                    }];
                }
            }
            vec![]
        }
        ViewKind::Diff => {
            let session_id = state.current_session().map(|s| s.to_string());
            if let Some(session_id) = session_id {
                state.diff.loading = true;
                state.diff.loaded = false;
                state.diff.session_id = Some(session_id.clone());
                vec![Effect::LoadDiff { session_id }]
            } else {
                vec![]
            }
        }
        _ => vec![],
    }
}

/// Mark a session as Starting in memory if it was Stashed. Returns true if
/// the session was stashed (and is now Starting). The caller should also emit
/// `Effect::MarkSessionStarting` to write the status file.
fn mark_session_starting(state: &mut AppState, session_id: &str) -> bool {
    if let Some(session) = state.store.sessions.iter_mut().find(|s| s.id == session_id) {
        if session.status == crate::domain::entities::SessionStatus::Stashed {
            session.status = crate::domain::entities::SessionStatus::Starting;
            session.is_running = true;
            return true;
        }
    }
    false
}

/// Spinner message for the shutdown phase.
fn shutdown_spinner_msg(running: usize) -> String {
    format!(
        "Stashing {} session{}...",
        running,
        if running == 1 { "" } else { "s" }
    )
}

/// Check shutdown progress on each tick; returns `Effect::Quit` when all sessions
/// are dead or the timeout (~15 s) has elapsed.
fn check_shutdown(state: &mut AppState) -> Vec<Effect> {
    let Some(start_tick) = state.shutting_down else {
        return vec![];
    };
    let elapsed = state.tick.wrapping_sub(start_tick);
    // Timeout: ~15s = 1500 ticks at 10ms/tick
    if elapsed >= 1500 {
        return vec![Effect::Quit];
    }
    let running = state.store.sessions.iter().filter(|s| s.is_running).count();
    if running == 0 {
        return vec![Effect::Quit];
    }
    // Update spinner message every ~1s
    if elapsed.is_multiple_of(100) {
        state.spinner = Some(shutdown_spinner_msg(running));
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::state::AppState;
    use crate::domain::entities::{Session, SessionStatus};

    fn test_state() -> AppState {
        AppState::new()
    }

    #[test]
    fn test_default_session_name() {
        let id = "019ed532-ade6-73a1-acfb-6a58581065c7";
        // Default is always a short clash- tag, regardless of cwd.
        assert_eq!(
            default_session_name("/Users/x/api-contracts", id),
            "clash-019ed532"
        );
        assert_eq!(default_session_name("/", id), "clash-019ed532");
        assert_eq!(default_session_name("", id), "clash-019ed532");
        // Short id must not panic on the [..8] slice.
        assert_eq!(default_session_name("", "abc"), "clash-abc");
    }

    #[test]
    fn test_clamp_session_selection() {
        let mut state = test_state();
        // is_running so the sessions pass the default Active session filter
        state.store.sessions = vec![
            Session {
                id: "s1".to_string(),
                is_running: true,
                ..Default::default()
            },
            Session {
                id: "s2".to_string(),
                is_running: true,
                ..Default::default()
            },
        ];

        // Selection past the end is pulled back to the last item
        state.table_state.selected = 5;
        clamp_session_selection(&mut state);
        assert_eq!(state.table_state.selected, 1);

        // In-range selection is untouched
        state.table_state.selected = 0;
        clamp_session_selection(&mut state);
        assert_eq!(state.table_state.selected, 0);

        // Empty list leaves the index alone (no underflow)
        state.store.sessions.clear();
        state.table_state.selected = 3;
        clamp_session_selection(&mut state);
        assert_eq!(state.table_state.selected, 3);
    }

    #[test]
    fn test_team_config_actions() {
        let mut state = test_state();
        state.store.teams = vec![crate::domain::entities::Team {
            name: "squad".to_string(),
            description: "old".to_string(),
            ..Default::default()
        }];

        // SetDescription mutates the store copy and emits UpdateTeam.
        let effects = reduce(
            &mut state,
            Action::Team(TeamAction::SetDescription {
                name: "squad".to_string(),
                description: "new".to_string(),
            }),
        );
        assert_eq!(state.store.teams[0].description, "new");
        assert!(matches!(effects[0], Effect::UpdateTeam { ref team } if team.description == "new"));

        // AddMember appends with defaults applied.
        let effects = reduce(
            &mut state,
            Action::Team(TeamAction::AddMember {
                team: "squad".to_string(),
                name: "alice".to_string(),
                agent_type: String::new(),
                model: "sonnet".to_string(),
            }),
        );
        assert_eq!(state.store.teams[0].members.len(), 1);
        assert_eq!(
            state.store.teams[0].members[0].agent_type,
            "general-purpose"
        );
        assert!(matches!(effects[0], Effect::UpdateTeam { .. }));

        // Duplicate member: toast, no effect.
        let effects = reduce(
            &mut state,
            Action::Team(TeamAction::AddMember {
                team: "squad".to_string(),
                name: "alice".to_string(),
                agent_type: String::new(),
                model: String::new(),
            }),
        );
        assert!(effects.is_empty());
        assert!(state.toast.as_deref().unwrap().contains("already exists"));

        // RemoveMember drops it.
        let effects = reduce(
            &mut state,
            Action::Team(TeamAction::RemoveMember {
                team: "squad".to_string(),
                member: "alice".to_string(),
            }),
        );
        assert!(state.store.teams[0].members.is_empty());
        assert!(matches!(effects[0], Effect::UpdateTeam { .. }));

        // Unknown team: toast, no effect.
        let effects = reduce(
            &mut state,
            Action::Team(TeamAction::SetDescription {
                name: "ghost".to_string(),
                description: String::new(),
            }),
        );
        assert!(effects.is_empty());
        assert!(state.toast.as_deref().unwrap().contains("not found"));
    }

    #[test]
    fn test_filter_apply_resets_selection() {
        let mut state = test_state();
        state.store.sessions = vec![
            Session {
                id: "s1".to_string(),
                name: Some("alpha".to_string()),
                is_running: true,
                ..Default::default()
            },
            Session {
                id: "s2".to_string(),
                name: Some("beta".to_string()),
                is_running: true,
                ..Default::default()
            },
        ];
        state.table_state.selected = 1;
        state.input_mode = InputMode::Filter;
        state.input = tui_input::Input::new("alpha".to_string());

        reduce(&mut state, Action::Ui(UiAction::SubmitInput(String::new())));

        // The filter shrank the list to one row — the cursor must land on
        // it, not stay out of range pointing at nothing.
        assert_eq!(state.filter, "alpha");
        assert_eq!(state.filtered_sessions().len(), 1);
        assert_eq!(state.table_state.selected, 0);
    }

    #[test]
    fn test_reduce_noop() {
        let mut state = test_state();
        let effects = reduce(&mut state, Action::Noop);
        assert!(effects.is_empty());
    }

    #[test]
    fn test_reduce_quit_shows_confirm() {
        let mut state = test_state();
        let effects = reduce(&mut state, Action::Ui(UiAction::Quit));
        assert!(effects.is_empty()); // No immediate quit — shows confirm dialog
        assert!(state.confirm_dialog.is_some());
        assert_eq!(state.input_mode, InputMode::Confirm);
    }

    #[test]
    fn test_reduce_quit_confirmed() {
        let mut state = test_state();
        let effects = reduce(&mut state, Action::Ui(UiAction::QuitConfirmed));
        assert!(matches!(effects.first(), Some(Effect::Quit)));
    }

    #[test]
    fn test_quit_confirmed_with_running_sessions() {
        let mut state = test_state();
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
        ];
        let effects = reduce(&mut state, Action::Ui(UiAction::QuitConfirmed));

        // WriteQuitStash must be first with all session IDs
        assert!(matches!(&effects[0], Effect::WriteQuitStash { .. }));
        if let Effect::WriteQuitStash { ref session_ids } = effects[0] {
            assert_eq!(session_ids.len(), 2);
            assert!(session_ids.contains(&"s1".to_string()));
            assert!(session_ids.contains(&"s2".to_string()));
        }
        // MarkAllSessionsIdle before DaemonKillAll
        assert!(matches!(effects[1], Effect::MarkAllSessionsIdle));
        assert!(matches!(effects[2], Effect::DaemonKillAll));
        assert!(matches!(effects[3], Effect::TerminateAllProcesses));
        assert!(state.shutting_down.is_some());
        assert!(state.spinner.is_some());
    }

    #[test]
    fn test_stash_all_sessions_writes_quit_stash() {
        let mut state = test_state();
        state.store.sessions = vec![
            crate::domain::entities::Session {
                id: "run1".to_string(),
                is_running: true,
                status: crate::domain::entities::SessionStatus::Running,
                ..Default::default()
            },
            crate::domain::entities::Session {
                id: "idle1".to_string(),
                is_running: false,
                status: crate::domain::entities::SessionStatus::Stashed,
                ..Default::default()
            },
        ];
        let effects = reduce(
            &mut state,
            Action::Agent(crate::application::actions::AgentAction::StashAllSessions),
        );

        // WriteQuitStash must be first with ALL session IDs (running + stashed)
        assert!(matches!(&effects[0], Effect::WriteQuitStash { .. }));
        if let Effect::WriteQuitStash { ref session_ids } = effects[0] {
            assert_eq!(session_ids.len(), 2);
            assert!(session_ids.contains(&"run1".to_string()));
            assert!(session_ids.contains(&"idle1".to_string()));
        }
        assert!(effects.iter().any(|e| matches!(e, Effect::DaemonKillAll)));
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::TerminateAllProcesses)));
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::MarkAllSessionsIdle)));
        assert!(effects.iter().any(|e| matches!(e, Effect::RefreshSessions)));
    }

    #[test]
    fn test_shutdown_spinner_msg_singular() {
        assert_eq!(shutdown_spinner_msg(1), "Stashing 1 session...");
    }

    #[test]
    fn test_shutdown_spinner_msg_plural() {
        assert_eq!(shutdown_spinner_msg(3), "Stashing 3 sessions...");
    }

    #[test]
    fn test_check_shutdown_not_active() {
        let mut state = test_state();
        assert!(check_shutdown(&mut state).is_empty());
    }

    #[test]
    fn test_check_shutdown_quits_when_all_dead() {
        let mut state = test_state();
        state.shutting_down = Some(0);
        state.tick = 50;
        // No running sessions in store → should quit
        let effects = check_shutdown(&mut state);
        assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
    }

    #[test]
    fn test_check_shutdown_timeout() {
        let mut state = test_state();
        state.shutting_down = Some(0);
        state.tick = 1500;
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        }];
        let effects = check_shutdown(&mut state);
        assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
    }

    #[test]
    fn test_check_shutdown_continues_when_running() {
        let mut state = test_state();
        state.shutting_down = Some(0);
        state.tick = 50; // Not a multiple of 100, not timed out
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        }];
        let effects = check_shutdown(&mut state);
        assert!(effects.is_empty());
    }

    #[test]
    fn test_session_detail_emits_load_repo_config() {
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "test-session".to_string(),
            project: "test-project".to_string(),
            is_running: true,
            ..Default::default()
        }];
        state.session_filter = crate::application::state::SessionFilter::All;
        // Navigate to SessionDetail with context
        let effects = reduce(
            &mut state,
            Action::Nav(NavAction::DrillIn {
                view: ViewKind::SessionDetail,
                context: "test-session".to_string(),
            }),
        );
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::LoadRepoConfig { session_id } if session_id == "test-session"
        )));
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
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::CreateTeam { .. })));
    }

    #[test]
    fn test_reduce_filter_mode() {
        let mut state = test_state();
        reduce(&mut state, Action::Ui(UiAction::EnterFilterMode));
        assert!(matches!(state.input_mode, InputMode::Filter));

        state.input = tui_input::Input::new("test".to_string());
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

    #[test]
    fn test_new_session_two_step_flow() {
        let mut state = test_state();

        // Step 1: Enter new session mode
        reduce(&mut state, Action::Ui(UiAction::EnterNewSessionMode));
        assert_eq!(state.input_mode, InputMode::NewSession);
        assert!(!state.input.value().is_empty()); // pre-filled with default_cwd

        // Simulate typing a path
        state.input = tui_input::Input::new("/tmp/my-project".to_string());

        // Step 2: Submit directory — should transition to NewSessionName
        let effects = reduce(
            &mut state,
            Action::Ui(UiAction::SubmitInput("/tmp/my-project".to_string())),
        );
        assert!(effects.is_empty());
        assert_eq!(state.input_mode, InputMode::NewSessionName);
        // Default name is a short clash- tag (see default_session_name).
        assert!(
            state.input.value().starts_with("clash-"),
            "expected clash-* default, got {:?}",
            state.input.value()
        );
        assert_eq!(
            state.pending_session.as_ref().map(|p| p.cwd.as_str()),
            Some("/tmp/my-project")
        );

        // Step 3: Submit name — should transition to NewSessionWorktree prompt
        let effects = reduce(
            &mut state,
            Action::Ui(UiAction::SubmitInput("my-project".to_string())),
        );
        assert!(effects.is_empty());
        assert_eq!(state.input_mode, InputMode::NewSessionWorktree);
        assert_eq!(state.input.value(), "n"); // default: no worktree

        // Step 4a: Answer "n" — should spawn session normally
        let effects = reduce(
            &mut state,
            Action::Ui(UiAction::SubmitInput("n".to_string())),
        );
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::DaemonAttach { .. })));
        assert_eq!(state.input_mode, InputMode::Attached);
    }

    #[test]
    fn test_new_session_worktree_yes() {
        let mut state = test_state();

        // Set up pending session state (as if NewSession + NewSessionName completed)
        state.input_mode = InputMode::NewSessionWorktree;
        state.pending_session = Some(crate::application::state::PendingSession {
            cwd: "/tmp/project".to_string(),
            name: Some("my-session".to_string()),
            worktree: false,
            preset: None,
        });
        state.input = tui_input::Input::new("y".to_string());

        let effects = reduce(
            &mut state,
            Action::Ui(UiAction::SubmitInput("y".to_string())),
        );
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::CreateWorktreeAndAttach { .. })));
        assert_eq!(state.input_mode, InputMode::Attached);
    }

    #[test]
    fn test_spawn_in_worktree_existing_worktree_delegates_to_attach() {
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            worktree: Some("my-wt".to_string()),
            ..Default::default()
        }];

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::SpawnInWorktree {
                session_id: "s1".to_string(),
            }),
        );
        // Should delegate to Attach → DaemonAttach effect
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::DaemonAttach { .. })));
    }

    #[test]
    fn test_spawn_in_worktree_no_project_path_toasts_error() {
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            project_path: String::new(),
            ..Default::default()
        }];

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::SpawnInWorktree {
                session_id: "s1".to_string(),
            }),
        );
        assert!(effects.is_empty());
        assert!(state.toast.as_deref().unwrap().contains("no project path"));
    }

    #[test]
    fn test_spawn_in_worktree_missing_session_toasts_error() {
        let mut state = test_state();

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::SpawnInWorktree {
                session_id: "nonexistent".to_string(),
            }),
        );
        assert!(effects.is_empty());
        assert!(state.toast.as_deref().unwrap().contains("not found"));
    }

    #[test]
    fn test_spawn_in_worktree_creates_worktree_effect() {
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            project_path: "/tmp/project".to_string(),
            ..Default::default()
        }];

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::SpawnInWorktree {
                session_id: "s1".to_string(),
            }),
        );
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::CreateWorktreeAndAttach { .. })));
        assert_eq!(state.input_mode, InputMode::Attached);
    }

    #[test]
    fn test_spawn_session_in_worktree() {
        let mut state = test_state();

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::SpawnSessionInWorktree {
                cwd: "/tmp/project".to_string(),
                name: Some("my-wt".to_string()),
            }),
        );
        assert!(effects
            .iter()
            .any(|e| matches!(e, Effect::CreateWorktreeAndAttach { .. })));
        assert_eq!(state.input_mode, InputMode::Attached);
    }

    #[test]
    fn test_attach_new_window_emits_effect_and_stays_normal() {
        let mut state = test_state();

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::AttachNewWindow {
                session_id: "test-session".to_string(),
            }),
        );

        // Must emit AttachInNewWindow effect
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::AttachInNewWindow { session_id } if session_id == "test-session"
        )));

        // Critical: TUI stays active — no mode change, no attached session
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.attached_session.is_none());
        assert!(state.spinner.is_none());
    }

    #[test]
    fn test_attach_all_new_windows_only_running() {
        let mut state = test_state();
        state.session_filter = crate::application::state::SessionFilter::All;
        state.store.sessions = vec![
            crate::domain::entities::Session {
                id: "running1".to_string(),
                is_running: true,
                ..Default::default()
            },
            crate::domain::entities::Session {
                id: "running2".to_string(),
                is_running: true,
                ..Default::default()
            },
            crate::domain::entities::Session {
                id: "idle1".to_string(),
                is_running: false,
                status: crate::domain::entities::SessionStatus::Stashed,
                ..Default::default()
            },
        ];

        let effects = reduce(&mut state, Action::Agent(AgentAction::AttachAllNewWindows));

        // Single batch effect with only running session IDs
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::AttachBatchInNewWindows { session_ids } => {
                assert_eq!(session_ids.len(), 2);
                assert!(session_ids.contains(&"running1".to_string()));
                assert!(session_ids.contains(&"running2".to_string()));
                assert!(!session_ids.contains(&"idle1".to_string()));
            }
            other => panic!("Expected AttachBatchInNewWindows, got {:?}", other),
        }
        // TUI stays normal
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_attach_new_window_blocked_when_externally_opened() {
        let mut state = test_state();
        state.externally_opened.insert("test-session".to_string());

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::AttachNewWindow {
                session_id: "test-session".to_string(),
            }),
        );

        assert!(effects.is_empty());
        assert!(state.toast.as_deref().unwrap().contains("already open"));
    }

    #[test]
    fn test_attach_all_excludes_externally_opened() {
        let mut state = test_state();
        state.session_filter = crate::application::state::SessionFilter::All;
        state.store.sessions = vec![
            crate::domain::entities::Session {
                id: "open1".to_string(),
                is_running: true,
                ..Default::default()
            },
            crate::domain::entities::Session {
                id: "not-open".to_string(),
                is_running: true,
                ..Default::default()
            },
        ];
        state.externally_opened.insert("open1".to_string());

        let effects = reduce(&mut state, Action::Agent(AgentAction::AttachAllNewWindows));

        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::AttachBatchInNewWindows { session_ids } => {
                assert_eq!(session_ids.len(), 1);
                assert_eq!(session_ids[0], "not-open");
            }
            other => panic!("Expected AttachBatchInNewWindows, got {:?}", other),
        }
    }

    // ── Open in IDE tests ──────────────────────────────────────

    #[test]
    fn test_open_in_ide_emits_detect_ides() {
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            cwd: Some("/tmp/project".to_string()),
            is_running: true,
            ..Default::default()
        }];

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::OpenInIde {
                session_id: "s1".to_string(),
            }),
        );

        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::DetectIdes { project_dir } if project_dir == "/tmp/project"
        )));
    }

    #[test]
    fn test_open_in_ide_prefers_cwd_over_project_path() {
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            cwd: Some("/tmp/cwd-dir".to_string()),
            project_path: "/tmp/project-path".to_string(),
            is_running: true,
            ..Default::default()
        }];

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::OpenInIde {
                session_id: "s1".to_string(),
            }),
        );

        match &effects[0] {
            Effect::DetectIdes { project_dir } => {
                assert_eq!(project_dir, "/tmp/cwd-dir");
            }
            other => panic!("Expected DetectIdes, got {:?}", other),
        }
    }

    #[test]
    fn test_open_in_ide_empty_project_dir_toasts() {
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "s1".to_string(),
            cwd: None,
            project_path: String::new(),
            is_running: true,
            ..Default::default()
        }];

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::OpenInIde {
                session_id: "s1".to_string(),
            }),
        );

        assert!(effects.is_empty());
        assert!(state
            .toast
            .as_deref()
            .unwrap()
            .contains("No project directory"));
    }

    #[test]
    fn test_open_in_ide_missing_session_toasts() {
        let mut state = test_state();

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::OpenInIde {
                session_id: "nonexistent".to_string(),
            }),
        );

        assert!(effects.is_empty());
        assert!(state.toast.as_deref().unwrap().contains("not found"));
    }

    // ── Picker tests ──────────────────────────────────────

    #[test]
    fn test_show_picker_empty_toasts() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Ui(UiAction::ShowPicker {
                title: "IDE".to_string(),
                items: vec![],
                on_select: crate::application::state::PickerAction::OpenInIde {
                    project_dir: "/tmp".to_string(),
                },
            }),
        );

        assert!(effects.is_empty());
        assert!(state.toast.as_deref().unwrap().contains("No IDEs"));
        assert!(state.picker_dialog.is_none());
    }

    #[test]
    fn test_show_picker_single_item_skips_picker() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Ui(UiAction::ShowPicker {
                title: "IDE".to_string(),
                items: vec![crate::application::state::PickerItem {
                    label: "VS Code".to_string(),
                    description: "".to_string(),
                    value: "code".to_string(),
                }],
                on_select: crate::application::state::PickerAction::OpenInIde {
                    project_dir: "/tmp".to_string(),
                },
            }),
        );

        // Should emit OpenIde directly, no picker dialog
        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::OpenIde { command, terminal, .. } if command == "code" && !terminal
        )));
        assert!(state.picker_dialog.is_none());
        assert!(state
            .toast
            .as_deref()
            .unwrap()
            .contains("Opening in VS Code"));
    }

    #[test]
    fn test_show_picker_sets_state() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Ui(UiAction::ShowPicker {
                title: "IDE".to_string(),
                items: vec![
                    crate::application::state::PickerItem {
                        label: "VS Code".to_string(),
                        description: "".to_string(),
                        value: "code".to_string(),
                    },
                    crate::application::state::PickerItem {
                        label: "Neovim".to_string(),
                        description: "".to_string(),
                        value: "terminal:nvim".to_string(),
                    },
                ],
                on_select: crate::application::state::PickerAction::OpenInIde {
                    project_dir: "/tmp".to_string(),
                },
            }),
        );

        assert!(effects.is_empty());
        assert!(state.picker_dialog.is_some());
        assert_eq!(state.input_mode, InputMode::Picker);
        assert_eq!(state.picker_dialog.as_ref().unwrap().items.len(), 2);
    }

    #[test]
    fn test_picker_up_at_zero_stays() {
        let mut state = test_state();
        state.picker_dialog = Some(crate::application::state::PickerDialog {
            title: "IDE".to_string(),
            items: vec![
                crate::application::state::PickerItem {
                    label: "A".to_string(),
                    description: "".to_string(),
                    value: "a".to_string(),
                },
                crate::application::state::PickerItem {
                    label: "B".to_string(),
                    description: "".to_string(),
                    value: "b".to_string(),
                },
            ],
            selected: 0,
            on_select_action: crate::application::state::PickerAction::OpenInIde {
                project_dir: "/tmp".to_string(),
            },
        });
        state.input_mode = InputMode::Picker;

        reduce(&mut state, Action::Ui(UiAction::PickerUp));
        assert_eq!(state.picker_dialog.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn test_picker_down_at_last_stays() {
        let mut state = test_state();
        state.picker_dialog = Some(crate::application::state::PickerDialog {
            title: "IDE".to_string(),
            items: vec![
                crate::application::state::PickerItem {
                    label: "A".to_string(),
                    description: "".to_string(),
                    value: "a".to_string(),
                },
                crate::application::state::PickerItem {
                    label: "B".to_string(),
                    description: "".to_string(),
                    value: "b".to_string(),
                },
            ],
            selected: 1,
            on_select_action: crate::application::state::PickerAction::OpenInIde {
                project_dir: "/tmp".to_string(),
            },
        });
        state.input_mode = InputMode::Picker;

        reduce(&mut state, Action::Ui(UiAction::PickerDown));
        assert_eq!(state.picker_dialog.as_ref().unwrap().selected, 1);
    }

    #[test]
    fn test_picker_select_emits_open_ide() {
        let mut state = test_state();
        state.picker_dialog = Some(crate::application::state::PickerDialog {
            title: "IDE".to_string(),
            items: vec![crate::application::state::PickerItem {
                label: "VS Code".to_string(),
                description: "".to_string(),
                value: "code".to_string(),
            }],
            selected: 0,
            on_select_action: crate::application::state::PickerAction::OpenInIde {
                project_dir: "/tmp/project".to_string(),
            },
        });
        state.input_mode = InputMode::Picker;

        let effects = reduce(&mut state, Action::Ui(UiAction::PickerSelect));

        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::OpenIde { command, project_dir, terminal }
                if command == "code" && project_dir == "/tmp/project" && !terminal
        )));
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(state.picker_dialog.is_none());
    }

    #[test]
    fn test_picker_select_terminal_ide_emits_terminal_flag() {
        let mut state = test_state();
        state.picker_dialog = Some(crate::application::state::PickerDialog {
            title: "IDE".to_string(),
            items: vec![crate::application::state::PickerItem {
                label: "Neovim".to_string(),
                description: "".to_string(),
                value: "terminal:nvim".to_string(),
            }],
            selected: 0,
            on_select_action: crate::application::state::PickerAction::OpenInIde {
                project_dir: "/tmp/project".to_string(),
            },
        });
        state.input_mode = InputMode::Picker;

        let effects = reduce(&mut state, Action::Ui(UiAction::PickerSelect));

        assert!(effects.iter().any(|e| matches!(
            e,
            Effect::OpenIde { command, terminal, .. } if command == "nvim" && *terminal
        )));
    }

    #[test]
    fn test_picker_cancel_clears_state() {
        let mut state = test_state();
        state.picker_dialog = Some(crate::application::state::PickerDialog {
            title: "IDE".to_string(),
            items: vec![],
            selected: 0,
            on_select_action: crate::application::state::PickerAction::OpenInIde {
                project_dir: "/tmp".to_string(),
            },
        });
        state.input_mode = InputMode::Picker;

        reduce(&mut state, Action::Ui(UiAction::PickerCancel));
        assert!(state.picker_dialog.is_none());
        assert_eq!(state.input_mode, InputMode::Normal);
    }

    #[test]
    fn test_picker_select_when_no_dialog_is_noop() {
        let mut state = test_state();
        let effects = reduce(&mut state, Action::Ui(UiAction::PickerSelect));
        assert!(effects.is_empty());
    }

    // ── Wild-session takeover flow ───────────────────────────────

    fn wild_session_in_state(
        state: &mut AppState,
        id: &str,
        cwd: &str,
        pid: Option<u32>,
        status: SessionStatus,
        source: crate::domain::entities::SessionSource,
    ) {
        let mut s = Session {
            id: id.to_string(),
            project: "p".into(),
            project_path: cwd.into(),
            status,
            ..Default::default()
        };
        s.source = source;
        s.wild_pid = pid;
        s.cwd = Some(cwd.into());
        state.store.sessions.push(s);
    }

    #[test]
    fn takeover_wild_kills_then_attaches() {
        // One confirmed keypress = kill the outside process, then the
        // regular attach flow (DaemonAttach resumes under the daemon).
        let mut state = test_state();
        wild_session_in_state(
            &mut state,
            "sid",
            "/repo",
            Some(1234),
            SessionStatus::Running,
            crate::domain::entities::SessionSource::Wild,
        );
        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::TakeoverWild {
                session_id: "sid".into(),
            }),
        );
        assert!(
            matches!(effects[0], Effect::KillWildProcess { pid: 1234 }),
            "first effect kills the wild PID, got {:?}",
            effects[0]
        );
        assert!(
            effects.iter().any(
                |e| matches!(e, Effect::DaemonAttach { session_id, .. } if session_id == "sid")
            ),
            "attach is chained after the kill"
        );
        assert_eq!(state.input_mode, InputMode::Attached);
        assert_eq!(state.attached_session.as_deref(), Some("sid"));
    }

    #[test]
    fn takeover_wild_uses_fresh_pid_from_store() {
        // The PID is read from the store at confirm time — the wild
        // scan may have refreshed it while the confirm dialog was open.
        let mut state = test_state();
        wild_session_in_state(
            &mut state,
            "sid",
            "/repo",
            Some(4321),
            SessionStatus::Running,
            crate::domain::entities::SessionSource::Wild,
        );
        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::TakeoverWild {
                session_id: "sid".into(),
            }),
        );
        assert!(matches!(effects[0], Effect::KillWildProcess { pid: 4321 }));
    }

    #[test]
    fn takeover_wild_on_stale_row_refuses_with_toast() {
        // Source flipped to Daemon between confirm open and Takeover
        // (e.g. clash adopted it via another path) — refuse takeover.
        let mut state = test_state();
        wild_session_in_state(
            &mut state,
            "sid",
            "/repo",
            None,
            SessionStatus::Running,
            crate::domain::entities::SessionSource::Daemon,
        );
        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::TakeoverWild {
                session_id: "sid".into(),
            }),
        );
        assert!(effects.is_empty(), "no effect on stale Daemon row");
        assert!(state.toast.is_some());
    }

    #[test]
    fn takeover_wild_on_synthetic_row_refuses_with_toast() {
        // Synthetic `wild-pid-<pid>` rows have no conversation to
        // resume — takeover must refuse with a reason.
        let mut state = test_state();
        wild_session_in_state(
            &mut state,
            "wild-pid-777",
            "/repo",
            Some(777),
            SessionStatus::Running,
            crate::domain::entities::SessionSource::Wild,
        );
        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::TakeoverWild {
                session_id: "wild-pid-777".into(),
            }),
        );
        assert!(effects.is_empty());
        assert!(state.toast.is_some());
    }

    #[test]
    fn drop_session_threads_wild_pid_into_terminate_effect() {
        // A synthetic wild row carries wild_pid but no real session id
        // for pgrep to find. The drop reducer must thread wild_pid into
        // Effect::TerminateProcess so the handler can SIGTERM the bare
        // claude directly. Without this, the row vanishes from the UI
        // but the underlying process keeps running.
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "wild-pid-12345".to_string(),
            wild_pid: Some(12345),
            source: crate::domain::entities::SessionSource::Wild,
            is_running: true,
            ..Default::default()
        }];

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::DropSession {
                session_id: "wild-pid-12345".to_string(),
            }),
        );

        let terminate = effects
            .iter()
            .find(|e| matches!(e, Effect::TerminateProcess { .. }))
            .expect("DropSession must emit TerminateProcess");
        match terminate {
            Effect::TerminateProcess {
                session_id,
                wild_pid,
                ..
            } => {
                assert_eq!(session_id, "wild-pid-12345");
                assert_eq!(*wild_pid, Some(12345));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn drop_session_wild_pid_none_for_normal_row() {
        // Daemon-managed (or any row without wild_pid set) keeps
        // wild_pid=None on the effect — only the existing pgrep-by-id
        // path fires, no spurious SIGTERM at a daemon-managed PID.
        let mut state = test_state();
        state.store.sessions = vec![crate::domain::entities::Session {
            id: "real-session-id".to_string(),
            source: crate::domain::entities::SessionSource::Daemon,
            is_running: true,
            ..Default::default()
        }];

        let effects = reduce(
            &mut state,
            Action::Agent(AgentAction::DropSession {
                session_id: "real-session-id".to_string(),
            }),
        );

        match effects
            .iter()
            .find(|e| matches!(e, Effect::TerminateProcess { .. }))
            .unwrap()
        {
            Effect::TerminateProcess { wild_pid, .. } => assert_eq!(*wild_pid, None),
            _ => unreachable!(),
        }
    }

    // ── Scratch notes ────────────────────────────────────────────

    #[test]
    fn test_scratch_create_emits_effects() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Scratch(ScratchAction::Create {
                parent: "sql".to_string(),
                title: "  ideas  ".to_string(),
            }),
        );
        assert!(matches!(
            effects[0],
            Effect::CreateScratchNote { ref parent, ref title }
                if parent == "sql" && title == "ideas"
        ));
        assert!(matches!(effects[1], Effect::RefreshScratchNotes));
        // The destination folder is expanded so the new note shows on refresh.
        assert!(state.expanded_scratch_dirs.contains("sql"));
    }

    #[test]
    fn test_scratch_create_empty_title_is_noop() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Scratch(ScratchAction::Create {
                parent: String::new(),
                title: "   ".to_string(),
            }),
        );
        assert!(effects.is_empty());
        assert!(state.toast.is_some());
    }

    #[test]
    fn test_scratch_create_dir_emits_effects_and_expands() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Scratch(ScratchAction::CreateDir {
                parent: String::new(),
                name: "sql".to_string(),
            }),
        );
        assert!(matches!(
            effects[0],
            Effect::CreateScratchDir { ref parent, ref name }
                if parent.is_empty() && name == "sql"
        ));
        assert!(matches!(effects[1], Effect::RefreshScratchNotes));
        assert!(state.expanded_scratch_dirs.contains("sql"));
    }

    #[test]
    fn test_scratch_rename_emits_effects() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Scratch(ScratchAction::Rename {
                id: "sql/q.md".to_string(),
                new_name: "query".to_string(),
            }),
        );
        assert!(matches!(
            effects[0],
            Effect::RenameScratch { ref id, ref new_name }
                if id == "sql/q.md" && new_name == "query"
        ));
        assert!(matches!(effects[1], Effect::RefreshScratchNotes));
    }

    #[test]
    fn test_scratch_move_emits_effects_and_expands_dest() {
        let mut state = test_state();
        let effects = reduce(
            &mut state,
            Action::Scratch(ScratchAction::Move {
                id: "a.md".to_string(),
                new_parent: "sql/sub".to_string(),
            }),
        );
        assert!(matches!(
            effects[0],
            Effect::MoveScratch { ref id, ref new_parent }
                if id == "a.md" && new_parent == "sql/sub"
        ));
        assert!(matches!(effects[1], Effect::RefreshScratchNotes));
        // Destination folder and its ancestors are expanded so the moved
        // entry is visible after the refresh rebuilds the tree.
        assert!(state.expanded_scratch_dirs.contains("sql"));
        assert!(state.expanded_scratch_dirs.contains("sql/sub"));
    }

    #[test]
    fn test_scratch_move_targets_excludes_self_descendants_and_parent() {
        // Tree: sql/, sql/sub/, notes/ (all folders).
        let notes = vec![
            crate::domain::entities::ScratchNote {
                id: "notes".to_string(),
                title: "notes".to_string(),
                is_dir: true,
                ..Default::default()
            },
            crate::domain::entities::ScratchNote {
                id: "sql".to_string(),
                title: "sql".to_string(),
                is_dir: true,
                ..Default::default()
            },
            crate::domain::entities::ScratchNote {
                id: "sql/sub".to_string(),
                title: "sub".to_string(),
                parent: "sql".to_string(),
                depth: 1,
                is_dir: true,
                ..Default::default()
            },
        ];
        // Moving the "sql" folder (currently at root): root is excluded (it's
        // the current parent), "sql" itself and its descendant "sql/sub" are
        // excluded (cycle). Only "notes" remains.
        let sql = notes[1].clone();
        let targets = scratch_move_targets(&notes, &sql);
        let values: Vec<&str> = targets.iter().map(|i| i.value.as_str()).collect();
        assert_eq!(values, vec!["notes"]);
    }

    #[test]
    fn test_scratch_move_targets_offers_root_for_nested_entry() {
        let notes = vec![
            crate::domain::entities::ScratchNote {
                id: "sql".to_string(),
                title: "sql".to_string(),
                is_dir: true,
                ..Default::default()
            },
            crate::domain::entities::ScratchNote {
                id: "sql/q.md".to_string(),
                title: "q".to_string(),
                parent: "sql".to_string(),
                depth: 1,
                ..Default::default()
            },
        ];
        // A note inside "sql": root is offered (first), and "sql" is excluded
        // as the current parent — leaving only root.
        let note = notes[1].clone();
        let targets = scratch_move_targets(&notes, &note);
        let values: Vec<&str> = targets.iter().map(|i| i.value.as_str()).collect();
        assert_eq!(values, vec![""]);
        assert_eq!(targets[0].label, "/ (root)");
    }

    #[test]
    fn test_enter_move_scratch_mode_opens_picker() {
        let mut state = test_state();
        state.nav.push(ViewKind::Scratch, None);
        state.store.scratch_notes = vec![
            crate::domain::entities::ScratchNote {
                id: "a.md".to_string(),
                title: "a".to_string(),
                ..Default::default()
            },
            crate::domain::entities::ScratchNote {
                id: "box".to_string(),
                title: "box".to_string(),
                is_dir: true,
                ..Default::default()
            },
        ];
        // Select the file (visible row 0); the only destination is "box".
        state.table_state.selected = 0;
        reduce(&mut state, Action::Ui(UiAction::EnterMoveScratchMode));
        assert_eq!(state.input_mode, InputMode::Picker);
        let picker = state.picker_dialog.as_ref().expect("picker opened");
        assert!(matches!(
            picker.on_select_action,
            crate::application::state::PickerAction::MoveScratch { ref id } if id == "a.md"
        ));
        let values: Vec<&str> = picker.items.iter().map(|i| i.value.as_str()).collect();
        assert_eq!(values, vec!["box"]);
    }

    #[test]
    fn test_enter_move_scratch_mode_no_targets_toasts() {
        let mut state = test_state();
        state.nav.push(ViewKind::Scratch, None);
        // A single root-level note: nowhere to move it (no folders, already root).
        state.store.scratch_notes = vec![crate::domain::entities::ScratchNote {
            id: "lonely.md".to_string(),
            title: "lonely".to_string(),
            ..Default::default()
        }];
        state.table_state.selected = 0;
        let effects = reduce(&mut state, Action::Ui(UiAction::EnterMoveScratchMode));
        assert!(effects.is_empty());
        assert!(state.picker_dialog.is_none());
        assert!(state.toast.is_some());
    }

    #[test]
    fn test_enter_copy_scratch_path_mode_offers_three_formats() {
        let mut state = test_state();
        state.nav.push(ViewKind::Scratch, None);
        state.store.scratch_notes = vec![crate::domain::entities::ScratchNote {
            id: "sql/q.md".to_string(),
            title: "q".to_string(),
            path: "/home/u/.claude/clash/scratch/sql/q.md".to_string(),
            parent: "sql".to_string(),
            depth: 1,
            ..Default::default()
        }];
        state.table_state.selected = 0;
        reduce(&mut state, Action::Ui(UiAction::EnterCopyScratchPathMode));
        assert_eq!(state.input_mode, InputMode::Picker);
        let picker = state.picker_dialog.as_ref().expect("picker opened");
        assert!(matches!(
            picker.on_select_action,
            crate::application::state::PickerAction::CopyToClipboard
        ));
        let values: Vec<&str> = picker.items.iter().map(|i| i.value.as_str()).collect();
        assert_eq!(
            values,
            vec!["/home/u/.claude/clash/scratch/sql/q.md", "sql/q.md", "q.md"]
        );
    }

    #[test]
    fn test_picker_select_copy_path_emits_clipboard_effect() {
        let mut state = test_state();
        state.picker_dialog = Some(crate::application::state::PickerDialog {
            title: "Copy path of 'q'".to_string(),
            items: vec![crate::application::state::PickerItem {
                label: "Relative path (from scratch root)".to_string(),
                description: "sql/q.md".to_string(),
                value: "sql/q.md".to_string(),
            }],
            selected: 0,
            on_select_action: crate::application::state::PickerAction::CopyToClipboard,
        });
        state.input_mode = InputMode::Picker;
        let effects = reduce(&mut state, Action::Ui(UiAction::PickerSelect));
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(matches!(
            effects[0],
            Effect::CopyToClipboard { ref text } if text == "sql/q.md"
        ));
        // Toast names the chosen format, not the generic "Opening in …".
        assert!(state
            .toast
            .as_deref()
            .is_some_and(|t| t.starts_with("Copied")));
    }

    #[test]
    fn test_picker_select_move_scratch_dispatches_move() {
        let mut state = test_state();
        state.picker_dialog = Some(crate::application::state::PickerDialog {
            title: "Move 'a' to…".to_string(),
            items: vec![crate::application::state::PickerItem {
                label: "box/".to_string(),
                description: String::new(),
                value: "box".to_string(),
            }],
            selected: 0,
            on_select_action: crate::application::state::PickerAction::MoveScratch {
                id: "a.md".to_string(),
            },
        });
        state.input_mode = InputMode::Picker;
        let effects = reduce(&mut state, Action::Ui(UiAction::PickerSelect));
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(matches!(
            effects[0],
            Effect::MoveScratch { ref id, ref new_parent }
                if id == "a.md" && new_parent == "box"
        ));
        assert!(matches!(effects[1], Effect::RefreshScratchNotes));
    }

    #[test]
    fn test_scratch_toggle_dir_flips_expansion() {
        let mut state = test_state();
        reduce(
            &mut state,
            Action::Scratch(ScratchAction::ToggleDir {
                id: "sql".to_string(),
            }),
        );
        assert!(state.expanded_scratch_dirs.contains("sql"));
        reduce(
            &mut state,
            Action::Scratch(ScratchAction::ToggleDir {
                id: "sql".to_string(),
            }),
        );
        assert!(!state.expanded_scratch_dirs.contains("sql"));
    }

    #[test]
    fn test_scratch_delete_removes_from_store_and_emits_effects() {
        let mut state = test_state();
        state.store.scratch_notes = vec![
            crate::domain::entities::ScratchNote {
                id: "a.md".to_string(),
                title: "a".to_string(),
                ..Default::default()
            },
            crate::domain::entities::ScratchNote {
                id: "b.md".to_string(),
                title: "b".to_string(),
                ..Default::default()
            },
        ];
        let effects = reduce(
            &mut state,
            Action::Scratch(ScratchAction::Delete {
                id: "a.md".to_string(),
            }),
        );
        // Optimistically removed from the in-memory list.
        assert_eq!(state.store.scratch_notes.len(), 1);
        assert_eq!(state.store.scratch_notes[0].id, "b.md");
        assert!(matches!(effects[0], Effect::DeleteScratchNote { ref id } if id == "a.md"));
        assert!(matches!(effects[1], Effect::RefreshScratchNotes));
    }

    #[test]
    fn test_scratch_delete_folder_drops_subtree_and_expansion() {
        let mut state = test_state();
        state.store.scratch_notes = vec![
            crate::domain::entities::ScratchNote {
                id: "sql".to_string(),
                title: "sql".to_string(),
                is_dir: true,
                ..Default::default()
            },
            crate::domain::entities::ScratchNote {
                id: "sql/q.md".to_string(),
                title: "q".to_string(),
                parent: "sql".to_string(),
                depth: 1,
                ..Default::default()
            },
            crate::domain::entities::ScratchNote {
                id: "keep.md".to_string(),
                title: "keep".to_string(),
                ..Default::default()
            },
        ];
        state.expanded_scratch_dirs.insert("sql".to_string());
        reduce(
            &mut state,
            Action::Scratch(ScratchAction::Delete {
                id: "sql".to_string(),
            }),
        );
        // Folder and its child are gone; the unrelated note stays.
        let ids: Vec<&str> = state
            .store
            .scratch_notes
            .iter()
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(ids, vec!["keep.md"]);
        assert!(!state.expanded_scratch_dirs.contains("sql"));
    }

    #[test]
    fn test_scratch_open_in_editor_resolves_path() {
        let mut state = test_state();
        state.store.scratch_notes = vec![crate::domain::entities::ScratchNote {
            id: "a.md".to_string(),
            title: "a".to_string(),
            path: "/home/u/.claude/clash/scratch/a.md".to_string(),
            ..Default::default()
        }];
        let effects = reduce(
            &mut state,
            Action::Scratch(ScratchAction::OpenInEditor {
                id: "a.md".to_string(),
            }),
        );
        // Reuses the editor picker with the note's file path.
        assert!(matches!(
            effects[0],
            Effect::DetectEditors { ref path }
                if path == "/home/u/.claude/clash/scratch/a.md"
        ));
    }

    #[test]
    fn test_scratch_title_submit_creates_note() {
        let mut state = test_state();
        state.input_mode = InputMode::NewScratchTitle;
        state.scratch_op_target = Some("sql".to_string());
        state.input = tui_input::Input::new("todo".to_string());
        let effects = reduce(&mut state, Action::Ui(UiAction::SubmitInput(String::new())));
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(matches!(
            effects[0],
            Effect::CreateScratchNote { ref parent, ref title }
                if parent == "sql" && title == "todo"
        ));
        // Target context is consumed on submit.
        assert!(state.scratch_op_target.is_none());
    }

    #[test]
    fn test_scratch_dir_submit_creates_folder() {
        let mut state = test_state();
        state.input_mode = InputMode::NewScratchDir;
        state.scratch_op_target = Some(String::new());
        state.input = tui_input::Input::new("archive".to_string());
        let effects = reduce(&mut state, Action::Ui(UiAction::SubmitInput(String::new())));
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(matches!(
            effects[0],
            Effect::CreateScratchDir { ref parent, ref name }
                if parent.is_empty() && name == "archive"
        ));
    }

    #[test]
    fn test_scratch_rename_submit_renames_selected() {
        let mut state = test_state();
        state.input_mode = InputMode::RenameScratch;
        state.scratch_op_target = Some("notes/old.md".to_string());
        state.input = tui_input::Input::new("new".to_string());
        let effects = reduce(&mut state, Action::Ui(UiAction::SubmitInput(String::new())));
        assert_eq!(state.input_mode, InputMode::Normal);
        assert!(matches!(
            effects[0],
            Effect::RenameScratch { ref id, ref new_name }
                if id == "notes/old.md" && new_name == "new"
        ));
    }
}
