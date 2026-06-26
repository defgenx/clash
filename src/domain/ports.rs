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
    // Free-form text files and folders kept under `~/.claude/clash/scratch/`,
    // organized as an IntelliJ-style tree. Entry ids are POSIX paths relative
    // to the scratch root (`""` denotes the root itself). These methods
    // list/create/rename/move/remove entries; the file *is* the note, so
    // contents are never read or written here. Default impls let lightweight
    // mock backends ignore scratch entirely.

    /// List the whole scratch tree, depth-first pre-order (folders first).
    fn load_scratch_notes(&self) -> Result<Vec<ScratchNote>> {
        Ok(Vec::new())
    }

    /// Create a new, empty scratch note titled `title` inside the folder at
    /// `parent` (relative path; `""` = root). Returns the created note.
    /// Errors if an entry with that name already exists.
    fn create_scratch_note(&self, _parent: &str, _title: &str) -> Result<ScratchNote> {
        Ok(ScratchNote::default())
    }

    /// Create a new folder named `name` inside the folder at `parent`
    /// (relative path; `""` = root). Returns the created folder entry.
    fn create_scratch_dir(&self, _parent: &str, _name: &str) -> Result<ScratchNote> {
        Ok(ScratchNote::default())
    }

    /// Rename the entry at `id` (file or folder) to `new_name`, keeping it in
    /// the same parent folder. Returns the renamed entry.
    fn rename_scratch(&self, _id: &str, _new_name: &str) -> Result<ScratchNote> {
        Ok(ScratchNote::default())
    }

    /// Move the entry at `id` into the folder at `new_parent` (`""` = root),
    /// keeping its name. Rejects moving a folder into itself or a descendant.
    /// Returns the moved entry at its new location.
    ///
    /// `dead_code` is allowed because the only non-test caller is the sibling
    /// `clash-gui` crate (drag-and-drop). The TUI reorganizes via create/
    /// rename/delete, so it never emits a move effect — mirrors `set_scratch_dir`.
    #[allow(dead_code)]
    fn move_scratch(&self, _id: &str, _new_parent: &str) -> Result<ScratchNote> {
        Ok(ScratchNote::default())
    }

    /// Delete the entry at `id`. Folders are removed recursively.
    fn delete_scratch_note(&self, _id: &str) -> Result<()> {
        Ok(())
    }
}
