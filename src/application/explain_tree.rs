//! Drill-down explanations — pure half.
//!
//! An explanation is a tree of levels: the overview (`explain-<target>.md` /
//! `.html` / `.json`) and one level per part someone zoomed into
//! (`explain-<target>/<id>.{md,html,json}`, ids `3`, `3.2`, `3.2.1`). Which
//! parts are expanded, stale or diveable is derived from the files, never
//! written. Contract: `docs/workflows.md` → *Drill-down explanations*.

use crate::domain::workflow::{
    ExplainBasis, ExplainIndex, ExplainLevel, ExplainNodeView, ReviewTarget,
};

/// Deepest level a part may get: overview → `3` → `3.2` → `3.2.1`.
pub const MAX_DEPTH: usize = 3;

/// The directory (and index stem) a target's tree lives under, or `None` for
/// a target that has no tree.
pub fn tree_dir(target: ReviewTarget) -> Option<&'static str> {
    match target {
        ReviewTarget::ExplainDiff => Some("explain-diff"),
        ReviewTarget::ExplainPlan => Some("explain-plan"),
        ReviewTarget::Drift => Some("drift"),
        _ => None,
    }
}

/// `1`..`n` segments of digits joined by dots, at most [`MAX_DEPTH`] of them.
/// Checked before an id becomes a path, so nothing escapes the tree.
pub fn valid_node_id(id: &str) -> bool {
    let parts: Vec<&str> = id.split('.').collect();
    parts.len() <= MAX_DEPTH
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Number of segments: `3` → 1, `3.2.1` → 3; the overview (`""`) → 0.
pub fn depth(id: &str) -> usize {
    if id.is_empty() {
        0
    } else {
        id.split('.').count()
    }
}

/// The level a part's level hangs under: `3.2` → `3`, `3` → `""` (the overview).
pub fn parent_of(id: &str) -> &str {
    id.rfind('.').map_or("", |i| &id[..i])
}

/// Is `id` a direct child of the level `parent`?
fn is_child_of(id: &str, parent: &str) -> bool {
    valid_node_id(id) && parent_of(id) == parent
}

/// One level as read off disk, before anything is derived from it.
#[derive(Debug, Clone, Default)]
pub struct RawLevel {
    /// Empty for the overview.
    pub node: String,
    /// The level's `.json` index text.
    pub json: String,
    /// Epoch ms the index was written.
    pub written_at: i64,
    pub md: bool,
    pub html: bool,
}

fn parse_index(raw: &str) -> ExplainIndex {
    serde_json::from_str(raw).unwrap_or_default()
}

fn basis_differs(a: &ExplainBasis, b: &ExplainBasis) -> bool {
    (a.iteration != b.iteration) || (!a.head.is_empty() && !b.head.is_empty() && a.head != b.head)
}

/// Pure: derive the tree's levels, overview first, then by id.
///
/// A level is stale when its parent level is missing, no longer lists it,
/// was written after it, or was drawn from another basis — and so is every
/// level under a stale one. Nothing is ever dropped: a stale level is shown
/// as such and can be dived again.
pub fn build_tree(raw: &[RawLevel]) -> Vec<ExplainLevel> {
    let mut levels: Vec<(RawLevel, ExplainIndex)> = raw
        .iter()
        .filter(|r| r.node.is_empty() || valid_node_id(&r.node))
        .map(|r| (r.clone(), parse_index(&r.json)))
        .collect();
    levels.sort_by(|a, b| node_order(&a.0.node, &b.0.node));

    let mut out: Vec<ExplainLevel> = Vec::with_capacity(levels.len());
    for (r, idx) in &levels {
        let stale = if r.node.is_empty() {
            false
        } else {
            match out.iter().find(|l| l.node == parent_of(&r.node)) {
                None => true,
                Some(p) => {
                    p.stale
                        || p.written_at > r.written_at
                        || basis_differs(&p.basis, &idx.basis)
                        || !p.nodes.iter().any(|n| n.node.id == r.node)
                }
            }
        };
        out.push(ExplainLevel {
            node: r.node.clone(),
            title: idx.title.clone(),
            basis: idx.basis.clone(),
            written_at: r.written_at,
            md: r.md,
            html: r.html,
            stale,
            nodes: Vec::new(),
            edges: idx.edges.clone(),
        });
        // A leaf level (the depth cap) has no parts of its own.
        let nodes = if depth(&r.node) >= MAX_DEPTH {
            Vec::new()
        } else {
            idx.nodes.clone()
        };
        let views = nodes
            .into_iter()
            .map(|n| {
                let well_formed = is_child_of(&n.id, &r.node);
                let child = levels.iter().find(|(c, _)| c.node == n.id);
                ExplainNodeView {
                    expanded: well_formed && child.is_some(),
                    diveable: well_formed && !n.leaf,
                    // Settled once the child is pushed; see the second pass.
                    stale: false,
                    node: n,
                }
            })
            .collect();
        out.last_mut().expect("just pushed").nodes = views;
    }
    // A part's `stale` is its own level's.
    let stale_levels: Vec<(String, bool)> = out.iter().map(|l| (l.node.clone(), l.stale)).collect();
    for l in &mut out {
        for n in &mut l.nodes {
            n.stale = n.expanded && stale_levels.iter().any(|(id, s)| *id == n.node.id && *s);
        }
    }
    out
}

/// Overview first, then ids in numeric order segment by segment, so `3.10`
/// follows `3.9` and a parent precedes its children.
fn node_order(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| -> Vec<u32> {
        if s.is_empty() {
            Vec::new()
        } else {
            s.split('.')
                .map(|p| p.parse().unwrap_or(u32::MAX))
                .collect()
        }
    };
    key(a).cmp(&key(b))
}

/// Pure: has the dive on `node` that launched at `started_at` written its
/// level? Its index is written last, so its presence is the finish signal.
pub fn dive_finished(raw: &[RawLevel], node: &str, started_at: i64) -> bool {
    raw.iter()
        .any(|r| r.node == node && r.written_at >= started_at)
}

/// May a dive into `node` of `target` be launched against this tree? The
/// part must be listed by its parent level, well formed and not a leaf.
/// Diving an already expanded (or stale) part redraws it.
pub fn can_dive(target: ReviewTarget, node: &str, tree: &[ExplainLevel]) -> Result<(), String> {
    if tree_dir(target).is_none() {
        return Err(format!("{} has no drill-down", target));
    }
    if !valid_node_id(node) {
        return Err(format!("Not a part of the explanation: '{}'", node));
    }
    let Some(parent) = tree.iter().find(|l| l.node == parent_of(node)) else {
        return Err(format!(
            "Part {} hangs under a level that has not been written yet",
            node
        ));
    };
    match parent.nodes.iter().find(|n| n.node.id == node) {
        None => Err(format!("The explanation lists no part {}", node)),
        Some(n) if !n.diveable => Err(format!("Part {} has nothing below it", node)),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(node: &str, at: i64, nodes: &str) -> RawLevel {
        RawLevel {
            node: node.to_string(),
            json: format!(
                r#"{{"title":"T{node}","basis":{{"iteration":1,"head":"abc"}},"nodes":[{nodes}]}}"#
            ),
            written_at: at,
            md: true,
            html: true,
        }
    }

    fn n(id: &str) -> String {
        format!(r#"{{"id":"{id}","title":"N{id}"}}"#)
    }

    #[test]
    fn node_ids_are_digits_and_capped() {
        for ok in ["1", "3.2", "3.2.1", "12.4"] {
            assert!(valid_node_id(ok), "{ok}");
        }
        for bad in [
            "", "..", "3/2", "3..2", "a", "3.2.1.4", "../x", "3.", ".3", "99999",
        ] {
            assert!(!valid_node_id(bad), "{bad}");
        }
        assert_eq!(parent_of("3.2.1"), "3.2");
        assert_eq!(parent_of("3"), "");
        assert_eq!(depth(""), 0);
        assert_eq!(depth("3.2"), 2);
    }

    #[test]
    fn expanded_is_derived_from_the_child_level() {
        let tree = build_tree(&[
            level("", 10, &[n("1"), n("2")].join(",")),
            level("2", 20, &n("2.1")),
        ]);
        assert_eq!(tree.len(), 2);
        let root = &tree[0];
        assert!(!root.nodes[0].expanded);
        assert!(root.nodes[1].expanded);
        assert!(root.nodes.iter().all(|x| x.diveable && !x.stale));
        assert!(!tree[1].stale);
    }

    #[test]
    fn a_level_older_than_its_parent_is_stale_and_so_are_its_children() {
        let tree = build_tree(&[
            level("", 50, &n("2")),
            level("2", 20, &n("2.1")),
            level("2.1", 30, &n("2.1.1")),
        ]);
        assert!(tree[1].stale, "redrawn overview");
        assert!(tree[2].stale, "inherits the stale parent");
        assert!(tree[0].nodes[0].stale);
    }

    #[test]
    fn a_level_its_parent_no_longer_lists_or_lacks_is_stale() {
        let tree = build_tree(&[
            level("", 10, &n("1")),
            level("4", 20, ""),
            level("5.1", 20, ""),
        ]);
        let four = tree.iter().find(|l| l.node == "4").unwrap();
        let orphan = tree.iter().find(|l| l.node == "5.1").unwrap();
        assert!(four.stale && orphan.stale);
    }

    #[test]
    fn a_level_from_another_basis_is_stale() {
        let mut child = level("1", 20, "");
        child.json = r#"{"basis":{"iteration":1,"head":"def"}}"#.to_string();
        let tree = build_tree(&[level("", 10, &n("1")), child]);
        assert!(tree[1].stale);
    }

    #[test]
    fn the_depth_cap_makes_the_third_level_a_leaf() {
        let tree = build_tree(&[
            level("", 1, &n("1")),
            level("1", 2, &n("1.1")),
            level("1.1", 3, &n("1.1.1")),
            level("1.1.1", 4, &n("1.1.1.1")),
        ]);
        let leaf = tree.iter().find(|l| l.node == "1.1.1").unwrap();
        assert!(leaf.nodes.is_empty());
        let l2 = tree.iter().find(|l| l.node == "1.1").unwrap();
        assert!(l2.nodes[0].diveable && l2.nodes[0].expanded);
    }

    #[test]
    fn malformed_and_leaf_parts_are_shown_but_not_diveable() {
        let tree = build_tree(&[level(
            "",
            1,
            &[n("../x"), n("2.1"), r#"{"id":"3","leaf":true}"#.to_string()].join(","),
        )]);
        assert!(tree[0].nodes.iter().all(|x| !x.diveable));
        assert_eq!(tree[0].nodes.len(), 3);
    }

    #[test]
    fn levels_are_ordered_numerically_parents_first() {
        let tree = build_tree(&[
            level("3.10", 5, ""),
            level("3", 4, &[n("3.9"), n("3.10")].join(",")),
            level("", 1, &n("3")),
            level("3.9", 5, ""),
        ]);
        let ids: Vec<&str> = tree.iter().map(|l| l.node.as_str()).collect();
        assert_eq!(ids, ["", "3", "3.9", "3.10"]);
    }

    #[test]
    fn a_corrupt_index_is_an_empty_level_not_an_error() {
        let mut bad = level("", 1, "");
        bad.json = "{not json".to_string();
        let tree = build_tree(&[bad]);
        assert_eq!(tree.len(), 1);
        assert!(tree[0].nodes.is_empty());
    }

    #[test]
    fn a_dive_is_finished_once_its_index_lands_after_the_launch() {
        let raw = [level("2", 100, "")];
        assert!(dive_finished(&raw, "2", 100));
        assert!(
            !dive_finished(&raw, "2", 101),
            "an older level is the previous dive's"
        );
        assert!(!dive_finished(&raw, "3", 0));
    }

    #[test]
    fn a_dive_needs_a_listed_diveable_part() {
        let tree = build_tree(&[level(
            "",
            1,
            &[n("1"), r#"{"id":"2","leaf":true}"#.to_string()].join(","),
        )]);
        assert!(can_dive(ReviewTarget::ExplainDiff, "1", &tree).is_ok());
        assert!(can_dive(ReviewTarget::Drift, "1", &tree).is_ok());
        assert!(
            can_dive(ReviewTarget::ExplainDiff, "2", &tree).is_err(),
            "leaf"
        );
        assert!(
            can_dive(ReviewTarget::ExplainDiff, "9", &tree).is_err(),
            "unlisted"
        );
        assert!(
            can_dive(ReviewTarget::ExplainDiff, "1.1", &tree).is_err(),
            "no level 1 yet"
        );
        assert!(can_dive(ReviewTarget::ExplainDiff, "../1", &tree).is_err());
        assert!(can_dive(ReviewTarget::Diff, "1", &tree).is_err(), "no tree");
    }
}
