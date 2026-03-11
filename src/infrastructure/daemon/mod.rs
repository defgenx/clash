//! Daemon infrastructure — persistent PTY session management.
//!
//! The daemon runs as a background process, managing PTY sessions that
//! survive TUI restarts. Multiple TUI clients can connect simultaneously.
//!
//! Architecture:
//! ```text
//! clash TUI (client) ──┐
//! clash TUI (client) ──┤── Unix Socket ── clash-daemon
//! clash TUI (client) ──┘                        │
//!                                       PTY sessions
//! ```

pub mod client;
pub mod protocol;
pub mod server;
pub mod session;
