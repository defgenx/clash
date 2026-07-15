use crate::domain::entities::TaskStatus;

#[derive(Debug, Clone)]
pub enum TaskAction {
    Create {
        team: String,
        subject: String,
        description: String,
    },
    UpdateStatus {
        team: String,
        task_id: String,
        status: TaskStatus,
    },
    CycleStatus {
        team: String,
        task_id: String,
    },
    /// Assign (or clear, with an empty string) a task's owner.
    SetOwner {
        team: String,
        task_id: String,
        owner: String,
    },
    /// Delete a task from a team.
    Delete {
        team: String,
        task_id: String,
    },
}
