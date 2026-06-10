//! clash GUI — cmux-style desktop client (experimental).
//!
//! Self-contained: the PTY session manager (`DaemonServer`) runs in-process,
//! exactly like the TUI. The webview is just another frontend over the same
//! core: session listing reuses `session_refresh`, terminal IO reuses the
//! daemon protocol (`DaemonClient` over the in-process Unix socket).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use clash::domain::entities::Session;
use clash::infrastructure::config::Config;
use clash::infrastructure::daemon::client::DaemonClient;
use clash::infrastructure::daemon::protocol::Event;
use clash::infrastructure::daemon::server::DaemonServer;
use clash::infrastructure::fs::backend::FsBackend;
use clash::infrastructure::session_refresh;
use tauri::{Emitter, Manager, State};

/// Shared backend state for all Tauri commands.
struct GuiState {
    backend: FsBackend,
    claude_bin: String,
    /// Custom IDE entries from config.toml (merged into detection).
    config_ides: Vec<clash::infrastructure::config::IdeEntry>,
    /// Previous session list — input to the merge step of the refresh pipeline.
    previous: Mutex<Vec<Session>>,
    /// Last seen status per session — attention-transition detection.
    prev_statuses: Mutex<HashMap<String, clash::domain::entities::SessionStatus>>,
    /// Control-plane client (list/kill). Separate from attach clients so a
    /// streaming attach never blocks a list request.
    control: tokio::sync::Mutex<DaemonClient>,
    /// One streaming client per attached session.
    attached: tokio::sync::Mutex<HashMap<String, DaemonClient>>,
}

/// Payload for `pty-output` events pushed to the webview.
#[derive(Clone, serde::Serialize)]
struct PtyOutput {
    session_id: String,
    /// Base64-encoded raw terminal bytes (as received from the daemon).
    data: String,
}

#[derive(Clone, serde::Serialize)]
struct PtyExited {
    session_id: String,
    exit_code: Option<i32>,
}

/// Fire a native desktop notification. Uses platform tools that work from a
/// bare (unbundled) binary: `osascript` on macOS, `notify-send` on Linux.
fn native_notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('\\', "").replace('"', "'"),
            title.replace('\\', "").replace('"', "'")
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = std::process::Command::new("notify-send")
            .args([title, body])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

/// Payload for `session-attention` events (sidebar unread badges).
#[derive(Clone, serde::Serialize)]
struct SessionAttention {
    session_id: String,
    name: String,
}

/// Full session list via the same pipeline the TUI uses
/// (disk + registry + hooks + daemon, merged and sorted by section).
///
/// Also detects attention transitions (→ Prompting/Waiting/Errored): fires a
/// desktop notification when the window is unfocused — cmux-style suppression
/// when you're already looking at the app — and always emits
/// `session-attention` so the sidebar can badge the row.
#[tauri::command]
async fn list_sessions(
    app: tauri::AppHandle,
    state: State<'_, GuiState>,
) -> Result<Vec<Session>, String> {
    let registry = clash::infrastructure::hooks::registry::load();
    let previous = state.previous.lock().unwrap().clone();
    let mut input = session_refresh::gather_sync_input(&state.backend, &previous, registry);
    {
        let mut control = state.control.lock().await;
        ensure_connected(&mut control).await;
        input.daemon_infos = session_refresh::gather_daemon_input(&mut control).await;
    }
    let sessions = session_refresh::build_session_list(&input);

    let focused = app
        .get_webview_window("main")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    {
        use clash::domain::entities::SessionStatus;
        let needs_attention = |s: &SessionStatus| {
            matches!(
                s,
                SessionStatus::Prompting | SessionStatus::Waiting | SessionStatus::Errored
            )
        };
        let mut prev_statuses = state.prev_statuses.lock().unwrap();
        for s in &sessions {
            let was = prev_statuses.get(s.id.as_str());
            if needs_attention(&s.status) && was.is_some_and(|w| !needs_attention(w)) {
                let name = s
                    .name
                    .clone()
                    .unwrap_or_else(|| s.id.chars().take(8).collect());
                let _ = app.emit(
                    "session-attention",
                    SessionAttention {
                        session_id: s.id.clone(),
                        name: name.clone(),
                    },
                );
                if !focused {
                    let what = match s.status {
                        SessionStatus::Errored => "errored",
                        _ => "needs your input",
                    };
                    native_notify(&format!("clash · {}", name), what);
                }
            }
        }
        *prev_statuses = sessions.iter().map(|s| (s.id.clone(), s.status)).collect();
    }

    *state.previous.lock().unwrap() = sessions.clone();
    Ok(sessions)
}

