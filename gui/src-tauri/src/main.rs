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
    /// Latest wild-process scan snapshot (background task, same scan the
    /// TUI runs). Read into `RefreshInput.wild_processes` on each refresh.
    wild_processes_rx:
        tokio::sync::watch::Receiver<Vec<clash::infrastructure::process_scan::WildProcess>>,
    /// Sessions killed via the GUI, with a per-refresh age counter — the
    /// GUI equivalent of the TUI's `recently_removed`. A dying claude
    /// keeps being reported by the daemon (until /exit→SIGTERM→SIGKILL
    /// lands and the 5s reaper sweeps) and by the wild scan (until its
    /// next tick), so `build_session_list` would re-admit the row; worse,
    /// once re-admitted into `previous` as running, the empty-daemon
    /// "hiccup" preservation keeps it alive forever. Entries are filtered
    /// out of `list_sessions` results and expire after
    /// `RECENTLY_REMOVED_TTL` refresh cycles.
    recently_removed: Mutex<HashMap<String, u8>>,
    /// Native desktop notifications on session attention (GUI setting,
    /// pushed by the frontend at boot and on toggle).
    notify_enabled: std::sync::atomic::AtomicBool,
}

/// Refresh cycles a killed session stays filtered from `list_sessions`.
/// Must outlive the worst-case dying window: 3s /exit grace + 3s SIGTERM
/// grace + 5s daemon reaper + a wild-scan tick ≈ 12s. At the frontend's
/// 2s poll cadence, 10 cycles ≈ 20s.
const RECENTLY_REMOVED_TTL: u8 = 10;

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
    input.wild_processes = state.wild_processes_rx.borrow().clone();
    let mut sessions = session_refresh::build_session_list(&input);

    // Drop freshly killed sessions the pipeline re-admitted (dying daemon
    // process, stale wild-scan snapshot), and age out the guard entries.
    {
        let mut removed = state.recently_removed.lock().unwrap();
        if !removed.is_empty() {
            sessions.retain(|s| !removed.contains_key(&s.id));
            removed.values_mut().for_each(|v| *v = v.saturating_add(1));
            removed.retain(|_, age| *age <= RECENTLY_REMOVED_TTL);
        }
    }

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
                if !focused
                    && state
                        .notify_enabled
                        .load(std::sync::atomic::Ordering::Relaxed)
                {
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

/// Kill a session and remove it from the registry (same as the TUI's `x`):
/// unregister, idle status, daemon kill, then process-tree teardown — the
/// wild scan would otherwise re-admit a surviving claude process and the
/// session would pop back into the list.
#[tauri::command]
async fn kill_session(state: State<'_, GuiState>, session_id: String) -> Result<(), String> {
    let (worktree, wild_pid) = {
        let mut prev = state.previous.lock().unwrap();
        let s = prev.iter().find(|s| s.id == session_id);
        let extracted = (
            s.and_then(|s| s.worktree.clone()),
            s.and_then(|s| s.wild_pid),
        );
        // Purge from the merge input immediately — a killed session left in
        // `previous` as running gets resurrected by the empty-daemon
        // preservation branch of the refresh pipeline.
        prev.retain(|s| s.id != session_id);
        extracted
    };
    state
        .recently_removed
        .lock()
        .unwrap()
        .insert(session_id.clone(), 0);
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
    // Idle now, so the daemon overlay won't re-admit the dying process
    // (`hook_says_idle` guard); re-written after death below in case the
    // dying Claude's Stop hook overwrites it meanwhile.
    clash::infrastructure::hooks::write_session_status(
        state.backend.base_dir(),
        &session_id,
        "idle",
    );

    let base_dir = state.backend.base_dir().to_path_buf();
    tauri::async_runtime::spawn(async move {
        use clash::infrastructure::app::{
            kill_tmux_session, remove_git_worktree, terminate_claude_process, terminate_pid_if_safe,
        };
        if let Some(pid) = wild_pid {
            terminate_pid_if_safe(pid).await;
        }
        terminate_claude_process(&session_id).await;
        if let Some(wt) = worktree {
            kill_tmux_session(&wt).await;
            remove_git_worktree(&wt).await;
        }
        // After the process is dead, force idle so a dying Claude's Stop
        // hook can't strand the row in Waiting.
        clash::infrastructure::hooks::write_session_status(&base_dir, &session_id, "idle");
    });
    Ok(())
}

/// Rename a session — same writes the TUI's rename effect does. The registry
/// rename alone is a no-op for sessions clash didn't spawn (not in the
/// registry), so the saved-names write is what makes rename stick for them.
#[tauri::command]
fn rename_session(
    state: State<'_, GuiState>,
    session_id: String,
    name: String,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Name cannot be empty".to_string());
    }
    clash::infrastructure::hooks::registry::rename(&session_id, name);
    let cwd = {
        let prev = state.previous.lock().unwrap();
        prev.iter().find(|s| s.id == session_id).and_then(|s| {
            s.cwd
                .clone()
                .or_else(|| (!s.project_path.is_empty()).then(|| s.project_path.clone()))
        })
    };
    clash::infrastructure::hooks::save_session_name(
        state.backend.base_dir(),
        &session_id,
        name,
        cwd.as_deref(),
    );
    Ok(())
}

