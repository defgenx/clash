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

test("Topbar shows the active view alone, full height, only while dropped down", () => {
  const p = L.sidebarPlan("topbar", { active: "workflows" });
  assert.deepEqual(p.views.filter((x) => x.shown).map((x) => x.id), ["workflows"]);
  assert.equal(by(p).workflows.fill, true);
  assert.equal(p.rail, true);
  assert.equal(p.workspaceChips, false, "workspaces live in the rail");
  assert.equal(L.sidebarPlan("topbar", {}).panel, false, "Topbar's panel is out only when opened");
  assert.equal(L.sidebarPlan("topbar", { panelOpen: true }).panel, true);
  assert.equal(L.sidebarPlan("topbar", {}).place, "top");
  assert.equal(L.sidebarPlan("rail", {}).panel, true, "Rail docks its panels");
});

test("Stack: the rail's icons stack views in one panel, always in the catalogue's order", () => {
  const shown = (p) => p.views.filter((v) => v.shown).map((v) => v.id);
  const p = L.sidebarPlan("stack", { stacked: ["teams", "sessions", "teams", "nope"] });
  assert.deepEqual(shown(p), ["sessions", "teams"], "known views, once each, catalogue order");
  assert.deepEqual(p.stacked, ["sessions", "teams"]);
  assert.ok(p.views.filter((v) => v.shown).every((v) => v.expanded && v.fill), "they share the height");
  assert.equal(p.place, "left");
  assert.equal(p.rail, true);
  assert.equal(p.rightView, null, "one panel, no right side");
  assert.ok(L.managesSections(L.layoutOf("stack")));
  // Nothing (or garbage) saved is the default stack, never an empty panel.
  assert.deepEqual(L.normalizeStack([]), L.DEFAULT_STACK);
  assert.deepEqual(L.normalizeStack("teams"), L.DEFAULT_STACK);
  assert.deepEqual(shown(L.sidebarPlan("stack", {})), L.DEFAULT_STACK);
  // Only Stack carries a stack.
  assert.equal(L.sidebarPlan("rail", {}).stacked, null);
  // The lit icons are the stack, and a folded panel lights none.
  assert.deepEqual(L.railLit(p), ["sessions", "teams"]);
  assert.deepEqual(L.railLit(p, { left: false }), []);
});

test("a Stack click adds, removes or solos a view, and the last one folds instead of emptying", () => {
  assert.deepEqual(L.stackToggle(["sessions", "teams"], "workflows"), { stacked: ["sessions", "workflows", "teams"], fold: false });
  assert.deepEqual(L.stackToggle(["sessions", "teams"], "teams"), { stacked: ["sessions"], fold: false });
  assert.deepEqual(L.stackToggle(["sessions"], "sessions"), { stacked: ["sessions"], fold: true });
  assert.deepEqual(L.stackToggle(["sessions", "teams"], "workflows", { solo: true }), { stacked: ["workflows"], fold: false });
  assert.deepEqual(L.stackToggle(["sessions"], "nope"), { stacked: ["sessions"], fold: false });
});

test("Dock and Split show every view at once, open and filling, whatever was active or collapsed", () => {
  for (const id of ["dock", "split"]) {
    const p = L.sidebarPlan(id, { active: "teams", open: { wfOpen: false, notesOpen: false } });
    assert.ok(p.views.every((v) => v.shown && v.expanded && v.fill), id);
    assert.equal(p.rail, false, id);
    assert.equal(p.workspaceChips, true, `${id} keeps the chips`);
    assert.equal(p.panel, true, `${id} docks its views`);
    assert.ok(L.managesSections(L.layoutOf(id)), `${id} drives the sections`);
  }
  assert.equal(L.sidebarPlan("dock", {}).place, "bottom");
  assert.equal(L.sidebarPlan("split", {}).place, "split");
  assert.equal(L.managesSections(L.layoutOf("classic")), false, "Classic leaves them to their flags");
  // Split moves the work lists, never Sessions.
  assert.deepEqual(L.SPLIT_RIGHT, ["workflows", "scratches", "teams"]);
  for (const id of L.SPLIT_RIGHT) assert.ok(L.viewOf(id), id);
});

test("Rail: Sessions always on the left, one right-hand view, switched from the rail", () => {
  const plan = (active) => L.sidebarPlan("rail", { active });
  const shown = (p) => p.views.filter((v) => v.shown).map((v) => v.id);
  assert.deepEqual(shown(plan("teams")), ["sessions", "teams"]);
  assert.equal(plan("teams").rightView, "teams");
  // Choosing Sessions (or nothing usable) keeps a right view on screen.
  assert.deepEqual(shown(plan("sessions")), ["sessions", "workflows"]);
  assert.deepEqual(shown(plan("nope")), ["sessions", "workflows"]);
  const p = plan("scratches");
  assert.equal(p.place, "split");
  assert.equal(p.rail, true);
  assert.equal(p.workspaceChips, false, "workspaces live in the rail");
  assert.equal(p.quickSettings, "popover");
  assert.equal(p.panels, "single");
  assert.ok(p.views.filter((v) => v.shown).every((v) => v.expanded && v.fill));
  // Split shows every view, so it has no right view to pick.
  assert.equal(L.sidebarPlan("split", { active: "teams" }).rightView, null);
  assert.equal(L.sidebarPlan("topbar", { active: "teams" }).rightView, null);
});

