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
    Done = 1,   // Idle
    Fail = 2,   // Errored
}

impl SessionSection {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Done => "Done",
            Self::Fail => "Fail",
        }
    }
}

/// Granular session status — detected by parsing the terminal screen content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// Session process is dead or exited.
    #[default]
    Idle,
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
}

impl SessionStatus {
    /// Map this status to its high-level session section.
    pub fn section(&self) -> SessionSection {
        match self {
            Self::Thinking | Self::Running | Self::Starting | Self::Prompting | Self::Waiting => {
                SessionSection::Active
            }
            Self::Idle => SessionSection::Done,
            Self::Errored => SessionSection::Fail,
        }
    }
}

impl std::str::FromStr for SessionStatus {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "idle" => Ok(SessionStatus::Idle),
            "starting" => Ok(SessionStatus::Starting),
            "running" => Ok(SessionStatus::Running),
            "thinking" => Ok(SessionStatus::Thinking),
            "waiting" => Ok(SessionStatus::Waiting),
            "prompting" => Ok(SessionStatus::Prompting),
            "errored" => Ok(SessionStatus::Errored),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Idle => write!(f, "IDLE"),
            SessionStatus::Starting => write!(f, "STARTING"),
            SessionStatus::Running => write!(f, "RUNNING"),
            SessionStatus::Thinking => write!(f, "THINKING"),
            SessionStatus::Waiting => write!(f, "WAITING"),
            SessionStatus::Prompting => write!(f, "PROMPTING"),
            SessionStatus::Errored => write!(f, "ERRORED"),
        }
    }
}

/// A Claude Code session (from ~/.claude/projects/*/sessions-index.json).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize)]
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
    /// Consumed by Superset; retained for forward-compat deserialization.
    #[serde(default)]
    #[allow(dead_code)]
    pub run: Vec<String>,
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
        assert_eq!(SessionStatus::Idle.section(), SessionSection::Done);
        assert_eq!(SessionStatus::Errored.section(), SessionSection::Fail);
    }

    #[test]
    fn test_session_section_ordering() {
        assert!(SessionSection::Active < SessionSection::Done);
        assert!(SessionSection::Done < SessionSection::Fail);
    }
}
