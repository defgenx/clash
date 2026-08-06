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
const { TOUR_STEPS, tourPlacement } = require("../dist/tour.js");

test("steps are well-formed: unique ids, anchors, tooltip-sized prose", () => {
  const ids = new Set();
  for (const s of TOUR_STEPS) {
    assert.ok(s.id && !ids.has(s.id), `duplicate/missing id ${s.id}`);
    ids.add(s.id);
    assert.ok(s.target && typeof s.target === "string", `${s.id}: target element id`);
    assert.ok(s.title && s.title.length <= 40, `${s.id}: title fits a tooltip header`);
    assert.ok(s.body && s.body.length >= 40 && s.body.length <= 400, `${s.id}: body fits`);
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
  for (const name of ["TOUR_STEPS", "tourPlacement"]) {
    assert.ok(name in sandbox.window, `${name} must be published to window`);
  }
});
