use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::application::state::{AppState, DiffFile, DiffLine, DiffLineKind};
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

/// Extract file boundaries and change counts from parsed diff lines.
///
/// Scans for `DiffLineKind::Meta` lines starting with "diff --git" to find
/// file boundaries, then counts additions/deletions within each file's range.
pub fn extract_files(lines: &[DiffLine]) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut current_start: Option<usize> = None;
    let mut current_path: Option<String> = None;

    for (i, line) in lines.iter().enumerate() {
        if line.kind == DiffLineKind::Meta && line.content.starts_with("diff --git") {
            // Close the previous file entry
            if let (Some(start), Some(path)) = (current_start, current_path.take()) {
                let (additions, deletions) = count_changes(lines, start, i);
                files.push(DiffFile {
                    path,
                    start_line: start,
                    end_line: i,
                    additions,
                    deletions,
                });
            }
            // Extract path from "diff --git a/path b/path"
            let path = line
                .content
                .strip_prefix("diff --git a/")
                .and_then(|rest| {
                    // The format is "a/<path> b/<path>" — find the " b/" separator
                    rest.find(" b/").map(|pos| rest[..pos].to_string())
                })
                .unwrap_or_else(|| line.content.clone());
            current_start = Some(i);
            current_path = Some(path);
        }
    }

    // Close the last file entry
    if let (Some(start), Some(path)) = (current_start, current_path) {
        let (additions, deletions) = count_changes(lines, start, lines.len());
        files.push(DiffFile {
            path,
            start_line: start,
            end_line: lines.len(),
            additions,
            deletions,
        });
    }

    files
}

/// Count Add and Remove lines in the range `[start, end)`.
fn count_changes(lines: &[DiffLine], start: usize, end: usize) -> (usize, usize) {
    let mut additions = 0;
    let mut deletions = 0;
    for line in &lines[start..end] {
        match line.kind {
            DiffLineKind::Add => additions += 1,
            DiffLineKind::Remove => deletions += 1,
            _ => {}
        }
    }
    (additions, deletions)
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

    // Loading / empty states — render as full-width single panel
    if !state.diff.loaded {
        let title = format!(" Diff: {} {}", session_name, title_suffix);
        let block = Block::default()
            .title(title)
            .title_style(theme::title_style())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER_COLOR))
            .style(Style::default().bg(theme::BG));
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "  Loading...",
            Style::default().fg(theme::MUTED),
        )))
        .block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    if state.diff.lines.is_empty() || state.diff.files.is_empty() {
        let title = format!(" Diff: {} {}", session_name, title_suffix);
        let block = Block::default()
            .title(title)
            .title_style(theme::title_style())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER_COLOR))
            .style(Style::default().bg(theme::BG));
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "  No changes (working tree clean)",
            Style::default().fg(theme::MUTED),
        )))
        .block(block);
        frame.render_widget(paragraph, area);
        return;
    }

    // Two-panel layout: 25% file list, 75% diff content
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    // ── Left panel: file list ──
    let file_items: Vec<ListItem> = state
        .diff
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let label = format!(" {} [+{}/-{}]", f.path, f.additions, f.deletions);
            let style = if i == state.diff.selected_file {
                theme::selected_style()
            } else {
                Style::default().fg(theme::TEXT)
            };
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect();

    let files_block = Block::default()
        .title(" Files ")
        .title_style(theme::title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER_COLOR))
        .style(Style::default().bg(theme::BG));

    let file_list = List::new(file_items).block(files_block);
    frame.render_widget(file_list, chunks[0]);

    // ── Right panel: selected file's diff ──
    let selected_file = state.diff.files.get(state.diff.selected_file);

    let (diff_lines, file_path): (Vec<Line>, String) = if let Some(file) = selected_file {
        let slice = &state.diff.lines[file.start_line..file.end_line];
        let lines: Vec<Line> = slice
            .iter()
            .map(|dl| {
                Line::from(Span::styled(
                    format!("  {}", dl.content),
                    style_for_kind(&dl.kind),
                ))
            })
            .collect();
        (lines, file.path.clone())
    } else {
        (vec![], String::new())
    };

    let diff_title = format!(" {} {}", file_path, title_suffix);
    let total_lines = diff_lines.len() as u16;
    let visible_height = chunks[1].height.saturating_sub(2);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_offset = state.diff.file_scroll.min(max_scroll);

    let diff_block = Block::default()
        .title(diff_title)
        .title_style(theme::title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER_COLOR))
        .style(Style::default().bg(theme::BG));

    let paragraph = Paragraph::new(diff_lines)
        .block(diff_block)
        .scroll((scroll_offset, 0));

    frame.render_widget(paragraph, chunks[1]);

    if total_lines > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(max_scroll as usize).position(scroll_offset as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(Style::default().fg(theme::BORDER_DIM)),
            chunks[1],
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

    // ── extract_files tests ──────────────────────────────────

    #[test]
    fn test_extract_files_empty() {
        let files = extract_files(&[]);
        assert!(files.is_empty());
    }

    #[test]
    fn test_extract_files_no_diff_headers() {
        // Lines without any "diff --git" header yield no files
        let lines = vec![
            DiffLine {
                kind: DiffLineKind::Context,
                content: "some context".to_string(),
            },
            DiffLine {
                kind: DiffLineKind::Add,
                content: "+added".to_string(),
            },
        ];
        let files = extract_files(&lines);
        assert!(files.is_empty());
    }

    #[test]
    fn test_extract_files_single_file() {
        let raw = "\
diff --git a/foo.rs b/foo.rs
index abc..def 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,4 @@
 context
+added line
-removed line
 more context";
        let lines = parse_diff_lines(raw);
        let files = extract_files(&lines);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "foo.rs");
        assert_eq!(files[0].start_line, 0);
        assert_eq!(files[0].end_line, lines.len());
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);
    }

    #[test]
    fn test_extract_files_multi_file() {
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
@@ -1 +1,3 @@
-foo
+bar
+baz";
        let lines = parse_diff_lines(raw);
        let files = extract_files(&lines);
        assert_eq!(files.len(), 2);

        assert_eq!(files[0].path, "a.rs");
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);

        assert_eq!(files[1].path, "b.rs");
        assert_eq!(files[1].additions, 2);
        assert_eq!(files[1].deletions, 1);

        // Boundaries are contiguous
        assert_eq!(files[0].end_line, files[1].start_line);
        assert_eq!(files[1].end_line, lines.len());
    }

    #[test]
    fn test_extract_files_binary_file() {
        let raw = "\
diff --git a/img.png b/img.png
Binary files a/img.png and b/img.png differ";
        let lines = parse_diff_lines(raw);
        let files = extract_files(&lines);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "img.png");
        assert_eq!(files[0].additions, 0);
        assert_eq!(files[0].deletions, 0);
    }

    #[test]
    fn test_extract_files_new_file() {
        let raw = "\
diff --git a/new.rs b/new.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,2 @@
+line1
+line2";
        let lines = parse_diff_lines(raw);
        let files = extract_files(&lines);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "new.rs");
        assert_eq!(files[0].additions, 2);
        assert_eq!(files[0].deletions, 0);
    }
}
