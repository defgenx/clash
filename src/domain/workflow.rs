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
///
/// The `pr-*` stages are **optional**: approving at `diff-review` may go
/// straight to `done`. Requiring a draft PR to approve would strand every repo
/// that merges to its default branch without one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkflowStatus {
    #[default]
    Draft,
    Planning,
    PlanReview,
    ChangesRequested,
    Implementing,
    /// An agent reviewer is working on the item. Entered from any state where
    /// there is something to review and always left by returning to
    /// [`WorkflowReview::return_status`] — that round-trip is what makes
    /// reviews repeatable without number: run one, land back where you were,
    /// run another.
    Reviewing,
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
            Self::Reviewing => "reviewing",
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
        // An agent review can be launched from any state that has something to
        // review, and returns to the one it came from — so a human may run as
        // many rounds as they want without the item ever advancing.
        if next == Reviewing {
            return self.can_request_review();
        }
        match self {
            Draft => matches!(next, Planning),
            Planning => matches!(next, PlanReview | Draft),
            PlanReview => matches!(next, Implementing | ChangesRequested | Planning),
            ChangesRequested => matches!(next, Implementing | PlanReview),
            // `PlanReview` is included because a `revise` launch parks the item
            // in `implementing` while the agent works, and a revision that only
            // touched the plan legally hands back to `plan-review`.
            Implementing => matches!(next, DiffReview | PrDraft | ChangesRequested | PlanReview),
            // Back to wherever the review was launched from. `ChangesRequested`
            // is included so findings can be turned into a change round
            // directly, without a detour through the origin state.
            Reviewing => matches!(
                next,
                PlanReview | DiffReview | PrDraft | PrReady | ChangesRequested
            ),
            // `Done` closes the item outright. Approval at `diff-review` means
            // the human is satisfied with the diff; the PR stages are one way to
            // continue, never a precondition — a repo that merges straight to
            // its default branch has no PR to shepherd, and review-only items
            // track a PR clash doesn't own.
            DiffReview => matches!(next, PrDraft | ChangesRequested | Implementing | Done),
            PrDraft => matches!(next, PrReady | Done | DiffReview | ChangesRequested),
            // `ChangesRequested` is reachable from both PR states: review
            // feedback (agent rounds, GitHub review comments) keeps arriving
            // after the PR exists, and without this edge those findings could
            // never become a fix round — the item was a dead end.
            PrReady => matches!(next, Done | PrDraft | ChangesRequested),
            // Reopen path for finished items.
            Done => matches!(next, DiffReview),
            Abandoned => matches!(next, DiffReview | Draft),
            // An unknown on-disk status can be repaired to anything.
            Unknown => true,
        }
    }

    /// States a human may launch an agent review round from: the ones where an
    /// artifact worth reviewing already exists and the pipeline is parked on a
    /// decision. Deliberately excludes the states where an agent is already
    /// working (`planning`, `implementing`, `reviewing`) and the terminal ones.
    pub fn can_request_review(&self) -> bool {
        matches!(
            self,
            Self::PlanReview | Self::DiffReview | Self::PrDraft | Self::PrReady
        )
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
    /// agent implements, the human reviews the diff, then — optionally — the PR
    /// stages. Approving the diff may close the item directly.
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

/// What an agent review round looks at. Plan/diff are derived from the item's
/// status at launch (see [`ReviewTarget::for_status`]) rather than chosen by
/// the human, because a plan review at `diff-review` (or vice versa) has
/// nothing to read; `structure` is launched by its own explicit action (the
/// "Explain changes" button) and never derived.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewTarget {
    /// `plan.md` — is the plan right, complete, and grounded in the real code?
    #[default]
    Plan,
    /// The working diff — is the implementation correct?
    Diff,
    /// The working diff again, but to *explain* rather than judge: the round
    /// writes `structure.md` (the Structure tab) instead of findings.
    Structure,
    #[serde(other)]
    Unknown,
}

impl ReviewTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Diff => "diff",
            Self::Structure => "structure",
            Self::Unknown => "unknown",
        }
    }

    /// The only sensible target for an item parked in `status`. `plan-review`
    /// reviews the plan; everything else reviews the code. A mode with no plan
    /// phase (`review-only`) can only ever review the diff.
    pub fn for_status(status: WorkflowStatus, mode: WorkflowMode) -> Self {
        if status == WorkflowStatus::PlanReview && mode.has_plan_phase() {
            Self::Plan
        } else {
            Self::Diff
        }
    }
}

