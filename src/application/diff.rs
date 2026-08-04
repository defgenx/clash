//! Unified-diff parsing — the single home for all Rust diff parsing.
//!
//! One walk over the raw text ([`parse_diff`]) produces two views:
//! - `files`: the structural view (files → hunks → numbered lines), consumed
//!   by workflow annotation anchoring;
//! - `display`: every raw line classified for styling, consumed by the TUI
//!   diff widget (via the [`parse_diff_lines`] truncation wrapper).
//!
//! Pure — no IO. The `git diff` subprocess lives in `infrastructure::git`.

use crate::application::state::{DiffFile, DiffLine, DiffLineKind};

/// Maximum number of raw diff lines the display view parses (truncate beyond).
pub const MAX_DIFF_LINES: usize = 10_000;

// ── Structural view ─────────────────────────────────────────────────────

/// Kind of a content line inside a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkLineKind {
    Add,
    Del,
    Ctx,
}

/// One content line of a hunk with its resolved line numbers.
#[derive(Debug, Clone)]
pub struct HunkLine {
    /// `dead_code` allowed: read by workflow anchoring (lib crate) and the
    /// GUI; the bin only uses the display view.
    #[allow(dead_code)]
    pub kind: HunkLineKind,
    /// Line number in the old file (None for added lines).
    pub old_no: Option<u32>,
    /// Line number in the new file (None for deleted lines).
    pub new_no: Option<u32>,
    /// Line text without the leading `+`/`-`/` ` marker.
    pub text: String,
}

/// One `@@` hunk.
#[derive(Debug, Clone)]
pub struct Hunk {
    /// The full `@@ -a,b +c,d @@ ctx` header line.
    /// `dead_code` allowed: read by the GUI's workflow diff view (lib crate).
    #[allow(dead_code)]
    pub header: String,
    pub lines: Vec<HunkLine>,
}

/// One file section of a unified diff (structural view).
#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    /// Path on the old side (`""` for added files).
    pub old_path: String,
    /// Path on the new side (`""` for deleted files).
    pub new_path: String,
    /// Set when the diff declares `rename from X` — lets annotations follow
    /// renamed files.
    pub renamed_from: Option<String>,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    /// Display path: the new path when present, else the old one.
    pub fn path(&self) -> &str {
        if self.new_path.is_empty() {
            &self.old_path
        } else {
            &self.new_path
        }
    }
}

/// Both views of a parsed diff, produced by one walk.
#[derive(Debug, Clone, Default)]
pub struct ParsedDiff {
    pub files: Vec<FileDiff>,
    pub display: Vec<DiffLine>,
}

/// Strip a `a/` or `b/` git prefix from a diff path.
fn strip_git_prefix(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

/// Parse `@@ -a[,b] +c[,d] @@ ...` into the two start line numbers.
fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old_part, rest) = rest.split_once(" +")?;
    let (new_part, _) = rest.split_once(" @@")?;
    let old_start = old_part.split(',').next()?.parse().ok()?;
    let new_start = new_part.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}

/// Line-local display classification (styling only — the structural state
/// machine below decides what a line *means*).
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

