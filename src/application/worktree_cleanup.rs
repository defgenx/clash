//! Which git worktrees can be removed without losing anything. Pure.
//!
//! One answer for every caller: the ⌫ "Clean up worktrees…" action, an item
//! reaching `done`, and `clash worktrees clean`. Worktrees are found by asking
//! git (`git worktree list --porcelain`), never by folder name — a name pattern
//! catches folders that are not worktrees and misses ones stored elsewhere.
//! A removal deletes the folder only; the branch stays. Contract:
//! `docs/workflows.md` → *Worktree cleanup*.

use std::path::Path;

/// One record of `git worktree list --porcelain`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    /// `None` when detached.
    pub branch: Option<String>,
    /// The repository's own checkout (the first record) — never removed.
    pub main: bool,
    /// `git worktree lock`ed: its owner asked git to keep it.
    pub locked: bool,
    /// Registered, but its folder is gone: only the registration is left.
    pub prunable: bool,
}

/// Pure: every record of `git worktree list --porcelain`, the main checkout
/// first, detached and locked ones included.
pub fn parse_porcelain(porcelain: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    for line in porcelain.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            out.push(Worktree {
                path: p.trim().to_string(),
                main: out.is_empty(),
                ..Worktree::default()
            });
            continue;
        }
        let Some(w) = out.last_mut() else { continue };
        if let Some(r) = line.strip_prefix("branch ") {
            let r = r.trim();
            w.branch = Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string());
        } else if line == "locked" || line.starts_with("locked ") {
            w.locked = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            w.prunable = true;
        } else if line == "bare" {
            // A bare repository's record is its git dir, not a checkout.
            w.main = true;
        }
    }
    out
}

/// What git says about one worktree's content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    /// Uncommitted or untracked paths; `None` when git could not tell.
    pub dirty: Option<usize>,
    /// Commits reachable from its HEAD that no remote has; `None` when git
    /// could not tell.
    pub unpushed: Option<u64>,
}

/// Who else depends on a worktree.
#[derive(Debug, Clone, Default)]
pub struct Guards {
    /// Open workflow items: (label, recorded worktree, branch names it owns).
    /// A branch owns its wave branches too (`<branch>-<wave>`).
    pub items: Vec<(String, Option<String>, Vec<String>)>,
    /// Working directories of running agent sessions: (label, cwd).
    pub sessions: Vec<(String, String)>,
}

/// Why a worktree stays, or that it can go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Clean, pushed, unused: removing it loses nothing.
    Remove,
    /// Its folder is already gone: only git's registration is pruned.
    Prune,
    /// The repository's own checkout.
    Main,
    Locked,
    /// An open workflow item uses it.
    Item(String),
    /// A running session works in it.
    Session(String),
    Dirty(usize),
    Unpushed(u64),
    /// git could not say whether it is clean and pushed — kept, since
    /// "unknown" is not "nothing to lose".
    Unknown,
}

impl Verdict {
    /// Whether a cleanup acts on it.
    pub fn removable(&self) -> bool {
        matches!(self, Verdict::Remove | Verdict::Prune)
    }

    /// Kept only until a session ends: worth checking again later.
    pub fn waits(&self) -> bool {
        matches!(self, Verdict::Session(_))
    }

