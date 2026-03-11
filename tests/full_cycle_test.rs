mod helpers;

use clash::adapters::views::ViewKind;
use clash::application::actions::{Action, NavAction, TableAction, TaskAction, UiAction};
use clash::application::effects::Effect;
use clash::application::reducer;
use clash::application::state::AppState;
use clash::infrastructure::fs::backend::FsBackend;

use helpers::test_data_dir::TestDataDir;

fn setup() -> (TestDataDir, FsBackend, AppState) {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());
    let mut state = AppState::new();
    state.store.refresh_all(&backend).unwrap();
    (test_dir, backend, state)
}

#[test]
fn test_navigate_teams_to_tasks() {
    let (_dir, _backend, mut state) = setup();

    assert_eq!(state.current_view(), ViewKind::Sessions);
    assert_eq!(state.store.teams.len(), 5);

    // Navigate to Teams first, then drill in
    reducer::reduce(
        &mut state,
        Action::Nav(NavAction::NavigateTo(ViewKind::Teams)),
    );
    assert_eq!(state.current_view(), ViewKind::Teams);

    reducer::reduce(
        &mut state,
        Action::Nav(NavAction::DrillIn {
            view: ViewKind::TeamDetail,
            context: "alpha-team".to_string(),
        }),
    );
    assert_eq!(state.current_view(), ViewKind::TeamDetail);

    reducer::reduce(
        &mut state,
        Action::Nav(NavAction::NavigateTo(ViewKind::Tasks)),
    );
    assert_eq!(state.current_view(), ViewKind::Tasks);
}

#[test]
fn test_table_navigation() {
    let (_dir, _backend, mut state) = setup();

    // Navigate to Teams (default is now Sessions, which has no test data)
    reducer::reduce(
        &mut state,
        Action::Nav(NavAction::NavigateTo(ViewKind::Teams)),
    );

    reducer::reduce(&mut state, Action::Table(TableAction::Next));
    assert_eq!(state.table_state.selected, 1);

    reducer::reduce(&mut state, Action::Table(TableAction::Last));
    assert_eq!(state.table_state.selected, 4);

    reducer::reduce(&mut state, Action::Table(TableAction::First));
    assert_eq!(state.table_state.selected, 0);
}

#[test]
fn test_cycle_task_status() {
    let (_dir, backend, mut state) = setup();

    reducer::reduce(
        &mut state,
        Action::Nav(NavAction::DrillIn {
            view: ViewKind::TeamDetail,
            context: "alpha-team".to_string(),
        }),
    );

    state.store.refresh_tasks(&backend, "alpha-team").unwrap();

    let effects = reducer::reduce(
        &mut state,
        Action::Task(TaskAction::CycleStatus {
            team: "alpha-team".to_string(),
            task_id: "task-2".to_string(),
        }),
    );

    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PersistTask { .. })));
}

#[test]
fn test_create_task_effects() {
    let (_dir, _backend, mut state) = setup();

    let effects = reducer::reduce(
        &mut state,
        Action::Task(TaskAction::Create {
            team: "alpha-team".to_string(),
            subject: "New task".to_string(),
            description: "Test".to_string(),
        }),
    );

    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::PersistTask { .. })));
    assert_eq!(state.toast.as_deref(), Some("Task created"));
}

#[test]
fn test_command_mode_navigation() {
    let (_dir, _backend, mut state) = setup();

    reducer::reduce(&mut state, Action::Ui(UiAction::EnterCommandMode));
    assert!(matches!(
        state.input_mode,
        clash::application::state::InputMode::Command
    ));

    reducer::reduce(
        &mut state,
        Action::Ui(UiAction::SubmitInput("tasks".to_string())),
    );
    assert_eq!(state.current_view(), ViewKind::Tasks);
}

#[test]
fn test_breadcrumb_trail() {
    let (_dir, _backend, mut state) = setup();

    reducer::reduce(
        &mut state,
        Action::Nav(NavAction::DrillIn {
            view: ViewKind::TeamDetail,
            context: "alpha-team".to_string(),
        }),
    );

    let crumbs = state.nav.breadcrumbs();
    assert_eq!(crumbs.len(), 2);
    assert!(crumbs[1].contains("alpha-team"));
}

#[test]
fn test_confirm_cancel() {
    let (_dir, _backend, mut state) = setup();
    let initial_teams = state.store.teams.len();

    reducer::reduce(
        &mut state,
        Action::Ui(UiAction::ShowConfirm {
            message: "Delete?".to_string(),
            on_confirm: Box::new(Action::Team(
                clash::application::actions::TeamAction::Delete {
                    name: "alpha-team".to_string(),
                },
            )),
        }),
    );

    reducer::reduce(&mut state, Action::Ui(UiAction::ConfirmNo));
    assert!(state.confirm_message.is_none());
    assert_eq!(state.store.teams.len(), initial_teams);
}

#[test]
fn test_quit_produces_effect() {
    let (_dir, _backend, mut state) = setup();
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::Quit));
    assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
}
