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

    /// Remove sockets left behind by clash instances that no longer run.
    ///
    /// Multiple instances are allowed (per-pid `daemon-<pid>.sock` files);
    /// this only sweeps files whose pid — parsed from the filename — is
    /// dead. It never signals other processes.
    fn cleanup_dead_instance_sockets(&self) {
        let Some(dir) = self.socket_path.parent() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(pid) = name
                .strip_prefix("daemon-")
                .and_then(|s| s.strip_suffix(".sock"))
                .and_then(|s| s.parse::<i32>().ok())
            else {
                continue;
            };
            if pid == std::process::id() as i32 {
                continue;
            }
            if unsafe { libc::kill(pid, 0) } != 0 {
                tracing::info!("Removing stale socket of dead instance pid={}", pid);
                let _ = std::fs::remove_file(entry.path());
            }
        }
        // Legacy artifacts from the old shared-socket/pid-file scheme
        let _ = std::fs::remove_file(dir.join("daemon.pid"));
    }

    /// Run the daemon server. Blocks until shutdown is requested.
    pub async fn run(&self) -> std::io::Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        self.cleanup_dead_instance_sockets();

        // Remove stale socket (pid reuse after a crash)
        let _ = std::fs::remove_file(&self.socket_path);

        let listener = UnixListener::bind(&self.socket_path)?;
        tracing::info!("Daemon listening on {:?}", self.socket_path);

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
                            tracing::error!("Daemon accept error on {:?}: {}", self.socket_path, e);
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
                raw_startup,
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
                    raw_startup,
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
                                message: format!("Spawn failed for '{}': {}", bin, e),
                            },
                        )
                        .await;
                    }
                }
            }

            Request::Attach { session_id } => {
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

                        // Subscribe to live output BEFORE sending the replay so
                        // any output produced while the replay is in flight is
                        // buffered on `rx` (broadcast, in order) rather than
                        // racing ahead of the history. The forwarder is then
                        // started only after the replay is fully sent, so the
                        // client always sees history first, then live output —
                        // previously the forwarder was spawned first and could
                        // interleave live frames ahead of (or amid) the replay,
                        // scrambling the restored screen.
                        let replay = session.output_history();
                        let mut rx = session.subscribe();
                        let w = writer.clone();
                        let sid = session_id.clone();

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

                        // Now forward live output (anything broadcast since the
                        // subscribe above is delivered in order, after replay).
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
