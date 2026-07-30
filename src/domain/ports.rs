//! Port interfaces — contracts that the domain defines and infrastructure implements.
//!
//! These traits follow the Dependency Inversion Principle: the inner layers
//! (domain/application) define the interfaces, outer layers (infrastructure)
//! provide the implementations.

use std::path::PathBuf;

use crate::domain::entities::{
    ConversationMessage, InboxMessage, ScratchNote, Session, Subagent, Task, Team,
};
use crate::domain::error::Result;
use crate::domain::workflow::{Annotation, AnnotationsFile, WorkflowItem, WorkflowMeta};

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

    /// Rename a team: move its config dir and its tasks dir to `new_name`.
    /// Errors if `new_name` is invalid or already exists. Default no-op keeps
    /// lightweight mock backends valid (real impl on `FsBackend`).
    fn rename_team(&self, _old: &str, _new_name: &str) -> Result<()> {
        Ok(())
    }

    /// Delete a single task from a team. Default no-op for mock backends.
    fn delete_task(&self, _team: &str, _task_id: &str) -> Result<()> {
        Ok(())
    }

    /// Load an agent's inbox messages (`teams/{team}/inboxes/{agent}.json`).
    /// Returns an empty list if the file is absent. Default empty impl for
    /// mock backends.
    fn load_inbox(&self, _team: &str, _agent: &str) -> Result<Vec<InboxMessage>> {
        Ok(Vec::new())
    }

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
    /// Reached from the TUI via `Effect::MoveScratch` (the `m` folder picker)
    /// and from the GUI's drag-and-drop (which calls the Tauri command).
    fn move_scratch(&self, _id: &str, _new_parent: &str) -> Result<ScratchNote> {
        Ok(ScratchNote::default())
    }

    /// Delete the entry at `id`. Folders are removed recursively.
    fn delete_scratch_note(&self, _id: &str) -> Result<()> {
        Ok(())
    }
}

/// Port for workflow-item storage — the plan → review → implement → PR
/// pipeline, stored under the dedicated workflows root (independent of
/// scratches). Items are identified by `(project, slug)`: the directory path
/// is the identity. Unlike scratch notes, document contents ARE read/written
/// through the port — these are clash-owned structured files the agent
/// co-edits. Default impls keep mock backends valid.
///
/// Kept separate from [`DataRepository`] so that trait stays minimal, and
/// because in v1 only the GUI consumes workflows (via Tauri commands) — the
/// TUI binary compiles this as dead code (hence the `allow`); a future TUI
/// view plugs in with a single refresh effect.
#[allow(dead_code)]
pub trait WorkflowRepository: Send + Sync {
    /// List every workflow item, sorted by project then slug. One malformed
    /// item is skipped, never failing the list.
    fn load_workflow_items(&self) -> Result<Vec<WorkflowItem>> {
        Ok(Vec::new())
    }

    /// Create an item under `project` with a slug derived from `title`
    /// (deduplicated). Seeds meta/plan/review/annotations files.
    fn create_workflow_item(
        &self,
        _project: &str,
        _title: &str,
        _repo_path: &str,
    ) -> Result<WorkflowItem> {
        Ok(WorkflowItem::default())
    }

    /// Read an item's `meta.json`.
    fn load_workflow_meta(&self, _project: &str, _slug: &str) -> Result<WorkflowMeta> {
        Ok(WorkflowMeta::default())
    }

    /// Persist an item's `meta.json` (stamps `updatedAt`).
    fn write_workflow_meta(&self, _project: &str, _slug: &str, _meta: &WorkflowMeta) -> Result<()> {
        Ok(())
    }

    /// Read `plan.md` or `review.md` (whitelisted; missing reads as empty).
    fn read_workflow_doc(&self, _project: &str, _slug: &str, _doc: &str) -> Result<String> {
        Ok(String::new())
    }

    /// Write `plan.md` or `review.md` (whitelisted), atomically.
    fn write_workflow_doc(
        &self,
        _project: &str,
        _slug: &str,
        _doc: &str,
        _content: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Append an iteration section (note + open-annotation digest) to the
    /// `review.md` audit trail.
    fn append_workflow_review_iteration(
        &self,
        _project: &str,
        _slug: &str,
        _iteration: u32,
        _note: &str,
        _open_annotations: &[Annotation],
    ) -> Result<()> {
        Ok(())
    }

    /// Read `annotations.json` (missing reads as empty; malformed is an
    /// error so a blind save can never clobber review data).
    fn load_workflow_annotations(&self, _project: &str, _slug: &str) -> Result<AnnotationsFile> {
        Ok(AnnotationsFile::default())
    }

    /// Persist `annotations.json` atomically.
    fn write_workflow_annotations(
        &self,
        _project: &str,
        _slug: &str,
        _file: &AnnotationsFile,
    ) -> Result<()> {
        Ok(())
    }

    /// Snapshot the current iteration's diff + annotations into
    /// `history/{iteration:03}/`. Returns the snapshotted iteration. Never
    /// bumps `iteration` — the caller owns the follow-up meta write.
    fn snapshot_workflow_iteration(&self, _project: &str, _slug: &str, _diff: &str) -> Result<u32> {
        Ok(0)
    }

    /// List snapshotted iterations for an item (works for terminal items
    /// too, unlike the listing DTO's summary field).
    fn list_workflow_history(&self, _project: &str, _slug: &str) -> Result<Vec<u32>> {
        Ok(Vec::new())
    }

    /// Read a snapshotted diff for an iteration.
    fn read_workflow_history_diff(
        &self,
        _project: &str,
        _slug: &str,
        _iteration: u32,
    ) -> Result<String> {
        Ok(String::new())
    }

    /// Delete an item directory recursively.
    fn delete_workflow_item(&self, _project: &str, _slug: &str) -> Result<()> {
        Ok(())
    }
}
