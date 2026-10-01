//! The file explorer's decisions, pure: how a directory listing is ordered,
//! what `git status` says about each path, which files a fuzzy query finds,
//! and how a file should be previewed. The IO wrappers live in
//! `infrastructure::explorer`; the GUI's Files panel is the only consumer.
//! Full description: README → *Files*.

use std::path::{Component, Path, PathBuf};

/// One child of a listed directory. `rel` is the POSIX path relative to the
/// explorer root — the stable identifier every other call takes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplorerEntry {
    pub name: String,
    pub rel: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Matched by `.gitignore` (or another exclude source). Listed only when
    /// the panel asks for ignored entries, so they can be shown dimmed.
    pub ignored: bool,
    pub size: u64,
}

/// Folders first, then case-insensitive by name, ties broken by the exact
/// name so the order is total (`a` and `A` cannot swap between listings).
pub fn sort_entries(entries: &mut [ExplorerEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Names never listed, whatever the ignore rules say: git's own store.
pub fn always_hidden(name: &str) -> bool {
    name == ".git"
}

/// `rel` as a path under the root, or `None` when it would escape it
/// (absolute, `..`, a drive prefix). `""` is the root itself.
pub fn safe_rel(rel: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

/// Join a parent `rel` and a child name with `/`.
pub fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

/// What `git status` says about one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitMark {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

/// One `git status` entry, path relative to the explorer root. A path ending
/// in `/` is an untracked directory (git reports those whole).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub rel: String,
    pub mark: GitMark,
    /// The change is in the index (left column of `XY`).
    pub staged: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitSummary {
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<StatusEntry>,
}

fn mark_for(x: u8, y: u8) -> GitMark {
    match (x, y) {
        (b'?', b'?') => GitMark::Untracked,
        (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => GitMark::Conflicted,
        (b'R', _) | (_, b'R') | (b'C', _) => GitMark::Renamed,
        (b'A', _) => GitMark::Added,
        (b'D', _) | (_, b'D') => GitMark::Deleted,
        _ => GitMark::Modified,
    }
}

/// Parse `git status --porcelain=v1 -z -b` output.
///
/// Paths in that format are relative to the *repository* root, whatever the
/// cwd, so `prefix` (`git rev-parse --show-prefix`, e.g. `"gui/"`) is stripped
/// and entries outside it are dropped — the explorer root may be a subfolder.
/// A rename's `-z` record carries its source path as an extra field, skipped.
pub fn parse_status(out: &[u8], prefix: &str) -> GitSummary {
    let text = String::from_utf8_lossy(out);
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut summary = GitSummary::default();
    while let Some(field) = fields.next() {
        if let Some(head) = field.strip_prefix("## ") {
            parse_branch_header(head, &mut summary);
            continue;
        }
        let bytes = field.as_bytes();
        if bytes.len() < 4 {
            continue;
        }
        let (x, y) = (bytes[0], bytes[1]);
        if x == b'!' {
            continue;
        }
        if matches!(x, b'R' | b'C') {
            fields.next();
        }
        let Some(rel) = field[3..].strip_prefix(prefix) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        summary.entries.push(StatusEntry {
            rel: rel.to_string(),
            mark: mark_for(x, y),
            staged: !matches!(x, b' ' | b'?'),
        });
    }
    summary
}

/// `main...origin/main [ahead 1, behind 2]`, `No commits yet on main`,
/// `HEAD (no branch)`.
fn parse_branch_header(head: &str, summary: &mut GitSummary) {
    let (name, rest) = match head.split_once(" [") {
        Some((n, r)) => (n, r.trim_end_matches(']')),
        None => (head, ""),
    };
    let name = name.strip_prefix("No commits yet on ").unwrap_or(name);
    let local = name.split("...").next().unwrap_or(name).trim();
    if !local.is_empty() && !local.starts_with("HEAD (") {
        summary.branch = Some(local.to_string());
    }
    for part in rest.split(", ") {
        let mut it = part.split_whitespace();
        match (it.next(), it.next().and_then(|v| v.parse().ok())) {
            (Some("ahead"), Some(v)) => summary.ahead = v,
            (Some("behind"), Some(v)) => summary.behind = v,
            _ => {}
        }
    }
}

/// A file a fuzzy query matched, with the character indexes that matched so
/// the panel can bold them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    pub rel: String,
    pub score: i64,
    pub positions: Vec<usize>,
}

/// Subsequence match of `query` against `path`, case-insensitive, scored so
/// that matches in the file name, at word starts and in runs rank first, and
/// shorter paths win ties. `None` when not every query char appears in order.
/// Greedy and O(len): it tries the file name alone first, then the whole path.
pub fn fuzzy_score(query: &str, path: &str) -> Option<(i64, Vec<usize>)> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<char> = path.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let name_start = chars
        .iter()
        .rposition(|&c| c == '/')
        .map(|i| i + 1)
        .unwrap_or(0);

    // Prefer a match living entirely in the file name when there is one.
    let positions = match_from(&q, &lower, name_start).or_else(|| match_from(&q, &lower, 0))?;

    let mut score: i64 = 0;
    let mut prev: Option<usize> = None;
    for &i in &positions {
        score += 1;
        if i >= name_start {
            score += 4;
        }
        let boundary = i == 0
            || matches!(chars[i - 1], '/' | '_' | '-' | '.' | ' ')
            || (chars[i].is_uppercase() && chars[i - 1].is_lowercase());
        if boundary {
            score += 6;
        }
        if prev.is_some_and(|p| p + 1 == i) {
            score += 5;
        }
        prev = Some(i);
    }
    if positions.first() == Some(&name_start) {
        score += 8;
    }
    score -= chars.len() as i64 / 8;
    Some((score, positions))
}

fn match_from(q: &[char], lower: &[char], start: usize) -> Option<Vec<usize>> {
    let mut out = Vec::with_capacity(q.len());
    let mut qi = 0;
    for (i, c) in lower.iter().enumerate().skip(start) {
        if qi < q.len() && *c == q[qi] {
            out.push(i);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(out)
}

/// The best `limit` matches, best first; equal scores keep the shorter path.
pub fn rank_files<'a>(
    query: &str,
    files: impl IntoIterator<Item = &'a str>,
    limit: usize,
) -> Vec<FileMatch> {
    let mut out: Vec<FileMatch> = files
        .into_iter()
        .filter_map(|rel| {
            fuzzy_score(query, rel).map(|(score, positions)| FileMatch {
                rel: rel.to_string(),
                score,
                positions,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.rel.len().cmp(&b.rel.len()))
            .then_with(|| a.rel.cmp(&b.rel))
    });
    out.truncate(limit);
    out
}

/// How a file is shown in its preview tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewKind {
    Text,
    Markdown,
    Image,
    Binary,
    TooLarge,
}

/// Text previews above this are refused rather than truncated: a truncated
/// file reads as the whole file, and a 50MB log in a DOM node freezes the tab.
pub const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
/// Images ride the IPC as base64, so the cap is lower than it could be.
pub const MAX_IMAGE_BYTES: u64 = 15 * 1024 * 1024;

/// MIME type for the image extensions the preview renders, else `None`.
pub fn image_mime(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        // Rendered through <img>, which never runs an SVG's scripts.
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

pub fn is_markdown(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdx"))
}

/// A NUL in the first 8KB — git's own binary heuristic.
pub fn looks_binary(sample: &[u8]) -> bool {
    sample.iter().take(8192).any(|&b| b == 0)
}

/// The preview decision from what is known before reading the whole file.
pub fn classify(path: &str, size: u64, sample: &[u8]) -> PreviewKind {
    if image_mime(path).is_some() {
        return if size > MAX_IMAGE_BYTES {
            PreviewKind::TooLarge
        } else {
            PreviewKind::Image
        };
    }
    if looks_binary(sample) {
        return PreviewKind::Binary;
    }
    if size > MAX_TEXT_BYTES {
        return PreviewKind::TooLarge;
    }
    if is_markdown(path) {
        PreviewKind::Markdown
    } else {
        PreviewKind::Text
    }
}

/// Directories a non-git walk never descends into: dependency and build
/// trees that dwarf the source and that nobody fuzzy-finds into.
pub fn skip_in_walk(name: &str) -> bool {
    name.starts_with('.')
        || matches!(
            name,
            "node_modules" | "target" | "dist" | "build" | "__pycache__" | "vendor" | "venv"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> ExplorerEntry {
        ExplorerEntry {
            name: name.into(),
            rel: name.into(),
            is_dir,
            is_symlink: false,
            ignored: false,
            size: 0,
        }
    }

    #[test]
    fn folders_sort_first_then_case_insensitive_names() {
        let mut v = vec![
            entry("b.rs", false),
            entry("Zed", true),
            entry("a.rs", false),
            entry("src", true),
            entry("B.md", false),
        ];
        sort_entries(&mut v);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "Zed", "a.rs", "B.md", "b.rs"]);
    }

    #[test]
    fn rel_paths_cannot_escape_the_root() {
        assert_eq!(safe_rel(""), Some(PathBuf::new()));
        assert_eq!(safe_rel("src/app.rs"), Some(PathBuf::from("src/app.rs")));
        assert_eq!(safe_rel("./src"), Some(PathBuf::from("src")));
        assert_eq!(safe_rel("../etc/passwd"), None);
        assert_eq!(safe_rel("src/../../x"), None);
        assert_eq!(safe_rel("/etc/passwd"), None);
    }

    #[test]
    fn join_rel_handles_the_root() {
        assert_eq!(join_rel("", "a"), "a");
        assert_eq!(join_rel("src", "a"), "src/a");
        assert_eq!(join_rel("src/", "a"), "src/a");
    }

    #[test]
    fn status_marks_each_porcelain_code() {
        let out = b"## main...origin/main [ahead 2, behind 1]\0 M src/a.rs\0M  src/b.rs\0A  new.rs\0 D gone.rs\0?? scratch/\0UU both.rs\0R  to.rs\0from.rs\0";
        let s = parse_status(out, "");
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        let got: Vec<_> = s
            .entries
            .iter()
            .map(|e| (e.rel.as_str(), e.mark, e.staged))
            .collect();
        assert_eq!(
            got,
            [
                ("src/a.rs", GitMark::Modified, false),
                ("src/b.rs", GitMark::Modified, true),
                ("new.rs", GitMark::Added, true),
                ("gone.rs", GitMark::Deleted, false),
                ("scratch/", GitMark::Untracked, false),
                ("both.rs", GitMark::Conflicted, true),
                ("to.rs", GitMark::Renamed, true),
            ]
        );
    }

    #[test]
    fn status_paths_are_rebased_onto_a_subfolder_root() {
        let out = b"## main\0 M gui/dist/app.js\0 M src/lib.rs\0?? gui/new/\0";
        let s = parse_status(out, "gui/");
        let rels: Vec<_> = s.entries.iter().map(|e| e.rel.as_str()).collect();
        assert_eq!(rels, ["dist/app.js", "new/"]);
    }

    #[test]
    fn branch_header_variants() {
        assert_eq!(
            parse_status(b"## No commits yet on dev\0", "")
                .branch
                .as_deref(),
            Some("dev")
        );
        assert_eq!(parse_status(b"## HEAD (no branch)\0", "").branch, None);
        assert_eq!(
            parse_status(b"## feat/x\0", "").branch.as_deref(),
            Some("feat/x")
        );
    }

    #[test]
    fn fuzzy_requires_every_char_in_order() {
        assert!(fuzzy_score("apjs", "gui/dist/app.js").is_some());
        assert!(fuzzy_score("zz", "gui/dist/app.js").is_none());
        assert!(fuzzy_score("", "anything").is_some());
        assert!(fuzzy_score("APP", "gui/dist/app.js").is_some());
    }

    #[test]
    fn fuzzy_prefers_file_names_and_word_starts() {
        let ranked = rank_files(
            "app",
            [
                "src/application/mod.rs",
                "gui/dist/app.js",
                "a/p/p/readme.md",
            ],
            10,
        );
        assert_eq!(ranked[0].rel, "gui/dist/app.js");
        assert_eq!(ranked.len(), 3);
    }

    #[test]
    fn fuzzy_positions_point_into_the_name_when_possible() {
        let (_, pos) = fuzzy_score("rs", "src/reducer.rs").unwrap();
        // "r" of reducer, "s" of .rs — not the "s"/"r" of "src".
        assert!(pos.iter().all(|&i| i >= 4), "{pos:?}");
    }

    #[test]
    fn rank_truncates_and_breaks_ties_by_length() {
        let ranked = rank_files("a", ["xx/a", "x/a", "yyy/a"], 2);
        assert_eq!(
            ranked.iter().map(|m| m.rel.as_str()).collect::<Vec<_>>(),
            ["x/a", "xx/a"]
        );
    }

    #[test]
    fn classify_picks_the_preview() {
        assert_eq!(classify("a.png", 10, b"\x89PNG\0"), PreviewKind::Image);
        assert_eq!(classify("README.md", 10, b"# hi"), PreviewKind::Markdown);
        assert_eq!(classify("a.rs", 10, b"fn main"), PreviewKind::Text);
        assert_eq!(classify("a.bin", 10, b"ab\0cd"), PreviewKind::Binary);
        assert_eq!(
            classify("big.log", MAX_TEXT_BYTES + 1, b"x"),
            PreviewKind::TooLarge
        );
        assert_eq!(
            classify("huge.png", MAX_IMAGE_BYTES + 1, b""),
            PreviewKind::TooLarge
        );
    }

    #[test]
    fn git_store_is_always_hidden() {
        assert!(always_hidden(".git"));
        assert!(!always_hidden(".github"));
    }
}
