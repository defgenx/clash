//! Re-export DataStore from its canonical home in the application layer.
//!
//! DataStore used to live here but was moved to `application::store` to fix
//! a layer violation (application was importing from infrastructure).
//! This re-export keeps external tests and existing imports working.

#[allow(unused_imports)] // Used by lib.rs consumers (tests), not by main.rs
pub use crate::application::store::DataStore;

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
