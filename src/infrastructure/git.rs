//! Shared `git` subprocess helpers — the single place clash shells out for
//! diff text (the TUI's `Effect::LoadDiff`, the GUI's session diff tab, the
//! workflow diff view) and for the branch/worktree plumbing that materializes
//! a checkout of code already written elsewhere (review-only workflow items).
//!
//! Output parsing is split out into pure functions (`parse_worktree_list`,
//! `parse_branch_lines`) so the decisions are unit-tested without a repo.

use std::path::Path;

/// What to diff the working tree against.
#[derive(Debug, Clone)]
pub enum DiffBase {
    /// `git diff HEAD` — uncommitted changes only.
    Head,
    /// Everything since the branch diverged from `origin/<base>`: diffs the
    /// working tree against `merge-base(origin/<base>, HEAD)`, so committed
    /// *and* uncommitted changes are included. Falls back to [`DiffBase::Head`]
    /// when the merge-base cannot be resolved (no remote-tracking ref).
    /// `dead_code` allowed: constructed by the GUI's workflow diff commands
    /// (lib crate); the TUI only diffs against HEAD.
    #[allow(dead_code)]
    MergeBase(String),
}

/// Run `git diff` in `dir` against the given base and return the raw unified
/// diff. `Err` carries a human-readable message (git's stderr, or the spawn
/// failure).
pub async fn git_diff(dir: &Path, base: &DiffBase) -> Result<String, String> {
    let against = match base {
        DiffBase::Head => "HEAD".to_string(),
        DiffBase::MergeBase(branch) => merge_base(dir, branch)
            .await
            .unwrap_or_else(|| "HEAD".to_string()),
    };

    let start = std::time::Instant::now();
    let output = tokio::process::Command::new("git")
        .args(["diff", &against])
        .current_dir(dir)
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {}", e))?;
    tracing::debug!(
        "git diff {} in {} took {:?}",
        against,
        dir.display(),
        start.elapsed()
    );

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// `git merge-base origin/<branch> HEAD`, if resolvable.
async fn merge_base(dir: &Path, branch: &str) -> Option<String> {
    let remote_ref = format!("origin/{}", branch);
    let output = tokio::process::Command::new("git")
        .args(["merge-base", &remote_ref, "HEAD"])
        .current_dir(dir)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Branch & worktree plumbing for review-only workflow items: materializing a
/// checkout of code that already exists elsewhere, and listing the branches to
/// choose from.
///
/// `dead_code` allowed for the whole module — every item is consumed by the GUI
/// crate through the lib, while the TUI bin (which declares these modules
/// privately) calls none of them. Same reason as `WorkflowRepository` and
/// `DiffBase::MergeBase`; a future TUI review view drops the attribute.
#[allow(dead_code)]
pub mod review {
    use std::path::Path;

    /// A local branch as offered in the "review a branch" picker.
    #[derive(Debug, Clone, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct LocalBranch {
        pub name: String,
        /// Relative commit date of the tip ("3 days ago") — the picker's detail line.
        pub last_commit: String,
        /// Where the branch is checked out, when it is (main checkout or worktree).
        pub worktree: Option<String>,
    }

    /// Pure: `worktree <path>` / `branch refs/heads/<name>` record pairs out of
    /// `git worktree list --porcelain`. Detached worktrees have no branch and are
    /// skipped; the main checkout is included like any other.
    pub fn parse_worktree_list(porcelain: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut path: Option<String> = None;
        for line in porcelain.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(p.trim().to_string());
            } else if let Some(r) = line.strip_prefix("branch ") {
                let branch = r.trim().strip_prefix("refs/heads/").unwrap_or(r.trim());
                if let Some(p) = path.take() {
                    out.push((p, branch.to_string()));
                }
            }
        }
        out
    }

    /// Pure: parse the `<name>\t<relative date>` lines emitted by the
    /// `for-each-ref` format in [`list_local_branches`].
    pub fn parse_branch_lines(stdout: &str) -> Vec<(String, String)> {
        stdout
            .lines()
            .filter_map(|l| {
                let (name, date) = l.split_once('\t').unwrap_or((l, ""));
                let name = name.trim();
                (!name.is_empty()).then(|| (name.to_string(), date.trim().to_string()))
            })
            .collect()
    }

    async fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
        let out = tokio::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .await
            .map_err(|e| format!("Failed to run git: {}", e))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    }

    /// Local branches, most recently committed first, annotated with the worktree
    /// each is checked out in (if any).
    pub async fn list_local_branches(repo: &Path) -> Result<Vec<LocalBranch>, String> {
        let stdout = git(
            repo,
            &[
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname:short)%09%(committerdate:relative)",
                "refs/heads",
            ],
        )
        .await?;
        let checkouts = worktrees_by_branch(repo).await;
        Ok(parse_branch_lines(&stdout)
            .into_iter()
            .map(|(name, last_commit)| LocalBranch {
                worktree: checkouts.get(&name).cloned(),
                name,
                last_commit,
            })
            .collect())
    }

    /// branch → checkout path for every worktree of `repo` (empty on git failure:
    /// callers treat "unknown" as "not checked out", which only costs a redundant
    /// `git worktree add` attempt that fails loudly).
    async fn worktrees_by_branch(repo: &Path) -> std::collections::HashMap<String, String> {
        match git(repo, &["worktree", "list", "--porcelain"]).await {
            Ok(out) => parse_worktree_list(&out)
                .into_iter()
                .map(|(p, b)| (b, p))
                .collect(),
            Err(e) => {
                tracing::warn!("git worktree list in {} failed: {}", repo.display(), e);
                std::collections::HashMap::new()
            }
        }
    }

    /// Worktree directory name for a branch: `feat/thing` → `feat-thing`, so a
    /// slashed branch doesn't turn into nested directories git won't create.
    fn worktree_dir_name(branch: &str) -> String {
        let name: String = branch
            .chars()
            .map(|c| if c == '/' || c == '\\' { '-' } else { c })
            .collect();
        let name = name.trim_matches(['-', '.', ' ']).to_string();
        if name.is_empty() {
            "review".to_string()
        } else {
            name
        }
    }

    /// Materialize a checkout of `branch` so its diff can be reviewed and an agent
    /// can work in it. Returns the directory to use as the item's worktree.
    ///
    /// Order matters:
    /// 1. Already checked out somewhere (main checkout or an existing worktree)?
    ///    Reuse that directory — never a second checkout of the same branch, and
    ///    reviewing a feature you already have open costs nothing.
    /// 2. Not local yet? Fetch it. With a PR number that is `refs/pull/<n>/head`,
    ///    which resolves fork PRs too; otherwise `origin/<branch>`.
    /// 3. Add a worktree under `<parent>/<repo>-worktrees/<branch>`, tracking
    ///    `origin/<branch>` when the branch is new (so the agent can push fixes).
    ///
    /// An existing local branch is trusted as-is and never fetched into — clobbering
    /// local commits to match a remote is not this function's call.
    ///
    /// The returned path is always canonical, so the same branch yields the same
    /// string whichever way it was resolved: `git worktree list` reports canonical
    /// paths (on macOS `/var/…` → `/private/var/…`) while a path built from the
    /// repo's parent does not, and the two forms would compare unequal against a
    /// session cwd or another item's `meta.worktree`.
    pub async fn checkout_for_review(
        repo: &Path,
        branch: &str,
        pr_number: Option<u64>,
    ) -> Result<String, String> {
        if branch.trim().is_empty() {
            return Err("No branch to review".to_string());
        }
        if !repo.is_dir() {
            return Err(format!("Not a directory: {}", repo.display()));
        }

        if let Some(dir) = worktrees_by_branch(repo).await.get(branch) {
            return Ok(canonical(Path::new(dir)));
        }

        let local_exists = git(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{}", branch),
            ],
        )
        .await
        .is_ok();
        if !local_exists {
            match pr_number {
                // The PR head ref lives on `origin` even when the PR comes from a
                // fork, so this is the one fetch that always works.
                Some(n) => {
                    let refspec = format!("refs/pull/{}/head:refs/heads/{}", n, branch);
                    git(repo, &["fetch", "origin", &refspec])
                        .await
                        .map_err(|e| format!("Could not fetch PR #{}: {}", n, e))?;
                }
                None => {
                    git(repo, &["fetch", "origin", branch])
                        .await
                        .map_err(|e| format!("Could not fetch branch '{}': {}", branch, e))?;
                }
            }
        }

        let repo_name = repo
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project");
        let base_dir = repo
            .parent()
            .unwrap_or(repo)
            .join(format!("{}-worktrees", repo_name));
        std::fs::create_dir_all(&base_dir).map_err(|e| e.to_string())?;
        let path = base_dir.join(worktree_dir_name(branch));
        let path_str = path.to_string_lossy().into_owned();

        // A fetched-by-refspec branch (the PR case) now exists locally; a plain
        // `fetch origin <branch>` only updated the remote ref, so branch it off
        // `origin/<branch>` to get upstream tracking.
        let exists_now = local_exists || pr_number.is_some();
        let upstream = format!("origin/{}", branch);
        let args: Vec<&str> = if exists_now {
            vec!["worktree", "add", &path_str, branch]
        } else {
            vec!["worktree", "add", &path_str, "-b", branch, &upstream]
        };
        git(repo, &args)
            .await
            .map_err(|e| format!("git worktree add failed: {}", e))?;
        Ok(canonical(&path))
    }

    /// Canonical form of an existing path, falling back to the path as given
    /// (canonicalization can only fail if the directory vanished under us).
    fn canonical(path: &Path) -> String {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn worktree_list_pairs_paths_with_branches() {
            let out = "\
worktree /w/clash
HEAD abc123
branch refs/heads/main

worktree /w/clash-worktrees/feat-x
HEAD def456
branch refs/heads/feat/x

worktree /w/clash-worktrees/loose
HEAD 999
detached
";
            let pairs = parse_worktree_list(out);
            assert_eq!(
                pairs,
                vec![
                    ("/w/clash".to_string(), "main".to_string()),
                    (
                        "/w/clash-worktrees/feat-x".to_string(),
                        "feat/x".to_string()
                    ),
                ]
            );
            // Detached worktrees carry no branch and must not be attributed to the
            // previous record.
            assert!(pairs.iter().all(|(p, _)| p != "/w/clash-worktrees/loose"));
            assert!(parse_worktree_list("").is_empty());
        }

        #[test]
        fn branch_lines_split_name_from_date() {
            let out = "main\t2 hours ago\nfeat/x\t3 days ago\nno-date\n\n";
            assert_eq!(
                parse_branch_lines(out),
                vec![
                    ("main".to_string(), "2 hours ago".to_string()),
                    ("feat/x".to_string(), "3 days ago".to_string()),
                    ("no-date".to_string(), String::new()),
                ]
            );
        }

        #[test]
        fn worktree_dir_names_stay_flat() {
            assert_eq!(worktree_dir_name("feat/x"), "feat-x");
            assert_eq!(worktree_dir_name("a/b/c"), "a-b-c");
            assert_eq!(worktree_dir_name("plain"), "plain");
            assert_eq!(worktree_dir_name("/"), "review");
        }
    }
}
