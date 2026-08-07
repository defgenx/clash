pub mod app;
pub mod clipboard;
pub mod config;
pub mod daemon;
pub mod env_path;
pub mod error;
pub mod event;
/// Forge implementations are consumed only through the lib crate (GUI Tauri
/// commands), like `domain::forge` — same dead-code allowance, same exit
/// condition (a TUI with PR features).
#[allow(dead_code)]
pub mod forge;
pub mod fs;
/// gh integration is consumed only through the lib crate (GUI PR commands);
/// the binary's private-`mod` compilation would otherwise flag it as dead.
#[allow(dead_code)]
pub mod gh;
pub mod git;
pub mod hooks;
pub mod ide;
/// Jira transport is consumed only through the lib crate (the GUI share
/// command) — same dead-code allowance as `gh`/`forge`.
#[allow(dead_code)]
pub mod jira;
pub mod logging;
pub mod process_scan;
pub mod session_refresh;
pub mod skills;
pub mod tui;
pub mod update;
/// Webhook transport is consumed only through the lib crate (GUI share and
/// notify commands) — same dead-code allowance as `gh`/`forge`.
#[allow(dead_code)]
pub mod webhook;
pub mod windowing;