/// Attach to a session, spawning it first if it isn't alive in the daemon.
/// Output is streamed to the webview as `pty-output` events.
#[tauri::command]
async fn open_session(
    app: tauri::AppHandle,
    state: State<'_, GuiState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let mut attached = state.attached.lock().await;
    if attached.contains_key(&session_id) {
        return Ok(());
    }

    // Is the session already alive in the daemon?
    let alive = {
        let mut control = state.control.lock().await;
        ensure_connected(&mut control).await;
        match control.list_sessions().await {
            Ok(infos) => infos
                .iter()
                .any(|i| i.session_id == session_id && i.is_alive),
            Err(_) => false,
        }
    };

    let mut client = DaemonClient::new(DaemonClient::instance_socket_path());
    client.connect().await.map_err(|e| e.to_string())?;

    if !alive {
        // Resume the recorded Claude session in its working directory.
        let (cwd, name) = {
            let prev = state.previous.lock().unwrap();
            let session = prev.iter().find(|s| s.id == session_id);
            (
                session
                    .map(|s| s.cwd.clone().unwrap_or_else(|| s.project_path.clone()))
                    .unwrap_or_default(),
                session.and_then(|s| s.name.clone()),
            )
        };
        // Clear the stale "idle" hook status so the daemon's Starting/Running
        // status can take effect in reconciliation (same as the TUI's resume).
        clash::infrastructure::hooks::write_session_status(
            state.backend.base_dir(),
            &session_id,
            "starting",
        );
        client
            .create_session(
                &session_id,
                &state.claude_bin,
                &["--resume".to_string(), session_id.clone()],
                if cwd.is_empty() { None } else { Some(&cwd) },
                name,
                cols,
                rows,
                HashMap::new(),
            )
            .await
            .map_err(|e| format!("Failed to spawn session: {}", e))?;
    }

    client
        .attach(&session_id)
        .await
        .map_err(|e| format!("Failed to attach: {}", e))?;
    let _ = client.resize(&session_id, cols, rows).await;

    // Forward daemon stream events (history replay + live output) to the webview.
    let mut stream_rx = client
        .take_stream_rx()
        .ok_or_else(|| "No stream channel from daemon".to_string())?;
    let sid = session_id.clone();
    tokio::spawn(async move {
        while let Some(event) = stream_rx.recv().await {
            match event {
                Event::Output { session_id, data } => {
                    // cmux-style in-band notifications: OSC 9 / OSC 777
                    // escape sequences in the output trigger desktop alerts.
                    if let Ok(bytes) =
                        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &data)
                    {
                        if let Some((title, body)) = parse_osc_notification(&bytes) {
                            native_notify(&title, &body);
                            let _ = app.emit(
                                "session-attention",
                                SessionAttention {
                                    session_id: session_id.clone(),
                                    name: title,
                                },
                            );
                        }
                    }
                    let _ = app.emit("pty-output", PtyOutput { session_id, data });
                }
                Event::Exited {
                    session_id,
                    exit_code,
                } => {
                    let _ = app.emit(
                        "pty-exited",
                        PtyExited {
                            session_id,
                            exit_code,
                        },
                    );
                    break;
                }
                _ => {}
            }
        }
        tracing::debug!("output forwarder for {} ended", sid);
    });

    attached.insert(session_id, client);
    Ok(())
}

