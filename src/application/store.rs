//! In-memory data store — caches domain data loaded via repository ports.
//!
//! Lives in the Application layer because it only depends on Domain types
//! and the DataRepository port trait. No filesystem or framework types here.

use std::collections::HashMap;

use crate::domain::entities::{ConversationMessage, Member, Preset, Session, Subagent, Task, Team};
use crate::domain::error::Result;
use crate::domain::ports::DataRepository;

/// In-memory data store with targeted refresh support.
pub struct DataStore {
    pub teams: Vec<Team>,
    pub tasks: HashMap<String, Vec<Task>>,
    pub sessions: Vec<Session>,
    pub subagents: Vec<Subagent>,
    /// Subagents indexed by parent session ID (for tree view).
    pub subagents_by_session: HashMap<String, Vec<Subagent>>,
    pub conversation: Vec<ConversationMessage>,
    pub conversation_loaded: bool,
    /// Flattened members from all teams (with team_name set).
    pub all_members: Vec<Member>,
    /// Flattened subagents from all sessions.
    pub all_subagents: Vec<Subagent>,
    /// Cached presets (loaded from .clash/presets.json, global config, .superset/config.json).
    pub presets: Vec<Preset>,
}

impl Default for DataStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DataStore {
    pub fn new() -> Self {
        Self {
            teams: Vec::new(),
            tasks: HashMap::new(),
            sessions: Vec::new(),
            subagents: Vec::new(),
            subagents_by_session: HashMap::new(),
            conversation: Vec::new(),
            conversation_loaded: false,
            all_members: Vec::new(),
            all_subagents: Vec::new(),
            presets: Vec::new(),
        }
    }

    pub fn refresh_teams(&mut self, backend: &dyn DataRepository) -> Result<()> {
        self.teams = backend.load_teams()?;
        self.rebuild_all_members();
        Ok(())
    }

    pub fn refresh_tasks(&mut self, backend: &dyn DataRepository, team: &str) -> Result<()> {
        let tasks = backend.load_tasks(team)?;
        self.tasks.insert(team.to_string(), tasks);
        Ok(())
    }

    pub fn refresh_all_tasks(&mut self, backend: &dyn DataRepository) -> Result<()> {
        let team_names: Vec<String> = self.teams.iter().map(|t| t.name.clone()).collect();
        for name in team_names {
            self.refresh_tasks(backend, &name)?;
        }
        Ok(())
    }

    /// Superseded by `session_refresh::build_session_list` — kept for tests only.
    #[cfg(test)]
    pub fn refresh_sessions(&mut self, backend: &dyn DataRepository) -> Result<()> {
        let new_sessions = backend.load_sessions()?;

        // Merge: preserve daemon-derived fields (status, is_running, name) from
        // existing sessions when the disk-based status would downgrade them.
        // This prevents "flickering" when disk says idle but daemon says running.
        let old_by_id: HashMap<String, &Session> =
            self.sessions.iter().map(|s| (s.id.clone(), s)).collect();

        self.sessions = new_sessions
            .into_iter()
            .map(|mut new| {
                if let Some(old) = old_by_id.get(&new.id) {
                    // Preserve in-memory status for existing sessions.
                    // Disk-based status detection is just a baseline for NEW sessions.
                    // Hooks and daemon overlays (which run immediately after this
                    // in refresh_daemon_sessions) are the authoritative sources
                    // for status updates on existing sessions.
                    new.is_running = old.is_running;
                    new.status = old.status;
                    // Preserve daemon-assigned name
                    if new.name.is_none() && old.name.is_some() {
                        new.name = old.name.clone();
                    }
                }
                new
            })
            .collect();

        Ok(())
    }

    /// Sort sessions by section (Active → Done → Fail), then alphabetically by name.
    ///
    /// Re-sort sessions by section then name. Called after in-memory name
    /// changes so the list stays ordered between merge-based refresh cycles.
    pub fn sort_sessions(&mut self) {
        self.sessions.sort_by(|a, b| {
            let name_key = |s: &Session| s.name.clone().unwrap_or_else(|| s.id.clone());
            a.status
                .section()
                .cmp(&b.status.section())
                .then_with(|| name_key(a).to_lowercase().cmp(&name_key(b).to_lowercase()))
                .then_with(|| a.id.cmp(&b.id))
        });
    }

