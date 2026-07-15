use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic counter making each temp file name unique within the process, so
/// concurrent writes to *different* targets never collide on the same temp path.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Write data to a file atomically (temp file + rename).
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(".clash-tmp-{}-{}", std::process::id(), seq));
    std::fs::write(&temp_path, data)?;
    std::fs::rename(&temp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_atomic_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.json");
        write_atomic(&path, b"hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn test_write_atomic_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub").join("dir").join("test.json");
        write_atomic(&path, b"nested").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "nested");
    }

    #[test]
    fn test_write_atomic_overwrites() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }
}
