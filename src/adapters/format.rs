//! Shared formatting utilities for the adapter layer.
//!
//! Reusable helpers for status rendering, string truncation, and display
//! formatting used across multiple view files.

use ratatui::style::{Modifier, Style};

use crate::domain::entities::SessionStatus;
use crate::infrastructure::tui::theme;

// ── Status formatting ────────────────────────────────────────────

/// Returns (icon, label) for a session status — used in detail views.
pub fn status_icon(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Waiting => "◉",
        SessionStatus::Thinking => "◎",
        SessionStatus::Running => "●",
        SessionStatus::Starting => "⦿",
        SessionStatus::Prompting => "◉",
        SessionStatus::Errored => "✗",
        SessionStatus::Idle => "○",
    }
}

/// Returns a styled (text, style) pair for table cells.
pub fn status_cell(status: SessionStatus) -> (String, Style) {
    let icon = status_icon(status);
    let label = status.to_string();
    let style = status_style(status);
    (format!("{} {}", icon, label), style)
}

/// Returns the display string for detail views (icon + label + context).
pub fn status_display(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Waiting => "◉ WAITING FOR INPUT",
        SessionStatus::Thinking => "◎ THINKING",
        SessionStatus::Running => "● RUNNING",
        SessionStatus::Starting => "⦿ STARTING",
        SessionStatus::Prompting => "◉ PROMPTING (approval needed)",
        SessionStatus::Errored => "✗ ERRORED (process died)",
        SessionStatus::Idle => "○ IDLE",
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
        SessionStatus::Errored => ratatui::style::Color::Red,
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
