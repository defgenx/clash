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
}
