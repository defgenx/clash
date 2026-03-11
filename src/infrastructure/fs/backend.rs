//! Filesystem implementation of the DataRepository port.

use std::path::{Path, PathBuf};

use crate::domain::entities::{Session, Subagent, Task, Team};
use crate::domain::ports::DataRepository;
use crate::infrastructure::error::Result;
use crate::infrastructure::fs::atomic::write_atomic;

/// Production filesystem-based data repository.
pub struct FsBackend {
    base_dir: PathBuf,
}

impl FsBackend {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.base_dir.join("projects")
    }

    fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
        let content = std::fs::read_to_string(path)?;
        let value = serde_json::from_str(&content)?;
        Ok(value)
    }

    /// Build a Session from metadata + filesystem checks.
    #[allow(clippy::too_many_arguments)]
    fn build_session(
        jsonl_path: &Path,
        session_id: &str,
        project_dir_name: &str,
        project_path_str: &str,
        summary: &str,
        first_prompt: &str,
        message_count: usize,
        git_branch: &str,
        fallback_modified: &str,
        project_dir: &Path,
        now: std::time::SystemTime,
    ) -> Session {
        // Detect status from JSONL tail content + file age
        let file_meta = jsonl_path.metadata().ok();
        let file_mtime = file_meta.as_ref().and_then(|m| m.modified().ok());

        let age_secs = file_mtime
            .and_then(|mtime| now.duration_since(mtime).ok())
            .map(|age| age.as_secs());

        let status = Self::detect_session_status(jsonl_path, age_secs);
        let is_running = !matches!(status, crate::domain::entities::SessionStatus::Idle);

        // Format last_modified from actual file mtime for accuracy
        let last_modified = file_mtime
            .map(|mtime| {
                let dt: chrono::DateTime<chrono::Local> = mtime.into();
                dt.format("%Y-%m-%d %H:%M").to_string()
            })
            .unwrap_or_else(|| fallback_modified.to_string());

        // Check for subagents
        let subagents_dir = project_dir.join(session_id).join("subagents");
        let has_subagents = subagents_dir.is_dir();
        let subagent_count = if has_subagents {
            std::fs::read_dir(&subagents_dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| {
                            e.path()
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map(|n| n.ends_with(".jsonl"))
                                .unwrap_or(false)
                        })
                        .count()
                })
                .unwrap_or(0)
        } else {
            0
        };

        // Use summary if available, fall back to first_prompt truncated
        let display_summary = if !summary.is_empty() {
            summary.to_string()
        } else if !first_prompt.is_empty() {
            let clean: String = first_prompt
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if clean.len() > 60 {
                format!("{}...", &clean[..57])
            } else {
                clean
            }
        } else {
            String::new()
        };

        // Resolve git branch: use provided value, or detect from project path
        let resolved_branch = if !git_branch.is_empty() {
            git_branch.to_string()
        } else {
            Self::detect_git_branch(project_path_str)
        };

        Session {
            id: session_id.to_string(),
            project: project_dir_name.to_string(),
            project_path: project_path_str.to_string(),
            last_modified,
            summary: display_summary,
            first_prompt: first_prompt.to_string(),
            has_subagents,
            subagent_count,
            message_count,
            git_branch: resolved_branch,
            is_running,
            status,
        }
    }

    /// Detect the current git branch from a project path by reading .git/HEAD.
    fn detect_git_branch(project_path: &str) -> String {
        if project_path.is_empty() {
            return String::new();
        }
        let git_head = std::path::Path::new(project_path).join(".git/HEAD");
        if let Ok(content) = std::fs::read_to_string(&git_head) {
            let content = content.trim();
            // "ref: refs/heads/my-branch" → "my-branch"
            if let Some(branch) = content.strip_prefix("ref: refs/heads/") {
                return branch.to_string();
            }
            // Detached HEAD — return short hash
            if content.len() >= 8 {
                return content[..8].to_string();
            }
        }
        String::new()
    }

    /// Extract the first user message from a .jsonl session file as a summary.
    fn extract_session_summary(path: &Path) -> String {
        use std::io::BufRead;

        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return String::new(),
        };

        let reader = std::io::BufReader::new(file);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            let parsed: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if parsed.get("type").and_then(|t| t.as_str()) == Some("user") {
                // Try message.content as string, or message.content[0].text
                if let Some(msg) = parsed.get("message") {
                    if let Some(content) = msg.get("content") {
                        let text = if let Some(s) = content.as_str() {
                            s.to_string()
                        } else if let Some(arr) = content.as_array() {
                            arr.iter()
                                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                                .next()
                                .unwrap_or("")
                                .to_string()
                        } else {
                            String::new()
                        };

                        if !text.is_empty() {
                            // Truncate to ~60 chars, collapse whitespace
                            let clean: String =
                                text.split_whitespace().collect::<Vec<_>>().join(" ");
                            if clean.len() > 60 {
                                return format!("{}...", &clean[..57]);
                            }
                            return clean;
                        }
                    }
                }
            }
        }

        String::new()
    }
}

