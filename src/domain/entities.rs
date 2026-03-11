//! Domain entities — the core data model.
//!
//! These types represent the business domain of Claude Code Agent Teams.
//! They use `#[serde(default)]` and `#[serde(flatten)]` for resilience
//! against schema changes in Claude Code's JSON files.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A team member / agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Granular session status — detected by parsing the terminal screen content.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    pub message_count: usize,
    #[serde(default)]
    pub git_branch: String,
    /// Whether this session is currently active (file modified recently).
    #[serde(default)]
    pub is_running: bool,
    /// Granular session status.
    #[serde(default)]
    pub status: SessionStatus,
}

/// A subagent spawned by a session or another agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// A conversation message from a session or subagent .jsonl file.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub role: String, // "user" or "assistant"
    pub text: String,
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
}
