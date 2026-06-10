//! Daemon client — used by the TUI to communicate with the daemon.
//!
//! Connects to `~/.clash/daemon.sock`. Provides async methods for
//! session management. Splits incoming events into two channels:
//! - **responses** (Ok, Error, Sessions, Pong) — consumed by request methods
//! - **stream** (Output, Exited) — consumed by the event loop for real-time updates

use std::collections::HashMap;
use std::path::PathBuf;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::protocol::{Event, Request, SessionInfo};

/// Client for communicating with the clash daemon.
pub struct DaemonClient {
    socket_path: PathBuf,
    stream: Option<ClientStream>,
    /// Stream events (Output, Exited) for the event loop to consume.
    stream_event_rx: Option<mpsc::UnboundedReceiver<Event>>,
}

struct ClientStream {
    writer: tokio::net::unix::OwnedWriteHalf,
    /// Response events (Ok, Error, Sessions, Pong) for request/response methods.
    response_rx: mpsc::UnboundedReceiver<Event>,
    _reader_task: tokio::task::JoinHandle<()>,
}

impl DaemonClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            stream: None,
            stream_event_rx: None,
        }
    }

    /// Get the legacy shared socket path (`daemon.sock`). Only used by the
    /// standalone `clash daemon` subcommand and as a discovery fallback.
    pub fn default_socket_path() -> PathBuf {
        Self::socket_dir().join("daemon.sock")
    }

    /// Socket path for this process's in-process daemon (`daemon-<pid>.sock`).
    ///
    /// Per-instance sockets let multiple clash apps (TUI and/or GUI) run
    /// simultaneously without fighting over a shared socket.
    pub fn instance_socket_path() -> PathBuf {
        Self::socket_dir().join(format!("daemon-{}.sock", std::process::id()))
    }

    fn socket_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clash")
    }

    /// Discover sockets of running clash instances, newest first.
    ///
    /// Scans for `daemon-<pid>.sock` files, skipping those whose pid is no
    /// longer alive. The legacy shared `daemon.sock` is appended last as a
    /// fallback. Used by `clash attach` to find the instance that owns a
    /// session.
    pub fn discover_socket_paths() -> Vec<PathBuf> {
        let dir = Self::socket_dir();
        let mut found: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
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
                // Skip sockets left behind by dead instances
                if unsafe { libc::kill(pid, 0) } != 0 {
                    continue;
                }
                let mtime = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                found.push((entry.path(), mtime));
            }
        }
        found.sort_by_key(|(_, mtime)| std::cmp::Reverse(*mtime));
        let mut paths: Vec<PathBuf> = found.into_iter().map(|(p, _)| p).collect();
        let legacy = Self::default_socket_path();
        if legacy.exists() {
            paths.push(legacy);
        }
        paths
    }

    /// Connect to the daemon (running in-process as a background task).
    pub async fn connect(&mut self) -> std::io::Result<()> {
        // Try a few times — the in-process daemon may still be starting up
        for _ in 0..20 {
            if let Ok(stream) = UnixStream::connect(&self.socket_path).await {
                self.setup_stream(stream);
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "Failed to connect to daemon",
        ))
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    /// Take the stream event receiver (Output, Exited) for the event loop.
    ///
    /// Call after `connect()`. The event loop owns this receiver and
    /// processes daemon output concurrently with terminal input.
    pub fn take_stream_rx(&mut self) -> Option<mpsc::UnboundedReceiver<Event>> {
        self.stream_event_rx.take()
    }

    fn setup_stream(&mut self, stream: UnixStream) {
        let (reader, writer) = stream.into_split();
        let (response_tx, response_rx) = mpsc::unbounded_channel();
        let (stream_tx, stream_rx) = mpsc::unbounded_channel();

        let reader_task = tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<Event>(&line) {
                    Ok(event) => {
                        let tx = match &event {
                            Event::Ok { .. }
                            | Event::Error { .. }
                            | Event::Sessions { .. }
                            | Event::Pong => &response_tx,
                            _ => &stream_tx,
                        };
                        if tx.send(event).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse daemon event: {}", e);
                    }
                }
            }
        });

        self.stream = Some(ClientStream {
            writer,
            response_rx,
            _reader_task: reader_task,
        });
        self.stream_event_rx = Some(stream_rx);
    }

    async fn send(&mut self, request: &Request) -> std::io::Result<()> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "Not connected to daemon")
        })?;
        let mut line = serde_json::to_string(request).unwrap_or_default();
        line.push('\n');
        stream.writer.write_all(line.as_bytes()).await?;
        stream.writer.flush().await?;
        Ok(())
    }

    async fn recv_response(&mut self) -> std::io::Result<Event> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "Not connected to daemon")
        })?;
        let timeout = std::time::Duration::from_secs(10);
        match tokio::time::timeout(timeout, stream.response_rx.recv()).await {
            Ok(Some(event)) => Ok(event),
            Ok(None) => Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "Connection closed",
            )),
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Daemon response timeout",
            )),
        }
    }

    // ── High-level API ──────────────────────────────────────────

    /// List all daemon-managed sessions.
    pub async fn list_sessions(&mut self) -> std::io::Result<Vec<SessionInfo>> {
        self.send(&Request::ListSessions).await?;
        match self.recv_response().await? {
            Event::Sessions { sessions } => Ok(sessions),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(Vec::new()),
        }
    }

    /// Kill a session.
    pub async fn kill_session(&mut self, session_id: &str) -> std::io::Result<()> {
        self.send(&Request::Kill {
            session_id: session_id.to_string(),
        })
        .await?;
        match self.recv_response().await? {
            Event::Ok { .. } => Ok(()),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(()),
        }
    }

    /// Create a new PTY session on the daemon.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_session(
        &mut self,
        session_id: &str,
        bin: &str,
        args: &[String],
        cwd: Option<&str>,
        name: Option<String>,
        cols: u16,
        rows: u16,
        env_vars: HashMap<String, String>,
    ) -> std::io::Result<()> {
        self.send(&Request::CreateSession {
            session_id: session_id.to_string(),
            bin: bin.to_string(),
            args: args.to_vec(),
            cwd: cwd.unwrap_or("").to_string(),
            name,
            cols,
            rows,
            env_vars,
        })
        .await?;
        match self.recv_response().await? {
            Event::Ok { .. } => Ok(()),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(()),
        }
    }

    /// Attach to a session (start receiving output events).
    ///
    /// The server replays the session's output history in chunks after
    /// the Ok response so the caller can paint Claude's last screen
    /// state — including background colors and SGR — before live output
    /// begins.
    pub async fn attach(&mut self, session_id: &str) -> std::io::Result<()> {
        self.send(&Request::Attach {
            session_id: session_id.to_string(),
        })
        .await?;
        match self.recv_response().await? {
            Event::Ok { .. } => Ok(()),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(()),
        }
    }

    /// Detach from a session (stop receiving output events).
    pub async fn detach(&mut self, session_id: &str) -> std::io::Result<()> {
        self.send(&Request::Detach {
            session_id: session_id.to_string(),
        })
        .await?;
        match self.recv_response().await? {
            Event::Ok { .. } => Ok(()),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(()),
        }
    }

    /// Send raw input bytes to a session's PTY.
    pub async fn send_input(&mut self, session_id: &str, data: &[u8]) -> std::io::Result<()> {
        self.send(&Request::Input {
            session_id: session_id.to_string(),
            data: super::protocol::encode_data(data),
        })
        .await
    }

    /// Resize a session's PTY.
    pub async fn resize(&mut self, session_id: &str, cols: u16, rows: u16) -> std::io::Result<()> {
        self.send(&Request::Resize {
            session_id: session_id.to_string(),
            cols,
            rows,
        })
        .await
    }
}
