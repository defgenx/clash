// node --test gui/tests/ — run the real check launcher against stubs.
//
// `checkButtons` is a closure inside `renderWfActions`, so no other test can
// reach it: a source match proves the gate is *written*, not that every
// identifier it reads resolves. A dangling reference here throws while the
// action bar is being built, which blanks the bar for the whole item — at
// click time, in a webview with no console, long after a healthy
// `frontend booted:` line. The comparison (plan vs changes) is the pass with
// the most gates, so it is the one pinned here.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction } = require("./extract-fn.js");

const dist = (f) => fs.readFileSync(path.join(__dirname, "..", "dist", f), "utf8");
const APP = dist("app.js");
const MODULES = ["wf-compose.js", "wf-plan.js", "wf-review.js", "wf-prs.js", "wf-pr-scope.js", "wf-checks.js"].map(
  dist
);

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

/// The real launcher, wired to stubs. `added` records every `add(...)` call
/// (what the bar would have rendered), `menu` the ▾ menu's entries.
function run(item, { status } = {}) {
  const added = [];
  const launched = [];
  let menu = [];
  const sandbox = {
    console,
    add: (label, cls, fn, title, zone, act) => {
      const b = { label, cls, fn, title, zone, act, getBoundingClientRect: () => ({ left: 0, bottom: 0 }) };
      added.push(b);
      return b;
    },
    showContextMenu: (x, y, entries) => {
      menu = entries;
    },
    launchWfReview: (it, root, opts) => launched.push({ fn: "review", opts }),
    launchWfReviewRespond: () => launched.push({ fn: "respond" }),
    launchWfSelfReview: () => launched.push({ fn: "self-review" }),
    wfLaunchExplain: (it, root, target) => launched.push({ fn: "explain", target }),
    wfComposeChecks: () => launched.push({ fn: "checklist" }),
    item,
    root: {},
    st: status ?? item.meta.status,
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  for (const src of MODULES) vm.runInContext(src, sandbox);
  // One contiguous slice of app.js's own gating helpers — `WF_DECISION`
  // through `wfHasPr`, which covers `wfCanReview` and `wfHasPlanPhase`.
  // Restating them as stubs would make this test assert what it believes
  // rather than what the bar actually consults.
  const from = APP.indexOf("const WF_DECISION = new Set");
  const to = APP.indexOf("\n", APP.indexOf("const wfHasPr = (item) =>"));
  assert.ok(from >= 0 && to > from, "app.js must declare its wf gating helpers");
  vm.runInContext(APP.slice(from, to), sandbox);
  for (const fn of ["wfChecksCtx", "wfSoloChecks", "wfCanSelfReview", "wfExplainAny"]) {
    vm.runInContext(extractFunction(APP, fn), sandbox);
  }
  vm.runInContext(extractArrow(extractFunction(APP, "renderWfActions"), "checkButtons"), sandbox);
  vm.runInContext("checkButtons();", sandbox);
  const open = () => {
    const m = added.find((b) => b.label === "▾" || b.label === "Run one check ▾");
    if (!m) return [];
    m.fn();
    return menu;
  };
  return { added, launched, open };
}

const BASE = {
  hasPlan: true,
  reviewRounds: {},
  meta: { status: "diff-review", mode: "full", repoPath: "/repo" },
};
const drift = (entries) => entries.find((e) => /^Plan vs changes/.test(e.label));

test("the comparison is offered where both a plan and a change exist", () => {
  for (const status of ["diff-review", "pr-draft", "pr-ready"]) {
    const { added, open } = run({ ...BASE, meta: { ...BASE.meta, status } });
    const check = added.find((b) => b.act === "checks");
    assert.ok(check, `${status} must offer the checklist`);
    // "This step" — checks never advance the pipeline.
    assert.equal(check.zone, "step");
    // The tooltip has to price it: this spends tokens on agent sessions.
    assert.match(check.title, /Spends tokens/);
    const entry = drift(open());
    assert.ok(entry, `${status} must offer plan vs changes on its own`);
    // The trailing … is the bar's convention for a click that will ask
    // something — this one opens the round composer.
    assert.match(entry.label, /…$/);
    assert.equal(entry.hint, "tokens");
  }
});

test("a click launches the composer with the target pinned", () => {
  const { open, launched } = run(BASE);
  drift(open()).action();
  assert.equal(launched.length, 1);
  // The options object is built inside the vm realm, so its prototype is not
  // this realm's — compare the fields, not the object.
  assert.deepEqual({ ...launched[0].opts }, { target: "drift" });
});

test("neither side of the comparison may be missing", () => {
  const offered = (item) => !!drift(run(item).open());
  // Nothing built yet: at these stages there is only a plan, so there is no
  // divergence to measure. `can_request_review` allows plan-review (a plan
  // review belongs there), which is why this needs its own check.
  for (const status of ["draft", "plan-review"]) {
    assert.equal(offered({ ...BASE, meta: { ...BASE.meta, status } }), false, status);
  }
  // No plan: a review-only item has none and never will.
  assert.equal(offered({ ...BASE, meta: { ...BASE.meta, mode: "review-only" } }), false);
  // A plan phase but an empty plan.md — the file is created empty, so
  // `hasPlan` is the real precondition, not the mode.
  assert.equal(offered({ ...BASE, hasPlan: false }), false);
  // Stages with no parked decision, and the ones where an agent of ours is
  // already writing the item's files.
  for (const status of ["planning", "implementing", "reviewing", "changes-requested", "done"]) {
    assert.equal(offered({ ...BASE, meta: { ...BASE.meta, status } }), false, status);
  }
  // No repository to read: every round needs one, and `wfCanReview` owns that.
  assert.equal(offered({ ...BASE, meta: { ...BASE.meta, repoPath: "" } }), false);
});

test("a run in flight leaves only the explanations, which run alongside it", () => {
  const { added, open } = run({ ...BASE, run: { id: 1, passes: [] } });
  assert.ok(!added.some((b) => b.act === "checks"), "no second run while one is open");
  const entries = open();
  assert.ok(entries.length > 0);
  assert.ok(entries.every((e) => /^Explain/.test(e.label)), entries.map((e) => e.label).join(", "));
});
