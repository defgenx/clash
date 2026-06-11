pub mod attach;
/// Lib-only API: the TUI binary compiles this module privately without
/// calling it (the picker is a GUI feature) — silence its dead-code lint.
#[allow(dead_code)]
pub mod terminal_choice;
pub mod terminal_spawn;
