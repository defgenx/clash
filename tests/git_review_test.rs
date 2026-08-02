//! Integration tests for the review-only checkout plumbing, against real git
//! repositories in a temp dir.
//!
//! The parsers in `infrastructure::git::review` are unit-tested pure; what needs
//! a real repo is the orchestration: which branch ends up checked out where, and
//! whether an existing checkout is reused instead of duplicated.

use std::path::Path;
use std::process::Command;

use clash::infrastructure::git::review::{checkout_for_review, list_local_branches};
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn head_branch(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo on `main` with one extra branch `feat/x` carrying a commit.
fn repo_with_feature_branch() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    // Nested so the sibling `<repo>-worktrees` dir lands inside the temp dir.
    let repo = tmp.path().join("proj");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@t.t"]);
    git(&repo, &["config", "user.name", "T"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);

    git(&repo, &["checkout", "-q", "-b", "feat/x"]);
    std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
    git(&repo, &["commit", "-qam", "feature work"]);
    git(&repo, &["checkout", "-q", "main"]);
    (tmp, repo)
}

#[tokio::test]
async fn checkout_creates_a_flat_worktree_for_a_local_branch() {
    let (_tmp, repo) = repo_with_feature_branch();

    let dir = checkout_for_review(&repo, "feat/x", None).await.unwrap();
    let path = Path::new(&dir);
    assert!(path.is_dir(), "{} should exist", dir);
    // Slashed branch → flat directory name, alongside the repo.
    assert!(
        dir.ends_with("proj-worktrees/feat-x"),
        "unexpected worktree path: {}",
        dir
    );
    assert_eq!(head_branch(path), "feat/x");
    // The reviewed content is there, and the main checkout is untouched.
    assert_eq!(
        std::fs::read_to_string(path.join("a.txt")).unwrap(),
        "one\ntwo\n"
    );
    assert_eq!(head_branch(&repo), "main");
}

#[tokio::test]
async fn checkout_reuses_an_existing_checkout_instead_of_duplicating_it() {
    let (_tmp, repo) = repo_with_feature_branch();

    // First call creates the worktree; the second must return the same dir —
    // `git worktree add` on an already-checked-out branch would fail.
    let first = checkout_for_review(&repo, "feat/x", None).await.unwrap();
    let second = checkout_for_review(&repo, "feat/x", None).await.unwrap();
    assert_eq!(first, second);

    // The branch checked out in the main repo resolves to the repo itself, so
    // reviewing what you already have open costs no worktree at all. The path
    // comes back canonical whichever way it was resolved (macOS reports
    // /private/var for a /var temp dir).
    let main_dir = checkout_for_review(&repo, "main", None).await.unwrap();
    assert_eq!(
        Path::new(&main_dir),
        std::fs::canonicalize(&repo).unwrap().as_path()
    );
    assert_eq!(
        Path::new(&first),
        std::fs::canonicalize(&first).unwrap().as_path(),
        "created worktree path must already be canonical"
    );
}

#[tokio::test]
async fn checkout_rejects_a_branch_that_exists_nowhere() {
    let (_tmp, repo) = repo_with_feature_branch();
    // No remote is configured, so the fetch fallback cannot invent it: the
    // review must fail before any item is created.
    let err = checkout_for_review(&repo, "no-such-branch", None)
        .await
        .unwrap_err();
    assert!(
        err.contains("no-such-branch"),
        "error should name the branch: {}",
        err
    );
    assert!(checkout_for_review(&repo, "  ", None).await.is_err());
}

#[tokio::test]
async fn branches_are_listed_newest_first_with_their_checkouts() {
    let (_tmp, repo) = repo_with_feature_branch();
    let worktree = checkout_for_review(&repo, "feat/x", None).await.unwrap();

    let branches = list_local_branches(&repo).await.unwrap();
    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    assert!(
        names.contains(&"main") && names.contains(&"feat/x"),
        "{:?}",
        names
    );
    // feat/x has the newer commit.
    assert_eq!(names[0], "feat/x");

    let feat = branches.iter().find(|b| b.name == "feat/x").unwrap();
    assert_eq!(feat.worktree.as_deref(), Some(worktree.as_str()));
    assert!(
        !feat.last_commit.is_empty(),
        "relative date is the detail line"
    );
    // main is checked out in the repo itself, not a worktree.
    let main = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main.worktree.is_some());
}
