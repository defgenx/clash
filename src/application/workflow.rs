//! Pure workflow logic — no IO, unit-tested directly.
//!
//! Home of everything the Workflows feature computes without touching the
//! filesystem: slugs, line hashing, unified-diff parsing, annotation
//! re-anchoring, attention/notification transition detection, and the agent
//! kickoff prompt.

use std::collections::HashMap;

use crate::domain::workflow::{Annotation, DiffSide, WorkflowItem, WorkflowStatus};

// ── Slugs ───────────────────────────────────────────────────────────────

/// Turn a human title into a filesystem-safe item slug: lowercase ASCII
/// alphanumerics, runs of anything else collapse to a single `-`, trimmed.
/// Never emits path separators or `..`, and never returns an empty string
/// (falls back to `"item"`).
///
/// Colons are excluded by construction — the GUI embeds `project/slug` in
/// `view:workflow:<project>/<slug>` tab keys, where a `:` would break the
/// tab-owner parsing.
pub fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_dash = true; // suppress leading dashes
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "item".to_string()
    } else {
        out
    }
}

// ── Line hashing ────────────────────────────────────────────────────────

/// FNV-1a 64-bit over the *trimmed* line text, rendered as lowercase hex.
///
/// Trimming makes the hash robust to indentation changes and CRLF/LF
/// differences (`\r` is whitespace). Hand-rolled because clash has no hashing
/// crate and stability across versions matters more than speed here.
pub fn line_hash(line: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for b in line.trim().bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{:016x}", hash)
}

// ── Unified diff parsing ────────────────────────────────────────────────

/// Kind of a line inside a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Add,
    Del,
    Ctx,
}

/// One content line of a hunk with its resolved line numbers.
#[derive(Debug, Clone)]
pub struct DiffLineRec {
    pub kind: DiffLineKind,
    /// Line number in the old file (None for added lines).
    pub old_no: Option<u32>,
    /// Line number in the new file (None for deleted lines).
    pub new_no: Option<u32>,
    /// Line text without the leading `+`/`-`/` ` marker.
    pub text: String,
}

/// One `@@` hunk.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// The full `@@ -a,b +c,d @@ ctx` header line.
    pub header: String,
    pub lines: Vec<DiffLineRec>,
}

/// One file section of a unified diff.
#[derive(Debug, Clone, Default)]
pub struct DiffFile {
    /// Path on the old side (`""` for added files).
    pub old_path: String,
    /// Path on the new side (`""` for deleted files).
    pub new_path: String,
    /// Set when the diff declares `rename from X` — lets annotations follow
    /// renamed files.
    pub renamed_from: Option<String>,
    pub hunks: Vec<DiffHunk>,
}

