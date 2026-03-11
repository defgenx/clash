use std::collections::HashMap;

use crate::domain::entities::{ConversationMessage, Session, Subagent, Task, Team};
use crate::domain::ports::DataRepository;
use crate::infrastructure::error::Result;

/// In-memory data store with targeted refresh support.
pub struct DataStore {
    pub teams: Vec<Team>,
    pub tasks: HashMap<String, Vec<Task>>,
    pub sessions: Vec<Session>,
    pub subagents: Vec<Subagent>,
    pub conversation: Vec<ConversationMessage>,
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
            conversation: Vec::new(),
        }
    }

    pub fn refresh_teams(&mut self, backend: &dyn DataRepository) -> Result<()> {
        self.teams = backend.load_teams()?;
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
        self.sessions = backend.load_sessions()?;
        Ok(())
    }

    /// Replace sessions with daemon-managed sessions directly.
    pub fn set_sessions(&mut self, sessions: Vec<Session>) {
        self.sessions = sessions;
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

    pub fn find_session(&self, session_id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == session_id)
    }

    pub fn load_conversation(
        &mut self,
        backend: &dyn DataRepository,
        project: &str,
        session_id: &str,
    ) -> Result<()> {
        self.conversation = backend.load_conversation(project, session_id)?;
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
        Ok(())
    }

    pub fn refresh_all(&mut self, backend: &dyn DataRepository) -> Result<()> {
        self.refresh_teams(backend)?;
        self.refresh_all_tasks(backend)?;
        // Sessions are loaded on-demand when navigating to Sessions view
        // (too expensive to scan 100s of .jsonl files on every refresh)
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
    use crate::infrastructure::fs::backend::FsBackend;
    use tempfile::TempDir;

    fn setup() -> (TempDir, FsBackend, DataStore) {
        let dir = TempDir::new().unwrap();
        let backend = FsBackend::new(dir.path().to_path_buf());
        let store = DataStore::new();
        (dir, backend, store)
    }

    #[test]
    fn test_refresh_empty() {
        let (_dir, backend, mut store) = setup();
        store.refresh_all(&backend).unwrap();
        assert!(store.teams.is_empty());
    }

    #[test]
    fn test_refresh_with_data() {
        let (dir, backend, mut store) = setup();

        let team_dir = dir.path().join("teams").join("test");
        std::fs::create_dir_all(&team_dir).unwrap();
        std::fs::write(
            team_dir.join("config.json"),
            r#"{"name": "test", "members": []}"#,
        )
        .unwrap();

        let tasks_dir = dir.path().join("tasks").join("test");
        std::fs::create_dir_all(&tasks_dir).unwrap();
        std::fs::write(
            tasks_dir.join("1.json"),
            r#"{"id": "1", "subject": "Do thing"}"#,
        )
        .unwrap();

        store.refresh_all(&backend).unwrap();
        assert_eq!(store.teams.len(), 1);
        assert_eq!(store.get_tasks("test").len(), 1);
    }

    #[test]
    fn test_find_team() {
        let (dir, backend, mut store) = setup();
        let team_dir = dir.path().join("teams").join("alpha");
        std::fs::create_dir_all(&team_dir).unwrap();
        std::fs::write(team_dir.join("config.json"), r#"{"name": "alpha"}"#).unwrap();

        store.refresh_teams(&backend).unwrap();
        assert!(store.find_team("alpha").is_some());
        assert!(store.find_team("beta").is_none());
    }
}
