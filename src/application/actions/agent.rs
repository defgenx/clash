#[derive(Debug, Clone)]
pub enum AgentAction {
    Attach {
        session_id: String,
    },
    /// Spawn a new interactive Claude session in the given directory.
    SpawnSession {
        cwd: String,
        /// Optional human-readable label for the session.
        name: Option<String>,
    },
    /// Drop a session: kill daemon PTY, terminate Claude process,
    /// unregister from clash registry. Session disappears from UI.
    DropSession {
        session_id: String,
    },
    /// Drop all sessions: kill all daemon PTYs, terminate all processes,
    /// clear the clash session registry.
    DropAllSessions,
    /// Spawn a new session inside a git worktree created from the selected session's project.
    SpawnInWorktree {
        session_id: String,
    },
    /// Spawn a new session in a worktree created from the given directory.
    SpawnSessionInWorktree {
        cwd: String,
        name: Option<String>,
    },
    /// Stash a session: terminate the Claude process and mark idle, but keep
    /// it in the registry. If already idle, unstash by re-attaching.
    StashSession {
        session_id: String,
    },
    /// Stash all running sessions, or unstash all idle sessions.
    StashAllSessions,
    /// Attach to a session in a new terminal window (TUI stays active).
    AttachNewWindow {
        session_id: String,
    },
    /// Open ALL running sessions in new terminal windows.
    AttachAllNewWindows,
    /// Open a session's project directory in an IDE.
    OpenInIde {
        session_id: String,
    },
    /// Rename a session (update its label in the registry).
    RenameSession {
        session_id: String,
        name: String,
    },
    /// Spawn a session from a named preset (resolved by reducer against store.presets).
    SpawnSessionFromPreset {
        preset_name: String,
    },
    /// Drop a session after teardown scripts have completed.
    DropSessionAfterTeardown {
        session_id: String,
    },
    /// Adopt a wild/external session in view-only mode — navigate to its
    /// detail view so the existing 1s-polled JSONL re-read shows the
    /// live conversation. Does NOT touch the PTY or contend with the
    /// foreground process driving the session elsewhere.
    AdoptViewOnly {
        session_id: String,
    },
    /// Open the adopt confirm dialog for a wild/external session. Wakes
    /// the background scan immediately so the dialog renders against
    /// the freshest PID snapshot, then sets `state.adopt_dialog`.
    OpenAdoptDialog {
        session_id: String,
    },
    /// Confirm takeover of a wild/external session — re-validate via
    /// `adoption_options`, then emit a single `Effect::TakeoverWildSession`
    /// that the infra layer translates into the SIGTERM → wait → resume
    /// sequence.
    TakeoverWild {
        session_id: String,
        pid: u32,
    },
    /// Confirm Convert on a wild/external session — re-validate via
    /// `adoption_options`, then emit a single `Effect::ConvertWildSession`
    /// that writes a registry entry for the session without touching the
    /// running process. The 🌿 marker stays until the user later attaches
    /// via the daemon, but the row is now persistent across clash and
    /// process restarts.
    ConvertWild {
        session_id: String,
    },
    /// Dismiss the adopt confirm dialog without taking any action.
    CloseAdoptDialog,
}
