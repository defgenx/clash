//! Port interfaces — contracts that the domain defines and infrastructure implements.
//!
//! These traits follow the Dependency Inversion Principle: the inner layers
//! (domain/application) define the interfaces, outer layers (infrastructure)
//! provide the implementations.

use std::path::PathBuf;

use crate::domain::entities::{ConversationMessage, Session, Subagent, Task, Team};
use crate::domain::error::Result;

/// Repository port for all data access operations.
///
/// Implemented by `FsBackend` in production and mock backends in tests.
pub trait DataRepository: Send + Sync {
    /// Load all teams.
    fn load_teams(&self) -> Result<Vec<Team>>;

    /// Load tasks for a specific team.
    fn load_tasks(&self, team: &str) -> Result<Vec<Task>>;

    /// Persist a task (create or update).
    fn write_task(&self, team: &str, task: &Task) -> Result<()>;

    /// Delete a team and all associated data.
    fn delete_team(&self, name: &str) -> Result<()>;

    /// Get the base directory for teams.
    fn teams_dir(&self) -> PathBuf;

    /// Get the base directory for tasks.
    fn tasks_dir(&self) -> PathBuf;

    /// Load all Claude Code sessions from ~/.claude/projects/.
    fn load_sessions(&self) -> Result<Vec<Session>>;

    /// Load subagents for a specific session.
    fn load_subagents(&self, project: &str, session_id: &str) -> Result<Vec<Subagent>>;

    /// Load conversation messages from a session .jsonl file.
    fn load_conversation(
        &self,
        project: &str,
        session_id: &str,
    ) -> Result<Vec<ConversationMessage>>;

    /// Load conversation messages from a subagent .jsonl file.
    fn load_subagent_conversation(
        &self,
        project: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<ConversationMessage>>;

    /// Delete a session .jsonl file.
    fn delete_session(&self, project: &str, session_id: &str) -> Result<()>;

    /// Delete all sessions across all projects.
    fn delete_all_sessions(&self) -> Result<()>;
}

/// CLI gateway output.
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// CLI gateway port for running Claude CLI commands.
///
/// Implemented by `RealCliRunner` in production and `MockCliRunner` in tests.
pub trait CliGateway: Send + Sync {
    /// Run a CLI command and return the output.
    fn run(&self, args: &[String]) -> impl std::future::Future<Output = Result<CliOutput>> + Send;
}
