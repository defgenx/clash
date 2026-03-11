#[derive(Debug, Clone)]
pub enum TeamAction {
    Create { name: String, description: String },
    Delete { name: String },
    Refresh,
}
