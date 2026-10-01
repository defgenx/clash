// node --test gui/tests/ — the split-pane layout tree behind drag-to-split.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  defaultLayout,
  validLayout,
  ensureLayout,
  leaves,
  splitLeaf,
  removeLeaf,
  dropZone,
  dropOnPane,
} = require("../dist/pane-layout.js");

test("the default layout reproduces the balanced grid", () => {
  assert.deepEqual(defaultLayout(1), { p: 0 });
  assert.deepEqual(defaultLayout(2), { dir: "row", kids: [{ p: 0 }, { p: 1 }], fracs: [1, 1] });
  // 3 → 2x2 with the third pane taking the whole bottom row.
  assert.deepEqual(defaultLayout(3), {
    dir: "col",
    kids: [{ dir: "row", kids: [{ p: 0 }, { p: 1 }], fracs: [1, 1] }, { p: 2 }],
    fracs: [1, 1],
  });
  for (let n = 1; n <= 10; n++) assert.ok(validLayout(defaultLayout(n), n), `n=${n}`);
});

test("legacy grid fractions survive the migration where they fit", () => {
  const l = defaultLayout(4, [2, 1], [1, 3]);
  assert.deepEqual(l.fracs, [1, 3]);
  assert.deepEqual(l.kids[0].fracs, [2, 1]);
  assert.deepEqual(l.kids[1].fracs, [2, 1]);
});

test("a layout that does not match the pane count falls back to the grid", () => {
  const two = defaultLayout(2);
  assert.deepEqual(ensureLayout(two, 3), defaultLayout(3));
  assert.deepEqual(ensureLayout(undefined, 2), two);
  assert.deepEqual(ensureLayout({ dir: "row", kids: [{ p: 0 }, { p: 0 }], fracs: [1, 1] }, 2), two);
  assert.deepEqual(ensureLayout({ dir: "row", kids: [{ p: 0 }, { p: 1 }], fracs: [1, -1] }, 2), two);
  // A valid custom layout is kept.
  const custom = { dir: "col", kids: [{ p: 1 }, { p: 0 }], fracs: [3, 1] };
  assert.deepEqual(ensureLayout(custom, 2), custom);
});

test("splitting along the parent's direction inserts a sibling with half the share", () => {
  const row = { dir: "row", kids: [{ p: 0 }, { p: 1 }], fracs: [2, 2] };
  assert.deepEqual(splitLeaf(row, 1, 2, "right"), {
    dir: "row",
    kids: [{ p: 0 }, { p: 1 }, { p: 2 }],
    fracs: [2, 1, 1],
  });
  assert.deepEqual(splitLeaf(row, 0, 2, "left"), {
    dir: "row",
    kids: [{ p: 2 }, { p: 0 }, { p: 1 }],
    fracs: [1, 1, 2],
  });
});

test("splitting across the parent's direction nests", () => {
  const row = { dir: "row", kids: [{ p: 0 }, { p: 1 }], fracs: [1, 1] };
  assert.deepEqual(splitLeaf(row, 1, 2, "bottom"), {
    dir: "row",
    kids: [{ p: 0 }, { dir: "col", kids: [{ p: 1 }, { p: 2 }], fracs: [1, 1] }],
    fracs: [1, 1],
  });
  assert.deepEqual(splitLeaf({ p: 0 }, 0, 1, "top"), {
    dir: "col",
    kids: [{ p: 1 }, { p: 0 }],
    fracs: [1, 1],
  });
});

test("removing a leaf renumbers the rest and collapses lone kids", () => {
  const l = {
    dir: "row",
    kids: [{ p: 0 }, { dir: "col", kids: [{ p: 1 }, { p: 2 }], fracs: [1, 1] }],
    fracs: [1, 1],
  };
  assert.deepEqual(removeLeaf(l, 1), { dir: "row", kids: [{ p: 0 }, { p: 1 }], fracs: [1, 1] });
  assert.deepEqual(removeLeaf(removeLeaf(l, 0), 0), { p: 0 });
  // A collapse that lands a split inside a same-direction parent flattens.
  const nested = {
    dir: "row",
    kids: [
      { p: 0 },
      { dir: "col", kids: [{ dir: "row", kids: [{ p: 1 }, { p: 2 }], fracs: [1, 1] }, { p: 3 }], fracs: [1, 1] },
    ],
    fracs: [1, 1],
  };
  assert.deepEqual(removeLeaf(nested, 3), {
    dir: "row",
    kids: [{ p: 0 }, { p: 1 }, { p: 2 }],
    fracs: [1, 0.5, 0.5],
  });
});

