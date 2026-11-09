//! PR stacks — ordering an item's PRs into a GitHub stack, pure.
//!
//! A stack is a chain of PRs in one repository where each PR's base branch is
//! the previous PR's head branch, so each layer's diff is only its own change.
//! GitHub has no stack object: the base/head chain is all `gh pr view` exposes,
//! so that chain is the definition. Anything that does not order into exactly
//! one chain is reported, never guessed. Full rules: `docs/workflows.md` →
//! *PR stacks*.

use crate::domain::forge::ForgeKind;
use crate::domain::workflow::{PrStack, StackLayer, WorkflowMeta};

/// One PR as the ordering sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StackPr {
    pub url: String,
    /// `owner/repo`, or empty when the URL could not be parsed — such PRs
    /// are grouped together, which is the best that can be said of them.
    pub repo: String,
    pub head: String,
    pub base: String,
    /// Closed without merging: abandoned, so never a layer.
    pub closed: bool,
    /// Merged: still a layer until GitHub retargets its child, but nothing
    /// new is built on it.
    pub merged: bool,
}

/// Why an item's PRs do not order into one stack.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StackError {
    /// Two PRs build on the same parent — a tree, not a stack.
    #[error("{} has several PRs built on it ({}) — a stack is one chain", label(.parent), labels(.children))]
    Fork {
        parent: String,
        children: Vec<String>,
    },
    /// Two PRs propose the same branch, so a child's parent is ambiguous.
    #[error("{} propose the same branch `{head}` — which one a PR builds on is ambiguous", labels(.urls))]
    SharedHead { head: String, urls: Vec<String> },
    /// The base/head chain loops back on itself.
    #[error("{} target each other in a loop", labels(.urls))]
    Cycle { urls: Vec<String> },
    /// More than one chain — clash orders one stack per item.
    #[error("this item's PRs form several stacks ({}) — clash orders one stack per item", .stacks.iter().map(|s| labels(s)).collect::<Vec<_>>().join("; "))]
    Several { stacks: Vec<Vec<String>> },
    /// PRs of one repository start from different bases and none builds on
    /// the other: a layer between them is missing from the item.
    #[error("{} start from different branches and none builds on another — a layer between them is not linked to this item", .roots.iter().map(|(u, b)| format!("{} → `{}`", label(u), b)).collect::<Vec<_>>().join(", "))]
    Gap { roots: Vec<(String, String)> },
    /// A new layer could go on top of any of these PRs: none builds on
    /// another, so which one is this item's parent is the human's call.
    #[error("{} could each be the PR this item builds on — link only its parent, or set the item's diff base", labels(.urls))]
    Ambiguous { urls: Vec<String> },
}

/// `repo#42` from a PR URL; the URL itself when it is not one.
fn label(url: &str) -> String {
    let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
    match parts.iter().rposition(|p| *p == "pull") {
        Some(i) if i >= 1 && i + 1 < parts.len() => format!("{}#{}", parts[i - 1], parts[i + 1]),
        _ => url.to_string(),
    }
}

fn labels(urls: &[String]) -> String {
    urls.iter().map(|u| label(u)).collect::<Vec<_>>().join(", ")
}

/// Pure: whether an item's PRs are treated as a stack. The item override
/// (`on`/`off`) wins over the global setting; only GitHub has stacks.
pub fn stacking_enabled(item: &str, global: bool, forge: ForgeKind) -> bool {
    if forge != ForgeKind::GitHub {
        return false;
    }
    match item.trim().to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => global,
    }
}

/// Pure: the ordering's input from an item's meta — primary first, then the
/// linked PRs. `repo_of` maps a PR URL to its `owner/repo` (the forge's URL
/// parser; empty when unknown).
pub fn stack_inputs(meta: &WorkflowMeta, repo_of: impl Fn(&str) -> String) -> Vec<StackPr> {
    meta.pr
        .iter()
        .chain(meta.linked_prs.iter())
        .filter(|p| !p.url.trim().is_empty())
        .map(|p| StackPr {
            url: p.url.clone(),
            repo: repo_of(&p.url),
            head: p.head_ref.clone(),
            base: p.base_ref.clone(),
            closed: p.state == "CLOSED",
            merged: p.state == "MERGED",
        })
        .collect()
}

