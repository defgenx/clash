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

/// Whether `dir` is inside a git work tree. `false` also when git is missing.
/// `dead_code` allowed: only the GUI's workflow launcher calls it.
#[allow(dead_code)]
pub async fn is_repository(dir: &Path) -> bool {
    tokio::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `git diff` in `dir` against the given base and return the raw unified
/// diff. `Err` carries a human-readable message (git's stderr, or the spawn
/// failure).
pub async fn git_diff(dir: &Path, base: &DiffBase) -> Result<String, String> {
    git_diff_with(dir, base, &DiffOptions::default()).await
}

/// What a working-tree diff shows beyond its base.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffOptions {
    /// `-w`: whitespace-only changes disappear.
    pub ignore_whitespace: bool,
    /// Files git does not track yet, appended as new-file diffs (`git diff`
    /// never shows them, which hides exactly what an agent just created).
    /// Only working-tree targets have any.
    pub untracked: bool,
    /// Lines of context around each change (`-U<n>`); `None` is git's 3.
    pub context: Option<u32>,
}

/// What a diff compares — the targets of the GUI's diff viewer. `dead_code`
/// allowed: the TUI only diffs the working tree (`WorkTree`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DiffTarget {
    /// The working tree against a revision (`git diff <rev>`); `HEAD` is
    /// "uncommitted changes".
    WorkTree(String),
    /// The index against HEAD (`git diff --cached`).
    Staged,
    /// The working tree against the index (`git diff`).
    Unstaged,
    /// One commit's own changes, against its first parent (`git show`).
    Commit(String),
}

impl DiffTarget {
    fn includes_untracked(&self) -> bool {
        matches!(self, DiffTarget::WorkTree(_) | DiffTarget::Unstaged)
    }
}

/// Pure: whether `rev` may be handed to git as a revision. Revisions come
/// from the frontend, so one starting with `-` would be read as an option.
pub fn valid_rev(rev: &str) -> bool {
    !rev.is_empty()
        && rev.len() < 256
        && !rev.starts_with('-')
        && !rev.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Untracked files appended to a diff, at most; the rest are left out.
pub const UNTRACKED_MAX_FILES: usize = 200;
/// An untracked file larger than this appears as a header without content.
pub const UNTRACKED_MAX_BYTES: u64 = 512 * 1024;

/// Pure: the `git` arguments of a working-tree diff against `against`.
/// Colour and external diff drivers are off whatever the user's config says:
/// the output is parsed, not shown.
pub fn diff_args(target: &DiffTarget, opts: &DiffOptions, paths: &[String]) -> Vec<String> {
    let mut args: Vec<String> = match target {
        DiffTarget::Commit(_) => vec![
            "show".into(),
            "--format=".into(),
            "--diff-merges=first-parent".into(),
        ],
        _ => vec!["diff".into()],
    };
    args.extend(["--no-color".into(), "--no-ext-diff".into(), "-M".into()]);
    if opts.ignore_whitespace {
        args.push("-w".into());
    }
    if let Some(n) = opts.context {
        args.push(format!("-U{n}"));
    }
    match target {
        DiffTarget::WorkTree(rev) | DiffTarget::Commit(rev) => args.push(rev.clone()),
        DiffTarget::Staged => args.push("--cached".into()),
        DiffTarget::Unstaged => {}
    }
    if !paths.is_empty() {
        args.push("--".into());
        args.extend(paths.iter().cloned());
    }
    args
}

/// Pure: `git ls-files -z` output into paths, at most `cap` of them.
pub fn parse_nul_paths(out: &str, cap: usize) -> Vec<String> {
    out.split('\0')
        .filter(|p| !p.is_empty())
        .take(cap)
        .map(str::to_string)
        .collect()
}

/// Pure: the diff header of an untracked file too large to show.
pub fn untracked_header(path: &str) -> String {
    format!("diff --git a/{path} b/{path}\nnew file mode 100644\n")
}

/// [`git_diff`] with [`DiffOptions`].
pub async fn git_diff_with(
    dir: &Path,
    base: &DiffBase,
    opts: &DiffOptions,
) -> Result<String, String> {
    let against = match base {
        DiffBase::Head => "HEAD".to_string(),
        DiffBase::MergeBase(branch) => merge_base(dir, branch)
            .await
            .unwrap_or_else(|| "HEAD".to_string()),
    };
    git_diff_target(dir, &DiffTarget::WorkTree(against), opts, &[]).await
}

/// The diff of any [`DiffTarget`], restricted to `paths` (relative to `dir`)
/// when there are any.
pub async fn git_diff_target(
    dir: &Path,
    target: &DiffTarget,
    opts: &DiffOptions,
    paths: &[String],
) -> Result<String, String> {
    if let DiffTarget::WorkTree(rev) | DiffTarget::Commit(rev) = target {
        if !valid_rev(rev) {
            return Err(format!("not a revision: {rev}"));
        }
    }
    let start = std::time::Instant::now();
    let output = tokio::process::Command::new("git")
        .args(diff_args(target, opts, paths))
        .current_dir(dir)
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {}", e))?;
    tracing::debug!(
        "git diff {:?} in {} took {:?}",
        target,
        dir.display(),
        start.elapsed()
    );

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if opts.untracked && target.includes_untracked() {
        text.push_str(&untracked_diff(dir, opts, paths).await);
    }
    Ok(text)
}

/// `git merge-base <rev> HEAD`, if resolvable — "since this branch left
/// `rev`". `dead_code` allowed: only the GUI's diff viewer calls it.
#[allow(dead_code)]
pub async fn merge_base_with(dir: &Path, rev: &str) -> Option<String> {
    if !valid_rev(rev) {
        return None;
    }
    let output = tokio::process::Command::new("git")
        .args(["merge-base", rev, "HEAD"])
        .current_dir(dir)
        .output()
        .await
        .ok()?;
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !sha.is_empty()).then_some(sha)
}

