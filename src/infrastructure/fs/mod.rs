pub mod atomic;
pub mod backend;
pub mod presets;
pub mod repo_config;
pub mod store;
pub mod watcher;
/// Workflow storage is consumed only through the lib crate in v1 (GUI
/// Tauri commands via `WorkflowRepository`); the binary's private-`mod`
/// compilation would otherwise flag it as dead code.
#[allow(dead_code)]
pub mod workflows;