impl DiffFile {
    /// Display path: the new path when present, else the old one.
    pub fn path(&self) -> &str {
        if self.new_path.is_empty() {
            &self.old_path
        } else {
            &self.new_path
        }
    }
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

/// Parse a unified diff (as produced by `git diff`) into files → hunks →
/// numbered lines. Pure; tolerant of unknown metadata lines (mode changes,
/// index lines, binary notices) — they are simply skipped.
pub fn parse_unified_diff(text: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut old_no: u32 = 0;
    let mut new_no: u32 = 0;
    let mut in_hunk = false;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            in_hunk = false;
            let mut file = DiffFile::default();
            // Best-effort paths from the header; `---`/`+++` refine them.
            if let Some((a, b)) = rest.split_once(' ') {
                file.old_path = strip_git_prefix(a).to_string();
                file.new_path = strip_git_prefix(b).to_string();
            }
            files.push(file);
            continue;
        }
        let Some(file) = files.last_mut() else {
            // Diff text that starts mid-stream (e.g. `git diff --no-prefix`
            // output without the git header): synthesize a file on `---`.
            if line.starts_with("--- ") {
                files.push(DiffFile::default());
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
                file.hunks.push(DiffHunk {
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
            hunk.lines.push(DiffLineRec {
                kind: DiffLineKind::Add,
                old_no: None,
                new_no: Some(new_no),
                text: text.to_string(),
            });
            new_no += 1;
        } else if let Some(text) = line.strip_prefix('-') {
            hunk.lines.push(DiffLineRec {
                kind: DiffLineKind::Del,
                old_no: Some(old_no),
                new_no: None,
                text: text.to_string(),
            });
            old_no += 1;
        } else if let Some(text) = line.strip_prefix(' ') {
            hunk.lines.push(DiffLineRec {
                kind: DiffLineKind::Ctx,
                old_no: Some(old_no),
                new_no: Some(new_no),
                text: text.to_string(),
            });
            old_no += 1;
            new_no += 1;
        } else if line.is_empty() {
            // An empty context line ("" instead of " ") — git emits these
            // for blank lines in some configurations.
            hunk.lines.push(DiffLineRec {
                kind: DiffLineKind::Ctx,
                old_no: Some(old_no),
                new_no: Some(new_no),
                text: String::new(),
            });
            old_no += 1;
            new_no += 1;
        }
        // `\ No newline at end of file` and any other metadata: skipped.
    }
    files
}

// ── Annotation anchoring ────────────────────────────────────────────────

/// An annotation resolved against the *current* diff.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchoredAnnotation {
    pub annotation: Annotation,
    /// Line number (on the annotation's side) in the current diff, when the
    /// anchor still resolves. `None` when orphaned.
    pub current_line: Option<u32>,
    /// The file the annotation resolves to in the current diff (follows
    /// renames). Falls back to the stored file when orphaned.
    pub current_file: String,
    /// True when the anchor no longer exists in the current diff. The GUI
    /// renders these in a per-file "unanchored" tray — never dropped.
    pub orphaned: bool,
}

/// Line number of `rec` on the given diff side, if it exists there.
fn side_line_no(rec: &DiffLineRec, side: DiffSide) -> Option<u32> {
    match side {
        DiffSide::Old => rec.old_no,
        DiffSide::New => rec.new_no,
    }
}

/// Resolve every annotation against the current diff.
///
/// Match order per annotation:
/// 1. exact `file + side + line` whose trimmed-content hash still matches;
/// 2. content-hash search within the same file (renames followed via
///    `rename from` headers), nearest to the stored line wins;
/// 3. no match → `orphaned: true`.
///
/// Annotations with an empty hash (hand-written files) get one computed from
/// `line_content` when available; with neither, only an exact line-number
/// match can anchor them.
pub fn anchor_annotations(
    files: &[DiffFile],
    annotations: &[Annotation],
) -> Vec<AnchoredAnnotation> {
    annotations
        .iter()
        .map(|ann| anchor_one(files, ann))
        .collect()
}

fn anchor_one(files: &[DiffFile], ann: &Annotation) -> AnchoredAnnotation {
    let hash = if !ann.line_content_hash.is_empty() {
        ann.line_content_hash.clone()
    } else if !ann.line_content.is_empty() {
        line_hash(&ann.line_content)
    } else {
        String::new()
    };

    // The annotated file in the current diff: direct path match first, then
    // follow a rename (old annotated path == `rename from`).
    let file = files
        .iter()
        .find(|f| f.path() == ann.file || f.old_path == ann.file)
        .or_else(|| {
            files
                .iter()
                .find(|f| f.renamed_from.as_deref() == Some(ann.file.as_str()))
        });

    let Some(file) = file else {
        return AnchoredAnnotation {
            annotation: ann.clone(),
            current_line: None,
            current_file: ann.file.clone(),
            orphaned: true,
        };
    };

    let lines = file.hunks.iter().flat_map(|h| h.lines.iter());

    // Pass 1: exact position, content still matching (or no hash to check).
    for rec in lines.clone() {
        if side_line_no(rec, ann.side) == Some(ann.line) {
            let content_ok = hash.is_empty() || line_hash(&rec.text) == hash;
            if content_ok {
                return AnchoredAnnotation {
                    annotation: ann.clone(),
                    current_line: Some(ann.line),
                    current_file: file.path().to_string(),
                    orphaned: false,
                };
            }
            break;
        }
    }

    // Pass 2: content search, nearest to the original line.
    if !hash.is_empty() {
        let mut best: Option<u32> = None;
        for rec in lines {
            let Some(no) = side_line_no(rec, ann.side) else {
                continue;
            };
            if line_hash(&rec.text) != hash {
                continue;
            }
            let better = match best {
                None => true,
                Some(b) => no.abs_diff(ann.line) < b.abs_diff(ann.line),
            };
            if better {
                best = Some(no);
            }
        }
        if let Some(no) = best {
            return AnchoredAnnotation {
                annotation: ann.clone(),
                current_line: Some(no),
                current_file: file.path().to_string(),
                orphaned: false,
            };
        }
    }

    AnchoredAnnotation {
        annotation: ann.clone(),
        current_line: None,
        current_file: file.path().to_string(),
        orphaned: true,
    }
}

// ── Attention ledger (notification transition detection) ───────────────

/// A workflow item that just entered a decision-needed state through an
/// external write (i.e. the agent, not a clash button).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionEvent {
    pub project: String,
    pub slug: String,
    pub title: String,
    pub status: WorkflowStatus,
}