    pub fn refresh_subagents(
        &mut self,
        backend: &dyn DataRepository,
        project: &str,
        session_id: &str,
    ) -> Result<()> {
        self.subagents = backend.load_subagents(project, session_id)?;
        Ok(())
    }

    /// Delta-based subagent reloading: only reload subagents for sessions whose
    /// status or subagent_count changed since the last refresh.
    pub fn refresh_changed_subagents(
        &mut self,
        backend: &dyn DataRepository,
        previous_sessions: &[Session],
    ) {
        let old_by_id: HashMap<&str, &Session> = previous_sessions
            .iter()
            .map(|s| (s.id.as_str(), s))
            .collect();

        for session in &self.sessions {
            if session.subagent_count == 0 || session.project.is_empty() {
                self.subagents_by_session.remove(&session.id);
                continue;
            }
            let changed = !old_by_id.get(session.id.as_str()).is_some_and(|old| {
                old.subagent_count == session.subagent_count && old.status == session.status
            });
            if changed {
                if let Ok(subs) = backend.load_subagents(&session.project, &session.id) {
                    if subs.is_empty() {
                        self.subagents_by_session.remove(&session.id);
                    } else {
                        self.subagents_by_session.insert(session.id.clone(), subs);
                    }
                }
            }
        }
        // Remove entries for sessions that no longer exist
        let current_ids: std::collections::HashSet<&str> =
            self.sessions.iter().map(|s| s.id.as_str()).collect();
        self.subagents_by_session
            .retain(|k, _| current_ids.contains(k.as_str()));
        self.rebuild_flat_subagents();
    }

    /// Flatten teams->members into `all_members`, setting `team_name` on each.
    /// Cross-references `is_active` with session liveness — if config.json says
    /// active but no matching running session exists, marks the agent as inactive.
    pub fn rebuild_all_members(&mut self) {
        self.all_members.clear();
        for team in &self.teams {
            for member in &team.members {
                let mut m = member.clone();
                m.team_name = team.name.clone();
                // Cross-reference: if config.json says active but no matching
                // running session exists (by CWD), mark as inactive.
                if m.is_active {
                    let has_running = self.sessions.iter().any(|s| {
                        s.is_running
                            && m.cwd.as_deref().is_some_and(|cwd| {
                                let cwd = cwd.trim_end_matches('/');
                                s.project_path.trim_end_matches('/') == cwd
                                    || s.cwd
                                        .as_deref()
                                        .is_some_and(|sc| sc.trim_end_matches('/') == cwd)
                            })
                    });
                    if !has_running {
                        m.is_active = false;
                    }
                }
                self.all_members.push(m);
            }
        }
    }

    /// Flatten `subagents_by_session` into `all_subagents` (sorted for stable ordering).
    pub fn rebuild_flat_subagents(&mut self) {
        self.all_subagents.clear();
        for subs in self.subagents_by_session.values() {
            for sa in subs {
                self.all_subagents.push(sa.clone());
            }
        }
        // Sort to ensure deterministic order regardless of HashMap iteration order.
        self.all_subagents
            .sort_by(|a, b| b.last_modified.cmp(&a.last_modified).then(a.id.cmp(&b.id)));
    }

