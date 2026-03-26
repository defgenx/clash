mod helpers;

use clash::adapters::views::ViewKind;
use clash::application::actions::{Action, NavAction, TableAction, TaskAction, UiAction};
use clash::application::effects::Effect;
use clash::application::reducer;
use clash::application::state::{AppState, InputMode, PickerAction, PickerItem};
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
    assert!(state.confirm_dialog.is_none());
    assert_eq!(state.store.teams.len(), initial_teams);
}

#[test]
fn test_quit_shows_confirm_dialog() {
    let (_dir, _backend, mut state) = setup();
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::Quit));
    assert!(effects.is_empty()); // Shows confirm dialog, no immediate quit
    assert!(state.confirm_dialog.is_some());
}

#[test]
fn test_quit_confirmed_immediate_when_no_running_sessions() {
    let (_dir, _backend, mut state) = setup();
    // No running sessions in the default test fixtures
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::QuitConfirmed));
    assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
    assert!(state.shutting_down.is_none());
}

#[test]
fn test_quit_confirmed_graceful_when_sessions_running() {
    let (_dir, _backend, mut state) = setup();
    state.store.sessions = vec![
        clash::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        },
        clash::domain::entities::Session {
            id: "s2".to_string(),
            is_running: true,
            ..Default::default()
        },
    ];
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::QuitConfirmed));
    assert!(state.shutting_down.is_some());
    assert!(state.spinner.is_some());
    assert!(state.confirm_dialog.is_none());
    assert!(effects.iter().any(|e| matches!(e, Effect::DaemonKillAll)));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::TerminateAllProcesses)));
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::MarkAllSessionsIdle)));
    assert!(!effects.iter().any(|e| matches!(e, Effect::Quit)));
}

#[test]
fn test_shutdown_tick_quits_when_all_dead() {
    let (_dir, _backend, mut state) = setup();
    state.shutting_down = Some(0);
    state.tick = 50;
    // No running sessions → should quit
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::Tick));
    assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
}

#[test]
fn test_shutdown_tick_timeout() {
    let (_dir, _backend, mut state) = setup();
    state.shutting_down = Some(0);
    state.tick = 1499; // Will be incremented to 1500 by Tick
    state.store.sessions = vec![clash::domain::entities::Session {
        id: "s1".to_string(),
        is_running: true,
        ..Default::default()
    }];
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::Tick));
    assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
}

#[test]
fn test_shutdown_tick_updates_spinner() {
    let (_dir, _backend, mut state) = setup();
    state.shutting_down = Some(0);
    state.tick = 99; // Will be incremented to 100 (multiple of 100)
    state.store.sessions = vec![
        clash::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        },
        clash::domain::entities::Session {
            id: "s2".to_string(),
            is_running: true,
            ..Default::default()
        },
    ];
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::Tick));
    assert!(effects.is_empty()); // Not quitting yet
    assert_eq!(state.spinner.as_deref(), Some("Stashing 2 sessions..."));
}

#[test]
fn test_force_quit() {
    let (_dir, _backend, mut state) = setup();
    state.shutting_down = Some(0); // Even during shutdown
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::ForceQuit));
    assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
}

#[test]
fn test_quit_dialog_message_no_running() {
    let (_dir, _backend, mut state) = setup();
    // No running sessions
    reducer::reduce(&mut state, Action::Ui(UiAction::Quit));
    let dialog = state.confirm_dialog.as_ref().unwrap();
    assert_eq!(dialog.message, "Are you sure you want to quit?");
}

#[test]
fn test_quit_dialog_message_one_running() {
    let (_dir, _backend, mut state) = setup();
    state.store.sessions = vec![clash::domain::entities::Session {
        id: "s1".to_string(),
        is_running: true,
        ..Default::default()
    }];
    reducer::reduce(&mut state, Action::Ui(UiAction::Quit));
    let dialog = state.confirm_dialog.as_ref().unwrap();
    assert_eq!(dialog.message, "Quit? 1 running session will be stashed.");
}

#[test]
fn test_quit_dialog_message_multiple_running() {
    let (_dir, _backend, mut state) = setup();
    state.store.sessions = vec![
        clash::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        },
        clash::domain::entities::Session {
            id: "s2".to_string(),
            is_running: true,
            ..Default::default()
        },
        clash::domain::entities::Session {
            id: "s3".to_string(),
            is_running: true,
            ..Default::default()
        },
    ];
    reducer::reduce(&mut state, Action::Ui(UiAction::Quit));
    let dialog = state.confirm_dialog.as_ref().unwrap();
    assert_eq!(dialog.message, "Quit? 3 running sessions will be stashed.");
}

#[test]
fn test_graceful_shutdown_full_flow() {
    let (_dir, _backend, mut state) = setup();

    // 1. Set up 2 running sessions
    state.store.sessions = vec![
        clash::domain::entities::Session {
            id: "s1".to_string(),
            is_running: true,
            ..Default::default()
        },
        clash::domain::entities::Session {
            id: "s2".to_string(),
            is_running: true,
            ..Default::default()
        },
    ];

    // 2. Quit → confirm dialog
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::Quit));
    assert!(effects.is_empty());
    assert!(state.confirm_dialog.is_some());
    assert_eq!(state.input_mode, InputMode::Confirm);

    // 3. Confirm → graceful shutdown starts
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::ConfirmYes));
    assert!(state.shutting_down.is_some());
    assert!(state.spinner.is_some());
    assert!(effects.iter().any(|e| matches!(e, Effect::DaemonKillAll)));
    assert!(!effects.iter().any(|e| matches!(e, Effect::Quit)));

    // 4. Simulate sessions dying
    for s in &mut state.store.sessions {
        s.is_running = false;
    }

    // 5. Next tick detects all dead → quits
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::Tick));
    assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
}

#[test]
fn test_open_in_ide_picker_cycle() {
    let (_dir, _backend, mut state) = setup();

    // ShowPicker with multiple items → sets picker dialog
    let items = vec![
        PickerItem {
            label: "VS Code".to_string(),
            description: "Visual Studio Code".to_string(),
            value: "code".to_string(),
        },
        PickerItem {
            label: "Neovim".to_string(),
            description: "Terminal editor".to_string(),
            value: "terminal:nvim".to_string(),
        },
    ];
    let effects = reducer::reduce(
        &mut state,
        Action::Ui(UiAction::ShowPicker {
            title: "Open in IDE".to_string(),
            items,
            on_select: PickerAction::OpenInIde {
                project_dir: "/tmp/project".to_string(),
            },
        }),
    );
    assert!(effects.is_empty());
    assert!(state.picker_dialog.is_some());
    assert!(matches!(state.input_mode, InputMode::Picker));
    assert_eq!(state.picker_dialog.as_ref().unwrap().selected, 0);

    // PickerDown → moves selection to 1
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::PickerDown));
    assert!(effects.is_empty());
    assert_eq!(state.picker_dialog.as_ref().unwrap().selected, 1);

    // PickerSelect → emits OpenIde with terminal flag (nvim is terminal:)
    let effects = reducer::reduce(&mut state, Action::Ui(UiAction::PickerSelect));
    assert!(state.picker_dialog.is_none());
    assert!(matches!(state.input_mode, InputMode::Normal));
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::OpenIde {
            command,
            project_dir,
            terminal: true,
        } if command == "nvim" && project_dir == "/tmp/project"
    )));
}