/// Pure transition detector behind workflow notifications.
///
/// The GUI holds one of these in a mutex: every reload calls [`observe`];
/// every clash-side mutation calls [`record_local_write`] right after
/// persisting, so the watcher-triggered reload that follows sees no
/// transition — the user is never notified about their own click. Only
/// external (agent) writes produce events.
///
/// [`observe`]: AttentionLedger::observe
/// [`record_local_write`]: AttentionLedger::record_local_write
#[derive(Debug, Default)]
pub struct AttentionLedger {
    prev: HashMap<(String, String), WorkflowStatus>,
}

impl AttentionLedger {
    /// Pre-seed the ledger with a status clash itself just wrote.
    pub fn record_local_write(&mut self, project: &str, slug: &str, status: WorkflowStatus) {
        self.prev
            .insert((project.to_string(), slug.to_string()), status);
    }

    /// Compare the freshly-loaded items against the previous observation and
    /// return the items that transitioned *into* a `needs_attention` state.
    /// The first observation of an item seeds the ledger silently (no
    /// notification replay on app start).
    pub fn observe(&mut self, items: &[WorkflowItem]) -> Vec<AttentionEvent> {
        let mut events = Vec::new();
        for item in items {
            let key = (item.project.clone(), item.slug.clone());
            let status = item.meta.status;
            match self.prev.insert(key, status) {
                Some(prev) if prev != status && status.needs_attention() => {
                    events.push(AttentionEvent {
                        project: item.project.clone(),
                        slug: item.slug.clone(),
                        title: item.meta.title.clone(),
                        status,
                    });
                }
                _ => {}
            }
        }
        events
    }
}

// ── Agent kickoff prompt ────────────────────────────────────────────────

