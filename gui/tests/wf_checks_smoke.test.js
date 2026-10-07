// node --test gui/tests/ — run the real check-run strips out of app.js.
//
// The strip is the first thing on every item's action bar, built on each
// render. A dangling reference in it throws while the bar is built, which
// blanks the bar for the whole item — at render time, in a webview with no
// console, while clash.log still shows a healthy boot line.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

const dist = (f) => fs.readFileSync(path.join(__dirname, "..", "dist", f), "utf8");
const APP = dist("app.js");
const MODULES = [
  "wf-compose.js",
  "wf-plan.js",
  "wf-review.js",
  "wf-prs.js",
  "wf-pr-scope.js",
  "wf-next.js",
  "wf-checks.js",
].map(dist);

function sandbox(over = {}) {
  const calls = [];
  const box = {
    console,
    document: { createElement: (tag) => stubEl(tag) },
    state: { wfAssist: "suggest" },
    busyButton: (b, fn) => fn(),
    wfStartRun: async (...args) => calls.push(["start", ...args]),
    wfComposeChecks: (...args) => calls.push(["checklist", ...args]),
    wfRunControl: (...args) => calls.push(["control", ...args]),
    wfApplyRun: (...args) => calls.push(["apply", ...args]),
    openSession: (...args) => calls.push(["open", ...args]),
    uiConfirm: async () => true,
    ...over,
  };
  box.window = box;
  vm.createContext(box);
  for (const src of MODULES) vm.runInContext(src, box);
  const from = APP.indexOf("const WF_DECISION = new Set");
  const to = APP.indexOf("\n", APP.indexOf("const wfHasPr = (item) =>"));
  vm.runInContext(APP.slice(from, to), box);
  const glyph = APP.slice(APP.indexOf("const WF_PASS_GLYPH"), APP.indexOf("\n", APP.indexOf("const WF_PASS_GLYPH")));
  vm.runInContext(glyph, box);
  for (const fn of [
    "wfChecksCtx",
    "wfCanSelfReview",
    "wfStaleExplanations",
    "wfDefaultPrUrls",
    "wfRunStrip",
    "wfRenderNextStep",
  ]) {
    vm.runInContext(extractFunction(APP, fn), box);
  }
  return { box, calls };
}

/// A bar that records what the strip prepends.
function bar() {
  const b = { strip: null, querySelectorAll: () => [], prepend: (el) => (b.strip = el) };
  return b;
}

const texts = (el) => descendants(el).map((c) => c.textContent).filter(Boolean);

const ITEM = {
  project: "p",
  slug: "s",
  agentAlive: true,
  hasPlan: true,
  openAnnotations: 0,
  reviewRounds: {},
  meta: { status: "diff-review", mode: "full", repoPath: "/repo", iteration: 1 },
};

test("due checks render as chips with one Run button", async () => {
  const { box, calls } = sandbox();
  const b = bar();
  const step = box.wfRenderNextStep(b, ITEM);
  assert.equal(step.kind, "check");
  const all = texts(b.strip);
  assert.ok(all.includes("✓ Code review"), all.join(" | "));
  assert.ok(all.includes("✓ Plan vs changes"), all.join(" | "));
  assert.ok(all.includes("▶ Run 2 checks"), all.join(" | "));
  const go = descendants(b.strip).find((c) => c.textContent === "▶ Run 2 checks");
  await go.onclick();
  const [, item, passes, opts] = calls.find((c) => c[0] === "start");
  assert.equal(item, ITEM);
  // Built in the vm realm: compare the values, not the arrays.
  assert.deepEqual([...passes.map((p) => p.target)], ["diff", "drift"]);
  assert.equal(opts.interactive, null);
});

test("unticking a chip shrinks the run before it starts", async () => {
  const { box, calls } = sandbox();
  const b = bar();
  box.wfRenderNextStep(b, ITEM);
  const chip = descendants(b.strip).find((c) => c.textContent === "✓ Plan vs changes");
  chip.onclick();
  assert.equal(chip.textContent, "Plan vs changes");
  const go = descendants(b.strip).find((c) => c.className === "wf-next-go primary");
  assert.equal(go.textContent, "▶ Run code review");
  await go.onclick();
  const [, , passes] = calls.find((c) => c[0] === "start");
  assert.deepEqual([...passes.map((p) => p.target)], ["diff"]);
});

test("a run in flight renders its progress and controls instead", async () => {
  const { box, calls } = sandbox();
  const b = bar();
  const run = {
    id: 4,
    by: "human",
    passes: [
      { target: "diff", state: "running", sessionId: "sid", round: 2 },
      { target: "drift", state: "queued" },
    ],
    apply: { state: "none" },
  };
  assert.equal(box.wfRenderNextStep(b, { ...ITEM, run }), null);
  const all = texts(b.strip);
  assert.ok(all.includes("Checks · 0 of 2 done · code review running · next: plan vs changes"), all.join(" | "));
  const stop = descendants(b.strip).find((c) => c.textContent === "Stop after this one");
  await stop.onclick();
  const [, , batchId, control] = calls.at(-1);
  assert.equal(batchId, 4);
  assert.deepEqual({ ...control }, { op: "stop" });
  const open = descendants(b.strip).find((c) => c.textContent === "Open session");
  await open.onclick();
  assert.deepEqual(calls.at(-1), ["open", "sid"]);
});

test("findings waiting on you and a failed pass each get their button", async () => {
  const { box, calls } = sandbox();
  const b = bar();
  const run = {
    id: 5,
    passes: [
      { target: "diff", state: "done", round: 1 },
      { target: "drift", state: "failed", error: "boom" },
    ],
    paused: true,
    apply: { state: "pending", auto: false, keys: ["diff:1"] },
  };
  box.wfRenderNextStep(b, { ...ITEM, run });
  const all = texts(b.strip);
  assert.ok(all.includes("↻ Retry plan vs changes"), all.join(" | "));
  assert.ok(all.includes("↻ Apply the findings"), all.join(" | "));
  const apply = descendants(b.strip).find((c) => c.textContent === "↻ Apply the findings");
  await apply.onclick();
  const [, project, slug, batch, opts] = calls.at(-1);
  assert.deepEqual([project, slug, batch.id, opts.human], ["p", "s", 5, true]);
});
