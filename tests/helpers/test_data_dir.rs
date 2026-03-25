use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Helper that creates a temp dir populated from fixtures.
pub struct TestDataDir {
    _temp: TempDir,
    pub path: PathBuf,
}

impl TestDataDir {
    /// Create a test data dir by copying fixtures into a temp dir.
    pub fn new() -> Self {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

        // Copy teams
        let teams_src = fixtures.join("teams");
        if teams_src.exists() {
            copy_dir_recursive(&teams_src, &temp.path().join("teams"));
        }

        // Copy tasks
        let tasks_src = fixtures.join("tasks");
        if tasks_src.exists() {
            copy_dir_recursive(&tasks_src, &temp.path().join("tasks"));
        }

        Self {
            path: temp.path().to_path_buf(),
            _temp: temp,
        }
    }

    /// Create an empty test data dir (no fixtures).
    #[allow(dead_code)]
    pub fn empty() -> Self {
        let temp = TempDir::new().expect("Failed to create temp dir");
        Self {
            path: temp.path().to_path_buf(),
            _temp: temp,
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("Failed to create dir");
    for entry in std::fs::read_dir(src).expect("Failed to read dir") {
        let entry = entry.expect("Failed to read entry");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            std::fs::copy(&src_path, &dst_path).expect("Failed to copy file");
        }
    }
}
