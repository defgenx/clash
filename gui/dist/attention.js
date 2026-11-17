// The attention inbox — the pure half.
//
// Attention is signalled in six places at once: a session goes Prompting, a
// workflow item parks on a decision, a PR's CI goes red, a PR collects
// replyless review comments, an agent dies mid-round, a session errors. Each surface knows only its own
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

  /// `wf-decisions.js`'s reason for an item's open decisions.
  function decisionsReason(item) {
    const f =
      typeof module !== "undefined" && module.exports
        ? require("./wf-decisions.js").decisionsReason
        : typeof window !== "undefined" && window.decisionsReason;
    return f ? f(item) : null;
  }

  // Urgency bands. Lower goes first.
  const BAND = {
    prompting: 0, // a tool call is blocked on yes/no
    stalled: 1, // the agent is gone; nothing moves until a human acts
    errored: 2,
    decision: 3, // the pipeline is parked on your review
    ciFailing: 4, // an open PR's build is red; nothing merges until it is fixed
    prComments: 5,
    waiting: 6, // turn finished, no question pending
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

  /// The open PRs of an item whose CI fails (`pr.health.checks`, written by
  /// the PR refresh), primary + linked.
  function ciFailingPrs(meta) {
    const prs = [(meta && meta.pr) || null].concat((meta && meta.linkedPrs) || []);
    return prs.filter(
      (pr) =>
        pr &&
        pr.url &&
        pr.state !== "MERGED" &&
        pr.state !== "CLOSED" &&
        pr.health &&
        pr.health.checks === "fail"
    );
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
    // Not on a finished item: its PRs no longer decide anything.
    const red = ["done", "abandoned"].includes(meta.status) ? [] : ciFailingPrs(meta);
    if (red.length) {
      const names = red.flatMap((pr) => pr.health.failing || []);
      out.push({
        band: BAND.ciFailing,
        text: `CI failing${red.length > 1 ? ` on ${red.length} PRs` : ""}${
          names.length ? ` — ${[...new Set(names)].slice(0, 3).join(", ")}` : ""
        }`,
      });
    }
    const decisions = decisionsReason(item);
    if (decisions) out.push({ band: BAND.decision, text: decisions });
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
        bands: [reason.band],
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
        bands: reasons.map((r) => r.band),
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

  /// The inbox's sections, in order. A row sits in the section of its most
  /// urgent reason, so one item never appears twice.
  const INBOX_GROUPS = [
    { id: "now", label: "Needs you now", bands: [BAND.prompting, BAND.stalled, BAND.errored] },
    { id: "decisions", label: "Decisions", bands: [BAND.decision] },
    { id: "prs", label: "Pull requests", bands: [BAND.ciFailing, BAND.prComments] },
    { id: "turns", label: "Finished turns", bands: [BAND.waiting] },
  ];

  function groupOf(row) {
    return INBOX_GROUPS.find((g) => g.bands.includes(row.band)) || INBOX_GROUPS[INBOX_GROUPS.length - 1];
  }

  /// What a snooze is pinned to: when any of it changes — a new status, new
  /// activity, a new reason — the row is news again and comes back.
  function rowSignature(row) {
    return [row.status, row.since || 0, (row.reasons || []).join("|")].join("·");
  }

  const SNOOZE_HOUR_MS = 60 * 60 * 1000;
  /// How long a snooze outlives its row's absence (a session answered, a list
  /// still loading) before it is forgotten.
  const SNOOZE_KEEP_MS = 7 * 24 * SNOOZE_HOUR_MS;

  /// A snooze for `row`: `until` null hides it until it changes; "hour" also
  /// brings it back after an hour if nothing has changed by then.
  function snoozeEntry(row, kind, now) {
    return { sig: rowSignature(row), until: kind === "hour" ? now + SNOOZE_HOUR_MS : null, at: now };
  }

  function isSnoozed(snoozes, row, now) {
    const s = snoozes && snoozes[row.key];
    if (!s || s.sig !== rowSignature(row)) return false;
    return !(s.until && now >= s.until);
  }

  /// Snoozes worth keeping. One whose row is on screen stays while it still
  /// hides that row (changed or expired: dropped). One whose row is absent —
  /// answered, or in a list that has not loaded yet — is kept until it would
  /// have expired, or for SNOOZE_KEEP_MS, so a reload never resurfaces it.
  function pruneSnoozes(snoozes, rows, now) {
    const out = {};
    const byKey = new Map((rows || []).map((r) => [r.key, r]));
    for (const [key, s] of Object.entries(snoozes || {})) {
      const row = byKey.get(key);
      const keep = row
        ? isSnoozed(snoozes, row, now)
        : s.until
          ? now < s.until
          : now - (s.at || 0) < SNOOZE_KEEP_MS;
      if (keep) out[key] = s;
    }
    return out;
  }

  /// Saved snoozes as the model reads them; anything malformed is dropped.
  function normalizeSnoozes(raw) {
    const out = {};
    if (!raw || typeof raw !== "object") return out;
    for (const [k, v] of Object.entries(raw)) {
      if (v && typeof v.sig === "string" && (v.until === null || Number.isFinite(v.until))) {
        out[k] = { sig: v.sig, until: v.until, at: Number.isFinite(v.at) ? v.at : 0 };
      }
    }
    return out;
  }

  /// The inbox split into what it shows and what is snoozed. The badge and
  /// the rail count `active` only — snoozing exists so the count can drop.
  function splitSnoozed(rows, snoozes, now) {
    const active = [];
    const snoozed = [];
    for (const r of rows || []) (isSnoozed(snoozes, r, now) ? snoozed : active).push(r);
    return { active, snoozed };
  }

  /// The filters the view offers, by id: every section, all of them, or the
  /// snoozed rows.
  const INBOX_FILTERS = ["all"].concat(INBOX_GROUPS.map((g) => g.id), ["snoozed"]);

  function matchesQuery(row, query) {
    const q = String(query || "").trim().toLowerCase();
    if (!q) return true;
    return [row.title, row.context, row.status, (row.reasons || []).join(" ")]
      .join(" ")
      .toLowerCase()
      .includes(q);
  }

  /// What the inbox tab draws: its sections (empty ones left out) for the
  /// chosen filter and search, plus each filter's count under that search.
  function inboxView(rows, { filter = "all", query = "", snoozes = {}, now = Date.now() } = {}) {
    const { active, snoozed } = splitSnoozed(rows, snoozes, now);
    const found = active.filter((r) => matchesQuery(r, query));
    const sleeping = snoozed.filter((r) => matchesQuery(r, query));
    const counts = { all: found.length, snoozed: sleeping.length };
    for (const g of INBOX_GROUPS) counts[g.id] = found.filter((r) => groupOf(r).id === g.id).length;
    const want = INBOX_FILTERS.includes(filter) ? filter : "all";
    let groups;
    if (want === "snoozed") {
      groups = sleeping.length ? [{ id: "snoozed", label: "Snoozed", rows: sleeping }] : [];
    } else {
      groups = INBOX_GROUPS.filter((g) => want === "all" || g.id === want)
        .map((g) => ({ id: g.id, label: g.label, rows: found.filter((r) => groupOf(r).id === g.id) }))
        .filter((g) => g.rows.length);
    }
    return {
      filter: want,
      groups,
      counts,
      total: active.length,
      blocking: attentionSummary(active).blocking,
    };
  }

  /// The actions a row offers beyond opening it, chosen by why it is there:
  /// a finished turn can take a follow-up or be stashed, an errored session
  /// reloaded, an item with failing CI or open threads opens its PR. A
  /// session that is not running cannot take a follow-up or a stash.
  function rowActions(row, { running = false, hasPr = false } = {}) {
    const out = [];
    const bands = row.bands || [row.band];
    if (row.kind === "session") {
      if (running && row.band === BAND.waiting) out.push("followup", "stash");
      if (row.band === BAND.errored) out.push("reload");
    } else if (hasPr && (bands.includes(BAND.ciFailing) || bands.includes(BAND.prComments))) {
      out.push("open-pr");
    }
    return out;
  }

  /// The PRs an item's row is about: the open ones failing CI or holding
  /// unanswered threads, primary first. Empty when the row is not about a PR.
  function problemPrUrls(meta) {
    const prs = [(meta && meta.pr) || null].concat((meta && meta.linkedPrs) || []);
    const failing = new Set(ciFailingPrs(meta).map((pr) => pr.url));
    return prs
      .filter((pr) => pr && pr.url && (failing.has(pr.url) || pr.unansweredComments > 0))
      .map((pr) => pr.url);
  }

  /// Keyboard selection: the key `delta` rows away from `current` in the
  /// order the rows are drawn, clamped at both ends; the first row when
  /// nothing (or a row that has gone) was selected.
  function inboxMove(keys, current, delta) {
    if (!keys.length) return null;
    const i = keys.indexOf(current);
    if (i < 0) return keys[0];
    return keys[Math.max(0, Math.min(keys.length - 1, i + delta))];
  }

  const api = {
    BAND,
    INBOX_GROUPS,
    INBOX_FILTERS,
    groupOf,
    rowSignature,
    snoozeEntry,
    isSnoozed,
    pruneSnoozes,
    normalizeSnoozes,
    splitSnoozed,
    inboxView,
    rowActions,
    problemPrUrls,
    inboxMove,
    parseStamp,
    unansweredCount,
    ciFailingPrs,
    sessionReason,
    workflowReasons,
    attentionRows,
    attentionSummary,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
