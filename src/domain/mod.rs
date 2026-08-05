pub mod entities;
pub mod error;
/// The forge port shares `workflow`'s dead-code situation: consumed only
/// through the lib crate (GUI Tauri commands), so the binary's private-`mod`
/// build would flag it. Remove the `allow` when the TUI gains PR features.
#[allow(dead_code)]
pub mod forge;
pub mod ports;
/// Workflow entities live in their own module because in v1 they are consumed
/// only through the lib crate (GUI Tauri commands + port impls); the binary's
/// private-`mod` compilation would otherwise flag the whole API as dead code.
/// Remove the `allow` once the TUI grows its read-only workflows view.
#[allow(dead_code)]
pub mod workflow;
