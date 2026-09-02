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
  shouldAutoApply,
} = require("../dist/wf-plan.js");

const versions = [
  { iteration: 1, current: false, lines: 40, heading: "2026-09-01 10:00", note: "First pass" },
  { iteration: 2, current: false, lines: 52, heading: "2026-09-01 11:00", note: "Apply review r1" },
  { iteration: 3, current: true, lines: 61, heading: "", note: "" },
];

test("the head is labelled by role, older versions by number", () => {
  // Its iteration changes under it the moment a round starts, so a number
  // would go stale in the UI while the file did not change.
  assert.equal(planVersionLabel(versions[0]), "v1");
  assert.equal(planVersionLabel(versions[2]), "current");
  assert.equal(planVersionLabel(null), "v1");
});

test("a version's caption says where it came from and why", () => {
  assert.equal(
    planVersionCaption(versions[0]),
    "frozen at iteration 1 · 40 lines · 2026-09-01 10:00 — First pass"
  );
  assert.match(planVersionCaption(versions[2]), /the live plan — 61 lines/);
  // A version with no recorded note still says what it is.
  assert.equal(
    planVersionCaption({ iteration: 4, lines: 1, heading: "", note: "" }),
    "frozen at iteration 4 · 1 line"
  );
});

test("the default comparison is against the previous version", () => {
  assert.equal(planDiffBase(versions, 3), 2);
  assert.equal(planDiffBase(versions, 2), 1);
  // Nothing precedes the first version, so it cannot be compared.
  assert.equal(planDiffBase(versions, 1), null);
  assert.equal(planCanCompare(versions, 1), false);
  assert.equal(planCanCompare(versions, 3), true);
  // An unknown iteration is not comparable rather than a crash.
  assert.equal(planDiffBase(versions, 99), null);
});

test("the head diffs against the live file, not a snapshot number", () => {
  // `to: null` is what tells the backend to read plan.md; passing the head's
  // iteration would look for a frozen copy that does not exist yet.
  assert.equal(planDiffTo(versions, 3), null);
  assert.equal(planDiffTo(versions, 2), 2);
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
    lastAgentReview: { round: 2, verdict: "v" },
    meta: { status: "plan-review", appliedReviewRound: 0, ...(over.meta || {}) },
    ...over,
  });
  assert.equal(pendingReviewRound(item()).round, 2);
  // Applied — the executor already read it.
  assert.equal(pendingReviewRound(item({ meta: { appliedReviewRound: 2 } })), null);
  // A later round supersedes an older application.
  assert.equal(
    pendingReviewRound({
      lastAgentReview: { round: 3 },
      meta: { status: "plan-review", appliedReviewRound: 2 },
    }).round,
    3
  );
  // Mid-round there is nothing to apply yet.
  assert.equal(pendingReviewRound(item({ meta: { status: "reviewing" } })), null);
  assert.equal(pendingReviewRound({ meta: {} }), null);
  assert.equal(pendingReviewRound(null), null);
});

test("an explainer round is never offered as work to apply", () => {
  // A structure round writes the Structure tab and judges nothing, so
  // "apply its findings" has no meaning — and it still bumps reviewRound and
  // shows up as the latest round, so it has to be excluded explicitly.
  const explained = {
    lastAgentReview: { round: 4, verdict: "n/a" },
    meta: { status: "diff-review", appliedReviewRound: 3, review: { target: "structure" } },
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
    meta: { status: "diff-review", appliedReviewRound: 0, review: { target: "plan" } },
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

test("the header says applied only once a change round carried the round", () => {
  const item = (applied) => ({
    lastAgentReview: { round: 2 },
    meta: { status: "plan-review", appliedReviewRound: applied, review: { target: "plan" } },
  });
  assert.equal(reviewAppliedState(item(0)), "pending");
  assert.equal(reviewAppliedState(item(2)), "applied");
  assert.equal(reviewAppliedState({ meta: {} }), "");
});

test("auto-apply needs the human's authorization AND the round's yes", () => {
  const base = (over = {}) => ({
    lastAgentReview: { round: 2, apply: true },
    meta: {
      status: "plan-review",
      appliedReviewRound: 0,
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
          appliedReviewRound: 2,
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
    "shouldAutoApply",
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
