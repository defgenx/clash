//! Daemon server — manages PTY sessions, handles client connections.
//!
//! Listens on `~/.clash/daemon.sock`. Each client gets a tokio task.
//! Sessions persist across client disconnects.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, Mutex, Notify};

use super::protocol::{self, Event, Request, SessionInfo};
use super::session::PtySession;

/// The daemon server.
pub struct DaemonServer {
    socket_path: PathBuf,
    sessions: Arc<Mutex<HashMap<String, PtySession>>>,
    shutdown: Arc<Notify>,
}

impl DaemonServer {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Get a handle to signal shutdown from outside.
    pub fn shutdown_handle(&self) -> Arc<Notify> {
        self.shutdown.clone()
    }

    /// Kill a stale daemon process from a previous run, if any.
    fn kill_stale_daemon(&self, pid_path: &std::path::Path) {
        let contents = match std::fs::read_to_string(pid_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let old_pid: i32 = match contents.trim().parse() {
            Ok(p) => p,
            Err(_) => return,
        };

        // Don't kill ourselves
        if old_pid == std::process::id() as i32 {
            return;
        }

        // Check if the process is alive (signal 0 = existence check)
        let alive = unsafe { libc::kill(old_pid, 0) } == 0;
        if !alive {
            tracing::info!(
                "Stale PID file (pid={}, not running) — cleaning up",
                old_pid
            );
            let _ = std::fs::remove_file(pid_path);
            return;
        }

        tracing::info!("Killing stale daemon process pid={}", old_pid);
        unsafe { libc::kill(old_pid, libc::SIGTERM) };

        // Brief wait for graceful exit, then force kill
        std::thread::sleep(std::time::Duration::from_millis(200));
        if unsafe { libc::kill(old_pid, 0) } == 0 {
            unsafe { libc::kill(old_pid, libc::SIGKILL) };
        }
        let _ = std::fs::remove_file(pid_path);
    }

    /// Run the daemon server. Blocks until shutdown is requested.
    pub async fn run(&self) -> std::io::Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Kill any stale daemon from a previous run (e.g. old separate-process daemon)
        let pid_path = self.socket_path.with_extension("pid");
        self.kill_stale_daemon(&pid_path);

        // Remove stale socket
        let _ = std::fs::remove_file(&self.socket_path);

        let listener = UnixListener::bind(&self.socket_path)?;
        tracing::info!("Daemon listening on {:?}", self.socket_path);

        // Write PID file
        std::fs::write(&pid_path, std::process::id().to_string())?;

        // Reaper task: clean up dead sessions periodically
        let sessions = self.sessions.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let mut map = sessions.lock().await;
                let dead: Vec<String> = map
                    .iter()
                    .filter(|(_, s)| !s.is_alive())
                    .map(|(k, _)| k.clone())
                    .collect();
                for id in dead {
                    tracing::info!("Reaping dead session {}", id);
                    map.remove(&id);
                }
            }
        });

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            let sessions = self.sessions.clone();
                            let shutdown = self.shutdown.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_client(stream, sessions, shutdown).await {
                                    tracing::warn!("Client error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {}", e);
                        }
                    }
                }
                _ = self.shutdown.notified() => {
                    tracing::info!("Daemon shutting down");
                    break;
                }
            }
        }

        // Cleanup
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(self.socket_path.with_extension("pid"));
        Ok(())
    }
}

