//! Shared `git diff` subprocess helper — the single place clash shells out
//! for diff text. Used by the TUI's `Effect::LoadDiff`, the GUI's session
//! diff tab, and the workflow diff view.

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
