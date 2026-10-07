//! Check runs: several review/explain rounds over one iteration of a workflow
//! item, then at most one change round for all of their findings.
//!
//! Pure. The run lives in the item's clash-only `run.json`, which records only
//! what clash *intended*; whether a round started, finished or was applied is
//! read back from the files the agents and `workflow_request_changes` write
//! (`meta.json`, `agent-review.md`, `explainers.json`). [`reconcile`] compares
//! the two and says what to do next, so a restart resumes a run instead of
//! forgetting it. Contract: `docs/workflows.md` → *Check runs*.

use crate::domain::workflow::{
    AgentReviewSummary, ApplyState, ExplainerState, PassState, ReviewDepth, ReviewPublish,
    ReviewTarget, RunApply, RunBatch, RunFile, RunPass, RunRecord, WorkflowMeta, WorkflowReview,
    WorkflowStatus,
};
use serde::{Deserialize, Serialize};

/// How long a recorded launch may go unconfirmed before it counts as lost.
/// A review launch spawns a session but never checks out a worktree, so
/// anything past this is a crash, not a slow spawn.
pub const LAUNCH_GRACE_MS: i64 = 120_000;
/// Relaunches of one pass after lost launches, before it is marked failed.
pub const MAX_LAUNCH_ATTEMPTS: u32 = 3;
const HISTORY_CAP: usize = 20;
const APPLIED_KEYS_CAP: usize = 50;

/// One pass as a launcher asks for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PassSpec {
    pub target: ReviewTarget,
    #[serde(default)]
    pub depth: ReviewDepth,
    #[serde(default)]
    pub publish: ReviewPublish,
    #[serde(default)]
    pub pr_urls: Vec<String>,
    #[serde(default)]
    pub focus: String,
}

/// Everything about a new run that is not its passes.
#[derive(Debug, Clone, Default)]
pub struct BatchStart {
    pub status: WorkflowStatus,
    pub iteration: u32,
    /// `human` | `autopilot`.
    pub by: String,
    pub interactive: Option<bool>,
    pub auto_apply: bool,
    pub agent: String,
    pub now: i64,
}

/// Launch order, and the identity a run dedups on: one pass per rank.
/// Explainers first, since they run alongside everything and start at once; then the judging passes, cheapest question first, with the
/// self-review last because its verdict should include everything found
/// before it.
pub fn pass_rank(target: ReviewTarget, publish: ReviewPublish) -> u8 {
    match target {
        ReviewTarget::ExplainPlan => 0,
        ReviewTarget::ExplainDiff => 1,
        // Answering PR comments is a mode of the stage's own review, whichever
        // artifact that is.
        _ if publish == ReviewPublish::RespondPrComments => 5,
        ReviewTarget::Plan => 2,
        ReviewTarget::Diff => 3,
        ReviewTarget::Drift => 4,
        ReviewTarget::SelfReview => 6,
        ReviewTarget::Unknown => 7,
    }
}

/// Does this pass park the item? Explainers never do.
pub fn is_judging(p: &RunPass) -> bool {
    !p.target.explains()
}

/// Start a run on `run`. Refuses a second one, a stage with nothing to
/// review, an empty pick and an unknown target; sorts the passes into launch
/// order and drops duplicates, so the order never depends on what was ticked
/// first.
pub fn start_batch(
    run: &mut RunFile,
    specs: Vec<PassSpec>,
    start: BatchStart,
) -> Result<u32, String> {
    if run.batch.is_some() {
        return Err("A check run is already in progress on this item".to_string());
    }
    if !start.status.can_request_review() {
        return Err(format!(
            "Can't run checks on an item in '{}' — wait for the current phase to hand back",
            start.status
        ));
    }
    let mut passes: Vec<RunPass> = Vec::new();
    for spec in specs {
        if spec.target == ReviewTarget::Unknown {
            return Err("Unknown check".to_string());
        }
        let rank = pass_rank(spec.target, spec.publish);
        if passes
            .iter()
            .any(|p| pass_rank(p.target, p.publish) == rank)
        {
            continue;
        }
        passes.push(RunPass {
            target: spec.target,
            depth: spec.depth,
            publish: spec.publish,
            pr_urls: spec.pr_urls,
            focus: spec.focus.trim().to_string(),
            ..Default::default()
        });
    }
    if passes.is_empty() {
        return Err("Pick at least one check".to_string());
    }
    passes.sort_by_key(|p| pass_rank(p.target, p.publish));
    run.next_id = run.next_id.saturating_add(1);
    if start.by == "autopilot" {
        run.autopilot_steps = run.autopilot_steps.saturating_add(1);
    }
    run.batch = Some(RunBatch {
        id: run.next_id,
        iteration: start.iteration,
        stage: start.status,
        by: start.by,
        interactive: start.interactive,
        auto_apply: start.auto_apply,
        agent: start.agent,
        passes,
        created_at: start.now,
        ..Default::default()
    });
    Ok(run.next_id)
}