/// Send keyboard input (raw bytes as typed in xterm.js) to a session.
#[tauri::command]
async fn send_input(
    state: State<'_, GuiState>,
    session_id: String,
    text: String,
) -> Result<(), String> {
    let mut attached = state.attached.lock().await;
    let client = attached
        .get_mut(&session_id)
        .ok_or_else(|| "Not attached".to_string())?;
    client
        .send_input(&session_id, text.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn resize_session(
    state: State<'_, GuiState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let mut attached = state.attached.lock().await;
    let client = attached
        .get_mut(&session_id)
        .ok_or_else(|| "Not attached".to_string())?;
    client
        .resize(&session_id, cols, rows)
        .await
        .map_err(|e| e.to_string())
}

/// Detach from a session (keeps it running in the daemon).
#[tauri::command]
async fn close_session(state: State<'_, GuiState>, session_id: String) -> Result<(), String> {
    let mut attached = state.attached.lock().await;
    if let Some(mut client) = attached.remove(&session_id) {
        let _ = client.detach(&session_id).await;
    }
    Ok(())
}

/// Stash a session: stop its process but keep it resumable (same semantics
/// as the TUI's `s` — daemon kill + hook status "idle").
#[tauri::command]
async fn stash_session(state: State<'_, GuiState>, session_id: String) -> Result<(), String> {
    {
        let mut attached = state.attached.lock().await;
        attached.remove(&session_id);
    }
    {
        let mut control = state.control.lock().await;
        ensure_connected(&mut control).await;
        let _ = control.kill_session(&session_id).await;
    }
    clash::infrastructure::hooks::write_session_status(
        state.backend.base_dir(),
        &session_id,
        "idle",
    );
    Ok(())
}

/// Kill a session and remove it from the registry (same as the TUI's `x`).
#[tauri::command]
async fn kill_session(state: State<'_, GuiState>, session_id: String) -> Result<(), String> {
    {
        let mut attached = state.attached.lock().await;
        attached.remove(&session_id);
    }
    {
        let mut control = state.control.lock().await;
        ensure_connected(&mut control).await;
        let _ = control.kill_session(&session_id).await;
    }
    clash::infrastructure::hooks::registry::unregister(&session_id);
    Ok(())
}

/// Rename a session — same registry write the TUI's rename uses.
#[tauri::command]
fn rename_session(session_id: String, name: String) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Name cannot be empty".to_string());
    }
    clash::infrastructure::hooks::registry::rename(&session_id, name.trim());
    Ok(())
}

/// Create a brand-new Claude session in `cwd` (same pipeline as the TUI's `n`:
/// register → save name → status starting → daemon spawn with --session-id).
#[tauri::command]
async fn create_new_session(
    state: State<'_, GuiState>,
    name: String,
    cwd: String,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return Err("Working directory is required".to_string());
    }
    if !std::path::Path::new(cwd).is_dir() {
        return Err(format!("Not a directory: {}", cwd));
    }
    let name = name.trim();
    let session_id = uuid::Uuid::now_v7().to_string();

    clash::infrastructure::hooks::registry::register(
        &session_id,
        if name.is_empty() { "session" } else { name },
        cwd,
        None,
    );
    if !name.is_empty() {
        clash::infrastructure::hooks::save_session_name(
            state.backend.base_dir(),
            &session_id,
            name,
            Some(cwd),
        );
    }
    clash::infrastructure::hooks::write_session_status(
        state.backend.base_dir(),
        &session_id,
        "starting",
    );

    let mut control = state.control.lock().await;
    ensure_connected(&mut control).await;
    control
        .create_session(
            &session_id,
            &state.claude_bin,
            &["--session-id".to_string(), session_id.clone()],
            Some(cwd),
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            },
            cols,
            rows,
            HashMap::new(),
        )
        .await
        .map_err(|e| format!("Failed to spawn session: {}", e))?;

    Ok(session_id)
}