test("drop zones: outer quarter is an edge, the rest is the center", () => {
  assert.equal(dropZone(50, 50, 100, 100), "center");
  assert.equal(dropZone(5, 50, 100, 100), "left");
  assert.equal(dropZone(95, 50, 100, 100), "right");
  assert.equal(dropZone(50, 3, 100, 100), "top");
  assert.equal(dropZone(50, 99, 100, 100), "bottom");
  // Corner: the nearer edge wins.
  assert.equal(dropZone(2, 10, 100, 100), "left");
  assert.equal(dropZone(10, 2, 100, 100), "top");
  assert.equal(dropZone(0, 0, 0, 0), "center");
});

test("dropping a tab on an edge opens it in a new pane there", () => {
  const r = dropOnPane(["a"], { p: 0 }, "b", 0, "right");
  assert.deepEqual(r.panes, ["a", "b"]);
  assert.deepEqual(r.layout, { dir: "row", kids: [{ p: 0 }, { p: 1 }], fracs: [1, 1] });
  assert.equal(r.focused, 1);
});

test("dropping a pane's tab on another pane's edge moves it, never duplicates it", () => {
  const layout = { dir: "row", kids: [{ p: 0 }, { p: 1 }, { p: 2 }], fracs: [1, 1, 1] };
  const r = dropOnPane(["a", "b", "c"], layout, "a", 2, "bottom");
  assert.deepEqual(r.panes, ["b", "c", "a"]);
  assert.deepEqual(r.layout, {
    dir: "row",
    kids: [{ p: 0 }, { dir: "col", kids: [{ p: 1 }, { p: 2 }], fracs: [1, 1] }],
    fracs: [1, 1],
  });
  assert.equal(r.focused, 2);
  assert.ok(validLayout(r.layout, r.panes.length));
});

test("dropping on a pane's center swaps or fills it", () => {
  const layout = defaultLayout(2);
  assert.deepEqual(dropOnPane(["a", "b"], layout, "b", 0, "center"), {
    panes: ["b", "a"],
    layout,
    focused: 0,
  });
  assert.deepEqual(dropOnPane(["a", null], layout, "c", 1, "center").panes, ["a", "c"]);
});

test("drops that change nothing report null", () => {
  assert.equal(dropOnPane(["a"], { p: 0 }, "a", 0, "left"), null);
  assert.equal(dropOnPane(["a", "b"], defaultLayout(2), "a", 0, "center"), null);
  assert.equal(dropOnPane(["a"], { p: 0 }, "b", 3, "left"), null);
});

test("leaves are listed in document order", () => {
  assert.deepEqual(leaves(splitLeaf(defaultLayout(2), 0, 2, "left")), [2, 0, 1]);
});

const fs = require("node:fs");
const path = require("node:path");
const DIST = path.join(__dirname, "..", "dist");

test("index.html loads pane-layout.js before app.js", () => {
  const html = fs.readFileSync(path.join(DIST, "index.html"), "utf8");
  const mod = html.indexOf('src="pane-layout.js"');
  assert.ok(mod > 0 && mod < html.indexOf('src="app.js"'));
});

test("a drag in flight is never repainted out from under itself", () => {
  // Repainting detaches the dragged tab/title and the drop overlays, which
  // cancels the drag in WebKit; the session poll repaints the strip every tick.
  const app = fs.readFileSync(path.join(DIST, "app.js"), "utf8");
  for (const fn of ["function renderTabs() {", "function renderPanes() {"]) {
    const body = app.slice(app.indexOf(fn), app.indexOf(fn) + 400);
    assert.match(body, /if \(paneDrag\) \{\s*paneDrag\.dirty = true;\s*return;/, fn);
  }
  // The payload must not be text: a drop that misses the overlays would type
  // it into the terminal underneath.
  const src = app.slice(app.indexOf("function makePaneDragSource"));
  assert.match(src.slice(0, 800), /setData\(PANE_DRAG_MIME, sid\)/);
  assert.doesNotMatch(src.slice(0, 800), /setData\("text\/plain"/);
});
