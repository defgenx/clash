pub mod actions;
pub mod effects;
pub mod nav;
pub mod reducer;
pub mod state;
pub mod store;
/// Pure workflow logic is consumed only through the lib crate in v1 (GUI
/// Tauri commands); the binary's private-`mod` compilation would otherwise
/// flag it as dead code. The diff parser becomes bin-used once the TUI's
/// `parse_diff_lines` is rebuilt on it — narrow the `allow` then.
#[allow(dead_code)]
pub mod workflow;
