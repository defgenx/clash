//! Domain entities — the core data model.
//!
//! These types represent the business domain of Claude Code Agent Teams.
//! They use `#[serde(default)]` and `#[serde(flatten)]` for resilience
//! against schema changes in Claude Code's JSON files.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A team member / agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub joined_at: Option<serde_json::Value>,
    #[serde(default)]
    pub tmux_pane_id: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub backend_type: Option<String>,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub mode: Option<String>,
    /// Capture unknown fields for forward compatibility.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
    /// Team name — populated by DataStore::rebuild_all_members(), not from JSON.
    #[serde(skip)]
    pub team_name: String,
}

/// A team configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Team {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub created_at: Option<serde_json::Value>,
    #[serde(default)]
    pub lead_agent_id: Option<String>,
    #[serde(default)]
    pub lead_session_id: Option<String>,
    #[serde(default)]
    pub members: Vec<Member>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Task status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Blocked,
    #[serde(other)]
    Unknown,
}

impl TaskStatus {
    /// Cycle to the next status.
    pub fn next(&self) -> Self {
        match self {
            Self::Pending => Self::InProgress,
            Self::InProgress => Self::Completed,
            Self::Completed => Self::Pending,
            Self::Blocked => Self::Pending,
            Self::Unknown => Self::Pending,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub active_form: Option<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// High-level session lifecycle section — groups statuses for display and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionSection {
    Active = 0, // Thinking, Running, Starting, Prompting, Waiting
    Done = 1,   // Stashed
    Fail = 2,   // Errored
}

impl SessionSection {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Done => "DONE",
            Self::Fail => "FAIL",
        }
    }
}

/// Where the running Claude process for a session lives, from clash's POV.
///
/// Computed at runtime from cross-referencing the daemon's session list, the
/// in-memory `externally_opened` set, and a periodic ps/lsof scan. Never
/// persisted to disk — `#[serde(skip)]` on the Session field.
///
/// Precedence (highest first): `Daemon > External > Wild > Unknown`. `External`
/// requires both a membership in `externally_opened` AND a wild PID match — that
/// way it self-heals across clash restarts (an empty `externally_opened` after
/// restart simply demotes the row to `Wild`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SessionSource {
    /// PTY managed by clash's own daemon. Default — what every session in the
    /// daemon's session list maps to.
    Daemon,
    /// Spawned by clash via `o`/`O` into another pane/tab/window AND its
    /// process is still detectable by the wild scan.
    External,
    /// Running `claude` process detected outside clash's daemon — bare
    /// invocation in some terminal, or a leftover from a crashed clash whose
    /// `externally_opened` set was lost.
    Wild,
    /// No correlation could be made (e.g. session file exists on disk but no
    /// matching daemon entry and no live PID found).
    #[default]
    Unknown,
}

/// What adoption actions the user can perform on a session row.
///
/// Single source of truth, derived purely from `(Session.source, Session.status)`
/// via [`Session::adoption_options`]. Consumed by the input handler, the
/// reducer, and the confirm dialog so the rule never drifts between sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdoptionOptions {
    /// Whether read-only conversation tail is offered (always at least
    /// available when *any* adoption action is allowed, since it's strictly
    /// safer than takeover).
    pub view_only: bool,
    /// Whether SIGTERM-and-resume takeover is offered.
    pub takeover: bool,
    /// Reason why no option is available, if both are false. Stable strings —
    /// rendered verbatim in the status-bar hint.
    pub reason_disabled: Option<&'static str>,
}

impl AdoptionOptions {
    pub const fn none(reason: &'static str) -> Self {
        Self {
            view_only: false,
            takeover: false,
            reason_disabled: Some(reason),
        }
    }
}