impl std::fmt::Display for ReviewTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How hard the reviewer digs. The distinction is about *grounding*, not
/// verbosity: a standard round reasons about the artifact in front of it, a
/// deep round goes and reads the surrounding implementation to check the
/// artifact against how the code actually works.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewDepth {
    /// Read the artifact plus the files it names. Fast pass.
    #[default]
    Standard,
    /// Trace the subsystems the change touches end to end — callers, invariants,
    /// tests, adjacent code — and verify the artifact against them.
    Deep,
    #[serde(other)]
    Unknown,
}

impl ReviewDepth {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Deep => "deep",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ReviewDepth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the round does with its findings beyond writing them into the item.
/// Chosen per round, because the answer genuinely changes over the life of an
/// item: early rounds stay local, a round before handing the PR to reviewers
/// publishes, and a round after they comment answers them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewPublish {
    /// Findings stay in the item (`agent-review.md` + annotations). Nothing
    /// leaves the machine.
    #[default]
    Local,
    /// Also post the findings to the PR as a review with line comments.
    PrComments,
    /// Read the PR's existing review comments, address them, and reply on the
    /// PR thread. Findings still land locally.
    RespondPrComments,
    #[serde(other)]
    Unknown,
}

impl ReviewPublish {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::PrComments => "pr-comments",
            Self::RespondPrComments => "respond-pr-comments",
            Self::Unknown => "unknown",
        }
    }

    /// True when the round talks to the forge, so the launcher can refuse it
    /// on an item with no PR instead of letting the agent discover that.
    pub fn needs_pr(&self) -> bool {
        matches!(self, Self::PrComments | Self::RespondPrComments)
    }
}

impl std::fmt::Display for ReviewPublish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The review round currently in flight, recorded in `meta.json.review` when
/// clash launches a reviewer and left in place afterwards as the record of the
/// last round.
///
/// `return_status` is the whole reason this block exists: the reviewer reads it
/// to know where to put the item back, so N rounds in a row all land the item
/// exactly where the human started from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowReview {
    #[serde(default)]
    pub target: ReviewTarget,
    #[serde(default)]
    pub depth: ReviewDepth,
    #[serde(default)]
    pub publish: ReviewPublish,
    /// Status to restore when the round finishes — the status the item was in
    /// when the round was launched.
    #[serde(default)]
    pub return_status: WorkflowStatus,
    /// 1-based round number, matching the `## Review N` section the agent
    /// appends to `agent-review.md`.
    #[serde(default)]
    pub round: u32,
    /// How the round runs: `Some(true)` = interactive (checkpoints, no
    /// opening question), `Some(false)` = autonomous, `None` = the human
    /// chose neither at launch, so the skill asks in-session before starting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive: Option<bool>,
    /// Whether clash may act on the round's own `**Apply:** yes` without a
    /// second click: the composer's "apply the findings when the round
    /// finishes" checkbox. Pre-authorization, not automation — the reviewer
    /// still decides whether there is anything worth applying, and a round
    /// that answers `no` applies nothing however this is set.
    ///
    /// Defaults to false so an older round, or one launched by a surface that
    /// never asked, can never spawn an executor nobody authorized.
    #[serde(default)]
    pub auto_apply: bool,
    /// The PR this round talks to, when the launcher picked one — multi-repo
    /// items answer reviewers per PR. Empty means the primary `meta.pr`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pr_url: String,
    #[serde(default)]
    pub started_at: i64,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
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
    /// Review-comment threads on the PR nobody has replied to, as of the last
    /// refresh. `None` until first fetched (gh unavailable, pre-existing
    /// items) — the GUI shows the count on its "Answer PR comments" action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unanswered_comments: Option<u64>,
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
    /// Free-form intent from the human: what is being built and why. The
    /// planning agent reads it as the primary source before opening its
    /// requirements discussion; empty means the title is all there is.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
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
    /// Change requests in *other* repositories that belong to this piece of
    /// work (a backend/frontend/contract split lands as several PRs). Tracked,
    /// refreshed and opened alongside the primary `pr`, but they never drive
    /// the item's status — only the primary does. Each entry's `url` is the
    /// identity; the owning repo is derived from it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_prs: Vec<WorkflowPr>,
    /// The agent review round in flight, or the last one that ran. Absent on
    /// items that were never reviewed by an agent.
    #[serde(default)]
    pub review: Option<WorkflowReview>,
    /// How many agent review rounds have been launched. Bumped only by clash
    /// (never by the agent), like `iteration` — reviews are unbounded, so this
    /// just keeps climbing and numbers the `agent-review.md` sections.
    #[serde(default)]
    pub review_round: u32,
    /// When true, this item's agent sessions carry the bare job name
    /// (`implement`) instead of the title-prefixed default
    /// (`Auth refactor · implement`). Stored as the negative so both serde's
    /// missing-field default and `WorkflowMeta::default()` agree on
    /// "prefixed" — a `default = "true_fn"` field silently reads `false` on
    /// every struct-literal construction. Toggled in the item Settings tab.
    #[serde(default)]
    pub bare_session_names: bool,
    /// Per-item PR-creation skill override (item Settings tab). Empty
    /// inherits the global `workflows.pr_skill` setting; `none` disables the
    /// skill for this item.
    #[serde(default)]
    pub pr_skill: String,
    /// Per-item default for how agent rounds run (item Settings tab):
    /// `""`/`ask` (the skill asks in-session), `interactive`, or
    /// `autonomous`. Pre-fills the review composer and applies to executor
    /// launches that offer no interaction choice of their own.
    #[serde(default)]
    pub interaction_default: String,
    /// Jira ticket this item belongs to (`PROJ-123`). Pre-fills the share
    /// dialog's Post-to-Jira prompt and is remembered after the first post;
    /// also editable in the item Settings tab. Empty means "detect from
    /// title/branch".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub jira_ticket: String,
    /// The highest agent review round whose findings have been handed to the
    /// executor. Every change round does that — the executor's contract has it
    /// read the latest `agent-review.md` round as input — so clash records it
    /// on request-changes. `review_round > applied_review_round` is therefore
    /// "a review has landed and nothing has been done with it yet", which is
    /// what the plan-review UI needs to stop looking like the review
    /// evaporated. Never written by the agent.
    #[serde(default)]
    pub applied_review_round: u32,
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
    /// Free-form intent: what is being built and why, in the human's words.
    /// The planning agent reads it before anything else — a title alone
    /// forces the requirements discussion to start from nothing.
    pub description: String,
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

