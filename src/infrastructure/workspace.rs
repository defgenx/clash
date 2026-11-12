//! Finding a workspace item's repositories on disk: every checkout directly
//! under the workspace that holds one of the item's branches (as a local
//! branch or a worktree) or is the origin of one of its PRs. Plain file
//! reads of what git keeps in `.git` — no `git` process per repository, so a
//! workspace of dozens of checkouts costs milliseconds. Parsing is
//! `application::workspace`.

use std::path::Path;

use crate::application::workspace::{
    origin_slug, packed_refs_have, parse_head_ref, worktree_from_gitdir,
};
use crate::domain::workflow::{ItemRepo, RepoWorktree};

/// A folder that is not itself a git checkout: an item living there is a
/// workspace item.
pub fn is_workspace_home(path: &str) -> bool {
    let p = Path::new(path.trim());
    !path.trim().is_empty() && p.is_dir() && !p.join(".git").exists()
}

/// The repositories under `workspace` the item touches. `branches` are the
/// item's branch and its wave branches; `pr_repos` the `owner/repo` of its
/// PRs (compared case-insensitively).
pub fn discover(workspace: &Path, branches: &[String], pr_repos: &[String]) -> Vec<ItemRepo> {
    let Ok(entries) = std::fs::read_dir(workspace) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("-worktrees") || n.starts_with('.'))
        })
        .collect();
    dirs.sort();
    let mut out = Vec::new();
    for dir in dirs {
        let git = dir.join(".git");
        if !git.is_dir() {
            continue;
        }
        let origin = std::fs::read_to_string(git.join("config"))
            .ok()
            .and_then(|c| origin_slug(&c))
            .unwrap_or_default();
        let mut worktrees = Vec::new();
        if let Ok(wts) = std::fs::read_dir(git.join("worktrees")) {
            for wt in wts.flatten() {
                let meta = wt.path();
                let Some(branch) = std::fs::read_to_string(meta.join("HEAD"))
                    .ok()
                    .and_then(|h| parse_head_ref(&h))
                else {
                    continue;
                };
                if !branches.contains(&branch) {
                    continue;
                }
                if let Some(path) = std::fs::read_to_string(meta.join("gitdir"))
                    .ok()
                    .and_then(|g| worktree_from_gitdir(&g))
                {
                    worktrees.push(RepoWorktree { path, branch });
                }
            }
        }
        worktrees.sort_by(|a, b| a.path.cmp(&b.path));
        let packed = std::fs::read_to_string(git.join("packed-refs")).unwrap_or_default();
        let local: Vec<String> = branches
            .iter()
            .filter(|b| git.join("refs/heads").join(b).is_file() || packed_refs_have(&packed, b))
            .cloned()
            .collect();
        let by_pr = !origin.is_empty() && pr_repos.iter().any(|r| r.eq_ignore_ascii_case(&origin));
        if worktrees.is_empty() && local.is_empty() && !by_pr {
            continue;
        }
        out.push(ItemRepo {
            name: dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string(),
            path: dir.to_string_lossy().into_owned(),
            origin,
            worktrees,
            branches: local,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_checkouts_holding_the_items_branches_or_prs() {
        let ws = tempfile::TempDir::new().unwrap();
        let root = ws.path();
        // api: a worktree of the item branch.
        let api = root.join("api/.git");
        std::fs::create_dir_all(api.join("worktrees/item")).unwrap();
        std::fs::create_dir_all(api.join("refs/heads")).unwrap();
        std::fs::write(api.join("refs/heads/item"), "abc\n").unwrap();
        std::fs::write(api.join("worktrees/item/HEAD"), "ref: refs/heads/item\n").unwrap();
        let wt = root.join("api-worktrees/item");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            api.join("worktrees/item/gitdir"),
            format!("{}/.git\n", wt.display()),
        )
        .unwrap();
        // web: only the origin of one of the item's PRs.
        let web = root.join("web/.git");
        std::fs::create_dir_all(&web).unwrap();
        std::fs::write(
            web.join("config"),
            "[remote \"origin\"]\n\turl = git@github.com:O/Web.git\n",
        )
        .unwrap();
        // docs: unrelated; plain: not a checkout.
        std::fs::create_dir_all(root.join("docs/.git/refs/heads")).unwrap();
        std::fs::create_dir_all(root.join("plain")).unwrap();

        let found = discover(root, &["item".into()], &["o/web".into()]);
        let names: Vec<&str> = found.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["api", "web"]);
        assert_eq!(found[0].branches, vec!["item"]);
        assert_eq!(found[0].worktrees[0].path, wt.to_string_lossy());
        assert_eq!(found[1].origin, "O/Web");
        assert!(is_workspace_home(&root.to_string_lossy()));
        assert!(!is_workspace_home(&root.join("api").to_string_lossy()));
    }
}
