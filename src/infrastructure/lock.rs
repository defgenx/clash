//! Single-instance lock — prevents multiple clash TUI instances from running simultaneously.
//!
//! Uses `flock(2)` via the `nix` crate's `Flock` wrapper. The kernel automatically
//! releases the lock when the process exits (even on SIGKILL), so there are no
//! stale lock files to clean up.

use std::fs::File;
use std::path::{Path, PathBuf};

use nix::fcntl::{Flock, FlockArg};

/// Holds an exclusive `flock(2)` lock for the lifetime of the clash TUI.
///
/// Drop releases the lock automatically (RAII via `Flock<File>::drop()`).
#[derive(Debug)]
pub struct SingleInstanceLock {
    _flock: Flock<File>,
}

impl SingleInstanceLock {
    /// Acquire the instance lock at the default path (`~/.local/share/clash/clash.lock`).
    pub fn acquire() -> Result<Self, String> {
        let path = default_lock_path()
            .ok_or_else(|| "Could not determine clash data directory".to_string())?;
        Self::acquire_at(&path)
    }

    /// Acquire the instance lock at an explicit path (useful for testing).
    pub fn acquire_at(path: &Path) -> Result<Self, String> {
        // Ensure parent directories exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create lock directory: {}", e))?;
        }

        let file = File::create(path).map_err(|e| format!("Failed to create lock file: {}", e))?;

        let flock = Flock::lock(file, FlockArg::LockExclusiveNonblock)
            .map_err(|(_file, _errno)| "Another clash instance is already running.".to_string())?;

        Ok(SingleInstanceLock { _flock: flock })
    }
}

/// Default lock file path: `~/.local/share/clash/clash.lock`.
fn default_lock_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("clash").join("clash.lock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_acquire_succeeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("clash.lock");
        let lock = SingleInstanceLock::acquire_at(&path);
        assert!(lock.is_ok());
    }

    #[test]
    fn test_lock_prevents_second_instance() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("clash.lock");
        let _lock1 = SingleInstanceLock::acquire_at(&path).unwrap();
        let lock2 = SingleInstanceLock::acquire_at(&path);
        assert!(lock2.is_err());
        assert!(lock2.unwrap_err().contains("already running"));
    }

    #[test]
    fn test_lock_released_on_drop() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("clash.lock");
        {
            let _lock = SingleInstanceLock::acquire_at(&path).unwrap();
        }
        // After drop, re-acquire should succeed
        let lock2 = SingleInstanceLock::acquire_at(&path);
        assert!(lock2.is_ok());
    }

    #[test]
    fn test_lock_creates_parent_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nested").join("deep").join("clash.lock");
        let lock = SingleInstanceLock::acquire_at(&path);
        assert!(lock.is_ok());
        assert!(path.exists());
    }
}
