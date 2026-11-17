// node --test gui/tests/ — the attention inbox's pure model.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  BAND,
  parseStamp,
  unansweredCount,
  sessionReason,
  workflowReasons,
  attentionRows,
  attentionSummary,
  INBOX_GROUPS,
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
} = require("../dist/attention.js");

const sess = (over = {}) => ({
  id: "s1",
  name: "one",
  project: "proj",
  status: "Waiting",
  is_running: true,
  last_modified: "2026-09-01 10:00",
  ...over,
});

const item = (over = {}) => ({
  project: "proj",
  slug: "slug",
  agentAlive: null,
  meta: { title: "An item", status: "planning", updatedAt: 0, ...(over.meta || {}) },
  ...over,
});

test("only sessions that cannot proceed without a human are listed", () => {
  assert.equal(sessionReason(sess({ status: "Running" })), null);
  assert.equal(sessionReason(sess({ status: "Thinking" })), null);
  assert.equal(sessionReason(sess({ status: "Starting" })), null);
  assert.equal(sessionReason(sess({ status: "Stashed" })), null);
  assert.equal(sessionReason(sess({ status: "Prompting" })).band, BAND.prompting);
  assert.equal(sessionReason(sess({ status: "Waiting" })).band, BAND.waiting);
});

test("a dead row's stale Waiting is not attention, but Errored is", () => {
  // Nothing is waiting for you on a session whose process is gone; an error is
  // the one case where the dead process IS the news.
  assert.equal(sessionReason(sess({ status: "Waiting", is_running: false })), null);
  assert.equal(
    sessionReason(sess({ status: "Errored", is_running: false })).band,
    BAND.errored
  );
});

test("an item wanting two things stays one row carrying both reasons", () => {
  const rows = attentionRows({
    workflows: [
      item({
        meta: {
          status: "diff-review",
          pr: { url: "u", unansweredComments: 2 },
        },
      }),
    ],
  });
  assert.equal(rows.length, 1);
  assert.deepEqual(rows[0].reasons, ["parked on your decision", "2 unanswered PR comments"]);
  // The row takes the most urgent band of its reasons, not the last one found.
  assert.equal(rows[0].band, BAND.decision);
});

test("unanswered comments are summed across primary and linked PRs", () => {
  assert.equal(unansweredCount({}), 0);
  assert.equal(unansweredCount({ pr: { url: "u", unansweredComments: 3 } }), 3);
  assert.equal(
    unansweredCount({
      pr: { url: "u", unansweredComments: 3 },
      linkedPrs: [{ url: "v", unansweredComments: 2 }, { url: "w" }],
    }),
    5
  );
  // A placeholder record with no URL is not a PR.
  assert.equal(unansweredCount({ pr: { unansweredComments: 9 } }), 0);
});

test("a dead agent outranks every decision, and a working item is silent", () => {
  assert.deepEqual(workflowReasons(item({ meta: { status: "implementing" } })), []);
  const stalled = workflowReasons(item({ agentAlive: false, meta: { status: "implementing" } }));
  assert.equal(stalled.length, 1);
  assert.equal(stalled[0].band, BAND.stalled);
});

test("bands order the inbox, oldest first inside a band", () => {
  const rows = attentionRows({
    sessions: [
      sess({ id: "old-wait", status: "Waiting", last_modified: "2026-09-01 08:00" }),
      sess({ id: "new-wait", status: "Waiting", last_modified: "2026-09-01 09:00" }),
      sess({ id: "approve", status: "Prompting", last_modified: "2026-09-01 09:30" }),
    ],
    workflows: [item({ meta: { status: "plan-review", updatedAt: 1 } })],
  });
  assert.deepEqual(
    rows.map((r) => r.key),
    [
      "session:approve", // band 0 — blocked tool call, newest of all, still first
      "workflow:proj/slug", // band 3
      "session:old-wait", // band 6, oldest
      "session:new-wait",
    ]
  );
});

