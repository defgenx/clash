// The decision queue — the pure half.
//
// The executor's hand-off asks the human questions only they can answer
// (`- [D3] … — options: a) … b) … — recommended: a`). clash keeps them in
// `decisions.json` across rounds (`application::decisions`), and the item's
// `decisions` field carries every record. This module decides what the
// Decisions tab, the bar button, the rail and the inbox show; the tab itself
// is in app.js. Contract: docs/workflows.md → Decisions. Tested by
// gui/tests/wf_decisions.test.js.
(function () {
  "use strict";

  /// The stages parked on the human, where a decision can be answered before
  /// the next round. Mirrors `WF_REVIEWABLE` in app.js.
  const DECISION_STAGES = ["plan-review", "diff-review", "pr-draft", "pr-ready"];

  function decisionsOf(item) {
    return (item && Array.isArray(item.decisions) && item.decisions) || [];
  }

  /// The decisions still waiting on an answer.
  function openDecisions(item) {
    return decisionsOf(item).filter((d) => d.status === "open");
  }

  /// The answer an option's button records: what the agent will read.
  function optionAnswer(opt) {
    return `${opt.key}) ${opt.text}`;
  }

  /// Where an answer stands: not handed to a round yet, or handed to the
  /// round of iteration N.
  function answerState(d, iteration) {
    const n = d.deliveredIteration || 0;
    if (!n) return "goes to the next round";
    if (n === (iteration || 0)) return `handed to this round (iteration ${n})`;
    return `handed to iteration ${n}`;
  }

  /// The tab: open ones first (each with its options and the recommended
  /// one flagged), then the settled ones, newest answer first.
  function decisionsModel(item) {
    const all = decisionsOf(item);
    const iteration = ((item && item.meta) || {}).iteration || 0;
    const row = (d) => ({
      id: d.id,
      text: d.text || "",
      recommended: d.recommended || "",
      firstSeen: d.firstSeenIteration || 0,
      options: (d.options || []).map((o) => ({
        key: o.key,
        text: o.text,
        label: optionAnswer(o),
        recommended: !!d.recommended && o.key === d.recommended,
      })),
      // A recommendation that names no option is the agent's own words.
      recommendedText:
        d.recommended && !(d.options || []).some((o) => o.key === d.recommended) ? d.recommended : "",
      status: d.status,
      answer: d.answer || "",
      answeredAt: d.answeredAt || 0,
      deliveredIteration: d.deliveredIteration || 0,
      state: d.status === "answered" ? answerState(d, iteration) : d.status === "dropped" ? "dropped" : "",
    });
    const settled = all
      .filter((d) => d.status === "answered" || d.status === "dropped")
      .map(row)
      .sort((a, b) => b.answeredAt - a.answeredAt);
    return { open: openDecisions(item).map(row), settled };
  }

  /// The sub-view's label: the open count as the badge.
  /// How long an open decision has waited, in rounds, and how to say it.
  function decisionAge(d, iteration) {
    const first = d.firstSeen || 0;
    const rounds = first && iteration ? Math.max(0, iteration - first) : 0;
    return {
      rounds,
      text: !first
        ? ""
        : rounds === 0
          ? `asked this round (iteration ${first})`
          : `asked in iteration ${first} · waiting ${rounds} round${rounds === 1 ? "" : "s"}`,
      stale: rounds >= 2,
    };
  }

  /// The header of the Decisions view: counts, progress and what an answer
  /// does next.
  function decisionsSummary(model) {
    const open = model.open.length;
    const answered = model.settled.filter((d) => d.status === "answered");
    const pending = answered.filter((d) => d.state === "goes to the next round").length;
    const dropped = model.settled.filter((d) => d.status === "dropped").length;
    const total = open + answered.length + dropped;
    const done = answered.length + dropped;
    const withRec = model.open.filter((d) => d.options.some((o) => o.recommended)).length;
    return {
      open,
      answered: answered.length,
      pending,
      dropped,
      total,
      percent: total ? Math.round((done / total) * 100) : 0,
      headline: !total
        ? "No decisions yet — agents list them in their hand-offs"
        : open
          ? `${open} waiting on you`
          : "Everything is answered",
      next: pending
        ? `${pending} answer${pending === 1 ? "" : "s"} will reach the next round — in its kickoff and your next change request`
        : "",
      acceptable: withRec,
    };
  }

  /// The rows a filter keeps, matched on id, question, options and answer.
  function filterDecisions(model, filter, query) {
    const q = (query || "").trim().toLowerCase();
    const pool =
      filter === "open" ? model.open : filter === "answered" ? model.settled : [...model.open, ...model.settled];
    if (!q) return pool;
    return pool.filter((d) =>
      [d.id, d.text, d.answer, ...d.options.map((o) => o.text)].some((s) => (s || "").toLowerCase().includes(q))
    );
  }

  /// The topic a "Discuss this" session opens with.
  function decisionTopic(d) {
    const opts = d.options.length ? ` Options: ${d.options.map((o) => o.label).join("; ")}.` : "";
    const rec = d.options.find((o) => o.recommended);
    return `Decision ${d.id}: ${d.text}${opts}${rec ? ` The agent recommends ${rec.key}.` : ""} Help me decide; when we agree, I will answer it in the Decisions tab.`;
  }

  function decisionsTabLabel(item) {
    const n = openDecisions(item).length;
    return n ? `❓ Decisions (${n})` : "❓ Decisions";
  }

  /// The attention-inbox reason, at a parked stage only: while an agent
  /// works, its questions are not yet the human's to answer.
  function decisionsReason(item) {
    const st = ((item && item.meta) || {}).status;
    const n = openDecisions(item).length;
    if (!n || !DECISION_STAGES.includes(st)) return null;
    return `${n} decision${n === 1 ? "" : "s"} waiting`;
  }

  /// What the change-request composer says about the answers clash appends
  /// to the note (`### Decisions`): those never handed to a round yet.
  function unsentAnswersNote(item) {
    const ids = decisionsOf(item)
      .filter((d) => d.status === "answered" && !d.deliveredIteration)
      .map((d) => d.id);
    if (!ids.length) return "";
    return ` Your answer${ids.length === 1 ? "" : "s"} to ${ids.join(", ")} ${
      ids.length === 1 ? "is" : "are"
    } appended under ### Decisions.`;
  }

  const api = {
    DECISION_STAGES,
    unsentAnswersNote,
    openDecisions,
    optionAnswer,
    answerState,
    decisionsModel,
    decisionsTabLabel,
    decisionsReason,
    decisionAge,
    decisionsSummary,
    filterDecisions,
    decisionTopic,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
