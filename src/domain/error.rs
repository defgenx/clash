//! Domain error type — used by port interfaces.
//!
//! This keeps the domain layer independent of infrastructure error types.
//! Infrastructure errors are converted via `From` impls at the boundary.

use std::fmt;

/// A domain-level error that port implementations return.
#[derive(Debug)]
#[allow(dead_code)]
pub enum DomainError {
    /// IO error (file not found, permission denied, etc.)
    Io(std::io::Error),
    /// Data parsing error.
    Parse(String),
    /// Generic error with a message.
    Other(String),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Parse(msg) => write!(f, "Parse error: {}", msg),
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for DomainError {}

impl From<std::io::Error> for DomainError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for DomainError {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DomainError>;