impl DataRepository for FsBackend {
    fn load_teams(&self) -> Result<Vec<Team>> {
        let teams_dir = self.teams_dir();
        if !teams_dir.exists() {
            return Ok(Vec::new());
        }

        let mut teams = Vec::new();
        let entries = std::fs::read_dir(&teams_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let config_path = path.join("config.json");
            if !config_path.exists() {
                continue;
            }

            match Self::read_json_file::<Team>(&config_path) {
                Ok(mut team) => {
                    if team.name.is_empty() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            team.name = name.to_string();
                        }
                    }
                    teams.push(team);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse team config at {:?}: {}", config_path, e);
                    teams.push(Team {
                        name: path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        description: format!("Parse error: {}", e),
                        ..Default::default()
                    });
                }
            }
        }

        teams.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(teams)
    }

    fn load_tasks(&self, team: &str) -> Result<Vec<Task>> {
        let tasks_dir = self.tasks_dir().join(team);
        if !tasks_dir.exists() {
            return Ok(Vec::new());
        }

        let mut tasks = Vec::new();
        let entries = std::fs::read_dir(&tasks_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            match Self::read_json_file::<Task>(&path) {
                Ok(task) => tasks.push(task),
                Err(e) => tracing::warn!("Failed to parse task at {:?}: {}", path, e),
            }
        }

        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }

    fn write_task(&self, team: &str, task: &Task) -> Result<()> {
        let path = self
            .tasks_dir()
            .join(team)
            .join(format!("{}.json", task.id));
        let data = serde_json::to_vec_pretty(task)?;
        write_atomic(&path, &data)
    }

    fn delete_team(&self, name: &str) -> Result<()> {
        let team_dir = self.teams_dir().join(name);
        if team_dir.exists() {
            std::fs::remove_dir_all(&team_dir)?;
        }
        let tasks_dir = self.tasks_dir().join(name);
        if tasks_dir.exists() {
            std::fs::remove_dir_all(&tasks_dir)?;
        }
        Ok(())
    }

    fn teams_dir(&self) -> PathBuf {
        self.base_dir.join("teams")
    }

    fn tasks_dir(&self) -> PathBuf {
        self.base_dir.join("tasks")
    }

    fn load_sessions(&self) -> Result<Vec<Session>> {
        let projects_dir = self.base_dir.join("projects");
        if !projects_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        let now = std::time::SystemTime::now();

        let project_entries = std::fs::read_dir(&projects_dir)?;
        for project_entry in project_entries {
            let project_entry = project_entry?;
            let project_path = project_entry.path();
            if !project_path.is_dir() {
                continue;
            }

            let project_dir_name = match project_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };

            let decoded_project_path = format!(
                "/{}",
                project_dir_name.trim_start_matches('-').replace('-', "/")
            );

            // Track which session IDs we've already found from the index
            let mut indexed_ids = std::collections::HashSet::new();

            // Try sessions-index.json first (has pre-computed summaries)
            let index_path = project_path.join("sessions-index.json");
            if index_path.exists() {
                if let Ok(index_content) = std::fs::read_to_string(&index_path) {
                    if let Ok(index) = serde_json::from_str::<serde_json::Value>(&index_content) {
                        if let Some(entries) = index.get("entries").and_then(|e| e.as_array()) {
                            for entry in entries {
                                let session_id = entry
                                    .get("sessionId")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if session_id.is_empty() {
                                    continue;
                                }

                                indexed_ids.insert(session_id.clone());

                                let summary = entry
                                    .get("summary")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let first_prompt = entry
                                    .get("firstPrompt")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let project_path_str = entry
                                    .get("projectPath")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let message_count = entry
                                    .get("messageCount")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0)
                                    as usize;
                                let git_branch = entry
                                    .get("gitBranch")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let modified = entry
                                    .get("modified")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let jsonl_path = project_path.join(format!("{}.jsonl", session_id));
                                let session = Self::build_session(
                                    &jsonl_path,
                                    &session_id,
                                    &project_dir_name,
                                    if project_path_str.is_empty() {
                                        &decoded_project_path
                                    } else {
                                        &project_path_str
                                    },
                                    &summary,
                                    &first_prompt,
                                    message_count,
                                    &git_branch,
                                    &modified,
                                    &project_path,
                                    now,
                                );
                                sessions.push(session);
                            }
                        }
                    }
                }
            }

            // Scan for .jsonl files not covered by the index
            if let Ok(dir_entries) = std::fs::read_dir(&project_path) {
                for entry in dir_entries {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let path = entry.path();
                    let fname = match path.file_name().and_then(|n| n.to_str()) {
                        Some(n) if n.ends_with(".jsonl") => n.to_string(),
                        _ => continue,
                    };

                    let session_id = fname.trim_end_matches(".jsonl").to_string();
                    if indexed_ids.contains(&session_id) {
                        continue;
                    }

                    // Extract summary from the .jsonl file (first user message)
                    let summary = Self::extract_session_summary(&path);
                    let session = Self::build_session(
                        &path,
                        &session_id,
                        &project_dir_name,
                        &decoded_project_path,
                        &summary,
                        "",
                        0,
                        "",
                        "",
                        &project_path,
                        now,
                    );
                    sessions.push(session);
                }
            }
        }

        // Sort: waiting first, then running, then idle, then by last_modified descending
        use crate::domain::entities::SessionStatus;
        sessions.sort_by(|a, b| {
            let status_ord = |s: &SessionStatus| match s {
                SessionStatus::Prompting => 0,
                SessionStatus::Waiting => 1,
                SessionStatus::Thinking => 2,
                SessionStatus::Running => 3,
                SessionStatus::Starting => 4,
                SessionStatus::Idle => 5,
            };
            status_ord(&a.status)
                .cmp(&status_ord(&b.status))
                .then(b.last_modified.cmp(&a.last_modified))
        });
        Ok(sessions)
    }

    fn load_subagents(&self, project: &str, session_id: &str) -> Result<Vec<Subagent>> {
        let subagents_dir = self
            .base_dir
            .join("projects")
            .join(project)
            .join(session_id)
            .join("subagents");

        if !subagents_dir.exists() {
            return Ok(Vec::new());
        }

        let mut subagents = Vec::new();
        let entries = std::fs::read_dir(&subagents_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Only process .jsonl files (skip .meta.json)
            let fname = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) if n.ends_with(".jsonl") => n.to_string(),
                _ => continue,
            };

            let agent_id = fname.trim_end_matches(".jsonl").to_string();

            // Read agent type from .meta.json if it exists
            let meta_path = subagents_dir.join(format!("{}.meta.json", agent_id));
            let agent_type = if meta_path.exists() {
                std::fs::read_to_string(&meta_path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| {
                        v.get("agentType")
                            .and_then(|t| t.as_str())
                            .map(String::from)
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let file_meta = path.metadata().ok();
            let file_mtime = file_meta.as_ref().and_then(|m| m.modified().ok());
            let now = std::time::SystemTime::now();

            let last_modified = file_mtime
                .map(|mtime| {
                    let dt: chrono::DateTime<chrono::Local> = mtime.into();
                    dt.format("%Y-%m-%d %H:%M").to_string()
                })
                .unwrap_or_else(|| "unknown".to_string());

            let age_secs = file_mtime
                .and_then(|mtime| now.duration_since(mtime).ok())
                .map(|age| age.as_secs());

            let status = Self::detect_session_status(&path, age_secs);
            let is_running = !matches!(status, crate::domain::entities::SessionStatus::Idle);

            let summary = Self::extract_session_summary(&path);

            // Decode project name to path
            let decoded_path = format!("/{}", project.trim_start_matches('-').replace('-', "/"));

            subagents.push(Subagent {
                id: agent_id,
                agent_type,
                parent_session_id: session_id.to_string(),
                project: project.to_string(),
                last_modified,
                summary,
                file_path: decoded_path,
                is_running,
                status,
            });
        }

        subagents.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
        Ok(subagents)
    }

    fn load_conversation(
        &self,
        project: &str,
        session_id: &str,
    ) -> Result<Vec<crate::domain::entities::ConversationMessage>> {
        let path = self
            .base_dir
            .join("projects")
            .join(project)
            .join(format!("{}.jsonl", session_id));
        Self::parse_conversation(&path)
    }

    fn load_subagent_conversation(
        &self,
        project: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<crate::domain::entities::ConversationMessage>> {
        let path = self
            .base_dir
            .join("projects")
            .join(project)
            .join(session_id)
            .join("subagents")
            .join(format!("{}.jsonl", agent_id));
        Self::parse_conversation(&path)
    }

    fn delete_session(&self, project: &str, session_id: &str) -> Result<()> {
        let jsonl_path = self
            .base_dir
            .join("projects")
            .join(project)
            .join(format!("{}.jsonl", session_id));
        if jsonl_path.exists() {
            std::fs::remove_file(&jsonl_path)?;
        }
        // Also remove the session directory (subagents, etc.)
        let session_dir = self
            .base_dir
            .join("projects")
            .join(project)
            .join(session_id);
        if session_dir.is_dir() {
            std::fs::remove_dir_all(&session_dir)?;
        }
        // Remove entry from sessions-index.json so it doesn't reappear
        let index_path = self
            .base_dir
            .join("projects")
            .join(project)
            .join("sessions-index.json");
        if index_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&index_path) {
                if let Ok(mut index) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(entries) = index.get_mut("entries").and_then(|e| e.as_array_mut()) {
                        let before = entries.len();
                        entries.retain(|e| {
                            e.get("sessionId")
                                .and_then(|v| v.as_str())
                                .map(|id| id != session_id)
                                .unwrap_or(true)
                        });
                        if entries.len() != before {
                            if let Ok(updated) = serde_json::to_string_pretty(&index) {
                                let _ = crate::infrastructure::fs::atomic::write_atomic(
                                    &index_path,
                                    updated.as_bytes(),
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn delete_all_sessions(&self) -> Result<()> {
        let sessions = self.load_sessions()?;
        for session in &sessions {
            if let Err(e) = self.delete_session(&session.project, &session.id) {
                tracing::warn!("Failed to delete session {}: {}", session.id, e);
            }
        }
        // Also remove sessions-index.json files so ghost entries don't reappear
        let projects_dir = self.base_dir.join("projects");
        if projects_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&projects_dir) {
                for entry in entries.flatten() {
                    let index_path = entry.path().join("sessions-index.json");
                    if index_path.exists() {
                        let _ = std::fs::remove_file(&index_path);
                    }
                }
            }
        }
        Ok(())
    }
}

impl FsBackend {
    /// Detect session status by reading the tail of the JSONL file.
    ///
    /// Key patterns from real Claude Code JSONL data:
    /// - `last-prompt` as last entry → WAITING for user input
    /// - `assistant stop_reason=end_turn` + `system` entries (no `last-prompt`) → session exited, Idle
    /// - `assistant stop_reason=None` or `progress` at end + recent file → RUNNING
    /// - Old file with none of the above → IDLE
    fn detect_session_status(
        jsonl_path: &Path,
        age_secs: Option<u64>,
    ) -> crate::domain::entities::SessionStatus {
        use crate::domain::entities::SessionStatus;
        use std::io::{Read, Seek, SeekFrom};

        let mut file = match std::fs::File::open(jsonl_path) {
            Ok(f) => f,
            Err(_) => return SessionStatus::Idle,
        };

        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let seek_pos = file_len.saturating_sub(16384);
        let _ = file.seek(SeekFrom::Start(seek_pos));

        let mut tail = String::new();
        if file.read_to_string(&mut tail).is_err() {
            return SessionStatus::Idle;
        }

        // Track the last meaningful entry's type and metadata
        let mut last_type = "";
        let mut last_subtype = "";
        let mut last_assistant_stop_reason = "";
        let mut has_end_turn_in_current_turn = false;

        for line in tail.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let parsed: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let msg_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if msg_type.is_empty() {
                continue;
            }

            last_type = match msg_type {
                "last-prompt" => "last-prompt",
                "assistant" => "assistant",
                "user" => "user",
                "system" => "system",
                "progress" => "progress",
                "result" => "result",
                "queue-operation" => "queue-operation",
                "file-history-snapshot" => "file-history-snapshot",
                _ => "other",
            };

            last_subtype = match parsed.get("subtype").and_then(|s| s.as_str()) {
                Some("stop_hook_summary") => "stop_hook_summary",
                Some("turn_duration") => "turn_duration",
                Some("api_error") => "api_error",
                Some("success") => "success",
                _ => "",
            };

            if msg_type == "assistant" {
                let sr = parsed
                    .get("message")
                    .and_then(|m| m.get("stop_reason"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                last_assistant_stop_reason = match sr {
                    "end_turn" => {
                        has_end_turn_in_current_turn = true;
                        "end_turn"
                    }
                    "tool_use" => "tool_use",
                    _ => "",
                };
            } else if msg_type == "user" {
                // User message means new turn — Claude is processing
                has_end_turn_in_current_turn = false;
                last_assistant_stop_reason = "";
            }
        }

        // 1. last-prompt at end → definitively WAITING for user input
        if last_type == "last-prompt" {
            return SessionStatus::Waiting;
        }

        // 2. result with subtype=success → turn completed, WAITING
        if last_type == "result" && last_subtype == "success" {
            return SessionStatus::Waiting;
        }

        // 3. Turn completed (end_turn seen) + system bookkeeping after → session exited
        if has_end_turn_in_current_turn
            && (last_type == "system" || last_type == "file-history-snapshot")
        {
            if age_secs.map(|a| a < 10).unwrap_or(false) {
                return SessionStatus::Running;
            }
            return SessionStatus::Idle;
        }

        // 4. assistant with end_turn as the very last entry
        if last_type == "assistant" && last_assistant_stop_reason == "end_turn" {
            if age_secs.map(|a| a < 10).unwrap_or(false) {
                return SessionStatus::Waiting;
            }
            return SessionStatus::Idle;
        }

        // 5. File is recent → still running
        if age_secs.map(|a| a < 30).unwrap_or(false) {
            return SessionStatus::Running;
        }

        // 6. Old file, no clear waiting signal → idle
        SessionStatus::Idle
    }

    /// Parse conversation messages from a .jsonl file.
    fn parse_conversation(
        path: &Path,
    ) -> Result<Vec<crate::domain::entities::ConversationMessage>> {
        use std::io::BufRead;

        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let mut messages = Vec::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }
            let parsed: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let msg_type = match parsed.get("type").and_then(|t| t.as_str()) {
                Some(t) if t == "user" || t == "assistant" => t.to_string(),
                _ => continue,
            };

            if let Some(msg) = parsed.get("message") {
                if let Some(content) = msg.get("content") {
                    let text = if let Some(s) = content.as_str() {
                        s.to_string()
                    } else if let Some(arr) = content.as_array() {
                        arr.iter()
                            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        continue;
                    };

                    if !text.is_empty() {
                        messages.push(crate::domain::entities::ConversationMessage {
                            role: msg_type,
                            text,
                        });
                    }
                }
            }
        }

        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_dir() -> (TempDir, FsBackend) {
        let dir = TempDir::new().unwrap();
        let backend = FsBackend::new(dir.path().to_path_buf());
        (dir, backend)
    }

    #[test]
    fn test_load_teams_empty_dir() {
        let (_dir, backend) = setup_test_dir();
        let teams = backend.load_teams().unwrap();
        assert!(teams.is_empty());
    }

    #[test]
    fn test_load_teams_with_config() {
        let (dir, backend) = setup_test_dir();
        let team_dir = dir.path().join("teams").join("my-team");
        std::fs::create_dir_all(&team_dir).unwrap();
        std::fs::write(
            team_dir.join("config.json"),
            r#"{"name": "my-team", "description": "Test team", "members": []}"#,
        )
        .unwrap();

        let teams = backend.load_teams().unwrap();
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].name, "my-team");
    }

    #[test]
    fn test_load_teams_malformed_json() {
        let (dir, backend) = setup_test_dir();
        let team_dir = dir.path().join("teams").join("bad-team");
        std::fs::create_dir_all(&team_dir).unwrap();
        std::fs::write(team_dir.join("config.json"), "not valid json").unwrap();

        let teams = backend.load_teams().unwrap();
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0].name, "bad-team");
        assert!(teams[0].description.contains("Parse error"));
    }

    #[test]
    fn test_write_and_load_task() {
        let (dir, backend) = setup_test_dir();
        let tasks_dir = dir.path().join("tasks").join("my-team");
        std::fs::create_dir_all(&tasks_dir).unwrap();

        let task = Task {
            id: "task-1".to_string(),
            subject: "Test task".to_string(),
            ..Default::default()
        };

        backend.write_task("my-team", &task).unwrap();
        let tasks = backend.load_tasks("my-team").unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "task-1");
    }

    #[test]
    fn test_load_tasks_empty() {
        let (_dir, backend) = setup_test_dir();
        let tasks = backend.load_tasks("nonexistent").unwrap();
        assert!(tasks.is_empty());
    }
}
