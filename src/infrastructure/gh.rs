//! GitHub CLI (`gh`) integration for workflow PRs.
//!
//! Everything shells out to the user's `gh` binary — clash never talks to the
//! GitHub API directly, so auth is entirely gh's concern. All functions are
//! synchronous `std::process::Command` (matching the raw-subprocess git
//! style); the async Tauri layer wraps calls in `spawn_blocking`.
//!
//! Degradation contract: [`GhError::NotInstalled`] / [`GhError::NotAuthenticated`]
//! map to the GUI's `gh-unavailable:` / `gh-unauthenticated:` error prefixes,
//! which disable PR buttons with a setup hint instead of failing flows.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Timeout for a read-only query (`gh auth status`, `gh pr view`, `gh pr ready`).
/// These are one small API call; a minute is already pathological.
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout for `gh pr create`. Longer than a query because gh may resolve the
/// repo, push and open the PR in one go.
const CREATE_TIMEOUT: Duration = Duration::from_secs(180);

/// Timeout for `git push`. The one call here that legitimately takes minutes —
/// a first push of a long-lived branch over a slow link is real work.
const PUSH_TIMEOUT: Duration = Duration::from_secs(600);

/// A pull request as reported by `gh pr view`.
///
/// One struct for both uses — the PR-lifecycle commands read the first four
/// fields, review-only item creation also needs the title and refs — so there
/// is a single `--json` field list and a single parser.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhPrView {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub is_draft: bool,
    /// "OPEN" | "MERGED" | "CLOSED".
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub title: String,
    /// Branch the PR is *from* — the one to check out for review.
    #[serde(default)]
    pub head_ref_name: String,
    /// Branch the PR is *into* — the diff base.
    #[serde(default)]
    pub base_ref_name: String,
}

/// The `--json` fields every `gh pr view` call in clash requests.
const PR_VIEW_FIELDS: &str = "url,number,isDraft,state,title,headRefName,baseRefName";

#[derive(Debug, thiserror::Error)]
pub enum GhError {
    #[error("gh CLI not installed")]
    NotInstalled,
    #[error("gh not authenticated: {0}")]
    NotAuthenticated(String),
    #[error("no pull request found for this branch")]
    NoPr,
    #[error("gh failed: {0}")]
    Command(String),
    #[error("could not parse gh output: {0}")]
    Parse(String),
    #[error("`{cmd}` did not finish within {secs}s — it was killed. {hint}")]
    TimedOut {
        cmd: String,
        secs: u64,
        hint: &'static str,
    },
}

/// Spawn `program` so it can never block on a prompt. Every `gh`/`git` call
/// here must go through this (or [`run_bounded`]).
///
/// Closed stdin alone is not enough — `ssh` reads `/dev/tty` directly, so the
/// env below is load-bearing, and the timeout backstops both. See the PR
/// integration section of `docs/workflows.md`.
fn spawn_quiet(program: &str, args: &[&str], dir: &Path) -> std::io::Result<std::process::Child> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        // Empty, not unset: an askpass helper would pop a dialog or hang
        // headless instead of erroring.
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("SSH_ASKPASS_REQUIRE", "never")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1");
    // Only when unset: clobbering a custom ssh command would break working
    // proxy/identity setups.
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes -oConnectTimeout=15");
    }
    cmd.spawn()
}

/// Read a child pipe to EOF on its own thread. Required, not an optimization:
/// polling `try_wait` while a child fills a pipe buffer deadlocks.
fn drain<P: Read + Send + 'static>(pipe: Option<P>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    })
}