test("a view's side, and the rail icons lit for what is on screen", () => {
  assert.equal(L.sideOf("rail", "teams"), "right");
  assert.equal(L.sideOf("rail", "sessions"), "left");
  assert.equal(L.sideOf("split", "workflows"), "right");
  assert.equal(L.sideOf("stack", "workflows"), "left", "only the split layouts have a right side");
  const rs = L.sidebarPlan("rail", { active: "teams" });
  assert.deepEqual(L.railLit(rs), ["sessions", "teams"]);
  assert.deepEqual(L.railLit(rs, { left: false }), ["teams"], "a folded side lights nothing");
  assert.deepEqual(L.railLit(rs, { right: false }), ["sessions"]);
  assert.deepEqual(L.railLit(L.sidebarPlan("topbar", { active: "teams" })), [], "Topbar's panel is not out");
  assert.deepEqual(L.railLit(L.sidebarPlan("topbar", { active: "teams", panelOpen: true })), ["teams"]);
});

test("every layout says where it sits and how its sections behave", () => {
  for (const l of L.LAYOUTS) {
    assert.ok(["left", "bottom", "split", "top"].includes(l.place), `${l.id}: place`);
    assert.ok(["collapsible", "one", "some", "all"].includes(l.sections), `${l.id}: sections`);
    // An overlay panel needs a rail to open it from.
    if (l.overlay) assert.ok(l.rail, `${l.id}: overlay without a rail`);
  }
  // The places are what make the layouts different; no two may share one shape.
  const shapes = L.LAYOUTS.map((l) => [l.place, l.sections, l.rail, !!l.tabs, l.overlay].join("/"));
  assert.equal(new Set(shapes).size, shapes.length, "two layouts with the same shape");
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

test("retired layouts: Focus falls back to Classic, Flyout and Rail Split become Rail", () => {
  assert.deepEqual(L.LAYOUTS.map((l) => l.id), ["classic", "rail", "stack", "tabbed", "dock", "split", "topbar"]);
  assert.equal(L.layoutOf("railsplit").id, "rail");
  assert.equal(L.layoutOf("focus").id, "classic");
  assert.equal(L.layoutOf("flyout").id, "rail");
  assert.equal(L.sidebarPlan("flyout", {}).layout, "rail");
});

test("right panels: stacked in Classic, one at a time in Rail and Split, drawers in Topbar", () => {
  assert.equal(L.sidebarPlan("classic", {}).panels, "stack");
  assert.equal(L.sidebarPlan("rail", {}).panels, "single");
  assert.equal(L.sidebarPlan("split", {}).panels, "single", "beside a second sidebar, one panel is enough");
  assert.equal(L.sidebarPlan("topbar", {}).panels, "overlay");
  assert.deepEqual(L.rightPanelsAfterOpen("stack", ["files"], "details"), ["files", "details"]);
  assert.deepEqual(L.rightPanelsAfterOpen("single", ["files"], "details"), ["details"]);
  assert.deepEqual(L.rightPanelsAfterOpen("overlay", ["details"], "files"), ["files"]);
  assert.deepEqual(L.rightPanelsAfterOpen("stack", [], "files"), ["files"]);
  for (const l of L.LAYOUTS) assert.ok(["stack", "single", "overlay"].includes(l.panels), `${l.id} says how panels open`);
});

test("quick settings: a footer section in Classic, the rail's ⚙ popover in Rail and Topbar", () => {
  assert.equal(L.sidebarPlan("classic", {}).quickSettings, "footer");
  assert.equal(L.sidebarPlan("rail", {}).quickSettings, "popover");
  assert.equal(L.sidebarPlan("stack", {}).quickSettings, "popover");
  assert.equal(L.sidebarPlan("topbar", {}).quickSettings, "popover");
  // A rail layout without the popover would leave them under the open view.
  for (const l of L.LAYOUTS) assert.equal(l.quickSettings === "popover", l.rail, `${l.id}: popover iff rail`);
});

test("Tabbed: one view at a time like Rail, but switched by a tab strip beside the workspace chips", () => {
  const plan = L.sidebarPlan("tabbed", { active: "teams", open: { wfOpen: true } });
  assert.equal(plan.rail, false);
  assert.equal(plan.viewTabs, true);
  assert.equal(plan.workspaceChips, true, "no rail, so the chips stay");
  assert.equal(plan.panel, true);
  assert.deepEqual(plan.views.filter((v) => v.shown).map((v) => v.id), ["teams"]);
  assert.ok(plan.views.find((v) => v.id === "teams").fill);
  assert.equal(plan.quickSettings, "footer");
  for (const id of ["classic", "rail", "stack", "dock", "split", "topbar"]) assert.equal(L.sidebarPlan(id, {}).viewTabs, false, id);
});
