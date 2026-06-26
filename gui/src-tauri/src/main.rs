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
use clash::infrastructure::fs::backend::{encode_project_dir, FsBackend};
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

/// Working dir + display name for resuming a Claude session with `--resume`.
/// `claude --resume <id>` only finds the conversation when launched from the
/// session's original project directory; a wrong or empty cwd surfaces to the
/// user as "No conversation found with session ID". Resolve defensively, in
/// order of freshness: the last-known session list, then the hooks registry
/// (clash-spawned sessions record their cwd there), then a fresh disk scan.
fn resume_context(state: &GuiState, session_id: &str) -> (String, Option<String>) {
    let pick = |cwd: Option<String>, project: String| -> Option<String> {
        cwd.filter(|c| !c.is_empty())
            .or(Some(project).filter(|p| !p.is_empty()))
    };
    {
        let prev = state.previous.lock().unwrap();
        if let Some(s) = prev.iter().find(|s| s.id == session_id) {
            if let Some(cwd) = pick(s.cwd.clone(), s.project_path.clone()) {
                return (cwd, s.name.clone());
            }
        }
    }
    let registry = clash::infrastructure::hooks::registry::load();
    if let Some(entry) = registry.get(session_id) {
        if !entry.cwd.is_empty() {
            return (entry.cwd.clone(), Some(entry.name.clone()));
        }
    }
    // Last resort: rebuild the session list straight from disk (no daemon /
    // wild input — just the on-disk record for this session's project dir).
    let input = session_refresh::gather_sync_input(&state.backend, &[], registry);
    if let Some(s) = session_refresh::build_session_list(&input)
        .iter()
        .find(|s| s.id == session_id)
    {
        if let Some(cwd) = pick(s.cwd.clone(), s.project_path.clone()) {
            return (cwd, s.name.clone());
        }
    }
    (String::new(), None)
}

/// True if a resumable Claude conversation transcript exists on disk for this
/// session. `claude --resume <id>` succeeds only when the `<id>.jsonl` is
/// present and non-empty; without it Claude exits 1. Checks every plausible
/// encoded project dir (resolved cwd plus the last-known session's cwd /
/// project path) to tolerate worktrees and daemon-only sessions.
fn has_resumable_conversation(state: &GuiState, session_id: &str, cwd: &str) -> bool {
    // Canonical-path-aware check shared with the TUI (handles symlinked cwds
    // like /tmp → /private/tmp, which Claude encodes from the resolved path).
    // Covers both the raw and canonicalized encodings of `cwd`.
    if !cwd.is_empty() && state.backend.has_resumable_transcript(cwd, session_id) {
        return true;
    }
    // Fall back to dirs derived from the last-known session list (a daemon-only
    // session may record a different cwd / project_path than the one passed in).
    let projects = state.backend.projects_dir();
    let dirs = project_dir_candidates(state, "", session_id);
    dirs.iter().any(|d| {
        let f = projects.join(d).join(format!("{session_id}.jsonl"));
        std::fs::metadata(&f).map(|m| m.len() > 0).unwrap_or(false)
    })
}

