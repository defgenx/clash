//! Daemon client — used by the TUI to communicate with the daemon.
//!
//! Connects to `~/.clash/daemon.sock`. Provides async methods for
//! session management. Can auto-start the daemon if not running.

use std::path::PathBuf;
use std::process::Command;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::protocol::{self, Event, Request, SessionInfo};

/// Client for communicating with the clash daemon.
pub struct DaemonClient {
    socket_path: PathBuf,
    stream: Option<ClientStream>,
}

struct ClientStream {
    writer: tokio::net::unix::OwnedWriteHalf,
    event_rx: mpsc::UnboundedReceiver<Event>,
    /// Buffered events that arrived during recv_response but weren't consumed.
    pending_events: Vec<Event>,
    _reader_task: tokio::task::JoinHandle<()>,
}

impl DaemonClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            stream: None,
        }
    }

    /// Get the default socket path (~/.clash/daemon.sock).
    pub fn default_socket_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clash")
            .join("daemon.sock")
    }

    /// Connect to the daemon, auto-starting it if needed.
    pub async fn connect(&mut self) -> std::io::Result<()> {
        // Try connecting first
        match UnixStream::connect(&self.socket_path).await {
            Ok(stream) => {
                self.setup_stream(stream);
                Ok(())
            }
            Err(_) => {
                // Daemon not running — start it
                self.start_daemon()?;
                // Wait for socket to appear
                for _ in 0..50 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if let Ok(stream) = UnixStream::connect(&self.socket_path).await {
                        self.setup_stream(stream);
                        return Ok(());
                    }
                }
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "Failed to connect to daemon after starting it",
                ))
            }
        }
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    /// Start the daemon process in the background.
    fn start_daemon(&self) -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        tracing::info!("Starting daemon: {:?} daemon", exe);

        Command::new(exe)
            .arg("daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        Ok(())
    }

    fn setup_stream(&mut self, stream: UnixStream) {
        let (reader, writer) = stream.into_split();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let reader_task = tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match serde_json::from_str::<Event>(&line) {
                    Ok(event) => {
                        if event_tx.send(event).is_err() {
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
            event_rx,
            pending_events: Vec::new(),
            _reader_task: reader_task,
        });
    }

    /// Send a request to the daemon.
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

    /// Receive the next event from the daemon (non-blocking via channel).
    pub async fn recv_event(&mut self) -> Option<Event> {
        let stream = self.stream.as_mut()?;
        stream.event_rx.recv().await
    }

    /// Try to receive an event without blocking.
    /// Checks pending buffer first (events saved during recv_response).
    pub fn try_recv_event(&mut self) -> Option<Event> {
        let stream = self.stream.as_mut()?;
        if !stream.pending_events.is_empty() {
            return Some(stream.pending_events.remove(0));
        }
        stream.event_rx.try_recv().ok()
    }

    /// Wait for a specific response (blocking until we get a non-Output event).
    /// Buffers Output/Exited events so they aren't lost.
    async fn recv_response(&mut self) -> std::io::Result<Event> {
        let timeout = std::time::Duration::from_secs(10);
        match tokio::time::timeout(timeout, async {
            loop {
                match self.recv_event().await {
                    Some(event @ Event::Ok { .. }) => return event,
                    Some(event @ Event::Error { .. }) => return event,
                    Some(event @ Event::Sessions { .. }) => return event,
                    Some(Event::Pong) => return Event::Pong,
                    Some(other) => {
                        // Buffer Output/Exited events so poll_daemon_events picks them up
                        if let Some(ref mut stream) = self.stream {
                            stream.pending_events.push(other);
                        }
                        continue;
                    }
                    None => return Event::Error { message: "Connection closed".into() },
                }
            }
        }).await {
            Ok(event) => Ok(event),
            Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "Daemon response timeout")),
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

    /// Create a new PTY session.
    pub async fn create_session(
        &mut self,
        session_id: &str,
        bin: &str,
        args: &[String],
        cwd: &str,
    ) -> std::io::Result<()> {
        self.send(&Request::CreateSession {
            session_id: session_id.to_string(),
            bin: bin.to_string(),
            args: args.to_vec(),
            cwd: cwd.to_string(),
        }).await?;

        match self.recv_response().await? {
            Event::Ok { .. } => Ok(()),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(()),
        }
    }

    /// Attach to a session (start receiving output).
    pub async fn attach(&mut self, session_id: &str) -> std::io::Result<()> {
        self.send(&Request::Attach {
            session_id: session_id.to_string(),
        }).await?;

        match self.recv_response().await? {
            Event::Ok { .. } => Ok(()),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(()),
        }
    }

    /// Detach from a session.
    pub async fn detach(&mut self, session_id: &str) -> std::io::Result<()> {
        self.send(&Request::Detach {
            session_id: session_id.to_string(),
        }).await?;

        match self.recv_response().await? {
            Event::Ok { .. } => Ok(()),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(()),
        }
    }

    /// Send input to a session.
    pub async fn send_input(&mut self, session_id: &str, data: &[u8]) -> std::io::Result<()> {
        self.send(&Request::Input {
            session_id: session_id.to_string(),
            data: protocol::encode_data(data),
        }).await
    }

    /// Resize a session's PTY.
    pub async fn resize(&mut self, session_id: &str, cols: u16, rows: u16) -> std::io::Result<()> {
        self.send(&Request::Resize {
            session_id: session_id.to_string(),
            cols,
            rows,
        }).await
    }

    /// Kill a session.
    pub async fn kill_session(&mut self, session_id: &str) -> std::io::Result<()> {
        self.send(&Request::Kill {
            session_id: session_id.to_string(),
        }).await?;

        match self.recv_response().await? {
            Event::Ok { .. } => Ok(()),
            Event::Error { message } => Err(std::io::Error::other(message)),
            _ => Ok(()),
        }
    }

}
