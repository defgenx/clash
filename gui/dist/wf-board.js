// The workflow board — the pure half.
//
// Every item across projects as a kanban: which lane it is in, what each card
// says (stage, how long ago it moved, PRs, waves, comments) and — the point of
// the board — which items wait on you. "Needs you" is the inbox's rule
// (attention.js): parked on your decision, its agent gone, or PR threads
// waiting on your answer, so the board and the inbox never disagree about
// what is yours to do. The board itself is drawn in app.js
// (`renderWorkflowBoard`).
(function () {
  "use strict";

  const node = typeof module !== "undefined" && module.exports;
  const deps = node ? { ...require("./wf-prs.js"), ...require("./wf-waves.js") } : null;
  const dep = (name) => (deps ? deps[name] : typeof window !== "undefined" ? window[name] : undefined);

  /// The lanes, in pipeline order. `kind` colours a lane: `you` (it waits on
  /// a person), `agent` (an agent works it), `pr`, `done`.
  const BOARD_LANES = [
    { id: "planning", label: "Planning", kind: "agent", statuses: ["draft", "planning"] },
    { id: "plan-review", label: "Plan review", kind: "you", statuses: ["plan-review"] },
    // A review round is in flight like an implementation: an agent has it.
    { id: "building", label: "In progress", kind: "agent", statuses: ["changes-requested", "implementing", "reviewing"] },
    { id: "code-review", label: "Code review", kind: "you", statuses: ["diff-review"] },
    { id: "pr", label: "Pull request", kind: "pr", statuses: ["pr-draft", "pr-ready"] },
    { id: "done", label: "Done", kind: "done", statuses: ["done", "abandoned"] },
  ];

  /// How many finished cards the Done lane shows before "Show all".
  const DONE_SHOWN = 8;

  const DECISION = ["plan-review", "diff-review", "pr-draft", "pr-ready"];
  const WORKING = ["planning", "implementing", "reviewing"];

  const STAGE = {
    draft: "Draft",
    planning: "Planning",
    "plan-review": "Plan review",
    "changes-requested": "Changes queued",
    implementing: "Implementing",
    reviewing: "Reviewing",
    "diff-review": "Code review",
    "pr-draft": "PR draft",
    "pr-ready": "PR ready",
    done: "Done",
    abandoned: "Abandoned",
  };

  function ago(ms, now) {
    if (!ms) return "";
    const min = Math.max(0, Math.floor((now - ms) / 60000));
    if (min < 1) return "just now";
    if (min < 60) return `${min}m ago`;
    const h = Math.floor(min / 60);
    if (h < 24) return `${h}h ago`;
    const d = Math.floor(h / 24);
    if (d < 30) return `${d}d ago`;
    return `${Math.floor(d / 30)}mo ago`;
  }

  /// What one card says. `needs` is why it waits on you (most urgent first),
  /// empty when it does not.
  function boardCard(item, now = Date.now()) {
    const meta = item.meta || {};
    const st = meta.status;
    const prs = dep("itemPrs")(meta, item.prStack);
    const settled = (p) => p.state === "MERGED" || p.state === "CLOSED";
    const unanswered = prs.reduce((n, p) => n + (p.unanswered > 0 ? p.unanswered : 0), 0);
    const needs = [];
    // A finished item asks nothing of you: its old PR threads are history.
    const finished = st === "done" || st === "abandoned";
    if (!finished && item.agentAlive === false && !item.agentResumable) needs.push("agent gone");
    if (DECISION.includes(st) && !item.run) needs.push("your decision");
    if (st === "changes-requested") needs.push("launch the fix round");
    if (!finished && unanswered) needs.push(`${unanswered} PR thread${unanswered === 1 ? "" : "s"}`);
    const w = dep("waveSummary") ? dep("waveSummary")(item) : null;
    return {
      key: `${item.project}/${item.slug}`,
      title: meta.title || item.slug,
      project: item.project,
      status: st,
      stage: STAGE[st] || st,
      iteration: meta.iteration || 1,
      needs,
      working: WORKING.includes(st) && item.agentAlive !== false,
      gone: item.agentAlive === false,
      running: !!item.run,
      comments: item.openAnnotations || 0,
      prs: prs.length
        ? {
            total: prs.length,
            open: prs.filter((p) => !settled(p)).length,
            merged: prs.filter((p) => p.state === "MERGED").length,
            drafts: prs.filter((p) => p.draft && !settled(p)).length,
          }
        : null,
      waves: w ? { done: w.done.length, total: w.total, text: w.text } : null,
      updated: meta.updatedAt || meta.createdAt || 0,
      ago: ago(meta.updatedAt || meta.createdAt || 0, now),
    };
  }

  function matches(item, q) {
    if (!q) return true;
    const meta = item.meta || {};
    const hay = [meta.title, item.slug, item.project, meta.branch, meta.ticket]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
    return q
      .toLowerCase()
      .split(/\s+/)
      .filter(Boolean)
      .every((w) => hay.includes(w));
  }

  /// The whole board for `items`:
  /// - `summary`: `{ needsYou, working, inPr, done, total }` over what the
  ///   filters keep (the header's counts);
  /// - `projects`: `[{ name, count }]`, every project, most items first;
  /// - `lanes`: `[{ id, label, kind, count, cards, hidden }]` — a lane's cards
  ///   put what needs you first, then the most recently moved; the Done lane
  ///   keeps its latest `DONE_SHOWN` unless `showDone`, and says how many it
  ///   left out (`hidden`).
  /// `filter` is `{ project, query, showDone }`.
  function boardModel(items, { project = null, query = "", showDone = false } = {}, now = Date.now()) {
    const all = items || [];
    const counts = new Map();
    for (const it of all) counts.set(it.project, (counts.get(it.project) || 0) + 1);
    const projects = [...counts]
      .map(([name, count]) => ({ name, count }))
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
    const kept = all.filter((it) => (!project || it.project === project) && matches(it, query.trim()));
    const cards = kept.map((it) => boardCard(it, now));
    const lanes = BOARD_LANES.map((l) => {
      let list = cards
        .filter((c) => l.statuses.includes(c.status))
        .sort((a, b) => Number(b.needs.length > 0) - Number(a.needs.length > 0) || b.updated - a.updated);
      const count = list.length;
      let hidden = 0;
      if (l.id === "done" && !showDone && count > DONE_SHOWN) {
        hidden = count - DONE_SHOWN;
        list = list.slice(0, DONE_SHOWN);
      }
      return { id: l.id, label: l.label, kind: l.kind, count, cards: list, hidden };
    });
    const lane = (id) => lanes.find((l) => l.id === id);
    return {
      summary: {
        needsYou: cards.filter((c) => c.needs.length).length,
        working: cards.filter((c) => c.working).length,
        inPr: lane("pr").count,
        done: lane("done").count,
        total: cards.length,
      },
      projects,
      lanes,
    };
  }

  const api = { BOARD_LANES, DONE_SHOWN, boardCard, boardModel };
  if (node) module.exports = api;
  else Object.assign(window, api);
})();