    pub fn find_session(&self, session_id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == session_id)
    }

    /// Find a subagent by ID, searching both the flat list and the per-session map.
    pub fn find_subagent(&self, agent_id: &str) -> Option<&Subagent> {
        // Check the flat list first (loaded when navigating to Subagents view)
        if let Some(sa) = self.subagents.iter().find(|s| s.id == agent_id) {
            return Some(sa);
        }
        // Fall back to the per-session map (loaded for tree view)
        for subs in self.subagents_by_session.values() {
            if let Some(sa) = subs.iter().find(|s| s.id == agent_id) {
                return Some(sa);
            }
        }
        None
    }

    pub fn load_conversation(
        &mut self,
        backend: &dyn DataRepository,
        project: &str,
        session_id: &str,
    ) -> Result<()> {
        self.conversation = backend.load_conversation(project, session_id)?;
        self.conversation_loaded = true;
        Ok(())
    }

    pub fn load_subagent_conversation(
        &mut self,
        backend: &dyn DataRepository,
        project: &str,
        session_id: &str,
        agent_id: &str,
    ) -> Result<()> {
        self.conversation = backend.load_subagent_conversation(project, session_id, agent_id)?;
        self.conversation_loaded = true;
        Ok(())
    }

    pub fn refresh_all(&mut self, backend: &dyn DataRepository) -> Result<()> {
        self.refresh_teams(backend)?;
        self.refresh_all_tasks(backend)?;
        Ok(())
    }

    pub fn get_tasks(&self, team: &str) -> &[Task] {
        self.tasks.get(team).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn find_team(&self, name: &str) -> Option<&Team> {
        self.teams.iter().find(|t| t.name == name)
    }

    pub fn find_task(&self, team: &str, task_id: &str) -> Option<&Task> {
        self.get_tasks(team).iter().find(|t| t.id == task_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::SessionStatus;

    /// Minimal mock that only implements load_sessions for merge tests.
    struct MockBackend {
        sessions: Vec<Session>,
    }

    impl crate::domain::ports::DataRepository for MockBackend {
        fn load_teams(&self) -> crate::domain::error::Result<Vec<Team>> {
            Ok(vec![])
        }
        fn load_tasks(&self, _team: &str) -> crate::domain::error::Result<Vec<Task>> {
            Ok(vec![])
        }
        fn write_task(&self, _team: &str, _task: &Task) -> crate::domain::error::Result<()> {
            Ok(())
        }
        fn delete_team(&self, _name: &str) -> crate::domain::error::Result<()> {
            Ok(())
        }
        fn teams_dir(&self) -> std::path::PathBuf {
            std::path::PathBuf::new()
        }
        fn tasks_dir(&self) -> std::path::PathBuf {
            std::path::PathBuf::new()
        }
        fn load_sessions(&self) -> crate::domain::error::Result<Vec<Session>> {
            Ok(self.sessions.clone())
        }
        fn load_subagents(
            &self,
            _project: &str,
            _session_id: &str,
        ) -> crate::domain::error::Result<Vec<Subagent>> {
            Ok(vec![])
        }
        fn load_conversation(
            &self,
            _project: &str,
            _session_id: &str,
        ) -> crate::domain::error::Result<Vec<crate::domain::entities::ConversationMessage>>
        {
            Ok(vec![])
        }
        fn load_subagent_conversation(
            &self,
            _project: &str,
            _session_id: &str,
            _agent_id: &str,
        ) -> crate::domain::error::Result<Vec<crate::domain::entities::ConversationMessage>>
        {
            Ok(vec![])
        }
    }

    #[test]
    fn test_refresh_preserves_idle_status() {
        let mut store = DataStore::new();
        // In-memory: session is idle (e.g., after stash)
        store.sessions = vec![Session {
            id: "s1".to_string(),
            is_running: false,
            status: SessionStatus::Stashed,
            ..Default::default()
        }];

        // Disk: session appears running (JSONL has recent activity)
        let backend = MockBackend {
            sessions: vec![Session {
                id: "s1".to_string(),
                is_running: true,
                status: SessionStatus::Running,
                ..Default::default()
            }],
        };

        store.refresh_sessions(&backend).unwrap();

        // Old idle status should be preserved — overlays are authoritative
        assert!(!store.sessions[0].is_running);
        assert_eq!(store.sessions[0].status, SessionStatus::Stashed);
    }

    #[test]
    fn test_refresh_preserves_running_status() {
        let mut store = DataStore::new();
        // In-memory: session is running (daemon says so)
        store.sessions = vec![Session {
            id: "s1".to_string(),
            is_running: true,
            status: SessionStatus::Running,
            ..Default::default()
        }];

        // Disk: session appears idle (stale JSONL)
        let backend = MockBackend {
            sessions: vec![Session {
                id: "s1".to_string(),
                is_running: false,
                status: SessionStatus::Stashed,
                ..Default::default()
            }],
        };

        store.refresh_sessions(&backend).unwrap();

        // Old running status should be preserved — overlays are authoritative
        assert!(store.sessions[0].is_running);
        assert_eq!(store.sessions[0].status, SessionStatus::Running);
    }

    #[test]
    fn test_rebuild_all_members() {
        let mut store = DataStore::new();
        store.teams = vec![
            Team {
                name: "alpha".to_string(),
                members: vec![
                    Member {
                        name: "alice".to_string(),
                        ..Default::default()
                    },
                    Member {
                        name: "bob".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            Team {
                name: "beta".to_string(),
                members: vec![Member {
                    name: "charlie".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];
        store.rebuild_all_members();
        assert_eq!(store.all_members.len(), 3);
        assert_eq!(store.all_members[0].team_name, "alpha");
        assert_eq!(store.all_members[0].name, "alice");
        assert_eq!(store.all_members[2].team_name, "beta");
        assert_eq!(store.all_members[2].name, "charlie");
    }

    #[test]
    fn test_rebuild_flat_subagents() {
        use crate::domain::entities::Subagent;
        let mut store = DataStore::new();
        store.subagents_by_session.insert(
            "s1".to_string(),
            vec![
                Subagent {
                    id: "sa1".to_string(),
                    parent_session_id: "s1".to_string(),
                    ..Default::default()
                },
                Subagent {
                    id: "sa2".to_string(),
                    parent_session_id: "s1".to_string(),
                    ..Default::default()
                },
            ],
        );
        store.subagents_by_session.insert(
            "s2".to_string(),
            vec![Subagent {
                id: "sa3".to_string(),
                parent_session_id: "s2".to_string(),
                ..Default::default()
            }],
        );
        store.rebuild_flat_subagents();
        assert_eq!(store.all_subagents.len(), 3);
    }

    #[test]
    fn test_rebuild_members_cross_ref_active_with_running_session() {
        let mut store = DataStore::new();
        store.sessions = vec![Session {
            id: "s1".to_string(),
            is_running: true,
            status: SessionStatus::Running,
            project_path: "/home/user/project".to_string(),
            ..Default::default()
        }];
        store.teams = vec![Team {
            name: "team1".to_string(),
            members: vec![Member {
                name: "alice".to_string(),
                is_active: true,
                cwd: Some("/home/user/project".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        store.rebuild_all_members();
        assert!(store.all_members[0].is_active);
    }

    #[test]
    fn test_rebuild_members_cross_ref_stale_active() {
        let mut store = DataStore::new();
        // No running sessions
        store.sessions = vec![Session {
            id: "s1".to_string(),
            is_running: false,
            status: SessionStatus::Stashed,
            project_path: "/home/user/project".to_string(),
            ..Default::default()
        }];
        store.teams = vec![Team {
            name: "team1".to_string(),
            members: vec![Member {
                name: "alice".to_string(),
                is_active: true,
                cwd: Some("/home/user/project".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        store.rebuild_all_members();
        // Agent should be marked inactive since no running session matches
        assert!(!store.all_members[0].is_active);
    }

    #[test]
    fn test_rebuild_members_cross_ref_trailing_slash_normalized() {
        let mut store = DataStore::new();
        store.sessions = vec![Session {
            id: "s1".to_string(),
            is_running: true,
            status: SessionStatus::Running,
            project_path: "/home/user/project/".to_string(),
            ..Default::default()
        }];
        store.teams = vec![Team {
            name: "team1".to_string(),
            members: vec![Member {
                name: "bob".to_string(),
                is_active: true,
                cwd: Some("/home/user/project".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        }];
        store.rebuild_all_members();
        assert!(store.all_members[0].is_active);
    }

    #[test]
    fn test_refresh_changed_subagents_only_reloads_changed() {
        let mut store = DataStore::new();
        store.sessions = vec![
            Session {
                id: "s1".to_string(),
                project: "proj".to_string(),
                subagent_count: 2,
                status: SessionStatus::Running,
                ..Default::default()
            },
            Session {
                id: "s2".to_string(),
                project: "proj".to_string(),
                subagent_count: 1,
                status: SessionStatus::Waiting,
                ..Default::default()
            },
        ];
        // Pre-populate s2 subagents (unchanged)
        store.subagents_by_session.insert(
            "s2".to_string(),
            vec![Subagent {
                id: "sa-existing".to_string(),
                parent_session_id: "s2".to_string(),
                ..Default::default()
            }],
        );

        // Previous: s1 had different status, s2 was the same
        let previous = vec![
            Session {
                id: "s1".to_string(),
                project: "proj".to_string(),
                subagent_count: 2,
                status: SessionStatus::Thinking, // changed
                ..Default::default()
            },
            Session {
                id: "s2".to_string(),
                project: "proj".to_string(),
                subagent_count: 1,
                status: SessionStatus::Waiting, // same
                ..Default::default()
            },
        ];

        let backend = MockBackend { sessions: vec![] };
        store.refresh_changed_subagents(&backend, &previous);

        // s1 was changed → reloaded (MockBackend returns empty → removed)
        assert!(!store.subagents_by_session.contains_key("s1"));
        // s2 was unchanged → kept from before
        assert!(store.subagents_by_session.contains_key("s2"));
        assert_eq!(store.subagents_by_session["s2"][0].id, "sa-existing");
    }
}
