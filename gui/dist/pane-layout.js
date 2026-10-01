// Split-pane layout tree — pure, no DOM, no IPC.
//
// A workspace keeps its pane *contents* in the flat `w.panes` list (focus,
// zoom, restore and every open path index into it) and its pane *geometry* in
// `w.layout`, a tree whose leaves name those indices:
//
//   leaf  = { p: <index into w.panes> }
//   split = { dir: "row" | "col", kids: [node, …], fracs: [number, …] }
//
// `row` lays its kids side by side, `col` stacks them; `fracs` are flex-grow
// shares. Invariant: the leaves are exactly 0..panes.length-1, once each.
// `ensureLayout` restores it from anything (a missing, stale or hand-edited
// layout falls back to the balanced grid), so a mutation site that forgets to
// update the tree degrades the arrangement, never the panes.
//
// Loaded before app.js (plain script) and tested by gui/tests/pane_layout.test.js.
(function (global) {
  const SIDES = ["left", "right", "top", "bottom"];

  const isLeaf = (n) => !!n && typeof n.p === "number";
  const leaf = (p) => ({ p });
  const goodFracs = (a, len) =>
    Array.isArray(a) && a.length === len && a.every((f) => Number.isFinite(f) && f > 0);
  const sum = (a) => a.reduce((x, y) => x + y, 0);

  /// The balanced grid clash used before split trees: columns first, rows
  /// follow (2 → 2x1, 3-4 → 2x2, 5-6 → 3x2, …), the short last row's panes
  /// sharing its width. Legacy `colFracs`/`rowFracs` are kept where they fit.
  function defaultLayout(n, colFracs, rowFracs) {
    if (n <= 1) return leaf(0);
    const cols = Math.ceil(Math.sqrt(n));
    const rows = Math.ceil(n / cols);
    const rowNodes = [];
    for (let r = 0; r < rows; r++) {
      const kids = [];
      for (let c = 0; c < cols && r * cols + c < n; c++) kids.push(leaf(r * cols + c));
      const fracs = goodFracs(colFracs, kids.length) ? [...colFracs] : kids.map(() => 1);
      rowNodes.push(kids.length === 1 ? kids[0] : { dir: "row", kids, fracs });
    }
    if (rows === 1) return rowNodes[0];
    const fracs = goodFracs(rowFracs, rows) ? [...rowFracs] : rowNodes.map(() => 1);
    return { dir: "col", kids: rowNodes, fracs };
  }

  /// Leaf indices in document order.
  function leaves(node, out = []) {
    if (isLeaf(node)) out.push(node.p);
    else if (node && Array.isArray(node.kids)) node.kids.forEach((k) => leaves(k, out));
    return out;
  }

  function wellFormed(node) {
    if (isLeaf(node)) return Number.isInteger(node.p);
    return (
      !!node &&
      (node.dir === "row" || node.dir === "col") &&
      Array.isArray(node.kids) &&
      node.kids.length >= 2 &&
      goodFracs(node.fracs, node.kids.length) &&
      node.kids.every(wellFormed)
    );
  }

  function validLayout(node, n) {
    if (!wellFormed(node)) return false;
    const ps = leaves(node).sort((a, b) => a - b);
    return ps.length === n && ps.every((p, i) => p === i);
  }

  /// Canonical copy: single-kid splits collapse into their kid, a split nested
  /// in a split of the same direction is flattened into it (its kids share its
  /// fraction proportionally), and `null` kids (removed leaves) are dropped.
  function normalize(node) {
    if (node == null) return null;
    if (isLeaf(node)) return leaf(node.p);
    const kids = [];
    const fracs = [];
    node.kids.forEach((k, i) => {
      const c = normalize(k);
      if (!c) return;
      const f = node.fracs[i];
      if (!isLeaf(c) && c.dir === node.dir) {
        const tot = sum(c.fracs);
        c.kids.forEach((g, j) => {
          kids.push(g);
          fracs.push((f * c.fracs[j]) / tot);
        });
      } else {
        kids.push(c);
        fracs.push(f);
      }
    });
    if (!kids.length) return null;
    if (kids.length === 1) return kids[0];
    return { dir: node.dir, kids, fracs };
  }

  function ensureLayout(layout, n, colFracs, rowFracs) {
    return validLayout(layout, n) ? normalize(layout) : defaultLayout(n, colFracs, rowFracs);
  }

  function mapLeaves(node, fn) {
    if (isLeaf(node)) return fn(node);
    return { dir: node.dir, kids: node.kids.map((k) => mapLeaves(k, fn)), fracs: [...node.fracs] };
  }

  /// Put the new leaf `newIdx` on `side` of leaf `target`, splitting the
  /// target's space in half. Splitting along the direction of the target's
  /// parent inserts a sibling there instead of nesting (via `normalize`).
  function splitLeaf(layout, target, newIdx, side) {
    if (!SIDES.includes(side)) throw new Error(`bad side: ${side}`);
    const dir = side === "left" || side === "right" ? "row" : "col";
    const before = side === "left" || side === "top";
    return normalize(
      mapLeaves(layout, (l) =>
        l.p !== target
          ? leaf(l.p)
          : { dir, kids: before ? [leaf(newIdx), leaf(l.p)] : [leaf(l.p), leaf(newIdx)], fracs: [1, 1] }
      )
    );
  }

  /// Drop leaf `idx` (its siblings absorb its space) and renumber the leaves
  /// after it, mirroring `w.panes.splice(idx, 1)`.
  function removeLeaf(layout, idx) {
    const prune = (node) => {
      if (isLeaf(node)) return node.p === idx ? null : leaf(node.p > idx ? node.p - 1 : node.p);
      return { dir: node.dir, kids: node.kids.map(prune), fracs: [...node.fracs] };
    };
    return normalize(prune(layout)) || leaf(0);
  }

  /// Which drop zone a point falls in, relative to a pane's box: the nearest
  /// edge when within the outer quarter of the pane, else its center.
  function dropZone(x, y, width, height) {
    if (!(width > 0 && height > 0)) return "center";
    const fx = x / width;
    const fy = y / height;
    const BAND = 0.25;
    const d = { left: fx, right: 1 - fx, top: fy, bottom: 1 - fy };
    let best = "center";
    let bestD = BAND;
    for (const side of SIDES) {
      if (d[side] < bestD) {
        best = side;
        bestD = d[side];
      }
    }
    return best;
  }

  /// Drop tab `sid` on pane `target` in `zone`. Center replaces the target's
  /// content (swapping with the pane `sid` came from, so nothing is evicted
  /// from view); an edge creates a new pane there, and a tab that was already
  /// in a pane moves rather than being shown twice. Returns new copies, or
  /// `null` when the drop changes nothing.
  function dropOnPane(panes, layout, sid, target, zone) {
    if (!(target >= 0 && target < panes.length)) return null;
    const from = panes.indexOf(sid);
    const next = [...panes];
    const base = ensureLayout(layout, panes.length);
    if (zone === "center") {
      if (from === target) return null;
      if (from >= 0) next[from] = next[target];
      next[target] = sid;
      return { panes: next, layout: base, focused: target };
    }
    // A pane dropped on its own edge, or the sole pane on any edge: nothing to
    // split relative to.
    if (from === target) return null;
    let tree = splitLeaf(base, target, next.length, zone);
    next.push(sid);
    let focused = next.length - 1;
    if (from >= 0) {
      next.splice(from, 1);
      tree = removeLeaf(tree, from);
      if (from < focused) focused--;
    }
    return { panes: next, layout: tree, focused };
  }

  const api = {
    defaultLayout,
    validLayout,
    ensureLayout,
    leaves,
    splitLeaf,
    removeLeaf,
    dropZone,
    dropOnPane,
  };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else global.PaneLayout = api;
})(typeof window !== "undefined" ? window : globalThis);
