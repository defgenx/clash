// node --test gui/tests/ — run the real drift action builder against stubs.
//
// `driftButton` is a closure inside `renderWfActions`, so no other test can
// reach it: a source match proves the gate is *written*, not that every
// identifier it reads resolves. A dangling reference here throws while the
// action bar is being built, which blanks the bar for the whole item — at
// click time, in a webview with no console, long after a healthy
// `frontend booted:` line.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
const WF_PLAN = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-plan.js"), "utf8");

/// Pull one `const <name> = () => { … };` out of a function body by brace
/// matching from the arrow's own brace.
function extractArrow(source, name) {
  const at = source.indexOf(`const ${name} = () => {`);
  assert.ok(at >= 0, `${name} must exist as a zero-arg arrow`);
  let i = source.indexOf("{", at);
  let depth = 0;
  for (; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) break;
  }
  return source.slice(at, i + 1);
}

/// The real gate, wired to stubs. `added` records every `add(...)` call, which
/// is what the action bar would have rendered.
function run(item, { status } = {}) {
  const win = {};
  vm.runInNewContext(WF_PLAN, { window: win });
  const added = [];
  const launched = [];
  const sandbox = {
    ...win,
    console,
    // The gate's real collaborators, taken from app.js rather than restated —
    // a stub of `wfCanReview` would pass whatever this test believed.
    add: (label, cls, fn, title, zone) => {
      added.push({ label, cls, fn, title, zone });
    },
    launchWfReview: (it, root, opts) => {
      launched.push(opts);
    },
    item,
    root: {},
    st: status ?? item.meta.status,
  };
  vm.createContext(sandbox);
  // One contiguous slice of app.js's own gating helpers — `WF_DECISION`
  // through `wfHasPr`, which covers `wfCanReview` and `wfHasPlanPhase`.
  // Restating them as stubs would make this test assert what it believes
  // rather than what the bar actually consults.
  const from = APP.indexOf("const WF_DECISION = new Set");
  const to = APP.indexOf("\n", APP.indexOf("const wfHasPr = (item) =>"));
  assert.ok(from >= 0 && to > from, "app.js must declare its wf gating helpers");
  vm.runInContext(APP.slice(from, to), sandbox);
  vm.runInContext(extractArrow(extractFunction(APP, "renderWfActions"), "driftButton"), sandbox);
  vm.runInContext("driftButton();", sandbox);
  return { added, launched };
}

const BASE = {
  hasPlan: true,
  reviewRounds: {},
  meta: { status: "diff-review", mode: "full", repoPath: "/repo" },
};

test("the comparison is offered where both a plan and a change exist", () => {
  for (const status of ["diff-review", "pr-draft", "pr-ready"]) {
    const { added } = run({ ...BASE, meta: { ...BASE.meta, status } });
    assert.equal(added.length, 1, `${status} must offer it`);
    const [btn] = added;
    assert.match(btn.label, /^⇄ Compare plan vs changes/);
    // The trailing … is the bar's convention for a click that will ask
    // something — this one opens the round composer.
    assert.match(btn.label, /…$/);
    // "This step" — it does not advance the pipeline. Putting it in Continue
    // would say a comparison moves the item, which it never does.
    assert.equal(btn.zone, "step");
    // The tooltip has to price it: this spends tokens on an agent session.
    assert.match(btn.title, /Spends tokens/);
    // …and name the one outcome the Apply button cannot carry.
    assert.match(btn.title, /amending the plan/);
  }
});

test("the round number appears only once there is a previous one", () => {
  const first = run(BASE).added[0];
  assert.equal(first.label, "⇄ Compare plan vs changes…");
  const again = run({ ...BASE, reviewRounds: { drift: 2 } }).added[0];
  assert.match(again.label, /· round 3…$/);
  // Another target's rounds must not bleed into this count — numbers restart
  // per target, which is the whole reason `reviewRounds` is a tally.
  const other = run({ ...BASE, reviewRounds: { diff: 7, plan: 4 } }).added[0];
  assert.equal(other.label, "⇄ Compare plan vs changes…");
});

test("a click launches the composer with the target pinned", () => {
  const { added, launched } = run(BASE);
  added[0].fn();
  assert.equal(launched.length, 1);
  // The options object is built inside the vm realm, so its prototype is not
  // this realm's — compare the fields, not the object.
  assert.deepEqual({ ...launched[0] }, { target: "drift" });
});

test("neither side of the comparison may be missing", () => {
  // Nothing built yet: at these stages there is only a plan, so there is no
  // divergence to measure. `can_request_review` allows plan-review (a plan
  // review belongs there), which is why this needs its own check.
  for (const status of ["draft", "plan-review"]) {
    assert.equal(run({ ...BASE, meta: { ...BASE.meta, status } }).added.length, 0, status);
  }
  // No plan: a review-only item has none and never will.
  assert.equal(run({ ...BASE, meta: { ...BASE.meta, mode: "review-only" } }).added.length, 0);
  // A plan phase but an empty plan.md — the file is created empty, so
  // `hasPlan` is the real precondition, not the mode.
  assert.equal(run({ ...BASE, hasPlan: false }).added.length, 0);
  // Stages with no parked decision, and the ones where an agent of ours is
  // already writing the item's files.
  for (const status of ["planning", "implementing", "reviewing", "changes-requested", "done"]) {
    assert.equal(run({ ...BASE, meta: { ...BASE.meta, status } }).added.length, 0, status);
  }
  // No repository to read: every round needs one, and `wfCanReview` owns that.
  assert.equal(run({ ...BASE, meta: { ...BASE.meta, repoPath: "" } }).added.length, 0);
});