/// Pure: order PRs into a stack, base → tip.
///
/// `Ok(None)` when no PR builds on another. PRs without known branches (never
/// refreshed) and closed PRs are left out; PRs in other repositories never
/// join a chain and stay outside the stack.
pub fn order_stack(prs: &[StackPr]) -> Result<Option<PrStack>, StackError> {
    let mut nodes: Vec<&StackPr> = Vec::new();
    for p in prs {
        if p.closed || p.head.trim().is_empty() || p.base.trim().is_empty() {
            continue;
        }
        if !nodes.iter().any(|n| n.url == p.url) {
            nodes.push(p);
        }
    }

    for (i, a) in nodes.iter().enumerate() {
        let same: Vec<String> = nodes
            .iter()
            .filter(|b| b.repo == a.repo && b.head == a.head)
            .map(|b| b.url.clone())
            .collect();
        if same.len() > 1 && nodes.iter().position(|b| b.url == same[0]) == Some(i) {
            return Err(StackError::SharedHead {
                head: a.head.clone(),
                urls: same,
            });
        }
    }

    // parent[i] = the PR whose head is i's base, in i's repository.
    let parent: Vec<Option<usize>> = nodes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            nodes
                .iter()
                .enumerate()
                .position(|(j, p)| j != i && p.repo == c.repo && p.head == c.base)
        })
        .collect();

    for (pi, p) in nodes.iter().enumerate() {
        let children: Vec<String> = (0..nodes.len())
            .filter(|&c| parent[c] == Some(pi))
            .map(|c| nodes[c].url.clone())
            .collect();
        if children.len() > 1 {
            return Err(StackError::Fork {
                parent: p.url.clone(),
                children,
            });
        }
    }

    // No forks, so every root starts exactly one chain; a node no root
    // reaches sits on a loop.
    let child_of = |pi: usize| (0..nodes.len()).find(|&c| parent[c] == Some(pi));
    let mut chains: Vec<Vec<usize>> = Vec::new();
    let mut reached = vec![false; nodes.len()];
    for root in (0..nodes.len()).filter(|&i| parent[i].is_none()) {
        let mut chain = vec![root];
        reached[root] = true;
        let mut at = root;
        while let Some(c) = child_of(at) {
            chain.push(c);
            reached[c] = true;
            at = c;
        }
        chains.push(chain);
    }
    let looped: Vec<String> = (0..nodes.len())
        .filter(|&i| !reached[i])
        .map(|i| nodes[i].url.clone())
        .collect();
    if !looped.is_empty() {
        return Err(StackError::Cycle { urls: looped });
    }

    // Every chain of one repository must start from the same branch: two
    // starting points means something between them is missing.
    let mut repos: Vec<&str> = Vec::new();
    for n in &nodes {
        if !repos.contains(&n.repo.as_str()) {
            repos.push(&n.repo);
        }
    }
    for repo in repos {
        let roots: Vec<(String, String)> = chains
            .iter()
            .map(|c| nodes[c[0]])
            .filter(|r| r.repo == repo)
            .map(|r| (r.url.clone(), r.base.clone()))
            .collect();
        let first = roots.first().map(|(_, b)| b.clone());
        if roots.iter().any(|(_, b)| Some(b) != first.as_ref()) {
            return Err(StackError::Gap { roots });
        }
    }

    let stacks: Vec<&Vec<usize>> = chains.iter().filter(|c| c.len() > 1).collect();
    match stacks.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(PrStack {
            layers: one
                .iter()
                .map(|&i| StackLayer {
                    url: nodes[i].url.clone(),
                    head_ref: nodes[i].head.clone(),
                    base_ref: nodes[i].base.clone(),
                })
                .collect(),
        })),
        many => Err(StackError::Several {
            stacks: many
                .iter()
                .map(|c| c.iter().map(|&i| nodes[i].url.clone()).collect())
                .collect(),
        }),
    }
}

/// What an item's own change builds on when its PRs are stacked: the parent
/// layer's head branch and that PR's URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackBase {
    pub branch: String,
    pub parent: String,
}

