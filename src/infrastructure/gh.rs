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

use std::path::Path;
use std::process::Command;

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
}

fn run(dir: &Path, args: &[&str]) -> Result<std::process::Output, GhError> {
    Command::new("gh")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GhError::NotInstalled
            } else {
                GhError::Command(e.to_string())
            }
        })
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

/// `gh pr create --draft` for the current branch in `dir`, then read the
/// created PR back via [`pr_view`].
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
    let output = run(dir, &args)?;
    if !output.status.success() {
        return Err(GhError::Command(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    pr_view(dir, "")
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
}
