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
}
