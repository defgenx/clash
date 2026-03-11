pub mod agent;
pub mod navigation;
pub mod table;
pub mod task;
pub mod team;
pub mod ui;

pub use agent::AgentAction;
pub use navigation::NavAction;
pub use table::TableAction;
pub use task::TaskAction;
pub use team::TeamAction;
pub use ui::UiAction;

/// Top-level action enum with nested domain actions.
#[derive(Debug, Clone)]
pub enum Action {
    Nav(NavAction),
    Table(TableAction),
    Team(TeamAction),
    Task(TaskAction),
    Agent(AgentAction),
    Ui(UiAction),
    /// No-op, used when an event doesn't map to an action.
    Noop,
    /// Result from a completed CLI call.
    CliResult {
        success: bool,
        output: String,
        follow_up: Box<Action>,
    },
}