/// Resolve persisted session ids forward to their current conversation id.
/// Used by the frontend at restore time so a workspace pane saved before a
/// `/clear` points at the latest conversation (and matches `list_sessions`)
/// instead of being dropped or resuming a stale snapshot. Unknown ids pass
/// through unchanged.
#[tauri::command]
fn resolve_session_ids(_state: State<'_, GuiState>, ids: Vec<String>) -> Vec<String> {
    let registry = clash::infrastructure::hooks::registry::load();
    ids.into_iter()
        .map(|id| {
            clash::infrastructure::hooks::registry::resolve_resume_id(&registry, &id).unwrap_or(id)
        })
        .collect()
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
        let (cwd, name) = resume_context(&state, &session_id);
        // A persisted id may be stale: after a `/clear` the hook re-keys the
        // registry to the new conversation id and records the old one in the
        // lineage. Resolve forward so we resume the LATEST conversation, not a
        // stale pre-`/clear` snapshot.
        let resume_id = {
            let registry = clash::infrastructure::hooks::registry::load();
            clash::infrastructure::hooks::registry::resolve_resume_id(&registry, &session_id)
                .unwrap_or_else(|| session_id.clone())
        };
        // `claude --resume <id>` only works when the conversation transcript
        // exists on disk. A session created with `--session-id` but never
        // messaged (e.g. a fresh tab stashed on quit, then restored) — or a
        // stale id with no surviving transcript — has none, so `--resume`
        // makes Claude exit 1 ("No conversation found") and leaves a dead
        // terminal where Enter does nothing. In that case start the session
        // FRESH under the GUI's id so it behaves like a brand-new session.
        let args = if has_resumable_conversation(&state, &resume_id, &cwd) {
            vec!["--resume".to_string(), resume_id]
        } else {
            vec!["--session-id".to_string(), session_id.clone()]
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
                &args,
                if cwd.is_empty() { None } else { Some(&cwd) },
                name,
                cols,
                rows,
                HashMap::new(),
                true, // TUI: Claude sets its own termios
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

/// Login shells available on this machine — choices for `create_terminal`.
#[tauri::command]
fn list_shells() -> Vec<String> {
    clash::infrastructure::windowing::terminal_choice::detect_shells()
}

/// Spawn a plain shell terminal in the in-process daemon. Shell PTYs use
/// the `shellterm-` id namespace, which the refresh pipeline excludes —
/// they live as GUI tabs/panes, never as Claude sessions. Dies with the
/// app (the daemon is in-process), so nothing is registered or persisted.
#[tauri::command]
async fn create_terminal(
    state: State<'_, GuiState>,
    shell: Option<String>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let shell = shell
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "/bin/sh".to_string());
    let cwd = cwd
        .filter(|c| !c.is_empty() && std::path::Path::new(c).is_dir())
        .or_else(|| std::env::var("HOME").ok());
    let session_id = format!(
        "{}{}",
        session_refresh::SHELL_TERMINAL_ID_PREFIX,
        uuid::Uuid::now_v7()
    );
    let name = std::path::Path::new(&shell)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| shell.clone());

    let mut control = state.control.lock().await;
    ensure_connected(&mut control).await;
    control
        .create_session(
            &session_id,
            &shell,
            // Login shell: loads the user's profile/rc, like a fresh
            // terminal window would.
            &["-l".to_string()],
            cwd.as_deref(),
            Some(name),
            cols,
            rows,
            HashMap::new(),
            // Interactive shell: keep the default cooked termios so Ctrl+C
            // and friends work — unlike Claude, a shell does not reset it.
            false,
        )
        .await
        .map_err(|e| format!("Failed to spawn terminal: {}", e))?;
    Ok(session_id)
}

/// Kill a shell terminal's PTY. Unlike `kill_session`, shell terminals
/// have no registry entry, hook status, worktree, or wild process — the
/// daemon kill is the whole job.
#[tauri::command]
async fn close_terminal(state: State<'_, GuiState>, session_id: String) -> Result<(), String> {
    if !session_id.starts_with(session_refresh::SHELL_TERMINAL_ID_PREFIX) {
        return Err("Not a shell terminal".to_string());
    }
    state.attached.lock().await.remove(&session_id);
    let mut control = state.control.lock().await;
    ensure_connected(&mut control).await;
    control
        .kill_session(&session_id)
        .await
        .map_err(|e| e.to_string())
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

    // A nameless tab used to register the useless literal "session" (and never
    // persisted it), so every unnamed session looked identical and appeared to
    // "lose its name" once stashed. Derive a meaningful default through the
    // shared core helper (same as the TUI's new-session flow) and persist it so
    // it survives stash/restart.
    let derived_name = if name.is_empty() {
        clash::application::reducer::default_session_name(cwd, &session_id)
    } else {
        name.to_string()
    };

    clash::infrastructure::hooks::registry::register(&session_id, &derived_name, cwd, None);
    clash::infrastructure::hooks::save_session_name(
        state.backend.base_dir(),
        &session_id,
        &derived_name,
        Some(cwd),
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
            Some(cwd),
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            },
            cols,
            rows,
            HashMap::new(),
            true, // TUI: Claude sets its own termios
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

/// All scratch notes (free-form text files), via the shared DataRepository.
#[tauri::command]
fn list_scratch_notes(
    state: State<'_, GuiState>,
) -> Result<Vec<clash::domain::entities::ScratchNote>, String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .load_scratch_notes()
        .map_err(|e| e.to_string())
}

