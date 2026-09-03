// node --test gui/tests/ — plan versions and review application, pure half.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
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
  planVersionForIteration,
  blueprintState,
  blueprintCaption,
} = require("../dist/wf-plan.js");

const NOW = new Date(2026, 8, 2, 12, 0).getTime();
// One version per applied review round, so the numbers and the iterations
// advance together — that is the property the store guarantees.
const versions = [
  {
    n: 1,
    current: false,
    lines: 40,
    savedAt: NOW - 7200_000,
    iteration: 1,
    reason: "the first plan",
  },
  {
    n: 2,
    current: false,
    lines: 52,
    savedAt: NOW - 600_000,
    iteration: 2,
    reason: "after the changes requested at iteration 1",
  },
  {
    n: 3,
    current: true,
    lines: 61,
    savedAt: NOW - 30_000,
    iteration: 3,
    reason: "after the changes requested at iteration 2",
  },
];

test("a revision is labelled by number, and the newest also by role", () => {
  // The number is how a revision is referred to; the role is how the live one
  // is found. The newest needs both.
  assert.equal(planVersionLabel(versions[0]), "v1");
  assert.equal(planVersionLabel(versions[2]), "v3 · current");
  assert.equal(planVersionLabel(null), "v1");
});

test("a revision's caption says when, why and how big", () => {
  assert.equal(
    planVersionCaption(versions[0], NOW),
    "2h ago · 40 lines — the first plan"
  );
  assert.equal(
    planVersionCaption(versions[2], NOW),
    "the live plan · just now · 61 lines — after the changes requested at iteration 2"
  );
  // No stamp and no reason recorded: still says what it is.
  assert.equal(planVersionCaption({ n: 4, lines: 1 }, NOW), "1 line");
});

test("the default comparison is against the previous revision", () => {
  assert.equal(planDiffBase(versions, 3), 2);
  assert.equal(planDiffBase(versions, 2), 1);
  // Nothing precedes the first revision, so it cannot be compared.
  assert.equal(planDiffBase(versions, 1), null);
  assert.equal(planCanCompare(versions, 1), false);
  assert.equal(planCanCompare(versions, 3), true);
  // An unknown revision is not comparable rather than a crash.
  assert.equal(planDiffBase(versions, 99), null);
});

test("the newest revision diffs against the live file", () => {
  // `to: null` tells the backend to read plan.md, so a write that landed since
  // the list was built still shows up.
  assert.equal(planDiffTo(versions, 3), null);
  assert.equal(planDiffTo(versions, 2), 2);
});

test("an iteration maps to its one version", () => {
  // One per round, so this is a lookup rather than a pick among several.
  assert.equal(planVersionForIteration(versions, 1), 1);
  assert.equal(planVersionForIteration(versions, 2), 2);
  assert.equal(planVersionForIteration(versions, 3), 3);
  // An iteration that recorded nothing — a round that asked for changes and
  // got none — falls back to the newest before it.
  assert.equal(planVersionForIteration(versions, 9), 3);
  // Nothing at or before it: no answer rather than a wrong one.
  assert.equal(planVersionForIteration([{ n: 5, iteration: 4 }], 2), null);
  assert.equal(planVersionForIteration([], 1), null);
});

test("the apply note carries the findings, not a pointer to them", () => {
  const note = applyReviewNote(
    { round: 2, verdict: "Two problems" },
    { round: 2, text: "### Findings\n1. Step 3 has no migration" },
    "plan"
  );
  assert.match(note, /^Apply agent review round 2 to plan\.md\./);
  assert.match(note, /Step 3 has no migration/);
  // The instruction that stops a finding from being dropped in silence.
  assert.match(note, /say so in your summary rather than skipping it silently/);
});

test("a findings-free round records its verdict instead of an empty note", () => {
  // "No changes needed" is a real outcome; the note is the item's only record
  // of why the round happened, so it must not be blank.
  const note = applyReviewNote({ round: 3, verdict: "Plan is sound" }, null, "plan");
  assert.match(note, /no separate findings/);
  assert.match(note, /Plan is sound/);
  const bare = applyReviewNote({ round: 4 }, { text: "   " }, "diff");
  assert.match(bare, /Apply agent review round 4 to the code\./);
  assert.match(bare, /agent-review\.md/);
});

