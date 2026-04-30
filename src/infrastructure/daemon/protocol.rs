//! Wire protocol for daemon ↔ client communication.
//!
//! Uses NDJSON (newline-delimited JSON) over Unix domain sockets.
//! Terminal data is base64-encoded to avoid escape-sequence issues in JSON.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ── Client → Daemon requests ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// List all managed PTY sessions.
    ListSessions,

    /// Create a new PTY session (spawns claude).
    CreateSession {
        /// Unique session ID (caller-generated or UUID).
        session_id: String,
        /// Claude binary path.
        bin: String,
        /// CLI arguments (e.g. ["--resume", "abc123"]).
        args: Vec<String>,
        /// Working directory (empty = inherit).
        cwd: String,
        /// Optional human-readable label for the session.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Initial terminal width (0 = default 120).
        #[serde(default)]
        cols: u16,
        /// Initial terminal height (0 = default 40).
        #[serde(default)]
        rows: u16,
        /// Additional environment variables to set in the spawned process.
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        env_vars: HashMap<String, String>,
    },

    /// Attach to an existing session (start receiving output).
    /// The server replays the session's output history in chunks after
    /// Ok so the client can paint Claude's last screen state — including
    /// background colors, cursor position, and SGR attributes — before
    /// live output begins.
    Attach { session_id: String },

    /// Detach from a session (stop receiving output).
    Detach { session_id: String },

    /// Send input bytes to a session's PTY.
    Input {
        session_id: String,
        /// Base64-encoded bytes.
        data: String,
    },

    /// Resize a session's PTY.
    Resize {
        session_id: String,
        cols: u16,
        rows: u16,
    },

    /// Kill a session.
    Kill { session_id: String },

    /// Ping (keepalive).
    Ping,

    /// Request daemon shutdown.
    Shutdown,
}

// ── Daemon → Client events ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// Response to ListSessions.
    Sessions { sessions: Vec<SessionInfo> },

    /// Terminal output from a session.
    Output {
        session_id: String,
        /// Base64-encoded bytes.
        data: String,
    },

    /// Session exited.
    Exited {
        session_id: String,
        exit_code: Option<i32>,
    },

    /// Acknowledgement (success).
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },

    /// Error response.
    Error { message: String },

    /// Pong (keepalive response).
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub pid: u32,
    pub is_alive: bool,
    pub attached_clients: usize,
    pub created_at: u64,
    /// Session status: "running", "waiting", "idle".
    #[serde(default)]
    pub status: String,
    /// Working directory the session was spawned in.
    #[serde(default)]
    pub cwd: String,
    /// Optional human-readable label for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

// ── Helpers ──────────────────────────────────────────────────────

/// Encode bytes as base64 for JSON transport.
pub fn encode_data(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Decode base64 data from JSON transport.
pub fn decode_data(encoded: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(encoded)
}
