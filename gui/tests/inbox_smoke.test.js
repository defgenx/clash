// node --test gui/tests/ — smoke-render the inbox against a stub DOM.
//
// `syncInbox()` runs inside `refreshSessions`' try block on every poll tick, so
// a dangling reference in the render path does not just break the tab: it
// aborts the rest of the refresh (details panel, teams) every two seconds,
// while clash.log still shows a healthy boot line. This executes the real
// `renderInbox` / `renderInboxBadge` / `inboxRows` bodies extracted from
// app.js — the shell, the sections, every row's buttons and the keyboard —
// so any name that is not actually in scope fails here.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
const ATTENTION = fs.readFileSync(path.join(__dirname, "..", "dist", "attention.js"), "utf8");

function sandboxFor(state) {
  const win = {};
  vm.runInNewContext(ATTENTION, { window: win });
  const button = stubEl();
  const sandbox = {
    state,
    ...win,
    inboxFilter: { filter: "all", query: "" },
    inboxSnoozes: {},
    inboxSelected: null,
    saveWorkspaces: () => {},
    syncInbox: () => {},
    showContextMenu: () => {},
    dropTerminal: () => {},
    refreshSessions: () => {},
    reloadSessionInteractive: () => {},
    openLinks: () => {},
    document: { createElement: () => stubEl(), createTextNode: () => stubEl() },
    $: (id) => (id === "inbox-btn" ? button : stubEl()),
    svgIcon: () => "<svg/>",
    escapeHtml: (s) => String(s),
    wfAgo: () => "2m ago",
    statusInfo: () => ({ cls: "waiting", icon: "◉", label: "WAITING" }),
    wfStatusInfo: () => ({ cls: "wf-review", icon: "◆", label: "PLAN REVIEW" }),
    openSession: () => {},
    openWorkflowTab: () => {},
    wfItem: () => null,
    queueFollowUp: () => {},
    renderSidebar: () => {},
    invoke: async () => ({}),
  };
  vm.createContext(sandbox);
  for (const fn of [
    "inboxRows",
    "inboxActiveRows",
    "snoozeInboxRow",
    "unsnoozeInboxRow",
    "inboxSnoozeMenu",
    "openInboxRow",
    "inboxRowButtons",
    "buildInboxShell",
    "inboxKey",
    "renderInbox",
    "renderInboxBadge",
  ]) {
    vm.runInContext(extractFunction(APP, fn), sandbox);
  }
  sandbox.__button = button;
  return sandbox;
}

const baseState = () => ({
  sessions: [],
  workflows: [],
  unread: new Set(),
  wfUnread: new Set(),
  open: new Map(),
  queued: {},
});

test("the inbox renders an empty state without throwing", () => {
  const sandbox = sandboxFor(baseState());
  sandbox.renderInbox(stubEl());
  sandbox.renderInboxBadge();
  assert.equal(sandbox.__button.innerHTML.includes("<svg/>"), true);
  // No pill and no title claiming work when there is none.
  assert.ok(!/inbox-pill/.test(sandbox.__button.innerHTML));
  assert.match(sandbox.__button.title, /nothing waiting/);
});

