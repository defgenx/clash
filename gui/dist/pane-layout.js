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

  /// Put the new leaf `newIdx` along `side` of the whole layout, spanning it
  /// (a bottom pane under every column). It takes half, like a pane split.
  function splitRoot(layout, newIdx, side) {
    if (!SIDES.includes(side)) throw new Error(`bad side: ${side}`);
    const dir = side === "left" || side === "right" ? "row" : "col";
    const before = side === "left" || side === "top";
    const kids = before ? [leaf(newIdx), layout] : [layout, leaf(newIdx)];
    return normalize({ dir, kids, fracs: [1, 1] });
  }

  /// Renumber the leaves into reading order (the tree's document order), so a
  /// pane's index — what ⌘⌥←/→ cycles through and what is persisted — follows
  /// the screen instead of the order panes happened to be created in.
  function canonical(panes, layout, focused) {
    const order = leaves(layout);
    const at = new Map(order.map((p, i) => [p, i]));
    return {
      panes: order.map((p) => panes[p]),
      layout: mapLeaves(layout, (l) => leaf(at.get(l.p))),
      focused: at.has(focused) ? at.get(focused) : 0,
    };
  }

  /// Split an empty pane off pane `target` on `side`, and focus it.
  function splitPane(panes, layout, target, side) {
    const tree = splitLeaf(ensureLayout(layout, panes.length), target, panes.length, side);
    return canonical([...panes, null], tree, panes.length);
  }

  /// Close pane `idx`: its siblings absorb its space. The sole pane is only
  /// emptied, never removed. `focused` follows the pane it named.
  function closePane(panes, layout, focused, idx) {
    if (!(idx >= 0 && idx < panes.length)) return { panes: [...panes], layout, focused };
    if (panes.length <= 1) return { panes: [null], layout: leaf(0), focused: 0 };
    const next = [...panes];
    next.splice(idx, 1);
    const tree = removeLeaf(ensureLayout(layout, panes.length), idx);
    const f = focused > idx ? focused - 1 : Math.min(focused, next.length - 1);
    return canonical(next, tree, f);
  }

  const edgeDistance = (half, x, y, w, h) =>
    ({ left: x, right: w - x, top: y, bottom: h - y })[half] ?? Infinity;

  /// iTerm2's half picker (SplitSelectionView `updateAtPoint:`): the pane's
  /// diagonals cut it into four triangles and the point picks the one whose
  /// edge is nearest relative to the pane's size — every drop is a split, there
  /// is no "replace" middle. Near the center (both shares >= 0.4) it keeps
  /// `prev`, and it leaves `prev` only once the new edge is nearer by a margin,
  /// so the highlight does not flicker across a diagonal. `null` = no half yet.
  function halfAt(x, y, width, height, prev = null) {
    if (!(width > 0 && height > 0)) return prev;
    const h = x < width / 2 ? ["left", x / width, x] : ["right", (width - x) / width, width - x];
    const v = y < height / 2 ? ["top", y / height, y] : ["bottom", (height - y) / height, height - y];
    const [best, score, dist] = v[1] < h[1] ? v : h;
    if (best === prev || score >= 0.4) return prev;
    const hysteresis = Math.max(0.05 * Math.min(width, height), 8);
    return dist < edgeDistance(prev, x, y, width, height) - hysteresis ? best : prev;
  }

  /// Within this many px of the pane area's own edge, a drop spans that side.
  const AREA_EDGE_PX = 24;

  /// Where a drag at (x, y) would land, all coordinates relative to the pane
  /// area. `geom` = { width, height, panes: [{ p, left, top, width, height,
  /// empty, titleHeight }] }. Returns `{ kind: "pane", target, zone }`,
  /// `{ kind: "edge", side }`, or `null`; `prev` is the previous answer.
  /// An empty pane is filled and a title bar swaps (zone "center"); the outer
  /// band spans the area only where that differs from splitting the pane under
  /// the cursor (a pane already spanning that side would split the same way).
  function dropTarget(geom, x, y, prev = null) {
    const pane = geom.panes.find(
      (r) => x >= r.left && x < r.left + r.width && y >= r.top && y < r.top + r.height
    );
    if (!pane) return null;
    if (pane.empty || y - pane.top < (pane.titleHeight || 0)) {
      return { kind: "pane", target: pane.p, zone: "center" };
    }
    const spansX = pane.left <= 1 && pane.left + pane.width >= geom.width - 1;
    const spansY = pane.top <= 1 && pane.top + pane.height >= geom.height - 1;
    const near = { left: x, right: geom.width - x, top: y, bottom: geom.height - y };
    for (const side of SIDES) {
      const spans = side === "left" || side === "right" ? spansY : spansX;
      if (geom.panes.length > 1 && !spans && near[side] < AREA_EDGE_PX) return { kind: "edge", side };
    }
    const keep = prev && prev.kind === "pane" && prev.target === pane.p ? prev.zone : null;
    const zone = halfAt(x - pane.left, y - pane.top, pane.width, pane.height, keep === "center" ? null : keep);
    return zone ? { kind: "pane", target: pane.p, zone } : null;
  }

  /// The box to highlight for a `dropTarget` answer — the half (or the whole
  /// pane, or the half of the area) the dropped tab would occupy.
  function dropHintRect(geom, drop) {
    if (!drop) return null;
    const box =
      drop.kind === "edge"
        ? { left: 0, top: 0, width: geom.width, height: geom.height }
        : geom.panes.find((r) => r.p === drop.target);
    if (!box) return null;
    const side = drop.kind === "edge" ? drop.side : drop.zone;
    const { left, top, width, height } = box;
    switch (side) {
      case "left": return { left, top, width: width / 2, height };
      case "right": return { left: left + width / 2, top, width: width / 2, height };
      case "top": return { left, top, width, height: height / 2 };
      case "bottom": return { left, top: top + height / 2, width, height: height / 2 };
      default: return { left, top, width, height };
    }
  }

  /// Drop tab `sid` on pane `target` in `zone`. Center replaces the target's
  /// content (swapping with the pane `sid` came from, so nothing is evicted
  /// from view); a half splits the target there, iTerm's move — a tab that was
  /// already in a pane leaves it, and its siblings absorb the space. Returns
  /// new copies, or `null` when the drop changes nothing.
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
    // A pane dropped on its own half: nothing to split relative to.
    if (from === target) return null;
    let tree = splitLeaf(base, target, next.length, zone);
    next.push(sid);
    let focused = next.length - 1;
    if (from >= 0) {
      next.splice(from, 1);
      tree = removeLeaf(tree, from);
      if (from < focused) focused--;
    }
    return canonical(next, tree, focused);
  }

  /// Drop tab `sid` on the `side` edge of the whole pane area: a new pane
  /// spanning that side. A tab already in a pane moves there. `null` when it
  /// changes nothing (the sole pane moved onto its own edge).
  function dropOnEdge(panes, layout, sid, side) {
    const from = panes.indexOf(sid);
    if (from >= 0 && panes.length === 1) return null;
    const next = [...panes];
    let tree = splitRoot(ensureLayout(layout, panes.length), next.length, side);
    next.push(sid);
    let focused = next.length - 1;
    if (from >= 0) {
      next.splice(from, 1);
      tree = removeLeaf(tree, from);
      focused--;
    }
    return canonical(next, tree, focused);
  }

  const api = {
    defaultLayout,
    validLayout,
    ensureLayout,
    leaves,
    splitLeaf,
    splitRoot,
    removeLeaf,
    canonical,
    splitPane,
    closePane,
    halfAt,
    dropTarget,
    dropHintRect,
    dropOnPane,
    dropOnEdge,
  };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else global.PaneLayout = api;
})(typeof window !== "undefined" ? window : globalThis);
