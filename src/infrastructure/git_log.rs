//! Repository history for the GUI's diff viewer and Files panel: the refs a
//! diff can be taken against, a file's (or the repo's) recent commits, and a
//! file's blame. Parsing is pure and unit-tested; the IO halves are thin.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use super::git::valid_rev;

/// Field and record separators of the `git log` format below — characters a
/// subject or an author name cannot contain.
const FS: char = '\u{1f}';
const RS: char = '\u{1e}';
const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%an%x1f%at%x1f%s%x1e";

/// The refs a diff can be compared with.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRefs {
    /// The checked-out branch; `None` on a detached HEAD.
    pub current: Option<String>,
    pub local: Vec<String>,
    /// `origin/main`, …; the symbolic `origin/HEAD` is left out.
    pub remote: Vec<String>,
    pub tags: Vec<String>,
}

/// One commit of a log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitInfo {
    pub sha: String,
    pub short: String,
    pub author: String,
    /// Seconds since the epoch (author date).
    pub time: i64,
    pub subject: String,
}

/// A file's blame: one commit id per line, and each commit once.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blame {
    pub lines: Vec<String>,
    pub commits: HashMap<String, BlameCommit>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlameCommit {
    pub author: String,
    pub time: i64,
    pub summary: String,
}

/// The all-zero id git blames uncommitted lines on.
pub const UNCOMMITTED: &str = "0000000000000000000000000000000000000000";

/// Pure: `git for-each-ref --format=%(refname)` into branches and tags,
/// keeping git's order (the caller sorts by recency).
pub fn parse_refs(out: &str, current: Option<String>, tag_cap: usize) -> GitRefs {
    let mut refs = GitRefs {
        current,
        ..GitRefs::default()
    };
    for line in out.lines().map(str::trim) {
        if let Some(b) = line.strip_prefix("refs/heads/") {
            refs.local.push(b.to_string());
        } else if let Some(r) = line.strip_prefix("refs/remotes/") {
            if !r.ends_with("/HEAD") {
                refs.remote.push(r.to_string());
            }
        } else if let Some(t) = line.strip_prefix("refs/tags/") {
            if refs.tags.len() < tag_cap {
                refs.tags.push(t.to_string());
            }
        }
    }
    refs
}

/// Pure: `git log` output in [`LOG_FORMAT`] into commits; a malformed record
/// is skipped rather than failing the list.
pub fn parse_log(out: &str) -> Vec<CommitInfo> {
    out.split(RS)
        .filter_map(|rec| {
            let rec = rec.trim_start_matches('\n');
            let f: Vec<&str> = rec.split(FS).collect();
            if f.len() < 5 || f[0].is_empty() {
                return None;
            }
            Some(CommitInfo {
                sha: f[0].to_string(),
                short: f[1].to_string(),
                author: f[2].to_string(),
                time: f[3].trim().parse().unwrap_or(0),
                subject: f[4..].join(" ").trim_end().to_string(),
            })
        })
        .collect()
}

