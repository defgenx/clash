pub mod actions;
pub mod diff;
pub mod effects;
/// Same allowance as `workflow`: drill-down explanations are GUI-only in v1.
#[allow(dead_code)]
pub mod explain_tree;
/// Consumed only through the lib crate (the GUI's Files panel); the binary's
/// private-`mod` compilation would otherwise flag it as dead code.
#[allow(dead_code)]
pub mod explorer;
pub mod nav;
/// Same allowance as `workflow`: PR stacks are consumed by the GUI in v1.
#[allow(dead_code)]
pub mod pr_stack;
pub mod prompt_queue;
pub mod reducer;
/// Same allowance as `workflow`: the phone remote is served by the GUI.
#[allow(dead_code)]
pub mod remote;
/// Same allowance as `workflow`: review rounds are launched by the GUI in v1
/// (the TUI only relaunches them, through `RoundModels`).
#[allow(dead_code)]
pub mod review_size;
pub mod state;
pub mod store;
/// Same allowance as `workflow`: ticket trackers are used by the GUI's
/// workflow and share commands in v1.
#[allow(dead_code)]
pub mod trackers;
/// Same allowance as `workflow`: multi-wave items are driven by the GUI in v1.
#[allow(dead_code)]
pub mod waves;
/// Pure workflow logic is consumed only through the lib crate in v1 (GUI
/// Tauri commands); the binary's private-`mod` compilation would otherwise
/// flag it as dead code. Remove the `allow` once the TUI grows its read-only
/// workflows view. (Diff parsing lives in `diff`, which the TUI does use.)
#[allow(dead_code)]
pub mod workflow;
/// Same allowance: check runs are driven by the GUI's Tauri commands in v1.
#[allow(dead_code)]
pub mod workflow_run;
/// Same allowance as `workflow`: consumed only through the lib crate (GUI
/// share/export commands) in v1.
#[allow(dead_code)]
pub mod workflow_share;
