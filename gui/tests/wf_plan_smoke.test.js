// node --test gui/tests/ — smoke-render the Plan tab against a stub DOM.
//
// `plan` is the default sub-view of every full-mode item, so a dangling
// reference in this render path breaks the first thing you see when you open
// any workflow item — silently, in a webview with no console. This runs the
// real `renderWfPlanView` body extracted from app.js, in both modes.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
const WF_PLAN = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-plan.js"), "utf8");

const VERSIONS = [
  { iteration: 1, current: false, lines: 40, heading: "2026-09-01 10:00", note: "First pass" },
  { iteration: 2, current: false, lines: 52, heading: "2026-09-01 11:00", note: "Apply review r1" },
  { iteration: 3, current: true, lines: 61, heading: "", note: "" },
];

function sandboxFor({ versions = VERSIONS, diff = "@@ -1 +1 @@\n-old\n+new\n", plan = "# Plan" }) {
  const win = {};
  vm.runInNewContext(WF_PLAN, { window: win });
  const calls = [];
  const sandbox = {
    ...win,
    invoke: async (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "list_workflow_plan_versions") return versions;
      if (cmd === "get_workflow_plan_diff") return diff;
      if (cmd === "get_workflow_doc") return plan;
      if (cmd === "get_workflow_history_plan") return `# Plan at ${args.iteration}`;
      throw new Error(`unexpected command ${cmd}`);
    },
    document: { createElement: (t) => stubEl(t), createTextNode: () => stubEl("text") },
    console,
    Object,
    Number,
    String,
    svgIcon: () => "<svg/>",
    renderMarkdown: (el, text) => {
      el.rendered = text;
    },
    renderMermaidIn: () => {},
    openScratchInEditor: () => {},
    buildWorkflowView: () => {},
    renderUnifiedDiff: null, // replaced by the real one below
  };
  vm.createContext(sandbox);
  vm.runInContext(extractFunction(APP, "renderUnifiedDiff"), sandbox);
  vm.runInContext(extractFunction(APP, "renderWfPlanView"), sandbox);
  sandbox.__calls = calls;
  return sandbox;
}

const item = { project: "p", slug: "s", path: "/items/s", meta: { iteration: 3 } };

test("the plan tab renders the live version with a version bar", async () => {
  const sandbox = sandboxFor({});
  const body = stubEl();
  const ts = { subView: "plan" };
  await sandbox.renderWfPlanView(body, stubEl(), item, ts);
  const all = descendants(body);
  const chips = all.filter((e) => (e.className || "").includes("wf-plan-chip"));
  assert.deepEqual(
    chips.map((c) => c.textContent),
    ["v1", "v2", "current"]
  );
  // Exactly one chip is selected, and it is the head.
  const on = chips.filter((c) => (c.className || "").includes(" on"));
  assert.equal(on.length, 1);
  assert.equal(on[0].textContent, "current");
  // The caption says what you are looking at, not just which number.
  const cap = all.find((e) => (e.className || "").includes("wf-plan-caption"));
  assert.match(cap.textContent, /^current — the live plan — 61 lines/);
  // The live plan is fetched as a doc, not as a snapshot.
  assert.ok(sandbox.__calls.some(([c, a]) => c === "get_workflow_doc" && a.doc === "plan.md"));
  assert.ok(all.some((e) => e.rendered === "# Plan"));
});

test("a single-version plan shows no version bar and no compare", async () => {
  const only = [{ iteration: 1, current: true, lines: 12, heading: "", note: "" }];
  const sandbox = sandboxFor({ versions: only });
  const body = stubEl();
  await sandbox.renderWfPlanView(body, stubEl(), { ...item, meta: { iteration: 1 } }, {
    subView: "plan",
  });
  const all = descendants(body);
  assert.equal(all.filter((e) => (e.className || "").includes("wf-plan-chip")).length, 0);
  // Nothing precedes v1, so there is nothing to compare it against.
  assert.equal(all.filter((e) => e.textContent === "⇄ Changes").length, 0);
});

test("compare mode diffs against the previous version by default", async () => {
  const sandbox = sandboxFor({});
  const body = stubEl();
  const ts = { subView: "plan", planCompare: true };
  await sandbox.renderWfPlanView(body, stubEl(), item, ts);
  const [cmd, args] = sandbox.__calls.find(([c]) => c === "get_workflow_plan_diff");
  assert.equal(cmd, "get_workflow_plan_diff");
  // The head compares against v2, and `to: null` means "the live file".
  assert.deepEqual({ ...args }, { project: "p", slug: "s", from: 2, to: null });
  const all = descendants(body);
  const pre = all.find((e) => (e.className || "").includes("wf-plan-diff"));
  assert.ok(pre, "the diff is rendered as coloured lines");
  const classes = descendants(pre).map((sp) => sp.className);
  assert.ok(classes.includes("pd-add") && classes.includes("pd-del"));
  const cap = all.find((e) => (e.className || "").includes("wf-plan-caption"));
  assert.match(cap.textContent, /^changes from v2 to current/);
});

test("an explicit base overrides the default, and a frozen version is read-only", async () => {
  const sandbox = sandboxFor({});
  const body = stubEl();
  const ts = { subView: "plan", planVersion: 2, planCompare: true, planBase: 1 };
  await sandbox.renderWfPlanView(body, stubEl(), item, ts);
  const [, args] = sandbox.__calls.find(([c]) => c === "get_workflow_plan_diff");
  assert.deepEqual({ ...args }, { project: "p", slug: "s", from: 1, to: 2 });
  // No edit button on a frozen version — it is the record of what was reviewed.
  const all = descendants(body);
  assert.equal(all.filter((e) => /Edit plan\.md/.test(e.innerHTML || "")).length, 0);
});

test("a stale selected version falls back to the live plan", async () => {
  // A restored tab can name a version this item no longer has (an older clash,
  // a deleted history dir). Rendering an error where the plan should be would
  // be worse than showing the plan.
  const sandbox = sandboxFor({});
  const body = stubEl();
  const ts = { subView: "plan", planVersion: 99, planCompare: true, planBase: 42 };
  await sandbox.renderWfPlanView(body, stubEl(), item, ts);
  assert.equal(ts.planVersion, null);
  assert.equal(ts.planBase, null);
  const all = descendants(body);
  assert.ok(all.some((e) => (e.className || "").includes("wf-plan-caption")));
});

test("an empty version list still renders the live plan", async () => {
  // list_workflow_plan_versions failing (or an item mid-creation) must not
  // leave the default tab blank.
  const sandbox = sandboxFor({ versions: [] });
  const body = stubEl();
  await sandbox.renderWfPlanView(body, stubEl(), item, { subView: "plan" });
  const all = descendants(body);
  assert.ok(all.some((e) => e.rendered === "# Plan"));
});
