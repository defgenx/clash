#[derive(Debug, Clone)]
pub enum AgentAction {
    Attach {
        session_id: String,
    },
    /// Spawn a new interactive Claude session in the given directory.
    SpawnSession {
        cwd: String,
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
    /// Delete all sessions.
    DeleteAllSessions,
}
