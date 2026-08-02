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
            // `Done` closes a review-only item, where clash owns no PR to
            // shepherd — approving IS the end of the pipeline.
            DiffReview => matches!(next, PrDraft | ChangesRequested | Implementing | Done),
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

/// How an item entered the pipeline — the entry mode chosen at creation.
/// Kebab-case in `meta.json`; absent (items created before modes existed)
/// reads as [`WorkflowMode::Full`], so nothing on disk needs migrating.
///
/// The mode is fixed for the item's life: it decides the initial status, which
/// phases exist, and how approval ends the item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowMode {
    /// The whole pipeline: an agent plans, the human reviews the plan, the
    /// agent implements, the human reviews the diff, then the PR.
    #[default]
    Full,
    /// The human supplies the plan (a file, a scratch note, pasted text), so
    /// no planning agent runs: the item starts at `plan-review` with `plan.md`
    /// already written and one approval away from implementation.
    FromPlan,
    /// The review loop only, over code that already exists (a PR or a branch):
    /// the item starts at `diff-review`, has no plan, and approval finishes
    /// it. `changes-requested → implementing → diff-review` still cycles, so
    /// the agent addresses annotations exactly as in the full pipeline.
    ReviewOnly,
    #[serde(other)]
    Unknown,
}

impl WorkflowMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::FromPlan => "from-plan",
            Self::ReviewOnly => "review-only",
            Self::Unknown => "unknown",
        }
    }

    /// Status a freshly created item of this mode starts in. Creation is not a
    /// transition, so this deliberately does not consult `can_transition_to`.
    pub fn initial_status(&self) -> WorkflowStatus {
        match self {
            Self::FromPlan => WorkflowStatus::PlanReview,
            Self::ReviewOnly => WorkflowStatus::DiffReview,
            // An unknown on-disk mode degrades to the full pipeline.
            Self::Full | Self::Unknown => WorkflowStatus::Draft,
        }
    }

    /// False for review-only, which has no plan phase at all: the frontends
    /// hide the plan sub-view and the agent must never write `plan.md` nor
    /// transition to `plan-review`.
    pub fn has_plan_phase(&self) -> bool {
        !matches!(self, Self::ReviewOnly)
    }

    /// True when the code under review predates the item, so clash neither
    /// created the branch nor owns the PR: approval ends the item instead of
    /// running the draft-PR ceremony, and the agent may push its fixes to the
    /// existing branch.
    pub fn is_review_only(&self) -> bool {
        matches!(self, Self::ReviewOnly)
    }
}

impl std::fmt::Display for WorkflowMode {
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
    /// Entry mode, fixed at creation. Missing on pre-mode items → `full`.
    #[serde(default)]
    pub mode: WorkflowMode,
    /// Absolute path of the main repo checkout this item works on.
    #[serde(default)]
    pub repo_path: String,
    #[serde(default)]
    pub branch: String,
    /// Ref the diff is taken against (a branch name, e.g. a PR's base).
    /// Empty means "the repo's origin default branch".
    #[serde(default)]
    pub base: String,
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

/// Everything needed to create a workflow item. A struct rather than a long
/// parameter list because the three entry modes contribute different subsets:
/// `full` needs only project/title/repo, `from-plan` adds `plan`, `review-only`
/// adds the already-existing `branch`/`base`/`worktree`/`pr`.
#[derive(Debug, Clone, Default)]
pub struct NewWorkflowItem {
    /// Project directory (first level under the workflows root).
    pub project: String,
    /// Human title; the slug is derived from it.
    pub title: String,
    /// Absolute path of the repo checkout the item works on.
    pub repo_path: String,
    pub mode: WorkflowMode,
    /// Seed content for `plan.md` (from-plan). Empty leaves the file empty.
    pub plan: String,
    /// Branch under review (review-only). Full-mode items get theirs when the
    /// first agent launch creates the worktree.
    pub branch: String,
    /// Diff base ref; empty means the repo's origin default branch.
    pub base: String,
    /// Pre-materialized checkout for review-only items (the reused or freshly
    /// created worktree holding `branch`).
    pub worktree: Option<String>,
    /// PR being reviewed (review-only, when the source was a PR).
    pub pr: Option<WorkflowPr>,
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
            (DiffReview, Done),             // Approve (review-only)
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
    fn test_workflow_mode_serde_and_entry_points() {
        // Kebab-case on disk, and as_str agrees with it for every variant.
        for m in [
            WorkflowMode::Full,
            WorkflowMode::FromPlan,
            WorkflowMode::ReviewOnly,
            WorkflowMode::Unknown,
        ] {
            assert_eq!(
                serde_json::to_string(&m).unwrap(),
                format!("\"{}\"", m.as_str())
            );
        }
        let parsed: WorkflowMode = serde_json::from_str(r#""review-only""#).unwrap();
        assert_eq!(parsed, WorkflowMode::ReviewOnly);
        // A mode clash doesn't know degrades to the full pipeline.
        let odd: WorkflowMode = serde_json::from_str(r#""telepathy""#).unwrap();
        assert_eq!(odd, WorkflowMode::Unknown);
        assert_eq!(odd.initial_status(), WorkflowStatus::Draft);
        assert!(odd.has_plan_phase());
        assert!(!odd.is_review_only());

        assert_eq!(WorkflowMode::Full.initial_status(), WorkflowStatus::Draft);
        assert_eq!(
            WorkflowMode::FromPlan.initial_status(),
            WorkflowStatus::PlanReview
        );
        assert_eq!(
            WorkflowMode::ReviewOnly.initial_status(),
            WorkflowStatus::DiffReview
        );
        assert!(!WorkflowMode::ReviewOnly.has_plan_phase());
        assert!(WorkflowMode::ReviewOnly.is_review_only());
        assert!(WorkflowMode::FromPlan.has_plan_phase());
        assert!(!WorkflowMode::FromPlan.is_review_only());
    }

    #[test]
    fn test_entry_mode_pipelines_are_walkable() {
        use WorkflowStatus::*;
        // from-plan: land on plan-review, approve into implementation.
        assert!(WorkflowMode::FromPlan
            .initial_status()
            .can_transition_to(Implementing));
        // review-only: land on diff-review, then either close out or loop
        // through the agent and come back.
        let start = WorkflowMode::ReviewOnly.initial_status();
        assert!(start.can_transition_to(Done));
        assert!(start.can_transition_to(ChangesRequested));
        assert!(ChangesRequested.can_transition_to(Implementing));
        assert!(Implementing.can_transition_to(DiffReview));
        // …and a closed review-only item reopens into the same loop.
        assert!(Done.can_transition_to(DiffReview));
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
        // Items written before modes existed read as the full pipeline.
        assert_eq!(meta.mode, WorkflowMode::Full);
        assert_eq!(meta.base, "");
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