    /// One line for the preview and the CLI.
    pub fn reason(&self) -> String {
        match self {
            Verdict::Remove => "clean and pushed".into(),
            Verdict::Prune => "folder already gone".into(),
            Verdict::Main => "the repository's own checkout".into(),
            Verdict::Locked => "locked (git worktree lock)".into(),
            Verdict::Item(i) => format!("used by the open workflow item {i}"),
            Verdict::Session(s) => format!("a running session works in it ({s})"),
            Verdict::Dirty(n) => format!("{n} uncommitted change{}", plural(*n as u64)),
            Verdict::Unpushed(n) => format!("{n} commit{} on no remote", plural(*n)),
            Verdict::Unknown => "git could not say whether it is clean and pushed".into(),
        }
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Pure: `child` is `parent` or inside it (whole path components).
pub fn path_within(child: &str, parent: &str) -> bool {
    let (c, p) = (child.trim_end_matches('/'), parent.trim_end_matches('/'));
    !p.is_empty() && (c == p || Path::new(c).starts_with(Path::new(p)))
}

/// Pure: the branches an open item owns: its own (and its slug), plus the
/// head of every PR it tracks — a wave's branch is often named after its
/// ticket, not `<item branch>-<id>`.
pub fn item_branches(meta: &crate::domain::workflow::WorkflowMeta, slug: String) -> Vec<String> {
    let mut out = vec![meta.branch.clone(), slug];
    out.extend(
        meta.pr
            .iter()
            .chain(meta.linked_prs.iter())
            .map(|p| p.head_ref.clone())
            .filter(|h| !h.is_empty()),
    );
    out
}

/// Pure: whether an item owning `branches` uses a worktree on `branch` at
/// `path` — its own branch or a wave branch of it, checked out there or
/// named after it.
fn item_owns(branches: &[String], branch: Option<&str>, path: &str) -> bool {
    let dir = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let names_it = |name: &str, b: &str| name == b || name.starts_with(&format!("{b}-"));
    branches
        .iter()
        .filter(|b| !b.is_empty())
        .any(|b| branch.is_some_and(|name| names_it(name, b)) || names_it(dir, b))
}

/// Pure: the verdict for one worktree. The order is the design: an open
/// item's worktree is kept even when its folder is gone, since git's record
/// of what was checked out there is what restoring it starts from; any other
/// gone folder has nothing to lose; ownership (git's lock, a live session)
/// comes before content, since those are kept even when clean; then
/// uncommitted work, then unpushed commits, and an unknown answer keeps.
pub fn verdict(w: &Worktree, facts: Facts, guards: &Guards) -> Verdict {
    if w.main {
        return Verdict::Main;
    }
    for (label, recorded, branches) in &guards.items {
        let at = recorded.as_deref().is_some_and(|r| path_within(&w.path, r));
        if at || item_owns(branches, w.branch.as_deref(), &w.path) {
            return Verdict::Item(label.clone());
        }
    }
    if w.prunable {
        return Verdict::Prune;
    }
    if w.locked {
        return Verdict::Locked;
    }
    if let Some((label, _)) = guards
        .sessions
        .iter()
        .find(|(_, cwd)| path_within(cwd, &w.path))
    {
        return Verdict::Session(label.clone());
    }
    match (facts.dirty, facts.unpushed) {
        (Some(n), _) if n > 0 => Verdict::Dirty(n),
        (_, Some(n)) if n > 0 => Verdict::Unpushed(n),
        (Some(0), Some(0)) => Verdict::Remove,
        _ => Verdict::Unknown,
    }
}

/// Pure: the branches to offer when an item's worktree folder is gone, most
/// likely first, each with why it is offered: what git still records as
/// checked out there, then each PR's head, then the item's own branch.
// GUI-only (`workflow_worktree_restore_choices`): unused by the bin's private `mod`.
#[allow(dead_code)]
pub fn restore_choices(
    recorded: Option<&str>,
    meta: &crate::domain::workflow::WorkflowMeta,
) -> Vec<(String, &'static str)> {
    let prs = meta.pr.iter().chain(meta.linked_prs.iter());
    let all = recorded
        .map(|b| (b.to_string(), "checked out there last"))
        .into_iter()
        .chain(prs.map(|p| (p.head_ref.clone(), "a PR's branch")))
        .chain(std::iter::once((meta.branch.clone(), "the item's branch")));
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for (b, why) in all {
        if !b.trim().is_empty() && !out.iter().any(|(o, _)| *o == b) {
            out.push((b, why));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_owns_the_head_branch_of_every_pr_it_tracks() {
        use crate::domain::workflow::{WorkflowMeta, WorkflowPr};
        let pr = |h: &str| WorkflowPr {
            head_ref: h.into(),
            ..WorkflowPr::default()
        };
        let meta = WorkflowMeta {
            branch: "item".into(),
            pr: Some(pr("feat/tt-1-a-w1")),
            linked_prs: vec![pr("fix/tt-2-b-w2"), pr("")],
            ..WorkflowMeta::default()
        };
        assert_eq!(
            item_branches(&meta, "slug".into()),
            ["item", "slug", "feat/tt-1-a-w1", "fix/tt-2-b-w2"]
        );
    }

    #[test]
    fn a_lost_worktree_offers_its_last_branch_first_then_the_prs() {
        use crate::domain::workflow::{WorkflowMeta, WorkflowPr};
        let pr = |h: &str| WorkflowPr {
            head_ref: h.into(),
            ..WorkflowPr::default()
        };
        let meta = WorkflowMeta {
            branch: "item".into(),
            linked_prs: vec![pr("w1"), pr(""), pr("w3")],
            ..WorkflowMeta::default()
        };
        let names = |c: Vec<(String, &str)>| c.into_iter().map(|(b, _)| b).collect::<Vec<_>>();
        assert_eq!(
            names(restore_choices(Some("w3"), &meta)),
            ["w3", "w1", "item"]
        );
        assert_eq!(names(restore_choices(None, &meta)), ["w1", "w3", "item"]);
    }

    const LIST: &str = "\
worktree /w/repo
HEAD a1
branch refs/heads/main

worktree /w/repo-worktrees/feat-x
HEAD b2
branch refs/heads/feat/x

worktree /w/repo-worktrees/loose
HEAD c3
detached

worktree /w/repo-worktrees/kept
HEAD d4
branch refs/heads/kept
locked reason here

worktree /w/repo-worktrees/gone
HEAD e5
branch refs/heads/gone
prunable gitdir file points to non-existent location
";

    fn clean() -> Facts {
        Facts {
            dirty: Some(0),
            unpushed: Some(0),
        }
    }

    #[test]
    fn every_record_is_parsed_with_its_flags() {
        let w = parse_porcelain(LIST);
        assert_eq!(w.len(), 5);
        assert!(w[0].main && !w[1].main);
        assert_eq!(w[1].branch.as_deref(), Some("feat/x"));
        assert_eq!(w[2].branch, None, "detached");
        assert!(w[3].locked);
        assert!(w[4].prunable);
    }

    #[test]
    fn only_a_clean_pushed_unused_worktree_is_removed() {
        let w = parse_porcelain(LIST);
        let g = Guards::default();
        assert_eq!(verdict(&w[0], clean(), &g), Verdict::Main);
        assert_eq!(verdict(&w[1], clean(), &g), Verdict::Remove);
        assert_eq!(verdict(&w[3], clean(), &g), Verdict::Locked);
        assert_eq!(verdict(&w[4], Facts::default(), &g), Verdict::Prune);
        let dirty = Facts {
            dirty: Some(3),
            unpushed: Some(0),
        };
        assert_eq!(verdict(&w[1], dirty, &g), Verdict::Dirty(3));
        let ahead = Facts {
            dirty: Some(0),
            unpushed: Some(2),
        };
        assert_eq!(verdict(&w[2], ahead, &g), Verdict::Unpushed(2));
        // Not knowing is not "nothing to lose".
        let unknown = Facts {
            dirty: Some(0),
            unpushed: None,
        };
        assert_eq!(verdict(&w[1], unknown, &g), Verdict::Unknown);
        assert!(!Verdict::Unknown.removable());
    }

    /// An open item keeps its recorded worktree, its branch, its wave
    /// branches and a folder named after it — clean or not.
    #[test]
    fn an_open_item_keeps_what_it_uses() {
        let wt = |path: &str, branch: Option<&str>| Worktree {
            path: path.into(),
            branch: branch.map(str::to_string),
            ..Worktree::default()
        };
        let g = Guards {
            items: vec![(
                "audit-logs/saturn".into(),
                Some("/w/al-worktrees/saturn".into()),
                vec!["saturn".into(), String::new()],
            )],
            ..Guards::default()
        };
        let item = Verdict::Item("audit-logs/saturn".into());
        assert_eq!(
            verdict(&wt("/w/al-worktrees/saturn", None), clean(), &g),
            item
        );
        assert_eq!(
            verdict(&wt("/w/x-worktrees/a", Some("saturn-w3")), clean(), &g),
            item
        );
        assert_eq!(
            verdict(&wt("/w/x-worktrees/saturn-w5", None), clean(), &g),
            item
        );
        assert_eq!(
            verdict(
                &wt("/w/x-worktrees/saturnine", Some("saturnine")),
                clean(),
                &g
            ),
            Verdict::Remove,
            "a longer name is not a wave branch"
        );
        // An empty branch name owns nothing.
        assert_eq!(
            verdict(&wt("/w/x-worktrees/b", Some("")), clean(), &g),
            Verdict::Remove
        );
    }

    #[test]
    fn a_running_session_inside_keeps_it() {
        let w = Worktree {
            path: "/w/r-worktrees/x".into(),
            ..Worktree::default()
        };
        let g = Guards {
            sessions: vec![("Auth · implement".into(), "/w/r-worktrees/x/src".into())],
            ..Guards::default()
        };
        assert_eq!(
            verdict(&w, clean(), &g),
            Verdict::Session("Auth · implement".into())
        );
        let elsewhere = Guards {
            sessions: vec![("other".into(), "/w/r-worktrees/x2".into())],
            ..Guards::default()
        };
        assert_eq!(verdict(&w, clean(), &elsewhere), Verdict::Remove);
    }

    #[test]
    fn path_within_compares_whole_components() {
        assert!(path_within("/a/b", "/a/b"));
        assert!(path_within("/a/b/c", "/a/b/"));
        assert!(!path_within("/a/bc", "/a/b"));
        assert!(!path_within("/a/b", ""));
    }
}
