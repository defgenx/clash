//! Workflow domain entities — the plan → review → implement → PR pipeline.
//!
//! Persisted types (`WorkflowMeta`, `Annotation`, …) follow the same lenient
//! serde conventions as [`crate::domain::entities`]: `#[serde(default)]` for
//! every field plus `#[serde(flatten)]` extras, because the agent and clash
//! both read-modify-write these JSON files.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Workflow item lifecycle status (kebab-case on disk in `meta.json`).
///
/// The pipeline: `draft → planning → plan-review → changes-requested →
/// implementing → diff-review → pr-draft → pr-ready → done`, with `abandoned`
/// reachable from anywhere. Decision states (`needs_attention`) are the ones
/// where the pipeline is blocked on a human.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowStatus {
    #[default]
    Draft,
    Planning,
    PlanReview,
    ChangesRequested,
    Implementing,
    DiffReview,
    PrDraft,
    PrReady,
    Done,
    Abandoned,
    #[serde(other)]
    Unknown,
}

impl WorkflowStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Planning => "planning",
            Self::PlanReview => "plan-review",
            Self::ChangesRequested => "changes-requested",
            Self::Implementing => "implementing",
            Self::DiffReview => "diff-review",
            Self::PrDraft => "pr-draft",
            Self::PrReady => "pr-ready",
            Self::Done => "done",
            Self::Abandoned => "abandoned",
            Self::Unknown => "unknown",
        }
    }

    /// States where the pipeline is blocked on a human decision — these drive
    /// notifications and the "NEEDS DECISION" grouping.
    pub fn needs_attention(&self) -> bool {
        matches!(self, Self::PlanReview | Self::DiffReview | Self::PrDraft)
    }

    /// True when the item is finished (skips expensive per-item reads during
    /// listing and collapses into the DONE group).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Abandoned)
    }

    /// Transition table. Deliberately permissive on back-edges (a human can
    /// always send an item backwards); `Abandoned` is reachable from every
    /// live state; nothing may transition *to* `Unknown` (it is a serde
    /// fallback, not a real state). Agent-side writes are not hard-gated by
    /// this — the GUI validates human-initiated transitions and a stale agent
    /// write self-heals on the next validated transition.
    pub fn can_transition_to(&self, next: WorkflowStatus) -> bool {
        use WorkflowStatus::*;
        if next == Unknown || *self == next {
            return false;
        }
        // Every live (non-terminal, known) state may be abandoned.
        if next == Abandoned {
            return !matches!(self, Abandoned);
        }
        match self {
            Draft => matches!(next, Planning),
            Planning => matches!(next, PlanReview | Draft),
            PlanReview => matches!(next, Implementing | ChangesRequested | Planning),
            ChangesRequested => matches!(next, Implementing | PlanReview),
            Implementing => matches!(next, DiffReview | PrDraft | ChangesRequested),
            DiffReview => matches!(next, PrDraft | ChangesRequested | Implementing),
            PrDraft => matches!(next, PrReady | Done | DiffReview),
            PrReady => matches!(next, Done | PrDraft),
            // Reopen path for finished items.
            Done => matches!(next, DiffReview),
            Abandoned => matches!(next, DiffReview | Draft),
            // An unknown on-disk status can be repaired to anything.
            Unknown => true,
        }
    }
}

impl std::fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// PR block inside a workflow item's `meta.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPr {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub draft: bool,
    /// Forge state as reported by `gh`: "OPEN" | "MERGED" | "CLOSED", or ""
    /// when never checked (e.g. manually attached URL without gh available).
    #[serde(default)]
    pub state: String,
    /// Epoch ms of the last successful `gh pr view` refresh (poll throttle).
    #[serde(default)]
    pub last_checked_at: i64,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Persisted shape of a workflow item's `meta.json`. Lenient serde like
/// [`crate::domain::entities::Task`]: missing fields default, unknown fields
/// survive round-trips via `extra` (the agent and clash both read-modify-write
/// this file).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowMeta {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub status: WorkflowStatus,
    /// Absolute path of the main repo checkout this item works on.
    #[serde(default)]
    pub repo_path: String,
    #[serde(default)]
    pub branch: String,
    /// Absolute worktree path when the item works in a dedicated worktree.
    #[serde(default)]
    pub worktree: Option<String>,
    /// Last clash session spawned for this item (drives "open agent session"
    /// and the liveness cross-check).
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub pr: Option<WorkflowPr>,
    /// Review iteration, starting at 1. Bumped only by clash on
    /// request-changes (never by the agent).
    #[serde(default)]
    pub iteration: u32,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// A workflow item as listed to the frontends — a runtime DTO like