/// `git diff HEAD` for a session's working directory (same as the TUI's diff view).
#[tauri::command]
async fn get_diff(state: State<'_, GuiState>, session_id: String) -> Result<String, String> {
    let dir = {
        let prev = state.previous.lock().unwrap();
        prev.iter()
            .find(|s| s.id == session_id)
            .and_then(|s| {
                s.cwd
                    .clone()
                    .filter(|c| !c.is_empty())
                    .or_else(|| Some(s.project_path.clone()).filter(|p| !p.is_empty()))
            })
            .ok_or_else(|| "No working directory for session".to_string())?
    };
    let output = tokio::process::Command::new("git")
        .args(["diff", "HEAD"])
        .current_dir(&dir)
        .output()
        .await
        .map_err(|e| format!("git diff failed: {}", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// All teams (with members), via the same DataRepository the TUI uses.
#[tauri::command]
fn list_teams(state: State<'_, GuiState>) -> Result<Vec<clash::domain::entities::Team>, String> {
    use clash::domain::ports::DataRepository;
    state.backend.load_teams().map_err(|e| e.to_string())
}

/// Tasks for a team.
#[tauri::command]
fn list_tasks(
    state: State<'_, GuiState>,
    team: String,
) -> Result<Vec<clash::domain::entities::Task>, String> {
    use clash::domain::ports::DataRepository;
    state.backend.load_tasks(&team).map_err(|e| e.to_string())
}

/// Conversation message DTO (ConversationMessage has no Serialize derive).
#[derive(Clone, serde::Serialize)]
struct MessageDto {
    role: String,
    text: String,
}

fn to_message_dtos(msgs: Vec<clash::domain::entities::ConversationMessage>) -> Vec<MessageDto> {
    msgs.into_iter()
        .map(|m| MessageDto {
            role: m.role,
            text: m.text,
        })
        .collect()
}

/// Conversation transcript of a session (same parser as the TUI detail view).
#[tauri::command]
fn get_conversation(
    state: State<'_, GuiState>,
    project: String,
    session_id: String,
) -> Result<Vec<MessageDto>, String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .load_conversation(&project, &session_id)
        .map(to_message_dtos)
        .map_err(|e| e.to_string())
}

/// Subagents of a session.
#[tauri::command]
fn get_subagents(
    state: State<'_, GuiState>,
    project: String,
    session_id: String,
) -> Result<Vec<clash::domain::entities::Subagent>, String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .load_subagents(&project, &session_id)
        .map_err(|e| e.to_string())
}

/// Conversation transcript of a subagent.
#[tauri::command]
fn get_subagent_conversation(
    state: State<'_, GuiState>,
    project: String,
    session_id: String,
    agent_id: String,
) -> Result<Vec<MessageDto>, String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .load_subagent_conversation(&project, &session_id, &agent_id)
        .map(to_message_dtos)
        .map_err(|e| e.to_string())
}

/// Inbox messages for a team agent (`teams/{team}/inboxes/{agent}.json`).
#[tauri::command]
fn get_inbox(
    state: State<'_, GuiState>,
    team: String,
    agent: String,
) -> Result<Vec<clash::domain::entities::InboxMessage>, String> {
    use clash::domain::ports::DataRepository;
    let path = state
        .backend
        .teams_dir()
        .join(&team)
        .join("inboxes")
        .join(format!("{}.json", agent));
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| format!("Malformed inbox: {}", e))
}

/// Session presets for a project directory (global + project + superset,
/// same precedence as the TUI's `n` picker).
#[tauri::command]
fn list_presets(project_dir: String) -> Vec<clash::domain::entities::Preset> {
    let global_config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clash");
    clash::infrastructure::fs::presets::load_presets(
        std::path::Path::new(&project_dir),
        &global_config_dir,
    )
}

