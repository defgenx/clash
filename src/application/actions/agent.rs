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
    /// Terminate the Claude process + delete session files.
    TerminateAndDelete {
        project: String,
        session_id: String,
    },
    /// Delete session files only (leave process running).
    DeleteSession {
        project: String,
        session_id: String,
    },
    /// Terminate all processes + delete all session files.
    TerminateAndDeleteAllSessions,
    /// Delete all session files only (leave processes running).
    DeleteAllSessions,
}