/// Granular session status — detected by parsing the terminal screen content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// Session process is dead or exited.
    #[default]
    #[serde(rename = "idle", alias = "Idle")]
    Stashed,
    /// Just spawned, not yet producing output.
    Starting,
    /// Actively producing output — Claude is executing tools, writing code.
    Running,
    /// Claude is reasoning / generating (thinking indicator visible or brief output pause).
    Thinking,
    /// Claude is waiting for free-form user text input (prompt visible).
    Waiting,
    /// Claude is asking for tool/action approval — needs Yes/No response.
    Prompting,
    /// Session process died shortly after starting (crash, bad resume, etc.).
    Errored,
    /// Subagent completed its work (not a session-level status).
    #[serde(rename = "done")]
    Done,
}

impl SessionStatus {
    /// Map this status to its high-level session section.
    pub fn section(&self) -> SessionSection {
        match self {
            Self::Thinking | Self::Running | Self::Starting | Self::Prompting | Self::Waiting => {
                SessionSection::Active
            }
            Self::Stashed | Self::Done => SessionSection::Done,
            Self::Errored => SessionSection::Fail,
        }
    }
}

impl std::str::FromStr for SessionStatus {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "idle" => Ok(SessionStatus::Stashed),
            "starting" => Ok(SessionStatus::Starting),
            "running" => Ok(SessionStatus::Running),
            "thinking" => Ok(SessionStatus::Thinking),
            "waiting" => Ok(SessionStatus::Waiting),
            "prompting" => Ok(SessionStatus::Prompting),
            "errored" => Ok(SessionStatus::Errored),
            "done" => Ok(SessionStatus::Done),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Stashed => write!(f, "Stashed"),
            SessionStatus::Starting => write!(f, "Starting"),
            SessionStatus::Running => write!(f, "Running"),
            SessionStatus::Thinking => write!(f, "Thinking"),
            SessionStatus::Waiting => write!(f, "Waiting"),
            SessionStatus::Prompting => write!(f, "Prompting"),
            SessionStatus::Errored => write!(f, "Errored"),
            SessionStatus::Done => write!(f, "Done"),
        }
    }
}

/// A Claude Code session (from ~/.claude/projects/*/sessions-index.json).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub project_path: String,
    #[serde(default)]
    pub last_modified: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub first_prompt: String,
    #[serde(default)]
    pub has_subagents: bool,
    #[serde(default)]
    pub subagent_count: usize,
    #[serde(default)]
    pub git_branch: String,
    /// Whether this session is currently active (file modified recently).
    #[serde(default)]
    pub is_running: bool,
    /// Granular session status.
    #[serde(default)]
    pub status: SessionStatus,
    /// Git worktree name, if the session is running inside a worktree.
    #[serde(default)]
    pub worktree: Option<String>,
    /// Parent project name for the worktree (display only, computed at runtime).
    #[serde(skip)]
    pub worktree_project: Option<String>,
    /// Optional human-readable label for the session.
    #[serde(default)]
    pub name: Option<String>,
    /// Working directory where the session was started (from clash registry).
    #[serde(default)]
    pub cwd: Option<String>,
    /// The original branch a worktree session was created from.
    #[serde(default)]
    pub source_branch: Option<String>,
    /// Which preset was used to create this session (for teardown lookup).
    #[serde(default)]
    pub preset_name: Option<String>,
    /// Repo-level configuration discovered from the session's cwd (lazy-loaded, not serialized).
    #[serde(skip)]
    pub repo_config: Option<RepoConfig>,
    /// Where the running process for this session lives (Daemon/External/Wild/Unknown).
    /// Computed at runtime by the refresh pipeline; never on disk.
    #[serde(skip)]
    pub source: SessionSource,
    /// PID of the wild claude process backing this session, if `source`
    /// is `Wild` or `External`. Populated by the precedence overlay
    /// alongside `source` so the adopt dialog can hand it to the
    /// takeover effect without a second correlation pass. Never on disk.
    #[serde(skip)]
    pub wild_pid: Option<u32>,
}

impl Session {
    /// Case-insensitive text filter match on key fields.
    pub fn matches_filter(&self, filter: &str) -> bool {
        let f = filter.to_lowercase();
        self.id.to_lowercase().contains(&f)
            || self.summary.to_lowercase().contains(&f)
            || self.project_path.to_lowercase().contains(&f)
            || self.git_branch.to_lowercase().contains(&f)
            || self.first_prompt.to_lowercase().contains(&f)
            || self
                .name
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains(&f)
            || self
                .source_branch
                .as_deref()
                .unwrap_or("")
                .to_lowercase()
                .contains(&f)
    }

