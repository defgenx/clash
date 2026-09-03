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

  /// Chip label for a revision: `v3`, and `v3 · current` for the newest — the
  /// newest revision *is* the live plan, so it needs both its number (to be
  /// referred to) and its role (to be found).
  function planVersionLabel(v) {
    if (!v) return "v1";
    return v.current ? `v${v.n} · current` : `v${v.n}`;
  }

  /// One-line description of a revision for the header: when clash recorded
  /// it, why, and how big it is. The reason is the useful part — "v2" says
  /// nothing, "revision requested at iteration 1" says everything.
  function planVersionCaption(v, now) {
    if (!v) return "";
    const size = `${v.lines || 0} line${v.lines === 1 ? "" : "s"}`;
    const bits = [];
    if (v.current) bits.push("the live plan");
    const when = planWhen(v.savedAt, now);
    if (when) bits.push(when);
    bits.push(size);
    const why = (v.reason || "").trim();
    return bits.join(" · ") + (why ? ` — ${why}` : "");
  }

  /// Compact "how long ago" for a revision stamp. Its own function because the
  /// captions are asserted in tests and "just now" must not drift with the
  /// clock; `now` is injectable for exactly that reason.
  function planWhen(savedAt, now) {
    if (!savedAt) return "";
    const s = Math.max(0, ((now || Date.now()) - savedAt) / 1000);
    if (s < 90) return "just now";
    if (s < 5400) return `${Math.round(s / 60)}m ago`;
    if (s < 129600) return `${Math.round(s / 3600)}h ago`;
    return `${Math.round(s / 86400)}d ago`;
  }

  /// Which revision to compare against by default: the one immediately before
  /// `n`. A revision against its predecessor is "what this change did", which
  /// is the question asked every round; any other base is the extra.
  ///
  /// Returns a revision number, or null when there is nothing earlier.
  function planDiffBase(versions, n) {
    const list = (versions || []).map((v) => v.n);
    const i = list.indexOf(n);
    if (i <= 0) return null;
    return list[i - 1];
  }

  /// Can this revision be compared at all? Only if something precedes it.
  function planCanCompare(versions, n) {
    return planDiffBase(versions, n) !== null;
  }

  /// The `to` argument for `get_workflow_plan_diff`: null for the newest
  /// revision, so the backend diffs against the live file itself and a write
  /// that landed since the list was built still shows up.
  function planDiffTo(versions, n) {
    const v = (versions || []).find((x) => x.n === n);
    return v && v.current ? null : n;
  }

  /// The revision to open when the Timeline asks for "the plan at iteration
  /// N". There is at most one per iteration — the store replaces a version in
  /// place while its round is still being written — so this is a lookup, with
  /// a fallback to the newest revision *before* N for an iteration that
  /// changed nothing (a round asked for changes and none landed).
  ///
  /// Null when nothing was recorded at or before N.
  function planVersionForIteration(versions, iteration) {
    const list = (versions || []).filter((v) => v.iteration === iteration);
    if (list.length) return list[list.length - 1].n;
    const earlier = (versions || []).filter((v) => v.iteration < iteration);
    return earlier.length ? earlier[earlier.length - 1].n : null;
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
  /// has not been handed to the executor yet? `appliedReviewKey` is stamped by
  /// every change round (the executor reads the latest round as input), so
  /// comparing it against this round's identity answers "has anything been
  /// done with it". Absent — an older item, or nothing applied — reads as not
  /// applied: offering the action once more is recoverable, hiding it is not.
  ///
  /// Two rounds are deliberately not applyable, and both would otherwise offer
  /// a button that makes no sense:
  ///
  /// - an **explainer** round judges nothing, so there are no findings to
  ///   apply. Both explainer targets: this checked `structure` alone, so a
  ///   plan explanation counted as a pending review — it demoted the stage's
  ///   own Approve and offered "apply the findings" for a round that produced
  ///   none;
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
    // Identity, not order: round numbers restart per target, so "3 plan rounds
    // applied" must not read as "code round 1 applied".
    if (meta.appliedReviewKey && roundKeyMatches(meta.appliedReviewKey, r)) return null;
    // Absent on items reviewed before the block was recorded — those predate
    // explainer rounds too, so "no target" means "the stage's own artifact".
    const target = meta.review && meta.review.target;
    if (isExplainTarget(target)) return null;
    const stagePlan = meta.status === "plan-review";
    if (target === "plan" && !stagePlan) return null;
    if (target === "diff" && stagePlan) return null;
    return r;
  }

  /// Mirrors `ReviewTarget::explains()` — is this round an explanation rather
  /// than a judgement? Accepts the pre-rename spellings, since a round
  /// recorded as `structure`/`blueprint` explains exactly as much as one
  /// recorded under the current names.
  function isExplainTarget(target) {
    return ["explain-diff", "explain-plan", "structure", "blueprint"].includes(
      String(target || "").toLowerCase()
    );
  }

  /// What the item header says about the last round: `pending` while it is
  /// waiting to be applied, `applied` once a change round has carried it, and
  /// nothing at all when neither is true — an explainer round or a round about
  /// the other artifact was never going to be "applied", and claiming either
  /// state for it would be a lie in one direction or the other.
  function reviewAppliedState(item) {
    if (pendingReviewRound(item)) return "pending";
    const r = item && item.lastAgentReview;
    const applied = (item && item.meta && item.meta.appliedReviewKey) || "";
    if (r && r.round && applied && roundKeyMatches(applied, r)) return "applied";
    return "";
  }

  /// A round's identity: `<target>:<round>`. Mirrors
  /// `application::workflow::review_round_key` — the number restarts per
  /// target, so neither half identifies a round on its own.
  function reviewRoundKey(r) {
    return `${canonicalTarget((r && r.target) || "")}:${(r && r.round) || 0}`;
  }

  /// Mirrors `ReviewTarget::canonical`: the explainer targets were renamed,
  /// and both spellings name the same target.
  function canonicalTarget(raw) {
    const t = String(raw || "").trim().toLowerCase();
    if (t === "structure") return "explain-diff";
    if (t === "blueprint") return "explain-plan";
    return t;
  }

  /// Does a stored `appliedReviewKey` name this round?
  ///
  /// Both sides are normalized, not just the round's: a key stamped as
  /// `blueprint:2` before the rename would otherwise never match the round it
  /// was stamped for, so the header would say "not applied yet" forever and
  /// the stage's own approve would stay demoted.
  function roundKeyMatches(stored, r) {
    const key = String(stored || "");
    const cut = key.lastIndexOf(":");
    if (cut < 0) return false;
    const normalized = `${canonicalTarget(key.slice(0, cut))}:${key.slice(cut + 1)}`;
    return normalized === reviewRoundKey(r);
  }

  /// The number the next round against `target` will carry — the count of
  /// rounds that target already has, plus one. Read from the item's own tally
  /// (`review_rounds`, parsed from agent-review.md at list time) rather than
  /// from a global counter, which is what made the first code review of a
  /// well-planned item read as "round 7".
  function wfNextReviewRound(item, target) {
    const tally = (item && item.reviewRounds) || {};
    return (tally[target] || 0) + 1;
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
    isExplainTarget,
    canonicalTarget,
    planVersionForIteration,
    planVersionLabel,
    planVersionCaption,
    planDiffBase,
    planCanCompare,
    planDiffTo,
    applyReviewNote,
    pendingReviewRound,
    reviewAppliedState,
    wfNextReviewRound,
    shouldAutoApply,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
