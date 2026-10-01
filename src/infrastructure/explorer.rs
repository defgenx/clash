//! IO half of the file explorer: directory listings, `git status`, the file
//! index the fuzzy finder searches, and file previews. Every decision is in
//! `application::explorer`; these wrappers only read.
//!
//! All of it is blocking (`std::fs`, `std::process`). The GUI runs it on the
//! blocking pool: the daemon shares the async runtime, and a `git status` on
//! a large repository holding a worker would starve it.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::Engine;

use crate::application::explorer::{self, ExplorerEntry, FileMatch, GitSummary, PreviewKind};

/// Cap on the fuzzy finder's index. A checkout past this is searched only in
/// its first `MAX_INDEXED_FILES` paths; the panel says so.
pub const MAX_INDEXED_FILES: usize = 200_000;

fn resolve(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let sub = explorer::safe_rel(rel).ok_or_else(|| format!("invalid path: {rel}"))?;
    Ok(root.join(sub))
}

/// Children of `rel` under `root`, sorted. Ignored entries are flagged, and
/// dropped unless `show_ignored`.
pub fn list_dir(root: &Path, rel: &str, show_ignored: bool) -> Result<Vec<ExplorerEntry>, String> {
    let dir = resolve(root, rel)?;
    let read = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut entries = Vec::new();
    for item in read.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if explorer::always_hidden(&name) {
            continue;
        }
        let is_symlink = item.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        // Follow links for the kind, so a linked folder expands like a folder.
        let meta = std::fs::metadata(item.path()).ok();
        entries.push(ExplorerEntry {
            rel: explorer::join_rel(rel, &name),
            name,
            is_dir: meta.as_ref().is_some_and(|m| m.is_dir()),
            is_symlink,
            ignored: false,
            size: meta.map(|m| m.len()).unwrap_or(0),
        });
    }
    let rels: Vec<&str> = entries.iter().map(|e| e.rel.as_str()).collect();
    let ignored = ignored_among(root, &rels);
    for e in &mut entries {
        e.ignored = ignored.contains(&e.rel);
    }
    if !show_ignored {
        entries.retain(|e| !e.ignored);
    }
    explorer::sort_entries(&mut entries);
    Ok(entries)
}

/// Which of `rels` git ignores, in one `git check-ignore --stdin` call.
/// Outside a repository (or without git) nothing is ignored.
fn ignored_among(root: &Path, rels: &[&str]) -> HashSet<String> {
    if rels.is_empty() {
        return HashSet::new();
    }
    let child = Command::new("git")
        .args(["check-ignore", "-z", "--stdin"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return HashSet::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        let input: Vec<u8> = rels
            .iter()
            .flat_map(|r| [r.as_bytes(), b"\0"].concat())
            .collect();
        let _ = stdin.write_all(&input);
    }
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut out);
    }
    let _ = child.wait();
    String::from_utf8_lossy(&out)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn git_output(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// Branch, ahead/behind and per-path status under `root`; `None` outside a
/// repository.
pub fn git_status(root: &Path) -> Option<GitSummary> {
    let prefix = git_output(root, &["rev-parse", "--show-prefix"])?;
    let prefix = String::from_utf8_lossy(&prefix).trim().to_string();
    let out = git_output(root, &["status", "--porcelain=v1", "-z", "-b"])?;
    Some(explorer::parse_status(&out, &prefix))
}

/// Every file under `root` the finder can search: git's view (tracked plus
/// untracked-but-not-ignored) inside a repository, otherwise a bounded walk
/// that skips dot-folders and dependency/build trees. The flag says the cap
/// was hit.
pub fn list_files(root: &Path) -> (Vec<String>, bool) {
    if let Some(out) = git_output(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    ) {
        let mut files: Vec<String> = String::from_utf8_lossy(&out)
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        // A file both staged-deleted and present lists twice; and --cached
        // lists files deleted from the work tree, which cannot be opened.
        files.sort_unstable();
        files.dedup();
        files.retain(|f| root.join(f).exists());
        let capped = files.len() > MAX_INDEXED_FILES;
        files.truncate(MAX_INDEXED_FILES);
        return (files, capped);
    }
    let mut files = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in read.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let child_rel = explorer::join_rel(&rel, &name);
            match item.file_type() {
                Ok(t) if t.is_dir() => {
                    if !explorer::skip_in_walk(&name) {
                        stack.push((item.path(), child_rel));
                    }
                }
                Ok(_) => files.push(child_rel),
                Err(_) => {}
            }
            if files.len() >= MAX_INDEXED_FILES {
                return (files, true);
            }
        }
    }
    (files, false)
}