/// Parse a unified diff (as produced by `git diff`) in a single walk,
/// producing the structural and display views together. Tolerant of unknown
/// metadata lines — they appear in `display` but not in the structure.
pub fn parse_diff(text: &str) -> ParsedDiff {
    let mut out = ParsedDiff::default();
    let mut old_no: u32 = 0;
    let mut new_no: u32 = 0;
    let mut in_hunk = false;

    for line in text.lines() {
        out.display.push(DiffLine {
            kind: classify_line(line),
            content: line.to_string(),
        });

        if let Some(rest) = line.strip_prefix("diff --git ") {
            in_hunk = false;
            let mut file = FileDiff::default();
            // Best-effort paths from the header; `---`/`+++` refine them.
            if let Some((a, b)) = rest.split_once(' ') {
                file.old_path = strip_git_prefix(a).to_string();
                file.new_path = strip_git_prefix(b).to_string();
            }
            out.files.push(file);
            continue;
        }
        let Some(file) = out.files.last_mut() else {
            // Diff text that starts mid-stream (e.g. output without the git
            // header): synthesize a file on `---`.
            if line.starts_with("--- ") {
                out.files.push(FileDiff::default());
            }
            continue;
        };
        if !in_hunk {
            if let Some(p) = line.strip_prefix("rename from ") {
                file.renamed_from = Some(p.to_string());
                continue;
            }
            if let Some(p) = line.strip_prefix("rename to ") {
                file.new_path = p.to_string();
                continue;
            }
            if let Some(p) = line.strip_prefix("--- ") {
                file.old_path = if p == "/dev/null" {
                    String::new()
                } else {
                    strip_git_prefix(p).to_string()
                };
                continue;
            }
            if let Some(p) = line.strip_prefix("+++ ") {
                file.new_path = if p == "/dev/null" {
                    String::new()
                } else {
                    strip_git_prefix(p).to_string()
                };
                continue;
            }
        }
        if line.starts_with("@@") {
            if let Some((o, n)) = parse_hunk_header(line) {
                file.hunks.push(Hunk {
                    header: line.to_string(),
                    lines: Vec::new(),
                });
                old_no = o;
                new_no = n;
                in_hunk = true;
            }
            continue;
        }
        if !in_hunk {
            continue;
        }
        let Some(hunk) = file.hunks.last_mut() else {
            continue;
        };
        if let Some(text) = line.strip_prefix('+') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Add,
                old_no: None,
                new_no: Some(new_no),
                text: text.to_string(),
            });
            new_no += 1;
        } else if let Some(text) = line.strip_prefix('-') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Del,
                old_no: Some(old_no),
                new_no: None,
                text: text.to_string(),
            });
            old_no += 1;
        } else if let Some(text) = line.strip_prefix(' ') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Ctx,
                old_no: Some(old_no),
                new_no: Some(new_no),
                text: text.to_string(),
            });
            old_no += 1;
            new_no += 1;
        } else if line.is_empty() {
            // An empty context line ("" instead of " ") — git emits these
            // for blank lines in some configurations.
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Ctx,
                old_no: Some(old_no),
                new_no: Some(new_no),
                text: String::new(),
            });
            old_no += 1;
            new_no += 1;
        }
        // `\ No newline at end of file` and any other metadata: skipped.
    }
    out
}

/// Structural view only — files → hunks → numbered lines.
/// `dead_code` allowed: called by workflow anchoring and the GUI (lib crate);
/// the bin's TUI uses `parse_diff_lines`.
#[allow(dead_code)]
pub fn parse_file_diffs(text: &str) -> Vec<FileDiff> {
    parse_diff(text).files
}

// ── Display view (TUI) ──────────────────────────────────────────────────

