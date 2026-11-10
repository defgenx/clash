// Item cockpit — the pure half of the context rail.
//
// In a wide item tab (Workbench view) a rail on the right keeps the item's
// state in view whatever tab is being read: the next step and its buttons
// (the action bar itself, moved there), the last round, and — what this
// module decides — the waves, the PRs and the item's agents. A narrow tab
// shows none of the cards: the header and the bar already carry them. The
// rail itself is built in app.js (`wfRenderRail`).
(function () {
  "use strict";

  const node = typeof module !== "undefined" && module.exports;
  const deps = node ? { ...require("./wf-prs.js"), ...require("./wf-waves.js") } : null;
  const dep = (name) => (deps ? deps[name] : window[name]);

  /// How many PR chips the rail lists before "+N more".
  const RAIL_PR_CHIPS = 8;

  const settled = (p) => p.state === "MERGED" || p.state === "CLOSED";

  /// What the rail shows for `item`:
  /// - `waves`: `{ now, next, track: [{ id, state, label }] }` or null;
  /// - `prs`: `{ text, fresh, chips, more, many, errors }` or null — open PRs
  ///   first, then settled, each a PR record carrying its last error;
  /// - `agents`: the item's sessions, running first, newest-named last —
  ///   `{ id, name, status, running }`.
  /// `sessions` is `state.sessions`; `subView` hides the card the open tab
  /// already shows in full.
  function railModel(item, sessions, { subView = "", now = Date.now() } = {}) {
    if (!item || !item.meta) return { waves: null, prs: null, agents: [] };
    const prs = dep("itemPrs")(item.meta, item.prStack).map((p) => ({
      ...p,
      error: (item.prErrors || {})[p.url] || "",
    }));
    let waves = null;
    if (item.waves && subView !== "waves") {
      const story = dep("waveNarrative")(item, prs);
      const board = dep("waveBoard")(item);
      waves = {
        now: story ? story.now : "",
        next: story ? story.next : "",
        track: board
          ? board.waves.map((w) => ({ id: w.id, state: w.state, label: dep("waveLabel")(w) }))
          : [],
      };
    }
    let prCard = null;
    if (prs.length && subView !== "prs") {
      const ordered = [...prs.filter((p) => !settled(p)), ...prs.filter(settled)];
      prCard = {
        text: dep("prCountsText")(dep("prCounts")(prs)),
        fresh: dep("checkedAgo")(item.prCheckedAt, now),
        chips: ordered.slice(0, RAIL_PR_CHIPS),
        more: Math.max(0, ordered.length - RAIL_PR_CHIPS),
        many: prs.length > 1,
        errors: prs.filter((p) => p.error).length,
      };
    }
    const key = `${item.project}/${item.slug}`;
    const agents = (sessions || [])
      .filter((s) => s.workflow_item === key)
      .map((s) => ({
        id: s.id,
        name: s.name || s.id,
        status: String(s.status || "").toLowerCase(),
        running: !!s.is_running,
      }))
      .sort((a, b) => Number(b.running) - Number(a.running));
    return { waves, prs: prCard, agents };
  }

  const api = { RAIL_PR_CHIPS, railModel };
  if (node) module.exports = api;
  else Object.assign(window, api);
})();