/// Create a new empty scratch note titled `title` inside the folder at
/// `parent` (relative path; `""` = root); returns the created note.
#[tauri::command]
fn create_scratch_note(
    state: State<'_, GuiState>,
    parent: String,
    title: String,
) -> Result<clash::domain::entities::ScratchNote, String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .create_scratch_note(&parent, &title)
        .map_err(|e| e.to_string())
}

/// Create a new folder named `name` inside the folder at `parent`
/// (relative path; `""` = root); returns the created folder entry.
#[tauri::command]
fn create_scratch_dir(
    state: State<'_, GuiState>,
    parent: String,
    name: String,
) -> Result<clash::domain::entities::ScratchNote, String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .create_scratch_dir(&parent, &name)
        .map_err(|e| e.to_string())
}

/// Rename the scratch entry at `id` (file or folder) to `new_name`.
#[tauri::command]
fn rename_scratch(
    state: State<'_, GuiState>,
    id: String,
    new_name: String,
) -> Result<clash::domain::entities::ScratchNote, String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .rename_scratch(&id, &new_name)
        .map_err(|e| e.to_string())
}

/// Move the scratch entry at `id` into the folder at `new_parent` (`""` = root).
#[tauri::command]
fn move_scratch(
    state: State<'_, GuiState>,
    id: String,
    new_parent: String,
) -> Result<clash::domain::entities::ScratchNote, String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .move_scratch(&id, &new_parent)
        .map_err(|e| e.to_string())
}

/// Detect available editors for a single file (the IDE list plus terminal
/// editors like vim/nano/emacs). Used by the scratch editor picker — the GUI
/// equivalent of the TUI's `Effect::DetectEditors`.
#[tauri::command]
fn detect_editors(state: State<'_, GuiState>) -> Vec<IdeDto> {
    clash::infrastructure::ide::detect_editors(&state.config_ides)
        .into_iter()
        .map(|i| IdeDto {
            label: i.label,
            description: i.description,
            value: i.value,
        })
        .collect()
}

/// Open a scratch note in a terminal editor inside an in-app terminal tab.
/// Spawns `<editor> <path>` as a shell-namespaced PTY (a GUI tab, never a
/// Claude session) and returns its id for the frontend to attach. `editor` is
/// the bare command with the `terminal:` picker prefix already stripped.
/// GUI editors take the external `open_in_ide` path instead.
#[tauri::command]
async fn open_scratch_terminal_editor(
    state: State<'_, GuiState>,
    editor: String,
    path: String,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let session_id = format!(
        "{}{}",
        session_refresh::SHELL_TERMINAL_ID_PREFIX,
        uuid::Uuid::now_v7()
    );
    // Run from the note's directory so relative writes/swap files land there.
    let cwd = std::path::Path::new(&path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned());
    let mut control = state.control.lock().await;
    ensure_connected(&mut control).await;
    control
        .create_session(
            &session_id,
            &editor,
            &[path],
            cwd.as_deref(),
            Some(editor.clone()),
            cols,
            rows,
            HashMap::new(),
            // Terminal editors set their own raw termios at startup, like Claude.
            true,
        )
        .await
        .map_err(|e| format!("Failed to open editor: {}", e))?;
    Ok(session_id)
}

