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
    PersistTask { team: String, task: Task },
    RemoveTeam { name: String },

    // ── CLI effects ─────────────────────────────────────────────
    RunCli {
        command: CliCommand,
        on_complete: Action,
    },

    // ── Daemon-managed session effects ────────────────────────
    DaemonCreateSession {
        session_id: String,
        args: Vec<String>,
        cwd: String,
    },
    DaemonAttach { session_id: String },
    DaemonDetach { session_id: String },
    DaemonKill { session_id: String },

    // ── Data refresh effects ────────────────────────────────────
    RefreshAll,
    RefreshTeamTasks { team: String },
    RefreshSessions,
    RefreshSubagents { project: String, session_id: String },
    LoadConversation { project: String, session_id: String },
    LoadSubagentConversation { project: String, session_id: String, agent_id: String },
    DeleteSession { project: String, session_id: String },
    DeleteAllSessions,

    // ── UI state effects ────────────────────────────────────────
    ShowSpinner(String),
    Quit,
}

/// High-level CLI commands (no raw args — infrastructure translates).
#[derive(Debug, Clone)]
pub enum CliCommand {
    CreateTeam { name: String, description: String },
}
