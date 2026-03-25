//! Shared formatting utilities for the adapter layer.
//!
//! Reusable helpers for status rendering, string truncation, and display
//! formatting used across multiple view files.

use ratatui::style::{Modifier, Style};

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
        SessionStatus::Idle => "○",
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
        SessionStatus::Idle => format!("{} IDLE", icon),
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
        SessionStatus::Idle => theme::STATUS_IDLE,
    };
    let base = Style::default().fg(color);
    if matches!(status, SessionStatus::Idle) {
        base
    } else {
        base.add_modifier(Modifier::BOLD)
    }
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

/// Detect if a project path is inside a git worktree.
/// Returns the worktree name if detected, None otherwise.
///
/// This reads `.git` to check if it's a file (worktree indicator) rather
/// than a directory (regular repo), and extracts the worktree name from
/// the gitdir reference.
pub fn detect_worktree(project_path: &str) -> Option<String> {
    if project_path.is_empty() {
        return None;
    }
    let git_path = std::path::Path::new(project_path).join(".git");
    if git_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&git_path) {
            if let Some(gitdir) = content.trim().strip_prefix("gitdir: ") {
                // "gitdir: /path/to/.git/worktrees/<name>" → extract <name>
                if let Some(name) = gitdir.rsplit('/').next() {
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
                return Some("yes".to_string());
            }
        }
        // .git is a file but couldn't parse — likely still a worktree
        Some("yes".to_string())
    } else {
        None
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
}
