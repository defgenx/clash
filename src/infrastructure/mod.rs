pub mod app;
pub mod clipboard;
pub mod config;
pub mod daemon;
pub mod env_path;
pub mod error;
pub mod event;
pub mod fs;
/// gh integration is consumed only through the lib crate (GUI PR commands);
/// the binary's private-`mod` compilation would otherwise flag it as dead.
#[allow(dead_code)]
pub mod gh;
pub mod git;
pub mod hooks;
pub mod ide;
pub mod logging;
pub mod process_scan;
pub mod session_refresh;
pub mod skills;
pub mod tui;
pub mod update;
pub mod windowing;
