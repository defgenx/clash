//! Forge implementations and detection.
//!
//! [`GithubForge`] wraps the existing `gh` CLI transport
//! ([`crate::infrastructure::gh`]) behind the [`Forge`] port; [`NoForge`]
//! makes a forge-less repo explicit — every change-request operation reports
//! itself as unsupported instead of failing deep inside a tool that was never
//! going to work.
//!
//! Detection reads the host of `git remote get-url origin`. The rule is
//! deliberately conservative: an *unknown* host maps to GitHub, because
//! before the port existed everything went through `gh` and a GitHub
//! Enterprise setup (custom host + `gh` configured for it) worked — detection
//! must not regress it. Only hosts that are recognizably another forge
//! (gitlab, bitbucket) map to [`ForgeKind::None`] until they have an
//! implementation. The `workflows.forge` config setting overrides everything.

use std::path::Path;
use std::sync::Arc;

use crate::domain::forge::{ChangeState, ChangeView, Forge, ForgeCaps, ForgeError, ForgeKind};
use crate::infrastructure::gh::{self, GhError, GhPrView};

/// GitHub via the `gh` CLI.
pub struct GithubForge;

/// No supported forge. The pipeline minus the PR stages is fully usable.
pub struct NoForge;

/// The implementation for a detected/overridden kind.
pub fn forge_of(kind: ForgeKind) -> Arc<dyn Forge> {
    match kind {
        ForgeKind::GitHub => Arc::new(GithubForge),
        ForgeKind::None => Arc::new(NoForge),
    }
}

fn err_from_gh(e: GhError) -> ForgeError {
    match e {
        GhError::NotInstalled => ForgeError::NotInstalled("gh"),
        GhError::NotAuthenticated(m) => ForgeError::NotAuthenticated(m),
        other => ForgeError::Other(other.to_string()),
    }
}

fn view_from_gh(v: GhPrView) -> ChangeView {
    ChangeView {
        state: ChangeState::parse(&v.state),
        url: v.url,
        number: v.number,
        draft: v.is_draft,
        title: v.title,
        head_ref: v.head_ref_name,
        base_ref: v.base_ref_name,
    }
}

impl Forge for GithubForge {
    fn kind(&self) -> ForgeKind {
        ForgeKind::GitHub
    }

    fn caps(&self) -> ForgeCaps {
        ForgeCaps {
            change_requests: true,
            drafts: true,
            review_comments: true,
        }
    }

    fn parse_change_url(&self, url: &str) -> Option<(String, u64)> {
        gh::parse_pr_url(url)
    }

    fn view(
        &self,
        dir: &Path,
        selector: &str,
        repo: Option<&str>,
    ) -> Result<ChangeView, ForgeError> {
        gh::pr_view_scoped(dir, selector, repo)
            .map(view_from_gh)
            .map_err(err_from_gh)
    }

    fn create_draft(
        &self,
        dir: &Path,
        title: &str,
        body: &str,
        base: Option<&str>,
    ) -> Result<ChangeView, ForgeError> {
        gh::pr_create_draft(dir, title, body, base)
            .map(view_from_gh)
            .map_err(err_from_gh)
    }

    fn mark_ready(&self, dir: &Path, number: u64, repo: Option<&str>) -> Result<(), ForgeError> {
        gh::pr_ready(dir, number, repo).map_err(err_from_gh)
    }

    fn comment(&self, dir: &Path, number: u64, body: &str) -> Result<(), ForgeError> {
        gh::pr_comment(dir, number, body).map_err(err_from_gh)
    }

    fn change_diff(
        &self,
        dir: &Path,
        number: u64,
        repo: Option<&str>,
    ) -> Result<String, ForgeError> {
        gh::pr_diff(dir, number, repo).map_err(err_from_gh)
    }

    fn unanswered_review_comments(
        &self,
        dir: &Path,
        number: u64,
        repo: Option<&str>,
    ) -> Result<u64, ForgeError> {
        gh::pr_unanswered_review_comments(dir, number, repo).map_err(err_from_gh)
    }
}

fn unsupported() -> ForgeError {
    ForgeError::Unsupported(
        "this repository has no supported forge — change-request features are \
         disabled (set `workflows.forge` in Settings to override)"
            .to_string(),
    )
}

impl Forge for NoForge {
    fn kind(&self) -> ForgeKind {
        ForgeKind::None
    }

    fn caps(&self) -> ForgeCaps {
        ForgeCaps {
            change_requests: false,
            drafts: false,
            review_comments: false,
        }
    }

    fn parse_change_url(&self, _url: &str) -> Option<(String, u64)> {
        None
    }

    fn view(&self, _: &Path, _: &str, _: Option<&str>) -> Result<ChangeView, ForgeError> {
        Err(unsupported())
    }