/// Current scratch-notes directory (absolute path) for the Settings field.
#[tauri::command]
fn get_scratch_dir(state: State<'_, GuiState>) -> String {
    state.backend.scratch_dir().to_string_lossy().into_owned()
}

/// Set (or reset) the scratch-notes directory. An empty path resets to the
/// default. Persists to the shared `config.toml` and applies live to the
/// running backend so both the GUI and a future TUI launch agree. Returns the
/// effective absolute path.
#[tauri::command]
fn set_scratch_dir(state: State<'_, GuiState>, path: String) -> Result<String, String> {
    let trimmed = path.trim();
    let mut config = Config::load();

    let effective = if trimmed.is_empty() {
        config.scratch_dir = None;
        config.scratch_dir()
    } else {
        let expanded = expand_tilde(trimmed);
        std::fs::create_dir_all(&expanded)
            .map_err(|e| format!("Cannot use {}: {}", expanded.display(), e))?;
        config.scratch_dir = Some(expanded.clone());
        expanded
    };

    config
        .save()
        .map_err(|e| format!("Save config failed: {}", e))?;
    state.backend.set_scratch_dir(effective.clone());
    Ok(effective.to_string_lossy().into_owned())
}

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

/// Delete a scratch note by id (file name).
#[tauri::command]
fn delete_scratch_note(state: State<'_, GuiState>, id: String) -> Result<(), String> {
    use clash::domain::ports::DataRepository;
    state
        .backend
        .delete_scratch_note(&id)
        .map_err(|e| e.to_string())
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
            let encoded = encode_project_dir(path);
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
            true, // TUI: Claude sets its own termios
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
            true, // TUI: Claude sets its own termios
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

fn set_browser_bounds(wv: &tauri::Webview, x: f64, y: f64, w: f64, h: f64) {
    let _ = wv.set_position(tauri::LogicalPosition::new(x, y));
    let _ = wv.set_size(tauri::LogicalSize::new(w, h));
}

/// Offset between the frontend's coordinate space (the main webview's
/// DOM viewport, where pane slot rects are measured) and the space wry
/// positions child webviews in (the window's content view). Two parts:
/// the main webview's frame inset inside the content view, plus its
/// safe-area inset — the window uses a full-size content view, so in
/// windowed mode WKWebView pushes the page content down by the title-bar
/// height (~32px) while the view itself still spans the full window.
/// A child webview placed with raw DOM coordinates therefore lands a
/// title-bar height too high, covering the browser pane's chrome strip
/// (address bar + nav buttons). Fullscreen has no title bar and no
/// inset. Measured live so windowed⇄fullscreen transitions re-sync
/// correctly (the frontend re-reports bounds on every resize).
async fn browser_coord_offset(app: &tauri::AppHandle) -> (f64, f64) {
    #[cfg(target_os = "macos")]
    {
        let Some(main) = app.get_webview("main") else {
            tracing::warn!("browser_coord_offset: no main webview");
            return (0.0, 0.0);
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let sent = main.with_webview(move |pw| {
            use objc2::runtime::AnyObject;
            let offset = unsafe {
                let wk: *mut AnyObject = pw.inner().cast();
                let superview: *mut AnyObject = objc2::msg_send![wk, superview];
                if superview.is_null() {
                    tracing::warn!("browser_coord_offset: main webview has no superview");
                    (0.0, 0.0)
                } else {
                    let mf: objc2_foundation::NSRect = objc2::msg_send![wk, frame];
                    let flipped: bool = objc2::msg_send![superview, isFlipped];
                    let top = if flipped {
                        mf.origin.y
                    } else {
                        let sf: objc2_foundation::NSRect = objc2::msg_send![superview, frame];
                        sf.size.height - (mf.origin.y + mf.size.height)
                    };
                    let insets: objc2_foundation::NSEdgeInsets =
                        objc2::msg_send![wk, safeAreaInsets];
                    (mf.origin.x + insets.left, top + insets.top)
                }
            };
            let _ = tx.send(offset);
        });
        if sent.is_err() {
            return (0.0, 0.0);
        }
        rx.await.unwrap_or((0.0, 0.0))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        (0.0, 0.0)
    }
}

/// Open (or navigate) the browser tab's child webview, positioned over
/// the tab's pane slot rect reported by the frontend. Visibility of other
/// tabs is the frontend's responsibility (tabs can live in different panes).
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
    let (dx, dy) = browser_coord_offset(&app).await;
    let (x, y) = (x + dx, y + dy);
    if let Some(wv) = browser_tab(&app, &tab) {
        tracing::info!(%tab, %url, "browser_open: navigating existing webview");
        wv.navigate(parsed).map_err(|e| e.to_string())?;
        set_browser_bounds(&wv, x, y, w, h);
        let _ = wv.show();
        return Ok(());
    }
    tracing::info!(%tab, %url, x, y, w, h, "browser_open: creating child webview");
    let win = app.get_window("main").ok_or("no main window")?;
    // add_child must run on the main thread on macOS.
    let win2 = win.clone();
    let label = browser_tab_label(&tab);
    let app2 = app.clone();
    let app3 = app.clone();
    let tab2 = tab.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    win.run_on_main_thread(move || {
        // Navigation lifecycle → frontend (spinner, stop button, URL bar,
        // tab label) without polling lag.
        let builder =
            tauri::webview::WebviewBuilder::new(label, tauri::WebviewUrl::External(parsed))
                .on_page_load(move |_wv, payload| {
                    let event = match payload.event() {
                        tauri::webview::PageLoadEvent::Started => "started",
                        tauri::webview::PageLoadEvent::Finished => "finished",
                    };
                    tracing::info!(tab = %tab2, event, url = %payload.url(), "browser page load");
                    let _ = app2.emit(
                        "browser-nav",
                        serde_json::json!({
                            "tab": tab2,
                            "event": event,
                            "url": payload.url().to_string(),
                        }),
                    );
                })
                // A link that wants a new window/tab (target="_blank",
                // window.open) opens a new clash browser tab instead of being
                // swallowed by WKWebView or replacing the current tab.
                .on_new_window(move |url, _features| {
                    tracing::info!(%url, "browser: new-window request → new tab");
                    let _ = app3.emit("browser-open-tab", url.to_string());
                    tauri::webview::NewWindowResponse::Deny
                });
        let res = win2
            .add_child(
                builder,
                tauri::LogicalPosition::new(x, y),
                tauri::LogicalSize::new(w, h),
            )
            .map(|_| ())
            .map_err(|e| e.to_string());
        if let Err(e) = &res {
            tracing::warn!(error = %e, "browser_open: add_child failed");
        }
        let _ = tx.send(res);
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())??;
    if let Some(wv) = browser_tab(&app, &tab) {
        let _ = wv.show();
    }
    Ok(())
}

/// Per-tab bounds — each browser tab can live in a different pane.
#[tauri::command]
async fn browser_set_bounds(
    app: tauri::AppHandle,
    tab: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    let (dx, dy) = browser_coord_offset(&app).await;
    let wv = browser_tab(&app, &tab).ok_or("no such tab")?;
    set_browser_bounds(&wv, x + dx, y + dy, w, h);
    Ok(())
}

/// Show/hide one tab's webview (pane/workspace visibility, zoom, dialogs).
#[tauri::command]
fn browser_set_visible(app: tauri::AppHandle, tab: String, visible: bool) -> Result<(), String> {
    let wv = browser_tab(&app, &tab).ok_or("no such tab")?;
    if visible {
        wv.show().map_err(|e| e.to_string())
    } else {
        wv.hide().map_err(|e| e.to_string())
    }
}

/// Page zoom (1.0 = 100%).
#[tauri::command]
fn browser_set_zoom(app: tauri::AppHandle, tab: String, factor: f64) -> Result<(), String> {
    let wv = browser_tab(&app, &tab).ok_or("no such tab")?;
    wv.set_zoom(factor.clamp(0.25, 5.0))
        .map_err(|e| e.to_string())
}

/// Stop loading the current page.
#[tauri::command]
fn browser_stop(app: tauri::AppHandle, tab: String) -> Result<(), String> {
    browser_tab(&app, &tab)
        .ok_or("no such tab")?
        .eval("window.stop()")
        .map_err(|e| e.to_string())
}

/// Open the native DevTools window for the tab's page.
#[tauri::command]
fn browser_devtools(app: tauri::AppHandle, tab: String) -> Result<(), String> {
    let wv = browser_tab(&app, &tab).ok_or("no such tab")?;
    wv.open_devtools();
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

/// Current page URL. Do NOT go through `Webview::url()` here: wry's macOS
/// impl unwraps WKWebView's `URL` property, which is nil until the first
/// navigation commits, and that panic happens on the tao event loop thread
/// and aborts the whole app (with the tab persisted in gui-state.json, the
/// abort then replays on every launch). Read the property directly with a
/// nil check instead; empty string = no committed navigation yet.
#[tauri::command]
async fn browser_get_url(app: tauri::AppHandle, tab: String) -> Result<String, String> {
    let wv = browser_tab(&app, &tab).ok_or("no such tab")?;
    #[cfg(target_os = "macos")]
    {
        use objc2::rc::Retained;
        use objc2::runtime::AnyObject;
        use objc2_foundation::NSURL;

        let (tx, rx) = tokio::sync::oneshot::channel();
        wv.with_webview(move |pw| {
            let wk: *mut AnyObject = pw.inner().cast();
            let url = unsafe {
                let url: Option<Retained<NSURL>> = objc2::msg_send![wk, URL];
                url.and_then(|u| u.absoluteString()).map(|s| s.to_string())
            };
            let _ = tx.send(url.unwrap_or_default());
        })
        .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        wv.url().map(|u| u.to_string()).map_err(|e| e.to_string())
    }
}

/// Close one tab's webview.
#[tauri::command]
fn browser_close_tab(app: tauri::AppHandle, tab: String) -> Result<(), String> {
    if let Some(wv) = browser_tab(&app, &tab) {
        let _ = wv.close();
    }
    Ok(())
}

/// Frontend log bridge — the main webview's console is invisible in
/// release builds; frontend errors and browser-tab lifecycle events are
/// forwarded here so they land in clash.log.
#[tauri::command]
fn gui_log(msg: String) {
    tracing::info!(%msg, "frontend");
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

/// Write text to the system clipboard. Used by the embedded terminals'
/// ⌘C/Ctrl+Shift+C copy: xterm's canvas selection isn't a DOM selection, so
/// the webview's native copy can't reach it. Goes through the clipboard
/// plugin's Rust API, which is reliable where `navigator.clipboard` is not.
#[tauri::command]
fn clipboard_write_text(app: tauri::AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// Read text from the system clipboard. Used by the embedded terminals'
/// ⌘V/Ctrl+Shift+V paste; the frontend feeds the result through
/// `term.paste()` so bracketed-paste mode is honored.
#[tauri::command]
fn clipboard_read_text(app: tauri::AppHandle) -> Result<String, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    // An empty/non-text clipboard surfaces as an error in the plugin; treat
    // that as "nothing to paste" rather than propagating a failure.
    Ok(app.clipboard().read_text().unwrap_or_default())
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

/// Consume the quit-stash marker written on the previous exit and force the
/// listed sessions back to "idle". A Claude `Stop` hook firing while the app
/// tears down can overwrite the quit "idle" status with "waiting"; without
/// this repair the GUI shows a dead session as active on the next launch and
/// resuming it fails. Mirrors the TUI's restore_sessions repair
/// (src/infrastructure/app.rs). The marker is taken (read + deleted) so it
/// only applies once.
fn repair_quit_stashed(base_dir: &std::path::Path) {
    let stashed = clash::infrastructure::hooks::take_quit_stashed();
    if stashed.is_empty() {
        return;
    }
    let statuses = clash::infrastructure::hooks::read_all_statuses(base_dir);
    for id in &stashed {
        let needs_repair = statuses
            .get(id.as_str())
            .map(|(s, _)| *s != clash::domain::entities::SessionStatus::Stashed)
            .unwrap_or(true);
        if needs_repair {
            tracing::info!("Repairing quit-stash status for session {}", id);
            clash::infrastructure::hooks::write_session_status(base_dir, id, "idle");
        }
    }
}

fn main() {
    // Same clash.log as the TUI — Finder/Dock launches have no usable
    // stderr, so file logging is the only trail for GUI sessions.
    match clash::infrastructure::logging::open_log_file() {
        Some(log_file) => tracing_subscriber::fmt()
            .with_writer(log_file)
            .with_ansi(false)
            .with_target(false)
            .with_max_level(tracing::Level::INFO)
            .init(),
        None => tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .init(),
    }

    // Launched from Finder/Dock the app gets launchd's minimal PATH —
    // without this, `claude` can't be found and session spawns fail.
    clash::infrastructure::env_path::adopt_login_shell_path();

    let config = Config::load();
    let data_dir: PathBuf = config.claude_dir();
    let claude_bin = config.claude_bin.clone();

    let (wild_processes_tx, wild_processes_rx) = tokio::sync::watch::channel(Vec::new());

    let state = GuiState {
        backend: FsBackend::new(data_dir).with_scratch_dir(config.scratch_dir.clone()),
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
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state)
        .setup(move |app| {
            // Repair any sessions whose quit "idle" status was clobbered by a
            // dying Claude hook on the previous exit, before the first refresh.
            repair_quit_stashed(app.state::<GuiState>().backend.base_dir());
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

            // Wild-process background scan — same scan and same start-time
            // filter the TUI runs. Only claudes that started AFTER this clash
            // launched are surfaced; pre-existing claudes (IDE agents, other
            // terminals, background runs) are intentionally hidden so the
            // list doesn't churn with processes the user never started here.
            let clash_started_at = std::time::SystemTime::now();
            tauri::async_runtime::spawn(async move {
                use clash::infrastructure::process_scan::{
                    default_fd_probe, gather_wild_processes,
                };
                let probe = default_fd_probe();
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let wild: Vec<_> = gather_wild_processes(&probe)
                        .into_iter()
                        .filter(|w| {
                            // Drop conservatively when start time is unknown
                            // (process exited mid-scan, ps unavailable) so we
                            // never accidentally surface a pre-clash row.
                            w.started_at.map(|t| t >= clash_started_at).unwrap_or(false)
                        })
                        .collect();
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
            resolve_session_ids,
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
            list_shells,
            create_terminal,
            close_terminal,
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
            list_scratch_notes,
            create_scratch_note,
            create_scratch_dir,
            rename_scratch,
            move_scratch,
            delete_scratch_note,
            detect_editors,
            open_scratch_terminal_editor,
            get_scratch_dir,
            set_scratch_dir,
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
            browser_set_bounds,
            browser_set_visible,
            browser_set_zoom,
            browser_stop,
            browser_devtools,
            browser_navigate,
            browser_history,
            browser_reload,
            browser_get_url,
            browser_close_tab,
            gui_log,
            open_external,
            clipboard_write_text,
            clipboard_read_text,
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
