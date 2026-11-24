//! Worktree cleanup — the IO half of `application::worktree_cleanup`: find the
//! repositories under a root, ask git for their worktrees, read the facts and
//! the guards, remove. Blocking (git subprocesses, each waited for): the GUI
//! runs it on the blocking pool. Contract: `docs/workflows.md` → *Worktree
//! cleanup*.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::application::worktree_cleanup::{self as pure, Facts, Guards, Verdict, Worktree};

/// One worktree with its verdict, for a preview or a run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// The repository's own checkout, which runs the git commands.
    pub repo: String,
    pub path: String,
    pub branch: Option<String>,
    pub removable: bool,
    /// Why it stays, or what removing it means.
    pub reason: String,
    /// Disk use in bytes, when measured.
    pub bytes: Option<u64>,
    /// Kept only while a session works in it ([`Verdict::waits`]).
    #[serde(skip)]
    pub waits: bool,
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repositories `root` holds: `root` itself when it is one, and each
/// direct child whose `.git` is a directory (a worktree's `.git` is a file,
/// so worktrees are reached through their repository, never on their own).
pub fn repos_in(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.join(".git").is_dir() {
        out.push(root.to_path_buf());
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut children: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(".git").is_dir())
            .collect();
        children.sort();
        out.extend(children);
    }
    out
}

pub fn worktrees_of(repo: &Path) -> Vec<Worktree> {
    git(repo, &["worktree", "list", "--porcelain"])
        .map(|s| pure::parse_porcelain(&s))
        .unwrap_or_default()
}

pub fn facts(path: &Path) -> Facts {
    // Explicit: `status.showUntrackedFiles=no` would otherwise report a
    // worktree holding untracked files as clean.
    let dirty = git(path, &["status", "--porcelain", "--untracked-files=normal"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count());
    let unpushed = git(path, &["rev-list", "--count", "HEAD", "--not", "--remotes"])
        .and_then(|s| s.trim().parse().ok());
    Facts { dirty, unpushed }
}

/// `p` as git reports it: symlinks resolved (`/tmp` → `/private/tmp`). A
/// gone folder resolves through its parent; an unresolvable one is kept as
/// spelled, minus a trailing slash.
pub fn resolved(p: &str) -> String {
    let path = Path::new(p.trim_end_matches('/'));
    if let Ok(c) = std::fs::canonicalize(path) {
        return c.to_string_lossy().into_owned();
    }
    match (path.parent().map(std::fs::canonicalize), path.file_name()) {
        (Some(Ok(parent)), Some(name)) => parent.join(name).to_string_lossy().into_owned(),
        _ => path.to_string_lossy().into_owned(),
    }
}

/// Disk use of a folder, via `du -sk` (fast, and what a person would check).
pub fn bytes(path: &Path) -> Option<u64> {
    let out = Command::new("du")
        .arg("-sk")
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|kb| kb * 1024)
}

/// Guards from the open workflow items under `workflows_root` (every status
/// but done and abandoned), plus the working directories of running agents:
/// `sessions` from the caller, and every live agent process the process scan
/// sees (both frontends, and sessions started outside clash). Paths are
/// [`resolved`], so they compare with git's spelling.
pub fn guards(workflows_root: &Path, sessions: Vec<(String, String)>) -> Guards {
    use crate::domain::workflow::{WorkflowMeta, WorkflowStatus};
    let mut g = Guards::default();
    for project in std::fs::read_dir(workflows_root)
        .into_iter()
        .flatten()
        .flatten()
    {
        for item in std::fs::read_dir(project.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let Ok(raw) = std::fs::read_to_string(item.path().join("meta.json")) else {
                continue;
            };
            let Ok(meta) = serde_json::from_str::<WorkflowMeta>(&raw) else {
                continue;
            };
            if matches!(
                meta.status,
                WorkflowStatus::Done | WorkflowStatus::Abandoned
            ) {
                continue;
            }
            let key = format!(
                "{}/{}",
                project.file_name().to_string_lossy(),
                item.file_name().to_string_lossy()
            );
            let slug = item.file_name().to_string_lossy().into_owned();
            let branches = pure::item_branches(&meta, slug);
            let recorded = meta.worktree.as_deref().map(resolved);
            g.items.push((key, recorded, branches));
        }
    }
    let wild = crate::infrastructure::process_scan::gather_wild_processes(
        &crate::infrastructure::process_scan::default_fd_probe(),
    );
    let found = wild
        .into_iter()
        .filter_map(|p| p.cwd.map(|cwd| (format!("pid {}", p.pid), cwd)));
    g.sessions = sessions
        .into_iter()
        .chain(found)
        .map(|(label, cwd)| (label, resolved(&cwd)))
        .collect();
    g
}