/// The finder: fuzzy-rank every indexed file under `root` against `query`.
pub fn find_files(root: &Path, query: &str, limit: usize) -> (Vec<FileMatch>, bool) {
    let (files, capped) = list_files(root);
    (
        explorer::rank_files(query, files.iter().map(String::as_str), limit),
        capped,
    )
}

/// What the preview tab renders. Text comes as text, images as base64 with
/// their MIME type; the other kinds carry only the size.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub kind: PreviewKind,
    pub path: String,
    pub size: u64,
    pub text: Option<String>,
    pub base64: Option<String>,
    pub mime: Option<String>,
}

pub fn read_preview(root: &Path, rel: &str) -> Result<FilePreview, String> {
    let path = resolve(root, rel)?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_dir() {
        return Err(format!("{} is a folder", path.display()));
    }
    let size = meta.len();
    let mut sample = vec![0u8; 8192];
    let n = std::fs::File::open(&path)
        .and_then(|mut f| f.read(&mut sample))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    sample.truncate(n);
    let kind = explorer::classify(rel, size, &sample);
    let mut preview = FilePreview {
        kind,
        path: path.to_string_lossy().into_owned(),
        size,
        text: None,
        base64: None,
        mime: None,
    };
    match kind {
        PreviewKind::Text | PreviewKind::Markdown => {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            preview.text = Some(String::from_utf8_lossy(&bytes).into_owned());
        }
        PreviewKind::Image => {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            preview.base64 = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
            preview.mime = explorer::image_mime(rel).map(str::to_string);
        }
        PreviewKind::Binary | PreviewKind::TooLarge => {}
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_sorts_hides_git_and_rejects_escapes() {
        let dir = std::env::temp_dir().join(format!("clash-explorer-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        std::fs::write(dir.join("sub/a.txt"), "a").unwrap();

        let top = list_dir(&dir, "", true).unwrap();
        let names: Vec<_> = top.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["sub", "b.txt"]);
        let sub = list_dir(&dir, "sub", true).unwrap();
        assert_eq!(sub[0].rel, "sub/a.txt");
        assert!(list_dir(&dir, "../", true).is_err());

        let p = read_preview(&dir, "b.txt").unwrap();
        assert_eq!(p.kind, PreviewKind::Text);
        assert_eq!(p.text.as_deref(), Some("b"));
        assert!(read_preview(&dir, "sub").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_repository_hides_ignored_entries_and_reports_status() {
        let dir = std::env::temp_dir().join(format!("clash-explorer-git-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(args)
                .current_dir(&dir)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.join("src/app.rs"), "fn a() {}").unwrap();
        std::fs::write(dir.join("target/out.bin"), "x").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        std::fs::write(dir.join("src/app.rs"), "fn b() {}").unwrap();
        std::fs::write(dir.join("src/new.rs"), "").unwrap();

        let names = |show| -> Vec<String> {
            list_dir(&dir, "", show)
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect()
        };
        assert_eq!(names(false), ["src", ".gitignore"]);
        assert_eq!(names(true), ["src", "target", ".gitignore"]);
        assert!(list_dir(&dir, "", true).unwrap()[1].ignored);

        let st = git_status(&dir.join("src")).unwrap();
        assert_eq!(st.branch.as_deref(), Some("main"));
        let marks: Vec<_> = st
            .entries
            .iter()
            .map(|e| (e.rel.as_str(), e.mark))
            .collect();
        assert_eq!(
            marks,
            [
                ("app.rs", explorer::GitMark::Modified),
                ("new.rs", explorer::GitMark::Untracked)
            ]
        );

        let (files, capped) = list_files(&dir);
        assert!(!capped);
        assert_eq!(files, [".gitignore", "src/app.rs", "src/new.rs"]);
        let (found, _) = find_files(&dir, "app", 10);
        assert_eq!(found[0].rel, "src/app.rs");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