/// Pure: `git blame --porcelain` into per-line commit ids. Porcelain names a
/// commit's metadata only the first time the commit appears, so it is
/// gathered once and looked up by id.
pub fn parse_blame(out: &str) -> Blame {
    let mut blame = Blame::default();
    let mut current: Option<String> = None;
    for line in out.lines() {
        // A tab starts the line's content, which the viewer already has.
        if line.starts_with('\t') {
            if let Some(sha) = current.take() {
                blame.lines.push(sha);
            }
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let head = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        if head.len() == 40 && head.chars().all(|c| c.is_ascii_hexdigit()) {
            blame.commits.entry(head.to_string()).or_default();
            current = Some(head.to_string());
            continue;
        }
        let Some(sha) = current.as_ref() else {
            continue;
        };
        let commit = blame.commits.entry(sha.clone()).or_default();
        match head {
            "author" => commit.author = rest.to_string(),
            "author-time" => commit.time = rest.trim().parse().unwrap_or(0),
            "summary" => commit.summary = rest.to_string(),
            _ => {}
        }
    }
    blame
}

async fn git_out(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The branches and (newest) tags of the repository at `dir`.
pub async fn refs(dir: &Path) -> Result<GitRefs, String> {
    let current = git_out(dir, &["symbolic-ref", "--short", "-q", "HEAD"])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let out = git_out(
        dir,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )
    .await?;
    Ok(parse_refs(&out, current, 50))
}

/// The newest `limit` commits of `rev` (HEAD when `None`), only those that
/// touched `path` when one is given (followed across renames).
pub async fn log(
    dir: &Path,
    rev: Option<&str>,
    path: Option<&str>,
    limit: usize,
) -> Result<Vec<CommitInfo>, String> {
    let n = format!("-n{}", limit.clamp(1, 500));
    let mut args = vec!["log", n.as_str(), LOG_FORMAT];
    if let Some(r) = rev {
        if !valid_rev(r) {
            return Err(format!("not a revision: {r}"));
        }
        args.push(r);
    }
    if let Some(p) = path {
        args.extend(["--follow", "--", p]);
    }
    Ok(parse_log(&git_out(dir, &args).await?))
}

/// Who last changed each line of `path` (relative to `dir`), working-tree
/// version — uncommitted lines blame on [`UNCOMMITTED`].
pub async fn blame(dir: &Path, path: &str) -> Result<Blame, String> {
    Ok(parse_blame(
        &git_out(dir, &["blame", "--porcelain", "--", path]).await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_split_into_branches_and_tags_without_origin_head() {
        let out = "refs/heads/main\nrefs/heads/feat/x\nrefs/remotes/origin/HEAD\nrefs/remotes/origin/main\nrefs/tags/v2\nrefs/tags/v1\n";
        let r = parse_refs(out, Some("feat/x".into()), 1);
        assert_eq!(r.current.as_deref(), Some("feat/x"));
        assert_eq!(r.local, ["main", "feat/x"]);
        assert_eq!(r.remote, ["origin/main"]);
        assert_eq!(r.tags, ["v2"], "capped, newest first as git sorted them");
    }

    #[test]
    fn log_records_survive_odd_subjects() {
        let out = format!(
            "aaa{FS}a1{FS}Ada{FS}1700000000{FS}fix: a | b {RS}\nbbb{FS}b1{FS}Bo{FS}1600000000{FS}init{RS}\ngarbage{RS}"
        );
        let log = parse_log(&out);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, "fix: a | b");
        assert_eq!(log[0].time, 1_700_000_000);
        assert_eq!(log[1].short, "b1");
    }

    #[test]
    fn blame_names_each_commit_once_and_every_line() {
        let a = "a".repeat(40);
        let out = format!(
            "{a} 1 1 2\nauthor Ada\nauthor-time 1700000000\nsummary first\nfilename f\n\tone\n{a} 2 2\n\ttwo\n{UNCOMMITTED} 3 3 1\nauthor Not Committed Yet\nauthor-time 1800000000\nsummary Version of f from f\nfilename f\n\tthree\n"
        );
        let b = parse_blame(&out);
        assert_eq!(b.lines, [a.clone(), a.clone(), UNCOMMITTED.to_string()]);
        assert_eq!(b.commits[&a].author, "Ada");
        assert_eq!(b.commits[&a].summary, "first");
        assert_eq!(b.commits[UNCOMMITTED].time, 1_800_000_000);
    }

    /// The IO halves against a real repository.
    #[test]
    fn refs_log_and_blame_read_a_real_repository() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(repo)
                .output()
                .expect("git");
            assert!(out.status.success(), "git {:?}: {:?}", args, out);
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "Tess"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "first"]);
        run(&["-c", "tag.gpgSign=false", "tag", "-a", "v1", "-m", "v1"]);
        run(&["checkout", "-qb", "feat"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        run(&["commit", "-qam", "second"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(refs(repo)).unwrap();
        assert_eq!(r.current.as_deref(), Some("feat"));
        assert!(r.local.contains(&"main".to_string()) && r.tags == ["v1"]);

        let l = rt.block_on(log(repo, None, Some("a.txt"), 10)).unwrap();
        assert_eq!(
            l.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert_eq!(l[0].author, "Tess");
        assert!(rt.block_on(log(repo, Some("-p"), None, 10)).is_err());

        let b = rt.block_on(blame(repo, "a.txt")).unwrap();
        assert_eq!(b.lines.len(), 3);
        assert_eq!(b.commits[&b.lines[0]].summary, "first");
        assert_eq!(b.commits[&b.lines[1]].summary, "second");
        assert_eq!(b.lines[2], UNCOMMITTED);
    }
}
