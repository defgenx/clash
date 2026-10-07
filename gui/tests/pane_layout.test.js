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
  splitRoot,
  closePane,
  canonical,
  splitPane,
  halfAt,
  dropTarget,
  dropHintRect,
  dropOnPane,
  dropOnEdge,
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

test("every point of a pane picks a half, by its diagonals (iTerm2)", () => {
  assert.equal(halfAt(5, 50, 100, 100), "left");
  assert.equal(halfAt(95, 50, 100, 100), "right");
  assert.equal(halfAt(50, 3, 100, 100), "top");
  assert.equal(halfAt(50, 99, 100, 100), "bottom");
  // Far from any edge, not an "edge band": still a half.
  assert.equal(halfAt(30, 45, 100, 100), "left");
  // Relative to the pane's size: a wide pane's left third is still "left".
  assert.equal(halfAt(150, 50, 1000, 200), "left");
  // The center keeps whatever was there — nothing yet means nothing.
  assert.equal(halfAt(50, 50, 100, 100), null);
  assert.equal(halfAt(50, 50, 100, 100, "top"), "top");
  // Hysteresis: just across the diagonal does not flip yet; well across does.
  assert.equal(halfAt(20, 18, 100, 100, "left"), "left");
  assert.equal(halfAt(30, 5, 100, 100, "left"), "top");
});

const twoCols = {
  width: 1000,
  height: 600,
  panes: [
    { p: 0, left: 0, top: 0, width: 500, height: 600, titleHeight: 16 },
    { p: 1, left: 500, top: 0, width: 500, height: 600, titleHeight: 16 },
  ],
};

test("a drop target is a half of the pane under the cursor", () => {
  assert.deepEqual(dropTarget(twoCols, 250, 550), { kind: "pane", target: 0, zone: "bottom" });
  assert.deepEqual(dropTarget(twoCols, 600, 300), { kind: "pane", target: 1, zone: "left" });
  // The previous half sticks only within the same pane: dead center of pane 1
  // after pane 0's top half is no half at all, but keeps pane 1's own.
  assert.equal(dropTarget(twoCols, 750, 300, { kind: "pane", target: 0, zone: "top" }), null);
  const own = { kind: "pane", target: 1, zone: "top" };
  assert.deepEqual(dropTarget(twoCols, 750, 300, own), own);
  assert.equal(dropTarget(twoCols, 2000, 300), null);
});

test("an empty pane is filled and a title bar swaps", () => {
  const g = { ...twoCols, panes: [{ ...twoCols.panes[0], empty: true }, twoCols.panes[1]] };
  assert.deepEqual(dropTarget(g, 250, 300), { kind: "pane", target: 0, zone: "center" });
  assert.deepEqual(dropTarget(twoCols, 750, 8), { kind: "pane", target: 1, zone: "center" });
});

test("the area's outer band spans a side only where that differs from a pane split", () => {
  // Under two columns: the bottom band spans both.
  assert.deepEqual(dropTarget(twoCols, 250, 590), { kind: "edge", side: "bottom" });
  // The left column already spans the full height: its left half is the same split.
  assert.deepEqual(dropTarget(twoCols, 5, 300), { kind: "pane", target: 0, zone: "left" });
  // A sole pane has nothing to span.
  const one = { width: 100, height: 100, panes: [{ p: 0, left: 0, top: 0, width: 100, height: 100 }] };
  assert.deepEqual(dropTarget(one, 50, 98), { kind: "pane", target: 0, zone: "bottom" });
});

test("the hint is the box the dropped tab would occupy", () => {
  assert.deepEqual(dropHintRect(twoCols, { kind: "pane", target: 1, zone: "bottom" }), {
    left: 500, top: 300, width: 500, height: 300,
  });
  assert.deepEqual(dropHintRect(twoCols, { kind: "edge", side: "bottom" }), {
    left: 0, top: 300, width: 1000, height: 300,
  });
  assert.deepEqual(dropHintRect(twoCols, { kind: "pane", target: 0, zone: "center" }), {
    left: 0, top: 0, width: 500, height: 600,
  });
  assert.equal(dropHintRect(twoCols, null), null);
});