    /// What the user can do via `a` (adopt) on this session.
    ///
    /// Single source of truth for adoption eligibility — consumed by the input
    /// handler (status-bar hint vs. dialog), the reducer (validates the action
    /// before emitting effects), and the confirm dialog (which buttons render).
    pub fn adoption_options(&self) -> AdoptionOptions {
        match self.source {
            SessionSource::Daemon => {
                AdoptionOptions::none("daemon-managed — already attachable with o/Enter")
            }
            SessionSource::Unknown => AdoptionOptions::none("no live process detected"),
            SessionSource::Wild | SessionSource::External => match self.status {
                // Active statuses: both options offered. Takeover warns about
                // in-flight tool calls in the confirm dialog itself.
                SessionStatus::Running
                | SessionStatus::Thinking
                | SessionStatus::Starting
                | SessionStatus::Prompting
                | SessionStatus::Waiting => AdoptionOptions {
                    view_only: true,
                    takeover: true,
                    reason_disabled: None,
                },
                // Process is gone — nothing to adopt or take over.
                SessionStatus::Stashed | SessionStatus::Done | SessionStatus::Errored => {
                    AdoptionOptions::none("process is no longer running")
                }
            },
        }
    }
}

/// A subagent spawned by a session or another agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Subagent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub parent_session_id: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub last_modified: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub file_path: String,
    /// Whether this subagent is currently active.
    #[serde(default)]
    pub is_running: bool,
    /// Granular status (same as sessions).
    #[serde(default)]
    pub status: SessionStatus,
    /// Git worktree name, if the subagent is running inside a worktree.
    #[serde(default)]
    pub worktree: Option<String>,
    /// Parent project name for the worktree (display only, computed at runtime).
    #[serde(skip)]
    pub worktree_project: Option<String>,
}

/// A conversation message from a session or subagent .jsonl file.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub role: String, // "user" or "assistant"
    pub text: String,
}

/// Repo-level configuration discovered from a session's working directory.
/// Built programmatically by infrastructure — not deserialized from a single JSON file.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RepoConfig {
    pub setup_scripts: Vec<String>,
    pub teardown_scripts: Vec<String>,
    pub mcp_servers: Vec<String>,
    /// Path to .mcp.json if it exists (for display; Claude auto-discovers from cwd).
    pub mcp_config_path: Option<String>,
    pub custom_commands: Vec<String>,
    pub agent_definitions: Vec<String>,
    pub has_claude_settings: bool,
}

/// A session preset — reusable template for session creation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Preset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Working directory (relative or absolute).
    #[serde(default)]
    pub directory: String,
    /// Initial prompt for Claude.
    #[serde(default)]
    pub prompt: String,
    /// None = ask, Some = auto-decide.
    #[serde(default)]
    pub worktree: Option<bool>,
    #[serde(default)]
    pub setup: Vec<String>,
    #[serde(default)]
    pub teardown: Vec<String>,
    #[serde(skip)]
    pub source: PresetSource,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Where a preset was loaded from.
#[derive(Debug, Clone, Default)]
pub enum PresetSource {
    #[default]
    Project,
    Global,
    Superset,
}

/// Container for .clash/presets.json.
#[derive(Debug, Default, Deserialize)]
pub struct PresetFile {
    #[serde(default)]
    pub presets: HashMap<String, Preset>,
}

/// Superset-compatible config (.superset/config.json).
#[derive(Debug, Default, Deserialize)]
pub struct SupersetConfig {
    #[serde(default)]
    pub setup: Vec<String>,
    #[serde(default)]
    pub teardown: Vec<String>,
}

