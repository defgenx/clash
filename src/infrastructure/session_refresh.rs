//! Session refresh pipeline — builds a complete session list from multiple sources.
//!
//! Extracted from `app.rs` to keep the event loop thin and make the refresh
//! logic independently testable. The core function `build_session_list` is
//! **pure** (no IO, no mutation of external state) and produces a fully sorted,
//! ready-to-display session vector.
//!
//! # Architecture
//!
//! ```text
//! gather_sync_input()          IO: disk, hooks, registry, mtimes
//! gather_daemon_input()        IO: daemon IPC
//!         │
//!         ▼
//! build_session_list()         Pure: filter, merge, overlay, sort
//!         │
//!         ▼
//! store.sessions = result      Atomic swap in app.rs
//! ```

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use crate::domain::entities::{Session, SessionStatus};
use crate::infrastructure::daemon::protocol::SessionInfo;
use crate::infrastructure::hooks::registry::ClashSession;

// ── Input ────────────────────────────────────────────────────────

/// All data needed to build the session list, gathered via IO.
///
/// `previous_sessions` is borrowed (not cloned) to avoid allocation on every
/// 500ms refresh cycle. `jsonl_mtimes` are pre-fetched so `build_session_list`
/// remains truly pure.
pub struct RefreshInput<'a> {
    /// Sessions loaded from disk (JSONL files).
    pub disk_sessions: Vec<Session>,
    /// Clash session registry (maps session ID → entry).
    pub registry: HashMap<String, ClashSession>,
    /// Hook-derived statuses with file mtimes (session_id → (status, mtime)).
    pub hook_statuses: HashMap<String, (SessionStatus, Option<SystemTime>)>,
    /// Daemon-reported session infos. `None` = daemon unreachable.
    pub daemon_infos: Option<Vec<SessionInfo>>,
    /// Saved session names from disk persistence.
    pub saved_names: HashMap<String, String>,
    /// Snapshot of the previous session list (borrowed for zero-cost merge).
    pub previous_sessions: &'a [Session],
    /// Pre-fetched JSONL file mtimes: (project, session_id) → mtime.
    pub jsonl_mtimes: HashMap<(String, String), SystemTime>,
}

// ── Gathering (IO-touching) ──────────────────────────────────────

/// Gather all synchronous input data (disk + hooks + registry + mtimes).
///
/// Called from `app.rs` before the pure `build_session_list`.
pub fn gather_sync_input<'a>(
    backend: &crate::infrastructure::fs::backend::FsBackend,
    previous: &'a [Session],
) -> RefreshInput<'a> {
    use crate::domain::ports::DataRepository;

    let disk_sessions = backend.load_sessions().unwrap_or_default();
    let registry = crate::infrastructure::hooks::registry::load();
    let hook_statuses = crate::infrastructure::hooks::read_all_statuses(backend.base_dir());
    let saved_names = crate::infrastructure::hooks::read_all_session_names(backend.base_dir());

    // Pre-fetch JSONL mtimes for all disk sessions so hook freshness checks
    // can be done without IO inside the pure builder.
    let jsonl_mtimes = disk_sessions
        .iter()
        .filter_map(|s| {
            backend
                .session_jsonl_mtime(&s.project, &s.id)
                .map(|mtime| ((s.project.clone(), s.id.clone()), mtime))
        })
        .collect();

    RefreshInput {
        disk_sessions,
        registry,
        hook_statuses,
        daemon_infos: None,
        saved_names,
        previous_sessions: previous,
        jsonl_mtimes,
    }
}

/// Gather daemon input (async IPC). Returns `None` if daemon is unreachable.
pub async fn gather_daemon_input(
    daemon: &mut crate::infrastructure::daemon::client::DaemonClient,
) -> Option<Vec<SessionInfo>> {
    if !daemon.is_connected() {
        return None;
    }
    daemon.list_sessions().await.ok()
}

// ── Building (pure) ──────────────────────────────────────────────