    fn create_draft(
        &self,
        _: &Path,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<ChangeView, ForgeError> {
        Err(unsupported())
    }

    fn mark_ready(&self, _: &Path, _: u64, _: Option<&str>) -> Result<(), ForgeError> {
        Err(unsupported())
    }

    fn comment(&self, _: &Path, _: u64, _: &str) -> Result<(), ForgeError> {
        Err(unsupported())
    }

    fn change_diff(&self, _: &Path, _: u64, _: Option<&str>) -> Result<String, ForgeError> {
        Err(unsupported())
    }

    fn unanswered_review_comments(
        &self,
        _: &Path,
        _: u64,
        _: Option<&str>,
    ) -> Result<u64, ForgeError> {
        Err(unsupported())
    }
}

// ── Detection ───────────────────────────────────────────────────────────

/// Pure: the host of a git remote URL, over the three spellings git accepts —
/// `https://host/…`, `ssh://[user@]host[:port]/…`, and the scp-like
/// `user@host:path`.
pub fn remote_host(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://"))
        .or_else(|| url.strip_prefix("git://"))
    {
        let authority = rest.split('/').next()?;
        let host = authority.rsplit('@').next()?;
        let host = host.split(':').next()?;
        return (!host.is_empty()).then(|| host.to_ascii_lowercase());
    }
    // scp-like: user@host:path (no scheme).
    if let Some((authority, _path)) = url.split_once(':') {
        let host = authority.rsplit('@').next()?;
        if !host.is_empty() && !host.contains('/') {
            return Some(host.to_ascii_lowercase());
        }
    }
    None
}

/// Pure: the forge for a remote host. Unknown hosts map to GitHub — see the
/// module docs for why (GHE setups worked before detection existed and must
/// keep working); only hosts recognizably another forge map to `None`.
pub fn kind_for_host(host: &str) -> ForgeKind {
    let h = host.to_ascii_lowercase();
    if h.contains("gitlab") || h.contains("bitbucket") {
        ForgeKind::None
    } else {
        ForgeKind::GitHub
    }
}

/// Detect the forge of a repository from its `origin` remote. Blocking (one
/// local `git remote get-url` — no network); callers cache per directory. A
/// repo with no origin degrades to GitHub, same rule as an unknown host.
pub fn detect(dir: &Path) -> ForgeKind {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(dir)
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let url = String::from_utf8_lossy(&o.stdout);
            remote_host(url.trim()).map_or(ForgeKind::GitHub, |h| kind_for_host(&h))
        }
        _ => ForgeKind::GitHub,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_host_covers_the_three_git_spellings() {
        for (url, host) in [
            ("https://github.com/acme/clash.git", "github.com"),
            ("http://github.com/acme/clash", "github.com"),
            ("git@github.com:acme/clash.git", "github.com"),
            ("ssh://git@github.com/acme/clash.git", "github.com"),
            (
                "ssh://git@gitlab.example.com:2222/g/p.git",
                "gitlab.example.com",
            ),
            ("git@gitlab.com:group/project.git", "gitlab.com"),
            ("GIT@GitHub.COM:o/r.git", "github.com"), // normalized
        ] {
            assert_eq!(remote_host(url).as_deref(), Some(host), "{}", url);
        }
        assert_eq!(remote_host(""), None);
        // A plain path (local remote) has no host.
        assert_eq!(remote_host("/srv/git/repo.git"), None);
    }

    #[test]
    fn host_mapping_is_conservative() {
        assert_eq!(kind_for_host("github.com"), ForgeKind::GitHub);
        // Recognizably another forge → None until it has an implementation.
        assert_eq!(kind_for_host("gitlab.com"), ForgeKind::None);
        assert_eq!(kind_for_host("gitlab.corp.example"), ForgeKind::None);
        assert_eq!(kind_for_host("bitbucket.org"), ForgeKind::None);
        // Unknown hosts keep the pre-port behavior (gh may be configured for
        // a GitHub Enterprise host) — the config override handles the rest.
        assert_eq!(kind_for_host("git.corp.example"), ForgeKind::GitHub);
    }

    #[test]
    fn no_forge_refuses_every_operation_and_offers_the_override() {
        let f = NoForge;
        assert_eq!(f.kind(), ForgeKind::None);
        assert!(!f.caps().change_requests);
        assert!(f
            .parse_change_url("https://github.com/o/r/pull/1")
            .is_none());
        let err = f.view(Path::new("."), "", None).unwrap_err();
        assert!(matches!(err, ForgeError::Unsupported(_)));
        assert!(err.to_string().contains("workflows.forge"), "{err}");
    }

    #[test]
    fn github_forge_delegates_url_parsing_to_gh() {
        let f = GithubForge;
        assert_eq!(f.kind(), ForgeKind::GitHub);
        assert!(f.caps().change_requests && f.caps().drafts && f.caps().review_comments);
        assert_eq!(
            f.parse_change_url("https://github.com/acme/clash/pull/42"),
            Some(("acme/clash".to_string(), 42))
        );
        assert_eq!(
            f.parse_change_url("https://gitlab.com/g/p/-/merge_requests/1"),
            None
        );
    }

    #[test]
    fn gh_errors_map_to_forge_errors() {
        assert!(matches!(
            err_from_gh(GhError::NotInstalled),
            ForgeError::NotInstalled("gh")
        ));
        assert!(matches!(
            err_from_gh(GhError::NotAuthenticated("x".into())),
            ForgeError::NotAuthenticated(_)
        ));
    }

    #[test]
    fn gh_views_normalize_their_state() {
        let v = view_from_gh(GhPrView {
            url: "https://github.com/o/r/pull/7".into(),
            number: 7,
            is_draft: true,
            state: "MERGED".into(),
            ..GhPrView::default()
        });
        assert_eq!(v.state, ChangeState::Merged);
        assert!(v.draft);
        assert_eq!(v.number, 7);
    }
}