/// Is a clash TUI process running anywhere? Exact binary-name match so
/// `clash-gui` (this process) never counts.
#[tauri::command]
fn tui_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "clash"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Terminal emulators detected on this machine — the choices offered by
/// the TUI launcher's picker.
#[tauri::command]
fn list_terminals() -> Vec<clash::infrastructure::windowing::terminal_choice::DetectedTerminal> {
    clash::infrastructure::windowing::terminal_choice::detect_terminals()
}

/// Launch the clash TUI alongside the GUI. With a `terminal` id (from
/// `list_terminals`), a new window of that emulator; without one, auto —
/// a split pane when the GUI was started from a pane-capable terminal,
/// otherwise the platform's default terminal. Prefers the sibling `clash`
/// binary next to this executable, falling back to PATH.
#[tauri::command]
fn launch_tui(terminal: Option<String>) -> Result<(), String> {
    let exe = resolve_tui_binary()?;
    if let Some(id) = terminal.as_deref().filter(|id| !id.is_empty()) {
        return clash::infrastructure::windowing::terminal_choice::open_window_in(id, &exe, &[])
            .map_err(|e| e.to_string());
    }
    let term_program = std::env::var("TERM_PROGRAM").ok();
    let in_tmux = std::env::var("TMUX").is_ok();
    clash::infrastructure::windowing::terminal_spawn::open_command(
        &exe,
        &[],
        term_program.as_deref(),
        in_tmux,
        200,
        50,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Find the `clash` TUI binary: sibling of this executable first (dev
/// builds, plain installs), then PATH (`which` — the GUI adopted the
/// login-shell PATH at startup, so this works from Finder launches too).
/// The app-bundle GUI has no sibling, and handing a bare `"clash"` to the
/// spawn layer used to fail silently when the caller's cwd was `/`.
fn resolve_tui_binary() -> Result<String, String> {
    if let Ok(exe) = std::env::current_exe() {
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        if let Some(sibling) = exe.parent().map(|d| d.join("clash")) {
            if sibling.exists() {
                return Ok(sibling.to_string_lossy().into_owned());
            }
        }
    }
    if let Ok(out) = std::process::Command::new("which").arg("clash").output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }
    Err(
        "clash TUI binary not found — run `clash update` from a terminal or put clash on PATH"
            .to_string(),
    )
}

/// Enable/disable native desktop notifications. Pushed by the frontend at
/// boot (from persisted settings) and whenever the user flips the toggle.
#[tauri::command]
fn set_notifications_enabled(state: State<'_, GuiState>, enabled: bool) {
    state
        .notify_enabled
        .store(enabled, std::sync::atomic::Ordering::Relaxed);
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

/// Working directory of a session — cwd, falling back to project path.
fn session_dir(state: &GuiState, session_id: &str) -> Result<String, String> {
    let prev = state.previous.lock().unwrap();
    prev.iter()
        .find(|s| s.id == session_id)
        .and_then(|s| {
            s.cwd
                .clone()
                .filter(|c| !c.is_empty())
                .or_else(|| Some(s.project_path.clone()).filter(|p| !p.is_empty()))
        })
        .ok_or_else(|| "No working directory for session".to_string())
}

/// `git diff HEAD` for a session's working directory (same as the TUI's diff view).
#[tauri::command]
async fn get_diff(state: State<'_, GuiState>, session_id: String) -> Result<String, String> {
    let dir = session_dir(&state, &session_id)?;
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

/// User's home directory — last-resort prefill for the new-session cwd.
#[tauri::command]
fn get_home_dir() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// Web URL of the session repo's `origin` remote, normalized to https
/// (ssh `git@host:owner/repo.git` forms included) — lets the embedded
/// browser open the code on the forge.
#[tauri::command]
async fn get_repo_url(state: State<'_, GuiState>, session_id: String) -> Result<String, String> {
    let dir = session_dir(&state, &session_id)?;
    let output = tokio::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(&dir)
        .output()
        .await
        .map_err(|e| format!("git remote failed: {}", e))?;
    if !output.status.success() {
        return Err("No origin remote".to_string());
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    remote_to_web_url(&raw).ok_or_else(|| format!("Unsupported remote url: {}", raw))
}

/// Default branch of the session repo (from `origin/HEAD`) — the base of
/// GitHub compare links. Falls back to `main` when origin/HEAD isn't set
/// locally (clones made before git started recording it).
#[tauri::command]
async fn get_default_branch(
    state: State<'_, GuiState>,
    session_id: String,
) -> Result<String, String> {
    let dir = session_dir(&state, &session_id)?;
    let output = tokio::process::Command::new("git")
        .args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .current_dir(&dir)
        .output()
        .await
        .map_err(|e| format!("git symbolic-ref failed: {}", e))?;
    if output.status.success() {
        let full = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(full.strip_prefix("origin/").unwrap_or(&full).to_string())
    } else {
        Ok("main".to_string())
    }
}

/// `git@host:owner/repo.git` / `ssh://git@host/owner/repo.git` /
/// `https://host/owner/repo.git` → `https://host/owner/repo`.
fn remote_to_web_url(raw: &str) -> Option<String> {
    let raw = raw.strip_suffix(".git").unwrap_or(raw);
    if let Some(rest) = raw.strip_prefix("https://").or(raw.strip_prefix("http://")) {
        return Some(format!("https://{}", rest));
    }
    if let Some(rest) = raw.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
        return Some(format!("https://{}", rest));
    }
    if let Some((host, path)) = raw.split_once('@').and_then(|(_, r)| r.split_once(':')) {
        return Some(format!("https://{}/{}", host, path));
    }
    None
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

/// Claude Code's project-dir encoding: every non-alphanumeric byte becomes
/// `-` (e.g. `/Users/x/alumni_connect` → `-Users-x-alumni-connect`).
fn encoded_project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Project-dir candidates for a session's transcripts. Daemon-only sessions
/// (spawned this launch, not yet merged with a disk scan) carry the cwd's
/// last component in `project` instead of the real encoded directory — fall
/// back to encoding the session's cwd / project path.
fn project_dir_candidates(state: &GuiState, project: &str, session_id: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    if !project.is_empty() {
        candidates.push(project.to_string());
    }
    let prev = state.previous.lock().unwrap();
    if let Some(s) = prev.iter().find(|s| s.id == session_id) {
        for path in [s.cwd.as_deref(), Some(s.project_path.as_str())]
            .into_iter()
            .flatten()
            .filter(|p| !p.is_empty())
        {
            let encoded = encoded_project_dir(path);
            if !candidates.contains(&encoded) {
                candidates.push(encoded);
            }
        }
    }
    candidates
}

/// Conversation transcript of a session (same parser as the TUI detail view).
#[tauri::command]
fn get_conversation(
    state: State<'_, GuiState>,
    project: String,
    session_id: String,
) -> Result<Vec<MessageDto>, String> {
    use clash::domain::ports::DataRepository;
    let mut messages = Vec::new();
    for proj in project_dir_candidates(&state, &project, &session_id) {
        messages = state
            .backend
            .load_conversation(&proj, &session_id)
            .map_err(|e| e.to_string())?;
        if !messages.is_empty() {
            break;
        }
    }
    Ok(to_message_dtos(messages))
}

/// Subagents of a session.
#[tauri::command]
fn get_subagents(
    state: State<'_, GuiState>,
    project: String,
    session_id: String,
) -> Result<Vec<clash::domain::entities::Subagent>, String> {
    use clash::domain::ports::DataRepository;
    let mut subagents = Vec::new();
    for proj in project_dir_candidates(&state, &project, &session_id) {
        subagents = state
            .backend
            .load_subagents(&proj, &session_id)
            .map_err(|e| e.to_string())?;
        if !subagents.is_empty() {
            break;
        }
    }
    Ok(subagents)
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
    let mut messages = Vec::new();
    for proj in project_dir_candidates(&state, &project, &session_id) {
        messages = state
            .backend
            .load_subagent_conversation(&proj, &session_id, &agent_id)
            .map_err(|e| e.to_string())?;
        if !messages.is_empty() {
            break;
        }
    }
    Ok(to_message_dtos(messages))
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
/// best-effort kill of the outside process, then respawn with --resume
/// under our daemon. The frontend opens the terminal right after, so
/// takeover-and-attach is a single click.
#[tauri::command]
async fn takeover_wild(
    state: State<'_, GuiState>,
    session_id: String,
    pid: u32,
    cwd: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    clash::infrastructure::process_scan::kill_wild_process(pid).await;

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

/// Create a team directly on the filesystem (config.json under the teams
/// dir — same write the TUI's team create performs). `claude team create`
/// is NOT a real CLI subcommand; shelling out silently fed it as a prompt.
#[tauri::command]
fn create_team(
    state: State<'_, GuiState>,
    name: String,
    description: String,
) -> Result<(), String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .create_team(name.trim(), description.trim())
        .map_err(|e| e.to_string())
}

/// Delete a team and its tasks (same backend call as the TUI).
#[tauri::command]
fn delete_team(state: State<'_, GuiState>, name: String) -> Result<(), String> {
    use clash::domain::ports::DataRepository;
    state.backend.delete_team(&name).map_err(|e| e.to_string())
}

/// Load a team fresh from disk, apply a pure mutation (the shared
/// `Team` helpers — same logic the TUI reducer uses), persist atomically.
fn mutate_team(
    state: &State<'_, GuiState>,
    name: &str,
    change: impl FnOnce(&mut clash::domain::entities::Team) -> Result<(), String>,
) -> Result<(), String> {
    use clash::domain::ports::DataRepository;
    let mut team = state
        .backend
        .load_teams()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| format!("Team '{}' not found", name))?;
    change(&mut team)?;
    state.backend.update_team(&team).map_err(|e| e.to_string())
}

/// Replace a team's description.
#[tauri::command]
fn update_team_description(
    state: State<'_, GuiState>,
    name: String,
    description: String,
) -> Result<(), String> {
    mutate_team(&state, &name, |t| {
        t.set_description(&description);
        Ok(())
    })
}

/// Add a member to a team (empty agent_type = general-purpose, empty
/// model = inherit).
#[tauri::command]
fn add_team_member(
    state: State<'_, GuiState>,
    team: String,
    name: String,
    agent_type: String,
    model: String,
) -> Result<(), String> {
    mutate_team(&state, &team, |t| t.add_member(&name, &agent_type, &model))
}

/// Remove a member from a team by name.
#[tauri::command]
fn remove_team_member(
    state: State<'_, GuiState>,
    team: String,
    member: String,
) -> Result<(), String> {
    mutate_team(&state, &team, |t| t.remove_member(&member))
}

/// Change a member's model (empty = inherit).
#[tauri::command]
fn set_team_member_model(
    state: State<'_, GuiState>,
    team: String,
    member: String,
    model: String,
) -> Result<(), String> {
    mutate_team(&state, &team, |t| t.set_member_model(&member, &model))
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

/// Relaunch the app binary — offered after a self-update so the new
/// version takes over. The in-process daemon (and every PTY session it
/// owns) dies with the old process; the frontend confirms first.
#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
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

// ── GUI state persistence (workspaces: layout + session ownership) ──
// Disk-backed because the bare-binary WKWebView's localStorage is not
// reliably persisted across restarts.

fn gui_state_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("clash").join("gui-state.json"))
}

#[tauri::command]
fn save_gui_state(state_json: String) -> Result<(), String> {
    let path = gui_state_path().ok_or("no data dir")?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    clash::infrastructure::fs::atomic::write_atomic(&path, state_json.as_bytes())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn load_gui_state() -> Result<String, String> {
    let path = gui_state_path().ok_or("no data dir")?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
    }
}

// ── Embedded browser panel (cmux-style child webviews, one per tab) ─

const BROWSER_LABEL_PREFIX: &str = "embedded-browser";

fn browser_tab_label(tab: &str) -> String {
    format!("{}-{}", BROWSER_LABEL_PREFIX, tab)
}

fn browser_tab(app: &tauri::AppHandle, tab: &str) -> Option<tauri::Webview> {
    app.webviews().get(&browser_tab_label(tab)).cloned()
}

/// All live browser-tab webviews (the main app webview is excluded).
fn browser_tabs(app: &tauri::AppHandle) -> Vec<tauri::Webview> {
    app.webviews()
        .into_iter()
        .filter(|(label, _)| label.starts_with(BROWSER_LABEL_PREFIX))
        .map(|(_, wv)| wv)
        .collect()
}

fn set_browser_bounds(wv: &tauri::Webview, x: f64, y: f64, w: f64, h: f64) {
    let _ = wv.set_position(tauri::LogicalPosition::new(x, y));
    let _ = wv.set_size(tauri::LogicalSize::new(w, h));
}

/// Show `tab` and hide every other browser tab (tabs are stacked child
/// webviews over the same `#browser-slot` rect).
fn raise_browser_tab(app: &tauri::AppHandle, tab: &str) {
    let label = browser_tab_label(tab);
    for wv in browser_tabs(app) {
        if wv.label() == label {
            let _ = wv.show();
        } else {
            let _ = wv.hide();
        }
    }
}

/// Open (or navigate) the browser tab's child webview, positioned over
/// the `#browser-slot` placeholder rect reported by the frontend.
#[tauri::command]
async fn browser_open(
    app: tauri::AppHandle,
    tab: String,
    url: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    let parsed: tauri::Url = url.parse().map_err(|e| format!("bad url: {}", e))?;
    if let Some(wv) = browser_tab(&app, &tab) {
        wv.navigate(parsed).map_err(|e| e.to_string())?;
        set_browser_bounds(&wv, x, y, w, h);
        raise_browser_tab(&app, &tab);
        return Ok(());
    }
    let win = app.get_window("main").ok_or("no main window")?;
    // add_child must run on the main thread on macOS.
    let win2 = win.clone();
    let label = browser_tab_label(&tab);
    let (tx, rx) = tokio::sync::oneshot::channel();
    win.run_on_main_thread(move || {
        let res = win2
            .add_child(
                tauri::webview::WebviewBuilder::new(label, tauri::WebviewUrl::External(parsed)),
                tauri::LogicalPosition::new(x, y),
                tauri::LogicalSize::new(w, h),
            )
            .map(|_| ())
            .map_err(|e| e.to_string());
        let _ = tx.send(res);
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())??;
    raise_browser_tab(&app, &tab);
    Ok(())
}

/// Bring an existing tab to the front (tab strip click).
#[tauri::command]
fn browser_select(
    app: tauri::AppHandle,
    tab: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    let wv = browser_tab(&app, &tab).ok_or("no such tab")?;
    set_browser_bounds(&wv, x, y, w, h);
    raise_browser_tab(&app, &tab);
    Ok(())
}

/// Reposition all browser tabs over the placeholder (layout changed) —
/// hidden tabs too, so they come back correctly sized.
#[tauri::command]
fn browser_bounds(app: tauri::AppHandle, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    for wv in browser_tabs(&app) {
        set_browser_bounds(&wv, x, y, w, h);
    }
    Ok(())
}

#[tauri::command]
fn browser_navigate(app: tauri::AppHandle, tab: String, url: String) -> Result<(), String> {
    let parsed: tauri::Url = url.parse().map_err(|e| format!("bad url: {}", e))?;
    browser_tab(&app, &tab)
        .ok_or("no such tab")?
        .navigate(parsed)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn browser_history(app: tauri::AppHandle, tab: String, delta: i32) -> Result<(), String> {
    browser_tab(&app, &tab)
        .ok_or("no such tab")?
        .eval(format!("history.go({})", delta))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn browser_reload(app: tauri::AppHandle, tab: String) -> Result<(), String> {
    browser_tab(&app, &tab)
        .ok_or("no such tab")?
        .eval("location.reload()")
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn browser_get_url(app: tauri::AppHandle, tab: String) -> Result<String, String> {
    let wv = browser_tab(&app, &tab).ok_or("no such tab")?;
    wv.url().map(|u| u.to_string()).map_err(|e| e.to_string())
}

/// Close one tab's webview.
#[tauri::command]
fn browser_close_tab(app: tauri::AppHandle, tab: String) -> Result<(), String> {
    if let Some(wv) = browser_tab(&app, &tab) {
        let _ = wv.close();
    }
    Ok(())
}

/// Close the whole panel: every tab's webview.
#[tauri::command]
fn browser_close(app: tauri::AppHandle) -> Result<(), String> {
    for wv in browser_tabs(&app) {
        let _ = wv.close();
    }
    Ok(())
}

/// Open a URL in the system default browser.
#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("only http(s) urls".to_string());
    }
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(not(target_os = "macos"))]
    let cmd = "xdg-open";
    std::process::Command::new(cmd)
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
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

    // Launched from Finder/Dock the app gets launchd's minimal PATH —
    // without this, `claude` can't be found and session spawns fail.
    clash::infrastructure::env_path::adopt_login_shell_path();

    let config = Config::load();
    let data_dir: PathBuf = config.claude_dir();
    let claude_bin = config.claude_bin.clone();

    let (wild_processes_tx, wild_processes_rx) = tokio::sync::watch::channel(Vec::new());

    let state = GuiState {
        backend: FsBackend::new(data_dir),
        claude_bin,
        config_ides: config.ides.clone(),
        previous: Mutex::new(Vec::new()),
        prev_statuses: Mutex::new(HashMap::new()),
        control: tokio::sync::Mutex::new(DaemonClient::new(DaemonClient::instance_socket_path())),
        attached: tokio::sync::Mutex::new(HashMap::new()),
        wild_processes_rx,
        recently_removed: Mutex::new(HashMap::new()),
        notify_enabled: std::sync::atomic::AtomicBool::new(true),
    };

    // FS watcher on ~/.claude/projects — same role as the TUI's watcher
    // wiring in app.rs. Without it the FsBackend session cache goes stale
    // after the first scan: sessions created during this launch never gain
    // their disk metadata (encoded project dir, summary), so conversation
    // and subagent lookups resolve to a non-existent path.
    let (fs_tx, mut fs_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<PathBuf>>();
    let fs_watcher = clash::infrastructure::fs::watcher::FsWatcher::new(
        &[state.backend.projects_dir()],
        fs_tx,
        std::time::Duration::from_millis(config.debounce_ms),
    )
    .map_err(|e| tracing::warn!("FS watcher unavailable: {}", e))
    .ok();

    tauri::Builder::default()
        .manage(state)
        .setup(move |app| {
            if let Some(watcher) = fs_watcher {
                // Keep the watcher alive for the app's lifetime.
                app.manage(Mutex::new(watcher));
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    while let Some(paths) = fs_rx.recv().await {
                        let state = handle.state::<GuiState>();
                        let jsonl: Vec<PathBuf> = paths
                            .iter()
                            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
                            .cloned()
                            .collect();
                        if !jsonl.is_empty() && jsonl.len() == paths.len() {
                            state.backend.invalidate_session_cache(&jsonl);
                        } else {
                            // Non-jsonl change (sessions-index.json, new
                            // project dir…) — full rescan on next load.
                            state.backend.invalidate_session_cache_all();
                        }
                    }
                });
            }
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

            // Wild-process background scan — same scan the TUI runs.
            // Unlike the TUI (which hides claudes started before launch),
            // the GUI shows ALL live external claudes: its session list is
            // the user's overview of everything running on the machine.
            tauri::async_runtime::spawn(async move {
                use clash::infrastructure::process_scan::{
                    default_fd_probe, gather_wild_processes,
                };
                let probe = default_fd_probe();
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let wild = gather_wild_processes(&probe);
                    if wild_processes_tx.send(wild).is_err() {
                        break;
                    }
                }
            });

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
            set_notifications_enabled,
            tui_running,
            launch_tui,
            list_terminals,
            create_new_session,
            create_worktree_session,
            get_diff,
            get_repo_url,
            get_default_branch,
            get_home_dir,
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
            create_team,
            delete_team,
            update_team_description,
            add_team_member,
            remove_team_member,
            set_team_member_model,
            start_update,
            get_version,
            restart_app,
            get_session_ports,
            browser_open,
            browser_select,
            browser_bounds,
            browser_navigate,
            browser_history,
            browser_reload,
            browser_get_url,
            browser_close_tab,
            browser_close,
            open_external,
            save_gui_state,
            load_gui_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running clash GUI");
}

#[cfg(test)]
mod tests {
    use super::remote_to_web_url;

    #[test]
    fn remote_to_web_url_forms() {
        assert_eq!(
            remote_to_web_url("git@github.com:owner/repo.git").as_deref(),
            Some("https://github.com/owner/repo")
        );
        assert_eq!(
            remote_to_web_url("https://github.com/owner/repo.git").as_deref(),
            Some("https://github.com/owner/repo")
        );
        assert_eq!(
            remote_to_web_url("ssh://git@gitlab.com/owner/repo.git").as_deref(),
            Some("https://gitlab.com/owner/repo")
        );
        assert_eq!(remote_to_web_url("/local/bare/repo.git"), None);
    }
}