/// One recorded revision of `plan.md`, as stored in the item's
/// `plan-history/` index.
///
/// The plan is versioned **continuously**, not per change round: clash records
/// a revision whenever it sees the file's content differ from the newest one it
/// holds, whoever wrote it — the planning agent's first draft, a revise round,
/// a hand-edit through the Edit button. Tying versions to change rounds lost
/// every plan written between them, which is most of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRevision {
    /// 1-based revision number and file name (`plan-history/0003.md`). The
    /// identity: iterations repeat and timestamps collide.
    #[serde(default)]
    pub n: u32,
    /// When clash recorded it (epoch ms) — not when it was written, which no
    /// writer tells us.
    #[serde(default)]
    pub saved_at: i64,
    /// The item's iteration at record time, so a revision can be tied back to
    /// the round it belongs to.
    #[serde(default)]
    pub iteration: u32,
    /// FNV-1a of the trimmed content — how "unchanged" is decided.
    #[serde(default)]
    pub hash: String,
    /// Why it was recorded, in one phrase ("first plan", "revision requested
    /// at iteration 2", "changed on disk"). Numbered versions are unreadable
    /// on their own.
    #[serde(default)]
    pub reason: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// The `plan-history/index.json` file: every recorded revision, oldest first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanHistory {
    #[serde(default)]
    pub versions: Vec<PlanRevision>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// One entry of the Plan tab's version switcher — a `PlanRevision` plus what
/// the view needs to label and order it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanVersion {
    /// Revision number (`v3`).
    pub n: u32,
    /// True for the newest revision — the live `plan.md`. Always exactly one,
    /// because clash records the current file before listing.
    pub current: bool,
    /// Line count, so the switcher can show growth without loading the text.
    pub lines: usize,
    /// Epoch ms this revision was recorded.
    pub saved_at: i64,
    /// The item's iteration when it was recorded.
    pub iteration: u32,
    /// Why it was recorded.
    pub reason: String,
}

