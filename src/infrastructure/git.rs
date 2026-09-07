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

/// How far a `git worktree add` checkout has got, from git's own progress
/// output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckoutProgress {
    pub files: u32,
    pub total: u32,
    /// Derived from the counts rather than read from git's `NN%`, so the two
    /// halves of the message can never disagree.
    pub percent: u8,
}

/// Pure: one chunk of git's checkout progress (`Updating files:  47%
/// (13176/28034)`) into counts. `None` for everything else git writes on the
/// same stream — `Preparing worktree (new branch 'x')`, `HEAD is now at
/// abc123 fix(auth): …`, a blank chunk — which is why a `%` *and* an
/// `(a/b)` group are both required: a commit subject alone can supply either.
pub fn parse_checkout_progress(chunk: &str) -> Option<CheckoutProgress> {
    if !chunk.contains('%') {
        return None;
    }
    let open = chunk.rfind('(')?;
    let inner = &chunk[open + 1..];
    let inner = &inner[..inner.find(')')?];
    let (files, total) = inner.split_once('/')?;
    let files: u32 = files.trim().parse().ok()?;
    let total: u32 = total.trim().parse().ok()?;
    if total == 0 {
        return None;
    }
    Some(CheckoutProgress {
        files,
        total,
        percent: (u64::from(files) * 100 / u64::from(total)).min(100) as u8,
    })
}

