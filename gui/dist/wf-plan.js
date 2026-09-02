// Plan versions and review application — the pure half.
//
// Two questions live here, both of which used to be answered inline and
// inconsistently:
//
// 1. **What versions of the plan are there, and what am I looking at?** A
//    version exists because a change round froze `plan.md` before the agent
//    revised it, so version N is "the plan as the human reviewed it at
//    iteration N" and the head is the live file. Numbers alone are unreadable,
//    so each chip carries the round's note.
//
// 2. **What note records "apply this review round"?** The note lands in
//    `review.md` and is the first thing the next round reads, so it is the
//    whole instruction — a bare "see the review" would make the round's intent
//    unrecoverable from the item's own history a month later.
//
// The view is app.js (the `wf-timeline.js` precedent).
(function () {
  "use strict";

  /// Chip label for a version: `v1`… by iteration, `current` for the head.
  /// The head is labelled by role, not by number, because its number changes
  /// under it the moment a round starts.
  function planVersionLabel(v) {
    return v && v.current ? "current" : `v${(v && v.iteration) || 1}`;
  }

  /// One-line description of what a version is, for the header line: where it
  /// came from and how big it is. The round note is the useful part — "frozen
  /// at iteration 2" says when, "Tighten the migration step" says why.
  function planVersionCaption(v) {
    if (!v) return "";
    const size = `${v.lines || 0} line${v.lines === 1 ? "" : "s"}`;
    if (v.current) return `the live plan — ${size}, the one the agent revises next`;
    const bits = [`frozen at iteration ${v.iteration}`, size];
    if (v.heading) bits.push(v.heading);
    const why = (v.note || "").trim();
    return bits.join(" · ") + (why ? ` — ${why}` : "");
  }

  /// Which version to compare against by default: the one immediately before
  /// `iteration` in the list. Comparing a version with its predecessor is "what
  /// this round changed", the question the Timeline asks; picking any other
  /// base is the Plan tab's extra.
  ///
  /// Returns an iteration number, or null when there is nothing earlier.
  function planDiffBase(versions, iteration) {
    const list = (versions || []).map((v) => v.iteration);
    const i = list.indexOf(iteration);
    if (i <= 0) return null;
    return list[i - 1];
  }

  /// Can this version be compared at all? Only if something precedes it.
  function planCanCompare(versions, iteration) {
    return planDiffBase(versions, iteration) !== null;
  }

  /// The `to` argument for `get_workflow_plan_diff` given a target version:
  /// null for the head (the backend reads the live file), else its iteration.
  function planDiffTo(versions, iteration) {
    const v = (versions || []).find((x) => x.iteration === iteration);
    return v && v.current ? null : iteration;
  }

  /// The note recorded when a review round is applied as-is.
  ///
  /// `findings` is `roundFindings(agentReviewMd, round)` — already stripped of
  /// the round's records (published / fixed / dismissed), so what is left is
  /// work. When the round left no findings text, the verdict is the note: a
  /// round can legitimately end in "no changes needed", and recording that is
  /// more useful than an empty section.
  function applyReviewNote(round, findings, target) {
    const n = (round && round.round) || 1;
    const what = target === "plan" ? "plan.md" : "the code";
    const head = `Apply agent review round ${n} to ${what}.`;
    const how =
      "Work through the findings below. Where one is wrong, already handled, or " +
      "not worth doing, say so in your summary rather than skipping it silently.";
    const body = (findings && findings.text ? findings.text : "").trim();
    if (body) return `${head}\n\n${how}\n\n${body}\n`;
    const verdict = ((round && round.verdict) || "").trim();
    return verdict
      ? `${head}\n\nThe round reported no separate findings. Verdict: ${verdict}\n`
      : `${head}\n\nSee \`agent-review.md\` for round ${n}.\n`;
  }

  /// Is there a review round that landed, is about *this* stage's artifact, and
  /// has not been handed to the executor yet? `appliedReviewRound` is bumped by
  /// every change round (the executor reads the latest round as input), so the
  /// counter answers "has anything been done with it".
  ///
  /// Two rounds are deliberately not applyable, and both would otherwise offer
  /// a button that makes no sense:
  ///
  /// - a `structure` round is the explainer — it judges nothing, so there are
  ///   no findings to apply;
  /// - a round about the other artifact. A plan round left unapplied when the
  ///   human approved the plan anyway is still the latest round once the item
  ///   reaches diff review, and "apply this plan review to the code" is not a
  ///   thing. Stage and target must agree.
  ///
  /// Returns the round summary, or null.
  function pendingReviewRound(item) {
    const r = item && item.lastAgentReview;
    if (!r || !r.round) return null;
    const meta = (item && item.meta) || {};
    if (meta.status === "reviewing") return null; // still running
    if ((meta.appliedReviewRound || 0) >= r.round) return null;
    // Absent on items reviewed before the block was recorded — those predate
    // structure rounds too, so "no target" means "the stage's own artifact".
    const target = meta.review && meta.review.target;
    if (target === "structure") return null;
    const stagePlan = meta.status === "plan-review";
    if (target === "plan" && !stagePlan) return null;
    if (target === "diff" && stagePlan) return null;
    return r;
  }

  /// What the item header says about the last round: `pending` while it is
  /// waiting to be applied, `applied` once a change round has carried it, and
  /// nothing at all when neither is true — a `structure` round or a round about
  /// the other artifact was never going to be "applied", and claiming either
  /// state for it would be a lie in one direction or the other.
  function reviewAppliedState(item) {
    if (pendingReviewRound(item)) return "pending";
    const r = item && item.lastAgentReview;
    const applied = (item && item.meta && item.meta.appliedReviewRound) || 0;
    if (r && r.round && applied >= r.round) return "applied";
    return "";
  }

  /// May clash turn this finished round into a change round with no further
  /// clicks? Both signals have to agree, and they answer different questions:
  ///
  /// - `meta.review.autoApply` is the human's pre-authorization, given in the
  ///   composer when the round was launched;
  /// - the round's own `**Apply:** yes` is the reviewer's judgement that there
  ///   is something worth applying.
  ///
  /// An undeclared or negative call never fires. A reviewer that found nothing
  /// material must not spend tokens on an executor to apply nothing, and a
  /// round from before the declaration contract existed has said nothing at
  /// all — silence is not consent in either direction.
  ///
  /// `review` is the summary from the hand-back event when there is one (it
  /// arrives before the item list refreshes), else the item's own.
  function shouldAutoApply(item, review) {
    const meta = (item && item.meta) || {};
    if (!(meta.review && meta.review.autoApply)) return false;
    const r = review || (item && item.lastAgentReview);
    if (!r || r.apply !== true) return false;
    // Stage/target agreement and "not already applied" are exactly the
    // pending-round rules, so ask the one function that owns them.
    return !!pendingReviewRound({ ...item, lastAgentReview: r });
  }

  const api = {
    planVersionLabel,
    planVersionCaption,
    planDiffBase,
    planCanCompare,
    planDiffTo,
    applyReviewNote,
    pendingReviewRound,
    reviewAppliedState,
    shouldAutoApply,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
