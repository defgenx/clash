// node --test gui/tests/ — smoke-render the inbox against a stub DOM.
//
// `syncInbox()` runs inside `refreshSessions`' try block on every poll tick, so
// a dangling reference in the render path does not just break the tab: it
// aborts the rest of the refresh (details panel, teams) every two seconds,
// while clash.log still shows a healthy boot line. This executes the real
// `renderInbox` / `renderInboxBadge` / `inboxRows` bodies extracted from
// app.js, so any name that is not actually in scope fails here.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
const ATTENTION = fs.readFileSync(path.join(__dirname, "..", "dist", "attention.js"), "utf8");

/// Extract a top-level `function name(...) {...}` by brace matching from the
/// body brace (the wf_compose_smoke precedent).
function extractFunction(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, `${name} must exist in app.js`);
  const body = /\)\s*\{/.exec(source.slice(start));
  let i = start + body.index + body[0].length - 1;
  let depth = 0;
  for (; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) break;
  }
  return source.slice(start, i + 1);
}

function stubEl() {
  const target = { children: [] };
  return new Proxy(target, {
    get(t, p) {
      if (p === "classList") return { add() {}, remove() {}, toggle() {} };
      if (p === "style" || p === "dataset") return t[p] || (t[p] = {});
      if (p === "appendChild" || p === "append") return (c) => t.children.push(c);
      if (["remove", "focus", "addEventListener", "prepend"].includes(p)) return () => {};
      if (p === "querySelector") return () => stubEl();
      if (p === "querySelectorAll") return () => [];
      return t[p];
    },
    set(t, p, v) {
      t[p] = v;
      return true;
    },
  });
}

function sandboxFor(state) {
  const win = {};
  vm.runInNewContext(ATTENTION, { window: win });
  const button = stubEl();
  const sandbox = {
    state,
    attentionRows: win.attentionRows,
    attentionSummary: win.attentionSummary,
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
  for (const fn of ["inboxRows", "renderInbox", "renderInboxBadge"]) {
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
  // Header plus one row per attention source, all five of them.
  assert.equal(el.children[0].children.length, 6);
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
  const row = el.children[0].children[1];
  // The button is appended after the row's innerHTML, so it lands as a child.
  assert.equal(row.children.length, 1);
  assert.equal(row.children[0].textContent, "Queue follow-up…");
});