/// Run `program args` in `dir`, killing it if it outruns `timeout`.
fn run_bounded(
    program: &str,
    args: &[&str],
    dir: &Path,
    timeout: Duration,
    hint: &'static str,
) -> Result<std::process::Output, GhError> {
    let mut child = spawn_quiet(program, args, dir).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            if program == "gh" {
                GhError::NotInstalled
            } else {
                GhError::Command(format!("could not run {}: {}", program, e))
            }
        } else {
            GhError::Command(e.to_string())
        }
    })?;

    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Err(e) => return Err(GhError::Command(e.to_string())),
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            tracing::warn!(
                "killed `{} {}` after {}s — exceeded timeout",
                program,
                args.join(" "),
                timeout.as_secs()
            );
            return Err(GhError::TimedOut {
                cmd: format!("{} {}", program, args.join(" ")),
                secs: timeout.as_secs(),
                hint,
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    Ok(std::process::Output {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

fn run(dir: &Path, args: &[&str]) -> Result<std::process::Output, GhError> {
    run_bounded(
        "gh",
        args,
        dir,
        QUERY_TIMEOUT,
        "Check `gh auth status` in a terminal.",
    )
}

/// Cheap health probe: is `gh` installed and authenticated?
pub fn check_gh() -> Result<(), GhError> {
    let output = run(Path::new("."), &["auth", "status"])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GhError::NotAuthenticated(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// Pure: parse the JSON emitted by `gh pr view --json url,number,isDraft,state`.
pub fn parse_pr_view_json(s: &str) -> Result<GhPrView, GhError> {
    serde_json::from_str(s.trim()).map_err(|e| GhError::Parse(e.to_string()))
}

/// Pure: extract `owner/repo` and PR number from a GitHub PR URL.
///
/// Lenient about how the link was copied: the scheme and `www.` are optional
/// (a URL pasted from a chat message or a `gh pr view` line often has neither)
/// and trailing sub-pages (`/files`, `/commits/<sha>`) are ignored. What it
/// still refuses is a link that isn't a PR (`/issues/42`) or isn't GitHub.
pub fn parse_pr_url(url: &str) -> Option<(String, u64)> {
    let rest = url.trim().trim_end_matches('/');
    let rest = rest
        .strip_prefix("https://")
        .or_else(|| rest.strip_prefix("http://"))
        .unwrap_or(rest);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.strip_prefix("github.com/")?;
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next()? != "pull" {
        return None;
    }
    let number: u64 = parts.next()?.split(['?', '#']).next()?.parse().ok()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((format!("{}/{}", owner, repo), number))
}

/// `gh pr view [selector] --json …` in `dir`. The selector is a PR number, a
/// branch or a URL; `""` means "the PR of the current branch".
pub fn pr_view(dir: &Path, branch: &str) -> Result<GhPrView, GhError> {
    pr_view_scoped(dir, branch, None)
}

/// [`pr_view`] scoped to an explicit `owner/repo` (`gh pr view <n> --repo …`).
///
/// A bare number is resolved against whatever repo `dir`'s remotes point at,
/// so a PR *number* taken from a URL must carry that URL's repo with it —
/// otherwise the same number silently resolves to a different PR in the local
/// repo (or to nothing at all).
pub fn pr_view_scoped(dir: &Path, selector: &str, repo: Option<&str>) -> Result<GhPrView, GhError> {
    let mut args = vec!["pr", "view"];
    if !selector.is_empty() {
        args.push(selector);
    }
    if let Some(repo) = repo {
        args.extend(["--repo", repo]);
    }
    args.extend(["--json", PR_VIEW_FIELDS]);
    let output = run(dir, &args)?;
    if output.status.success() {
        parse_pr_view_json(&String::from_utf8_lossy(&output.stdout))
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.to_lowercase().contains("no pull requests found") {
            Err(GhError::NoPr)
        } else if err.to_lowercase().contains("auth") && err.to_lowercase().contains("login") {
            Err(GhError::NotAuthenticated(err))
        } else {
            Err(GhError::Command(err))
        }
    }
}

/// Pure: does this `gh pr create` failure mean "the branch is not on the remote
/// yet"?
///
/// Run from a terminal, `gh` offers to push the branch itself; clash always
/// runs it non-interactively, so instead it aborts with
/// `aborted: you must first push the current branch to a remote, or use the
/// --head flag`. There is no distinct exit code, so the message is the only
/// signal — match the stable middle of it, not the punctuation around it.
pub fn is_unpushed_branch_error(stderr: &str) -> bool {
    stderr
        .to_lowercase()
        .contains("must first push the current branch")
}

/// Pure: pick the remote to push to out of `git remote`'s output. `origin`
/// wins when present (that is what `gh` resolves the repo from), otherwise the
/// first one listed; `None` when the repo has no remotes at all.
pub fn pick_push_remote(remote_list: &str) -> Option<String> {
    let mut first = None;
    for name in remote_list.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if name == "origin" {
            return Some(name.to_string());
        }
        first = first.or_else(|| Some(name.to_string()));
    }
    first
}

fn git(dir: &Path, args: &[&str]) -> Result<String, GhError> {
    git_bounded(dir, args, QUERY_TIMEOUT)
}

/// `git` with an explicit timeout — `push` needs a far longer one than the
/// local queries (`rev-parse`, `remote`) that surround it.
fn git_bounded(dir: &Path, args: &[&str], timeout: Duration) -> Result<String, GhError> {
    let output = run_bounded(
        "git",
        args,
        dir,
        timeout,
        "It was most likely waiting on a credential prompt — \
         try the same command in a terminal.",
    )?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(GhError::Command(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// `git push --set-upstream <remote> <current branch>` in `dir`.
///
/// Publishing the branch is the missing half of "create a PR" — a PR cannot
/// exist without a remote branch — so this is not a separate decision the user
/// has to make; it is what `gh` itself would have offered interactively.
fn push_current_branch(dir: &Path) -> Result<(), GhError> {
    let branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    if branch.is_empty() || branch == "HEAD" {
        return Err(GhError::Command(
            "cannot push a detached HEAD — check out a branch first".to_string(),
        ));
    }
    let remote = pick_push_remote(&git(dir, &["remote"])?).ok_or_else(|| {
        GhError::Command("this repo has no git remote to push the branch to".to_string())
    })?;
    tracing::info!("pushing {} to {} so a PR can be opened", branch, remote);
    git_bounded(
        dir,
        &["push", "--set-upstream", &remote, &branch],
        PUSH_TIMEOUT,
    )
    .map(|_| ())
    .map_err(|e| match e {
        // Passed through unwrapped: its message already names the cause.
        timed_out @ GhError::TimedOut { .. } => timed_out,
        other => GhError::Command(format!("git push {} {}: {}", remote, branch, other)),
    })
}

/// `gh pr create --draft` for the current branch in `dir`, then read the
/// created PR back via [`pr_view`].
///
/// A branch that has never been pushed is pushed first and the create retried
/// once — see [`is_unpushed_branch_error`]. The retry is single and gated on
/// that one message, so any other failure still surfaces as gh reported it.
pub fn pr_create_draft(
    dir: &Path,
    title: &str,
    body: &str,
    base: Option<&str>,
) -> Result<GhPrView, GhError> {
    let mut args = vec!["pr", "create", "--draft", "--title", title, "--body", body];
    if let Some(base) = base {
        args.extend(["--base", base]);
    }
    let create = |args: &[&str]| {
        run_bounded(
            "gh",
            args,
            dir,
            CREATE_TIMEOUT,
            "It may have been waiting on a credential or remote prompt — \
             try `gh pr create --draft` in a terminal.",
        )
    };
    let output = create(&args)?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !is_unpushed_branch_error(&err) {
            return Err(GhError::Command(err));
        }
        push_current_branch(dir)?;
        let retry = create(&args)?;
        if !retry.status.success() {
            return Err(GhError::Command(
                String::from_utf8_lossy(&retry.stderr).trim().to_string(),
            ));
        }
    }
    pr_view(dir, "")
}

/// `gh pr comment <number> --body-file <tmp>` in `dir` — post one issue-level
/// comment on the PR. The body travels through a temp file: the hardened
/// runner closes stdin (see `spawn_quiet`), and a full review round pasted
/// into argv can exceed platform limits.
pub fn pr_comment(dir: &Path, number: u64, body: &str) -> Result<(), GhError> {
    let tmp = std::env::temp_dir().join(format!(
        "clash-pr-comment-{}-{}.md",
        std::process::id(),
        number
    ));
    std::fs::write(&tmp, body).map_err(|e| GhError::Command(e.to_string()))?;
    let n = number.to_string();
    let tmp_arg = tmp.to_string_lossy().into_owned();
    let result = run(dir, &["pr", "comment", &n, "--body-file", &tmp_arg]);
    let _ = std::fs::remove_file(&tmp);
    let output = result?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GhError::Command(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// `gh pr ready <number>` in `dir` — flips a draft PR to ready-for-review.
pub fn pr_ready(dir: &Path, number: u64) -> Result<(), GhError> {
    let number = number.to_string();
    let output = run(dir, &["pr", "ready", &number])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GhError::Command(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pr_view_json_full() {
        let json = r#"{"url":"https://github.com/o/r/pull/7","number":7,"isDraft":true,
            "state":"OPEN","title":"Add auth","headRefName":"feat/auth","baseRefName":"develop"}"#;
        let pr = parse_pr_view_json(json).unwrap();
        assert_eq!(pr.number, 7);
        assert!(pr.is_draft);
        assert_eq!(pr.state, "OPEN");
        assert_eq!(pr.url, "https://github.com/o/r/pull/7");
        // The review-only creation path needs these three.
        assert_eq!(pr.title, "Add auth");
        assert_eq!(pr.head_ref_name, "feat/auth");
        assert_eq!(pr.base_ref_name, "develop");
    }

    #[test]
    fn parse_pr_view_json_missing_fields_default() {
        let pr = parse_pr_view_json(r#"{"number": 3}"#).unwrap();
        assert_eq!(pr.number, 3);
        assert!(!pr.is_draft);
        assert_eq!(pr.state, "");
        assert_eq!(pr.title, "");
        assert_eq!(pr.head_ref_name, "");
        assert_eq!(pr.base_ref_name, "");
        assert!(parse_pr_view_json("not json").is_err());
    }

    #[test]
    fn pr_view_fields_cover_every_struct_field() {
        // The `--json` list and the struct must not drift apart: every field
        // parsed here has to be requested from gh.
        for field in [
            "url",
            "number",
            "isDraft",
            "state",
            "title",
            "headRefName",
            "baseRefName",
        ] {
            assert!(
                PR_VIEW_FIELDS.split(',').any(|f| f == field),
                "{} missing from PR_VIEW_FIELDS",
                field
            );
        }
    }

    #[test]
    fn parse_pr_url_variants() {
        assert_eq!(
            parse_pr_url("https://github.com/acme/clash/pull/42"),
            Some(("acme/clash".to_string(), 42))
        );
        assert_eq!(
            parse_pr_url("https://github.com/acme/clash/pull/42/"),
            Some(("acme/clash".to_string(), 42))
        );
        assert_eq!(
            parse_pr_url("https://github.com/acme/clash/pull/42#discussion_r1"),
            Some(("acme/clash".to_string(), 42))
        );
        assert_eq!(
            parse_pr_url("https://github.com/acme/clash/pull/42?diff=split"),
            Some(("acme/clash".to_string(), 42))
        );
        assert!(parse_pr_url("https://github.com/acme/clash/issues/42").is_none());
        assert!(parse_pr_url("https://gitlab.com/acme/clash/pull/42").is_none());
        assert!(parse_pr_url("https://github.com/acme/clash/pull/abc").is_none());
    }

    /// Shapes a user actually pastes: a sub-page of the PR, no scheme (copied
    /// from a chat message), `www.`, `http://`, surrounding whitespace.
    #[test]
    fn parse_pr_url_tolerates_real_world_pastes() {
        let expected = Some(("acme/clash".to_string(), 42));
        assert_eq!(
            parse_pr_url("https://github.com/acme/clash/pull/42/files"),
            expected
        );
        assert_eq!(
            parse_pr_url("https://github.com/acme/clash/pull/42/commits/abc123"),
            expected
        );
        assert_eq!(parse_pr_url("github.com/acme/clash/pull/42"), expected);
        assert_eq!(parse_pr_url("www.github.com/acme/clash/pull/42"), expected);
        assert_eq!(
            parse_pr_url("http://www.github.com/acme/clash/pull/42"),
            expected
        );
        assert_eq!(
            parse_pr_url("  https://github.com/acme/clash/pull/42  "),
            expected
        );
        // Still not a PR / not GitHub.
        assert!(parse_pr_url("github.com/acme/clash/issues/42").is_none());
        assert!(parse_pr_url("gitlab.com/acme/clash/pull/42").is_none());
    }

    /// The message gh actually prints when a PR is opened from a branch that
    /// has never been pushed. Only this one aborts into a push-and-retry —
    /// everything else must keep surfacing as gh reported it.
    #[test]
    fn unpushed_branch_error_is_recognized() {
        assert!(is_unpushed_branch_error(
            "aborted: you must first push the current branch to a remote, or use the --head flag"
        ));
        // Case is not guaranteed across gh versions.
        assert!(is_unpushed_branch_error(
            "You must first push the current branch to a remote"
        ));
        assert!(!is_unpushed_branch_error(
            "pull request create failed: GraphQL: No commits between main and feat/x"
        ));
        assert!(!is_unpushed_branch_error(
            "a pull request for branch \"feat/x\" already exists"
        ));
        assert!(!is_unpushed_branch_error(""));
    }

    #[test]
    fn push_remote_prefers_origin() {
        assert_eq!(
            pick_push_remote("upstream\norigin\nfork\n"),
            Some("origin".to_string())
        );
        // No origin: the first one listed, which is what a single-remote fork
        // (`git remote` → "upstream") needs.
        assert_eq!(
            pick_push_remote("upstream\nfork\n"),
            Some("upstream".to_string())
        );
        assert_eq!(
            pick_push_remote("  \n  origin  \n"),
            Some("origin".to_string())
        );
        // No remotes at all — the caller turns this into a real error rather
        // than pushing to a guessed name.
        assert_eq!(pick_push_remote(""), None);
        assert_eq!(pick_push_remote("\n \n"), None);
    }
}