/// Build the initial prompt for a workflow agent session. The skill owns the
/// actual behavior; the prompt only routes it to the item directory and the
/// requested phase (`plan` | `revise` | `implement`).
pub fn build_agent_prompt(item_dir: &str, phase: &str) -> String {
    format!(
        "Use the clash-workflow skill. Workflow item directory: {}. Phase: {}.",
        item_dir, phase
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::AnnotationStatus;

    fn ann(file: &str, side: DiffSide, line: u32, content: &str) -> Annotation {
        Annotation {
            id: format!("a-{}-{}", file, line),
            file: file.to_string(),
            side,
            line,
            line_content: content.to_string(),
            line_content_hash: line_hash(content),
            body: "comment".into(),
            status: AnnotationStatus::Open,
            ..Annotation::default()
        }
    }

    // ── slugify ─────────────────────────────────────────────────────

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Auth Refactor"), "auth-refactor");
        assert_eq!(slugify("  Fix: PTY / locale bug!  "), "fix-pty-locale-bug");
        assert_eq!(slugify("émoji 🚀 title"), "moji-title");
        assert_eq!(slugify("---"), "item");
        assert_eq!(slugify(""), "item");
        assert_eq!(slugify("../evil"), "evil");
        assert!(!slugify("a:b/c\\d").contains([':', '/', '\\']));
    }

    // ── line_hash ───────────────────────────────────────────────────

    #[test]
    fn line_hash_trims_and_is_stable() {
        // Indentation and CRLF do not change the hash.
        assert_eq!(line_hash("let x = 1;"), line_hash("    let x = 1;"));
        assert_eq!(line_hash("let x = 1;"), line_hash("let x = 1;\r"));
        assert_ne!(line_hash("let x = 1;"), line_hash("let x = 2;"));
        // Known FNV-1a vector: empty input → offset basis.
        assert_eq!(line_hash(""), "cbf29ce484222325");
    }

    // ── parse_unified_diff ──────────────────────────────────────────

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
        let files = parse_unified_diff(SIMPLE_DIFF);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path(), "src/lib.rs");
        let lines = &files[0].hunks[0].lines;
        // ctx "keep one": old 10 / new 10
        assert_eq!(lines[0].old_no, Some(10));
        assert_eq!(lines[0].new_no, Some(10));
        // del "old line": old 11
        assert_eq!(lines[1].kind, DiffLineKind::Del);
        assert_eq!(lines[1].old_no, Some(11));
        assert_eq!(lines[1].new_no, None);
        // add "new line": new 11
        assert_eq!(lines[2].kind, DiffLineKind::Add);
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
        let files = parse_unified_diff(&text);
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
        let files = parse_unified_diff(text);
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
        let files = parse_unified_diff(text);
        assert_eq!(files[0].old_path, "");
        assert_eq!(files[0].path(), "new.rs");
        assert_eq!(files[0].hunks[0].lines[0].new_no, Some(1));
        assert_eq!(files[1].new_path, "");
        assert_eq!(files[1].path(), "gone.rs");
        assert_eq!(files[1].hunks[0].lines[1].old_no, Some(2));
    }

    // ── anchor_annotations ──────────────────────────────────────────

    #[test]
    fn anchor_exact_match() {
        let files = parse_unified_diff(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::New, 11, "new line");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_line, Some(11));
        assert_eq!(out[0].current_file, "src/lib.rs");
    }

    #[test]
    fn anchor_old_side() {
        let files = parse_unified_diff(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::Old, 11, "old line");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_line, Some(11));
    }

    #[test]
    fn anchor_reanchors_when_line_shifts() {
        // Same content, but the annotation was recorded at line 5 in an older
        // iteration; the line now lives at new 11.
        let files = parse_unified_diff(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::New, 5, "new line");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_line, Some(11));
    }

    #[test]
    fn anchor_orphans_when_content_gone() {
        let files = parse_unified_diff(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::New, 11, "content that vanished");
        let out = anchor_annotations(&files, &[a]);
        assert!(out[0].orphaned);
        assert_eq!(out[0].current_line, None);
        // File context is preserved for the orphan tray.
        assert_eq!(out[0].current_file, "src/lib.rs");
    }

    #[test]
    fn anchor_orphans_when_file_gone() {
        let files = parse_unified_diff(SIMPLE_DIFF);
        let a = ann("src/other.rs", DiffSide::New, 1, "whatever");
        let out = anchor_annotations(&files, &[a]);
        assert!(out[0].orphaned);
        assert_eq!(out[0].current_file, "src/other.rs");
    }

    #[test]
    fn anchor_follows_renames() {
        let text = "\
diff --git a/old_name.rs b/new_name.rs
rename from old_name.rs
rename to new_name.rs
--- a/old_name.rs
+++ b/new_name.rs
@@ -3 +3 @@
+let x = qux();
";
        let files = parse_unified_diff(text);
        let a = ann("old_name.rs", DiffSide::New, 3, "let x = qux();");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_file, "new_name.rs");
        assert_eq!(out[0].current_line, Some(3));
    }

    #[test]
    fn anchor_duplicate_lines_nearest_wins() {
        let text = "\
diff --git a/dup.rs b/dup.rs
--- a/dup.rs
+++ b/dup.rs
@@ -1,7 +1,7 @@
+let x = 1;
 a
 b
+let x = 1;
 c
 d
+let x = 1;
";
        let files = parse_unified_diff(text);
        // Duplicates live at new 1, 4, 7. Recorded at 5 → nearest is 4.
        let a = ann("dup.rs", DiffSide::New, 5, "let x = 1;");
        let out = anchor_annotations(&files, &[a]);
        assert_eq!(out[0].current_line, Some(4));
    }

    #[test]
    fn anchor_crlf_content_still_matches() {
        // The stored content came from a CRLF checkout; the diff has LF.
        let files = parse_unified_diff(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::New, 11, "new line\r");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
    }

    #[test]
    fn anchor_context_line_dropped_from_shrunken_hunk() {
        // The annotated context line fell out of the hunk in this iteration.
        let a = ann("src/lib.rs", DiffSide::New, 42, "some far away context");
        let out = anchor_annotations(&parse_unified_diff(SIMPLE_DIFF), &[a]);
        assert!(out[0].orphaned);
    }

    #[test]
    fn anchor_without_hash_uses_exact_line_only() {
        let files = parse_unified_diff(SIMPLE_DIFF);
        let mut a = ann("src/lib.rs", DiffSide::New, 11, "");
        a.line_content_hash = String::new();
        let out = anchor_annotations(&files, &[a]);
        // Line 11 exists on the new side → anchors by position alone.
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_line, Some(11));
    }

    // ── AttentionLedger ─────────────────────────────────────────────

    fn item(project: &str, slug: &str, status: WorkflowStatus) -> WorkflowItem {
        WorkflowItem {
            project: project.into(),
            slug: slug.into(),
            meta: crate::domain::workflow::WorkflowMeta {
                title: format!("{} title", slug),
                status,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn ledger_external_transition_notifies() {
        let mut ledger = AttentionLedger::default();
        // First sight seeds silently — even in an attention state.
        let first = ledger.observe(&[item("p", "a", WorkflowStatus::PlanReview)]);
        assert!(first.is_empty());

        // Agent moves it planning → plan-review externally.
        ledger.observe(&[item("p", "a", WorkflowStatus::Planning)]);
        let events = ledger.observe(&[item("p", "a", WorkflowStatus::PlanReview)]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, WorkflowStatus::PlanReview);
        assert_eq!(events[0].slug, "a");
    }

    #[test]
    fn ledger_local_write_is_suppressed() {
        let mut ledger = AttentionLedger::default();
        ledger.observe(&[item("p", "a", WorkflowStatus::Implementing)]);
        // Clash itself moves the item into diff-review (e.g. a test flow),
        // pre-seeding the ledger — the reload must stay silent.
        ledger.record_local_write("p", "a", WorkflowStatus::DiffReview);
        let events = ledger.observe(&[item("p", "a", WorkflowStatus::DiffReview)]);
        assert!(events.is_empty());
    }

    #[test]
    fn ledger_does_not_renotify_same_status() {
        let mut ledger = AttentionLedger::default();
        ledger.observe(&[item("p", "a", WorkflowStatus::Planning)]);
        assert_eq!(
            ledger
                .observe(&[item("p", "a", WorkflowStatus::PlanReview)])
                .len(),
            1
        );
        // Subsequent reloads with the unchanged status stay quiet.
        assert!(ledger
            .observe(&[item("p", "a", WorkflowStatus::PlanReview)])
            .is_empty());
        assert!(ledger
            .observe(&[item("p", "a", WorkflowStatus::PlanReview)])
            .is_empty());
    }

    #[test]
    fn ledger_non_attention_transition_is_silent() {
        let mut ledger = AttentionLedger::default();
        ledger.observe(&[item("p", "a", WorkflowStatus::Draft)]);
        assert!(ledger
            .observe(&[item("p", "a", WorkflowStatus::Planning)])
            .is_empty());
    }

    // ── build_agent_prompt ──────────────────────────────────────────

    #[test]
    fn prompt_routes_dir_and_phase() {
        let p = build_agent_prompt("/x/workflows/clash/auth", "revise");
        assert!(p.contains("clash-workflow skill"));
        assert!(p.contains("/x/workflows/clash/auth"));
        assert!(p.contains("Phase: revise."));
    }
}
