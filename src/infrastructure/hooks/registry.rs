//! Clash session registry — tracks which sessions are managed by clash.
//!
//! Sessions created via clash's `c`/`n` commands are registered here.
//! Only registered sessions appear in the UI. The registry is stored as
//! `sessions.json` in the clash data directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::clash_data_dir;

const REGISTRY_FILE: &str = "sessions.json";

/// Cached registry that avoids re-reading `sessions.json` on every refresh cycle.
/// Invalidated by FS-watcher when the file changes.
#[derive(Default)]
pub struct RegistryCache {
    cached: Option<HashMap<String, ClashSession>>,
}

impl RegistryCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached registry, loading from disk on first call or after invalidation.
    pub fn get(&mut self) -> HashMap<String, ClashSession> {
        if let Some(ref cached) = self.cached {
            return cached.clone();
        }
        let registry = load();
        self.cached = Some(registry.clone());
        registry
    }

    /// Clear the cache so the next `get()` re-reads from disk.
    pub fn invalidate(&mut self) {
        self.cached = None;
    }

    /// Path to the registry file, for adding to FS watcher.
    pub fn watched_path() -> PathBuf {
        clash_data_dir()
    }
}

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
    /// Prior Claude session IDs this entry was keyed by before a `/clear`
    /// re-keyed it (oldest first). The hook appends the old key here so a
    /// stale id persisted elsewhere (e.g. a GUI workspace pane) can be
    /// resolved forward to the current `claude_session_id` — see
    /// [`resolve_resume_id`]. Kept out of display lookups (`find_entry`) so
    /// the pre-`/clear` transcript stays hidden.
    #[serde(default)]
    pub previous_ids: Vec<String>,
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
            previous_ids: Vec::new(),
        },
    );
    save(&registry);
}

/// Find a registry entry matching the given session ID (by registry key or claude_session_id).
pub fn find_entry<'a>(
    registry: &'a HashMap<String, ClashSession>,
    session_id: &str,
) -> Option<(&'a String, &'a ClashSession)> {
    registry.get_key_value(session_id).or_else(|| {
        registry
            .iter()
            .find(|(_, v)| v.claude_session_id == session_id)
    })
}

/// Resolve a possibly-stale session ID to the Claude session ID that should
/// actually be resumed. After a `/clear`, the hook re-keys the entry to the
/// new conversation id and records the old id in `previous_ids`; a caller
/// holding the old id (e.g. a persisted GUI workspace pane) would otherwise
/// resume a stale pre-`/clear` transcript. Resolution order: a direct
/// `find_entry` match (current key / claude_session_id), then a `previous_ids`
/// lineage match. Returns `None` when the id is unknown to the registry.
pub fn resolve_resume_id(
    registry: &HashMap<String, ClashSession>,
    session_id: &str,
) -> Option<String> {
    if let Some((_, entry)) = find_entry(registry, session_id) {
        if !entry.claude_session_id.is_empty() {
            return Some(entry.claude_session_id.clone());
        }
    }
    registry
        .values()
        .find(|v| v.previous_ids.iter().any(|p| p == session_id))
        .map(|v| v.claude_session_id.clone())
        .filter(|s| !s.is_empty())
}

/// Maximum resume-fork hops to chase (defensive bound; real chains are short —
/// one hop per app restart/reload).
const MAX_FORK_HOPS: usize = 32;
/// Per-transcript line cap while searching for the parent reference. The
/// first user turn (whose hook payload echoes the resumed id) sits near the
/// top of the file; full transcripts can be tens of MB.
const FORK_SCAN_LINES: usize = 5_000;

/// Chase resume forks on disk and return the id of the newest descendant
/// conversation.
///
/// `claude --resume <id>` does NOT continue writing to `<id>.jsonl`: it
/// forks the conversation into a NEW transcript (fresh session id) — while
/// hook payloads keep reporting the *resumed* id, so unlike `/clear` there
/// is no SessionStart event carrying the new id and the registry cannot be
/// re-keyed by the hook. The linkage lives inside the forked transcript:
/// its hook entries embed `"session_id":"<resumed-id>"`. This walks those
/// references (newest match wins when a conversation was resumed twice)
/// until no descendant exists, so a restart resumes where the user actually
/// left off instead of the first-ever quit point.
pub fn chase_resume_forks(projects_dir: &Path, cwd: &str, start: &str) -> String {
    use std::io::BufRead;

    let dir = projects_dir.join(crate::infrastructure::fs::backend::encode_project_dir(cwd));
    let mut current = start.to_string();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    visited.insert(current.clone());

    for _ in 0..MAX_FORK_HOPS {
        let needle = format!("\"session_id\":\"{}\"", current);
        let mut best: Option<(std::time::SystemTime, String)> = None;

        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => break,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            if visited.contains(&id) {
                continue;
            }
            let Ok(file) = std::fs::File::open(&path) else {
                continue;
            };
            let mut reader = std::io::BufReader::new(file);
            let mut line = String::new();
            let mut found = false;
            for i in 0..FORK_SCAN_LINES {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                // Subagent sidechain transcripts live in the same project
                // dir and may echo hook payloads — never resume those.
                if i == 0 && line.contains("\"isSidechain\":true") {
                    break;
                }
                if line.contains(&needle) {
                    found = true;
                    break;
                }
            }
            if !found {
                continue;
            }
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let better = match &best {
                None => true,
                Some((t, _)) => mtime > *t,
            };
            if better {
                best = Some((mtime, id));
            }
        }

        match best {
            Some((_, next)) => {
                visited.insert(next.clone());
                current = next;
            }
            None => break,
        }
    }
    current
}

