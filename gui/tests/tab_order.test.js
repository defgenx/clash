// node --test gui/tests/ — tab strip reordering (gui/dist/tab-order.js).
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { dropBefore, moveKey, applyOrder } = require("../dist/tab-order.js");

const strip = [
  { id: "a", left: 0, width: 100 },
  { id: "b", left: 100, width: 100 },
  { id: "c", left: 200, width: 100 },
];

test("a drop lands in front of the first tab whose midpoint is to its right", () => {
  assert.equal(dropBefore(strip, 10), "a");
  assert.equal(dropBefore(strip, 49), "a");
  assert.equal(dropBefore(strip, 51), "b");
  assert.equal(dropBefore(strip, 260), null);
  assert.equal(dropBefore([], 10), null);
});

test("moving a key in front of another, or to the end", () => {
  assert.deepEqual(moveKey(["a", "b", "c"], "c", "a"), ["c", "a", "b"]);
  assert.deepEqual(moveKey(["a", "b", "c"], "a", "c"), ["b", "a", "c"]);
  assert.deepEqual(moveKey(["a", "b", "c"], "a", null), ["b", "c", "a"]);
});

test("a drop where the tab already is changes nothing", () => {
  assert.equal(moveKey(["a", "b", "c"], "b", "b"), null); // on itself
  assert.equal(moveKey(["a", "b", "c"], "b", "c"), null); // just right of itself
  assert.equal(moveKey(["a", "b", "c"], "c", null), null); // already last
  assert.equal(moveKey(["a", "b"], "x", "a"), null); // not a tab
});

test("tabs hidden in other workspaces keep their places", () => {
  // h1/h2 belong to another workspace; the strip shows a, b, c.
  assert.deepEqual(moveKey(["a", "h1", "b", "h2", "c"], "c", "b"), ["a", "h1", "c", "b", "h2"]);
});

test("the persisted order goes first, newer or renamed tabs keep theirs after", () => {
  assert.deepEqual(applyOrder(["a", "b", "c", "d"], ["c", "a"]), ["c", "a", "b", "d"]);
  assert.deepEqual(applyOrder(["a", "b"], ["gone", "b", "b"]), ["b", "a"]);
  assert.deepEqual(applyOrder(["a", "b"], null), ["a", "b"]);
});

test("index.html loads tab-order.js before app.js", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  const mod = html.indexOf('src="tab-order.js"');
  assert.ok(mod > 0 && mod < html.indexOf('src="app.js"'));
});
