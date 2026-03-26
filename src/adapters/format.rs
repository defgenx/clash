//! Shared formatting utilities for the adapter layer.
//!
//! Reusable helpers for status rendering, string truncation, and display
//! formatting used across multiple view files.

use ratatui::style::Style;

use crate::domain::entities::{Session, SessionStatus};
use crate::infrastructure::tui::theme;

// ── Status formatting ────────────────────────────────────────────

/// Ticks per animation frame. At ~10ms/tick this gives ~120ms per frame.
const TICKS_PER_STATUS_FRAME: usize = 12;

/// Returns an animated icon for a session status.
///
/// Active statuses (Thinking, Running, Starting, Prompting) cycle through
/// animation frames driven by the tick counter, giving visual feedback that
/// the session is alive. Static statuses (Waiting, Errored, Idle) use a
/// fixed icon.
pub fn status_icon(status: SessionStatus, tick: usize) -> &'static str {
    let frame = tick / TICKS_PER_STATUS_FRAME;
    match status {
        SessionStatus::Prompting => {
            // Blinking diamond — most urgent, demands user action
            const FRAMES: &[&str] = &["◆", "◇", "◆", "◇"];
            FRAMES[frame % FRAMES.len()]
        }
        SessionStatus::Thinking => {
            // Pulsing circle — Claude is reasoning
            const FRAMES: &[&str] = &["◌", "◎", "◉", "◎"];
            FRAMES[frame % FRAMES.len()]
        }
        SessionStatus::Running => {
            // Braille spinner — active tool execution
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            FRAMES[frame % FRAMES.len()]
        }
        SessionStatus::Starting => {
            // Filling circle — session booting up
            const FRAMES: &[&str] = &["○", "◔", "◑", "◕", "●", "◕", "◑", "◔"];
            FRAMES[frame % FRAMES.len()]
        }
        SessionStatus::Waiting => "◉",
        SessionStatus::Errored => "✗",
        SessionStatus::Stashed => "○",
    }
}

/// Returns a styled (text, style) pair for table cells.
pub fn status_cell(status: SessionStatus, tick: usize) -> (String, Style) {
    let icon = status_icon(status, tick);
    let label = status.to_string();
    let style = status_style(status);
    (format!("{} {}", icon, label), style)
}

/// Returns the display string for detail views (animated icon + label + context).
pub fn status_display(status: SessionStatus, tick: usize) -> String {
    let icon = status_icon(status, tick);
    match status {
        SessionStatus::Waiting => format!("{} WAITING FOR INPUT", icon),
        SessionStatus::Thinking => format!("{} THINKING", icon),
        SessionStatus::Running => format!("{} RUNNING", icon),
        SessionStatus::Starting => format!("{} STARTING", icon),
        SessionStatus::Prompting => format!("{} PROMPTING (approval needed)", icon),
        SessionStatus::Errored => format!("{} ERRORED (process died)", icon),
        SessionStatus::Stashed => format!("{} STASHED", icon),
    }
}

/// Returns the color style for a status.
pub fn status_style(status: SessionStatus) -> Style {
    let color = match status {
        SessionStatus::Waiting => theme::STATUS_WAITING,
        SessionStatus::Thinking => theme::STATUS_THINKING,
        SessionStatus::Running => theme::STATUS_RUNNING,
        SessionStatus::Starting => theme::STATUS_STARTING,
        SessionStatus::Prompting => theme::STATUS_PROMPTING,
        SessionStatus::Errored => theme::ERROR_COLOR,
        SessionStatus::Stashed => theme::STATUS_IDLE,
    };
    Style::default().fg(color)
}

// ── String utilities ─────────────────────────────────────────────

/// Truncate a string at a char boundary, appending a suffix if truncated.
/// Safe for all UTF-8 strings.
pub fn truncate(s: &str, max_chars: usize, suffix: &str) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        return s.to_string();
    }
    let take = max_chars.saturating_sub(suffix.chars().count());
    let truncated: String = s.chars().take(take).collect();
    format!("{}{}", truncated, suffix)
}

/// Display name for a session: the human-readable name if set, or a truncated ID.
pub fn session_display_name(session: &Session) -> &str {
    session
        .name
        .as_deref()
        .unwrap_or_else(|| short_id(&session.id, 8))
}

/// Shorten an ID to `max` characters. IDs are ASCII-safe (UUIDs, hashes).
pub fn short_id(id: &str, max: usize) -> &str {
    if id.len() > max {
        &id[..max]
    } else {
        id
    }
}

/// Return the value or "—" if empty.
pub fn or_dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

// ── Filesystem display helpers ──────────────────────────────────

/// Worktree metadata extracted from a `.git` file's gitdir reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// The worktree name (last component of the gitdir path).
    pub name: String,
    /// The parent project name (directory containing `.git`), if extractable.
    pub parent_project: Option<String>,
}

/// Parse the content of a `.git` file to extract worktree info.
///
/// Pure function — no filesystem access. Takes the raw content of a `.git` file
/// (e.g., `"gitdir: /path/to/project/.git/worktrees/wt-name"`) and extracts
/// the worktree name and parent project name.
pub fn parse_gitdir_content(content: &str) -> Option<WorktreeInfo> {
    let gitdir = content.trim().strip_prefix("gitdir: ")?;

    // Try to split on "/.git/worktrees/" to get both project path and worktree name
    if let Some((repo_path, wt_name)) = gitdir.rsplit_once("/.git/worktrees/") {
        if !wt_name.is_empty() {
            let parent_project = repo_path
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            return Some(WorktreeInfo {
                name: wt_name.to_string(),
                parent_project,
            });
        }
    }

    // Fallback: extract just the last path component as the name
    if let Some(name) = gitdir.rsplit('/').next() {
        if !name.is_empty() {
            return Some(WorktreeInfo {
                name: name.to_string(),
                parent_project: None,
            });
        }
    }

    Some(WorktreeInfo {
        name: "yes".to_string(),
        parent_project: None,
    })
}