/// Build a complete, sorted session list from all gathered input.
///
/// This function is pure: no IO, no side effects, fully testable.
pub fn build_session_list(input: &RefreshInput<'_>) -> Vec<Session> {
    // Phase 1: Filter by registry, overlay registry fields
    let mut sessions = filter_by_registry(&input.disk_sessions, &input.registry);

    // Keep a side-map of ALL disk sessions (before registry filtering) so we
    // can enrich daemon-only sessions with disk metadata later.
    let disk_by_id: HashMap<&str, &Session> = input
        .disk_sessions
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();

    // Phase 2: Merge with previous — preserve status/is_running for existing sessions
    merge_with_previous(&mut sessions, input.previous_sessions);

    // Phase 3: Overlay hook statuses
    overlay_hook_statuses(&mut sessions, &input.hook_statuses, &input.jsonl_mtimes);

    // Phase 4: Overlay daemon statuses, add daemon-only sessions
    overlay_daemon_sessions(
        &mut sessions,
        &input.daemon_infos,
        &input.hook_statuses,
        &input.registry,
        &disk_by_id,
        input.previous_sessions,
    );

    // Phase 5: Resolve names from daemon infos and saved names
    resolve_names(&mut sessions, &input.daemon_infos, &input.saved_names);

    // Phase 6: Sort by section (Active/Done/Fail) then name
    sessions.sort_by(|a, b| {
        let name_key = |s: &Session| s.name.clone().unwrap_or_else(|| s.id.clone());
        a.status
            .section()
            .cmp(&b.status.section())
            .then_with(|| name_key(a).to_lowercase().cmp(&name_key(b).to_lowercase()))
    });

    sessions
}

// ── Phase 1: Registry filtering ──────────────────────────────────

/// Filter disk sessions to only those in the clash registry, and overlay
/// registry fields (name, cwd, source_branch).
fn filter_by_registry(
    disk_sessions: &[Session],
    registry: &HashMap<String, ClashSession>,
) -> Vec<Session> {
    use crate::infrastructure::hooks::registry::find_entry;

    if registry.is_empty() {
        // Empty registry = no clash sessions yet; show nothing from disk
        return Vec::new();
    }

    let mut result: Vec<Session> = disk_sessions
        .iter()
        .filter(|s| find_entry(registry, &s.id).is_some())
        .cloned()
        .collect();

    // Overlay registry fields onto matching sessions
    for session in &mut result {
        if let Some((_, entry)) = find_entry(registry, &session.id) {
            if !entry.name.is_empty() {
                session.name = Some(entry.name.clone());
            }
            if !entry.cwd.is_empty() {
                session.cwd = Some(entry.cwd.clone());
            }
            if entry.source_branch.is_some() {
                session.source_branch = entry.source_branch.clone();
            }
        }
    }

    result
}

// ── Phase 2: Merge with previous ─────────────────────────────────

/// For sessions that existed in the previous cycle, preserve their in-memory
/// status and is_running. Disk-based status detection is only a baseline for
/// NEW sessions — hooks and daemon overlays are the authoritative sources.
fn merge_with_previous(sessions: &mut [Session], previous: &[Session]) {
    let old_by_id: HashMap<&str, &Session> = previous.iter().map(|s| (s.id.as_str(), s)).collect();

    for session in sessions.iter_mut() {
        if let Some(old) = old_by_id.get(session.id.as_str()) {
            session.is_running = old.is_running;
            session.status = old.status;
            // Preserve daemon-assigned name
            if session.name.is_none() && old.name.is_some() {
                session.name = old.name.clone();
            }
        }
    }
}

// ── Phase 3: Hook status overlay ─────────────────────────────────