/// [`crate::domain::entities::ScratchNote`]: `project`/`slug` are computed
/// from the directory layout (the path *is* the identity, never trusted from
/// `meta.json`), the summary fields are derived from sibling files at load
/// time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowItem {
    /// Project directory name (first level under the workflows root).
    pub project: String,
    /// Item directory name — with `project`, the stable identifier.
    pub slug: String,
    /// Absolute path of the item directory on disk.
    pub path: String,
    pub meta: WorkflowMeta,
    pub has_plan: bool,
    pub has_review: bool,
    /// Count of annotations with status `open` (0 for terminal items, whose
    /// annotations are not read during listing).
    pub open_annotations: usize,
    /// Snapshotted iterations found under `history/`, sorted ascending.
    pub history_iterations: Vec<u32>,
    /// False when `meta.session_id` points at a session that is no longer
    /// alive while the item claims an agent is working (planning /
    /// implementing). Computed by the GUI layer against live sessions.
    pub agent_alive: bool,
}

/// Resolution state of a diff annotation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationStatus {
    #[default]
    Open,
    Addressed,
    Wontfix,
    #[serde(other)]
    Unknown,
}

/// Which side of a unified diff a line-level annotation anchors to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffSide {
    Old,
    #[default]
    New,
}

/// A reply on an annotation thread.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationReply {
    /// "user" | "agent".
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: i64,
}

/// A line-level diff comment, persisted in a workflow item's
/// `annotations.json`. Anchored by `file + side + line` at creation time and
/// re-anchored by `line_content_hash` when the diff drifts across iterations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    #[serde(default)]
    pub id: String,
    /// Path relative to the repo root, as it appears in the diff.
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub side: DiffSide,
    /// Line number on `side` at anchor time.
    #[serde(default)]
    pub line: u32,
    /// `@@ -a,b +c,d @@ ctx` header of the hunk the line lived in at anchor
    /// time (display context for orphaned annotations).
    #[serde(default)]
    pub hunk_header: String,
    /// Raw text of the anchored line (display context; hash input).
    #[serde(default)]
    pub line_content: String,
    /// FNV-1a hash (hex) of the trimmed line text — computed by the backend
    /// on save; the re-anchoring key when line numbers drift.
    #[serde(default)]
    pub line_content_hash: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub status: AnnotationStatus,
    /// "user" | "agent".
    #[serde(default)]
    pub author: String,
    /// Iteration the annotation was written against.
    #[serde(default)]
    pub iteration: u32,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub replies: Vec<AnnotationReply>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// On-disk wrapper for `annotations.json` (room for future top-level fields