/// Detect if a project path is inside a git worktree.
///
/// Reads `.git` to check if it's a file (worktree indicator) rather
/// than a directory (regular repo), and extracts worktree metadata
/// from the gitdir reference.
pub fn detect_worktree(project_path: &str) -> Option<WorktreeInfo> {
    if project_path.is_empty() {
        return None;
    }
    let git_path = std::path::Path::new(project_path).join(".git");
    if git_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&git_path) {
            return parse_gitdir_content(&content);
        }
        // .git is a file but couldn't read — likely still a worktree
        Some(WorktreeInfo {
            name: "yes".to_string(),
            parent_project: None,
        })
    } else {
        None
    }
}

/// Format a worktree name for display using stored fields.
///
/// Returns `"⊟ project/name"` when project is known, `"⊟ name"` otherwise.
pub fn worktree_display(name: &str, project: Option<&str>) -> String {
    match project {
        Some(proj) => format!("⊟ {}/{}", proj, name),
        None => format!("⊟ {}", name),
    }
}

/// Format a worktree display string by detecting worktree info from a cwd path.
///
/// Convenience function that combines `detect_worktree` + `worktree_display`.
/// Returns `"—"` if the path is not a worktree.
pub fn worktree_display_from_cwd(cwd: &str) -> String {
    match detect_worktree(cwd) {
        Some(info) => worktree_display(&info.name, info.parent_project.as_deref()),
        None => "—".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_display_name_with_name() {
        let session = Session {
            id: "abcdef1234567890".to_string(),
            name: Some("my-session".to_string()),
            ..Default::default()
        };
        assert_eq!(session_display_name(&session), "my-session");
    }

    #[test]
    fn session_display_name_without_name() {
        let session = Session {
            id: "abcdef1234567890".to_string(),
            name: None,
            ..Default::default()
        };
        assert_eq!(session_display_name(&session), "abcdef12");
    }

    // ── parse_gitdir_content tests ──────────────────────────────

    #[test]
    fn parse_gitdir_standard_worktree() {
        let content = "gitdir: /Users/user/repos/clash/.git/worktrees/wt-abc12345";
        let info = parse_gitdir_content(content).unwrap();
        assert_eq!(info.name, "wt-abc12345");
        assert_eq!(info.parent_project.as_deref(), Some("clash"));
    }

    #[test]
    fn parse_gitdir_nested_project_path() {
        let content = "gitdir: /home/dev/workspace/my-org/my-project/.git/worktrees/feature-branch";
        let info = parse_gitdir_content(content).unwrap();
        assert_eq!(info.name, "feature-branch");
        assert_eq!(info.parent_project.as_deref(), Some("my-project"));
    }

    #[test]
    fn parse_gitdir_with_trailing_newline() {
        let content = "gitdir: /Users/user/repos/clash/.git/worktrees/wt-abc\n";
        let info = parse_gitdir_content(content).unwrap();
        assert_eq!(info.name, "wt-abc");
        assert_eq!(info.parent_project.as_deref(), Some("clash"));
    }

    #[test]
    fn parse_gitdir_no_worktrees_segment() {
        // Submodule-style gitdir without /worktrees/ path
        let content = "gitdir: /Users/user/repos/parent/.git/modules/child";
        let info = parse_gitdir_content(content).unwrap();
        assert_eq!(info.name, "child");
        assert_eq!(info.parent_project, None);
    }

    #[test]
    fn parse_gitdir_empty_content() {
        assert_eq!(parse_gitdir_content(""), None);
    }

    #[test]
    fn parse_gitdir_no_prefix() {
        assert_eq!(parse_gitdir_content("not a gitdir reference"), None);
    }

    #[test]
    fn parse_gitdir_bare_prefix() {
        // "gitdir: " with nothing after — trim strips the trailing space,
        // leaving "gitdir:" which doesn't match the prefix. Returns None.
        assert_eq!(parse_gitdir_content("gitdir: "), None);
    }

    #[test]
    fn parse_gitdir_root_level_project() {
        // Project at filesystem root
        let content = "gitdir: /project/.git/worktrees/wt-1";
        let info = parse_gitdir_content(content).unwrap();
        assert_eq!(info.name, "wt-1");
        assert_eq!(info.parent_project.as_deref(), Some("project"));
    }

    // ── worktree_display tests ──────────────────────────────────

    #[test]
    fn worktree_display_with_project() {
        assert_eq!(worktree_display("wt-abc", Some("clash")), "⊟ clash/wt-abc");
    }

    #[test]
    fn worktree_display_without_project() {
        assert_eq!(worktree_display("wt-abc", None), "⊟ wt-abc");
    }

    #[test]
    fn worktree_display_from_cwd_non_worktree() {
        // A path that doesn't exist or has no .git file → "—"
        assert_eq!(worktree_display_from_cwd("/nonexistent/path"), "—");
    }

    #[test]
    fn worktree_display_from_cwd_empty() {
        assert_eq!(worktree_display_from_cwd(""), "—");
    }
}
