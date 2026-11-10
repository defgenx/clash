// Waves — the pure half of a multi-wave item's UI.
//
// A project-sized item is planned as waves (`waves.json`): slices approved one
// by one, in dependency order, inside the one item. The backend computes the
// board (`item.waves`: each wave's state, level and the waves it may run
// alongside — `application::waves`); this module turns it into what the bar,
// the Waves tab and the wave picker show, so none of those decides anything on
// its own. Contract: docs/workflows.md → Waves.
(function () {
  "use strict";

  /// The item's usable board, or null (no waves.json, or one with an issue —
  /// `wavesIssue` says which).
  function waveBoard(item) {
    const b = item && item.waves;
    return b && !b.issue && Array.isArray(b.waves) && b.waves.length ? b : null;
  }

  function wavesIssue(item) {
    return (item && item.waves && item.waves.issue) || null;
  }

  const ids = (ws) => ws.map((w) => w.id);
  const of = (board, state) => board.waves.filter((w) => w.state === state);

  /// "w2 · API".
  function waveLabel(w) {
    return w.title ? `${w.id} · ${w.title}` : w.id;
  }

  /// "w2, w3".
  function waveList(list) {
    return list.map((w) => (typeof w === "string" ? w : w.id)).join(", ");
  }

  /// Where the item's waves stand, in one place every surface reads.
  function waveSummary(item) {
    const board = waveBoard(item);
    if (!board) return null;
    const done = of(board, "done");
    const active = of(board, "active");
    const review = of(board, "review");
    const ready = of(board, "ready");
    const total = board.waves.length;
    // After approving the round in review, does anything remain?
    const left = board.waves.filter((w) => w.state !== "done" && w.state !== "review");
    let text = `Waves ${done.length}/${total}`;
    if (active.length) text += ` · ${waveList(active)} in progress`;
    else if (review.length) text += ` · ${waveList(review)} to approve`;
    else if (ready.length && done.length < total) text += ` · ${waveList(ready)} ready`;
    if (done.length === total) text += " · all approved";
    return {
      total,
      done: ids(done),
      active: ids(active),
      review: ids(review),
      ready: ids(ready),
      remainingAfterReview: ids(left),
      allDone: done.length === total,
      text,
    };
  }

  /// What a parked item's bar offers for its waves:
  /// - `approve` — the round's waves wait for approval and others remain;
  /// - `approve-last` — they are the last ones (the item's own approvals take
  ///   over, approving them on the way);
  /// - `start` — nothing waits, and some waves are ready;
  /// - `blocked` — nothing ready and nothing to approve (should not happen
  ///   with a valid plan; said rather than hidden);
  /// - null — no waves, or all approved.
  function waveBarState(item) {
    const s = waveSummary(item);
    if (!s || s.allDone) return null;
    if (s.review.length) return { kind: s.remainingAfterReview.length ? "approve" : "approve-last", ...s };
    if (s.active.length) return null; // an agent is on it; nothing to decide
    if (s.ready.length) return { kind: "start", ...s };
    return { kind: "blocked", ...s };
  }

  /// Do the item's final approvals (done, PR stage) wait on waves?
  function wavesHoldApproval(item) {
    const st = waveBarState(item);
    return !!st && st.kind !== "approve-last";
  }

  /// The picker's rows: every ready wave, each disabled while it cannot run
  /// with the ones ticked (a dependency between them, overlapping areas, or
  /// areas nobody declared). Pure over the board and the current pick.
  function wavePickRows(item, picked) {
    const board = waveBoard(item);
    if (!board) return [];
    const ready = of(board, "ready");
    const chosen = new Set(picked || []);
    return ready.map((w) => {
      const clash = [...chosen].filter((id) => id !== w.id && !(w.compatibleWith || []).includes(id));
      const checked = chosen.has(w.id);
      const together = (w.compatibleWith || []).filter((id) => ready.some((r) => r.id === id));
      return {
        id: w.id,
        label: waveLabel(w),
        detail:
          (w.summary ? `${w.summary} ` : "") +
          (w.areas && w.areas.length ? `Touches ${w.areas.join(", ")}.` : "Declares no areas, so it runs alone."),
        checked,
        disabled: !checked && clash.length > 0,
        reason: !checked && clash.length ? `Can't run with ${clash.join(", ")}` : "",
        together,
      };
    });
  }

  /// The default pick: the first ready wave, in plan order — waves go in
  /// order unless you choose to run compatible ones together.
  function defaultWavePick(item) {
    const board = waveBoard(item);
    if (!board) return [];
    const first = of(board, "ready")[0];
    return first ? [first.id] : [];
  }

  /// Toggle `id` in a pick, refusing a wave that cannot join the others.
  function toggleWavePick(item, picked, id) {
    const cur = new Set(picked || []);
    if (cur.has(id)) {
      cur.delete(id);
    } else {
      const row = wavePickRows(item, [...cur]).find((r) => r.id === id);
      if (!row || row.disabled) return [...cur];
      cur.add(id);
    }
    const order = (waveBoard(item) || { waves: [] }).waves.map((w) => w.id);
    return order.filter((x) => cur.has(x));
  }

  /// The board's columns: waves grouped by dependency level, so a column
  /// holds the waves that may run side by side once the previous ones land.
  function waveColumns(item) {
    const board = waveBoard(item);
    if (!board) return [];
    const cols = [];
    for (const w of board.waves) {
      const l = w.level || 0;
      (cols[l] = cols[l] || []).push(w);
    }
    return cols.filter(Boolean).map((waves, i) => ({
      title: i === 0 ? "First" : `Then (after ${waveList(cols[i - 1] || [])})`,
      waves,
    }));
  }

  /// A wave state's badge text.
  function waveStateLabel(state) {
    switch (state) {
      case "done":
        return "approved";
      case "review":
        return "to approve";
      case "active":
        return "in progress";
      case "ready":
        return "ready";
      default:
        return "waiting";
    }
  }

  const api = {
    waveBoard,
    wavesIssue,
    waveLabel,
    waveList,
    waveSummary,
    waveBarState,
    wavesHoldApproval,
    wavePickRows,
    defaultWavePick,
    toggleWavePick,
    waveColumns,
    waveStateLabel,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