/// Handle a single client connection.
async fn handle_client(
    stream: UnixStream,
    sessions: Arc<Mutex<HashMap<String, PtySession>>>,
    shutdown: Arc<Notify>,
) -> std::io::Result<()> {
    let (reader, writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let writer = Arc::new(Mutex::new(writer));

    // Track output subscriptions for this client
    let mut output_tasks: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();

    loop {
        let line = match lines.next_line().await? {
            Some(l) => l,
            None => break, // Client disconnected
        };

        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                send_event(
                    &writer,
                    &Event::Error {
                        message: format!("Parse error: {}", e),
                    },
                )
                .await;
                continue;
            }
        };

        match request {
            Request::Ping => {
                send_event(&writer, &Event::Pong).await;
            }

            Request::Shutdown => {
                send_event(
                    &writer,
                    &Event::Ok {
                        message: Some("Shutting down".into()),
                    },
                )
                .await;
                shutdown.notify_one();
                break;
            }

            Request::ListSessions => {
                let map = sessions.lock().await;
                let infos: Vec<SessionInfo> = map
                    .values()
                    .map(|s| SessionInfo {
                        session_id: s.session_id.clone(),
                        pid: s.pid,
                        is_alive: s.is_alive(),
                        attached_clients: s.subscriber_count(),
                        created_at: s.created_at,
                        status: s.status().to_string(),
                        cwd: s.cwd.clone(),
                        name: s.name.clone(),
                    })
                    .collect();
                send_event(&writer, &Event::Sessions { sessions: infos }).await;
            }

            Request::CreateSession {
                session_id,
                bin,
                args,
                cwd,
                name,
                cols,
                rows,
                env_vars,
            } => {
                let mut map = sessions.lock().await;
                if map.contains_key(&session_id) {
                    send_event(
                        &writer,
                        &Event::Error {
                            message: format!("Session {} already exists", session_id),
                        },
                    )
                    .await;
                    continue;
                }

                let cwd_opt = if cwd.is_empty() {
                    None
                } else {
                    Some(cwd.as_str())
                };
                let pty_cols = if cols > 0 { cols } else { 120 };
                let pty_rows = if rows > 0 { rows } else { 40 };
                match PtySession::spawn(
                    session_id.clone(),
                    name,
                    &bin,
                    &args,
                    cwd_opt,
                    pty_cols,
                    pty_rows,
                    &env_vars,
                ) {
                    Ok(session) => {
                        map.insert(session_id.clone(), session);
                        send_event(
                            &writer,
                            &Event::Ok {
                                message: Some(format!("Session {} created", session_id)),
                            },
                        )
                        .await;
                    }
                    Err(e) => {
                        send_event(
                            &writer,
                            &Event::Error {
                                message: format!("Spawn failed: {}", e),
                            },
                        )
                        .await;
                    }
                }
            }

            Request::Attach {
                session_id,
                skip_replay,
            } => {
                let map = sessions.lock().await;
                match map.get(&session_id) {
                    Some(session) => {
                        // Idempotent attach: if the same connection is
                        // already attached (e.g. a prior detach RPC failed
                        // silently), abort the old forwarding task and
                        // start a new one instead of rejecting the client.
                        if let Some(old) = output_tasks.remove(&session_id) {
                            tracing::debug!(
                                "Re-attach for {}: aborting previous output task",
                                session_id
                            );
                            old.abort();
                        }

                        // Only fetch the history if the client wants it;
                        // building the chunked replay below is wasted work
                        // when skip_replay is set.
                        let replay = if skip_replay {
                            Vec::new()
                        } else {
                            session.output_history()
                        };
                        let mut rx = session.subscribe();
                        let w = writer.clone();
                        let sid = session_id.clone();

                        // Spawn task to forward live output to this client
                        let handle = tokio::spawn(async move {
                            loop {
                                match rx.recv().await {
                                    Ok(data) => {
                                        let encoded = protocol::encode_data(&data);
                                        let event = Event::Output {
                                            session_id: sid.clone(),
                                            data: encoded,
                                        };
                                        send_event(&w, &event).await;
                                    }
                                    Err(broadcast::error::RecvError::Lagged(n)) => {
                                        tracing::warn!(
                                            "Client lagged {} frames for session {}",
                                            n,
                                            sid
                                        );
                                    }
                                    Err(broadcast::error::RecvError::Closed) => {
                                        break;
                                    }
                                }
                            }
                        });
                        output_tasks.insert(session_id.clone(), handle);

                        // Send Ok first (client waits for this in recv_response)
                        send_event(
                            &writer,
                            &Event::Ok {
                                message: Some(format!("Attached to {}", session_id)),
                            },
                        )
                        .await;

                        // Send replay buffer (full session history) in chunks
                        // to avoid one giant NDJSON line on the socket.
                        const REPLAY_CHUNK: usize = 64 * 1024;
                        for chunk in replay.chunks(REPLAY_CHUNK) {
                            let encoded = protocol::encode_data(chunk);
                            send_event(
                                &writer,
                                &Event::Output {
                                    session_id: session_id.clone(),
                                    data: encoded,
                                },
                            )
                            .await;
                        }
                    }
                    None => {
                        send_event(
                            &writer,
                            &Event::Error {
                                message: format!("Session {} not found", session_id),
                            },
                        )
                        .await;
                    }
                }
            }

            Request::Detach { session_id } => {
                if let Some(handle) = output_tasks.remove(&session_id) {
                    handle.abort();
                    send_event(
                        &writer,
                        &Event::Ok {
                            message: Some(format!("Detached from {}", session_id)),
                        },
                    )
                    .await;
                } else {
                    send_event(
                        &writer,
                        &Event::Error {
                            message: "Not attached".into(),
                        },
                    )
                    .await;
                }
            }

            Request::Input { session_id, data } => {
                let decoded = match protocol::decode_data(&data) {
                    Ok(d) => d,
                    Err(e) => {
                        send_event(
                            &writer,
                            &Event::Error {
                                message: format!("Base64 decode error: {}", e),
                            },
                        )
                        .await;
                        continue;
                    }
                };

                let map = sessions.lock().await;
                match map.get(&session_id) {
                    Some(session) => {
                        if let Err(e) = session.write_input(&decoded) {
                            send_event(
                                &writer,
                                &Event::Error {
                                    message: format!("Write error: {}", e),
                                },
                            )
                            .await;
                        }
                        // No ack for input — fire and forget for performance
                    }
                    None => {
                        send_event(
                            &writer,
                            &Event::Error {
                                message: format!("Session {} not found", session_id),
                            },
                        )
                        .await;
                    }
                }
            }

            Request::Resize {
                session_id,
                cols,
                rows,
            } => {
                let map = sessions.lock().await;
                if let Some(session) = map.get(&session_id) {
                    session.resize(cols, rows);
                }
            }

            Request::Kill { session_id } => {
                let map = sessions.lock().await;
                match map.get(&session_id) {
                    Some(session) => {
                        session.kill();
                        send_event(
                            &writer,
                            &Event::Ok {
                                message: Some(format!("Killed session {}", session_id)),
                            },
                        )
                        .await;
                    }
                    None => {
                        send_event(
                            &writer,
                            &Event::Error {
                                message: format!("Session {} not found", session_id),
                            },
                        )
                        .await;
                    }
                }
            }
        }
    }

    // Cleanup: abort all output tasks for this client
    for (_, handle) in output_tasks {
        handle.abort();
    }

    Ok(())
}

/// Send an event to a client (NDJSON line).
async fn send_event(writer: &Arc<Mutex<tokio::net::unix::OwnedWriteHalf>>, event: &Event) {
    let mut line = serde_json::to_string(event).unwrap_or_default();
    line.push('\n');
    let mut w = writer.lock().await;
    let _ = w.write_all(line.as_bytes()).await;
    let _ = w.flush().await;
}