/// The commit `HEAD` points at in `dir`, or `None` outside a repository (or
/// before the first commit). GUI-only (wave start commits), hence the allow
/// for the binary's build.
#[allow(dead_code)]
pub async fn head_sha(dir: &Path) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .current_dir(dir)
        .output()
        .await
        .ok()?;
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !sha.is_empty()).then_some(sha)
}

/// How many paths `git status` reports in `dir` (uncommitted or untracked),
/// or `None` when `dir` is not a checkout.
#[allow(dead_code)]
pub async fn dirty_count(dir: &Path) -> Option<usize> {
    let out = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir)
        .output()
        .await
        .ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count()
    })
}

/// Commits on local `branch` that no remote-tracking ref contains, or `None`
/// when the branch does not exist.
#[allow(dead_code)]
pub async fn local_only_commits(repo: &Path, branch: &str) -> Option<u64> {
    if !valid_rev(branch) {
        return None;
    }
    let local = format!("refs/heads/{branch}");
    let out = tokio::process::Command::new("git")
        .args(["rev-list", "--count", &local, "--not", "--remotes"])
        .current_dir(repo)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// The commit local `branch` points at in `repo`, if it exists.
#[allow(dead_code)]
pub async fn branch_head(repo: &Path, branch: &str) -> Option<String> {
    if !valid_rev(branch) {
        return None;
    }
    let sha = run_git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .ok()?;
    let sha = sha.trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

/// How many commits `to` has that `from` does not (`git rev-list --count
/// from..to`), run in `dir`.
#[allow(dead_code)]
pub async fn commits_between(dir: &Path, from: &str, to: &str) -> Option<u64> {
    if !valid_rev(from) || !valid_rev(to) {
        return None;
    }
    run_git(dir, &["rev-list", "--count", &format!("{from}..{to}")])
        .await
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// `git worktree remove [--force] <path>` from `repo`, then a prune.
#[allow(dead_code)]
pub async fn worktree_remove(repo: &Path, path: &str, force: bool) -> Result<(), String> {
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push("--");
    args.push(path);
    run_git(repo, &args).await?;
    let _ = run_git(repo, &["worktree", "prune"]).await;
    Ok(())
}

/// Delete local `branch` (`git branch -D`): squash merges leave a merged
/// branch that `-d` refuses.
#[allow(dead_code)]
pub async fn branch_delete(repo: &Path, branch: &str) -> Result<(), String> {
    if !valid_rev(branch) {
        return Err(format!("not a branch name: {branch}"));
    }
    run_git(repo, &["branch", "-D", "--", branch])
        .await
        .map(|_| ())
}

async fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The repository's top-level directory, if `dir` is inside one.
pub async fn toplevel(dir: &Path) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .await
        .ok()?;
    let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !top.is_empty()).then_some(top)
}

/// Every untracked, non-ignored file under `dir` as a new-file diff, with
/// paths relative to the repository root like the rest of `git diff`.
async fn untracked_diff(dir: &Path, opts: &DiffOptions, paths: &[String]) -> String {
    let Some(top) = toplevel(dir).await else {
        return String::new();
    };
    let mut ls = vec![
        "ls-files",
        "--others",
        "--exclude-standard",
        "--full-name",
        "-z",
    ];
    if !paths.is_empty() {
        ls.push("--");
        ls.extend(paths.iter().map(String::as_str));
    }
    let Ok(listed) = tokio::process::Command::new("git")
        .args(&ls)
        .current_dir(dir)
        .output()
        .await
    else {
        return String::new();
    };
    let mut text = String::new();
    for path in parse_nul_paths(
        &String::from_utf8_lossy(&listed.stdout),
        UNTRACKED_MAX_FILES,
    ) {
        let abs = Path::new(&top).join(&path);
        if std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0) > UNTRACKED_MAX_BYTES {
            text.push_str(&untracked_header(&path));
            continue;
        }
        let mut args = vec![
            "diff".to_string(),
            "--no-color".into(),
            "--no-ext-diff".into(),
            "--no-index".into(),
        ];
        if opts.ignore_whitespace {
            args.push("-w".into());
        }
        if let Some(n) = opts.context {
            args.push(format!("-U{n}"));
        }
        args.extend(["--".into(), "/dev/null".into(), path.clone()]);
        // `--no-index` exits 1 when the files differ, which they always do here.
        if let Ok(out) = tokio::process::Command::new("git")
            .args(&args)
            .current_dir(&top)
            .output()
            .await
        {
            if matches!(out.status.code(), Some(0) | Some(1)) {
                text.push_str(&String::from_utf8_lossy(&out.stdout));
            }
        }
    }
    text
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
    fn diff_args_per_target() {
        let plain = DiffOptions::default();
        let head = DiffTarget::WorkTree("HEAD".into());
        assert_eq!(
            diff_args(&head, &plain, &[]),
            ["diff", "--no-color", "--no-ext-diff", "-M", "HEAD"]
        );
        assert_eq!(
            diff_args(&DiffTarget::Staged, &plain, &[]),
            ["diff", "--no-color", "--no-ext-diff", "-M", "--cached"]
        );
        assert_eq!(
            diff_args(&DiffTarget::Unstaged, &plain, &[]),
            ["diff", "--no-color", "--no-ext-diff", "-M"]
        );
        let knobs = DiffOptions {
            ignore_whitespace: true,
            untracked: true,
            context: Some(10),
        };
        assert_eq!(
            diff_args(
                &DiffTarget::Commit("abc123".into()),
                &knobs,
                &["src/a.rs".into()]
            ),
            [
                "show",
                "--format=",
                "--diff-merges=first-parent",
                "--no-color",
                "--no-ext-diff",
                "-M",
                "-w",
                "-U10",
                "abc123",
                "--",
                "src/a.rs"
            ]
        );
    }

    #[test]
    fn a_revision_cannot_smuggle_an_option() {
        for ok in [
            "HEAD",
            "origin/main",
            "abc123",
            "v1.2.0",
            "HEAD~3",
            "feature/x",
        ] {
            assert!(valid_rev(ok), "{ok}");
        }
        for bad in ["", "-p", "--output=/tmp/x", "a b", "a\nb"] {
            assert!(!valid_rev(bad), "{bad:?}");
        }
    }

    #[test]
    fn nul_paths_are_split_and_capped() {
        assert_eq!(
            parse_nul_paths("a.rs\0dir/b c.txt\0", 10),
            ["a.rs", "dir/b c.txt"]
        );
        assert_eq!(parse_nul_paths("a\0b\0c\0", 2), ["a", "b"]);
        assert!(parse_nul_paths("", 5).is_empty());
        assert_eq!(
            untracked_header("x/y.bin"),
            "diff --git a/x/y.bin b/x/y.bin\nnew file mode 100644\n"
        );
    }

    /// Untracked files appear only when asked, as root-relative new-file
    /// diffs even from a subdirectory, and ignored files never do.
    #[test]
    fn untracked_files_join_the_diff_when_asked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(repo)
                .output()
                .expect("git");
            assert!(out.status.success(), "git {:?}: {:?}", args, out);
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::write(repo.join("tracked.txt"), "one\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "skip.log\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "seed"]);
        std::fs::write(repo.join("tracked.txt"), "one  \n").unwrap();
        std::fs::write(repo.join("sub/new.txt"), "hello\n").unwrap();
        std::fs::write(repo.join("skip.log"), "noise\n").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let diff = |d: &Path, opts: DiffOptions| {
            rt.block_on(git_diff_with(d, &DiffBase::Head, &opts))
                .unwrap()
        };
        let plain = diff(repo, DiffOptions::default());
        assert!(plain.contains("tracked.txt"));
        assert!(!plain.contains("sub/new.txt"));

        let all = diff(
            &repo.join("sub"),
            DiffOptions {
                untracked: true,
                ..Default::default()
            },
        );
        assert!(all.contains("+++ b/sub/new.txt"), "{all}");
        assert!(all.contains("+hello"));
        assert!(!all.contains("skip.log"));

        let ws = diff(
            repo,
            DiffOptions {
                ignore_whitespace: true,
                ..Default::default()
            },
        );
        assert!(
            !ws.contains("tracked.txt"),
            "a whitespace-only change is gone: {ws}"
        );

        // A path restricts both halves; staged sees nothing until `git add`.
        let only = rt
            .block_on(git_diff_target(
                repo,
                &DiffTarget::WorkTree("HEAD".into()),
                &DiffOptions {
                    untracked: true,
                    ..Default::default()
                },
                &["sub".into()],
            ))
            .unwrap();
        assert!(
            only.contains("sub/new.txt") && !only.contains("tracked.txt"),
            "{only}"
        );
        let staged = |rt: &tokio::runtime::Runtime| {
            rt.block_on(git_diff_target(
                repo,
                &DiffTarget::Staged,
                &DiffOptions::default(),
                &[],
            ))
            .unwrap()
        };
        assert!(staged(&rt).is_empty());
        run(&["add", "tracked.txt"]);
        assert!(staged(&rt).contains("tracked.txt"));
        let seed = rt
            .block_on(merge_base_with(repo, "HEAD"))
            .expect("merge-base");
        let commit = rt
            .block_on(git_diff_target(
                repo,
                &DiffTarget::Commit(seed),
                &DiffOptions::default(),
                &[],
            ))
            .unwrap();
        assert!(
            commit.contains("+++ b/tracked.txt") && commit.contains("+one"),
            "{commit}"
        );
    }

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
