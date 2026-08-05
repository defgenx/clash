// Workflow timeline — the pure half.
//
// The Timeline sub-view replaces the flat "Iteration N" history list: one
// chronological feed joining what used to live in four separate files — the
// change rounds of review.md (with the human's note, which is *why* a round
// happened), the agent review rounds of agent-review.md (verdict + what was
// published), the history/ snapshots (what can be diffed or read back), and
// the item's creation. The backend hands the parsed pieces over verbatim
// (`get_workflow_timeline`); this model merges and orders them into cards.
//
// Same shape as `wf-compose.js`/`wf-review.js`: everything testable without a
// DOM lives here, the rendering is in `app.js`.
(function () {
  "use strict";

  /// "YYYY-MM-DD HH:MM" (the stamp clash and the skills write) → epoch ms,
  /// or null. Hand-parsed: WKWebView's Date.parse is unreliable on the
  /// non-ISO space-separated form.
  function parseStamp(s) {
    const m = /(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/.exec(s || "");
    if (!m) return null;
    return new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]).getTime();
  }

  /// "diff · deep · 2026-08-04 17:27" (the contractual round heading) →
  /// { target, depth, date }. Missing pieces come back empty, never throw —
  /// the heading is agent-written prose.
  function parseReviewHeading(heading) {
    const parts = String(heading || "")
      .split("·")
      .map((p) => p.trim());
    return {
      target: parts[0] || "",
      depth: parts[1] || "",
      date: parts.length > 2 ? parts.slice(2).join(" · ") : "",
    };
  }

  /// Merge the backend's timeline pieces into display cards, newest first.
  ///
  /// Ordering: events carrying a parseable stamp sort by it; undated ones
  /// (legacy files) keep their file order and sort as oldest — a wrong-but-
  /// stable position beats dropping them. A history snapshot with no matching
  /// review.md section (legacy note-less rounds) still gets a card, so every
  /// diffable iteration is reachable from the timeline.
  function timelineModel({
    iterations = [],
    reviews = [],
    history = [],
    planSnapshots = [],
    hasPlanPhase = true,
    createdAt = 0,
    mode = "full",
  } = {}) {
    const plans = new Set(planSnapshots);
    const noted = new Set(iterations.map((it) => it.iteration));
    const events = [];

    for (const it of iterations) {
      events.push({
        kind: "change-round",
        iteration: it.iteration,
        date: it.heading || "",
        stamp: parseStamp(it.heading),
        note: it.note || "",
        annotations: it.annotations || [],
        hasCodeDiff: history.includes(it.iteration),
        hasPlanDiff: hasPlanPhase && plans.has(it.iteration),
        hasPlanSnapshot: plans.has(it.iteration),
      });
    }
    for (const n of history) {
      if (noted.has(n)) continue;
      events.push({
        kind: "change-round",
        iteration: n,
        date: "",
        stamp: null,
        note: "",
        annotations: [],
        hasCodeDiff: true,
        hasPlanDiff: hasPlanPhase && plans.has(n),
        hasPlanSnapshot: plans.has(n),
      });
    }
    for (const r of reviews) {
      const h = parseReviewHeading(r.heading);
      events.push({
        kind: "agent-review",
        round: r.round,
        target: h.target,
        depth: h.depth,
        date: h.date,
        stamp: parseStamp(h.date),
        verdict: r.verdict || "",
        published: r.published || [],
      });
    }
    events.push({ kind: "created", stamp: createdAt || 0, date: "", mode });

    // Stable ascending sort by stamp, then reversed: the feed reads newest
    // first, like the sidebar. Undated events (legacy/hand-written files)
    // sort as "just after creation" and keep their file order among
    // themselves — a stable approximation beats dropping them.
    const fallback = (createdAt || 0) + 1;
    const indexed = events.map((e, i) => [e, i]);
    indexed.sort(
      (a, b) => (a[0].stamp ?? fallback) - (b[0].stamp ?? fallback) || a[1] - b[1]
    );
    return indexed.map(([e]) => e).reverse();
  }

  const api = { timelineModel, parseStamp, parseReviewHeading };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