test("the inbox renders every row kind without throwing", () => {
  const state = baseState();
  state.sessions = [
    {
      id: "s1",
      name: "approve me",
      project: "proj",
      status: "Prompting",
      is_running: true,
      last_modified: "2026-09-01 09:00",
    },
    {
      id: "s2",
      name: "idle one",
      project: "proj",
      status: "Waiting",
      is_running: true,
      last_modified: "2026-09-01 08:00",
    },
    {
      id: "s3",
      name: "broken",
      project: "proj",
      status: "Errored",
      is_running: false,
      last_modified: "2026-09-01 07:00",
    },
  ];
  state.workflows = [
    {
      project: "p",
      slug: "a",
      agentAlive: false,
      meta: { title: "wedged", status: "implementing", updatedAt: 1 },
    },
    {
      project: "p",
      slug: "b",
      meta: {
        title: "parked",
        status: "diff-review",
        updatedAt: 2,
        pr: { url: "u", unansweredComments: 2 },
      },
    },
  ];
  const sandbox = sandboxFor(state);
  const el = stubEl();
  sandbox.renderInbox(el);
  // Header, then the body: three sections (now, decisions, turns), each a
  // label over its rows — one row per attention source, all five of them.
  const body = el.children[1];
  const labels = body.children.filter((c) => c.className === "inbox-group");
  const rows = body.children.filter((c) => String(c.className).startsWith("inbox-row"));
  assert.equal(labels.length, 3);
  assert.equal(rows.length, 5);
  // A repaint reuses the header instead of rebuilding it (search keeps focus).
  const shell = el._inbox;
  sandbox.renderInbox(el);
  assert.equal(el._inbox, shell);
  sandbox.renderInboxBadge();
  assert.match(sandbox.__button.innerHTML, /inbox-pill hot">5</);
  assert.match(sandbox.__button.title, /5 waiting on you, 3 blocked/);
});

test("a running session row offers the follow-up composer", () => {
  const state = baseState();
  state.sessions = [
    { id: "s1", name: "idle", project: "p", status: "Waiting", is_running: true },
  ];
  const sandbox = sandboxFor(state);
  const el = stubEl();
  sandbox.renderInbox(el);
  const row = el.children[1].children[1];
  // Buttons are appended after the row's innerHTML, so they land as children.
  assert.deepEqual(
    row.children.map((b) => b.textContent),
    ["Queue follow-up…", "Stash", "Snooze ▾"]
  );
});

test("a snoozed row leaves the list and the badge, and comes back on its own filter", () => {
  const state = baseState();
  state.sessions = [{ id: "s1", name: "idle", project: "p", status: "Waiting", is_running: true }];
  const sandbox = sandboxFor(state);
  const row = sandbox.inboxRows()[0];
  sandbox.inboxSnoozes = { [row.key]: sandbox.snoozeEntry(row, "change", Date.now()) };
  const el = stubEl();
  sandbox.renderInbox(el);
  assert.equal(el.children[1].children[0].className, "hint", "nothing left to show");
  sandbox.renderInboxBadge();
  assert.ok(!/inbox-pill/.test(sandbox.__button.innerHTML), "the badge drops with it");
  sandbox.inboxFilter.filter = "snoozed";
  sandbox.renderInbox(el);
  // The stub keeps children across `innerHTML = ""`, so the row is the last one.
  const shown = el.children[1].children.at(-1);
  assert.equal(shown.children.at(-1).textContent, "Unsnooze");
});

test("the inbox's keys move, open and snooze — and stop before the window's shortcuts", () => {
  const state = baseState();
  state.sessions = [
    { id: "a", name: "first", project: "p", status: "Prompting", is_running: true, last_modified: "2026-09-01 07:00" },
    { id: "b", name: "second", project: "p", status: "Waiting", is_running: true, last_modified: "2026-09-01 08:00" },
  ];
  const sandbox = sandboxFor(state);
  let opened = null;
  sandbox.openSession = (id) => (opened = id);
  const el = stubEl();
  sandbox.renderInbox(el);
  let stopped = 0;
  const press = (key) =>
    sandbox.inboxKey({ key, preventDefault() {}, stopPropagation() { stopped++; } }, el);
  press("j");
  assert.equal(sandbox.inboxSelected, "session:a");
  press("ArrowDown");
  assert.equal(sandbox.inboxSelected, "session:b");
  press("Enter");
  assert.equal(opened, "b");
  press("z");
  assert.ok(sandbox.inboxSnoozes["session:b"], "z snoozes the selected row");
  press("q");
  assert.equal(stopped, 4, "a key the inbox does not use is left to the window");
});
