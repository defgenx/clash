//! Files dropped onto a GUI terminal.
//!
//! The window runs with `dragDropEnabled: false` (in-page drag-and-drop needs
//! it), so a Finder drop reaches the page as a DOM `File` with bytes but no
//! path. The GUI hands the bytes here; they are stored under
//! `<clash data>/drops/` and the path is typed into the session, which is
//! what a terminal does with a drop.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Largest file accepted. The bytes cross the IPC boundary as base64.
pub const MAX_DROP_BYTES: usize = 64 * 1024 * 1024;

/// How long a stored drop is kept: long enough for a prompt written slowly.
const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 3600);

/// The stored name: `<stamp>-<name>` with every character outside
/// `[A-Za-z0-9._-]` replaced by `_`, so the typed path never needs quoting
/// and the agent's image-path detection always matches. The extension is
/// kept because that detection keys on it.
pub fn dropped_file_name(original: &str, stamp: u128) -> String {
    let base = Path::new(original)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let mut clean: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean_trimmed = clean.trim_start_matches('.');
    if clean_trimmed.is_empty() {
        clean = "file".to_string();
    } else {
        clean = clean_trimmed.to_string();
    }
    if clean.len() > 120 {
        let ext = Path::new(&clean)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        let keep = 120usize.saturating_sub(ext.len());
        clean = format!("{}{ext}", &clean[..keep]);
    }
    format!("{stamp}-{clean}")
}

/// Store `bytes` under `dir` and return the absolute path. Drops older than
/// [`KEEP_FOR`] are removed on the way (best-effort).
pub fn save_dropped_file(dir: &Path, original: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    if bytes.len() > MAX_DROP_BYTES {
        return Err(std::io::Error::other(format!(
            "file is larger than {} MB",
            MAX_DROP_BYTES / (1024 * 1024)
        )));
    }
    std::fs::create_dir_all(dir)?;
    prune(dir, SystemTime::now());
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = dir.join(dropped_file_name(original, stamp));
    crate::infrastructure::fs::atomic::write_atomic(&path, bytes)?;
    Ok(path)
}

fn prune(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > KEEP_FOR);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_plain_name_and_its_extension() {
        assert_eq!(dropped_file_name("shot.png", 7), "7-shot.png");
    }

    #[test]
    fn replaces_what_would_need_quoting() {
        assert_eq!(
            dropped_file_name("Screenshot 2026-11-03 at 10.12.04 (2).png", 1),
            "1-Screenshot_2026-11-03_at_10.12.04__2_.png"
        );
        assert_eq!(dropped_file_name("café'$.jpg", 1), "1-caf___.jpg");
    }

    #[test]
    fn never_escapes_the_directory() {
        assert_eq!(dropped_file_name("../../etc/passwd", 1), "1-passwd");
        assert_eq!(dropped_file_name("..", 1), "1-file");
        assert_eq!(dropped_file_name("", 1), "1-file");
        assert_eq!(dropped_file_name(".hidden", 1), "1-hidden");
    }

    #[test]
    fn a_long_name_keeps_its_extension() {
        let name = format!("{}.png", "a".repeat(300));
        let out = dropped_file_name(&name, 1);
        assert!(out.ends_with(".png"));
        assert_eq!(out.len(), "1-".len() + 120);
    }

    #[test]
    fn saves_and_refuses_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = save_dropped_file(dir.path(), "a b.png", b"x").unwrap();
        assert!(p.starts_with(dir.path()));
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
        assert!(p
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with("-a_b.png"));
        let big = vec![0u8; MAX_DROP_BYTES + 1];
        assert!(save_dropped_file(dir.path(), "big.bin", &big).is_err());
    }
}
