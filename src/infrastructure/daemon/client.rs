//! Daemon client — used by the TUI to communicate with the daemon.
//!
//! Connects to `~/.clash/daemon.sock`. Provides async methods for
//! session management. Splits incoming events into two channels:
//! - **responses** (Ok, Error, Sessions, Pong) — consumed by request methods
//! - **stream** (Output, Exited) — consumed by the event loop for real-time updates

use std::path::PathBuf;
use std::process::Command;

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

    /// Get the default socket path (~/.clash/daemon.sock).
    pub fn default_socket_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clash")
            .join("daemon.sock")
    }

    /// Connect to the daemon, auto-starting it if needed.
    pub async fn connect(&mut self) -> std::io::Result<()> {
        match UnixStream::connect(&self.socket_path).await {
            Ok(stream) => {
                self.setup_stream(stream);
                Ok(())
            }
            Err(_) => {
                self.start_daemon()?;
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

    /// Take the stream event receiver (Output, Exited) for the event loop.
    ///
    /// Call after `connect()`. The event loop owns this receiver and
    /// processes daemon output concurrently with terminal input.
    pub fn take_stream_rx(&mut self) -> Option<mpsc::UnboundedReceiver<Event>> {
        self.stream_event_rx.take()
    }

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
}