/// What is on disk, as [`reconcile`] needs it.
#[derive(Debug, Clone, Copy)]
pub struct RunInput<'a> {
    pub status: WorkflowStatus,
    pub iteration: u32,
    pub review: Option<&'a WorkflowReview>,
    pub session_id: Option<&'a str>,
    /// Every round of `agent-review.md`, in file order.
    pub rounds: &'a [AgentReviewSummary],
    pub explainers: &'a [ExplainerState],
    /// `meta.appliedReviewKeys` plus `meta.appliedReviewKey`.
    pub applied_keys: &'a [String],
    /// A launch of the item's own agent is in flight in this process.
    pub item_claimed: bool,
    /// Explainer targets whose launch is in flight in this process.
    pub explainers_claimed: &'a [ReviewTarget],
    pub now: i64,
}

/// The one side effect a run needs next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunAction {
    /// Waiting on an agent or on the human — or no run at all.
    None,
    /// Launch these passes (indices into `batch.passes`), in this order.
    Launch(Vec<usize>),
    /// Apply these rounds as one change round.
    Apply(Vec<String>),
}

/// How many rounds `target` has in the report.
fn tally(rounds: &[AgentReviewSummary], target: ReviewTarget) -> u32 {
    rounds
        .iter()
        .filter(|r| ReviewTarget::canonical(&r.target) == target.as_str())
        .count() as u32
}

fn round_of<'a>(rounds: &'a [AgentReviewSummary], p: &RunPass) -> Option<&'a AgentReviewSummary> {
    rounds
        .iter()
        .find(|r| ReviewTarget::canonical(&r.target) == p.target.as_str() && r.round == p.round)
}

/// The round key of a pass, for `meta.appliedReviewKeys`.
fn pass_key(p: &RunPass) -> String {
    crate::application::workflow::review_round_key(p.target.as_str(), p.round)
}

fn record_of(batch: &RunBatch, outcome: String, now: i64) -> RunRecord {
    RunRecord {
        id: batch.id,
        iteration: batch.iteration,
        by: batch.by.clone(),
        passes: batch
            .passes
            .iter()
            .map(|p| {
                if p.state == PassState::Done {
                    pass_key(p)
                } else {
                    let state = serde_json::to_value(p.state)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default();
                    format!("{}:{}", p.target.as_str(), state)
                }
            })
            .collect(),
        outcome,
        closed_at: now,
    }
}

fn close(run: &mut RunFile, batch: RunBatch, outcome: String, now: i64) {
    run.history.push(record_of(&batch, outcome, now));
    let excess = run.history.len().saturating_sub(HISTORY_CAP);
    run.history.drain(..excess);
}

/// A launch nobody confirmed within the grace: relaunch it, or give up.
fn lost_launch(p: &mut RunPass) -> bool {
    if p.attempts >= MAX_LAUNCH_ATTEMPTS {
        p.state = PassState::Failed;
        p.error = "the round never started".to_string();
        true
    } else {
        p.state = PassState::Queued;
        false
    }
}

