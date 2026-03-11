use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("Watcher error: {0}")]
    Watcher(#[from] notify_debouncer_full::notify::Error),
}

pub type Result<T> = std::result::Result<T, AppError>;