/// Apply hook-derived statuses. Hooks provide authoritative signals for:
/// - `prompting`: from PermissionRequest event
/// - `starting`: from SessionStart event
/// - `idle`: from SessionEnd event (only if hook file is newer than JSONL)
fn overlay_hook_statuses(
    sessions: &mut [Session],
    hook_statuses: &HashMap<String, (SessionStatus, Option<SystemTime>)>,
    jsonl_mtimes: &HashMap<(String, String), SystemTime>,
) {
    for session in sessions.iter_mut() {
        if let Some((hook_status, hook_mtime)) = hook_statuses.get(&session.id) {
            match hook_status {
                SessionStatus::Prompting => {
                    session.status = SessionStatus::Prompting;
                    session.is_running = true;
                }
                SessionStatus::Starting => {
                    session.status = SessionStatus::Starting;
                    session.is_running = true;
                }
                SessionStatus::Stashed => {
                    let jsonl_mtime =
                        jsonl_mtimes.get(&(session.project.clone(), session.id.clone()));
                    let hook_is_fresher = match (hook_mtime, jsonl_mtime) {
                        (Some(h), Some(j)) => h >= j,
                        (Some(_), None) => true,
                        _ => false,
                    };
                    if hook_is_fresher {
                        session.status = SessionStatus::Stashed;
                        session.is_running = false;
                    }
                }
                // Waiting/Thinking/Running: defer to daemon screen detection
                _ => {}
            }
        }
    }
}

// ── Phase 4: Daemon status overlay ───────────────────────────────

/// Overlay daemon-reported statuses onto sessions, and add daemon-only sessions.
///
/// If `daemon_infos` is `None` (daemon unreachable), preserve running
/// daemon-only sessions from `previous_sessions` to prevent flickering.
fn overlay_daemon_sessions(
    sessions: &mut Vec<Session>,
    daemon_infos: &Option<Vec<SessionInfo>>,
    hook_statuses: &HashMap<String, (SessionStatus, Option<SystemTime>)>,
    registry: &HashMap<String, ClashSession>,
    disk_by_id: &HashMap<&str, &Session>,
    previous_sessions: &[Session],
) {
    let infos = match daemon_infos {
        Some(infos) => infos,
        None => {
            // Daemon unreachable — preserve running daemon-only sessions from
            // previous cycle to prevent flickering.
            preserve_daemon_only_sessions(sessions, previous_sessions);
            return;
        }
    };

    let mut claimed_indices = HashSet::new();

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for info in infos {
        let hook_says_idle = hook_statuses
            .get(&info.session_id)
            .is_some_and(|(s, _)| matches!(s, SessionStatus::Stashed));

        let mut status = info
            .status
            .parse::<SessionStatus>()
            .unwrap_or(SessionStatus::Stashed);

        // If the process died shortly after creation, mark as errored
        // (not a stash/drop — hook would have said idle).
        if !hook_says_idle && !info.is_alive && matches!(status, SessionStatus::Stashed) {
            let age_secs = now.saturating_sub(info.created_at);
            if age_secs < 120 {
                status = SessionStatus::Errored;
            }
        }

        let is_running = !matches!(status, SessionStatus::Stashed);

        // Try to match by ID first
        let matched_by_id = sessions.iter().position(|s| s.id == info.session_id);

        if let Some(idx) = matched_by_id {
            let existing = &mut sessions[idx];
            if !is_status_dominated(existing.status, status, hook_says_idle) {
                existing.status = status;
                existing.is_running = is_running;
            }
            if existing.name.is_none() && info.name.is_some() {
                existing.name = info.name.clone();
            }
            claimed_indices.insert(idx);
        } else if info.name.is_some() && !info.cwd.is_empty() {
            // Don't re-add intentionally killed or dead sessions
            if hook_says_idle || !info.is_alive {
                continue;
            }

            // Try to match by CWD (unnamed disk session in same directory)
            let daemon_cwd = info.cwd.trim_end_matches('/');
            let matched_by_cwd = sessions.iter().enumerate().find_map(|(idx, s)| {
                let disk_path = s.project_path.trim_end_matches('/');
                if disk_path == daemon_cwd && s.name.is_none() && !claimed_indices.contains(&idx) {
                    Some(idx)
                } else {
                    None
                }
            });

            if let Some(idx) = matched_by_cwd {
                let existing = &mut sessions[idx];
                if !is_status_dominated(existing.status, status, hook_says_idle) {
                    existing.status = status;
                    existing.is_running = is_running;
                }
                existing.name = info.name.clone();
                claimed_indices.insert(idx);
            } else {
                // Daemon-only session — create from daemon info, enriched with disk data
                let mut new_session =
                    session_from_daemon_info(info, String::new(), status, is_running, registry);
                // Enrich with disk metadata if available
                if let Some(disk) = disk_by_id.get(info.session_id.as_str()) {
                    enrich_from_disk(&mut new_session, disk);
                }
                sessions.push(new_session);
            }
        } else {
            // No name or no CWD — don't add dead/idle-hooked sessions
            if hook_says_idle || !info.is_alive {
                continue;
            }

            let summary = if !info.cwd.is_empty() {
                format!("New session in {}", info.cwd)
            } else {
                let clients_info = if info.attached_clients > 0 {
                    format!("{} attached", info.attached_clients)
                } else {
                    "detached".to_string()
                };
                format!("PID {} | {}", info.pid, clients_info)
            };
            let mut new_session =
                session_from_daemon_info(info, summary, status, is_running, registry);
            if let Some(disk) = disk_by_id.get(info.session_id.as_str()) {
                enrich_from_disk(&mut new_session, disk);
            }
            sessions.push(new_session);
        }
    }
}

