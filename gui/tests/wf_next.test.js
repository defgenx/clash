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
  REVIEW_LOOP_LIMIT,
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
  "self-review", "approve-waves", "start-waves",
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

test("a red build and a stale base come after PR threads, as change requests with a note", () => {
  const reviewed = { reviewRounds: { diff: 1, drift: 1 } };
  const marks = { reviewMarks: { diff: { iteration: 1, round: 1 }, drift: { iteration: 1, round: 1 } } };
  const red = {
    url: "https://github.com/acme/api/pull/7",
    number: 7,
    primary: true,
    state: "OPEN",
    headRef: "feat/x",
    baseRef: "main",
    health: { checks: "fail", failing: ["lint", "unit"], mergeable: "conflicts" },
  };
  for (const st of ["diff-review", "pr-draft", "pr-ready"]) {
    // Threads first: answering them is cheaper than a fix round.
    const threads = wfNextStep(item(st, reviewed, marks), facts({ prs: [{ ...red, unanswered: 2 }] }));
    assert.equal(threads.id, "answer-comments", st);

    const ci = wfNextStep(item(st, reviewed, marks), facts({ prs: [red] }));
    assert.equal(ci.kind, "action", st);
    assert.equal(ci.id, "request-changes");
    assert.equal(ci.reason, "CI is failing on api#7 — lint, unit");
    assert.equal(ci.params.prefill, "CI is failing on api#7: lint, unit. Fix the build.");
    // Never started without a click: the note is a change request.
    assert.equal(ci.auto, false);
    assert.ok(!AUTO_ACTIONS.has("request-changes"));

    const green = { ...red, health: { checks: "pass", mergeable: "conflicts" } };
    const rebase = wfNextStep(item(st, reviewed, marks), facts({ prs: [green] }));
    assert.equal(rebase.id, "request-changes");
    assert.equal(rebase.reason, "api#7 conflicts with its base");
    assert.equal(rebase.params.prefill, "Rebase feat/x onto main and resolve the conflicts.");
    assert.ok(rebase.settled.includes("CI is not failing"));
  }
  const behind = { ...red, health: { checks: "pass", mergeable: "yes", behind: true } };
  const step = wfNextStep(item("pr-ready", reviewed, marks), facts({ prs: [behind] }));
  assert.equal(step.reason, "api#7 is behind its base");
  assert.equal(step.params.prefill, "Rebase feat/x onto main — api#7 is behind it.");
});