/// Read the run against the disk: settle what has happened, close a run the
/// item has moved past, and say what to do next. Returns the updated file —
/// unchanged when nothing happened, so a caller writes only on a real change.
pub fn reconcile(run: &RunFile, input: &RunInput) -> (RunFile, RunAction) {
    let mut run = run.clone();
    let Some(mut batch) = run.batch.take() else {
        return (run, RunAction::None);
    };
    let applied = |k: &String| input.applied_keys.contains(k);

    // The run's findings were applied — by clash, or by the human through the
    // change composer. Request-changes stamps the keys in the same meta write
    // that bumps the iteration, so this is checked before staleness.
    if matches!(
        batch.apply.state,
        ApplyState::Pending | ApplyState::Applying
    ) && !batch.apply.keys.is_empty()
        && batch.apply.keys.iter().all(applied)
    {
        let outcome = format!("applied as iteration {}", input.iteration);
        close(&mut run, batch, outcome, input.now);
        return (run, RunAction::None);
    }
    if input.iteration != batch.iteration {
        let outcome = format!("closed: the item moved on to iteration {}", input.iteration);
        close(&mut run, batch, outcome, input.now);
        return (run, RunAction::None);
    }
    let parked = input.status != WorkflowStatus::Reviewing;
    if parked && input.status != batch.stage {
        let outcome = format!("closed: the item moved to {}", input.status);
        close(&mut run, batch, outcome, input.now);
        return (run, RunAction::None);
    }

    let stale = |p: &RunPass| input.now - p.launched_at > LAUNCH_GRACE_MS;
    for p in batch.passes.iter_mut() {
        if p.target.explains() {
            let rec = input.explainers.iter().find(|e| e.target == p.target);
            match p.state {
                PassState::Launching => {
                    if let Some(e) = rec.filter(|e| e.started_at >= p.launched_at) {
                        p.state = if e.finished {
                            PassState::Done
                        } else {
                            PassState::Running
                        };
                        p.round = e.round;
                        p.session_id = e.session_id.clone();
                    } else if !input.explainers_claimed.contains(&p.target) && stale(p) {
                        // An explainer never pauses the run: it judges nothing.
                        lost_launch(p);
                    }
                }
                PassState::Running if rec.is_some_and(|e| e.round == p.round && e.finished) => {
                    p.state = PassState::Done;
                }
                _ => {}
            }
            continue;
        }
        match p.state {
            PassState::Launching => {
                let started = input
                    .review
                    .filter(|r| r.target == p.target && r.started_at >= p.launched_at);
                if let Some(r) = started {
                    p.state = PassState::Running;
                    p.round = r.round;
                    p.session_id = input.session_id.unwrap_or_default().to_string();
                } else if !input.item_claimed && stale(p) && lost_launch(p) {
                    batch.paused = true;
                }
            }
            PassState::Running if parked => {
                if tally(input.rounds, p.target) >= p.round {
                    p.state = PassState::Done;
                } else {
                    p.state = PassState::Failed;
                    p.error = "the round ended without a report".to_string();
                    batch.paused = true;
                }
            }
            _ => {}
        }
    }

    let judging_busy = !parked
        || batch
            .passes
            .iter()
            .any(|p| is_judging(p) && matches!(p.state, PassState::Launching | PassState::Running));
    if batch.stopping && !judging_busy {
        for p in batch
            .passes
            .iter_mut()
            .filter(|p| p.state == PassState::Queued)
        {
            p.state = PassState::Skipped;
        }
    }
    if !batch.paused {
        let mut launch: Vec<usize> = batch
            .passes
            .iter()
            .enumerate()
            .filter(|(_, p)| !is_judging(p) && p.state == PassState::Queued)
            .map(|(i, _)| i)
            .collect();
        if !judging_busy {
            if let Some(i) = batch
                .passes
                .iter()
                .position(|p| is_judging(p) && p.state == PassState::Queued)
            {
                launch.push(i);
            }
        }
        if !launch.is_empty() {
            run.batch = Some(batch);
            return (run, RunAction::Launch(launch));
        }
    }

    let settled = batch
        .passes
        .iter()
        .filter(|p| is_judging(p))
        .all(|p| p.state.is_settled());
    if !settled || batch.paused || judging_busy {
        run.batch = Some(batch);
        return (run, RunAction::None);
    }

    let action = match batch.apply.state {
        ApplyState::None | ApplyState::Unknown => {
            let mut yes = Vec::new();
            let mut maybe = Vec::new();
            for p in batch
                .passes
                .iter()
                .filter(|p| is_judging(p) && p.state == PassState::Done)
            {
                let key = pass_key(p);
                if applied(&key) {
                    continue;
                }
                let says = round_of(input.rounds, p).and_then(|r| r.apply);
                if says == Some(true) {
                    yes.push(key.clone());
                }
                if says != Some(false) {
                    maybe.push(key);
                }
            }
            if maybe.is_empty() {
                close(
                    &mut run,
                    batch,
                    "finished — nothing to apply".to_string(),
                    input.now,
                );
                return (run, RunAction::None);
            }
            let auto = batch.auto_apply && !yes.is_empty();
            batch.apply = RunApply {
                state: ApplyState::Pending,
                keys: if auto { yes } else { maybe },
                auto,
                ..Default::default()
            };
            if auto {
                RunAction::Apply(batch.apply.keys.clone())
            } else {
                RunAction::None
            }
        }
        ApplyState::Pending if batch.apply.auto => RunAction::Apply(batch.apply.keys.clone()),
        ApplyState::Pending => RunAction::None,
        ApplyState::Applying => {
            if !input.item_claimed && input.now - batch.apply.started_at > LAUNCH_GRACE_MS {
                batch.apply.state = ApplyState::Pending;
                if batch.apply.auto {
                    RunAction::Apply(batch.apply.keys.clone())
                } else {
                    RunAction::None
                }
            } else {
                RunAction::None
            }
        }
    };
    run.batch = Some(batch);
    (run, action)
}

/// What the human (or the driver) can do to a run in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "op", content = "pass")]
pub enum RunControl {
    /// Finish the judging pass under way, skip the rest.
    Stop,
    /// Skip a queued or failed pass.
    Skip(usize),
    /// Queue a failed pass again.
    Retry(usize),
    /// Queue a failed review again with its findings kept local: the
    /// recovery for a round that was to post on a PR the item does not have.
    Local(usize),
    /// Close the run now. A round under way keeps running on its own.
    Dismiss,
}

fn open_batch(run: &mut RunFile, id: u32) -> Result<&mut RunBatch, String> {
    run.batch
        .as_mut()
        .filter(|b| b.id == id)
        .ok_or_else(|| "That check run is no longer open".to_string())
}