/// Pure: where this item's change sits in its stack — what its diff is
/// measured from and what a new PR for it targets.
///
/// With a PR (`primary`), it is that PR's parent layer (none for the bottom
/// layer). Without one, the item is the next layer: it goes on the tip of the
/// stack its open PRs in `item_repo` form — or on the only such PR — and
/// several PRs none of which builds on another is [`StackError::Ambiguous`],
/// never a guess. `item_branch` is left out: it is this item's own head.
pub fn item_stack_base(
    prs: &[StackPr],
    primary: Option<&str>,
    item_repo: &str,
    item_branch: &str,
) -> Result<Option<StackBase>, StackError> {
    if let Some(url) = primary.filter(|u| prs.iter().any(|p| p.url == *u)) {
        let Some(stack) = order_stack(prs)? else {
            return Ok(None);
        };
        let Some(i) = stack.layers.iter().position(|l| l.url == url) else {
            return Ok(None);
        };
        return Ok((i > 0).then(|| StackBase {
            branch: stack.layers[i - 1].head_ref.clone(),
            parent: stack.layers[i - 1].url.clone(),
        }));
    }
    let candidates: Vec<StackPr> = prs
        .iter()
        .filter(|p| {
            !p.closed
                && !p.merged
                && p.repo == item_repo
                && !p.head.trim().is_empty()
                && !p.base.trim().is_empty()
                && p.head != item_branch
        })
        .cloned()
        .collect();
    match (order_stack(&candidates)?, candidates.as_slice()) {
        (Some(stack), _) => {
            let tip = stack.layers.last().expect("a stack has layers");
            Ok(Some(StackBase {
                branch: tip.head_ref.clone(),
                parent: tip.url.clone(),
            }))
        }
        (None, []) => Ok(None),
        (None, [one]) => Ok(Some(StackBase {
            branch: one.head.clone(),
            parent: one.url.clone(),
        })),
        (None, many) => Err(StackError::Ambiguous {
            urls: many.iter().map(|p| p.url.clone()).collect(),
        }),
    }
}