/// Registry resolution + on-disk fork chase in one step: the id to actually
/// pass to `claude --resume` for a session. Every resume site should use
/// this (and then [`record_resumed_conversation`]) instead of the bare
/// [`resolve_resume_id`].
pub fn resolve_latest_conversation(
    registry: &HashMap<String, ClashSession>,
    projects_dir: &Path,
    cwd: &str,
    session_id: &str,
) -> String {
    let rid = resolve_resume_id(registry, session_id).unwrap_or_else(|| session_id.to_string());
    if cwd.is_empty() {
        return rid;
    }
    chase_resume_forks(projects_dir, cwd, &rid)
}

/// Point a session's `claude_session_id` at the conversation that was
/// actually resumed, recording the old id in the lineage (like the `/clear`
/// hook does). The registry key stays stable. No-op when nothing changed.
pub fn record_resumed_conversation(session_id: &str, resumed: &str) {
    if resumed.is_empty() {
        return;
    }
    let mut registry = load();
    let key = match find_entry(&registry, session_id) {
        Some((k, _)) => k.clone(),
        None => match registry
            .values()
            .find(|v| v.previous_ids.iter().any(|p| p == session_id))
        {
            Some(v) => v.session_id.clone(),
            None => return,
        },
    };
    let Some(entry) = registry.get_mut(&key) else {
        return;
    };
    if entry.claude_session_id == resumed {
        return;
    }
    let old = std::mem::replace(&mut entry.claude_session_id, resumed.to_string());
    if !old.is_empty() && old != resumed && !entry.previous_ids.contains(&old) {
        entry.previous_ids.push(old);
    }
    save(&registry);
}