pub fn control(run: &mut RunFile, id: u32, op: RunControl, now: i64) -> Result<(), String> {
    let batch = open_batch(run, id)?;
    let pass = |batch: &mut RunBatch, i: usize| -> Result<(), String> {
        if i < batch.passes.len() {
            Ok(())
        } else {
            Err("No such check in this run".to_string())
        }
    };
    match op {
        RunControl::Stop => batch.stopping = true,
        RunControl::Skip(i) => {
            pass(batch, i)?;
            let p = &mut batch.passes[i];
            if !matches!(p.state, PassState::Queued | PassState::Failed) {
                return Err("Only a waiting or failed check can be skipped".to_string());
            }
            p.state = PassState::Skipped;
        }
        RunControl::Retry(i) => {
            pass(batch, i)?;
            let p = &mut batch.passes[i];
            if p.state != PassState::Failed {
                return Err("Only a failed check can be retried".to_string());
            }
            p.state = PassState::Queued;
            p.error.clear();
            p.attempts = 0;
        }
        RunControl::Local(i) => {
            pass(batch, i)?;
            let p = &mut batch.passes[i];
            if p.state != PassState::Failed
                || p.publish != ReviewPublish::PrComments
                || p.target.publishes()
            {
                return Err(
                    "Only a failed review that was to post its findings can run locally"
                        .to_string(),
                );
            }
            p.publish = ReviewPublish::Local;
            p.pr_urls.clear();
            p.state = PassState::Queued;
            p.error.clear();
            p.attempts = 0;
        }
        RunControl::Dismiss => {
            let batch = run.batch.take().expect("checked by open_batch");
            close(run, batch, "closed by you".to_string(), now);
            return Ok(());
        }
    }
    batch.paused = batch.passes.iter().any(|p| p.state == PassState::Failed);
    Ok(())
}

/// Mark pass `i` launched, before the spawn. A crash after this is what
/// [`reconcile`]'s lost-launch rule recovers.
pub fn begin_launch(run: &mut RunFile, id: u32, i: usize, now: i64) -> Result<RunPass, String> {
    let batch = open_batch(run, id)?;
    let p = batch
        .passes
        .get_mut(i)
        .ok_or_else(|| "No such check in this run".to_string())?;
    if p.state != PassState::Queued {
        return Err("That check is not waiting to launch".to_string());
    }
    p.state = PassState::Launching;
    p.launched_at = now;
    p.attempts = p.attempts.saturating_add(1);
    p.error.clear();
    Ok(p.clone())
}

/// A launch that returned an error. Judging passes pause the run; an
/// explainer's failure is reported and leaves the rest alone.
pub fn fail_launch(run: &mut RunFile, id: u32, i: usize, error: &str) -> Result<(), String> {
    let batch = open_batch(run, id)?;
    let judging = match batch.passes.get_mut(i) {
        Some(p) => {
            p.state = PassState::Failed;
            p.error = error.to_string();
            is_judging(p)
        }
        None => return Err("No such check in this run".to_string()),
    };
    if judging {
        batch.paused = true;
    }
    Ok(())
}

/// A launch refused because another launch of the item is in flight: not a
/// failure, just not yet — the pass waits for its turn again.
pub fn requeue_launch(run: &mut RunFile, id: u32, i: usize) -> Result<(), String> {
    let batch = open_batch(run, id)?;
    if let Some(p) = batch
        .passes
        .get_mut(i)
        .filter(|p| p.state == PassState::Launching)
    {
        p.state = PassState::Queued;
        p.attempts = p.attempts.saturating_sub(1);
    }
    Ok(())
}

/// Claim the pending apply. Refuses unless one is pending, so two drivers can
/// never both start the change round.
pub fn begin_apply(run: &mut RunFile, id: u32, now: i64) -> Result<Vec<String>, String> {
    let batch = open_batch(run, id)?;
    if batch.apply.state != ApplyState::Pending {
        return Err("Nothing is waiting to be applied".to_string());
    }
    batch.apply.state = ApplyState::Applying;
    batch.apply.started_at = now;
    batch.apply.error.clear();
    Ok(batch.apply.keys.clone())
}

/// The change round did not start: hand the apply back to the human.
pub fn abort_apply(run: &mut RunFile, id: u32, error: &str) -> Result<(), String> {
    let batch = open_batch(run, id)?;
    if batch.apply.state == ApplyState::Applying {
        batch.apply.state = ApplyState::Pending;
        batch.apply.auto = false;
        batch.apply.error = error.to_string();
    }
    Ok(())
}

/// Record that a change round carried `keys`. Newest last, deduplicated,
/// capped.
pub fn stamp_applied_keys(meta: &mut WorkflowMeta, keys: &[String]) {
    for k in keys {
        if k.is_empty() {
            continue;
        }
        meta.applied_review_keys.retain(|x| x != k);
        meta.applied_review_keys.push(k.clone());
    }
    let excess = meta
        .applied_review_keys
        .len()
        .saturating_sub(APPLIED_KEYS_CAP);
    meta.applied_review_keys.drain(..excess);
}

