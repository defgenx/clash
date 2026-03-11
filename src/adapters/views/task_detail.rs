use crate::application::state::AppState;
use crate::adapters::views::{DetailView, Keybinding, Section};

pub struct TaskDetailView;

impl DetailView for TaskDetailView {
    fn title(state: &AppState) -> String {
        if let Some(task_id) = state.nav.current().context.as_deref() {
            format!("Task: {}", task_id)
        } else {
            "Task".to_string()
        }
    }

    fn sections(state: &AppState) -> Vec<Section> {
        let team_name = match state.current_team() {
            Some(n) => n,
            None => return vec![],
        };
        let task_id = match state.nav.current().context.as_deref() {
            Some(id) => id,
            None => return vec![],
        };
        let task = match state.store.find_task(team_name, task_id) {
            Some(t) => t,
            None => return vec![Section::new("Error").row("", "Task not found")],
        };

        let info = Section::new("Info")
            .row("ID", &task.id)
            .row("Subject", &task.subject)
            .row("Status", task.status.as_str())
            .row("Owner", task.owner.as_deref().unwrap_or("—"));

        let mut details = Section::new("Details")
            .row("Description", &task.description);

        if let Some(ref form) = task.active_form {
            details = details.row("Active Form", form);
        }

        let mut deps = Section::new("Dependencies");
        if !task.blocks.is_empty() {
            deps = deps.row("Blocks", &task.blocks.join(", "));
        }
        if !task.blocked_by.is_empty() {
            deps = deps.row("Blocked By", &task.blocked_by.join(", "));
        }
        if task.blocks.is_empty() && task.blocked_by.is_empty() {
            deps = deps.row("", "No dependencies");
        }

        vec![info, details, deps]
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("s", "Cycle status"),
            Keybinding::new("a", "Assign owner"),
            Keybinding::new("e", "Edit"),
            Keybinding::new("d", "Delete"),
        ]
    }
}
