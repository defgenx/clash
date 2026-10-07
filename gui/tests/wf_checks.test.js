// node --test gui/tests/ — check runs, the pure half.
//
// A check run is any subset of the agent passes that judge or explain an item
// without moving it. What is pinned here is what the backend cannot see: which
// passes a stage offers, which start ticked (never one that posts), the spec
// each sends — whose target must be the one the launcher derives, or the run
// waits for a round that never comes — and the note a combined apply sends.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const {
  PASSES,
  availablePasses,
  passSpec,
  passIdOf,
  passLabel,
  recommendedSelection,
  autopilotSelection,
  runSummary,
  combinedApplyNote,
} = require("../dist/wf-checks.js");
const { wfNextStep } = require("../dist/wf-next.js");

const ctx = (extra = {}) => ({
  status: "diff-review",
  canReview: true,
  reviewTarget: "diff",
  hasPlan: true,
  canExplainPlan: true,
  canExplainDiff: true,
  hasPrs: true,
  canSelfReview: true,
  ...extra,
});

test("the catalogue is in the backend's launch order", () => {
  const rs = fs.readFileSync(
    path.join(__dirname, "../../src/application/workflow_run.rs"),
    "utf8"
  );
  const rank = (needle) => {
    const m = new RegExp(`${needle}\\s*=>\\s*(\\d+)`).exec(rs);
    assert.ok(m, `no rank for ${needle}`);
    return Number(m[1]);
  };
  const ranks = {
    "explain-plan": rank("ReviewTarget::ExplainPlan"),
    "explain-diff": rank("ReviewTarget::ExplainDiff"),
    review: rank("ReviewTarget::Diff"),
    drift: rank("ReviewTarget::Drift"),
    respond: rank("_ if publish == ReviewPublish::RespondPrComments"),
    "self-review": rank("ReviewTarget::SelfReview"),
  };
  const ids = PASSES.map((p) => p.id);
  const sorted = [...ids].sort((a, b) => ranks[a] - ranks[b]);
  assert.deepEqual(ids, sorted);
});

test("only passes with something to read are offered", () => {
  assert.deepEqual(availablePasses(ctx()), [
    "explain-plan",
    "explain-diff",
    "review",
    "drift",
    "respond",
    "self-review",
  ]);
  // Nothing is built at plan-review: no comparison to make.
  assert.ok(!availablePasses(ctx({ status: "plan-review" })).includes("drift"));
  // Review-only items have no plan.
  assert.ok(!availablePasses(ctx({ hasPlan: false })).includes("drift"));
  assert.ok(!availablePasses(ctx({ hasPrs: false })).includes("respond"));
  // A running agent owns the item: only the explainers its gate allows.
  assert.deepEqual(availablePasses(ctx({ canReview: false, canSelfReview: false })), [
    "explain-plan",
    "explain-diff",
  ]);
});

test("the stage's review names the target the launcher derives", () => {
  assert.equal(passSpec("review", {}, ctx()).target, "diff");
  assert.equal(passSpec("review", {}, ctx({ reviewTarget: "plan" })).target, "plan");
  // Answering comments is a mode of the stage's review, at plan-review too.
  const respond = passSpec("respond", { prUrls: ["u"] }, ctx({ reviewTarget: "plan" }));
  assert.deepEqual(respond, {
    target: "plan",
    depth: "standard",
    publish: "respond-pr-comments",
    prUrls: ["u"],
    focus: "",
  });
  assert.equal(passSpec("self-review", {}, ctx()).publish, "pr-comments");
  assert.equal(passSpec("explain-diff", { focus: " the cache " }, ctx()).depth, "deep");
  assert.equal(passSpec("explain-diff", {}, ctx()).depth, "standard");
});

test("a recorded pass maps back to its catalogue id", () => {
  for (const id of PASSES.map((p) => p.id)) {
    const spec = passSpec(id, {}, ctx());
    assert.equal(passIdOf(spec), id);
  }
  const atPlan = passSpec("respond", {}, ctx({ reviewTarget: "plan" }));
  assert.equal(passIdOf(atPlan), "respond");
});

test("labels follow the stage and the thread count", () => {
  assert.equal(passLabel("review", { reviewTarget: "plan" }), "Plan review");
  assert.equal(passLabel("respond", { unanswered: 1 }), "Answer 1 open PR thread");
  assert.equal(passLabel("respond", { unanswered: 0 }), "Answer PR comments · all answered");
  // Never fetched is not zero.
  assert.equal(passLabel("respond", { unanswered: null }), "Answer PR comments");
});

const item = (status, extra = {}, meta = {}) => ({
  project: "p",
  slug: "s",
  agentAlive: true,
  openAnnotations: 0,
  reviewRounds: {},
  ...extra,
  meta: { status, iteration: 1, ...meta },
});
const ALL_PASSES = new Set(PASSES.map((p) => p.id));
const facts = (extra = {}) => ({
  available: new Set(["approve-done", "request-changes", "apply-review", "open-prs"]),
  passes: ALL_PASSES,
  pending: null,
  prs: [],
  hasPlan: true,
  ...extra,
});

test("every due check is recommended together, not just the first", () => {
  const step = wfNextStep(item("diff-review"), facts());
  assert.equal(step.kind, "check");
  assert.deepEqual(
    step.passes.map((p) => p.id),
    ["review", "drift"]
  );
  assert.ok(step.passes.every((p) => p.auto));
});