/// An inbox message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub timestamp: Option<serde_json::Value>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub read: bool,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_team_minimal() {
        let json = r#"{"name": "test-team"}"#;
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "test-team");
        assert!(team.members.is_empty());
    }

    #[test]
    fn test_parse_team_with_extra_fields() {
        let json = r#"{"name": "test", "unknownField": 42, "anotherNew": "hi"}"#;
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "test");
        assert_eq!(
            team.extra.get("unknownField").unwrap(),
            &serde_json::json!(42)
        );
    }

    #[test]
    fn test_parse_team_empty_json() {
        let json = "{}";
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "");
        assert!(team.members.is_empty());
    }

    #[test]
    fn test_parse_member_defaults() {
        let json = "{}";
        let member: Member = serde_json::from_str(json).unwrap();
        assert_eq!(member.name, "");
        assert!(!member.is_active);
    }

    #[test]
    fn test_task_status_cycle() {
        assert_eq!(TaskStatus::Pending.next(), TaskStatus::InProgress);
        assert_eq!(TaskStatus::InProgress.next(), TaskStatus::Completed);
        assert_eq!(TaskStatus::Completed.next(), TaskStatus::Pending);
        assert_eq!(TaskStatus::Blocked.next(), TaskStatus::Pending);
    }

    #[test]
    fn test_parse_task_with_status() {
        let json = r#"{"id": "1", "subject": "Test", "status": "in_progress"}"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
    }

    #[test]
    fn test_parse_task_unknown_status() {
        let json = r#"{"id": "1", "status": "something_new"}"#;
        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.status, TaskStatus::Unknown);
    }

    #[test]
    fn test_parse_inbox_message() {
        let json = r#"{"from": "agent-1", "text": "hello", "read": false}"#;
        let msg: InboxMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.from, "agent-1");
        assert!(!msg.read);
    }

    #[test]
    fn test_session_matches_filter() {
        let session = Session {
            id: "abc12345".to_string(),
            summary: "Fix login bug".to_string(),
            project_path: "/home/user/myproject".to_string(),
            git_branch: "feature/auth".to_string(),
            first_prompt: "Please fix the auth flow".to_string(),
            ..Default::default()
        };
        assert!(session.matches_filter("abc"));
        assert!(session.matches_filter("login"));
        assert!(session.matches_filter("myproject"));
        assert!(session.matches_filter("auth"));
        assert!(session.matches_filter("fix the auth"));
        assert!(!session.matches_filter("nonexistent"));
        // Case insensitive
        assert!(session.matches_filter("FIX LOGIN"));
    }

    #[test]
    fn test_matches_filter_source_branch() {
        let session = Session {
            id: "abc12345".to_string(),
            source_branch: Some("feature/auth".to_string()),
            ..Default::default()
        };
        assert!(session.matches_filter("auth"));
        assert!(session.matches_filter("feature"));
        assert!(!session.matches_filter("nonexistent"));
    }

    #[test]
    fn test_malformed_json_fails() {
        let json = "not json at all";
        assert!(serde_json::from_str::<Team>(json).is_err());
    }

    #[test]
    fn test_parse_full_team() {
        let json = r##"{
            "name": "my-team",
            "description": "A test team",
            "createdAt": "2025-01-15T10:30:00Z",
            "leadAgentId": "agent-lead",
            "leadSessionId": "session-123",
            "members": [
                {
                    "agentId": "agent-1",
                    "name": "researcher",
                    "agentType": "claude",
                    "model": "sonnet",
                    "isActive": true,
                    "color": "#ff0000"
                },
                {
                    "agentId": "agent-2",
                    "name": "coder",
                    "isActive": false
                }
            ]
        }"##;
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "my-team");
        assert_eq!(team.members.len(), 2);
        assert!(team.members[0].is_active);
        assert!(!team.members[1].is_active);
    }

    #[test]
    fn test_session_section_mapping() {
        assert_eq!(SessionStatus::Thinking.section(), SessionSection::Active);
        assert_eq!(SessionStatus::Running.section(), SessionSection::Active);
        assert_eq!(SessionStatus::Starting.section(), SessionSection::Active);
        assert_eq!(SessionStatus::Prompting.section(), SessionSection::Active);
        assert_eq!(SessionStatus::Waiting.section(), SessionSection::Active);
        assert_eq!(SessionStatus::Stashed.section(), SessionSection::Done);
        assert_eq!(SessionStatus::Errored.section(), SessionSection::Fail);
        assert_eq!(SessionStatus::Stashed.to_string(), "Stashed");
    }

    #[test]
    fn test_session_section_ordering() {
        assert!(SessionSection::Active < SessionSection::Done);
        assert!(SessionSection::Done < SessionSection::Fail);
    }

    /// Truth-table coverage for `Session::adoption_options` — the central
    /// business rule of the wild-session-adoption feature. Representative cases
    /// across (status × source); the rule itself collapses many combinations.
    #[test]
    fn test_adoption_options_truth_table() {
        // (label, source, status, expected view_only, expected takeover, expected has_reason)
        let cases: &[(&str, SessionSource, SessionStatus, bool, bool, bool)] = &[
            // Daemon: never adoptable, regardless of status.
            (
                "daemon+running",
                SessionSource::Daemon,
                SessionStatus::Running,
                false,
                false,
                true,
            ),
            (
                "daemon+stashed",
                SessionSource::Daemon,
                SessionStatus::Stashed,
                false,
                false,
                true,
            ),
            // Unknown: nothing to do.
            (
                "unknown+running",
                SessionSource::Unknown,
                SessionStatus::Running,
                false,
                false,
                true,
            ),
            // Wild + active statuses → both options.
            (
                "wild+running",
                SessionSource::Wild,
                SessionStatus::Running,
                true,
                true,
                false,
            ),
            (
                "wild+thinking",
                SessionSource::Wild,
                SessionStatus::Thinking,
                true,
                true,
                false,
            ),
            (
                "wild+starting",
                SessionSource::Wild,
                SessionStatus::Starting,
                true,
                true,
                false,
            ),
            (
                "wild+waiting",
                SessionSource::Wild,
                SessionStatus::Waiting,
                true,
                true,
                false,
            ),
            (
                "wild+prompting",
                SessionSource::Wild,
                SessionStatus::Prompting,
                true,
                true,
                false,
            ),
            // Wild + dead statuses → no options.
            (
                "wild+stashed",
                SessionSource::Wild,
                SessionStatus::Stashed,
                false,
                false,
                true,
            ),
            (
                "wild+errored",
                SessionSource::Wild,
                SessionStatus::Errored,
                false,
                false,
                true,
            ),
            (
                "wild+done",
                SessionSource::Wild,
                SessionStatus::Done,
                false,
                false,
                true,
            ),
            // External behaves like Wild for adoption purposes.
            (
                "external+running",
                SessionSource::External,
                SessionStatus::Running,
                true,
                true,
                false,
            ),
            (
                "external+stashed",
                SessionSource::External,
                SessionStatus::Stashed,
                false,
                false,
                true,
            ),
        ];

        for (label, source, status, want_view, want_takeover, want_reason) in cases {
            let s = Session {
                source: *source,
                status: *status,
                ..Default::default()
            };
            let opts = s.adoption_options();
            assert_eq!(opts.view_only, *want_view, "{label}: view_only mismatch");
            assert_eq!(opts.takeover, *want_takeover, "{label}: takeover mismatch");
            assert_eq!(
                opts.reason_disabled.is_some(),
                *want_reason,
                "{label}: reason_disabled presence mismatch"
            );
        }
    }

    #[test]
    fn test_adoption_options_default_session_unknown_source() {
        // Default Session has source = Unknown; ensures the default doesn't
        // silently look adoptable.
        let s = Session::default();
        assert_eq!(s.source, SessionSource::Unknown);
        let opts = s.adoption_options();
        assert!(!opts.view_only);
        assert!(!opts.takeover);
    }

    #[test]
    fn test_session_source_default_is_unknown() {
        // The Default impl must not return Daemon — that would silently mark
        // every session daemon-managed in tests / fixtures.
        assert_eq!(SessionSource::default(), SessionSource::Unknown);
    }
}
