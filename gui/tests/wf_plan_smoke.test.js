// node --test gui/tests/ — smoke-render the plan readers against a stub DOM.
//
// `plan` is the default sub-view of every full-mode item, so a dangling
// reference in either render path breaks the first thing you see when you open
// a workflow item — silently, in a webview with no console. This runs the real
// `renderWfPlanView` and `renderWfRevisionsView` bodies extracted from app.js.
//
// Reachability is a different question and lives in app_source.test.js: this
// file would happily pass on a renderer nothing ever calls, which is exactly
// what once shipped.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
const WF_PLAN = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-plan.js"), "utf8");

const VERSIONS = [
  { n: 1, current: false, lines: 40, savedAt: 1_000, iteration: 1, reason: "first plan" },
  { n: 2, current: false, lines: 52, savedAt: 2_000, iteration: 2, reason: "revision requested" },
  { n: 3, current: true, lines: 61, savedAt: 3_000, iteration: 2, reason: "changed on disk" },
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
      if (cmd === "get_workflow_plan_version") return `# Plan v${args.n}`;
      throw new Error(`unexpected command ${cmd}`);
    },
    document: { createElement: (t) => stubEl(t), createTextNode: () => stubEl("text") },
    console,
    Object,
    Number,
    String,
    svgIcon: () => "<svg/>",
    escapeHtml: (v) => String(v),
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
  vm.runInContext(extractFunction(APP, "renderWfRevisionsView"), sandbox);
  vm.runInContext(extractFunction(APP, "renderWfPlanView"), sandbox);
  sandbox.__calls = calls;
  return sandbox;
}

const item = { project: "p", slug: "s", path: "/items/s", meta: { iteration: 3 } };

test("the revisions tab lists every recorded revision, newest first", async () => {
  const sandbox = sandboxFor({});
  const body = stubEl();
  const ts = { subView: "plan" };
  await sandbox.renderWfRevisionsView(body, stubEl(), item, ts);
  const all = descendants(body);
  const rows = all.filter((e) => (e.className || "").includes("wf-rev-row"));
  // Newest first: the list is read from the top.
  assert.equal(rows.length, 3);
  assert.match(rows[0].innerHTML, /v3 · current/);
  assert.match(rows[2].innerHTML, /first plan/);
  // Exactly one row is selected, and it is the newest.
  const on = rows.filter((r) => (r.className || "").includes(" on"));
  assert.equal(on.length, 1);
  assert.match(on[0].innerHTML, /v3 · current/);
  // The caption says what you are looking at, not just which number.
  const cap = all.find((e) => (e.className || "").includes("wf-plan-caption"));
  assert.match(cap.textContent, /^the live plan/);
  assert.match(cap.textContent, /changed on disk$/);
  // The live plan is fetched as a doc, not as a snapshot.
  assert.ok(sandbox.__calls.some(([c, a]) => c === "get_workflow_doc" && a.doc === "plan.md"));
  assert.ok(all.some((e) => e.rendered === "# Plan"));
});

test("a single-version plan shows no version bar and no compare", async () => {
  const only = [{ n: 1, current: true, lines: 12, savedAt: 1_000, iteration: 1, reason: "first plan" }];
  const sandbox = sandboxFor({ versions: only });
  const body = stubEl();
  await sandbox.renderWfRevisionsView(body, stubEl(), { ...item, meta: { iteration: 1 } }, {
    subView: "plan",
  });
  const all = descendants(body);
  // One revision still gets its row — the list is the history, however short.
  assert.equal(all.filter((e) => (e.className || "").includes("wf-rev-row")).length, 1);
  // Nothing precedes v1, so there is nothing to compare it against.
  assert.equal(all.filter((e) => e.textContent === "⇄ Changes").length, 0);
});

test("compare mode diffs against the previous version by default", async () => {
  const sandbox = sandboxFor({});
  const body = stubEl();
  const ts = { subView: "plan", planCompare: true };
  await sandbox.renderWfRevisionsView(body, stubEl(), item, ts);
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
  assert.match(cap.textContent, /^changes from v2 to v3 · current/);
});

test("an explicit base overrides the default, and a frozen version is read-only", async () => {
  const sandbox = sandboxFor({});
  const body = stubEl();
  const ts = { subView: "plan", planVersion: 2, planCompare: true, planBase: 1 };
  // v2 is a frozen revision: its text comes from the store, not the live file.
  await sandbox.renderWfRevisionsView(body, stubEl(), item, ts);
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
  // 99 is not a revision this item has.
  await sandbox.renderWfRevisionsView(body, stubEl(), item, ts);
  assert.equal(ts.planVersion, null);
  assert.equal(ts.planBase, null);
  const all = descendants(body);
  assert.ok(all.some((e) => (e.className || "").includes("wf-plan-caption")));
});

test("no recorded revisions says so instead of rendering nothing", async () => {
  const sandbox = sandboxFor({ versions: [] });
  const body = stubEl();
  await sandbox.renderWfRevisionsView(body, stubEl(), item, { subView: "revisions" });
  assert.match(body.innerHTML, /no plan recorded yet/);
});

test("the Plan tab always renders the live plan, with a way into the history", async () => {
  // The reading tab must never open on a comparison, so it holds no version
  // state at all — just the current text and a link.
  const sandbox = sandboxFor({});
  const body = stubEl();
  const ts = { subView: "plan", planVersion: 2, planCompare: true };
  await sandbox.renderWfPlanView(body, stubEl(), item, ts);
  const all = descendants(body);
  assert.ok(all.some((e) => e.rendered === "# Plan"), "the live plan is rendered");
  const link = all.find((e) => /revisions →/.test(e.textContent || ""));
  assert.ok(link, "a way into the history is offered");
  assert.equal(link.textContent, "◫ 3 revisions →");
  // It fetched the live doc, never a frozen revision.
  assert.ok(sandbox.__calls.some(([c, a]) => c === "get_workflow_doc" && a.doc === "plan.md"));
  assert.ok(!sandbox.__calls.some(([c]) => c === "get_workflow_plan_version"));
});

test("a single-revision plan offers no way into the history", async () => {
  const only = [{ n: 1, current: true, lines: 12, savedAt: 1_000, iteration: 1, reason: "first plan" }];
  const sandbox = sandboxFor({ versions: only });
  const body = stubEl();
  await sandbox.renderWfPlanView(body, stubEl(), item, { subView: "plan" });
  assert.ok(!descendants(body).some((e) => /revisions →/.test(e.textContent || "")));
});
