//! The forge port — clash's window onto the code host that owns an item's
//! change request.
//!
//! Everything the Workflows feature asks of a forge fits seven operations
//! (view, create-draft, mark-ready, comment, count unanswered review
//! comments, parse a change URL, capabilities). GitHub via the `gh` CLI is
//! the only implementation today; the port exists so a GitLab `glab`
//! implementation — or a forge-less repo — is a configuration, not a rewrite.
//! Implementations live in `infrastructure::forge`.
//!
//! On-disk compatibility: `ChangeState` serializes to the same uppercase
//! strings `gh` reports (`OPEN`/`MERGED`/`CLOSED`), so `meta.json.pr.state`
//! written before the port reads back unchanged.

use std::path::Path;

/// Which forge a repository talks to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ForgeKind {
    #[default]
    GitHub,
    /// No supported forge: every change-request operation reports itself as
    /// unsupported instead of failing deep inside a tool that was never going
    /// to work. The pipeline minus the PR stages is fully usable.
    None,
}

impl ForgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::None => "none",
        }
    }
}

impl std::fmt::Display for ForgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lifecycle state of a change request, normalized across forges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChangeState {
    Open,
    Merged,
    Closed,
    /// Never fetched, or a state string this clash doesn't know.
    #[default]
    Unknown,
}

impl ChangeState {
    /// Lenient parse of a forge's state string (today: `gh`'s).
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_uppercase().as_str() {
            "OPEN" => Self::Open,
            "MERGED" => Self::Merged,
            "CLOSED" => Self::Closed,
            _ => Self::Unknown,
        }
    }

    /// The canonical on-disk string — `gh`'s spelling, kept for compatibility
    /// with `meta.json.pr.state` written before the port existed. Unknown is
    /// empty, matching "never checked".
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Merged => "MERGED",
            Self::Closed => "CLOSED",
            Self::Unknown => "",
        }
    }
}

/// What a forge reports about one change request — the forge-neutral shape of
/// `gh pr view`.
#[derive(Debug, Clone, Default)]
pub struct ChangeView {
    pub url: String,
    pub number: u64,
    pub draft: bool,
    pub state: ChangeState,
    pub title: String,
    /// Branch the change request proposes (its head).
    pub head_ref: String,
    /// Branch it targets (its base).
    pub base_ref: String,
}

/// What a forge can do. UI affordances gate on these instead of letting a
/// button fail at click time inside a tool that was never going to work.
#[derive(Debug, Clone, Copy)]
pub struct ForgeCaps {
    /// Change requests (PRs/MRs) exist at all.
    pub change_requests: bool,
    /// Draft → ready lifecycle (GitHub/GitLab yes, Bitbucket no).
    pub drafts: bool,
    /// Line-level review comments that can be listed and answered.
    pub review_comments: bool,
}

/// Forge errors, normalized from the tool-specific ones (`GhError` today).
/// The GUI maps these to its degradation prefixes (`gh-unavailable:` …).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ForgeError {
    /// The forge's CLI tool is missing. Carries the tool name for the hint.
    #[error("{0} CLI not installed")]
    NotInstalled(&'static str),
    #[error("not authenticated: {0}")]
    NotAuthenticated(String),
    /// The operation makes no sense on this forge (e.g. any change-request
    /// call on [`ForgeKind::None`]).
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(String),
}

/// The port. Implementations are synchronous — every caller already runs
/// forge work on a blocking thread — and `Send + Sync` so one instance can
/// move into it.
pub trait Forge: Send + Sync {
    fn kind(&self) -> ForgeKind;
    fn caps(&self) -> ForgeCaps;

    /// `owner/repo` and change number from a change-request URL of this
    /// forge. `None` when the URL is not one of its change requests.
    fn parse_change_url(&self, url: &str) -> Option<(String, u64)>;

    /// Look up one change request. `selector` is a number, branch or URL;
    /// empty means "the change request of the current branch". `repo` scopes
    /// the lookup to an explicit `owner/repo` when the selector came from a
    /// URL naming one.
    fn view(
        &self,
        dir: &Path,
        selector: &str,
        repo: Option<&str>,
    ) -> Result<ChangeView, ForgeError>;

    /// Create a draft change request for the current branch (publishing the
    /// branch first when the forge tool requires it).
    fn create_draft(
        &self,
        dir: &Path,
        title: &str,
        body: &str,
        base: Option<&str>,
    ) -> Result<ChangeView, ForgeError>;

    /// Flip a draft to ready-for-review. `repo` scopes the call to an
    /// explicit `owner/repo` — required for a linked PR, whose repository is
    /// not the one `dir`'s remotes point at.
    fn mark_ready(&self, dir: &Path, number: u64, repo: Option<&str>) -> Result<(), ForgeError>;

    /// Post one top-level comment.
    fn comment(&self, dir: &Path, number: u64, body: &str) -> Result<(), ForgeError>;

    /// Count review-comment threads nobody has replied to. `repo` scopes the
    /// lookup to an explicit `owner/repo` — required for a linked PR, whose
    /// repository is not the one `dir`'s remotes point at.
    fn unanswered_review_comments(
        &self,
        dir: &Path,
        number: u64,
        repo: Option<&str>,
    ) -> Result<u64, ForgeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_state_parses_leniently_and_round_trips() {
        assert_eq!(ChangeState::parse("OPEN"), ChangeState::Open);
        assert_eq!(ChangeState::parse(" merged "), ChangeState::Merged);
        assert_eq!(ChangeState::parse("Closed"), ChangeState::Closed);
        assert_eq!(ChangeState::parse(""), ChangeState::Unknown);
        assert_eq!(ChangeState::parse("locked"), ChangeState::Unknown);
        // The on-disk strings are gh's — items written before the port must
        // read back unchanged.
        for (s, txt) in [
            (ChangeState::Open, "OPEN"),
            (ChangeState::Merged, "MERGED"),
            (ChangeState::Closed, "CLOSED"),
        ] {
            assert_eq!(s.as_str(), txt);
            assert_eq!(ChangeState::parse(txt), s);
        }
        // Unknown means "never checked" — the empty string pre-port items hold.
        assert_eq!(ChangeState::Unknown.as_str(), "");
    }

    #[test]
    fn forge_kind_strings() {
        assert_eq!(ForgeKind::GitHub.as_str(), "github");
        assert_eq!(ForgeKind::None.as_str(), "none");
    }
}