/// Startup pass: chase every registry entry's conversation forward through
/// resume forks and re-key the stale ones, so the session list, persisted
/// pane ids, and the first resume all agree on the *current* conversation.
/// Cheap — a handful of transcript-head scans per stale entry.
pub fn heal_registry_forks(projects_dir: &Path) {
    let mut registry = load();
    let mut changed = false;
    for entry in registry.values_mut() {
        if entry.cwd.is_empty() || entry.claude_session_id.is_empty() {
            continue;
        }
        let latest = chase_resume_forks(projects_dir, &entry.cwd, &entry.claude_session_id);
        if latest != entry.claude_session_id {
            let old = std::mem::replace(&mut entry.claude_session_id, latest);
            if !entry.previous_ids.contains(&old) {
                entry.previous_ids.push(old);
            }
            changed = true;
        }
    }
    if changed {
        save(&registry);
    }
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
    let key = find_entry(&registry, session_id).map(|(k, _)| k.clone());
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
                previous_ids: Vec::new(),
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
                previous_ids: Vec::new(),
            },
        );

        let json = serde_json::to_string(&reg).unwrap();
        let loaded: HashMap<String, ClashSession> = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded["test-3"].source_branch.as_deref(), Some("main"));
    }

    fn make_session(session_id: &str, claude_session_id: &str) -> ClashSession {
        ClashSession {
            session_id: session_id.to_string(),
            name: "test".to_string(),
            cwd: "/tmp".to_string(),
            claude_session_id: claude_session_id.to_string(),
            created_at: String::new(),
            source_branch: None,
            previous_ids: Vec::new(),
        }
    }

    #[test]
    fn test_find_entry_by_key() {
        let mut reg = HashMap::new();
        reg.insert("sess-1".to_string(), make_session("sess-1", "sess-1"));
        let result = find_entry(&reg, "sess-1");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "sess-1");
    }

    #[test]
    fn test_find_entry_by_claude_session_id() {
        let mut reg = HashMap::new();
        // Key is the original session ID, but claude_session_id was updated (e.g. after /clear)
        reg.insert("orig-id".to_string(), make_session("orig-id", "new-id"));
        let result = find_entry(&reg, "new-id");
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "orig-id");
    }

    #[test]
    fn test_find_entry_not_found() {
        let mut reg = HashMap::new();
        reg.insert("sess-1".to_string(), make_session("sess-1", "sess-1"));
        assert!(find_entry(&reg, "unknown").is_none());
    }

    #[test]
    fn test_resolve_resume_id_identity_for_known() {
        let mut reg = HashMap::new();
        reg.insert("sess-1".to_string(), make_session("sess-1", "sess-1"));
        assert_eq!(resolve_resume_id(&reg, "sess-1").as_deref(), Some("sess-1"));
    }

    #[test]
    fn test_resolve_resume_id_follows_lineage_after_clear() {
        // After /clear the hook re-keyed orig-id → new-id and recorded the
        // old id in previous_ids. A stale persisted "orig-id" must resolve
        // forward to the current conversation "new-id".
        let mut reg = HashMap::new();
        let mut entry = make_session("new-id", "new-id");
        entry.previous_ids = vec!["orig-id".to_string()];
        reg.insert("new-id".to_string(), entry);
        assert_eq!(
            resolve_resume_id(&reg, "orig-id").as_deref(),
            Some("new-id")
        );
        // And the current id still resolves to itself.
        assert_eq!(resolve_resume_id(&reg, "new-id").as_deref(), Some("new-id"));
    }

    #[test]
    fn test_resolve_resume_id_unknown_is_none() {
        let reg = HashMap::new();
        assert!(resolve_resume_id(&reg, "nope").is_none());
    }

    // ── resume-fork chasing ─────────────────────────────────────────

    /// Build a fake `projects/<encoded-cwd>` dir. `parent = Some(id)` embeds
    /// the hook echo (`"session_id":"<id>"`) a resumed transcript carries.
    fn write_transcript(
        projects: &Path,
        cwd: &str,
        id: &str,
        parent: Option<&str>,
        sidechain: bool,
    ) {
        let dir = projects.join(crate::infrastructure::fs::backend::encode_project_dir(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let mut lines = Vec::new();
        if sidechain {
            lines.push(format!(
                r#"{{"isSidechain":true,"type":"user","sessionId":"{}"}}"#,
                id
            ));
        } else {
            lines.push(format!(r#"{{"type":"mode","sessionId":"{}"}}"#, id));
        }
        if let Some(p) = parent {
            lines.push(format!(
                r#"{{"type":"user","sessionId":"{}","hookInfos":[{{"session_id":"{}"}}]}}"#,
                id, p
            ));
        }
        lines.push(format!(r#"{{"type":"assistant","sessionId":"{}"}}"#, id));
        std::fs::write(dir.join(format!("{}.jsonl", id)), lines.join("\n")).unwrap();
        // Distinct mtimes so "newest match wins" is deterministic.
        std::thread::sleep(std::time::Duration::from_millis(15));
    }

    #[test]
    fn test_chase_resume_forks_follows_chain() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects = tmp.path();
        let cwd = "/tmp/proj";
        write_transcript(projects, cwd, "aaa", None, false);
        write_transcript(projects, cwd, "bbb", Some("aaa"), false);
        write_transcript(projects, cwd, "ccc", Some("bbb"), false);
        // A conversation in the same cwd that is NOT part of the lineage.
        write_transcript(projects, cwd, "other", None, false);

        assert_eq!(chase_resume_forks(projects, cwd, "aaa"), "ccc");
        assert_eq!(chase_resume_forks(projects, cwd, "bbb"), "ccc");
        assert_eq!(chase_resume_forks(projects, cwd, "ccc"), "ccc");
        assert_eq!(chase_resume_forks(projects, cwd, "other"), "other");
        // Unknown id / missing dir: identity.
        assert_eq!(chase_resume_forks(projects, cwd, "nope"), "nope");
        assert_eq!(chase_resume_forks(projects, "/elsewhere", "aaa"), "aaa");
    }

    #[test]
    fn test_chase_resume_forks_newest_of_two_children_wins() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects = tmp.path();
        let cwd = "/tmp/proj";
        write_transcript(projects, cwd, "aaa", None, false);
        write_transcript(projects, cwd, "child-early", Some("aaa"), false);
        write_transcript(projects, cwd, "child-late", Some("aaa"), false);
        assert_eq!(chase_resume_forks(projects, cwd, "aaa"), "child-late");
    }

    #[test]
    fn test_chase_resume_forks_skips_sidechains() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects = tmp.path();
        let cwd = "/tmp/proj";
        write_transcript(projects, cwd, "aaa", None, false);
        // A subagent sidechain echoing the parent id must never be resumed.
        write_transcript(projects, cwd, "side", Some("aaa"), true);
        assert_eq!(chase_resume_forks(projects, cwd, "aaa"), "aaa");
    }

    #[test]
    fn test_resolve_latest_conversation_composes_lineages() {
        let tmp = tempfile::TempDir::new().unwrap();
        let projects = tmp.path();
        let cwd = "/tmp/proj";
        // /clear lineage in the registry: orig → cleared…
        let mut reg = HashMap::new();
        let mut entry = make_session("cleared", "cleared");
        entry.previous_ids = vec!["orig".to_string()];
        reg.insert("cleared".to_string(), entry);
        // …then a resume fork on disk: cleared → forked.
        write_transcript(projects, cwd, "cleared", None, false);
        write_transcript(projects, cwd, "forked", Some("cleared"), false);

        assert_eq!(
            resolve_latest_conversation(&reg, projects, cwd, "orig"),
            "forked"
        );
        // Empty cwd: registry resolution only, no disk walk.
        assert_eq!(
            resolve_latest_conversation(&reg, projects, "", "orig"),
            "cleared"
        );
    }
}