/// Preserve daemon-only running sessions from the previous cycle when the
/// daemon is unreachable. A session is "daemon-only" if it was running in the
/// previous cycle but doesn't appear in the current disk-loaded list.
fn preserve_daemon_only_sessions(sessions: &mut Vec<Session>, previous: &[Session]) {
    let current_ids: HashSet<String> = sessions.iter().map(|s| s.id.clone()).collect();
    for old in previous {
        if old.is_running && !current_ids.contains(&old.id) {
            sessions.push(old.clone());
        }
    }
}

/// Enrich a daemon-only session with metadata from a disk-backed session.
fn enrich_from_disk(session: &mut Session, disk: &Session) {
    if session.summary.is_empty() && !disk.summary.is_empty() {
        session.summary = disk.summary.clone();
    }
    if session.git_branch.is_empty() && !disk.git_branch.is_empty() {
        session.git_branch = disk.git_branch.clone();
    }
    if session.first_prompt.is_empty() && !disk.first_prompt.is_empty() {
        session.first_prompt = disk.first_prompt.clone();
    }
    if disk.has_subagents {
        session.has_subagents = true;
    }
    if session.subagent_count == 0 && disk.subagent_count > 0 {
        session.subagent_count = disk.subagent_count;
    }
    if session.project.is_empty() && !disk.project.is_empty() {
        session.project = disk.project.clone();
    }
    if session.project_path.is_empty() && !disk.project_path.is_empty() {
        session.project_path = disk.project_path.clone();
    }
    if session.last_modified.is_empty() && !disk.last_modified.is_empty() {
        session.last_modified = disk.last_modified.clone();
    }
    if session.worktree.is_none() && disk.worktree.is_some() {
        session.worktree = disk.worktree.clone();
    }
    if session.worktree_project.is_none() && disk.worktree_project.is_some() {
        session.worktree_project = disk.worktree_project.clone();
    }
}

// ── Phase 5: Name resolution ─────────────────────────────────────

