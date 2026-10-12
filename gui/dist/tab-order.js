// Tab strip order — pure, no DOM, no IPC.
//
// The strip lists `state.open` in its Map order, which holds the tabs of every
// workspace; the strip shows only the active workspace's. Reordering therefore
// moves a key within the full list, relative to a *visible* neighbour, and the
// hidden tabs keep their places.
//
// Loaded before app.js (plain script) and tested by gui/tests/tab_order.test.js.
(function (global) {
  /// The tab a drop at `x` lands in front of: the first tab whose midpoint is
  /// right of `x`, or null for the end of the strip. `tabs` is the visible
  /// strip, left to right, as [{ id, left, width }].
  function dropBefore(tabs, x) {
    const hit = tabs.find((t) => x < t.left + t.width / 2);
    return hit ? hit.id : null;
  }

  /// `keys` with `id` moved in front of `before` (null = to the end), or null
  /// when that is where it already is.
  function moveKey(keys, id, before) {
    if (!keys.includes(id) || before === id) return null;
    const rest = keys.filter((k) => k !== id);
    const at = before === null ? rest.length : rest.indexOf(before);
    if (at < 0) return null;
    const out = [...rest.slice(0, at), id, ...rest.slice(at)];
    return out.every((k, i) => k === keys[i]) ? null : out;
  }

  /// `keys` in the persisted order `saved`: the saved ones first, in that
  /// order, then the rest as they are (tabs opened since, or whose id moved).
  function applyOrder(keys, saved) {
    const have = new Set(keys);
    const first = [...new Set(saved || [])].filter((k) => have.has(k));
    const placed = new Set(first);
    return [...first, ...keys.filter((k) => !placed.has(k))];
  }

  const api = { dropBefore, moveKey, applyOrder };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else global.TabOrder = api;
})(typeof window !== "undefined" ? window : globalThis);