/// One `## Iteration N` section of `review.md`, parsed at read time — a
/// runtime DTO, never persisted (the file is the source of truth). The
/// Timeline view renders one change-round card per entry: the human's note is
/// *why* that round happened, which the flat history list never showed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewIterationNote {
    /// Iteration number from the `## Iteration <n>` heading.
    pub iteration: u32,
    /// Heading tail after the number — normally the `YYYY-MM-DD HH:MM` stamp
    /// clash wrote.
    pub heading: String,
    /// The human's change-request note, verbatim markdown (open-annotations
    /// digest excluded).
    pub note: String,
    /// The `### Open annotations` digest lines, bullets stripped.
    pub annotations: Vec<String>,
}

/// The latest round of `agent-review.md`, parsed at list time — a runtime DTO,
/// never persisted (the file is the source of truth). What the GUI needs to
/// answer "what did the last review round conclude, and did it publish
/// anything?" without the user opening the whole report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewSummary {
    /// 1-based round number from the `## Review <n>` heading.
    pub round: u32,
    /// Heading tail after the round number, e.g. `diff · deep · 2026-08-04 17:27`.
    pub heading: String,
    /// The `**Verdict:**` paragraph, whitespace-collapsed to one line.
    pub verdict: String,
    /// The `### Published` bullet lines; empty when the round declared nothing.
    pub published: Vec<String>,
    /// The round's own call on whether its findings should be applied to the
    /// artifact now (`**Apply:** yes|no`). `None` when the round declared
    /// nothing — every round before this contract existed, and any round whose
    /// report is malformed. It is a *recommendation*: applying is still a
    /// change round clash runs, never something the reviewer did itself.
    #[serde(default)]
    pub apply: Option<bool>,
    /// The one-line reason the round gave for that call. Shown next to the
    /// action, because "the reviewer says apply this" is only useful with the
    /// because.
    #[serde(default)]
    pub apply_reason: String,
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
    /// True once `agent-review.md` holds at least one review round.
    pub has_agent_review: bool,
    /// True once `structure.md` (the explain round's document) has content —
    /// gates the GUI's Structure tab.
    pub has_structure: bool,
    /// Count of annotations with status `open` (0 for terminal items, whose
    /// annotations are not read during listing).
    pub open_annotations: usize,
    /// Snapshotted iterations found under `history/`, sorted ascending.
    pub history_iterations: Vec<u32>,
    /// False when `meta.session_id` points at a session that is no longer
    /// alive while the item claims an agent is working (planning /
    /// implementing). Computed by the GUI layer against live sessions.
    pub agent_alive: bool,
    /// Latest round parsed from `agent-review.md`, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_agent_review: Option<AgentReviewSummary>,
}

