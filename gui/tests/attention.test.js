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
      "session:old-wait", // band 5, oldest
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
