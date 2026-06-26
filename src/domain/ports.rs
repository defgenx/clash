//! Port interfaces — contracts that the domain defines and infrastructure implements.
//!
//! These traits follow the Dependency Inversion Principle: the inner layers
//! (domain/application) define the interfaces, outer layers (infrastructure)
//! provide the implementations.

use std::path::PathBuf;

use crate::domain::entities::{ConversationMessage, ScratchNote, Session, Subagent, Task, Team};
use crate::domain::error::Result;

/// Repository port for all data access operations.
///
/// Implemented by `FsBackend` in production and mock backends in tests.
pub trait DataRepository: Send + Sync {
    /// Load all teams.
    fn load_teams(&self) -> Result<Vec<Team>>;

    /// Create a team (writes its config under the teams dir). Errors if a
    /// team with that name already exists.
    fn create_team(&self, name: &str, description: &str) -> Result<()>;

    /// Persist a full team config (description, members, …). Errors if the
    /// team does not exist — creation goes through `create_team`.
    fn update_team(&self, team: &Team) -> Result<()>;

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

    // ── Scratch notes ───────────────────────────────────────────
    // Free-form text files kept under `~/.claude/clash/scratch/`. The file
    // itself is the note; these methods list/create/read/write/remove them.
    // Default impls let lightweight mock backends ignore scratch entirely.

    /// List all scratch notes (sorted by the implementation).
    fn load_scratch_notes(&self) -> Result<Vec<ScratchNote>> {
        Ok(Vec::new())
    }

    /// Create a new, empty scratch note from a user-supplied title.
    /// Returns the created note. Errors if a note with that name exists.
    fn create_scratch_note(&self, _title: &str) -> Result<ScratchNote> {
        Ok(ScratchNote::default())
    }

    /// Delete a scratch note by id (file name).
    fn delete_scratch_note(&self, _id: &str) -> Result<()> {
        Ok(())
    }
}
