use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::application::state::{AppState, DiffLine, DiffLineKind};
use crate::infrastructure::tui::theme;

/// Maximum number of raw diff lines to parse (truncate beyond this).
const MAX_DIFF_LINES: usize = 10_000;

/// Parse raw `git diff` output into typed lines.
pub fn parse_diff_lines(raw: &str) -> Vec<DiffLine> {
    if raw.is_empty() {
        return Vec::new();
    }

    let raw_lines: Vec<&str> = raw.lines().collect();
    let truncated = raw_lines.len() > MAX_DIFF_LINES;
    let lines_to_parse = if truncated {
        &raw_lines[..MAX_DIFF_LINES]
    } else {
        &raw_lines[..]
    };

    let mut result: Vec<DiffLine> = lines_to_parse
        .iter()
        .map(|line| {
            let kind = classify_line(line);
            DiffLine {
                kind,
                content: line.to_string(),
            }
        })
        .collect();

    if truncated {
        result.push(DiffLine {
            kind: DiffLineKind::Meta,
            content: format!("(truncated — {} lines total)", raw_lines.len()),
        });
    }

    result
}

fn classify_line(line: &str) -> DiffLineKind {
    if line.starts_with("diff --git") || line.starts_with("index ") {
        DiffLineKind::Meta
    } else if line.starts_with("--- ") || line.starts_with("+++ ") {
        DiffLineKind::FilePath
    } else if line.starts_with("@@") {
        DiffLineKind::Hunk
    } else if line.starts_with('+') {
        DiffLineKind::Add
    } else if line.starts_with('-') {
        DiffLineKind::Remove
    } else if line.starts_with("Binary files")
        || line.starts_with("new file mode")
        || line.starts_with("deleted file mode")
        || line.starts_with("old mode")
        || line.starts_with("new mode")
        || line.starts_with("similarity index")
        || line.starts_with("rename from")
        || line.starts_with("rename to")
        || line.starts_with("copy from")
        || line.starts_with("copy to")
    {
        DiffLineKind::Meta
    } else {
        DiffLineKind::Context
    }
}

fn style_for_kind(kind: &DiffLineKind) -> Style {
    match kind {
        DiffLineKind::Add => Style::default().fg(theme::DIFF_ADD),
        DiffLineKind::Remove => Style::default().fg(theme::DIFF_REMOVE),
        DiffLineKind::Hunk => Style::default().fg(theme::DIFF_HUNK),
        DiffLineKind::Meta => Style::default()
            .fg(theme::DIFF_META)
            .add_modifier(Modifier::BOLD),
        DiffLineKind::FilePath => Style::default().fg(theme::TEXT_DIM),
        DiffLineKind::Context => Style::default().fg(theme::TEXT),
    }
}

