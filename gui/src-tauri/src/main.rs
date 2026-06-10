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
use clash::infrastructure::lock::SingleInstanceLock;
use clash::infrastructure::session_refresh;
use tauri::{Emitter, Manager, State};

/// Shared backend state for all Tauri commands.
struct GuiState {
    backend: FsBackend,
    claude_bin: String,
    /// Previous session list — input to the merge step of the refresh pipeline.
    previous: Mutex<Vec<Session>>,
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

/// Full session list via the same pipeline the TUI uses
/// (disk + registry + hooks + daemon, merged and sorted by section).
#[tauri::command]
async fn list_sessions(state: State<'_, GuiState>) -> Result<Vec<Session>, String> {
    let registry = clash::infrastructure::hooks::registry::load();
    let previous = state.previous.lock().unwrap().clone();
    let mut input = session_refresh::gather_sync_input(&state.backend, &previous, registry);
    {
        let mut control = state.control.lock().await;
        ensure_connected(&mut control).await;
        input.daemon_infos = session_refresh::gather_daemon_input(&mut control).await;
    }
    let sessions = session_refresh::build_session_list(&input);
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

    let mut client = DaemonClient::new(DaemonClient::default_socket_path());
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

#[tauri::command]
async fn kill_session(state: State<'_, GuiState>, session_id: String) -> Result<(), String> {
    {
        let mut attached = state.attached.lock().await;
        attached.remove(&session_id);
    }
    let mut control = state.control.lock().await;
    ensure_connected(&mut control).await;
    control
        .kill_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// (Re)connect the control client if the connection was lost.
async fn ensure_connected(client: &mut DaemonClient) {
    if !client.is_connected() {
        let _ = client.connect().await;
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Same one-owner rule as the TUI: only one clash app (TUI or GUI) may
    // own sessions at a time. Self-contained — no external daemon.
    let instance_lock = match SingleInstanceLock::acquire() {
        Ok(lock) => lock,
        Err(msg) => {
            eprintln!("{}", msg);
            std::process::exit(1);
        }
    };

    let config = Config::load();
    let data_dir: PathBuf = config.claude_dir();
    let claude_bin = config.claude_bin.clone();

    let state = GuiState {
        backend: FsBackend::new(data_dir),
        claude_bin,
        previous: Mutex::new(Vec::new()),
        control: tokio::sync::Mutex::new(DaemonClient::new(DaemonClient::default_socket_path())),
        attached: tokio::sync::Mutex::new(HashMap::new()),
    };

    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            // In-process PTY session manager — the GUI's backbone, identical
            // to the TUI's in-process daemon. Dies with the app.
            let server = DaemonServer::new(DaemonClient::default_socket_path());
            let shutdown = server.shutdown_handle();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server.run().await {
                    tracing::error!("Daemon server error: {}", e);
                }
            });
            // Keep the instance lock and shutdown handle alive for the app's lifetime.
            app.manage(shutdown);
            app.manage(instance_lock);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
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
            kill_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running clash GUI");
}
