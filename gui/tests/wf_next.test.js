// node --test gui/tests/ — the next-step recommender's pure half.
//
// The recommender decides which button is highlighted on every item, so its
// ordering is the feature: an unapplied review outranks everything, an agent's
// `Next:` only speaks where clash's own checks are silent, and nothing is ever
// recommended that the bar did not render.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const {
  NEXT_ACTIONS,
  AUTO_ACTIONS,
  wfNextStep,
  reviewedThisIteration,
  staleExplanations,
} = require("../dist/wf-next.js");

const ALL = new Set([
  "start-planning", "open-session", "relaunch", "end-round", "launch-round",
  "apply-review", "plan-review", "code-review", "drift", "request-changes",
  "answer-comments", "approve-plan", "approve-pr-draft", "approve-done",
  "create-pr", "pr-is-ready", "mark-ready", "attach-pr", "mark-done", "open-prs",
  "self-review",
]);

const item = (status, extra = {}, meta = {}) => ({
  project: "p",
  slug: "s",
  agentAlive: true,
  openAnnotations: 0,
  reviewRounds: {},
  ...extra,
  meta: { status, iteration: 1, ...meta },
});
const facts = (extra = {}) => ({ available: ALL, pending: null, prs: [], hasPlan: true, ...extra });

test("the vocabulary matches the Rust NEXT_ACTIONS list", () => {
  const rs = fs.readFileSync(path.join(__dirname, "../../src/application/workflow.rs"), "utf8");
  const block = rs.split("pub const NEXT_ACTIONS: &[&str] = &[")[1].split("];")[0];
  const words = [...block.matchAll(/"([a-z-]+)"/g)].map((m) => m[1]);
  assert.deepEqual(words, NEXT_ACTIONS);
});

test("an unapplied round that says apply outranks everything at a decision stage", () => {
  const pending = { round: 2, apply: true, applyReason: "a missing step" };
  for (const st of ["plan-review", "diff-review", "pr-draft", "pr-ready"]) {
    const step = wfNextStep(item(st, { openAnnotations: 4 }), facts({ pending }));
    assert.equal(step.id, "apply-review", st);
    assert.equal(step.auto, true);
    assert.match(step.reason, /a missing step/);
  }
});

test("a round that said not worth applying does not demote the stage", () => {
  const pending = { round: 1, apply: false };
  const it = item("plan-review", { reviewRounds: { plan: 1 } }, { reviewMarks: { plan: { iteration: 1, round: 1 } } });
  assert.equal(wfNextStep(it, facts({ pending })).id, "approve-plan");
});

test("an undeclared apply call still recommends applying, but never automatically", () => {
  const step = wfNextStep(item("diff-review"), facts({ pending: { round: 1, apply: null } }));
  assert.equal(step.id, "apply-review");
  assert.equal(step.auto, false);
});

test("a plan revision asks for another plan review; a reviewed one asks for approval", () => {
  const revised = item("plan-review", { reviewRounds: { plan: 1 } }, {
    iteration: 2,
    reviewMarks: { plan: { iteration: 1, round: 1 } },
  });
  assert.equal(wfNextStep(revised, facts()).id, "plan-review");
  const reviewed = item("plan-review", { reviewRounds: { plan: 2 } }, {
    iteration: 2,
    reviewMarks: { plan: { iteration: 2, round: 2 } },
  });
  const step = wfNextStep(reviewed, facts());
  assert.equal(step.id, "approve-plan");
  assert.ok(step.settled.includes("the plan was reviewed at this iteration"));
});

test("a launched round that has not landed yet does not count as reviewed", () => {
  const it = item("diff-review", { reviewRounds: { diff: 1 } }, {
    reviewMarks: { diff: { iteration: 1, round: 2 } },
  });
  assert.equal(reviewedThisIteration(it, "diff"), false);
});

test("items predating the marks are not nagged for a review they may have had", () => {
  assert.equal(reviewedThisIteration(item("diff-review", { reviewRounds: { diff: 1 } }), "diff"), true);
  assert.equal(reviewedThisIteration(item("diff-review"), "diff"), false);
});

test("diff review walks open comments, PR threads, review, drift, then approval", () => {
  const reviewed = { reviewRounds: { diff: 1, drift: 1 } };
  const marks = { reviewMarks: { diff: { iteration: 1, round: 1 }, drift: { iteration: 1, round: 1 } } };
  assert.equal(wfNextStep(item("diff-review", { openAnnotations: 2 }), facts()).id, "request-changes");
  const prs = [{ primary: true, unanswered: 3 }];
  assert.equal(wfNextStep(item("diff-review", reviewed, marks), facts({ prs })).id, "answer-comments");
  assert.equal(wfNextStep(item("diff-review"), facts()).id, "code-review");
  const codeOnly = item("diff-review", { reviewRounds: { diff: 1 } }, { reviewMarks: { diff: { iteration: 1, round: 1 } } });
  assert.equal(wfNextStep(codeOnly, facts()).id, "drift");
  assert.equal(wfNextStep(codeOnly, facts({ hasPlan: false })).id, "approve-pr-draft");
  assert.equal(wfNextStep(item("diff-review", reviewed, marks), facts()).id, "approve-pr-draft");
});

