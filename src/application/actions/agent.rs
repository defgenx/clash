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
}