test("a round counts as pending until a change round hands it over", () => {
  const item = (over = {}) => ({
    lastAgentReview: { round: 2, target: "plan", verdict: "v" },
    meta: { status: "plan-review", appliedReviewKey: "", ...(over.meta || {}) },
    ...over,
  });
  assert.equal(pendingReviewRound(item()).round, 2);
  // Applied — the executor already read this exact round.
  assert.equal(pendingReviewRound(item({ meta: { appliedReviewKey: "plan:2" } })), null);
  // A later round supersedes an older application.
  assert.equal(
    pendingReviewRound({
      lastAgentReview: { round: 3, target: "plan" },
      meta: { status: "plan-review", appliedReviewKey: "plan:2" },
    }).round,
    3
  );
  // Mid-round there is nothing to apply yet.
  assert.equal(pendingReviewRound(item({ meta: { status: "reviewing" } })), null);
  assert.equal(pendingReviewRound({ meta: {} }), null);
  assert.equal(pendingReviewRound(null), null);
});

test("a restarted number does not read as already applied", () => {
  // The bug per-phase numbering would have introduced with a numeric
  // comparison: three applied plan rounds, then code review 1 lands and
  // 3 >= 1 hides the action on a review nobody has acted on.
  const item = {
    lastAgentReview: { round: 1, target: "diff" },
    meta: { status: "diff-review", appliedReviewKey: "plan:3", review: { target: "diff" } },
  };
  assert.equal(pendingReviewRound(item).round, 1);
  assert.equal(reviewAppliedState(item), "pending");
  // The same number under the same target *is* applied, so the comparison is
  // an identity and not just "different target wins".
  assert.equal(
    reviewAppliedState({
      lastAgentReview: { round: 1, target: "diff" },
      meta: { status: "diff-review", appliedReviewKey: "diff:1", review: { target: "diff" } },
    }),
    "applied"
  );
  // Case in the stored target must not create a second identity.
  assert.equal(
    reviewAppliedState({
      lastAgentReview: { round: 1, target: "Diff" },
      meta: { status: "diff-review", appliedReviewKey: "diff:1", review: { target: "diff" } },
    }),
    "applied"
  );
});

test("the next round's number counts only its own phase", () => {
  const item = { reviewRounds: { plan: 3, diff: 1 } };
  assert.equal(wfNextReviewRound(item, "plan"), 4);
  assert.equal(wfNextReviewRound(item, "diff"), 2);
  // An untouched phase starts at 1 however many rounds the other one had.
  assert.equal(wfNextReviewRound({ reviewRounds: { plan: 6 } }, "diff"), 1);
  assert.equal(wfNextReviewRound({}, "plan"), 1);
  assert.equal(wfNextReviewRound(null, "plan"), 1);
});

test("an explainer round is never offered as work to apply", () => {
  // A structure round writes the Structure tab and judges nothing, so
  // "apply its findings" has no meaning — and it still bumps reviewRound and
  // shows up as the latest round, so it has to be excluded explicitly.
  const explained = {
    lastAgentReview: { round: 4, verdict: "n/a" },
    meta: { status: "diff-review", appliedReviewKey: "", review: { target: "structure" } },
  };
  assert.equal(pendingReviewRound(explained), null);
  // And the header claims neither state for it.
  assert.equal(reviewAppliedState(explained), "");
});

test("a round about the other artifact is not applyable at this stage", () => {
  // Approving a plan over an unapplied plan review leaves that round as the
  // latest one; once the item reaches diff review, "apply this plan review to
  // the code" is not a thing.
  const stale = {
    lastAgentReview: { round: 1 },
    meta: { status: "diff-review", appliedReviewKey: "", review: { target: "plan" } },
  };
  assert.equal(pendingReviewRound(stale), null);
  assert.equal(reviewAppliedState(stale), "");
  // A diff round at plan-review is the mirror case.
  assert.equal(
    pendingReviewRound({
      lastAgentReview: { round: 1 },
      meta: { status: "plan-review", review: { target: "diff" } },
    }),
    null
  );
  // Stage and target agreeing is the applyable case.
  assert.equal(
    pendingReviewRound({
      lastAgentReview: { round: 1 },
      meta: { status: "diff-review", review: { target: "diff" } },
    }).round,
    1
  );
  // An item with no review block predates the field; don't gate on it.
  assert.equal(
    pendingReviewRound({ lastAgentReview: { round: 1 }, meta: { status: "diff-review" } }).round,
    1
  );
});

test("the header says applied only once a change round carried that round", () => {
  const item = (applied) => ({
    lastAgentReview: { round: 2, target: "plan" },
    meta: { status: "plan-review", appliedReviewKey: applied, review: { target: "plan" } },
  });
  assert.equal(reviewAppliedState(item("")), "pending");
  assert.equal(reviewAppliedState(item("plan:2")), "applied");
  // An older round's key is not this round's.
  assert.equal(reviewAppliedState(item("plan:1")), "pending");
  assert.equal(reviewAppliedState({ meta: {} }), "");
});