/// Pure: `owner/repo` of a git remote URL on any host (scp-like, `https://`,
/// `ssh://`), the same identity PR URLs carry — GitHub Enterprise included.
pub fn remote_repo(remote: &str) -> Option<String> {
    let raw = remote.trim().trim_end_matches('/');
    let raw = raw.strip_suffix(".git").unwrap_or(raw);
    let rest = raw.split_once("://").map(|(_, r)| r).unwrap_or(raw);
    let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
    let (_host, path) = rest.split_once([':', '/'])?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [.., owner, name] => Some(format!("{owner}/{name}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const API: &str = "acme/api";

    fn pr(n: u64, base: &str, head: &str) -> StackPr {
        pr_in(API, n, base, head)
    }

    fn pr_in(repo: &str, n: u64, base: &str, head: &str) -> StackPr {
        StackPr {
            url: format!("https://github.com/{}/pull/{}", repo, n),
            repo: repo.to_string(),
            head: head.to_string(),
            base: base.to_string(),
            closed: false,
            merged: false,
        }
    }

    fn urls(s: &PrStack) -> Vec<u64> {
        s.layers
            .iter()
            .map(|l| l.url.rsplit('/').next().unwrap().parse().unwrap())
            .collect()
    }

    #[test]
    fn a_chain_orders_base_to_tip_whatever_the_input_order() {
        let prs = [pr(3, "b", "c"), pr(1, "main", "a"), pr(2, "a", "b")];
        let stack = order_stack(&prs).unwrap().unwrap();
        assert_eq!(urls(&stack), vec![1, 2, 3]);
        assert_eq!(stack.layers[1].base_ref, "a");
        assert_eq!(
            item_stack_base(&prs, Some(&prs[0].url), API, "c")
                .unwrap()
                .unwrap()
                .branch,
            "b"
        );
        assert_eq!(
            item_stack_base(&prs, Some(&prs[1].url), API, "a").unwrap(),
            None
        );
    }

    #[test]
    fn no_edge_is_no_stack() {
        assert_eq!(order_stack(&[]).unwrap(), None);
        assert_eq!(order_stack(&[pr(1, "main", "a")]).unwrap(), None);
        // Two independent PRs into the same trunk.
        assert_eq!(
            order_stack(&[pr(1, "main", "a"), pr(2, "main", "b")]).unwrap(),
            None
        );
    }

    #[test]
    fn prs_of_other_repositories_stay_outside_the_stack() {
        let prs = [
            pr(1, "main", "a"),
            pr_in("acme/web", 9, "main", "a"),
            pr(2, "a", "b"),
        ];
        let stack = order_stack(&prs).unwrap().unwrap();
        assert_eq!(urls(&stack), vec![1, 2]);
        // Same branch names in another repository are not a parent.
        let cross = [pr(1, "main", "a"), pr_in("acme/web", 2, "a", "b")];
        assert_eq!(order_stack(&cross).unwrap(), None);
    }

    #[test]
    fn unrefreshed_and_closed_prs_are_not_layers() {
        let mut closed = pr(2, "a", "b");
        closed.closed = true;
        let prs = [pr(1, "main", "a"), closed, pr(5, "", "")];
        assert_eq!(order_stack(&prs).unwrap(), None);
    }

    #[test]
    fn a_merged_bottom_layer_retargeted_to_trunk_leaves_the_rest() {
        // GitHub retargets the child to main once the parent merges.
        let prs = [pr(1, "main", "a"), pr(2, "main", "b"), pr(3, "b", "c")];
        let stack = order_stack(&prs).unwrap().unwrap();
        assert_eq!(urls(&stack), vec![2, 3]);
    }

    #[test]
    fn a_fork_is_reported_not_guessed() {
        let prs = [pr(1, "main", "a"), pr(2, "a", "b"), pr(3, "a", "c")];
        let err = order_stack(&prs).unwrap_err();
        assert_eq!(
            err,
            StackError::Fork {
                parent: prs[0].url.clone(),
                children: vec![prs[1].url.clone(), prs[2].url.clone()],
            }
        );
        assert!(err.to_string().contains("api#1"), "{err}");
    }

    #[test]
    fn a_cycle_is_reported() {
        let prs = [pr(1, "b", "a"), pr(2, "a", "b"), pr(3, "main", "x")];
        let StackError::Cycle { urls } = order_stack(&prs).unwrap_err() else {
            panic!("expected a cycle");
        };
        assert_eq!(urls, vec![prs[0].url.clone(), prs[1].url.clone()]);
    }

    #[test]
    fn a_missing_middle_layer_is_a_gap() {
        // #2 (a → b) is not linked: #3 targets `b`, which nothing proposes.
        let prs = [pr(1, "main", "a"), pr(3, "b", "c")];
        let err = order_stack(&prs).unwrap_err();
        assert_eq!(
            err,
            StackError::Gap {
                roots: vec![
                    (prs[0].url.clone(), "main".to_string()),
                    (prs[1].url.clone(), "b".to_string())
                ]
            }
        );
        assert!(err.to_string().contains("api#3 → `b`"), "{err}");
        // Also beside a complete chain.
        let more = [pr(1, "main", "a"), pr(2, "a", "b"), pr(4, "x", "y")];
        assert!(matches!(
            order_stack(&more).unwrap_err(),
            StackError::Gap { .. }
        ));
    }

    #[test]
    fn two_prs_from_one_branch_are_ambiguous() {
        let prs = [pr(1, "main", "a"), pr(2, "main", "a"), pr(3, "a", "b")];
        assert!(matches!(
            order_stack(&prs).unwrap_err(),
            StackError::SharedHead { ref head, .. } if head == "a"
        ));
    }

    #[test]
    fn two_stacks_in_two_repositories_are_reported() {
        let prs = [
            pr(1, "main", "a"),
            pr(2, "a", "b"),
            pr_in("acme/web", 7, "main", "x"),
            pr_in("acme/web", 8, "x", "y"),
        ];
        let StackError::Several { stacks } = order_stack(&prs).unwrap_err() else {
            panic!("expected several stacks");
        };
        assert_eq!(stacks.len(), 2);
    }

    #[test]
    fn duplicate_urls_count_once() {
        let prs = [pr(1, "main", "a"), pr(2, "a", "b"), pr(2, "a", "b")];
        assert_eq!(urls(&order_stack(&prs).unwrap().unwrap()), vec![1, 2]);
    }

    #[test]
    fn the_item_override_wins_and_only_github_stacks() {
        assert!(!stacking_enabled("", false, ForgeKind::GitHub));
        assert!(stacking_enabled("", true, ForgeKind::GitHub));
        assert!(stacking_enabled("on", false, ForgeKind::GitHub));
        assert!(!stacking_enabled("off", true, ForgeKind::GitHub));
        assert!(!stacking_enabled("on", true, ForgeKind::None));
    }

    #[test]
    fn inputs_come_from_the_primary_then_the_linked_prs() {
        use crate::domain::workflow::WorkflowPr;
        let rec = |url: &str, base: &str, head: &str, state: &str| WorkflowPr {
            url: url.into(),
            base_ref: base.into(),
            head_ref: head.into(),
            state: state.into(),
            ..Default::default()
        };
        let meta = WorkflowMeta {
            pr: Some(rec("https://github.com/acme/api/pull/2", "a", "b", "OPEN")),
            linked_prs: vec![
                rec("https://github.com/acme/api/pull/1", "main", "a", "MERGED"),
                rec("", "", "", ""),
                rec("https://github.com/acme/api/pull/9", "b", "z", "CLOSED"),
            ],
            ..Default::default()
        };
        let inputs = stack_inputs(&meta, |_| API.to_string());
        assert_eq!(inputs.len(), 3);
        assert!(inputs[0].url.ends_with("/2"));
        assert!(inputs[2].closed);
        let stack = order_stack(&inputs).unwrap().unwrap();
        assert_eq!(urls(&stack), vec![1, 2]);
    }

    #[test]
    fn a_new_layer_goes_on_the_tip_of_its_linked_stack() {
        // No PR of its own yet: the tip of the chain its linked PRs form.
        let prs = [pr(1, "main", "a"), pr(2, "a", "b")];
        assert_eq!(
            item_stack_base(&prs, None, API, "mine").unwrap(),
            Some(StackBase {
                branch: "b".into(),
                parent: prs[1].url.clone()
            })
        );
        // One linked PR is a stack of one: build on it.
        let one = [pr(1, "main", "a")];
        assert_eq!(
            item_stack_base(&one, None, API, "mine")
                .unwrap()
                .unwrap()
                .branch,
            "a"
        );
        // Other repositories, merged, closed and our own branch never count.
        let mut merged = pr(3, "main", "m");
        merged.merged = true;
        let mut closed = pr(4, "main", "c");
        closed.closed = true;
        let skip = [
            pr_in("acme/web", 5, "main", "w"),
            merged,
            closed,
            pr(6, "main", "mine"),
        ];
        assert_eq!(item_stack_base(&skip, None, API, "mine").unwrap(), None);
        // Two PRs, neither on the other: which is the parent is not ours to guess.
        let two = [pr(1, "main", "a"), pr(2, "main", "b")];
        assert!(matches!(
            item_stack_base(&two, None, API, "mine"),
            Err(StackError::Ambiguous { urls }) if urls.len() == 2
        ));
    }

    #[test]
    fn with_its_own_pr_the_item_builds_on_its_parent_layer() {
        let prs = [pr(1, "main", "a"), pr(2, "a", "b"), pr(3, "b", "c")];
        let base = item_stack_base(&prs, Some(&prs[2].url), API, "c")
            .unwrap()
            .unwrap();
        assert_eq!(
            (base.branch.as_str(), base.parent.as_str()),
            ("b", prs[1].url.as_str())
        );
        assert_eq!(
            item_stack_base(&prs, Some(&prs[0].url), API, "a").unwrap(),
            None,
            "the bottom layer"
        );
        let lone = [pr(1, "main", "a")];
        assert_eq!(
            item_stack_base(&lone, Some(&lone[0].url), API, "a").unwrap(),
            None
        );
    }

    #[test]
    fn remote_repo_reads_any_host() {
        let want = Some("acme/api".to_string());
        assert_eq!(remote_repo("git@github.com:acme/api.git"), want);
        assert_eq!(remote_repo("https://ghe.corp.example/acme/api"), want);
        assert_eq!(
            remote_repo("ssh://git@ghe.corp.example:22/acme/api.git"),
            want
        );
        assert_eq!(remote_repo("nonsense"), None);
    }
}