/// without breaking the array shape).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnnotationsFile {
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_WORKFLOW_STATUSES: [WorkflowStatus; 11] = [
        WorkflowStatus::Draft,
        WorkflowStatus::Planning,
        WorkflowStatus::PlanReview,
        WorkflowStatus::ChangesRequested,
        WorkflowStatus::Implementing,
        WorkflowStatus::DiffReview,
        WorkflowStatus::PrDraft,
        WorkflowStatus::PrReady,
        WorkflowStatus::Done,
        WorkflowStatus::Abandoned,
        WorkflowStatus::Unknown,
    ];

    #[test]
    fn test_workflow_status_serde_kebab_case() {
        assert_eq!(
            serde_json::to_string(&WorkflowStatus::PlanReview).unwrap(),
            r#""plan-review""#
        );
        assert_eq!(
            serde_json::to_string(&WorkflowStatus::PrDraft).unwrap(),
            r#""pr-draft""#
        );
        let parsed: WorkflowStatus = serde_json::from_str(r#""changes-requested""#).unwrap();
        assert_eq!(parsed, WorkflowStatus::ChangesRequested);
        // Unknown on-disk value falls back instead of failing the parse.
        let unknown: WorkflowStatus = serde_json::from_str(r#""half-baked""#).unwrap();
        assert_eq!(unknown, WorkflowStatus::Unknown);
        // as_str agrees with the serde representation for every variant.
        for s in ALL_WORKFLOW_STATUSES {
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
    }

    #[test]
    fn test_workflow_status_needs_attention() {
        for s in ALL_WORKFLOW_STATUSES {
            let expected = matches!(
                s,
                WorkflowStatus::PlanReview | WorkflowStatus::DiffReview | WorkflowStatus::PrDraft
            );
            assert_eq!(s.needs_attention(), expected, "{}", s);
        }
    }

    #[test]
    fn test_workflow_status_graph_consistency() {
        use WorkflowStatus::*;
        // Every real status is reachable from Draft via the transition table
        // (Unknown is a serde fallback, not a state, and is never a target).
        let mut reachable = std::collections::HashSet::from([Draft]);
        loop {
            let mut grew = false;
            for from in ALL_WORKFLOW_STATUSES {
                if !reachable.contains(&from) {
                    continue;
                }
                for to in ALL_WORKFLOW_STATUSES {
                    if from.can_transition_to(to) && reachable.insert(to) {
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        for s in ALL_WORKFLOW_STATUSES {
            if s == Unknown {
                assert!(!reachable.contains(&s), "unknown must be unreachable");
            } else {
                assert!(reachable.contains(&s), "{} unreachable from draft", s);
            }
        }

        // Nothing transitions to Unknown or to itself.
        for s in ALL_WORKFLOW_STATUSES {
            assert!(!s.can_transition_to(Unknown), "{} -> unknown", s);
            assert!(!s.can_transition_to(s), "{} -> self", s);
        }
    }

    #[test]
    fn test_workflow_action_bar_transitions_allowed() {
        use WorkflowStatus::*;
        // Every transition the GUI action bar performs must pass the table.
        let action_bar = [
            (Draft, Planning),              // Start planning
            (PlanReview, Implementing),     // Approve plan
            (PlanReview, ChangesRequested), // Request changes
            (DiffReview, PrDraft),          // Approve
            (DiffReview, ChangesRequested), // Request changes
            (PrDraft, PrReady),             // Mark PR ready
            (PrReady, Done),                // Mark done
            (Done, DiffReview),             // Reopen
            (Abandoned, DiffReview),        // Reopen
        ];
        for (from, to) in action_bar {
            assert!(
                from.can_transition_to(to),
                "{} -> {} must be allowed",
                from,
                to
            );
        }
        // Abandon is available from every live state.
        for s in ALL_WORKFLOW_STATUSES {
            if !matches!(s, Abandoned) {
                assert!(s.can_transition_to(Abandoned), "{} -> abandoned", s);
            }
        }
        // Agent-side contract transitions.
        let agent = [
            (Planning, PlanReview),
            (ChangesRequested, PlanReview), // plan revision round
            (ChangesRequested, Implementing),
            (Implementing, DiffReview),
            (Implementing, PrDraft),
        ];
        for (from, to) in agent {
            assert!(
                from.can_transition_to(to),
                "{} -> {} must be allowed",
                from,
                to
            );
        }
    }

    #[test]
    fn test_parse_workflow_meta_lenient() {
        // Missing fields default; unknown fields survive a round-trip.
        let json = r#"{
            "title": "Auth refactor",
            "status": "diff-review",
            "repoPath": "/w/clash",
            "iteration": 3,
            "pr": {"url": "https://github.com/o/r/pull/7", "number": 7, "draft": true, "futureField": 1},
            "somethingNew": {"x": true}
        }"#;
        let meta: WorkflowMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.status, WorkflowStatus::DiffReview);
        assert_eq!(meta.repo_path, "/w/clash");
        assert_eq!(meta.iteration, 3);
        assert_eq!(meta.branch, "");
        assert!(meta.worktree.is_none());
        let pr = meta.pr.as_ref().unwrap();
        assert_eq!(pr.number, 7);
        assert!(pr.draft);
        assert!(meta.extra.contains_key("somethingNew"));
        assert!(pr.extra.contains_key("futureField"));

        let back = serde_json::to_string(&meta).unwrap();
        let reparsed: WorkflowMeta = serde_json::from_str(&back).unwrap();
        assert!(reparsed.extra.contains_key("somethingNew"));
        assert!(reparsed.pr.unwrap().extra.contains_key("futureField"));
    }

    #[test]
    fn test_parse_workflow_meta_empty_json() {
        let meta: WorkflowMeta = serde_json::from_str("{}").unwrap();
        assert_eq!(meta.status, WorkflowStatus::Draft);
        assert_eq!(meta.iteration, 0);
        assert!(meta.pr.is_none());
    }

    #[test]
    fn test_parse_annotation_lenient() {
        let json = r#"{
            "id": "a-1",
            "file": "src/lib.rs",
            "side": "old",
            "line": 42,
            "status": "wontfix",
            "author": "agent",
            "replies": [{"author": "user", "body": "ok"}],
            "futureField": true
        }"#;
        let ann: Annotation = serde_json::from_str(json).unwrap();
        assert_eq!(ann.side, DiffSide::Old);
        assert_eq!(ann.line, 42);
        assert_eq!(ann.status, AnnotationStatus::Wontfix);
        assert_eq!(ann.replies.len(), 1);
        assert!(ann.extra.contains_key("futureField"));
        // Unknown status value degrades, never fails.
        let odd: Annotation = serde_json::from_str(r#"{"status": "deferred"}"#).unwrap();
        assert_eq!(odd.status, AnnotationStatus::Unknown);
        assert_eq!(odd.side, DiffSide::New);
    }

    #[test]
    fn test_annotations_file_round_trip() {
        let file = AnnotationsFile {
            annotations: vec![Annotation {
                id: "a-1".into(),
                body: "rename this".into(),
                ..Annotation::default()
            }],
            ..AnnotationsFile::default()
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: AnnotationsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.annotations.len(), 1);
        assert_eq!(back.annotations[0].body, "rename this");
        // Empty file is valid too.
        let empty: AnnotationsFile = serde_json::from_str("{}").unwrap();
        assert!(empty.annotations.is_empty());
    }
}