test("a session and an item in the same band order by real time, not spelling", () => {
  // Session stamps are formatted local time, item stamps epoch millis. Sorting
  // the two spellings as strings put every session first regardless of clock.
  const early = new Date(2026, 8, 1, 8, 0).getTime();
  const late = new Date(2026, 8, 1, 9, 0).getTime();
  assert.equal(parseStamp("2026-09-01 08:00"), early);
  assert.equal(parseStamp(late), late);
  assert.equal(parseStamp("nonsense"), 0);
  assert.equal(parseStamp(undefined), 0);

  const rows = attentionRows({
    sessions: [sess({ id: "errored-late", status: "Errored", is_running: false, last_modified: "2026-09-01 09:00" })],
    workflows: [item({ agentAlive: false, meta: { status: "implementing", updatedAt: early } })],
  });
  // Different bands here (stalled beats errored), so check the stamps landed
  // on one scale: the item's `since` must be the smaller number.
  assert.ok(rows[0].since < rows[1].since);
});

test("an undated row sorts last within its band", () => {
  const rows = attentionRows({
    sessions: [
      sess({ id: "undated", status: "Waiting", last_modified: "" }),
      sess({ id: "dated", status: "Waiting", last_modified: "2026-09-01 09:00" }),
    ],
  });
  assert.deepEqual(rows.map((r) => r.sessionId), ["dated", "undated"]);
});

test("the badge separates what blocks from what merely finished", () => {
  const rows = attentionRows({
    sessions: [
      sess({ id: "a", status: "Prompting" }),
      sess({ id: "b", status: "Waiting" }),
      sess({ id: "c", status: "Waiting" }),
    ],
    workflows: [item({ meta: { status: "plan-review" } })],
  });
  assert.deepEqual(attentionSummary(rows), { total: 4, blocking: 1 });
  assert.deepEqual(attentionSummary([]), { total: 0, blocking: 0 });
});

test("unread flags come from the caller's sets, not from the ordering", () => {
  const rows = attentionRows({
    sessions: [sess({ id: "s1", status: "Waiting" })],
    workflows: [item({ meta: { status: "plan-review" } })],
    unread: new Set(["s1"]),
    wfUnread: new Set(["proj/slug"]),
  });
  assert.ok(rows.every((r) => r.unread));
  const quiet = attentionRows({ sessions: [sess({ status: "Waiting" })] });
  assert.equal(quiet[0].unread, false);
});

test("the browser branch publishes every name app.js calls", () => {
  // attention.js is a plain script that app.js reads off `window`, so a rename
  // here — or a missing <script> tag — throws at click time while the boot line
  // still looks healthy (the wf-compose.js precedent).
  const fs = require("node:fs");
  const path = require("node:path");
  const vm = require("node:vm");

  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "attention.js"), "utf8");
  const win = {};
  // No `module` in scope → the IIFE takes its browser branch.
  vm.runInNewContext(src, { window: win });

  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  for (const name of ["attentionRows", "attentionSummary"]) {
    assert.equal(typeof win[name], "function", `${name} must be on window`);
    assert.ok(app.includes(name), `app.js is expected to use ${name}`);
  }

  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(
    html.indexOf("attention.js") < html.indexOf("app.js"),
    "attention.js must be loaded before app.js"
  );
  // The button the badge renders into has to exist, or renderInboxBadge is a
  // silent no-op and the count never appears.
  assert.ok(html.includes('id="inbox-btn"'), "index.html must hold #inbox-btn");
});

