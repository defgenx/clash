use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::{Task, TaskStatus};

pub struct TasksTable;

fn status_color(status: &TaskStatus) -> Color {
    match status {
        TaskStatus::Completed => Color::Green,
        TaskStatus::InProgress => Color::Yellow,
        TaskStatus::Blocked => Color::Red,
        TaskStatus::Pending => Color::DarkGray,
        TaskStatus::Unknown => Color::Magenta,
    }
}

impl TableView for TasksTable {
    type Item = Task;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("ID", 10),
            ColumnDef::new("STATUS", 15),
            ColumnDef::new("OWNER", 20),
            ColumnDef::new("SUBJECT", 55),
        ]
    }

    fn row(item: &Task, _tick: usize) -> Vec<Cell<'static>> {
        let color = status_color(&item.status);
        vec![
            Cell::from(item.id.clone()),
            Cell::from(item.status.as_str().to_string())
                .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Cell::from(item.owner.as_deref().unwrap_or("—").to_string()),
            Cell::from(item.subject.clone()),
        ]
    }

    fn items(state: &AppState) -> Vec<&Task> {
        if let Some(team) = state.current_team() {
            state.store.get_tasks(team).iter().collect()
        } else {
            Vec::new()
        }
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("c", "Create task"),
            Keybinding::new("d", "Delete task"),
            Keybinding::new("s", "Cycle status"),
            Keybinding::new("a", "Assign task"),
            Keybinding::new("Enter", "View task"),
        ]
    }

    fn empty_message() -> &'static str {
        "No tasks. Press 'c' to create one."
    }
}