test("a change outranks every check", () => {
  const step = wfNextStep(item("diff-review", { openAnnotations: 2 }), facts());
  assert.equal(step.kind, "action");
  assert.equal(step.id, "request-changes");
});

test("unanswered threads join the set, unticked because answering posts", () => {
  const prs = [{ url: "u", primary: true, unanswered: 2 }];
  const step = wfNextStep(item("diff-review"), facts({ prs }));
  assert.deepEqual(
    step.passes.map((p) => p.id),
    ["respond", "review", "drift"]
  );
  const sel = recommendedSelection(step);
  assert.deepEqual(
    sel.map((p) => [p.id, p.ticked]),
    [
      ["review", true],
      ["drift", true],
      ["respond", false],
    ]
  );
  assert.deepEqual(
    autopilotSelection(sel).map((p) => p.id),
    ["review", "drift"]
  );
});

test("a stale explanation rides along with the comparison", () => {
  const step = wfNextStep(item("diff-review"), facts());
  const sel = recommendedSelection(step, [{ target: "explain-diff", label: "changes explanation" }]);
  assert.deepEqual(
    sel.map((p) => p.id),
    ["explain-diff", "review", "drift"]
  );
  assert.ok(sel[0].ticked);
});

test("once everything is checked the recommendation is the stage's own action", () => {
  const done = item(
    "diff-review",
    { reviewRounds: { diff: 1, drift: 1 } },
    { reviewMarks: { diff: { iteration: 1, round: 1 }, drift: { iteration: 1, round: 1 } } }
  );
  const step = wfNextStep(done, facts());
  assert.equal(step.kind, "action");
  assert.equal(step.id, "approve-done");
});

test("without the pass set checks are recommended as buttons, as before", () => {
  const step = wfNextStep(
    item("diff-review"),
    facts({ passes: undefined, available: new Set(["code-review"]) })
  );
  assert.equal(step.kind, "action");
  assert.equal(step.id, "code-review");
});

const batch = (passes, extra = {}) => ({
  id: 1,
  passes,
  apply: { state: "none" },
  ...extra,
});

test("the strip line says what runs, what is next and what is alongside", () => {
  const s = runSummary(
    batch([
      { target: "explain-diff", state: "running" },
      { target: "diff", state: "done", round: 2 },
      { target: "drift", state: "running" },
      { target: "self-review", publish: "pr-comments", state: "queued" },
    ])
  );
  assert.equal(
    s.text,
    "Checks · 1 of 3 done · plan vs changes running · next: self-review · explain changes alongside"
  );
  assert.ok(s.canStop);
  assert.equal(s.next.index, 3);
});

test("a paused run names its failure and an unapproved apply its count", () => {
  const s = runSummary(
    batch([{ target: "diff", state: "failed", error: "no-pr: none" }], { paused: true })
  );
  assert.match(s.text, /paused/);
  assert.deepEqual(
    s.failed.map((f) => [f.index, f.error]),
    [[0, "no-pr: none"]]
  );
  const w = runSummary(
    batch([{ target: "diff", state: "done" }], {
      apply: { state: "pending", auto: false, keys: ["diff:1", "drift:1"] },
    })
  );
  assert.match(w.text, /2 rounds to apply/);
  assert.ok(!w.canStop);
});

const MD = [
  "# Agent review",
  "",
  "## Review 1 — diff · standard · 2026-10-07 10:00",
  "**Verdict:** two issues",
  "",
  "### Findings",
  "- the cache never expires",
  "",
  "### Published",
  "- nothing",
  "",
  "## Review 1 — drift · 2026-10-07 10:30",
  "**Verdict:** one gap",
  "",
  "### Issues",
  "- the migration test promised in step 4 is missing",
  "",
].join("\n");

test("a single-round apply is that round's own note", () => {
  const note = combinedApplyNote(MD, ["diff:1"]);
  assert.match(note, /^Apply agent review round 1 to the code\./);
  assert.match(note, /the cache never expires/);
  assert.doesNotMatch(note, /migration/);
});

test("a combined apply carries every round, in order, as one list of work", () => {
  const note = combinedApplyNote(MD, ["diff:1", "drift:1"]);
  assert.match(note, /applies 2 review rounds from one check run/);
  const cache = note.indexOf("the cache never expires");
  const migration = note.indexOf("the migration test");
  assert.ok(cache > 0 && migration > cache);
  // The drift note says where the specification is.
  assert.match(note, /divergences from `plan.md`/);
});

test("the browser branch publishes every name app.js calls", () => {
  const vm = require("node:vm");
  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-checks.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });
  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  for (const name of [
    "availablePasses",
    "passSpec",
    "passLabel",
    "passBadge",
    "passDetail",
    "recommendedSelection",
    "autopilotSelection",
    "runSummary",
    "combinedApplyNote",
  ]) {
    assert.equal(typeof win[name], "function", `${name} must be on window`);
    assert.ok(app.includes(`${name}(`), `app.js is expected to call ${name}`);
  }
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  for (const dep of ["wf-compose.js", "wf-plan.js", "wf-review.js", "wf-pr-scope.js"]) {
    assert.ok(html.indexOf(dep) < html.indexOf("wf-checks.js"), `${dep} must load before wf-checks.js`);
  }
  assert.ok(html.indexOf("wf-checks.js") < html.indexOf("app.js"), "wf-checks.js must load before app.js");
});