test("merged PRs and PRs with no recorded health raise no CI check, and say nothing", () => {
  const reviewed = { reviewRounds: { diff: 1, drift: 1 } };
  const marks = { reviewMarks: { diff: { iteration: 1, round: 1 }, drift: { iteration: 1, round: 1 } } };
  const merged = { url: "u", state: "MERGED", health: { checks: "fail", failing: ["x"] } };
  const step = wfNextStep(item("diff-review", reviewed, marks), facts({ prs: [merged, { url: "v" }] }));
  assert.equal(step.id, "approve-pr-draft");
  assert.ok(!step.settled.some((t) => /CI|base/.test(t)));
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

test("a stopped agent is resumed, not relaunched", () => {
  const stopped = { agentAlive: false, agentResumable: true };
  assert.equal(wfNextStep(item("implementing", stopped), facts()).id, "open-session");
  assert.equal(wfNextStep(item("planning", stopped), facts()).id, "open-session");
  assert.equal(wfNextStep(item("reviewing", stopped), facts()).id, "open-session");
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

test("a verdict follows this iteration's code review instead of racing it", () => {
  // Nothing reviewed yet: the code review comes first, and alone — a verdict
  // is how that review ends on the PR, not a second reviewer next to it.
  const passes = new Set(["review", "drift", "respond", "self-review"]);
  const fresh = wfNextStep(item("pr-ready", { reviewRounds: { drift: 1 } }, {
    reviewMarks: { drift: { iteration: 1, round: 1 } },
  }), facts({ passes }));
  assert.equal(fresh.id, "code-review");
  assert.ok(!fresh.passes.some((p) => p.id === "self-review"));
  // A verdict round at this iteration counts as the code review.
  const verdicted = item(
    "diff-review",
    { reviewRounds: { drift: 1, "self-review": 1 } },
    { reviewMarks: { drift: { iteration: 1, round: 1 }, "self-review": { iteration: 1, round: 1 } } }
  );
  assert.notEqual(wfNextStep(verdicted, facts()).id, "code-review");
});

test("a multi-wave item approves its waves before the item, then starts the next", () => {
  const board = (states) => ({
    waves: {
      waves: states.map((state, i) => ({
        id: `w${i + 1}`, title: "", summary: "", dependsOn: [], areas: [`w${i + 1}`],
        state, blockedBy: [], level: i, compatibleWith: [],
      })),
    },
  });
  const reviewed = {
    reviewRounds: { diff: 1, drift: 1 },
  };
  const marks = { reviewMarks: { diff: { iteration: 1, round: 1 }, drift: { iteration: 1, round: 1 } } };
  const inReview = item("diff-review", { ...reviewed, ...board(["done", "review", "blocked"]) }, marks);
  assert.equal(wfNextStep(inReview, facts()).id, "approve-waves");
  const next = item("diff-review", { ...reviewed, ...board(["done", "done", "ready"]) }, marks);
  assert.equal(wfNextStep(next, facts()).id, "start-waves");
  // The last waves are signed off too, at every code stage: the item never
  // closes itself while one is unsigned.
  for (const st of ["diff-review", "pr-draft", "pr-ready"]) {
    const last = item(st, { ...reviewed, ...board(["done", "done", "review"]) }, marks);
    const step = wfNextStep(last, facts());
    assert.equal(step.id, "approve-waves", st);
    assert.match(step.reason, /Wave 3 reviewed — the last wave: sign it off/);
  }
  // A PR waiting on merge says the waves still need signing off.
  const verdict = { reviewRounds: { diff: 1, drift: 1, "self-review": 1 } };
  const vmarks = { reviewMarks: { ...marks.reviewMarks, "self-review": { iteration: 1, round: 1 } } };
  const waiting = item("pr-ready", { ...verdict, ...board(["done", "done", "review"]) }, vmarks);
  const noSignOff = new Set([...ALL].filter((a) => a !== "approve-waves"));
  const open = wfNextStep(waiting, facts({ available: noSignOff }));
  assert.equal(open.id, "open-prs");
  assert.doesNotMatch(open.reason, /closes itself/);
  assert.ok(!open.settled.includes("no wave waiting for sign-off"));
  const plain = wfNextStep(item("pr-ready", verdict, vmarks), facts());
  assert.match(plain.reason, /closes itself when the PR merges/);
  // An unreadable waves.json: nothing approves, a discussion repairs it; at
  // plan review a change request does.
  const broken = { reviewRounds: reviewed.reviewRounds, waves: { waves: [], issue: "a cycle" } };
  assert.equal(wfNextStep(item("pr-ready", broken, marks), facts({ available: new Set([...ALL, "discuss"]) })).id, "discuss");
  assert.equal(wfNextStep(item("plan-review", broken), facts()).id, "request-changes");
  // Never on autopilot: approving and starting waves is the human's call.
  assert.ok(!AUTO_ACTIONS.has("approve-waves") && !AUTO_ACTIONS.has("start-waves"));
});

test("review and fix cannot circle forever: a non-converging target hands the call back", () => {
  // Three code reviews in a row each asked for changes and were applied; the
  // code changed again (iteration 4), so "review the fix round" would fire
  // forever. At the cap the recommender stops proposing another round.
  assert.equal(REVIEW_LOOP_LIMIT, 3);
  const looping = item(
    "diff-review",
    { reviewRounds: { diff: 3, drift: 1 }, reviewStreaks: { diff: 3 } },
    { iteration: 4, reviewMarks: { diff: { iteration: 3, round: 3 }, drift: { iteration: 1, round: 1 } } }
  );
  const step = wfNextStep(looping, facts());
  assert.notEqual(step.kind, "check", "no further review is proposed");
  assert.equal(step.id, "approve-pr-draft");
  assert.match(step.reason, /3 code review rounds in a row each asked for changes/);
  assert.ok(step.settled.includes("code review paused: 3 rounds in a row asked for changes"));
  assert.equal(step.auto, false, "autopilot has nothing to start");
  // Below the cap, the fix round is reviewed as before.
  const two = { ...looping, reviewStreaks: { diff: 2 } };
  assert.equal(wfNextStep(two, facts({ passes: new Set(["review", "drift"]) })).kind, "check");
  // A round that judged its findings not worth applying ends the streak
  // (the backend resets it), so a converging review never hits the cap.
  const converged = { ...looping, reviewStreaks: {} };
  assert.equal(wfNextStep(converged, facts()).id, "code-review");
});

test("the round that reaches the cap is applied only by a click", () => {
  const pending = { round: 3, target: "diff", apply: true, applyReason: "x" };
  const it = item("diff-review", { reviewStreaks: { diff: 3 } });
  const step = wfNextStep(it, facts({ pending }));
  assert.equal(step.id, "apply-review");
  assert.equal(step.auto, false);
  assert.match(step.reason, /the 3rd round in a row to ask for changes: read it before applying/);
  // Under the cap it still applies itself under autopilot.
  assert.equal(wfNextStep(item("diff-review", { reviewStreaks: { diff: 2 } }), facts({ pending })).auto, true);
});

test("plan reviews, verdicts and drift stop at the cap too", () => {
  const plan = item("plan-review", { reviewRounds: { plan: 3 }, reviewStreaks: { plan: 3 } }, {
    iteration: 4,
    reviewMarks: { plan: { iteration: 3, round: 3 } },
  });
  const p = wfNextStep(plan, facts());
  assert.equal(p.id, "approve-plan");
  assert.match(p.reason, /3 plan review rounds in a row/);
  // Verdict rounds count as code reviews.
  const verdicts = item("pr-ready", { reviewRounds: { diff: 1, "self-review": 3, drift: 1 }, reviewStreaks: { "self-review": 3 } }, {
    iteration: 5,
    reviewMarks: { diff: { iteration: 1, round: 1 }, drift: { iteration: 1, round: 1 } },
  });
  assert.equal(wfNextStep(verdicts, facts()).id, "open-prs");
  // An agent asking for drift again cannot restart a drift loop.
  const drift = item(
    "diff-review",
    { reviewRounds: { diff: 1, drift: 3 }, reviewStreaks: { drift: 3 }, lastAgentReview: { target: "diff", round: 1, next: "drift" } },
    { iteration: 2, reviewMarks: { diff: { iteration: 2, round: 1 }, drift: { iteration: 1, round: 3 } } }
  );
  assert.equal(wfNextStep(drift, facts()).id, "approve-pr-draft");
});

test("a working agent's suggestion follows what it is doing right now", () => {
  const it = item("implementing");
  assert.match(wfNextStep(it, facts()).reason, /An agent is working/);
  assert.match(wfNextStep(it, facts({ agentStatus: "Prompting" })).reason, /waiting for you — a question or a permission/);
  assert.match(wfNextStep(it, facts({ agentStatus: "Waiting" })).reason, /finished its turn and is at its prompt/);
  assert.match(wfNextStep(item("reviewing"), facts({ agentStatus: "Prompting" })).reason, /The reviewer is waiting for you/);
});

test("a waved item compares each wave with the plan, then the whole project", () => {
  const board = (states) => ({
    waves: {
      waves: states.map((state, i) => ({
        id: `w${i + 1}`, title: "", summary: "", dependsOn: [], areas: [`w${i + 1}`],
        state, blockedBy: [], level: i, compatibleWith: [],
      })),
    },
  });
  const marks = { diff: { iteration: 1, round: 1 }, drift: { iteration: 1, round: 1 } };
  const rounds = { reviewRounds: { diff: 1, drift: 1 } };
  // Wave 2 is in review; the last comparison covered wave 1 only — due, even
  // though a drift round already ran at this iteration.
  const w2 = item(
    "diff-review",
    { ...rounds, ...board(["done", "review", "blocked"]), docScopes: { drift: { waves: ["w1"], startedAt: 1 } } },
    { reviewMarks: { diff: marks.diff }, waves: { done: ["w1"], scope: ["w2"] } }
  );
  const step = wfNextStep(w2, facts());
  assert.equal(step.id, "drift", JSON.stringify(step));
  assert.match(step.reason, /Wave 2 has not been compared with its part of the plan/);
  // Compared for wave 2: sign-off is next.
  const compared = item(
    "diff-review",
    { ...rounds, ...board(["done", "review", "blocked"]), docScopes: { drift: { waves: ["w2"], startedAt: 1 } } },
    { reviewMarks: marks, waves: { done: ["w1"], scope: ["w2"] } }
  );
  assert.equal(wfNextStep(compared, facts()).id, "approve-waves");
  // Every wave signed off, no whole-project comparison yet: that comes first.
  const all = item(
    "diff-review",
    { ...rounds, ...board(["done", "done", "done"]), docScopes: { drift: { waves: ["w3"], startedAt: 1 } } },
    { reviewMarks: marks, waves: { done: ["w1", "w2", "w3"], scope: [] } }
  );
  const fin = wfNextStep(all, facts());
  assert.equal(fin.id, "drift", JSON.stringify(fin));
  assert.match(fin.reason, /Every wave is signed off — compare the whole project/);
  // Done once: the whole-project comparison is not proposed again.
  const after = item(
    "diff-review",
    { ...rounds, ...board(["done", "done", "done"]), docScopes: { drift: { whole: true, startedAt: 1 } } },
    { reviewMarks: marks, waves: { done: ["w1", "w2", "w3"], scope: [] } }
  );
  const later = wfNextStep(after, facts());
  assert.notEqual(later.id, "drift", JSON.stringify(later));
});