/// Parse raw `git diff` output into typed display lines, truncated at
/// [`MAX_DIFF_LINES`] with a trailing marker.
pub fn parse_diff_lines(raw: &str) -> Vec<DiffLine> {
    if raw.is_empty() {
        return Vec::new();
    }

    let total = raw.lines().count();
    if total <= MAX_DIFF_LINES {
        return parse_diff(raw).display;
    }

    let truncated: String = raw
        .lines()
        .take(MAX_DIFF_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    let mut result = parse_diff(&truncated).display;
    result.push(DiffLine {
        kind: DiffLineKind::Meta,
        content: format!("(truncated — {} lines total)", total),
    });
    result
}

/// Extract file boundaries and change counts from parsed display lines.
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

// ── Diff generation ─────────────────────────────────────────────────────

/// Pure: produce a unified diff between two texts (LCS-based, 3 context
/// lines), headed `--- <old_label>` / `+++ <new_label>`.
///
/// Exists for diffing *item documents* (plan snapshots against the current
/// plan) where shelling out to `git diff --no-index` would drag temp-file
/// paths into the headers and an exit-code-1-on-difference quirk into the
/// caller. Documents are small; the O(n·m) LCS table is fine here — do not
/// point this at source trees.
///
/// `dead_code` allowed: consumed by the GUI (lib crate) only, like the
/// workflow port — the private-`mod` bin build never calls it.
#[allow(dead_code)]
pub fn unified_diff(old: &str, new: &str, old_label: &str, new_label: &str) -> String {
    const CTX: usize = 3;
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();

    // LCS table (suffix lengths).
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    // Walk into an edit script: (kind, old_idx, new_idx).
    #[derive(Clone, Copy, PartialEq)]
    enum Op {
        Ctx,
        Del,
        Add,
    }
    let mut script: Vec<(Op, usize, usize)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            script.push((Op::Ctx, i, j));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            script.push((Op::Del, i, j));
            i += 1;
        } else {
            script.push((Op::Add, i, j));
            j += 1;
        }
    }
    for k in i..a.len() {
        script.push((Op::Del, k, j));
    }
    for k in j..b.len() {
        script.push((Op::Add, i, k));
    }
    if !script.iter().any(|(op, _, _)| *op != Op::Ctx) {
        return String::new();
    }

    // Group changes into hunks with CTX lines of context.
    let mut out = format!("--- {}\n+++ {}\n", old_label, new_label);
    let change_idx: Vec<usize> = script
        .iter()
        .enumerate()
        .filter(|(_, (op, _, _))| *op != Op::Ctx)
        .map(|(k, _)| k)
        .collect();
    let mut hunk_start = 0usize;
    while hunk_start < change_idx.len() {
        let mut hunk_end = hunk_start;
        while hunk_end + 1 < change_idx.len()
            && change_idx[hunk_end + 1] - change_idx[hunk_end] <= CTX * 2
        {
            hunk_end += 1;
        }
        let lo = change_idx[hunk_start].saturating_sub(CTX);
        let hi = (change_idx[hunk_end] + CTX + 1).min(script.len());
        let slice = &script[lo..hi];
        let old_start = slice
            .iter()
            .find(|(op, _, _)| *op != Op::Add)
            .map(|(_, oi, _)| oi + 1)
            .unwrap_or_else(|| slice.first().map(|(_, oi, _)| oi + 1).unwrap_or(1));
        let new_start = slice
            .iter()
            .find(|(op, _, _)| *op != Op::Del)
            .map(|(_, _, ni)| ni + 1)
            .unwrap_or_else(|| slice.first().map(|(_, _, ni)| ni + 1).unwrap_or(1));
        let old_count = slice.iter().filter(|(op, _, _)| *op != Op::Add).count();
        let new_count = slice.iter().filter(|(op, _, _)| *op != Op::Del).count();
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_count, new_start, new_count
        ));
        for (op, oi, ni) in slice {
            match op {
                Op::Ctx => {
                    out.push(' ');
                    out.push_str(a[*oi]);
                }
                Op::Del => {
                    out.push('-');
                    out.push_str(a[*oi]);
                }
                Op::Add => {
                    out.push('+');
                    out.push_str(b[*ni]);
                }
            }
            out.push('\n');
        }
        hunk_start = hunk_end + 1;
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── unified_diff generation ─────────────────────────────────────

    #[test]
    fn unified_diff_identical_is_empty() {
        assert_eq!(unified_diff("a\nb\n", "a\nb\n", "old", "new"), "");
    }

    #[test]
    fn unified_diff_simple_change_roundtrips_through_the_parser() {
        let old = "# Plan\n\nstep one\nstep two\nstep three\ntail\n";
        let new = "# Plan\n\nstep one\nstep 2 (revised)\nstep three\ntail\n";
        let d = unified_diff(old, new, "plan.md (it.1)", "plan.md (current)");
        assert!(d.starts_with("--- plan.md (it.1)\n+++ plan.md (current)\n"));
        assert!(d.contains("-step two\n"));
        assert!(d.contains("+step 2 (revised)\n"));
        // Our own structural parser must be able to consume what we emit.
        let files = parse_file_diffs(&d);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[test]
    fn unified_diff_distant_changes_get_separate_hunks() {
        let old: String = (1..=40).map(|i| format!("line {}\n", i)).collect();
        let new = old
            .replace("line 3\n", "LINE 3\n")
            .replace("line 38\n", "LINE 38\n");
        let d = unified_diff(&old, &new, "a", "b");
        assert_eq!(d.matches("@@ ").count(), 2, "{}", d);
    }

    #[test]
    fn unified_diff_from_empty_old() {
        let d = unified_diff("", "first\nsecond\n", "a", "b");
        assert!(d.contains("+first\n+second\n"), "{}", d);
    }

    // ── display view (moved verbatim from the TUI diff widget) ─────

    #[test]
    fn test_empty_input() {
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

    // ── extract_files (moved verbatim from the TUI diff widget) ────

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

    // ── structural view (moved from application::workflow) ─────────

    const SIMPLE_DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,4 +10,5 @@ fn ctx()
 keep one
-old line
+new line
+added line
 keep two
";

    #[test]
    fn parse_simple_diff_numbers_lines() {
        let files = parse_file_diffs(SIMPLE_DIFF);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path(), "src/lib.rs");
        let lines = &files[0].hunks[0].lines;
        // ctx "keep one": old 10 / new 10
        assert_eq!(lines[0].old_no, Some(10));
        assert_eq!(lines[0].new_no, Some(10));
        // del "old line": old 11
        assert_eq!(lines[1].kind, HunkLineKind::Del);
        assert_eq!(lines[1].old_no, Some(11));
        assert_eq!(lines[1].new_no, None);
        // add "new line": new 11
        assert_eq!(lines[2].kind, HunkLineKind::Add);
        assert_eq!(lines[2].new_no, Some(11));
        // add "added line": new 12
        assert_eq!(lines[3].new_no, Some(12));
        // ctx "keep two": old 12 / new 13
        assert_eq!(lines[4].old_no, Some(12));
        assert_eq!(lines[4].new_no, Some(13));
    }

    #[test]
    fn parse_multi_hunk_and_multi_file() {
        let text = format!(
            "{}diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-x\n+y\n@@ -7,2 +7,2 @@\n z\n-w\n+v\n",
            SIMPLE_DIFF
        );
        let files = parse_file_diffs(&text);
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].hunks.len(), 2);
        assert_eq!(files[1].hunks[1].lines[0].old_no, Some(7));
    }

    #[test]
    fn parse_rename_headers() {
        let text = "\
diff --git a/old_name.rs b/new_name.rs
similarity index 90%
rename from old_name.rs
rename to new_name.rs
--- a/old_name.rs
+++ b/new_name.rs
@@ -1 +1 @@
-a
+b
";
        let files = parse_file_diffs(text);
        assert_eq!(files[0].renamed_from.as_deref(), Some("old_name.rs"));
        assert_eq!(files[0].path(), "new_name.rs");
    }

    #[test]
    fn parse_added_and_deleted_files() {
        let text = "\
diff --git a/new.rs b/new.rs
--- /dev/null
+++ b/new.rs
@@ -0,0 +1,2 @@
+one
+two
diff --git a/gone.rs b/gone.rs
--- a/gone.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-one
-two
";
        let files = parse_file_diffs(text);
        assert_eq!(files[0].old_path, "");
        assert_eq!(files[0].path(), "new.rs");
        assert_eq!(files[0].hunks[0].lines[0].new_no, Some(1));
        assert_eq!(files[1].new_path, "");
        assert_eq!(files[1].path(), "gone.rs");
        assert_eq!(files[1].hunks[0].lines[1].old_no, Some(2));
    }

    // ── one-walk consistency ────────────────────────────────────────

    #[test]
    fn both_views_come_from_the_same_walk() {
        let parsed = parse_diff(SIMPLE_DIFF);
        // Display view keeps every raw line; structural view has the file.
        assert_eq!(parsed.display.len(), SIMPLE_DIFF.lines().count());
        assert_eq!(parsed.files.len(), 1);
        // And extract_files over the display view agrees with the structure.
        let display_files = extract_files(&parsed.display);
        assert_eq!(display_files[0].path, parsed.files[0].path());
    }
}
