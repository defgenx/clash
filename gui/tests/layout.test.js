// node --test gui/tests/ — the sidebar layouts' pure model (gui/dist/layout.js).
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const L = require("../dist/layout.js");

const by = (plan) => Object.fromEntries(plan.views.map((v) => [v.id, v]));

test("Classic stacks every view and keeps each section's own collapse flag", () => {
  const p = L.sidebarPlan("classic", { active: "teams", open: { wfOpen: true, notesOpen: false } });
  const v = by(p);
  assert.ok(p.views.every((x) => x.shown), "everything on screen");
  assert.equal(v.workflows.expanded, true);
  assert.equal(v.scratches.expanded, false);
  assert.equal(v.teams.expanded, false, "the active view means nothing to Classic");
  assert.ok(p.views.every((x) => !x.fill));
  assert.equal(p.rail, false);
  assert.equal(p.workspaceChips, true);
});

test("Rail and Flyout show the active view alone, full height", () => {
  for (const id of ["rail", "flyout"]) {
    const p = L.sidebarPlan(id, { active: "workflows" });
    assert.deepEqual(p.views.filter((x) => x.shown).map((x) => x.id), ["workflows"]);
    assert.equal(by(p).workflows.fill, true);
    assert.equal(p.rail, true);
    assert.equal(p.workspaceChips, false, "workspaces live in the rail");
  }
  assert.equal(L.sidebarPlan("rail", {}).panel, true, "Rail docks its panel");
  assert.equal(L.sidebarPlan("flyout", {}).panel, false, "Flyout's panel is out only when opened");
  assert.equal(L.sidebarPlan("flyout", { panelOpen: true }).panel, true);
});

test("unknown layouts and views fall back instead of blanking the sidebar", () => {
  const p = L.sidebarPlan("nope", { active: "nope" });
  assert.equal(p.layout, L.DEFAULT_LAYOUT);
  assert.equal(p.active, "sessions");
  assert.equal(L.sidebarPlan("rail", { active: "nope" }).active, "sessions");
});

test("workspace initials and rail badges", () => {
  assert.equal(L.workspaceInitials("Saturn Core", 0), "SC");
  assert.equal(L.workspaceInitials("main", 0), "MA");
  assert.equal(L.workspaceInitials("audit-logs", 0), "AL");
  assert.equal(L.workspaceInitials("  ", 4), "5");
  const items = L.railItems("teams", { workflows: 3, teams: 0 });
  assert.deepEqual(items.map((i) => i.id), L.SIDEBAR_VIEWS.map((v) => v.id));
  assert.equal(items.find((i) => i.id === "workflows").badge, 3);
  assert.equal(items.find((i) => i.id === "teams").active, true);
});

test("every view's elements and every layout exist where app.js expects them", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  for (const v of L.SIDEBAR_VIEWS) {
    for (const id of [...v.elements, ...(v.resizer ? [v.resizer] : [])]) {
      assert.ok(html.includes(`id="${id}"`), `${v.id} owns #${id}, which index.html lacks`);
    }
  }
  // The setting offers exactly the catalogue.
  const select = html.slice(html.indexOf('id="set-layout"'), html.indexOf("</select>", html.indexOf('id="set-layout"')));
  for (const l of L.LAYOUTS) assert.ok(select.includes(`value="${l.id}"`), `${l.id} missing from the setting`);
  // The browser branch publishes the global app.js reads.
  const sandbox = { globalThis: {} };
  sandbox.globalThis = sandbox;
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "..", "dist", "layout.js"), "utf8"), sandbox);
  assert.equal(typeof sandbox.SidebarLayout.sidebarPlan, "function");
});

test("Focus is gone: a saved Focus layout falls back to Classic", () => {
  assert.deepEqual(L.LAYOUTS.map((l) => l.id), ["classic", "rail", "flyout"]);
  assert.equal(L.layoutOf("focus").id, "classic");
});

test("right panels: stacked in Classic, one at a time in Rail, drawers in Flyout", () => {
  assert.equal(L.sidebarPlan("classic", {}).panels, "stack");
  assert.equal(L.sidebarPlan("rail", {}).panels, "single");
  assert.equal(L.sidebarPlan("flyout", {}).panels, "overlay");
  assert.deepEqual(L.rightPanelsAfterOpen("stack", ["files"], "details"), ["files", "details"]);
  assert.deepEqual(L.rightPanelsAfterOpen("single", ["files"], "details"), ["details"]);
  assert.deepEqual(L.rightPanelsAfterOpen("overlay", ["details"], "files"), ["files"]);
  assert.deepEqual(L.rightPanelsAfterOpen("stack", [], "files"), ["files"]);
  for (const l of L.LAYOUTS) assert.ok(["stack", "single", "overlay"].includes(l.panels), `${l.id} says how panels open`);
});
