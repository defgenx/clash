// The attention inbox — the pure half.
//
// Attention is signalled in five places at once: a session goes Prompting, a
// workflow item parks on a decision, a PR collects replyless review comments,
// an agent dies mid-round, a session errors. Each surface knows only its own
// slice, so with a dozen agents in flight the only way to find what is waiting
// on *you* is to scan all of them and hold the result in your head. This folds
// them into one ordered list. The tab that renders it lives in app.js (the
// `wf-prs.js` precedent).
//
// Bands, not timestamps, decide the order: a blocked tool call outranks a
// finished turn no matter which happened first. Within a band the oldest goes
// first, because the thing that has been waiting longest is the thing you
// forgot about.
(function () {
  "use strict";

  // Mirrors WorkflowStatus::needs_attention — kept local so the module stays
  // loadable without app.js (node tests).
  const WF_DECISION_STATUSES = ["plan-review", "diff-review", "pr-draft"];

  // Session statuses that cannot proceed without you. Same set the backend
  // notifies on (`list_sessions`), so the inbox and the desktop notification
  // can never disagree about what "needs attention" means.
  const SESSION_ATTENTION = ["Prompting", "Errored", "Waiting"];

  // Urgency bands. Lower goes first.
  const BAND = {
    prompting: 0, // a tool call is blocked on yes/no
    stalled: 1, // the agent is gone; nothing moves until a human acts
    errored: 2,
    decision: 3, // the pipeline is parked on your review
    prComments: 4,
    waiting: 5, // turn finished, no question pending
  };

  /// `Session.last_modified` is formatted local time at minute resolution
  /// ("%Y-%m-%d %H:%M"), and workflow items carry epoch millis. Both become ms
  /// here so one sort can order them against each other — comparing the two
  /// spellings as strings put every session ahead of every item, whatever the
  /// clock said. Built from parts rather than handed to `new Date(str)`: that
  /// form is unspecified, and JSC is not V8.
  function parseStamp(v) {
    if (typeof v === "number") return Number.isFinite(v) ? v : 0;
    const m = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/.exec(String(v || ""));
    if (!m) return 0;
    const t = new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]).getTime();
    return Number.isFinite(t) ? t : 0;
  }

  /// Every unanswered PR-comment count on an item, primary + linked.
  function unansweredCount(meta) {
    const prs = [(meta && meta.pr) || null].concat((meta && meta.linkedPrs) || []);
    let n = 0;
    for (const pr of prs) {
      if (pr && pr.url && typeof pr.unansweredComments === "number") n += pr.unansweredComments;
    }
    return n;
  }

  /// Why this session wants you, or null. One reason per session — the status
  /// is a single value, so there is nothing to merge.
  function sessionReason(s) {
    if (!s || !SESSION_ATTENTION.includes(s.status)) return null;
    // A stashed or dead row can still carry a stale Waiting status; it is not
    // waiting for anything, it is over. Errored is the exception — the process
    // is gone precisely because that is the news.
    if (s.status !== "Errored" && !s.is_running) return null;
    if (s.status === "Prompting") return { band: BAND.prompting, text: "needs your approval" };
    if (s.status === "Errored") return { band: BAND.errored, text: "errored" };
    return { band: BAND.waiting, text: "turn finished — waiting for your next message" };
  }

  /// Why this workflow item wants you: zero or more reasons, most urgent
  /// first. An item can want two things at once (parked on a review *and*
  /// carrying unanswered PR comments) and must stay one row — two rows for one
  /// item is how an inbox stops being countable.
  function workflowReasons(item) {
    const meta = (item && item.meta) || {};
    const out = [];
    if (item && item.agentAlive === false) {
      out.push({ band: BAND.stalled, text: "agent session is gone — relaunch or end the round" });
    }
    if (WF_DECISION_STATUSES.includes(meta.status)) {
      out.push({ band: BAND.decision, text: "parked on your decision" });
    }
    const unanswered = unansweredCount(meta);
    if (unanswered > 0) {
      out.push({
        band: BAND.prComments,
        text: `${unanswered} unanswered PR comment${unanswered === 1 ? "" : "s"}`,
      });
    }
    return out;
  }

  /// The inbox, ordered. `sessions` and `workflows` are the frontend's own
  /// lists; `unread`/`wfUnread` are the Sets of keys with unseen attention
  /// events, surfaced as a dot rather than as ordering (an event you already
  /// saw is not less urgent).
  function attentionRows({ sessions, workflows, unread, wfUnread } = {}) {
    const rows = [];
    for (const s of sessions || []) {
      const reason = sessionReason(s);
      if (!reason) continue;
      rows.push({
        key: `session:${s.id}`,
        kind: "session",
        sessionId: s.id,
        title: s.name || s.summary || s.first_prompt || String(s.id).slice(0, 8),
        context: [s.worktree ? `⊟ ${s.worktree}` : "", s.project].filter(Boolean).join(" · "),
        status: s.status,
        band: reason.band,
        reasons: [reason.text],
        since: parseStamp(s.last_modified),
        unread: !!(unread && unread.has(s.id)),
      });
    }
    for (const item of workflows || []) {
      const reasons = workflowReasons(item);
      if (!reasons.length) continue;
      const meta = item.meta || {};
      const key = `${item.project}/${item.slug}`;
      rows.push({
        key: `workflow:${key}`,
        kind: "workflow",
        project: item.project,
        slug: item.slug,
        title: meta.title || item.slug,
        context: item.project,
        status: meta.status || "unknown",
        band: Math.min.apply(null, reasons.map((r) => r.band)),
        reasons: reasons.map((r) => r.text),
        since: parseStamp(meta.updatedAt),
        unread: !!(wfUnread && wfUnread.has(key)),
      });
    }
    rows.sort((a, b) => {
      if (a.band !== b.band) return a.band - b.band;
      // Oldest first inside a band. An unknown stamp sorts last: "no idea how
      // long" must not jump the queue ahead of a measured wait.
      if (!a.since !== !b.since) return a.since ? -1 : 1;
      if (a.since !== b.since) return a.since - b.since;
      return a.title.localeCompare(b.title);
    });
    return rows;
  }

  /// What the sidebar badge shows. Blocked work (a pending approval, a wedged
  /// agent, an error) is counted apart from the merely-finished, so the badge
  /// can distinguish "3 need you now" from a long tail of idle sessions that
  /// would otherwise keep the number permanently high and unreadable.
  function attentionSummary(rows) {
    const list = rows || [];
    return {
      total: list.length,
      blocking: list.filter((r) => r.band <= BAND.errored).length,
    };
  }

  const api = {
    BAND,
    parseStamp,
    unansweredCount,
    sessionReason,
    workflowReasons,
    attentionRows,
    attentionSummary,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
