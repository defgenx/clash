//! Shared log-file setup for the TUI and GUI binaries.
//!
//! Both write to the same `clash.log` in the app-support dir (appending —
//! multiple instances can interleave safely) so Finder/Dock launches, whose
//! stderr goes nowhere, still leave a trail. The file is rotated (deleted)
//! when older than the retention window.

use std::fs::File;
use std::path::PathBuf;

/// Default retention before the log file is rotated away, in hours.
/// Override with `CLASH_LOG_RETENTION_HOURS`.
const DEFAULT_RETENTION_HOURS: u64 = 24;

/// Path of the shared log file (`…/clash/clash.log` under the platform
/// data dir).
pub fn log_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("clash")
        .join("clash.log")
}

/// Open `clash.log` for appending, rotating it away first when stale.
/// Returns `None` when the directory or file cannot be created — callers
/// fall back to stderr-only logging.
pub fn open_log_file() -> Option<File> {
    let path = log_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }

    let retention_hours: u64 = std::env::var("CLASH_LOG_RETENTION_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RETENTION_HOURS);
    if let Ok(meta) = std::fs::metadata(&path) {
        if let Ok(modified) = meta.modified() {
            if modified.elapsed().unwrap_or_default()
                > std::time::Duration::from_secs(retention_hours * 3600)
            {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}
