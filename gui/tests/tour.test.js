// node --test gui/tests/ — the GUI tour's pure half.
//
// Steps are data (unique ids, real anchors, prose that fits a tooltip) and
// the placement math is what keeps the tooltip on screen for any anchor —
// both testable without a DOM.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { TOUR_STEPS, windowTourSteps, tourPlacement } = require("../dist/tour.js");

test("steps are well-formed: unique ids, anchors, tooltip-sized prose", () => {
  const ids = new Set();
  for (const s of TOUR_STEPS) {
    assert.ok(s.id && !ids.has(s.id), `duplicate/missing id ${s.id}`);
    ids.add(s.id);
    assert.ok(s.target && typeof s.target === "string", `${s.id}: target element id`);
    assert.ok(s.title && s.title.length <= 40, `${s.id}: title fits a tooltip header`);
  }
  // Every layout gets a complete tour, every body fitting the tooltip.
  for (const layout of ["classic", "rail", "flyout"]) {
    for (const s of windowTourSteps(layout)) {
      assert.equal(typeof s.body, "string", `${layout}/${s.id}: one body`);
      assert.ok(s.body.length >= 40 && s.body.length <= 400, `${layout}/${s.id}: body fits (${s.body.length})`);
    }
  }
  assert.ok(TOUR_STEPS.length >= 6, "a tour of the whole window has at least 6 stops");
});

test("every tour anchor exists in index.html", () => {
  // A renamed element would otherwise skip its step silently, forever.
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  for (const s of TOUR_STEPS) {
    assert.ok(html.includes(`id="${s.target}"`), `${s.id}: #${s.target} missing from index.html`);
  }
});

test("placement prefers the asked side when it fits", () => {
  const anchor = { left: 100, top: 100, width: 50, height: 20 };
  const tip = { width: 200, height: 100 };
  const vp = { width: 1000, height: 800 };
  const p = tourPlacement(anchor, tip, vp, "right");
  assert.equal(p.side, "right");
  assert.equal(p.left, 100 + 50 + 12);
  // Vertically centered on the anchor.
  assert.equal(p.top, 100 + 10 - 50);
});

test("placement falls over to the next side when the preferred one has no room", () => {
  const vp = { width: 500, height: 500 };
  const tip = { width: 200, height: 100 };
  // Anchor hugging the right edge: right can't fit, left can.
  const p = tourPlacement({ left: 420, top: 200, width: 60, height: 20 }, tip, vp, "right");
  assert.equal(p.side, "left");
  // Anchor filling the whole width: neither side fits — below wins.
  const q = tourPlacement({ left: 0, top: 0, width: 500, height: 30 }, tip, vp, "right");
  assert.equal(q.side, "bottom");
});

test("placement clamps into the viewport even when nothing fits", () => {
  const vp = { width: 300, height: 200 };
  const tip = { width: 280, height: 180 };
  const p = tourPlacement({ left: 0, top: 0, width: 300, height: 200 }, tip, vp, "right");
  assert.ok(p.left >= 0 && p.left + tip.width <= vp.width + 8, "horizontal clamp");
  assert.ok(p.top >= 0 && p.top + tip.height <= vp.height + 8, "vertical clamp");
});

test("browser branch publishes every global app.js reads", () => {
  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "tour.js"), "utf8");
  const sandbox = { window: {} };
  vm.createContext(sandbox);
  vm.runInContext(src, sandbox);
  for (const name of ["TOUR_STEPS", "windowTourSteps", "itemTourSteps", "tourPlacement"]) {
    assert.ok(name in sandbox.window, `${name} must be published to window`);
  }
});

test("the item tour resolves per view, with anchors the item view really renders", () => {
  const { ITEM_TOUR_STEPS, itemTourSteps } = require("../dist/tour.js");
  const APP = require("node:fs").readFileSync(require("node:path").join(__dirname, "..", "dist", "app.js"), "utf8");
  const ids = new Set();
  for (const s of ITEM_TOUR_STEPS) {
    assert.ok(s.id && !ids.has(s.id), `duplicate/missing id ${s.id}`);
    ids.add(s.id);
  }
  for (const view of ["workbench", "tabs"]) {
    const steps = itemTourSteps(view);
    assert.ok(steps.length >= 6, `${view}: a tour of an item has at least 6 stops`);
    for (const s of steps) {
      assert.equal(typeof s.selector, "string", `${view}/${s.id}: one selector`);
      assert.equal(typeof s.body, "string", `${view}/${s.id}: one body`);
      assert.ok(s.title.length <= 40 && s.body.length >= 40 && s.body.length <= 400, `${view}/${s.id} fits`);
      // Every class the selector names is one app.js gives an element.
      for (const cls of s.selector.match(/\.[\w-]+/g) || []) {
        assert.ok(APP.includes(cls.slice(1)), `${view}/${s.id}: app.js never renders ${cls}`);
      }
    }
  }
  // The documents step points at the outline in one view and the tab row in the other.
  assert.equal(itemTourSteps("workbench").find((s) => s.id === "item-documents").selector, ".wf-outline");
  assert.equal(itemTourSteps("tabs").find((s) => s.id === "item-documents").selector, ".wf-subbar");
});

test("the window tour speaks the layout: rail buttons, one-at-a-time panels, drawers", () => {
  const body = (layout, id) => windowTourSteps(layout).find((s) => s.id === id).body;
  assert.match(body("classic", "workspaces"), /⌘N creates one/);
  assert.match(body("rail", "workspaces"), /button on the rail/);
  assert.match(body("flyout", "workspaces"), /button on the rail/, "Flyout falls back to Rail's text");
  assert.match(body("rail", "files"), /one panel at a time/);
  assert.match(body("flyout", "files"), /drawer over the terminals/);
  assert.match(body("flyout", "sessions"), /slides out from the rail/);
  assert.match(body("classic", "inspector"), /beside Files/);
});