/// Every key a change round has carried, for [`RunInput::applied_keys`].
pub fn applied_keys(meta: &WorkflowMeta) -> Vec<String> {
    let mut keys = meta.applied_review_keys.clone();
    if !meta.applied_review_key.is_empty() && !keys.contains(&meta.applied_review_key) {
        keys.push(meta.applied_review_key.clone());
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_000_000;

    fn spec(target: ReviewTarget) -> PassSpec {
        PassSpec {
            target,
            ..Default::default()
        }
    }

    fn started(specs: Vec<PassSpec>, auto_apply: bool) -> RunFile {
        let mut run = RunFile::default();
        start_batch(
            &mut run,
            specs,
            BatchStart {
                status: WorkflowStatus::DiffReview,
                iteration: 2,
                by: "human".into(),
                auto_apply,
                now: T0,
                ..Default::default()
            },
        )
        .unwrap();
        run
    }

    fn round(target: &str, n: u32, apply: Option<bool>) -> AgentReviewSummary {
        AgentReviewSummary {
            round: n,
            target: target.into(),
            apply,
            ..Default::default()
        }
    }

    struct Disk {
        status: WorkflowStatus,
        iteration: u32,
        review: Option<WorkflowReview>,
        rounds: Vec<AgentReviewSummary>,
        explainers: Vec<ExplainerState>,
        applied: Vec<String>,
        claimed: bool,
        now: i64,
    }

    impl Disk {
        fn new() -> Self {
            Disk {
                status: WorkflowStatus::DiffReview,
                iteration: 2,
                review: None,
                rounds: Vec::new(),
                explainers: Vec::new(),
                applied: Vec::new(),
                claimed: false,
                now: T0 + 10,
            }
        }
        fn step(&self, run: &RunFile) -> (RunFile, RunAction) {
            reconcile(
                run,
                &RunInput {
                    status: self.status,
                    iteration: self.iteration,
                    review: self.review.as_ref(),
                    session_id: Some("sid"),
                    rounds: &self.rounds,
                    explainers: &self.explainers,
                    applied_keys: &self.applied,
                    item_claimed: self.claimed,
                    explainers_claimed: &[],
                    now: self.now,
                },
            )
        }
    }

    fn launch(run: &mut RunFile, i: usize, now: i64) {
        let id = run.batch.as_ref().unwrap().id;
        begin_launch(run, id, i, now).unwrap();
    }

    fn reviewing(target: ReviewTarget, n: u32, at: i64) -> WorkflowReview {
        WorkflowReview {
            target,
            round: n,
            started_at: at,
            ..Default::default()
        }
    }

    fn pass(run: &RunFile, i: usize) -> &RunPass {
        &run.batch.as_ref().unwrap().passes[i]
    }

    #[test]
    fn passes_run_in_a_fixed_order_whatever_was_ticked_first() {
        let respond = PassSpec {
            target: ReviewTarget::Diff,
            publish: ReviewPublish::RespondPrComments,
            ..Default::default()
        };
        let run = started(
            vec![
                spec(ReviewTarget::SelfReview),
                respond,
                spec(ReviewTarget::Drift),
                spec(ReviewTarget::ExplainDiff),
                spec(ReviewTarget::Diff),
                spec(ReviewTarget::Drift),
            ],
            false,
        );
        let order: Vec<(ReviewTarget, ReviewPublish)> = run
            .batch
            .unwrap()
            .passes
            .iter()
            .map(|p| (p.target, p.publish))
            .collect();
        assert_eq!(
            order,
            vec![
                (ReviewTarget::ExplainDiff, ReviewPublish::Local),
                (ReviewTarget::Diff, ReviewPublish::Local),
                (ReviewTarget::Drift, ReviewPublish::Local),
                (ReviewTarget::Diff, ReviewPublish::RespondPrComments),
                (ReviewTarget::SelfReview, ReviewPublish::Local),
            ]
        );
    }

    #[test]
    fn both_explainers_fit_in_one_run() {
        let run = started(
            vec![
                spec(ReviewTarget::ExplainDiff),
                spec(ReviewTarget::ExplainPlan),
            ],
            false,
        );
        let targets: Vec<ReviewTarget> =
            run.batch.unwrap().passes.iter().map(|p| p.target).collect();
        assert_eq!(
            targets,
            vec![ReviewTarget::ExplainPlan, ReviewTarget::ExplainDiff]
        );
    }

    #[test]
    fn a_run_refuses_a_second_run_an_empty_pick_and_a_working_stage() {
        let mut run = started(vec![spec(ReviewTarget::Diff)], false);
        let again = start_batch(
            &mut run,
            vec![spec(ReviewTarget::Diff)],
            BatchStart {
                status: WorkflowStatus::DiffReview,
                ..Default::default()
            },
        );
        assert!(again.is_err());
        let mut fresh = RunFile::default();
        let empty = start_batch(
            &mut fresh,
            vec![],
            BatchStart {
                status: WorkflowStatus::DiffReview,
                ..Default::default()
            },
        );
        assert!(empty.is_err());
        let working = start_batch(
            &mut fresh,
            vec![spec(ReviewTarget::Diff)],
            BatchStart {
                status: WorkflowStatus::Implementing,
                ..Default::default()
            },
        );
        assert!(working.is_err());
        assert!(fresh.batch.is_none());
    }

    #[test]
    fn autopilot_runs_count_against_its_budget() {
        let mut run = RunFile::default();
        start_batch(
            &mut run,
            vec![spec(ReviewTarget::Diff)],
            BatchStart {
                status: WorkflowStatus::DiffReview,
                by: "autopilot".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(run.autopilot_steps, 1);
    }

    #[test]
    fn explainers_launch_at_once_and_judges_one_at_a_time() {
        let run = started(
            vec![
                spec(ReviewTarget::Diff),
                spec(ReviewTarget::Drift),
                spec(ReviewTarget::ExplainDiff),
            ],
            false,
        );
        let (_, action) = Disk::new().step(&run);
        assert_eq!(action, RunAction::Launch(vec![0, 1]));
    }

    #[test]
    fn a_confirmed_launch_becomes_running_and_the_next_judge_waits() {
        let mut run = started(
            vec![spec(ReviewTarget::Diff), spec(ReviewTarget::Drift)],
            false,
        );
        launch(&mut run, 0, T0);
        let mut disk = Disk::new();
        disk.status = WorkflowStatus::Reviewing;
        disk.review = Some(reviewing(ReviewTarget::Diff, 3, T0 + 1));
        let (run, action) = disk.step(&run);
        assert_eq!(pass(&run, 0).state, PassState::Running);
        assert_eq!(pass(&run, 0).round, 3);
        assert_eq!(pass(&run, 0).session_id, "sid");
        assert_eq!(action, RunAction::None);
    }

    #[test]
    fn a_launch_lost_to_a_crash_is_relaunched_after_the_grace() {
        let mut run = started(vec![spec(ReviewTarget::Diff)], false);
        launch(&mut run, 0, T0);
        let mut disk = Disk::new();
        // Within the grace: still waiting for the spawn.
        let (same, action) = disk.step(&run);
        assert_eq!(same, run);
        assert_eq!(action, RunAction::None);
        // Past it, with no launch in flight: launch again.
        disk.now = T0 + LAUNCH_GRACE_MS + 1;
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::Launch(vec![0]));
        assert_eq!(pass(&after, 0).state, PassState::Queued);
        // A launch still holding the claim is slow, not lost.
        disk.claimed = true;
        assert_eq!(disk.step(&run).1, RunAction::None);
    }

    #[test]
    fn a_round_older_than_the_launch_is_not_this_pass() {
        let mut run = started(vec![spec(ReviewTarget::Diff)], false);
        launch(&mut run, 0, T0);
        let mut disk = Disk::new();
        disk.review = Some(reviewing(ReviewTarget::Diff, 2, T0 - 5));
        let (after, _) = disk.step(&run);
        assert_eq!(pass(&after, 0).state, PassState::Launching);
    }

    #[test]
    fn a_launch_that_keeps_getting_lost_fails_and_pauses_the_run() {
        let mut run = started(vec![spec(ReviewTarget::Diff)], false);
        run.batch.as_mut().unwrap().passes[0].attempts = MAX_LAUNCH_ATTEMPTS - 1;
        launch(&mut run, 0, T0);
        let mut disk = Disk::new();
        disk.now = T0 + LAUNCH_GRACE_MS + 1;
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::None);
        assert_eq!(pass(&after, 0).state, PassState::Failed);
        assert!(after.batch.unwrap().paused);
    }

    /// Run pass 0 (a diff review) to its hand-back, landing `rounds`.
    fn finished_review(
        auto_apply: bool,
        extra: Vec<PassSpec>,
        rounds: Vec<AgentReviewSummary>,
    ) -> (RunFile, Disk) {
        let mut specs = vec![spec(ReviewTarget::Diff)];
        specs.extend(extra);
        let mut run = started(specs, auto_apply);
        launch(&mut run, 0, T0);
        let mut disk = Disk::new();
        disk.status = WorkflowStatus::Reviewing;
        disk.review = Some(reviewing(ReviewTarget::Diff, 1, T0 + 1));
        let (run, _) = disk.step(&run);
        disk.status = WorkflowStatus::DiffReview;
        disk.rounds = rounds;
        (run, disk)
    }

    #[test]
    fn a_landed_round_is_done_and_the_next_judge_launches() {
        let (run, disk) = finished_review(
            false,
            vec![spec(ReviewTarget::Drift)],
            vec![round("diff", 1, Some(true))],
        );
        let (after, action) = disk.step(&run);
        assert_eq!(pass(&after, 0).state, PassState::Done);
        assert_eq!(action, RunAction::Launch(vec![1]));
    }

    #[test]
    fn a_round_that_ended_without_a_report_fails_and_pauses() {
        let (run, disk) = finished_review(false, vec![spec(ReviewTarget::Drift)], vec![]);
        let (after, action) = disk.step(&run);
        assert_eq!(pass(&after, 0).state, PassState::Failed);
        assert_eq!(action, RunAction::None);
        assert!(after.batch.unwrap().paused);
    }

    #[test]
    fn a_pre_authorized_run_applies_the_rounds_that_said_yes() {
        let (run, disk) = finished_review(true, vec![], vec![round("diff", 1, Some(true))]);
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::Apply(vec!["diff:1".into()]));
        let apply = &after.batch.as_ref().unwrap().apply;
        assert_eq!(apply.state, ApplyState::Pending);
        assert!(apply.auto);
    }

    #[test]
    fn without_authorization_the_findings_wait_for_the_human() {
        let (run, disk) = finished_review(false, vec![], vec![round("diff", 1, None)]);
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::None);
        let apply = &after.batch.as_ref().unwrap().apply;
        assert_eq!(apply.state, ApplyState::Pending);
        assert!(!apply.auto);
        assert_eq!(apply.keys, vec!["diff:1".to_string()]);
    }

    #[test]
    fn a_silent_round_is_never_auto_applied() {
        let (run, disk) = finished_review(true, vec![], vec![round("diff", 1, None)]);
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::None);
        assert!(!after.batch.unwrap().apply.auto);
    }

    #[test]
    fn a_run_whose_rounds_all_said_no_closes_with_nothing_to_apply() {
        let (run, disk) = finished_review(true, vec![], vec![round("diff", 1, Some(false))]);
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::None);
        assert!(after.batch.is_none());
        assert_eq!(
            after.history.last().unwrap().outcome,
            "finished — nothing to apply"
        );
        assert_eq!(
            after.history.last().unwrap().passes,
            vec!["diff:1".to_string()]
        );
    }

    #[test]
    fn an_apply_whose_keys_landed_closes_the_run_even_though_the_iteration_moved() {
        let (run, disk) = finished_review(true, vec![], vec![round("diff", 1, Some(true))]);
        let (mut run, _) = disk.step(&run);
        let id = run.batch.as_ref().unwrap().id;
        begin_apply(&mut run, id, T0 + 20).unwrap();
        let mut disk = disk;
        disk.iteration = 3;
        disk.status = WorkflowStatus::ChangesRequested;
        disk.applied = vec!["diff:1".into()];
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::None);
        assert!(after.batch.is_none());
        assert_eq!(
            after.history.last().unwrap().outcome,
            "applied as iteration 3"
        );
    }

    #[test]
    fn findings_applied_by_hand_through_the_composer_close_the_run_as_applied() {
        let (run, disk) = finished_review(false, vec![], vec![round("diff", 1, None)]);
        let (run, _) = disk.step(&run);
        let mut disk = disk;
        disk.iteration = 3;
        disk.status = WorkflowStatus::ChangesRequested;
        disk.applied = vec!["diff:1".into()];
        let (after, _) = disk.step(&run);
        assert!(after.batch.is_none());
        assert_eq!(
            after.history.last().unwrap().outcome,
            "applied as iteration 3"
        );
    }

    #[test]
    fn an_apply_lost_to_a_crash_is_retried_after_the_grace() {
        let (run, disk) = finished_review(true, vec![], vec![round("diff", 1, Some(true))]);
        let (mut run, _) = disk.step(&run);
        let id = run.batch.as_ref().unwrap().id;
        begin_apply(&mut run, id, T0 + 20).unwrap();
        let mut disk = disk;
        assert_eq!(disk.step(&run).1, RunAction::None);
        disk.now = T0 + 20 + LAUNCH_GRACE_MS + 1;
        assert_eq!(disk.step(&run).1, RunAction::Apply(vec!["diff:1".into()]));
    }

    #[test]
    fn a_run_closes_when_the_item_moves_on_under_it() {
        let run = started(vec![spec(ReviewTarget::Diff)], false);
        let mut disk = Disk::new();
        disk.iteration = 3;
        let (after, _) = disk.step(&run);
        assert!(after.batch.is_none());
        let mut disk = Disk::new();
        disk.status = WorkflowStatus::PrDraft;
        let (after, _) = disk.step(&run);
        assert!(after.batch.is_none());
        assert!(after.history.last().unwrap().outcome.contains("pr-draft"));
    }

    #[test]
    fn stop_skips_the_rest_once_the_running_judge_lands() {
        let (mut run, disk) = finished_review(
            false,
            vec![spec(ReviewTarget::Drift)],
            vec![round("diff", 1, Some(false))],
        );
        let id = run.batch.as_ref().unwrap().id;
        control(&mut run, id, RunControl::Stop, T0).unwrap();
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::None);
        assert!(after.batch.is_none());
        assert_eq!(after.history.last().unwrap().passes[1], "drift:skipped");
    }

    #[test]
    fn retry_and_skip_unpause_a_failed_run() {
        let (run, disk) = finished_review(false, vec![spec(ReviewTarget::Drift)], vec![]);
        let (mut run, _) = disk.step(&run);
        let id = run.batch.as_ref().unwrap().id;
        let mut skipped = run.clone();
        control(&mut skipped, id, RunControl::Skip(0), T0).unwrap();
        assert!(!skipped.batch.as_ref().unwrap().paused);
        assert_eq!(disk.step(&skipped).1, RunAction::Launch(vec![1]));
        control(&mut run, id, RunControl::Retry(0), T0).unwrap();
        assert_eq!(disk.step(&run).1, RunAction::Launch(vec![0]));
        assert!(control(&mut run, id, RunControl::Retry(0), T0).is_err());
    }

    #[test]
    fn a_review_that_could_not_post_can_run_locally_instead() {
        let mut run = started(
            vec![PassSpec {
                target: ReviewTarget::Diff,
                publish: ReviewPublish::PrComments,
                pr_urls: vec!["u".into()],
                ..Default::default()
            }],
            false,
        );
        let id = run.batch.as_ref().unwrap().id;
        launch(&mut run, 0, T0);
        fail_launch(&mut run, id, 0, "no-pr: none").unwrap();
        control(&mut run, id, RunControl::Local(0), T0).unwrap();
        let p = pass(&run, 0);
        assert_eq!(
            (p.state, p.publish),
            (PassState::Queued, ReviewPublish::Local)
        );
        assert!(p.pr_urls.is_empty());
        assert!(!run.batch.as_ref().unwrap().paused);
        // A self-review's verdict IS the post: it has no local form.
        let mut sr = started(vec![spec(ReviewTarget::SelfReview)], false);
        let id = sr.batch.as_ref().unwrap().id;
        sr.batch.as_mut().unwrap().passes[0].publish = ReviewPublish::PrComments;
        launch(&mut sr, 0, T0);
        fail_launch(&mut sr, id, 0, "no-pr: none").unwrap();
        assert!(control(&mut sr, id, RunControl::Local(0), T0).is_err());
    }

    #[test]
    fn controls_refuse_a_run_that_is_no_longer_open() {
        let mut run = started(vec![spec(ReviewTarget::Diff)], false);
        let id = run.batch.as_ref().unwrap().id;
        control(&mut run, id, RunControl::Dismiss, T0).unwrap();
        assert!(run.batch.is_none());
        assert!(control(&mut run, id, RunControl::Stop, T0).is_err());
        assert!(begin_apply(&mut run, id, T0).is_err());
    }

    #[test]
    fn an_explainer_finishes_on_its_own_and_never_blocks_the_apply() {
        let mut run = started(
            vec![spec(ReviewTarget::ExplainDiff), spec(ReviewTarget::Diff)],
            true,
        );
        launch(&mut run, 0, T0);
        launch(&mut run, 1, T0);
        let mut disk = Disk::new();
        disk.explainers = vec![ExplainerState {
            target: ReviewTarget::ExplainDiff,
            round: 4,
            started_at: T0 + 2,
            ..Default::default()
        }];
        disk.status = WorkflowStatus::Reviewing;
        disk.review = Some(reviewing(ReviewTarget::Diff, 1, T0 + 1));
        let (run, _) = disk.step(&run);
        assert_eq!(pass(&run, 0).state, PassState::Running);
        disk.status = WorkflowStatus::DiffReview;
        disk.rounds = vec![round("diff", 1, Some(true))];
        let (run, action) = disk.step(&run);
        assert_eq!(action, RunAction::Apply(vec!["diff:1".into()]));
        disk.explainers[0].finished = true;
        let (run, _) = disk.step(&run);
        assert_eq!(pass(&run, 0).state, PassState::Done);
    }

    #[test]
    fn an_explainer_launch_failure_does_not_pause_the_run() {
        let mut run = started(
            vec![spec(ReviewTarget::ExplainDiff), spec(ReviewTarget::Diff)],
            false,
        );
        let id = run.batch.as_ref().unwrap().id;
        launch(&mut run, 0, T0);
        fail_launch(&mut run, id, 0, "boom").unwrap();
        assert!(!run.batch.as_ref().unwrap().paused);
        assert_eq!(Disk::new().step(&run).1, RunAction::Launch(vec![1]));
    }

    #[test]
    fn already_applied_rounds_are_not_applied_twice() {
        let (run, mut disk) = finished_review(true, vec![], vec![round("diff", 1, Some(true))]);
        disk.applied = vec!["diff:1".into()];
        let (after, action) = disk.step(&run);
        assert_eq!(action, RunAction::None);
        assert!(after.batch.is_none());
    }

    #[test]
    fn begin_apply_can_be_claimed_once() {
        let (run, disk) = finished_review(false, vec![], vec![round("diff", 1, None)]);
        let (mut run, _) = disk.step(&run);
        let id = run.batch.as_ref().unwrap().id;
        assert_eq!(
            begin_apply(&mut run, id, T0).unwrap(),
            vec!["diff:1".to_string()]
        );
        assert!(begin_apply(&mut run, id, T0).is_err());
        abort_apply(&mut run, id, "spawn failed").unwrap();
        let apply = &run.batch.as_ref().unwrap().apply;
        assert_eq!(apply.state, ApplyState::Pending);
        assert_eq!(apply.error, "spawn failed");
    }

    #[test]
    fn applied_keys_are_deduplicated_newest_last_and_include_the_legacy_key() {
        let mut meta = WorkflowMeta {
            applied_review_key: "plan:2".into(),
            ..Default::default()
        };
        stamp_applied_keys(&mut meta, &["diff:1".into(), "drift:1".into()]);
        stamp_applied_keys(&mut meta, &["diff:1".into()]);
        assert_eq!(
            meta.applied_review_keys,
            vec!["drift:1".to_string(), "diff:1".into()]
        );
        assert_eq!(
            applied_keys(&meta),
            vec!["drift:1".to_string(), "diff:1".into(), "plan:2".into()]
        );
    }

    #[test]
    fn history_is_capped() {
        let mut run = RunFile::default();
        for _ in 0..(HISTORY_CAP + 5) {
            start_batch(
                &mut run,
                vec![spec(ReviewTarget::Diff)],
                BatchStart {
                    status: WorkflowStatus::DiffReview,
                    ..Default::default()
                },
            )
            .unwrap();
            let id = run.batch.as_ref().unwrap().id;
            control(&mut run, id, RunControl::Dismiss, T0).unwrap();
        }
        assert_eq!(run.history.len(), HISTORY_CAP);
        assert_eq!(run.history.last().unwrap().id, (HISTORY_CAP + 5) as u32);
    }
}