fn candidate(repo: &Path, w: &Worktree, guards: &Guards, measure: bool) -> Candidate {
    let f = if w.prunable || w.main {
        Facts::default()
    } else {
        facts(Path::new(&w.path))
    };
    let v = pure::verdict(w, f, guards);
    Candidate {
        repo: repo.to_string_lossy().into_owned(),
        path: w.path.clone(),
        branch: w.branch.clone(),
        removable: v.removable(),
        reason: v.reason(),
        waits: v.waits(),
        bytes: (measure && !w.prunable)
            .then(|| bytes(Path::new(&w.path)))
            .flatten(),
    }
}

/// Every worktree of every repository under `root`, the main checkouts left
/// out, each with its verdict.
pub fn scan(root: &Path, guards: &Guards, measure: bool) -> Vec<Candidate> {
    let mut out = Vec::new();
    for repo in repos_in(root) {
        for w in worktrees_of(&repo).iter().filter(|w| !w.main) {
            out.push(candidate(&repo, w, guards, measure));
        }
    }
    out
}

/// The worktrees at `(repo, path)` pairs (an item's own), with their
/// verdicts. A pair git no longer lists is skipped: nothing to clean.
/// `dead_code` allowed: only the `clash-gui` crate cleans up a done item.
#[allow(dead_code)]
pub fn scan_pairs(pairs: &[(String, String)], guards: &Guards) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (repo, path) in pairs {
        let repo = Path::new(repo);
        let want = resolved(path);
        if let Some(w) = worktrees_of(repo)
            .iter()
            .find(|w| !w.main && resolved(&w.path) == want)
        {
            out.push(candidate(repo, w, guards, false));
        }
    }
    out
}

/// Drop git's registration of a worktree whose folder is gone — that one
/// only: `git worktree prune` would drop every dead registration of the
/// repository, an open item's among them.
pub fn forget(repo: &Path, path: &str) -> Result<(), String> {
    worktree_remove(repo, &["--force"], path)
}

fn worktree_remove(repo: &Path, flags: &[&str], path: &str) -> Result<(), String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        // A config hiding untracked files would let `remove` delete them.
        .args([
            "-c",
            "status.showUntrackedFiles=normal",
            "worktree",
            "remove",
        ])
        .args(flags)
        .arg("--")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Remove one worktree — re-checked now, since anything may have changed
/// since the preview. A gone folder's registration is dropped; anything no
/// longer safe is refused with its reason. The branch is never touched.
pub fn remove(repo: &Path, path: &str, guards: &Guards) -> Result<(), String> {
    let Some(w) = worktrees_of(repo).into_iter().find(|w| w.path == path) else {
        return Err(format!(
            "{path} is no longer a worktree of {}",
            repo.display()
        ));
    };
    let v = pure::verdict(
        &w,
        if w.prunable {
            Facts::default()
        } else {
            facts(Path::new(path))
        },
        guards,
    );
    match v {
        Verdict::Prune => forget(repo, path),
        // Not forced: git checks again and refuses a worktree that gained
        // changes since `facts`; ignored build output does not stop it.
        Verdict::Remove => worktree_remove(repo, &[], path),
        other => Err(format!("kept: {}", other.reason())),
    }
}

/// What a cleanup run did.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub removed: Vec<String>,
    /// `(path, reason)` for each worktree left in place.
    pub kept: Vec<(String, String)>,
    /// `(path, error)` for each removal that failed.
    pub failed: Vec<(String, String)>,
    /// Something was kept only because a session works in it.
    #[serde(skip)]
    pub waiting: bool,
}

