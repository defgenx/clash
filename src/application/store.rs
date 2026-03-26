//! In-memory data store — caches domain data loaded via repository ports.
//!
//! Lives in the Application layer because it only depends on Domain types
//! and the DataRepository port trait. No filesystem or framework types here.

use std::collections::HashMap;

use crate::domain::entities::{ConversationMessage, Member, Session, Subagent, Task, Team};
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

    /// Sort sessions by section (Busy before Pending), then alphabetically by name.
    ///
    /// Called after all status overlays (hooks, daemon) are applied so the sort
    /// reflects final statuses. Sorting lives here (application layer) because
    /// it's a presentation concern, not a data-loading concern.
    pub fn sort_sessions(&mut self) {
        use crate::domain::entities::SessionStatus;
        self.sessions.sort_by(|a, b| {
            let section_ord = |s: &SessionStatus| match s {
                SessionStatus::Thinking | SessionStatus::Running | SessionStatus::Starting => 0,
                SessionStatus::Prompting
                | SessionStatus::Waiting
                | SessionStatus::Errored
                | SessionStatus::Idle => 1,
            };
            let name_key = |s: &Session| s.name.clone().unwrap_or_else(|| s.id.clone());
            section_ord(&a.status)
                .cmp(&section_ord(&b.status))
                .then_with(|| name_key(a).to_lowercase().cmp(&name_key(b).to_lowercase()))
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

    /// Load subagents for all sessions that have them, indexed by session ID.
    pub fn refresh_all_subagents(&mut self, backend: &dyn DataRepository) {
        self.subagents_by_session.clear();
        for session in &self.sessions {
            if session.subagent_count > 0 && !session.project.is_empty() {
                if let Ok(subs) = backend.load_subagents(&session.project, &session.id) {
                    if !subs.is_empty() {
                        self.subagents_by_session.insert(session.id.clone(), subs);
                    }
                }
            }
        }
        self.rebuild_flat_subagents();
    }

    /// Flatten teams->members into `all_members`, setting `team_name` on each.
    pub fn rebuild_all_members(&mut self) {
        self.all_members.clear();
        for team in &self.teams {
            for member in &team.members {
                let mut m = member.clone();
                m.team_name = team.name.clone();
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
            status: SessionStatus::Idle,
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
        assert_eq!(store.sessions[0].status, SessionStatus::Idle);
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
                status: SessionStatus::Idle,
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
}