test("the inbox is wired into every path that can change it", () => {
  const fs = require("node:fs");
  const path = require("node:path");
  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  // A count that only updates when you open the tab is a count nobody trusts:
  // both refresh paths (sessions poll, workflow list) must repaint it.
  assert.match(app, /renderTabs\(\);\s*syncInbox\(\);/);
  assert.match(app, /updateWfBadge\(\);\s*\/\/[^\n]*\n\s*syncInbox\(\);/);
  // A persisted inbox tab must come back at boot like the other view tabs.
  assert.match(app, /if \(key === "view:inbox"\) \{\s*openInboxTab\(\);/);
  // Unanswered PR comments are one of its bands, so the lazy poll has to
  // treat the open inbox as "on screen".
  assert.match(app, /state\.open\.has\("view:inbox"\)/);
});

test("a red build on an open PR is its own band, after decisions and before PR threads", () => {
  const red = { url: "https://github.com/o/r/pull/1", state: "OPEN", health: { checks: "fail", failing: ["lint", "unit"] } };
  const reasons = workflowReasons(item({ meta: { status: "pr-ready", pr: { ...red, unansweredComments: 1 } } }));
  assert.deepEqual(
    reasons.map((r) => [r.band, r.text]),
    [
      [BAND.ciFailing, "CI failing — lint, unit"],
      [BAND.prComments, "1 unanswered PR comment"],
    ]
  );
  assert.ok(BAND.decision < BAND.ciFailing && BAND.ciFailing < BAND.prComments);
  // Two red PRs say so; a merged one, a green one or a finished item do not count.
  const two = workflowReasons(
    item({ meta: { status: "pr-ready", pr: red, linkedPrs: [{ ...red, url: "x", health: { checks: "fail" } }] } })
  );
  assert.equal(two[0].text, "CI failing on 2 PRs — lint, unit");
  assert.deepEqual(workflowReasons(item({ meta: { status: "pr-ready", pr: { ...red, state: "MERGED" } } })), []);
  assert.deepEqual(workflowReasons(item({ meta: { status: "pr-ready", pr: { ...red, health: { checks: "pass" } } } })), []);
  assert.deepEqual(workflowReasons(item({ meta: { status: "done", pr: red } })), []);
  // A row in the band, not blocking: the badge stays the blocked count.
  const rows = attentionRows({ workflows: [item({ meta: { status: "pr-ready", pr: red } })] });
  assert.equal(rows[0].band, BAND.ciFailing);
  assert.equal(attentionSummary(rows).blocking, 0);
});

// ── The inbox view: sections, filters, search, snooze, actions ─────────────

const mixed = () =>
  attentionRows({
    sessions: [
      sess({ id: "p", name: "approve", status: "Prompting", last_modified: "2026-09-01 08:00" }),
      sess({ id: "w", name: "idle", status: "Waiting", last_modified: "2026-09-01 09:00" }),
    ],
    workflows: [
      item({ slug: "d", meta: { title: "parked", status: "plan-review", updatedAt: 5 } }),
      item({ slug: "c", meta: { title: "red build", status: "pr-ready", pr: { url: "u1", state: "OPEN", health: { checks: "fail" } } } }),
    ],
  });

test("the inbox sorts its rows into sections by their most urgent reason, empty ones left out", () => {
  const v = inboxView(mixed(), { now: 0 });
  assert.deepEqual(
    v.groups.map((g) => [g.id, g.rows.map((r) => r.title)]),
    [
      ["now", ["approve"]],
      ["decisions", ["parked"]],
      ["prs", ["red build"]],
      ["turns", ["idle"]],
    ]
  );
  assert.deepEqual(v.counts, { all: 4, snoozed: 0, now: 1, decisions: 1, prs: 1, turns: 1 });
  assert.equal(v.total, 4);
  assert.equal(v.blocking, 1);
  // Every band has a section.
  for (const b of Object.values(BAND)) assert.ok(INBOX_GROUPS.some((g) => g.bands.includes(b)), `band ${b}`);
  // One item wanting two things stays one row, in its most urgent section.
  const two = attentionRows({ workflows: [item({ meta: { status: "diff-review", pr: { url: "u", unansweredComments: 2 } } })] });
  assert.equal(groupOf(two[0]).id, "decisions");
});

test("a filter shows one section; search narrows every count; an unknown filter is All", () => {
  const rows = mixed();
  assert.deepEqual(inboxView(rows, { filter: "prs", now: 0 }).groups.map((g) => g.id), ["prs"]);
  const q = inboxView(rows, { query: "RED", now: 0 });
  assert.deepEqual(q.groups.map((g) => g.rows.map((r) => r.title)), [["red build"]]);
  assert.equal(q.counts.all, 1);
  assert.equal(q.counts.turns, 0);
  // Reasons are searchable too.
  assert.equal(inboxView(rows, { query: "approval", now: 0 }).counts.all, 1);
  assert.equal(inboxView(rows, { filter: "nope", now: 0 }).filter, "all");
});

test("a snooze hides a row until it changes, or for an hour — and the count drops with it", () => {
  const rows = mixed();
  const idle = rows.find((r) => r.title === "idle");
  const until = { [idle.key]: snoozeEntry(idle, "change", 1000) };
  assert.ok(isSnoozed(until, idle, 10 ** 12), "until it changes: no clock");
  const v = inboxView(rows, { snoozes: until, now: 2000 });
  assert.equal(v.total, 3, "the badge counts what is not snoozed");
  assert.equal(v.counts.snoozed, 1);
  assert.deepEqual(inboxView(rows, { filter: "snoozed", snoozes: until, now: 2000 }).groups[0].rows.map((r) => r.title), ["idle"]);
  // New activity is news: the row comes back.
  const moved = { ...idle, since: idle.since + 60000 };
  assert.notEqual(rowSignature(moved), rowSignature(idle));
  assert.equal(isSnoozed(until, moved, 2000), false);
  // An hour's snooze ends on time, and on a change before that.
  const hour = { [idle.key]: snoozeEntry(idle, "hour", 0) };
  assert.equal(isSnoozed(hour, idle, 59 * 60 * 1000), true);
  assert.equal(isSnoozed(hour, idle, 60 * 60 * 1000), false);
  assert.equal(isSnoozed(hour, moved, 1), false);
  const { active, snoozed } = splitSnoozed(rows, until, 2000);
  assert.equal(active.length + snoozed.length, rows.length);
});

test("pruning keeps a snooze while it still matters, and while its row is only briefly absent", () => {
  const rows = mixed();
  const idle = rows.find((r) => r.title === "idle");
  const s = { [idle.key]: snoozeEntry(idle, "change", 0) };
  assert.deepEqual(Object.keys(pruneSnoozes(s, rows, 1)), [idle.key]);
  // Changed while on screen: gone.
  assert.deepEqual(pruneSnoozes(s, [{ ...idle, status: "Prompting" }], 1), {});
  // Absent (answered, or the list is reloading): kept for a week…
  assert.deepEqual(Object.keys(pruneSnoozes(s, [], 6 * 24 * 3600 * 1000)), [idle.key]);
  // …then forgotten; an hour's snooze is forgotten once it would have ended.
  assert.deepEqual(pruneSnoozes(s, [], 8 * 24 * 3600 * 1000), {});
  assert.deepEqual(pruneSnoozes({ [idle.key]: snoozeEntry(idle, "hour", 0) }, [], 2 * 3600 * 1000), {});
});

test("saved snoozes are read leniently: anything malformed is dropped", () => {
  assert.deepEqual(normalizeSnoozes(null), {});
  assert.deepEqual(normalizeSnoozes({ a: { sig: "x", until: null, at: 5 }, b: { sig: 1 }, c: null, d: { sig: "y", until: "soon" } }), {
    a: { sig: "x", until: null, at: 5 },
  });
  assert.deepEqual(normalizeSnoozes({ a: { sig: "x", until: 9 } }).a.at, 0);
});

test("each row offers the actions its reason calls for", () => {
  const rows = mixed();
  const by = (title) => rows.find((r) => r.title === title);
  assert.deepEqual(rowActions(by("idle"), { running: true }), ["followup", "stash"]);
  assert.deepEqual(rowActions(by("idle"), { running: false }), [], "a stopped session takes neither");
  assert.deepEqual(rowActions(by("approve"), { running: true }), [], "an approval is answered in the session");
  const errored = attentionRows({ sessions: [sess({ status: "Errored", is_running: false })] })[0];
  assert.deepEqual(rowActions(errored), ["reload"]);
  assert.deepEqual(rowActions(by("red build"), { hasPr: true }), ["open-pr"]);
  assert.deepEqual(rowActions(by("red build"), { hasPr: false }), []);
  assert.deepEqual(rowActions(by("parked"), { hasPr: true }), [], "a decision is not about a PR");
});

test("Open PR opens the PRs the row is about, primary first", () => {
  const meta = {
    pr: { url: "a", state: "OPEN", health: { checks: "pass" }, unansweredComments: 0 },
    linkedPrs: [
      { url: "b", state: "OPEN", health: { checks: "fail" } },
      { url: "c", state: "OPEN", unansweredComments: 3 },
      { url: "d", state: "MERGED", health: { checks: "fail" } },
    ],
  };
  assert.deepEqual(problemPrUrls(meta), ["b", "c"]);
  assert.deepEqual(problemPrUrls({}), []);
});

test("keyboard selection walks the drawn order and stops at both ends", () => {
  const keys = ["a", "b", "c"];
  assert.equal(inboxMove(keys, null, 1), "a");
  assert.equal(inboxMove(keys, "a", 1), "b");
  assert.equal(inboxMove(keys, "c", 1), "c");
  assert.equal(inboxMove(keys, "a", -1), "a");
  assert.equal(inboxMove(keys, "gone", -1), "a");
  assert.equal(inboxMove([], "a", 1), null);
});