/// Remove every removable candidate; report the rest.
pub fn run(candidates: &[Candidate], guards: &Guards) -> Outcome {
    let mut o = Outcome::default();
    for c in candidates {
        if !c.removable {
            o.kept.push((c.path.clone(), c.reason.clone()));
            o.waiting |= c.waits;
            continue;
        }
        match remove(Path::new(&c.repo), &c.path, guards) {
            Ok(()) => o.removed.push(c.path.clone()),
            Err(e) if e.starts_with("kept: ") => o
                .kept
                .push((c.path.clone(), e.trim_start_matches("kept: ").to_string())),
            Err(e) => o.failed.push((c.path.clone(), e)),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    /// A real repository with a remote, a pushed worktree, a dirty one and an
    /// unpushed one: only the first goes, and the branch stays.
    #[test]
    fn a_run_removes_only_what_loses_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let (origin, ws) = (dir.path().join("origin.git"), dir.path().join("ws"));
        let repo = ws.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        sh(
            dir.path(),
            &["init", "-q", "--bare", origin.to_str().unwrap()],
        );
        sh(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a"), "a").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-qm", "a"]);
        sh(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh(&repo, &["push", "-q", "origin", "main"]);
        // A config that hides untracked files must not make them look clean.
        sh(&repo, &["config", "status.showUntrackedFiles", "no"]);
        let wt = |name: &str| ws.join("repo-worktrees").join(name);
        for name in ["pushed", "dirty", "ahead"] {
            sh(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    name,
                    wt(name).to_str().unwrap(),
                    "main",
                ],
            );
        }
        std::fs::write(wt("dirty").join("new"), "x").unwrap();
        std::fs::write(wt("ahead").join("b"), "b").unwrap();
        sh(&wt("ahead"), &["add", "."]);
        sh(&wt("ahead"), &["commit", "-qm", "b"]);
        // Ignored build output (`info/exclude` is shared by every worktree)
        // does not make a worktree dirty, and is removed with it.
        std::fs::write(repo.join(".git").join("info").join("exclude"), "target/\n").unwrap();
        std::fs::create_dir_all(wt("pushed").join("target")).unwrap();
        std::fs::write(wt("pushed").join("target").join("o"), "o").unwrap();

        let g = Guards::default();
        let cands = scan(&ws, &g, true);
        let by = |n: &str| cands.iter().find(|c| c.path.ends_with(n)).unwrap();
        assert_eq!(cands.len(), 3, "{cands:?}");
        assert!(by("pushed").removable, "{:?}", by("pushed"));
        assert!(!by("dirty").removable && by("dirty").reason.contains("uncommitted"));
        assert!(!by("ahead").removable && by("ahead").reason.contains("no remote"));
        assert!(by("pushed").bytes.is_some());

        let out = run(&cands, &g);
        assert_eq!(out.removed.len(), 1);
        assert!(!wt("pushed").exists());
        assert!(wt("dirty").exists() && wt("ahead").exists());
        assert!(
            git(&repo, &["rev-parse", "--verify", "refs/heads/pushed"]).is_some(),
            "the branch stays"
        );

        // A folder deleted by hand leaves a registration that is dropped —
        // that one only: an open item's lost worktree keeps its record.
        std::fs::remove_dir_all(wt("dirty")).unwrap();
        std::fs::remove_dir_all(wt("ahead")).unwrap();
        let g = Guards {
            items: vec![("p/ahead".into(), None, vec!["ahead".into()])],
            ..Guards::default()
        };
        let cands = scan(&ws, &g, false);
        let gone = cands.iter().find(|c| c.path.ends_with("dirty")).unwrap();
        assert!(gone.removable && gone.reason.contains("gone"), "{gone:?}");
        run(&cands, &g);
        let left = worktrees_of(&repo);
        assert!(left.iter().all(|w| !w.path.ends_with("dirty")));
        assert!(
            left.iter().any(|w| w.path.ends_with("ahead") && w.prunable),
            "the open item's registration stays: {left:?}"
        );
    }

    #[test]
    fn resolved_follows_symlinks_even_for_a_gone_folder() {
        let dir = tempfile::TempDir::new().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let canon = std::fs::canonicalize(&real).unwrap();
        let gone = format!("{}/gone/", link.display());
        assert_eq!(resolved(&gone), canon.join("gone").to_string_lossy());
        assert_eq!(resolved(link.to_str().unwrap()), canon.to_string_lossy());
    }
}
