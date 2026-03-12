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
    LoadSubagentConversation {
        project: String,
        session_id: String,
        agent_id: String,
    },
    /// Mark a session as idle in the clash status file (persists across refreshes).
    MarkSessionIdle {
        session_id: String,
    },
    /// Mark all sessions as idle in their clash status files.
    MarkAllSessionsIdle,

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