pub fn render_diff(state: &AppState, frame: &mut Frame, area: Rect) {
    let session_name = state
        .diff
        .session_id
        .as_deref()
        .and_then(|id| state.store.find_session(id))
        .and_then(|s| s.name.clone())
        .unwrap_or_else(|| "?".to_string());

    let auto_refresh = state
        .diff
        .session_id
        .as_deref()
        .and_then(|id| state.store.find_session(id))
        .map(|s| s.is_running)
        .unwrap_or(false);

    let title_suffix = if auto_refresh { " [auto-refresh]" } else { "" };
    let title = format!(" Diff: {} {}", session_name, title_suffix);

    let lines: Vec<Line> = if !state.diff.loaded {
        vec![Line::from(Span::styled(
            "  Loading...",
            Style::default().fg(theme::MUTED),
        ))]
    } else if state.diff.lines.is_empty() {
        vec![Line::from(Span::styled(
            "  No changes (working tree clean)",
            Style::default().fg(theme::MUTED),
        ))]
    } else {
        state
            .diff
            .lines
            .iter()
            .map(|dl| {
                Line::from(Span::styled(
                    format!("  {}", dl.content),
                    style_for_kind(&dl.kind),
                ))
            })
            .collect()
    };

    let total_lines = lines.len() as u16;
    let visible_height = area.height.saturating_sub(2);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_offset = state.scroll_state.offset.min(max_scroll);

    let block = Block::default()
        .title(title)
        .title_style(theme::title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER_COLOR))
        .style(Style::default().bg(theme::BG));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll_offset, 0));

    frame.render_widget(paragraph, area);

    if total_lines > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(max_scroll as usize).position(scroll_offset as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(Style::default().fg(theme::BORDER_DIM)),
            area,
            &mut scrollbar_state,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        let result = parse_diff_lines("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_clean_repo() {
        let result = parse_diff_lines("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_single_hunk() {
        let raw = "\
diff --git a/foo.rs b/foo.rs
index abc1234..def5678 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,4 @@
 fn main() {
+    println!(\"hello\");
-    println!(\"old\");
     // done
 }";
        let lines = parse_diff_lines(raw);
        assert_eq!(lines[0].kind, DiffLineKind::Meta); // diff --git
        assert_eq!(lines[1].kind, DiffLineKind::Meta); // index
        assert_eq!(lines[2].kind, DiffLineKind::FilePath); // ---
        assert_eq!(lines[3].kind, DiffLineKind::FilePath); // +++
        assert_eq!(lines[4].kind, DiffLineKind::Hunk); // @@
        assert_eq!(lines[5].kind, DiffLineKind::Context); // fn main()
        assert_eq!(lines[6].kind, DiffLineKind::Add); // +println
        assert_eq!(lines[7].kind, DiffLineKind::Remove); // -println
        assert_eq!(lines[8].kind, DiffLineKind::Context); // // done
        assert_eq!(lines[9].kind, DiffLineKind::Context); // }
    }

    #[test]
    fn test_multi_file_diff() {
        let raw = "\
diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -1 +1 @@
-old
+new
diff --git a/b.rs b/b.rs
index 333..444 100644
--- a/b.rs
+++ b/b.rs
@@ -1 +1 @@
-foo
+bar";
        let lines = parse_diff_lines(raw);
        let meta_count = lines
            .iter()
            .filter(|l| l.kind == DiffLineKind::Meta)
            .count();
        assert_eq!(meta_count, 4); // 2 diff --git + 2 index lines
    }

    #[test]
    fn test_binary_file_marker() {
        let raw = "\
diff --git a/img.png b/img.png
Binary files a/img.png and b/img.png differ";
        let lines = parse_diff_lines(raw);
        assert_eq!(lines[0].kind, DiffLineKind::Meta);
        assert_eq!(lines[1].kind, DiffLineKind::Meta); // Binary files
    }

    #[test]
    fn test_plus_inside_context() {
        // A context line that happens to contain a + character should be Context, not Add
        let raw = "\
@@ -1,3 +1,3 @@
 a + b = c";
        let lines = parse_diff_lines(raw);
        assert_eq!(lines[0].kind, DiffLineKind::Hunk);
        assert_eq!(lines[1].kind, DiffLineKind::Context); // starts with space
    }

    #[test]
    fn test_file_path_vs_add_remove() {
        let raw = "\
--- a/foo.rs
+++ b/foo.rs
-removed
+added";
        let lines = parse_diff_lines(raw);
        assert_eq!(lines[0].kind, DiffLineKind::FilePath); // --- a/foo.rs
        assert_eq!(lines[1].kind, DiffLineKind::FilePath); // +++ b/foo.rs
        assert_eq!(lines[2].kind, DiffLineKind::Remove); // -removed
        assert_eq!(lines[3].kind, DiffLineKind::Add); // +added
    }

    #[test]
    fn test_truncation() {
        let raw: String = (0..10_005)
            .map(|i| format!(" line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = parse_diff_lines(&raw);
        assert_eq!(lines.len(), MAX_DIFF_LINES + 1); // 10000 + truncation marker
        assert!(lines.last().unwrap().content.contains("truncated"));
        assert_eq!(lines.last().unwrap().kind, DiffLineKind::Meta);
    }

    #[test]
    fn test_new_file_mode() {
        let raw = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/new.rs
@@ -0,0 +1 @@
+hello";
        let lines = parse_diff_lines(raw);
        assert_eq!(lines[0].kind, DiffLineKind::Meta); // diff --git
        assert_eq!(lines[1].kind, DiffLineKind::Meta); // new file mode
        assert_eq!(lines[2].kind, DiffLineKind::Meta); // index
        assert_eq!(lines[3].kind, DiffLineKind::FilePath); // --- /dev/null
        assert_eq!(lines[4].kind, DiffLineKind::FilePath); // +++ b/new.rs
        assert_eq!(lines[5].kind, DiffLineKind::Hunk); // @@
        assert_eq!(lines[6].kind, DiffLineKind::Add); // +hello
    }
}