test("auto-apply needs the human's authorization AND the round's yes", () => {
  const base = (over = {}) => ({
    lastAgentReview: { round: 2, apply: true },
    meta: {
      status: "plan-review",
      appliedReviewKey: "",
      review: { target: "plan", autoApply: true },
      ...(over.meta || {}),
    },
    ...over,
  });
  assert.equal(shouldAutoApply(base()), true);
  // The human did not pre-authorize: the round's yes is a recommendation.
  assert.equal(
    shouldAutoApply(base({ meta: { review: { target: "plan", autoApply: false } } })),
    false
  );
  // Pre-authorized, but the round found nothing worth a round — spending
  // tokens to apply nothing is exactly what the reviewer just advised against.
  assert.equal(shouldAutoApply(base({ lastAgentReview: { round: 2, apply: false } })), false);
  // A round that declared nothing (older report, or a hedge) never fires.
  assert.equal(shouldAutoApply(base({ lastAgentReview: { round: 2 } })), false);
  // Already applied.
  assert.equal(
    shouldAutoApply(
      base({
        meta: {
          appliedReviewKey: "plan:2",
          review: { target: "plan", autoApply: true },
        },
      })
    ),
    false
  );
  // An explainer round is not applyable however it was launched.
  assert.equal(
    shouldAutoApply(base({ meta: { review: { target: "structure", autoApply: true } } })),
    false
  );
  assert.equal(shouldAutoApply(null), false);
});

test("the hand-back's own summary outranks the item's stale one", () => {
  // The attention event arrives before the item list refreshes, so the round
  // passed in is the fresher of the two.
  const item = {
    lastAgentReview: { round: 1, apply: false },
    meta: { status: "plan-review", review: { target: "plan", autoApply: true } },
  };
  assert.equal(shouldAutoApply(item), false);
  assert.equal(shouldAutoApply(item, { round: 2, apply: true }), true);
});

test("a blueprint waits on a decision, and says which one it got", () => {
  const item = (over = {}) => ({
    hasBlueprint: true,
    meta: { status: "plan-review", blueprint: { round: 1, status: "pending" }, ...(over.meta || {}) },
    ...over,
  });
  // Pending is the state that matters: it demotes the stage's own approve,
  // because a blueprint exists to be read *before* the plan is implemented.
  assert.equal(blueprintState(item()), "pending");
  assert.match(blueprintCaption(item()), /waiting on your accept or reject/);

  for (const [status, expected] of [
    ["accepted", /this is the shape to build/],
    ["rejected", /needs another pass/],
    ["stale", /revalidation asked/],
  ]) {
    const it = item({ meta: { blueprint: { round: 2, status } } });
    assert.equal(blueprintState(it), status);
    assert.match(blueprintCaption(it), expected);
    assert.match(blueprintCaption(it), /Blueprint 2/);
  }

  // No document, no state — the tab and its decision only exist once a round
  // has written one.
  assert.equal(blueprintState({ hasBlueprint: false, meta: {} }), "");
  assert.equal(blueprintCaption({ hasBlueprint: false, meta: {} }), "");
  assert.equal(blueprintState(null), "");
  // A document with no block yet (or an unreadable status) is pending, never
  // silently "accepted".
  assert.equal(blueprintState({ hasBlueprint: true, meta: {} }), "pending");
  assert.equal(
    blueprintState({ hasBlueprint: true, meta: { blueprint: { status: "nonsense" } } }),
    "pending"
  );
});

test("the browser branch publishes every name app.js calls", () => {
  const fs = require("node:fs");
  const path = require("node:path");
  const vm = require("node:vm");
  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-plan.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });
  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  for (const name of [
    "planVersionLabel",
    "planVersionCaption",
    "planDiffBase",
    "planCanCompare",
    "planDiffTo",
    "applyReviewNote",
    "pendingReviewRound",
    "reviewAppliedState",
    "wfNextReviewRound",
    "shouldAutoApply",
    "planVersionForIteration",
    "blueprintState",
    "blueprintCaption",
  ]) {
    assert.equal(typeof win[name], "function", `${name} must be on window`);
    assert.ok(app.includes(name), `app.js is expected to use ${name}`);
  }
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(
    html.indexOf("wf-plan.js") < html.indexOf("app.js"),
    "wf-plan.js must be loaded before app.js"
  );
});