test("drift is suggested once per item unless an agent asks for it again", () => {
  const it = item("diff-review", {
    reviewRounds: { diff: 2, drift: 1 },
    lastAgentReview: { round: 2, target: "diff", next: "drift", nextReason: "the migration moved" },
  }, {
    iteration: 2,
    reviewMarks: { diff: { iteration: 2, round: 2 }, drift: { iteration: 1, round: 1 } },
  });
  const step = wfNextStep(it, facts());
  assert.equal(step.id, "drift");
  assert.match(step.reason, /review round 2 recommends it: the migration moved/);
  const quiet = { ...it, lastAgentReview: { round: 2, target: "diff" } };
  assert.equal(wfNextStep(quiet, facts()).id, "approve-pr-draft");
});

test("a deep-review hint upgrades the review the recommender starts", () => {
  const it = item("diff-review", { handoff: { next: "deep-review", nextReason: "touches auth" } });
  const step = wfNextStep(it, facts());
  assert.equal(step.id, "code-review");
  assert.equal(step.params.depth, "deep");
  assert.match(step.reason, /hand-off recommends it: touches auth/);
});

test("an agent's Next never outranks clash's own checks", () => {
  const it = item("diff-review", {
    openAnnotations: 1,
    lastAgentReview: { round: 1, target: "diff", next: "approve" },
  });
  assert.equal(wfNextStep(it, facts()).id, "request-changes");
});

test("only rendered buttons are recommended", () => {
  const available = new Set(["approve-done", "request-changes"]);
  const step = wfNextStep(item("diff-review"), facts({ available }));
  assert.equal(step.id, "approve-done");
  assert.equal(wfNextStep(item("done"), facts()), null);
});

test("autopilot may only start actions that decide nothing", () => {
  assert.deepEqual([...AUTO_ACTIONS].sort(), ["apply-review", "code-review", "drift", "plan-review"]);
  const approve = wfNextStep(item("plan-review", { reviewRounds: { plan: 1 } }), facts());
  assert.equal(approve.id, "approve-plan");
  assert.equal(approve.auto, false);
  assert.equal(wfNextStep(item("draft"), facts()).auto, false);
});

test("working and wedged agents get their own recommendation", () => {
  assert.equal(wfNextStep(item("implementing"), facts()).id, "open-session");
  assert.equal(wfNextStep(item("implementing", { agentAlive: false }), facts()).id, "relaunch");
  assert.equal(wfNextStep(item("reviewing", { agentAlive: false }), facts()).id, "end-round");
  assert.equal(wfNextStep(item("changes-requested"), facts()).id, "launch-round");
});

test("pr-draft prefers recording a PR already flipped on GitHub", () => {
  const reviewed = item("pr-draft", { reviewRounds: { diff: 1 } }, { reviewMarks: { diff: { iteration: 1, round: 1 } } });
  assert.equal(wfNextStep(reviewed, facts({ hasPlan: false })).id, "pr-is-ready");
  const noFlip = new Set([...ALL].filter((a) => a !== "pr-is-ready"));
  assert.equal(wfNextStep(reviewed, facts({ hasPlan: false, available: noFlip })).id, "mark-ready");
});

test("stale explanations are the existing ones written for an older iteration", () => {
  const it = item("diff-review", {
    planExplain: { md: true },
    diffExplain: { html: true },
    explainers: [
      { target: "explain-plan", iteration: 2, finished: true },
      { target: "explain-diff", iteration: 1, finished: true },
    ],
  }, { iteration: 2 });
  assert.deepEqual(staleExplanations(it).map((r) => r.target), ["explain-diff"]);
  // No record: written before clash tracked it, so it may describe anything.
  const legacy = item("diff-review", { planExplain: { md: true } });
  assert.deepEqual(staleExplanations(legacy).map((r) => r.target), ["explain-plan"]);
  // A running refresh, a missing explanation, or a gate that says no: nothing.
  const running = item("diff-review", {
    planExplain: { md: true },
    explainers: [{ target: "explain-plan", running: true }],
  });
  assert.deepEqual(staleExplanations(running), []);
  assert.deepEqual(staleExplanations(item("diff-review")), []);
  assert.deepEqual(staleExplanations(legacy, () => false), []);
});

test("the browser branch publishes every global app.js reads", () => {
  const src = fs.readFileSync(path.join(__dirname, "../dist/wf-next.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });
  for (const name of ["wfNextStep", "staleExplanations", "reviewedThisIteration", "AUTO_ACTIONS"])
    assert.ok(win[name], name);
});

test("a PR in human hands is offered a self-review once per iteration, never on autopilot", () => {
  const reviewed = {
    reviewRounds: { diff: 1, drift: 1 },
  };
  const marks = { reviewMarks: { diff: { iteration: 1, round: 1 }, drift: { iteration: 1, round: 1 } } };
  const step = wfNextStep(item("pr-ready", reviewed, marks), facts());
  assert.equal(step.id, "self-review");
  // It posts on GitHub, so it waits for a click.
  assert.equal(step.auto, false);
  // Once this iteration has a verdict, the PR is just waiting on merge.
  const done = item(
    "pr-ready",
    { reviewRounds: { diff: 1, drift: 1, "self-review": 1 } },
    { reviewMarks: { ...marks.reviewMarks, "self-review": { iteration: 1, round: 1 } } }
  );
  assert.equal(wfNextStep(done, facts()).id, "open-prs");
  assert.ok(!AUTO_ACTIONS.has("self-review"));
});
