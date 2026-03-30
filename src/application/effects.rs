//! Effects — side effects returned by the reducer for the infrastructure to execute.
//!
//! **Clean Architecture principle**: Effects are domain-level descriptions of
//! what should happen, not how. They contain no file paths, no serialization,
//! no framework types. The infrastructure layer translates them into real IO.

use crate::application::actions::Action;
use crate::domain::entities::Task;

/// Side effects returned by the reducer.
///
/// The reducer is a pure function: it takes state + action and returns
/// new state + a list of effects. The `App` coordinator then executes
/// these effects asynchronously.
#[derive(Debug, Clone)]
pub enum Effect {
    // ── Domain persistence effects ──────────────────────────────
    PersistTask {
        team: String,
        task: Task,
    },
    RemoveTeam {
        name: String,
    },

    // ── CLI effects ─────────────────────────────────────────────
    RunCli {
        command: CliCommand,
        on_complete: Action,
    },

    // ── Session effects ────────────────────────────────────────
    DaemonAttach {
        session_id: String,
        /// CLI args for the subprocess. Empty means `--resume <session_id>`.
        args: Vec<String>,
        /// Working directory for the subprocess.
        cwd: Option<String>,
        /// Optional session name to persist (for new sessions).
        name: Option<String>,
    },
    /// Start a session in the daemon without entering passthrough (background).
    DaemonStart {
        session_id: String,
        /// CLI args for the subprocess. Empty means `--resume <session_id>`.
        args: Vec<String>,
        /// Working directory for the subprocess.
        cwd: Option<String>,
        /// Optional session name to persist.
        name: Option<String>,
    },
    DaemonKill {
        session_id: String,
    },
    DaemonKillAll,
    /// Find and terminate the external Claude process for this session,
    /// and kill any associated tmux session.
    TerminateProcess {
        session_id: String,
        worktree: Option<String>,
    },
    /// Find and terminate all external Claude processes for all sessions.
    TerminateAllProcesses,

    // ── Data refresh effects ────────────────────────────────────
    RefreshAll,
    RefreshTeamTasks {
        team: String,
    },
    RefreshSessions,
    RefreshSubagents {
        project: String,
        session_id: String,
    },
    LoadConversation {
        project: String,
        session_id: String,
    },
    /// Lazy-load repo config for a session (for display in SessionDetail).
    LoadRepoConfig {
        session_id: String,
    },
    /// Run `git diff HEAD` in a session's project directory.
    LoadDiff {
        session_id: String,
    },
    LoadSubagentConversation {
        project: String,
        session_id: String,
        agent_id: String,
    },
    /// Register a session in the clash session registry.
    RegisterSession {
        session_id: String,
        name: String,
        cwd: String,
        source_branch: Option<String>,
    },
    /// Remove a session from the clash session registry.
    UnregisterSession {
        session_id: String,
    },
    /// Clear all sessions from the clash session registry.
    ClearSessionRegistry,
    /// Rename a session in the clash session registry.
    RenameSession {
        session_id: String,
        name: String,
    },
    /// Mark a session as idle in the clash status file (persists across refreshes).
    MarkSessionIdle {
        session_id: String,
    },
    /// Mark all sessions as idle in their clash status files.
    MarkAllSessionsIdle,
    /// Write the quit-stash marker with pre-captured session IDs.
    /// Must execute before DaemonKillAll to avoid the race where daemon
    /// kill → SessionExited → refresh removes sessions from store before
    /// their IDs are captured. See QuitConfirmed in reducer.rs.
    WriteQuitStash {
        session_ids: Vec<String>,
    },
    /// Create a git worktree, then spawn a new daemon session inside it.
    CreateWorktreeAndAttach {
        /// For existing-session flow: the session whose project_path/git_branch to use.
        source_session_id: Option<String>,
        /// Direct cwd for new-session flow.
        cwd: Option<String>,
        new_session_id: String,
        name: String,
    },

    /// Open a single session in a new pane/tab/window.
    /// Unlike DaemonAttach, this does NOT suspend the TUI.
    AttachInNewWindow {
        session_id: String,
    },
    /// Open multiple sessions with smart pane/tab layout.
    AttachBatchInNewWindows {
        session_ids: Vec<String>,
    },

    // ── IDE effects ────────────────────────────────────────────
    /// Detect available IDEs for the given project directory.
    DetectIdes {
        project_dir: String,
    },
    /// Open a project directory in an IDE.
    OpenIde {
        command: String,
        project_dir: String,
        terminal: bool,
    },

    // ── Preset effects ──────────────────────────────────────────
    /// Load presets from .clash/presets.json, global config, .superset/config.json.
    LoadPresets {
        project_dir: String,
    },
    /// Run setup scripts after session creation.
    RunSetupScripts {
        session_id: String,
        scripts: Vec<String>,
        cwd: String,
    },
    /// Run teardown scripts before session drop, then dispatch on_complete.
    RunTeardownScripts {
        scripts: Vec<String>,
        cwd: String,
        on_complete: Action,
    },

    // ── UI state effects ────────────────────────────────────────
    ShowSpinner(String),
    PerformUpdate,
    Quit,
}

/// High-level CLI commands (no raw args — infrastructure translates).
#[derive(Debug, Clone)]
pub enum CliCommand {
    CreateTeam { name: String, description: String },
}