/// Create a git worktree and spawn a new session in it — the TUI's
/// worktree-spawn pipeline (worktree add -b <name> + register + daemon spawn).
#[tauri::command]
async fn create_worktree_session(
    state: State<'_, GuiState>,
    name: String,
    project_path: String,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Name is required".to_string());
    }
    let project_dir = std::path::Path::new(&project_path);
    if !project_dir.is_dir() {
        return Err(format!("Not a directory: {}", project_path));
    }

    // Source branch = current branch of the project
    let git_branch = {
        let out = tokio::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(&project_path)
            .output()
            .await
            .map_err(|e| format!("git failed: {}", e))?;
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    let project_name = project_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    let worktree_base = project_dir
        .parent()
        .unwrap_or(project_dir)
        .join(format!("{}-worktrees", project_name));
    let worktree_path = worktree_base.join(&name);
    std::fs::create_dir_all(&worktree_base).map_err(|e| e.to_string())?;

    let wt_str = worktree_path.to_string_lossy().to_string();
    let mut git_args = vec!["worktree", "add", &wt_str, "-b", &name];
    if !git_branch.is_empty() {
        git_args.push(&git_branch);
    }
    let out = tokio::process::Command::new("git")
        .args(&git_args)
        .current_dir(&project_path)
        .output()
        .await
        .map_err(|e| format!("git worktree failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let session_id = uuid::Uuid::now_v7().to_string();
    clash::infrastructure::hooks::registry::register(
        &session_id,
        &name,
        &wt_str,
        Some(git_branch.as_str()),
    );
    clash::infrastructure::hooks::save_session_name(
        state.backend.base_dir(),
        &session_id,
        &name,
        Some(&wt_str),
    );
    clash::infrastructure::hooks::write_session_status(
        state.backend.base_dir(),
        &session_id,
        "starting",
    );

    let mut control = state.control.lock().await;
    ensure_connected(&mut control).await;
    control
        .create_session(
            &session_id,
            &state.claude_bin,
            &["--session-id".to_string(), session_id.clone()],
            Some(&wt_str),
            Some(name),
            cols,
            rows,
            HashMap::new(),
        )
        .await
        .map_err(|e| format!("Failed to spawn session: {}", e))?;

    Ok(session_id)
}

/// IDE picker item DTO (PickerItem has no Serialize derive).
#[derive(Clone, serde::Serialize)]
struct IdeDto {
    label: String,
    description: String,
    value: String,
}

/// Detected IDEs (same detection as the TUI's `e`).
#[tauri::command]
fn detect_ides(state: State<'_, GuiState>) -> Vec<IdeDto> {
    clash::infrastructure::ide::detect_ides(&state.config_ides)
        .into_iter()
        .map(|i| IdeDto {
            label: i.label,
            description: i.description,
            value: i.value,
        })
        .collect()
}

/// Open a project directory in an IDE. `value` is the picker value
/// ("code", "cursor", or "terminal:nvim" for terminal editors).
#[tauri::command]
fn open_in_ide(value: String, project_dir: String) -> Result<(), String> {
    if let Some(cmd) = value.strip_prefix("terminal:") {
        // No host terminal in GUI context: Fallback strategy opens the
        // platform terminal app (Terminal.app / x-terminal-emulator).
        clash::infrastructure::windowing::terminal_spawn::open_command(
            cmd,
            &[&project_dir],
            None,
            false,
            0,
            0,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    } else {
        clash::infrastructure::ide::open_ide(&value, &project_dir)
    }
}

/// Stash all running sessions (the TUI's `S`): quit-stash marker first,
/// then statuses idle, then daemon kill — same order as the reducer.
#[tauri::command]
async fn stash_all(state: State<'_, GuiState>) -> Result<usize, String> {
    let mut control = state.control.lock().await;
    ensure_connected(&mut control).await;
    let infos = control.list_sessions().await.unwrap_or_default();
    let ids: Vec<String> = infos.iter().map(|i| i.session_id.clone()).collect();
    if ids.is_empty() {
        return Ok(0);
    }
    clash::infrastructure::hooks::write_quit_stashed(&ids);
    for id in &ids {
        clash::infrastructure::hooks::write_session_status(state.backend.base_dir(), id, "idle");
    }
    for id in &ids {
        let _ = control.kill_session(id).await;
    }
    state.attached.lock().await.clear();
    Ok(ids.len())
}

/// Adopt a wild claude process into the daemon (the TUI's takeover):
/// SIGTERM → wait → SIGKILL, then respawn with --resume under our daemon.
#[tauri::command]
async fn takeover_wild(
    state: State<'_, GuiState>,
    session_id: String,
    pid: u32,
    cwd: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    use clash::infrastructure::process_scan::{should_signal, LiveProcessProbe, SignalDecision};
    let probe = LiveProcessProbe;
    match should_signal(pid, &probe) {
        SignalDecision::Allow => {}
        _ => return Err("Process exited or changed — refresh and retry".to_string()),
    }
    let pid_i = pid as i32;
    unsafe { libc::kill(pid_i, libc::SIGTERM) };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut exited = false;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if unsafe { libc::kill(pid_i, 0) } != 0 {
            exited = true;
            break;
        }
    }
    if !exited {
        unsafe { libc::kill(pid_i, libc::SIGKILL) };
    }

    let mut control = state.control.lock().await;
    ensure_connected(&mut control).await;
    control
        .create_session(
            &session_id,
            &state.claude_bin,
            &["--resume".to_string(), session_id.clone()],
            if cwd.is_empty() { None } else { Some(&cwd) },
            None,
            cols,
            rows,
            HashMap::new(),
        )
        .await
        .map_err(|e| format!("Failed to adopt session: {}", e))
}

/// Track a wild session in clash without touching its process (the TUI's convert).
#[tauri::command]
fn convert_wild(session_id: String, name: String, cwd: String) -> Result<(), String> {
    clash::infrastructure::hooks::registry::register(
        &session_id,
        if name.trim().is_empty() {
            "session"
        } else {
            name.trim()
        },
        &cwd,
        None,
    );
    Ok(())
}

/// Create a team via the claude CLI (same args as the TUI's team create).
#[tauri::command]
async fn create_team(
    state: State<'_, GuiState>,
    name: String,
    description: String,
) -> Result<(), String> {
    let out = tokio::process::Command::new(&state.claude_bin)
        .args([
            "team",
            "create",
            "--name",
            &name,
            "--description",
            &description,
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to run {}: {}", state.claude_bin, e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

/// Delete a team and its tasks (same backend call as the TUI).
#[tauri::command]
fn delete_team(state: State<'_, GuiState>, name: String) -> Result<(), String> {
    use clash::domain::ports::DataRepository;
    state.backend.delete_team(&name).map_err(|e| e.to_string())
}

/// Update phase DTO (UpdatePhase has no Serialize derive).
#[derive(Clone, serde::Serialize)]
struct UpdatePhaseDto {
    phase: String,
    version: Option<String>,
    message: Option<String>,
}

/// Self-update: runs the shared updater (installs both clash and clash-gui),
/// streaming phases to the webview as `update-phase` events.
#[tauri::command]
fn start_update(app: tauri::AppHandle) {
    use clash::application::state::UpdatePhase;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        clash::infrastructure::update::perform_update(tx).await;
    });
    tauri::async_runtime::spawn(async move {
        while let Some(phase) = rx.recv().await {
            let dto = match phase {
                UpdatePhase::Checking => UpdatePhaseDto {
                    phase: "checking".into(),
                    version: None,
                    message: None,
                },
                UpdatePhase::Downloading { version } => UpdatePhaseDto {
                    phase: "downloading".into(),
                    version: Some(version),
                    message: None,
                },
                UpdatePhase::Extracting => UpdatePhaseDto {
                    phase: "extracting".into(),
                    version: None,
                    message: None,
                },
                UpdatePhase::Installing => UpdatePhaseDto {
                    phase: "installing".into(),
                    version: None,
                    message: None,
                },
                UpdatePhase::Done { version } => UpdatePhaseDto {
                    phase: "done".into(),
                    version: Some(version),
                    message: None,
                },
                UpdatePhase::Failed { message } => UpdatePhaseDto {
                    phase: "failed".into(),
                    version: None,
                    message: Some(message),
                },
            };
            let done = matches!(dto.phase.as_str(), "done" | "failed");
            let _ = app.emit("update-phase", dto);
            if done {
                break;
            }
        }
    });
}

#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Parse a terminal notification escape sequence from raw PTY output.
///
/// Supports the two formats cmux popularized for agent workflows:
/// - OSC 777: `ESC]777;notify;<title>;<body>(BEL|ESC\)`
/// - OSC 9:   `ESC]9;<message>(BEL|ESC\)`
fn parse_osc_notification(bytes: &[u8]) -> Option<(String, String)> {
    fn payload_until_st(rest: &[u8]) -> Option<&[u8]> {
        for (i, w) in rest.iter().enumerate() {
            match w {
                0x07 => return Some(&rest[..i]),
                0x1b if rest.get(i + 1) == Some(&b'\\') => return Some(&rest[..i]),
                _ => {}
            }
        }
        None
    }
    // OSC 777
    if let Some(pos) = bytes.windows(13).position(|w| w == b"\x1b]777;notify;") {
        let payload = payload_until_st(&bytes[pos + 13..])?;
        let text = String::from_utf8_lossy(payload);
        let (title, body) = text.split_once(';').unwrap_or((&text, ""));
        return Some((title.to_string(), body.to_string()));
    }
    // OSC 9
    if let Some(pos) = bytes.windows(3).position(|w| w == b"\x1b]9") {
        let rest = &bytes[pos + 3..];
        // Must be `ESC]9;` exactly — exclude OSC 99, OSC 9;4 progress etc.
        if rest.first() == Some(&b';') {
            let payload = payload_until_st(&rest[1..])?;
            let text = String::from_utf8_lossy(payload).to_string();
            if !text.is_empty() && !text.starts_with("4;") {
                return Some(("clash session".to_string(), text));
            }
        }
    }
    None
}

/// Listening TCP ports of a session's process tree (cmux shows ports per
/// workspace; we expose them per session, on demand from the details panel).
#[tauri::command]
async fn get_session_ports(
    state: State<'_, GuiState>,
    session_id: String,
) -> Result<Vec<String>, String> {
    // Root pid: daemon-owned session, or wild pid from the session list
    let pid = {
        let mut control = state.control.lock().await;
        ensure_connected(&mut control).await;
        let daemon_pid = control
            .list_sessions()
            .await
            .ok()
            .and_then(|infos| infos.into_iter().find(|i| i.session_id == session_id))
            .map(|i| i.pid);
        daemon_pid.or_else(|| {
            let prev = state.previous.lock().unwrap();
            prev.iter()
                .find(|s| s.id == session_id)
                .and_then(|s| s.wild_pid)
        })
    };
    let Some(root) = pid else {
        return Ok(Vec::new());
    };

    // Collect the process tree (root + descendants), then lsof their ports.
    let mut pids = vec![root.to_string()];
    if let Ok(out) = tokio::process::Command::new("pgrep")
        .args(["-P", &root.to_string()])
        .output()
        .await
    {
        pids.extend(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty()),
        );
    }
    let out = tokio::process::Command::new("lsof")
        .args([
            "-a",
            "-iTCP",
            "-sTCP:LISTEN",
            "-P",
            "-n",
            "-p",
            &pids.join(","),
        ])
        .output()
        .await
        .map_err(|e| format!("lsof failed: {}", e))?;
    let mut ports: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter_map(|line| {
            line.split_whitespace()
                .nth(8)
                .and_then(|addr| addr.rsplit(':').next())
                .map(String::from)
        })
        .collect();
    ports.sort();
    ports.dedup();
    Ok(ports)
}

/// (Re)connect the control client if the connection was lost.
async fn ensure_connected(client: &mut DaemonClient) {
    if !client.is_connected() {
        let _ = client.connect().await;
    }
}

/// Quit-stash on app exit — same sequence as the TUI's quit: marker first,
/// then statuses idle, then daemon kill. Runs synchronously in the close
/// handler so it completes before the process exits.
fn quit_stash(state: &GuiState) {
    tauri::async_runtime::block_on(async {
        let mut control = state.control.lock().await;
        ensure_connected(&mut control).await;
        let infos = control.list_sessions().await.unwrap_or_default();
        let ids: Vec<String> = infos.iter().map(|i| i.session_id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        clash::infrastructure::hooks::write_quit_stashed(&ids);
        for id in &ids {
            clash::infrastructure::hooks::write_session_status(
                state.backend.base_dir(),
                id,
                "idle",
            );
        }
        for id in &ids {
            let _ = control.kill_session(id).await;
        }
    });
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let config = Config::load();
    let data_dir: PathBuf = config.claude_dir();
    let claude_bin = config.claude_bin.clone();

    let state = GuiState {
        backend: FsBackend::new(data_dir),
        claude_bin,
        config_ides: config.ides.clone(),
        previous: Mutex::new(Vec::new()),
        prev_statuses: Mutex::new(HashMap::new()),
        control: tokio::sync::Mutex::new(DaemonClient::new(DaemonClient::instance_socket_path())),
        attached: tokio::sync::Mutex::new(HashMap::new()),
    };

    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            // In-process PTY session manager — the GUI's backbone, identical
            // to the TUI's in-process daemon. Dies with the app. Per-instance
            // socket (daemon-<pid>.sock): TUI and GUI can run side by side,
            // each owning its own sessions.
            let server = DaemonServer::new(DaemonClient::instance_socket_path());
            let shutdown = server.shutdown_handle();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server.run().await {
                    tracing::error!("Daemon server error: {}", e);
                }
            });
            // Keep the shutdown handle alive for the app's lifetime.
            app.manage(shutdown);

            // Dock icon. macOS only reads icons from an .app bundle's
            // Info.plist; the bare dev binary must set one at runtime.
            #[cfg(target_os = "macos")]
            {
                use objc2::{AnyThread, MainThreadMarker};
                use objc2_app_kit::{NSApplication, NSImage};
                use objc2_foundation::NSData;
                if let Some(mtm) = MainThreadMarker::new() {
                    let data = NSData::with_bytes(include_bytes!("../icons/icon.png"));
                    if let Some(img) = NSImage::initWithData(NSImage::alloc(), &data) {
                        // SAFETY: main thread (MainThreadMarker), valid NSImage.
                        unsafe {
                            NSApplication::sharedApplication(mtm)
                                .setApplicationIconImage(Some(&img));
                        }
                    }
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Quit-stash before shutdown so sessions show as resumable
                // (stashed) instead of errored on the next launch.
                if let Some(state) = window.app_handle().try_state::<GuiState>() {
                    quit_stash(&state);
                }
                if let Some(shutdown) = window
                    .app_handle()
                    .try_state::<std::sync::Arc<tokio::sync::Notify>>()
                {
                    shutdown.notify_one();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            open_session,
            send_input,
            resize_session,
            close_session,
            stash_session,
            kill_session,
            rename_session,
            create_new_session,
            create_worktree_session,
            get_diff,
            get_conversation,
            get_subagents,
            get_subagent_conversation,
            get_inbox,
            list_teams,
            list_tasks,
            list_presets,
            detect_ides,
            open_in_ide,
            stash_all,
            takeover_wild,
            convert_wild,
            create_team,
            delete_team,
            start_update,
            get_version,
            get_session_ports
        ])
        .run(tauri::generate_context!())
        .expect("error while running clash GUI");
}