/// Resolution state of a diff annotation.
///
/// `Parked` is "kept but not sent": the human wrote it, chose not to include
/// it in a change round yet, and can reopen it later. The agent contract is
/// untouched by parking — agents act on `open` annotations only, so a parked
/// one is invisible to them without any skill change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationStatus {
    #[default]
    Open,
    Addressed,
    Wontfix,
    Parked,
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

    const ALL_WORKFLOW_STATUSES: [WorkflowStatus; 12] = [
        WorkflowStatus::Draft,
        WorkflowStatus::Planning,
        WorkflowStatus::PlanReview,
        WorkflowStatus::ChangesRequested,
        WorkflowStatus::Implementing,
        WorkflowStatus::Reviewing,
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
            (DiffReview, PrDraft),          // Approve → PR draft (PR exists)
            (DiffReview, Done),             // Approve → done (no PR required)
            (DiffReview, ChangesRequested), // Request changes
            (PrDraft, PrReady),             // Mark PR ready
            (PrDraft, ChangesRequested),    // Request changes on the draft PR
            (PrReady, Done),                // Mark done
            (PrReady, ChangesRequested),    // Request changes on the ready PR
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
            // A `revise` launch parks the item in `implementing`; a revision
            // that only touched the plan hands back to `plan-review`.
            (Implementing, PlanReview),
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
    fn test_linked_prs_round_trip_and_stay_off_disk_when_empty() {
        // Items written before linked PRs existed read back with an empty
        // list, and an empty list never lands on disk (no churn on every
        // meta write of every existing item).
        let old: WorkflowMeta = serde_json::from_str(r#"{"title":"pre-linked"}"#).unwrap();
        assert!(old.linked_prs.is_empty());
        let back = serde_json::to_string(&old).unwrap();
        assert!(!back.contains("linkedPrs"));

        let json = r#"{
            "pr": {"url": "https://github.com/o/r/pull/7", "number": 7},
            "linkedPrs": [
                {"url": "https://github.com/o/front/pull/12", "number": 12, "draft": true},
                {"url": "https://github.com/o/contracts/pull/3", "state": "MERGED", "futureField": 1}
            ]
        }"#;
        let meta: WorkflowMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.linked_prs.len(), 2);
        assert_eq!(meta.linked_prs[0].number, 12);
        assert!(meta.linked_prs[0].draft);
        // A URL-only entry is legal — number derives from the URL at need,
        // same as the primary's agent contract.
        assert_eq!(meta.linked_prs[1].number, 0);
        assert_eq!(meta.linked_prs[1].state, "MERGED");
        let back = serde_json::to_string(&meta).unwrap();
        let reparsed: WorkflowMeta = serde_json::from_str(&back).unwrap();
        assert_eq!(reparsed.linked_prs.len(), 2);
        assert!(reparsed.linked_prs[1].extra.contains_key("futureField"));
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
        // Items written before the toggle existed keep prefixed sessions
        // (and so does a bare struct-literal construction).
        assert!(!meta.bare_session_names);
        assert!(!WorkflowMeta::default().bare_session_names);
        let off: WorkflowMeta = serde_json::from_str(r#"{"bareSessionNames": true}"#).unwrap();
        assert!(off.bare_session_names);
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
        // Parked round-trips (kept-but-not-sent; agents act on `open` only).
        let parked: Annotation = serde_json::from_str(r#"{"status": "parked"}"#).unwrap();
        assert_eq!(parked.status, AnnotationStatus::Parked);
        assert_eq!(
            serde_json::to_string(&AnnotationStatus::Parked).unwrap(),
            r#""parked""#
        );
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

    // ── Agent review rounds ─────────────────────────────────────────

    #[test]
    fn review_is_reachable_from_every_decision_state_and_nowhere_else() {
        use WorkflowStatus::*;
        for s in [PlanReview, DiffReview, PrDraft, PrReady] {
            assert!(s.can_request_review(), "{s} should allow a review round");
            assert!(s.can_transition_to(Reviewing), "{s} -> reviewing");
        }
        // States where an agent is already working, or that are finished, have
        // nothing to hand to a reviewer.
        for s in [
            Draft,
            Planning,
            ChangesRequested,
            Implementing,
            Done,
            Abandoned,
        ] {
            assert!(!s.can_request_review(), "{s} should refuse a review round");
            assert!(!s.can_transition_to(Reviewing), "{s} -> reviewing");
        }
        // Not even from itself — a second round is launched after the first
        // returns, never on top of it.
        assert!(!Reviewing.can_transition_to(Reviewing));
    }

    #[test]
    fn a_review_round_returns_to_where_it_came_from() {
        use WorkflowStatus::*;
        // The repeatability contract: every state a round can start in must be
        // a legal destination when it ends, or round two could never run.
        for s in [PlanReview, DiffReview, PrDraft, PrReady] {
            assert!(
                Reviewing.can_transition_to(s),
                "reviewing must be able to return to {s}"
            );
        }
        // Plus the shortcut straight into a change round.
        assert!(Reviewing.can_transition_to(ChangesRequested));
        // A round is still an agent-working state: it must not silently finish
        // the item.
        assert!(!Reviewing.can_transition_to(Done));
    }

    #[test]
    fn reviewing_is_agent_work_not_a_human_decision() {
        // It must not join the NEEDS DECISION grouping or fire a notification —
        // the agent is busy, there is nothing to decide yet.
        assert!(!WorkflowStatus::Reviewing.needs_attention());
        assert!(!WorkflowStatus::Reviewing.is_terminal());
        // An abandon is always available, so a wedged round is never a dead end.
        assert!(WorkflowStatus::Reviewing.can_transition_to(WorkflowStatus::Abandoned));
    }

    #[test]
    fn review_target_follows_the_status_and_the_mode() {
        use WorkflowStatus::*;
        assert_eq!(
            ReviewTarget::for_status(PlanReview, WorkflowMode::Full),
            ReviewTarget::Plan
        );
        assert_eq!(
            ReviewTarget::for_status(PlanReview, WorkflowMode::FromPlan),
            ReviewTarget::Plan
        );
        // review-only has no plan at all, so even at plan-review (which it
        // never reaches) the only reviewable artifact is the diff.
        assert_eq!(
            ReviewTarget::for_status(PlanReview, WorkflowMode::ReviewOnly),
            ReviewTarget::Diff
        );
        for s in [DiffReview, PrDraft, PrReady] {
            assert_eq!(
                ReviewTarget::for_status(s, WorkflowMode::Full),
                ReviewTarget::Diff
            );
        }
    }

    #[test]
    fn publish_modes_know_when_they_need_a_forge() {
        assert!(!ReviewPublish::Local.needs_pr());
        assert!(ReviewPublish::PrComments.needs_pr());
        assert!(ReviewPublish::RespondPrComments.needs_pr());
    }

    #[test]
    fn review_enums_are_kebab_case_and_lenient() {
        assert_eq!(
            serde_json::to_string(&ReviewPublish::RespondPrComments).unwrap(),
            r#""respond-pr-comments""#
        );
        assert_eq!(
            serde_json::to_string(&ReviewDepth::Deep).unwrap(),
            r#""deep""#
        );
        assert_eq!(
            serde_json::to_string(&WorkflowStatus::Reviewing).unwrap(),
            r#""reviewing""#
        );
        // The explicit explain-round target round-trips.
        assert_eq!(
            serde_json::to_string(&ReviewTarget::Structure).unwrap(),
            r#""structure""#
        );
        let s: ReviewTarget = serde_json::from_str(r#""structure""#).unwrap();
        assert_eq!(s, ReviewTarget::Structure);
        // Unknown on-disk values degrade instead of failing the whole meta read.
        let d: ReviewDepth = serde_json::from_str(r#""paranoid""#).unwrap();
        assert_eq!(d, ReviewDepth::Unknown);
        let p: ReviewPublish = serde_json::from_str(r#""telepathy""#).unwrap();
        assert_eq!(p, ReviewPublish::Unknown);
        let t: ReviewTarget = serde_json::from_str(r#""vibes""#).unwrap();
        assert_eq!(t, ReviewTarget::Unknown);
    }

    #[test]
    fn meta_without_a_review_block_round_trips() {
        // Items written before reviews existed must read back unchanged.
        let meta: WorkflowMeta = serde_json::from_str(r#"{"title":"old","status":"diff-review"}"#)
            .expect("pre-review meta must parse");
        assert!(meta.review.is_none());
        assert_eq!(meta.review_round, 0);
        assert_eq!(meta.status, WorkflowStatus::DiffReview);
    }

    #[test]
    fn meta_preserves_a_review_block_and_unknown_fields() {
        let raw = r#"{
            "title": "auth",
            "status": "reviewing",
            "reviewRound": 3,
            "review": {
                "target": "diff", "depth": "deep", "publish": "pr-comments",
                "returnStatus": "pr-draft", "round": 3, "startedAt": 42,
                "interactive": false,
                "futureField": "kept"
            },
            "somethingNew": true
        }"#;
        let meta: WorkflowMeta = serde_json::from_str(raw).unwrap();
        let review = meta.review.clone().expect("review block");
        assert_eq!(review.target, ReviewTarget::Diff);
        assert_eq!(review.depth, ReviewDepth::Deep);
        assert_eq!(review.publish, ReviewPublish::PrComments);
        assert_eq!(review.return_status, WorkflowStatus::PrDraft);
        assert_eq!(review.round, 3);
        assert_eq!(review.interactive, Some(false));
        assert_eq!(meta.review_round, 3);
        // A review block written before `interactive` existed reads as None —
        // "ask in-session", the safe default.
        let old: WorkflowReview = serde_json::from_str(r#"{"target":"plan"}"#).unwrap();
        assert_eq!(old.interactive, None);
        // Unknown fields survive the round-trip at both levels — the agent and
        // clash both read-modify-write this file.
        let back = serde_json::to_string(&meta).unwrap();
        assert!(back.contains("somethingNew"));
        assert!(back.contains("futureField"));
    }
}