/// `git worktree add <args>` in `repo`, reporting the checkout to
/// `on_progress` as it goes.
///
/// The progress is the whole point of not using `.output()` here: a worktree
/// add is a full checkout, so on a large repository it writes tens of
/// thousands of files and takes the better part of a minute. A frontend with
/// nothing to show for that is indistinguishable from one that is wedged,
/// which is exactly how it was read. git reports "Updating files" on stderr
/// even when that is a pipe, carriage-return separated; `GIT_PROGRESS_DELAY=0`
/// only drops the 2s it otherwise waits before the first report, so the first
/// thing the human sees is a number rather than a pause.
pub async fn worktree_add(
    repo: &Path,
    args: &[&str],
    mut on_progress: impl FnMut(CheckoutProgress),
) -> Result<(), String> {
    use tokio::io::AsyncReadExt;

    let mut child = tokio::process::Command::new("git")
        .arg("worktree")
        .arg("add")
        .args(args)
        .current_dir(repo)
        .env("GIT_PROGRESS_DELAY", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run git: {}", e))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "git stderr not piped".to_string())?;

    // Chunks are split on bytes and only then decoded, so a read that lands
    // mid-character can't turn a path in git's output into replacement marks.
    let mut buf = [0u8; 4096];
    let mut pending: Vec<u8> = Vec::new();
    // Everything that wasn't progress, kept for the failure message — git's
    // reason for refusing (a taken path, a locked worktree) is in there.
    let mut said = String::new();
    let mut consume = |chunk: &[u8], said: &mut String| {
        let text = String::from_utf8_lossy(chunk);
        let text = text.trim();
        match parse_checkout_progress(text) {
            Some(p) => on_progress(p),
            None if !text.is_empty() => {
                said.push_str(text);
                said.push('\n');
            }
            None => {}
        }
    };
    loop {
        let n = match stderr.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(i) = pending.iter().position(|b| *b == b'\r' || *b == b'\n') {
            let chunk: Vec<u8> = pending.drain(..=i).collect();
            consume(&chunk, &mut said);
        }
    }
    consume(&pending, &mut said);

    let status = child
        .wait()
        .await
        .map_err(|e| format!("git worktree add failed: {}", e))?;
    if !status.success() {
        let why = said.trim();
        return Err(format!(
            "git worktree add failed{}{}",
            if why.is_empty() { "" } else { ": " },
            why
        ));
    }
    Ok(())
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

    /// Pure: GitHub `owner/repo` out of a remote URL — scp-like
    /// (`git@github.com:owner/repo.git`), `https://`, `ssh://`, with or without
    /// userinfo and the `.git` suffix. `None` for anything not on github.com,
    /// so a non-GitHub remote simply skips the checks built on this.
    pub fn parse_github_slug(remote: &str) -> Option<String> {
        let raw = remote.trim().trim_end_matches('/');
        let raw = raw.strip_suffix(".git").unwrap_or(raw);
        // Scheme, then userinfo — what's left starts at the host.
        let rest = raw.split_once("://").map(|(_, r)| r).unwrap_or(raw);
        let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
        // `:` separates host from path in the scp-like form, `/` in a URL.
        let (host, path) = rest.split_once([':', '/'])?;
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com")
        {
            return None;
        }
        let mut parts = path.trim_start_matches('/').split('/');
        let owner = parts.next()?;
        let name = parts.next()?;
        (!owner.is_empty() && !name.is_empty()).then(|| format!("{}/{}", owner, name))
    }

    /// `owner/repo` of the repo's `origin` remote, when it is a GitHub remote.
    pub async fn origin_repo_slug(repo: &Path) -> Option<String> {
        let out = git(repo, &["remote", "get-url", "origin"]).await.ok()?;
        parse_github_slug(out.trim())
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
        fn github_slug_from_every_remote_form() {
            let want = Some("owner/repo".to_string());
            assert_eq!(parse_github_slug("git@github.com:owner/repo.git"), want);
            assert_eq!(parse_github_slug("git@github.com:owner/repo"), want);
            assert_eq!(parse_github_slug("https://github.com/owner/repo.git"), want);
            assert_eq!(parse_github_slug("https://github.com/owner/repo/"), want);
            assert_eq!(
                parse_github_slug("ssh://git@github.com/owner/repo.git"),
                want
            );
            assert_eq!(
                parse_github_slug("https://token@github.com/owner/repo"),
                want
            );
            assert_eq!(parse_github_slug("https://GitHub.com/owner/repo"), want);
            // Not GitHub, or not a repo path.
            assert_eq!(parse_github_slug("git@gitlab.com:owner/repo.git"), None);
            assert_eq!(parse_github_slug("https://github.com/owner"), None);
            assert_eq!(parse_github_slug("/local/bare/repo.git"), None);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkout_progress_reads_gits_own_counts() {
        // The two shapes git emits, with the leading spaces it pads to.
        assert_eq!(
            parse_checkout_progress("Updating files:  47% (13176/28034)"),
            Some(CheckoutProgress {
                files: 13176,
                total: 28034,
                percent: 47,
            })
        );
        assert_eq!(
            parse_checkout_progress("Updating files: 100% (28034/28034), done."),
            Some(CheckoutProgress {
                files: 28034,
                total: 28034,
                percent: 100,
            })
        );
    }

    /// The streaming half, against a real repository: it must not hang, it
    /// must report git's own reason for a refusal, and the checkout it reports
    /// must be the one it performed. Only the progress *parsing* is unit
    /// tested; the reason this exists is that the plumbing around it — piping
    /// stderr, splitting on carriage returns, reaping the child — is what
    /// turns "slow" into "wedged forever" if it is wrong.
    #[test]
    fn worktree_add_reports_its_checkout_and_its_failures() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .expect("git");
            assert!(out.status.success(), "git {:?}: {:?}", args, out);
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        // Enough files that git has something to count.
        for i in 0..40 {
            std::fs::write(repo.join(format!("f{}.txt", i)), "x").unwrap();
        }
        run(&["add", "-A"]);
        run(&["commit", "-qm", "seed"]);

        let wt = dir.path().join("wt");
        let wt_str = wt.to_string_lossy().into_owned();
        let seen = std::cell::RefCell::new(Vec::new());
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(worktree_add(&repo, &[&wt_str, "-b", "feat"], |p| {
            seen.borrow_mut().push(p)
        }))
        .expect("worktree add");
        assert!(wt.join("f0.txt").is_file(), "the checkout must have landed");
        // `GIT_PROGRESS_DELAY=0` is what makes this deterministic: without it
        // git waits 2s before its first report and a 40-file checkout would
        // finish first — the flag is the difference between a number on
        // screen from the first frame and 44 silent seconds.
        assert!(
            !seen.borrow().is_empty(),
            "the checkout must have reported itself"
        );
        for p in seen.borrow().iter() {
            assert!(p.files <= p.total, "nonsense progress: {:?}", p);
            assert_eq!(p.total, 40, "the counts must describe this checkout");
        }

        // Same path twice: git refuses, and its reason must survive to the
        // caller — a bare "failed" leaves nothing to act on.
        let err = rt
            .block_on(worktree_add(&repo, &[&wt_str, "-b", "feat2"], |_| {}))
            .expect_err("a taken path must fail");
        assert!(err.contains("already exists"), "unhelpful error: {}", err);
    }

    #[test]
    fn only_progress_chunks_parse_as_progress() {
        // Everything else git writes on the same stream. The commit subject is
        // the trap: parentheses and a slash are both ordinary in one, so a
        // `%` is required too — and a percentage alone isn't enough either.
        for other in [
            "Preparing worktree (new branch 'user-consent-revocation')",
            "HEAD is now at 6d58ea6878a refactor(auth): nest the serializers",
            "HEAD is now at abc1234 chore: bump coverage to 90% (unit/integration)",
            "fatal: '/w/x' already exists",
            "Updating files: 100%",
            "",
            "   ",
        ] {
            assert_eq!(parse_checkout_progress(other), None, "parsed {:?}", other);
        }
    }
}
