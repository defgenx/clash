//! Clash session registry — tracks which sessions are managed by clash.
//!
//! Sessions created via clash's `c`/`n` commands are registered here.
//! Only registered sessions appear in the UI. The registry is stored as
//! `sessions.json` in the clash data directory.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::clash_data_dir;

const REGISTRY_FILE: &str = "sessions.json";

/// A clash-managed session entry in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClashSession {
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    /// The Claude Code session ID currently linked to this entry.
    /// Updated on `/clear` when Claude creates a new session.
    pub claude_session_id: String,
    #[serde(default)]
    pub created_at: String,
    /// The original branch a worktree session was created from.
    #[serde(default)]
    pub source_branch: Option<String>,
}

/// Path to the session registry file.
fn registry_path() -> std::path::PathBuf {
    clash_data_dir().join(REGISTRY_FILE)
}

/// Load the session registry from disk.
pub fn load() -> HashMap<String, ClashSession> {
    let path = registry_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Save the session registry to disk (atomic write).
fn save(registry: &HashMap<String, ClashSession>) {
    let path = registry_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
    if let Ok(json) = serde_json::to_string_pretty(registry) {
        let _ = crate::infrastructure::fs::atomic::write_atomic(&path, json.as_bytes());
    }
}

/// Register a new session in the registry.
pub fn register(session_id: &str, name: &str, cwd: &str, source_branch: Option<&str>) {
    let mut registry = load();
    registry.insert(
        session_id.to_string(),
        ClashSession {
            session_id: session_id.to_string(),
            name: name.to_string(),
            cwd: cwd.to_string(),
            claude_session_id: session_id.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source_branch: source_branch.map(|s| s.to_string()),
        },
    );
    save(&registry);
}

/// Remove a session from the registry.
pub fn unregister(session_id: &str) {
    let mut registry = load();
    // Remove by session_id key OR by claude_session_id value
    // (in case /clear updated the claude_session_id)
    registry.retain(|k, v| k != session_id && v.claude_session_id != session_id);
    save(&registry);
}

/// Rename a session in the registry.
pub fn rename(session_id: &str, new_name: &str) {
    let mut registry = load();
    // Look up by key or by claude_session_id
    let key = registry
        .iter()
        .find(|(k, v)| *k == session_id || v.claude_session_id == session_id)
        .map(|(k, _)| k.clone());
    if let Some(key) = key {
        if let Some(entry) = registry.get_mut(&key) {
            entry.name = new_name.to_string();
        }
        save(&registry);
    }
}

/// Remove all sessions from the registry.
pub fn clear() {
    save(&HashMap::new());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_round_trip() {
        let mut reg = HashMap::new();
        reg.insert(
            "test-1".to_string(),
            ClashSession {
                session_id: "test-1".to_string(),
                name: "my-session".to_string(),
                cwd: "/tmp/project".to_string(),
                claude_session_id: "test-1".to_string(),
                created_at: String::new(),
                source_branch: None,
            },
        );

        let json = serde_json::to_string(&reg).unwrap();
        let loaded: HashMap<String, ClashSession> = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded["test-1"].name, "my-session");
        assert_eq!(loaded["test-1"].cwd, "/tmp/project");
    }

    #[test]
    fn test_registry_backward_compat_no_source_branch() {
        // Old JSON without source_branch field should deserialize fine
        let json = r#"{"test-2":{"session_id":"test-2","name":"old","cwd":"/tmp","claude_session_id":"test-2","created_at":""}}"#;
        let loaded: HashMap<String, ClashSession> = serde_json::from_str(json).unwrap();
        assert_eq!(loaded["test-2"].name, "old");
        assert!(loaded["test-2"].source_branch.is_none());
    }

    #[test]
    fn test_registry_round_trip_with_source_branch() {
        let mut reg = HashMap::new();
        reg.insert(
            "test-3".to_string(),
            ClashSession {
                session_id: "test-3".to_string(),
                name: "wt-session".to_string(),
                cwd: "/tmp/worktree".to_string(),
                claude_session_id: "test-3".to_string(),
                created_at: String::new(),
                source_branch: Some("main".to_string()),
            },
        );

        let json = serde_json::to_string(&reg).unwrap();
        let loaded: HashMap<String, ClashSession> = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded["test-3"].source_branch.as_deref(), Some("main"));
    }
}
