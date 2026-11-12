//! Workspace items — workflow items whose home is a folder of repositories
//! rather than one repository. The ticket and the plan decide which
//! repositories change; the agent creates a worktree in each, and clash finds
//! them again from the item's branches and PRs. Pure half: parsing what git
//! leaves on disk and the kickoff field. Full rules: `docs/workflows.md` →
//! *Workspace items*.

use crate::domain::workflow::ItemRepo;

/// `ref: refs/heads/<branch>` (a `HEAD` file) → the branch.
pub fn parse_head_ref(content: &str) -> Option<String> {
    content
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
        .filter(|b| !b.is_empty())
}

/// Does a `packed-refs` file hold `refs/heads/<branch>`?
pub fn packed_refs_have(content: &str, branch: &str) -> bool {
    let want = format!("refs/heads/{branch}");
    content
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with('^'))
        .any(|l| l.split_whitespace().nth(1) == Some(want.as_str()))
}

/// The `owner/repo` of a `.git/config`'s `origin` remote, when it is a
/// GitHub-style URL (`git@host:o/r.git`, `https://host/o/r(.git)`).
pub fn origin_slug(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_origin = l == "[remote \"origin\"]";
            continue;
        }
        if !in_origin {
            continue;
        }
        let Some(url) = l.strip_prefix("url").map(str::trim_start) else {
            continue;
        };
        let url = url.strip_prefix('=')?.trim();
        let path = match url.split_once("://") {
            Some((_, rest)) => rest.split_once('/')?.1,
            None => url.split_once(':')?.1,
        };
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let mut parts = path.rsplitn(3, '/');
        let repo = parts.next()?;
        let owner = parts.next()?;
        return (!repo.is_empty() && !owner.is_empty()).then(|| format!("{owner}/{repo}"));
    }
    None
}

/// The worktree path git records for a linked worktree (its `gitdir` file
/// names `<worktree>/.git`).
pub fn worktree_from_gitdir(content: &str) -> Option<String> {
    let p = content.trim();
    let p = p.strip_suffix("/.git").unwrap_or(p);
    (!p.is_empty()).then(|| p.to_string())
}

/// Where the agent puts a repository's worktree for `branch`, by the
/// workspace's own convention.
pub fn worktree_path(workspace: &str, repo: &str, branch: &str) -> String {
    format!(
        "{}/{repo}-worktrees/{branch}",
        workspace.trim_end_matches('/')
    )
}

/// The `Repos:` kickoff field of a workspace item (executor, reviewer and
/// discussion; it replaces `Workspace: directory`, which means "no git"): where it
/// lives, that the repositories come from the ticket and the plan, the
/// worktree convention, and the repositories already in play.
pub fn workspace_clause(workspace: &str, branch: &str, repos: &[ItemRepo]) -> String {
    let ws = workspace.trim_end_matches('/');
    let mut out = format!(
        " Repos: from the ticket and the plan — this item lives in the workspace {ws}, not one repository. \
         In each one, work in the worktree `{}` on branch `{branch}` (a wave works on its wave branch, \
         in `<repo>-worktrees/<wave branch>`), created from the repository's default branch when missing \
         (`git -C {ws}/<repo> worktree add ../<repo>-worktrees/{branch} -b {branch} origin/<default>`); \
         never commit in a repository's main checkout.",
        worktree_path(ws, "<repo>", branch)
    );
    if !repos.is_empty() {
        let named: Vec<String> = repos
            .iter()
            .map(|r| {
                let wts: Vec<String> = r
                    .worktrees
                    .iter()
                    .map(|w| format!("{} on {}", w.path, w.branch))
                    .collect();
                if wts.is_empty() {
                    r.name.clone()
                } else {
                    format!("{} ({})", r.name, wts.join(", "))
                }
            })
            .collect();
        out.push_str(&format!(" In play: {}.", named.join("; ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::RepoWorktree;

    #[test]
    fn git_files_are_read_as_git_writes_them() {
        assert_eq!(
            parse_head_ref("ref: refs/heads/item-w2\n").as_deref(),
            Some("item-w2")
        );
        assert_eq!(parse_head_ref("4f1c0d2e"), None);
        let packed = "# pack-refs with: peeled\nabc refs/heads/item\n^def\nabc refs/heads/main\n";
        assert!(packed_refs_have(packed, "item"));
        assert!(!packed_refs_have(packed, "ite"));
        assert_eq!(
            worktree_from_gitdir("/w/api-worktrees/item/.git\n").as_deref(),
            Some("/w/api-worktrees/item")
        );
    }

    #[test]
    fn the_origin_names_the_github_repository() {
        let ssh = "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:Hivebrite/api-contracts.git\n";
        assert_eq!(origin_slug(ssh).as_deref(), Some("Hivebrite/api-contracts"));
        let https = "[remote \"upstream\"]\n\turl = https://github.com/x/y\n[remote \"origin\"]\n\turl = https://github.com/o/r.git\n";
        assert_eq!(origin_slug(https).as_deref(), Some("o/r"));
        assert_eq!(origin_slug("[core]\n"), None);
    }

    #[test]
    fn the_kickoff_names_the_convention_and_what_is_in_play() {
        let c = workspace_clause("/ws/", "item", &[]);
        assert!(
            c.starts_with(
                " Repos: from the ticket and the plan — this item lives in the workspace /ws"
            ),
            "{c}"
        );
        assert!(
            c.contains("`/ws/<repo>-worktrees/item` on branch `item`"),
            "{c}"
        );
        assert!(!c.contains("In play"));
        let repos = vec![
            ItemRepo {
                name: "api".into(),
                worktrees: vec![RepoWorktree {
                    path: "/ws/api-worktrees/item".into(),
                    branch: "item".into(),
                }],
                ..Default::default()
            },
            ItemRepo {
                name: "web".into(),
                ..Default::default()
            },
        ];
        assert!(workspace_clause("/ws", "item", &repos)
            .ends_with(" In play: api (/ws/api-worktrees/item on item); web."));
    }
}
