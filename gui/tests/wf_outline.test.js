// node --test gui/tests/ — the workflow item views' pure model (gui/dist/wf-outline.js).
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const O = require("../dist/wf-outline.js");

// The sub-view list app.js builds for a fully worked item, labels as shown.
const FULL = [
  ["plan", "Plan"],
  ["revisions", "◫ Revisions"],
  ["review", "Change requests"],
  ["handoff", "↩ Hand-off"],
  ["agentReview", "Agent reviews (2)"],
  ["explainPlan", "◫ Plan explained"],
  ["explainDiff", "◫ Changes explained"],
  ["drift", "⇄ Plan vs changes"],
  ["diff", "Diff 💬3"],
  ["history", "Timeline (5)"],
  ["settings", "⚙ Settings"],
];

test("every sub-view lands in exactly one place, grouped by what it is", () => {
  const o = O.wfOutline(FULL, "diff");
  const placed = o.groups.flatMap((g) => g.items.map((i) => i.id)).concat(o.settings ? ["settings"] : []);
  assert.deepEqual([...placed].sort(), FULL.map(([id]) => id).sort());
  assert.deepEqual(
    o.groups.map((g) => [g.id, g.items.map((i) => i.id)]),
    [
      ["build", ["plan", "diff"]],
      ["feedback", ["review", "agentReview", "handoff"]],
      ["explained", ["explainPlan", "explainDiff", "drift"]],
      ["history", ["revisions", "history"]],
    ]
  );
  assert.equal(o.groups[0].items.find((i) => i.id === "diff").active, true);
});

test("labels lose their glyphs, counts become badges, ·empty a flag", () => {
  assert.deepEqual(O.outlineEntry("agentReview", "Agent reviews (2)"), {
    id: "agentReview",
    text: "Agent reviews",
    badge: 2,
    hot: false,
    empty: false,
  });
  assert.deepEqual(O.outlineEntry("diff", "Diff 💬3"), { id: "diff", text: "Diff", badge: 3, hot: true, empty: false });
  assert.deepEqual(O.outlineEntry("plan", "Plan ·empty"), { id: "plan", text: "Plan", badge: 0, hot: false, empty: true });
  assert.equal(O.outlineEntry("drift", "⇄ Plan vs changes").text, "Plan vs changes");
});

test("a review-only item has no plan entries and no empty groups", () => {
  const o = O.wfOutline(
    [
      ["review", "Change requests ·empty"],
      ["diff", "Diff"],
      ["history", "Timeline"],
      ["settings", "⚙ Settings"],
    ],
    "diff"
  );
  assert.deepEqual(o.groups.map((g) => g.id), ["build", "feedback", "history"]);
  assert.equal(o.groups[1].items[0].empty, true);
});

test("an id no group names is still reachable", () => {
  const o = O.wfOutline([["plan", "Plan"], ["future", "Something new"], ["settings", "⚙ Settings"]], "future");
  const all = o.groups.flatMap((g) => g.items);
  assert.ok(all.some((i) => i.id === "future" && i.active));
});

test("views fall back to the default and the More menu keeps order", () => {
  assert.equal(O.itemViewOf("nope").id, O.DEFAULT_ITEM_VIEW);
  assert.equal(O.itemViewOf("tabs").id, "tabs");
  assert.deepEqual(
    O.moreMenuItems([{ text: "↩ Move back to…" }, { text: " " }, { text: "Abandon", danger: true }]),
    [
      { index: 0, label: "↩ Move back to…", danger: false },
      { index: 2, label: "Abandon", danger: true },
    ]
  );
});

test("the setting offers every view and the script loads before app.js", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  const at = html.indexOf('id="set-wf-item-view"');
  assert.ok(at > 0, "Settings has the item view choice");
  const select = html.slice(at, html.indexOf("</select>", at));
  for (const v of O.WF_ITEM_VIEWS) assert.ok(select.includes(`value="${v.id}"`), v.id);
  assert.ok(html.indexOf("wf-outline.js") < html.indexOf('src="app.js"'));
  const sandbox = {};
  sandbox.globalThis = sandbox;
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "..", "dist", "wf-outline.js"), "utf8"), sandbox);
  assert.equal(typeof sandbox.WfOutline.wfOutline, "function");
});