/// Resolve session names from daemon infos and saved disk names.
/// Single pass — no second daemon IPC call.
fn resolve_names(
    sessions: &mut [Session],
    daemon_infos: &Option<Vec<SessionInfo>>,
    saved_names: &HashMap<String, String>,
) {
    // Apply daemon-reported names (matched by project path)
    if let Some(infos) = daemon_infos {
        for info in infos {
            if let Some(ref daemon_name) = info.name {
                if info.cwd.is_empty() {
                    continue;
                }
                let daemon_project = path_last_component(&info.cwd);
                if daemon_project.is_empty() {
                    continue;
                }
                for session in sessions.iter_mut() {
                    if session.name.is_some() {
                        continue;
                    }
                    if path_last_component(&session.project_path) == daemon_project {
                        session.name = Some(daemon_name.clone());
                    }
                }
            }
        }
    }

    // Apply saved names from disk persistence
    for session in sessions.iter_mut() {
        if session.name.is_none() {
            if let Some(name) = saved_names.get(&session.id) {
                session.name = Some(name.clone());
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────

/// Returns `true` if the `existing` status should NOT be overridden by `new`.
///
/// Priority rules:
/// - Prompting (hook-derived) > Waiting/Stashed (daemon-derived)
/// - Stashed (intentional) cannot be overridden to Errored
/// - Hook-idle blocks any non-idle daemon override
pub fn is_status_dominated(
    existing: SessionStatus,
    new: SessionStatus,
    hook_says_idle: bool,
) -> bool {
    // Prompting is authoritative (from hook PermissionRequest event)
    (matches!(existing, SessionStatus::Prompting)
        && matches!(new, SessionStatus::Waiting | SessionStatus::Stashed))
    // Stashed cannot become Errored (intentional stop, not a crash)
    || (matches!(existing, SessionStatus::Stashed) && matches!(new, SessionStatus::Errored))
    // Hook-idle blocks daemon from overriding to any non-idle status
    || (hook_says_idle && !matches!(new, SessionStatus::Stashed))
}

/// Build a `Session` from daemon `SessionInfo` for sessions with no disk file.
/// Takes registry as parameter instead of loading from disk (eliminates hidden IO).
fn session_from_daemon_info(
    info: &SessionInfo,
    summary: String,
    status: SessionStatus,
    is_running: bool,
    registry: &HashMap<String, ClashSession>,
) -> Session {
    let cwd = if info.cwd.is_empty() {
        registry
            .get(&info.session_id)
            .map(|e| e.cwd.clone())
            .filter(|c| !c.is_empty())
    } else {
        Some(info.cwd.clone())
    };
    Session {
        id: info.session_id.clone(),
        project: path_last_component(&info.cwd).to_string(),
        project_path: info.cwd.clone(),
        summary,
        is_running,
        status,
        name: info.name.clone(),
        cwd,
        ..Default::default()
    }
}

/// Extract the last component of a path string (e.g., "/foo/bar" → "bar").
fn path_last_component(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(id: &str, status: SessionStatus, is_running: bool) -> Session {
        Session {
            id: id.to_string(),
            status,
            is_running,
            ..Default::default()
        }
    }

    fn make_disk_session(id: &str, project: &str, summary: &str) -> Session {
        Session {
            id: id.to_string(),
            project: project.to_string(),
            project_path: format!("/home/user/{}", project),
            summary: summary.to_string(),
            git_branch: "main".to_string(),
            ..Default::default()
        }
    }

    fn make_registry_entry(session_id: &str, name: &str, cwd: &str) -> ClashSession {
        ClashSession {
            session_id: session_id.to_string(),
            name: name.to_string(),
            cwd: cwd.to_string(),
            claude_session_id: session_id.to_string(),
            created_at: String::new(),
            source_branch: None,
        }
    }

    fn make_daemon_info(session_id: &str, cwd: &str, status: &str, is_alive: bool) -> SessionInfo {
        SessionInfo {
            session_id: session_id.to_string(),
            pid: 1234,
            is_alive,
            attached_clients: 0,
            created_at: 0,
            status: status.to_string(),
            cwd: cwd.to_string(),
            name: Some("test-session".to_string()),
        }
    }

    fn empty_input<'a>(previous: &'a [Session]) -> RefreshInput<'a> {
        RefreshInput {
            disk_sessions: Vec::new(),
            registry: HashMap::new(),
            hook_statuses: HashMap::new(),
            daemon_infos: None,
            saved_names: HashMap::new(),
            previous_sessions: previous,
            jsonl_mtimes: HashMap::new(),
        }
    }

    // ── is_status_dominated truth table ──────────────────────────

    #[test]
    fn test_is_status_dominated_truth_table() {
        // Prompting > Waiting (hook-derived prompting shouldn't be downgraded)
        assert!(is_status_dominated(
            SessionStatus::Prompting,
            SessionStatus::Waiting,
            false
        ));
        // Prompting > Stashed
        assert!(is_status_dominated(
            SessionStatus::Prompting,
            SessionStatus::Stashed,
            false
        ));
        // Prompting does NOT dominate Running (daemon upgrade is OK)
        assert!(!is_status_dominated(
            SessionStatus::Prompting,
            SessionStatus::Running,
            false
        ));
        // Stashed > Errored (intentional stop, not a crash)
        assert!(is_status_dominated(
            SessionStatus::Stashed,
            SessionStatus::Errored,
            false
        ));
        // Stashed does NOT dominate Running (session restarted)
        assert!(!is_status_dominated(
            SessionStatus::Stashed,
            SessionStatus::Running,
            false
        ));
        // Hook-idle blocks daemon Running override
        assert!(is_status_dominated(
            SessionStatus::Stashed,
            SessionStatus::Running,
            true
        ));
        // Hook-idle blocks daemon Waiting override
        assert!(is_status_dominated(
            SessionStatus::Stashed,
            SessionStatus::Waiting,
            true
        ));
        // Hook-idle does NOT block Stashed (consistent)
        assert!(!is_status_dominated(
            SessionStatus::Running,
            SessionStatus::Stashed,
            true
        ));
        // Normal: Running can be overridden by Waiting
        assert!(!is_status_dominated(
            SessionStatus::Running,
            SessionStatus::Waiting,
            false
        ));
        // Normal: Waiting can be overridden by Running
        assert!(!is_status_dominated(
            SessionStatus::Waiting,
            SessionStatus::Running,
            false
        ));
        // Normal: Thinking can be overridden by Running
        assert!(!is_status_dominated(
            SessionStatus::Thinking,
            SessionStatus::Running,
            false
        ));
    }

    // ── Registry filtering ───────────────────────────────────────

    #[test]
    fn test_registry_filtering() {
        let disk = vec![
            make_disk_session("s1", "proj-a", "summary 1"),
            make_disk_session("s2", "proj-b", "summary 2"),
            make_disk_session("s3", "proj-c", "summary 3"),
        ];
        let mut registry = HashMap::new();
        registry.insert(
            "s1".to_string(),
            make_registry_entry("s1", "session-1", "/tmp/a"),
        );
        registry.insert(
            "s3".to_string(),
            make_registry_entry("s3", "session-3", "/tmp/c"),
        );

        let result = filter_by_registry(&disk, &registry);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, "s1");
        assert_eq!(result[0].name.as_deref(), Some("session-1"));
        assert_eq!(result[0].cwd.as_deref(), Some("/tmp/a"));
        assert_eq!(result[1].id, "s3");
    }

    #[test]
    fn test_registry_empty_returns_nothing() {
        let disk = vec![make_disk_session("s1", "proj-a", "summary")];
        let registry = HashMap::new();
        let result = filter_by_registry(&disk, &registry);
        assert!(result.is_empty());
    }

    // ── Merge with previous ──────────────────────────────────────

    #[test]
    fn test_merge_preserves_running_status() {
        let mut sessions = vec![make_session("s1", SessionStatus::Stashed, false)];
        let previous = vec![make_session("s1", SessionStatus::Running, true)];
        merge_with_previous(&mut sessions, &previous);
        assert_eq!(sessions[0].status, SessionStatus::Running);
        assert!(sessions[0].is_running);
    }

    #[test]
    fn test_merge_preserves_idle_status() {
        let mut sessions = vec![make_session("s1", SessionStatus::Running, true)];
        let previous = vec![make_session("s1", SessionStatus::Stashed, false)];
        merge_with_previous(&mut sessions, &previous);
        assert_eq!(sessions[0].status, SessionStatus::Stashed);
        assert!(!sessions[0].is_running);
    }

    #[test]
    fn test_merge_new_session_keeps_disk_status() {
        let mut sessions = vec![make_session("s1", SessionStatus::Waiting, true)];
        let previous: Vec<Session> = vec![]; // no previous — brand new
        merge_with_previous(&mut sessions, &previous);
        assert_eq!(sessions[0].status, SessionStatus::Waiting);
    }

    // ── Hook overlay ─────────────────────────────────────────────

    #[test]
    fn test_hook_overlay_prompting_authoritative() {
        let mut sessions = vec![make_session("s1", SessionStatus::Waiting, true)];
        let mut hooks = HashMap::new();
        hooks.insert(
            "s1".to_string(),
            (SessionStatus::Prompting, Some(SystemTime::UNIX_EPOCH)),
        );
        overlay_hook_statuses(&mut sessions, &hooks, &HashMap::new());
        assert_eq!(sessions[0].status, SessionStatus::Prompting);
        assert!(sessions[0].is_running);
    }

    #[test]
    fn test_hook_stale_idle_ignored() {
        let now = SystemTime::now();
        let earlier = now - std::time::Duration::from_secs(10);
        let mut sessions = vec![{
            let mut s = make_session("s1", SessionStatus::Running, true);
            s.project = "proj".to_string();
            s
        }];
        let mut hooks = HashMap::new();
        hooks.insert("s1".to_string(), (SessionStatus::Stashed, Some(earlier)));
        let mut mtimes = HashMap::new();
        mtimes.insert(("proj".to_string(), "s1".to_string()), now);

        overlay_hook_statuses(&mut sessions, &hooks, &mtimes);
        // Hook is older than JSONL → should NOT override
        assert_eq!(sessions[0].status, SessionStatus::Running);
        assert!(sessions[0].is_running);
    }

    // ── Daemon overlay ───────────────────────────────────────────

    #[test]
    fn test_daemon_overlay_merges_status() {
        let mut sessions = vec![make_session("s1", SessionStatus::Stashed, false)];
        let infos = Some(vec![make_daemon_info("s1", "/tmp", "running", true)]);
        overlay_daemon_sessions(
            &mut sessions,
            &infos,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &[],
        );
        assert_eq!(sessions[0].status, SessionStatus::Running);
        assert!(sessions[0].is_running);
    }

    #[test]
    fn test_daemon_only_sessions_added() {
        let mut sessions = Vec::new();
        let infos = Some(vec![make_daemon_info(
            "s1",
            "/tmp/project",
            "running",
            true,
        )]);
        overlay_daemon_sessions(
            &mut sessions,
            &infos,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &[],
        );
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "s1");
        assert!(sessions[0].is_running);
    }

    #[test]
    fn test_daemon_unreachable_preserves_running_sessions() {
        let mut sessions = vec![make_session("s1", SessionStatus::Waiting, true)];
        let previous = vec![
            make_session("s1", SessionStatus::Waiting, true),
            make_session("s2", SessionStatus::Running, true), // daemon-only from previous
        ];
        // daemon_infos = None → daemon unreachable
        overlay_daemon_sessions(
            &mut sessions,
            &None,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &previous,
        );
        // s2 should be preserved from previous
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|s| s.id == "s2" && s.is_running));
    }

    #[test]
    fn test_multi_cycle_stability() {
        // Simulate 3 refresh cycles: daemon present → absent → present
        let registry = {
            let mut r = HashMap::new();
            r.insert(
                "s1".to_string(),
                make_registry_entry("s1", "sess-1", "/tmp"),
            );
            r
        };
        let disk = vec![make_disk_session("s1", "proj", "summary")];

        // Cycle 1: daemon present, reports s1 running + s2 daemon-only
        let mut input = empty_input(&[]);
        input.disk_sessions = disk.clone();
        input.registry = registry.clone();
        input.daemon_infos = Some(vec![
            make_daemon_info("s1", "/tmp", "running", true),
            make_daemon_info("s2", "/tmp/other", "waiting", true),
        ]);
        let cycle1 = build_session_list(&input);
        assert_eq!(cycle1.len(), 2);

        // Cycle 2: daemon unreachable
        let mut input2 = empty_input(&cycle1);
        input2.disk_sessions = disk.clone();
        input2.registry = registry.clone();
        input2.daemon_infos = None; // unreachable
        let cycle2 = build_session_list(&input2);
        // Both sessions should survive
        assert_eq!(cycle2.len(), 2);
        assert!(cycle2.iter().any(|s| s.id == "s2"));

        // Cycle 3: daemon back, reports same sessions
        let mut input3 = empty_input(&cycle2);
        input3.disk_sessions = disk;
        input3.registry = registry;
        input3.daemon_infos = Some(vec![
            make_daemon_info("s1", "/tmp", "running", true),
            make_daemon_info("s2", "/tmp/other", "waiting", true),
        ]);
        let cycle3 = build_session_list(&input3);
        assert_eq!(cycle3.len(), 2);
    }

    // ── Daemon enrichment from disk ──────────────────────────────

    #[test]
    fn test_daemon_only_session_enriched_with_disk_metadata() {
        let mut sessions = Vec::new();
        let infos = Some(vec![make_daemon_info(
            "s1",
            "/home/user/proj-a",
            "running",
            true,
        )]);
        let disk_session = make_disk_session("s1", "proj-a", "My task summary");
        let disk_by_id: HashMap<&str, &Session> = [("s1", &disk_session)].into_iter().collect();

        overlay_daemon_sessions(
            &mut sessions,
            &infos,
            &HashMap::new(),
            &HashMap::new(),
            &disk_by_id,
            &[],
        );

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].summary, "My task summary");
        assert_eq!(sessions[0].git_branch, "main");
    }

    // ── Name resolution ──────────────────────────────────────────

    #[test]
    fn test_name_resolution_single_pass() {
        let mut sessions = vec![
            {
                let mut s = make_session("s1", SessionStatus::Running, true);
                s.project_path = "/home/user/project".to_string();
                s
            },
            {
                let mut s = make_session("s2", SessionStatus::Waiting, true);
                s.name = Some("already-named".to_string());
                s.project_path = "/home/user/project".to_string();
                s
            },
            make_session("s3", SessionStatus::Stashed, false),
        ];

        let daemon_infos = Some(vec![SessionInfo {
            session_id: "d1".to_string(),
            pid: 100,
            is_alive: true,
            attached_clients: 0,
            created_at: 0,
            status: "running".to_string(),
            cwd: "/home/user/project".to_string(),
            name: Some("daemon-name".to_string()),
        }]);

        let mut saved = HashMap::new();
        saved.insert("s3".to_string(), "saved-name".to_string());

        resolve_names(&mut sessions, &daemon_infos, &saved);

        // s1 gets daemon name (matched by project path)
        assert_eq!(sessions[0].name.as_deref(), Some("daemon-name"));
        // s2 already had a name — not overwritten
        assert_eq!(sessions[1].name.as_deref(), Some("already-named"));
        // s3 gets saved name from disk
        assert_eq!(sessions[2].name.as_deref(), Some("saved-name"));
    }

    // ── Sort ─────────────────────────────────────────────────────

    #[test]
    fn test_sort_active_before_done() {
        let previous: Vec<Session> = vec![];
        let mut input = empty_input(&previous);
        input.disk_sessions = vec![
            make_session("s1", SessionStatus::Stashed, false),
            make_session("s2", SessionStatus::Running, true),
            make_session("s3", SessionStatus::Errored, false),
        ];
        // No registry filtering — use a registry with all sessions
        input.registry = [
            ("s1".to_string(), make_registry_entry("s1", "a-done", "")),
            ("s2".to_string(), make_registry_entry("s2", "b-active", "")),
            ("s3".to_string(), make_registry_entry("s3", "c-fail", "")),
        ]
        .into_iter()
        .collect();

        let result = build_session_list(&input);
        // Active first, then Done, then Fail
        assert_eq!(result[0].id, "s2"); // Running → Active
        assert_eq!(result[1].id, "s1"); // Stashed → Done
        assert_eq!(result[2].id, "s3"); // Errored → Fail
    }
}
