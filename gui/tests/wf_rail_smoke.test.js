// node --test gui/tests/ — the item cockpit's rail, run.
//
// The rail is reached only by opening an item in a wide tab, so a dangling
// reference in it would show as an empty right column while clash.log looks
// healthy. This runs the real `wfRenderRail` against the real pure modules,
// and pins how the Workbench assembles its three columns.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

const dist = (f) => fs.readFileSync(path.join(__dirname, "..", "dist", f), "utf8");
const APP = dist("app.js");
const gh = (r, n) => `https://github.com/o/${r}/pull/${n}`;

const ITEM = {
  project: "p",
  slug: "s",
  prCheckedAt: Date.now(),
  meta: {
    status: "pr-ready",
    pr: { url: gh("api", 1), number: 1, state: "OPEN" },
    linkedPrs: [{ url: gh("web", 2), number: 2, state: "MERGED" }],
  },
  waves: {
    waves: [{ id: "w1", title: "Schema", state: "review", blockedBy: [], compatibleWith: [], prs: [gh("api", 1)] }],
  },
};

function run(subView = "plan", sessions = []) {
  const box = {
    console,
    Date,
    state: { sessions },
    document: { createElement: (t) => stubEl(t) },
    svgIcon: (n) => `<svg data-icon="${n}"></svg>`,
    spinButton: (b, fn) => fn(),
    invoke: async () => ({}),
    refreshWorkflows: async () => {},
    buildWorkflowView: () => {},
    openSession: () => {},
    openLink: () => {},
    showContextMenu: () => {},
    wfPrMenu: () => [],
    flashToast: () => {},
  };
  box.window = box;
  vm.createContext(box);
  for (const f of ["wf-prs.js", "wf-waves.js", "wf-rail.js"]) vm.runInContext(dist(f), box);
  for (const fn of ["wfPrStateChip", "wfRenderRail"]) vm.runInContext(extractFunction(APP, fn), box);
  const rail = stubEl("aside");
  box.wfRenderRail(rail, {}, ITEM, { subView });
  return rail;
}

const texts = (root) => descendants(root).map((e) => e.textContent).filter(Boolean);

test("the rail carries the waves, the PRs and the item's agents", () => {
  const rail = run("plan", [
    { id: "a", name: "S · code review r3", status: "Thinking", is_running: true, workflow_item: "p/s" },
  ]);
  const t = texts(rail);
  for (const h of ["Waves", "Pull requests", "Agents"]) assert.ok(t.includes(h), h);
  assert.ok(t.some((x) => /w1 · Schema is waiting for your approval/.test(x)));
  assert.ok(t.some((x) => /^2 PRs · 1 merged · 1 open · checked just now$/.test(x)), t.join("\n"));
  assert.ok(t.includes("api#1 · open") && t.includes("web#2 · merged"));
  assert.ok(t.includes("All pull requests →"));
  assert.ok(t.includes("S · code review r3") && t.includes("thinking"));
  assert.ok(rail.children.every((c) => c.className === "wf-rail-card"));
});

test("the rail leaves out the card of the tab beside it", () => {
  const t = texts(run("waves"));
  assert.ok(!t.includes("Waves") && t.includes("Pull requests"));
  assert.ok(texts(run("plan")).includes("No agent session yet."));
});

test("the workbench is outline | content | rail, the actions on top of the content", () => {
  // The next-step card stays where it always was, above the content, at
  // every width; the rail only adds context beside it when there is room.
  const view = extractFunction(APP, "buildWorkflowView");
  assert.match(view, /wrap\.append\(wfRenderOutline\(subViews, ts\.subView, selectSubView\), rail, main\);/);
  assert.match(view, /if \(bench\) host\.insertBefore\(actions, host\.firstChild\);/);
  assert.match(view, /host\.appendChild\(strip\);/);
  assert.doesNotMatch(view, /rail\.insertBefore\(actions/);
  assert.match(view, /if \(rail\) wfRenderRail\(rail, el, item, ts\);/);
  const css = dist("style.css");
  assert.match(css, /\.wf-view\.wf-bench-view \{ container: wfitem \/ inline-size; \}/);
  assert.match(css, /\.wf-bench > \.wf-rail \{ grid-area: rail; display: none; \}/);
  assert.match(css, /@container wfitem \(min-width: 1180px\) \{[\s\S]*?grid-template-areas: "outline main rail";/);
  assert.match(css, /grid-template-areas: "outline main";/);
});

test("nothing in the item tab is pushed past its right edge", () => {
  // The tab clips (.wf-view overflow: hidden), so a row that cannot wrap
  // hides its last buttons with no way to scroll to them.
  const css = dist("style.css");
  assert.match(css, /\.wf-view \{[^}]*overflow: hidden;/);
  assert.match(css, /\.wf-head \{[^}]*flex-wrap: wrap;/);
  assert.match(css, /\.wf-title \{[^}]*text-overflow: ellipsis;/);
  assert.match(css, /\.wf-head \.wf-meta \{[^}]*text-overflow: ellipsis;/);
  // The stepper scrolls rather than clips.
  assert.match(css, /\.wf-pipeline \{[^}]*overflow-x: auto;/);
});
