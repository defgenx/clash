mod helpers;

use clash::domain::entities::TaskStatus;
use clash::domain::ports::DataRepository;
use clash::infrastructure::fs::backend::FsBackend;
use clash::infrastructure::fs::store::DataStore;

use helpers::test_data_dir::TestDataDir;

#[test]
fn test_load_all_teams_from_fixtures() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());

    let teams = backend.load_teams().unwrap();

    assert_eq!(teams.len(), 5);

    let alpha = teams.iter().find(|t| t.name == "alpha-team").unwrap();
    assert_eq!(alpha.description, "The alpha research team");
    assert_eq!(alpha.members.len(), 2);
    assert!(alpha.members[0].is_active);

    let bad = teams.iter().find(|t| t.name == "bad-team").unwrap();
    assert!(bad.description.contains("Parse error"));

    let extra = teams.iter().find(|t| t.name == "extra-fields-team").unwrap();
    assert!(extra.extra.contains_key("futureField1"));
}

#[test]
fn test_load_tasks_from_fixtures() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());

    let tasks = backend.load_tasks("alpha-team").unwrap();
    assert_eq!(tasks.len(), 4);

    let task1 = tasks.iter().find(|t| t.id == "task-1").unwrap();
    assert_eq!(task1.status, TaskStatus::InProgress);
    assert_eq!(task1.owner.as_deref(), Some("researcher"));

    let task4 = tasks.iter().find(|t| t.id == "task-4").unwrap();
    assert_eq!(task4.status, TaskStatus::Blocked);
    assert_eq!(task4.blocked_by, vec!["task-3"]);
}

#[test]
fn test_data_store_full_refresh() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());
    let mut store = DataStore::new();

    store.refresh_all(&backend).unwrap();

    assert_eq!(store.teams.len(), 5);
    assert_eq!(store.get_tasks("alpha-team").len(), 4);
    assert!(store.find_team("alpha-team").is_some());
    assert!(store.find_task("alpha-team", "task-1").is_some());
}

#[test]
fn test_empty_data_dir() {
    let test_dir = TestDataDir::empty();
    let backend = FsBackend::new(test_dir.path.clone());
    let mut store = DataStore::new();

    store.refresh_all(&backend).unwrap();
    assert!(store.teams.is_empty());
}

#[test]
fn test_write_and_reload_task() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());

    let new_task = clash::domain::entities::Task {
        id: "task-new".to_string(),
        subject: "New task".to_string(),
        status: TaskStatus::Pending,
        ..Default::default()
    };

    backend.write_task("alpha-team", &new_task).unwrap();
    let tasks = backend.load_tasks("alpha-team").unwrap();
    assert_eq!(tasks.len(), 5);
    assert!(tasks.iter().any(|t| t.id == "task-new"));
}
