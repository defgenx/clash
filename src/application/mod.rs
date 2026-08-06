pub mod actions;
pub mod diff;
pub mod effects;
pub mod nav;
pub mod reducer;
pub mod state;
pub mod store;
/// Pure workflow logic is consumed only through the lib crate in v1 (GUI
/// Tauri commands); the binary's private-`mod` compilation would otherwise
/// flag it as dead code. Remove the `allow` once the TUI grows its read-only
/// workflows view. (Diff parsing lives in `diff`, which the TUI does use.)
#[allow(dead_code)]
pub mod workflow;
/// Same allowance as `workflow`: consumed only through the lib crate (GUI
/// share/export commands) in v1.
#[allow(dead_code)]
pub mod workflow_share;