test("pane order follows reading order after every mutation", () => {
  // (A / C) | B with C created last: C reads second.
  const l = { dir: "row", kids: [{ dir: "col", kids: [{ p: 0 }, { p: 2 }], fracs: [1, 1] }, { p: 1 }], fracs: [1, 1] };
  const c = canonical(["a", "b", "c"], l, 2);
  assert.deepEqual(c.panes, ["a", "c", "b"]);
  assert.deepEqual(leaves(c.layout), [0, 1, 2]);
  assert.equal(c.focused, 1);
  // Splitting the first of two columns: the new pane is second, and focused.
  const s = splitPane(["a", "b"], defaultLayout(2), 0, "bottom");
  assert.deepEqual(s.panes, ["a", null, "b"]);
  assert.equal(s.focused, 1);
});

test("columns fold into rows by dragging, as in iTerm", () => {
  // A | B | C → drag B under A, then C under B: one stack.
  const three = { dir: "row", kids: [{ p: 0 }, { p: 1 }, { p: 2 }], fracs: [1, 1, 1] };
  const r1 = dropOnPane(["a", "b", "c"], three, "b", 0, "bottom");
  assert.deepEqual(r1.panes, ["a", "b", "c"]);
  const r2 = dropOnPane(r1.panes, r1.layout, "c", 1, "bottom");
  assert.equal(r2.layout.dir, "col");
  assert.deepEqual(leaves(r2.layout), [0, 1, 2]);
  assert.deepEqual(r2.panes, ["a", "b", "c"]);
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
});

test("a root split spans every column", () => {
  const cols = defaultLayout(2); // A | B
  assert.deepEqual(splitRoot(cols, 2, "bottom"), {
    dir: "col",
    kids: [cols, { p: 2 }],
    fracs: [1, 1],
  });
  // Same direction as the root: flattened, the new pane still takes half.
  const r = splitRoot(cols, 2, "right");
  assert.deepEqual(r.kids.map((k) => k.p), [0, 1, 2]);
  assert.equal(r.fracs[2], r.fracs[0] + r.fracs[1]);
});

test("dropping a tab on the bottom edge puts it under all the columns", () => {
  const three = { dir: "row", kids: [{ p: 0 }, { p: 1 }, { p: 2 }], fracs: [1, 1, 1] };
  const r = dropOnEdge(["a", "b", "c"], three, "d", "bottom");
  assert.deepEqual(r.panes, ["a", "b", "c", "d"]);
  assert.deepEqual(r.layout, { dir: "col", kids: [three, { p: 3 }], fracs: [1, 1] });
  assert.equal(r.focused, 3);
  // A tab already in a pane moves: its column closes.
  const m = dropOnEdge(["a", "b", "c"], three, "b", "bottom");
  assert.deepEqual(m.panes, ["a", "c", "b"]);
  assert.ok(validLayout(m.layout, 3));
  assert.equal(m.layout.dir, "col");
  assert.deepEqual(m.layout.kids[0].kids.map((k) => k.p), [0, 1]);
  assert.equal(m.focused, 2);
  // The sole pane onto its own edge changes nothing.
  assert.equal(dropOnEdge(["a"], { p: 0 }, "a", "bottom"), null);
});

test("closing a pane gives its space to its siblings", () => {
  const l = splitLeaf(defaultLayout(2), 1, 2, "bottom"); // A | (B / C)
  const r = closePane(["a", "b", "c"], l, 2, 1);
  assert.deepEqual(r.panes, ["a", "c"]);
  assert.deepEqual(r.layout, { dir: "row", kids: [{ p: 0 }, { p: 1 }], fracs: [1, 1] });
  assert.equal(r.focused, 1);
  // The sole pane is emptied, not removed.
  assert.deepEqual(closePane(["a"], { p: 0 }, 0, 0), { panes: [null], layout: { p: 0 }, focused: 0 });
});
