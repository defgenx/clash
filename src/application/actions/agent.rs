#[derive(Debug, Clone)]
pub enum AgentAction {
    Attach {
        session_id: String,
    },
    /// Spawn a new interactive Claude session.
    SpawnSession,
    /// Delete a session.
    DeleteSession {
        project: String,
        session_id: String,
    },
    /// Delete all sessions.
    DeleteAllSessions,
}
