// node --test gui/tests/ — smoke-run the Files panel against a stub DOM.
//
// The panel repaints from a 4s poll and from renderTabs on every focus change,
// so a dangling name in its render path breaks far more than the panel. This
// executes the real bodies extracted from app.js with files.js's real helpers.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
const FILES = fs.readFileSync(path.join(__dirname, "..", "dist", "files.js"), "utf8");

const FUNCTIONS = [
  "sessionDir",
  "syncFilesRoot",
  "setFilesRoot",
  "refreshFiles",
  "toggleFilesDir",
  "runFilesFind",
  "visibleFileRows",
  "renderFilesPanel",
  "buildFileRow",
  "mentionTarget",
  "mentionInSession",
  "fileContextMenu",
  "moveFilesSelection",
  "onFilesKey",
  "openFilePreview",
  "renderFilePreview",
];

function sandboxFor({ replies = {}, sessions = [] } = {}) {
  const win = {};
  vm.runInNewContext(FILES, { window: win });
  const els = new Map();
  // The stub keeps children across `innerHTML = ""`; a real list does not.
  const list = stubEl();
  const clearing = new Proxy(list, {
    set(t, p, v) {
      if (p === "innerHTML") t.children.length = 0;
      t[p] = v;
      return true;
    },
  });
  els.set("files-list", clearing);
  const calls = [];
  const menus = [];
  const workspace = { panes: [null], focused: 0 };
  const sandbox = {
    ...win,
    state: { sessions, activeTab: null, open: new Map(), settings: {} },
    document: { createElement: () => stubEl(), hidden: false, activeElement: null },
    $: (id) => els.get(id) || (els.set(id, stubEl()), els.get(id)),
    svgIcon: () => "<svg/>",
    escapeHtml: (s) => String(s),
    displayName: (s) => s.name,
    dlog: () => {},
    flashToast: () => {},
    uiAlert: (m) => calls.push(["alert", m]),
    copyText: () => {},
    renderMarkdown: () => {},
    renderMermaidIn: () => {},
    openScratchInEditor: () => {},
    openSession: async () => {},
    pickDirectory: async () => null,
    ensureHighlight: async () => false,
    showContextMenu: (_x, _y, items) => menus.push(items),
    renderTabs: () => {},
    ensureFreePane: () => {
      if (workspace.panes[workspace.focused] != null) {
        workspace.panes.push(null);
        workspace.focused = workspace.panes.length - 1;
      }
    },
    ws: () => workspace,
    assignToFocusedPane: (key) => {
      workspace.panes[workspace.focused] = key;
      sandbox.state.activeTab = key;
    },
    openViewTab: (key, name, build) => {
      if (!sandbox.state.open.has(key)) {
        sandbox.state.open.set(key, { kind: "view", el: stubEl(), name });
      }
      sandbox.assignToFocusedPane(key);
      build(sandbox.state.open.get(key).el);
    },
    navigator: { platform: "MacIntel" },
    invoke: async (cmd, args) => {
      calls.push([cmd, args]);
      const r = replies[cmd];
      return typeof r === "function" ? r(args) : r;
    },
    FILES_FIND_LIMIT: 200,
    HIGHLIGHT_MAX_CHARS: 400000,
  };
  vm.createContext(sandbox);
  vm.runInContext(
    `var filesPanel = { open: true, root: null, rootSession: null, pinned: false,
      showIgnored: false, children: {}, expanded: new Set(), expandedByRoot: new Map(),
      status: null, index: null, query: "", matches: null, capped: false,
      selected: null, error: null, seq: 0, sig: "" };`,
    sandbox
  );
  for (const fn of FUNCTIONS) vm.runInContext(extractFunction(APP, fn), sandbox);
  return { sandbox, els, calls, menus, workspace };
}

const entry = (rel, isDir = false) => ({
  rel,
  name: rel.split("/").pop(),
  isDir,
  isSymlink: false,
  ignored: false,
  size: 12,
});

test("FUNCTIONS covers every function of the panel section", () => {
  const start = APP.indexOf("// ── Files panel ──");
  const end = APP.indexOf("const TASK_STATES");
  const declared = [...APP.slice(start, end).matchAll(/^(?:async )?function (\w+)\(/gm)].map(
    (m) => m[1]
  );
  const exempt = new Set([
    "toggleFilesPanel",
    "onFilesFilterInput",
    "openShellAt",
    "pickFilesRoot",
    "initFilesPanel",
    "ensureHighlight",
  ]);
  for (const name of declared) {
    assert.ok(FUNCTIONS.includes(name) || exempt.has(name), `${name} is not smoke-tested`);
  }
});

test("the panel follows the focused session and renders the tree with marks", async () => {
  const { sandbox, els } = sandboxFor({
    sessions: [{ id: "s1", name: "work", cwd: "/r/clash" }],
    replies: {
      explorer_list: ({ dirs }) =>
        Object.fromEntries(
          dirs.map((d) => [d, d === "" ? [entry("src", true), entry("README.md")] : [entry("src/a.rs")]])
        ),
      explorer_git_status: {
        branch: "main",
        ahead: 1,
        behind: 0,
        entries: [{ rel: "src/a.rs", mark: "modified", staged: false }],
      },
      explorer_read: { kind: "text", size: 1, text: "x" },
    },
  });
  sandbox.state.activeTab = "s1";
  sandbox.syncFilesRoot();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(sandbox.filesPanel.root, "/r/clash");
  assert.equal(sandbox.filesPanel.rootSession, "s1");
  assert.match(els.get("files-branch").textContent, /main ↑1/);
  let rows = els.get("files-list").children;
  assert.deepEqual(rows.map((r) => r.dataset.rel), ["src", "README.md"]);
  // The collapsed folder carries the "contains changes" dot.
  assert.ok(descendants(rows[0]).some((c) => c.textContent === "•"));

  await sandbox.toggleFilesDir("src");
  rows = els.get("files-list").children;
  assert.deepEqual(rows.map((r) => r.dataset.rel), ["src", "src/a.rs", "README.md"]);
  assert.ok(descendants(rows[1]).some((c) => c.textContent === "M"));

  // Keyboard: ↓ selects, Enter on a file opens its preview.
  const key = (k, extra = {}) => sandbox.onFilesKey({ key: k, preventDefault() {}, stopPropagation() {}, ...extra });
  key("ArrowDown");
  key("ArrowDown");
  assert.equal(sandbox.filesPanel.selected, "src/a.rs");
  key("Enter");
  assert.ok(sandbox.state.open.has("view:file:/r/clash/src/a.rs"));
  await new Promise((r) => setTimeout(r, 0));
});

test("previews share one pane and a click replaces a transient one", async () => {
  const { sandbox, workspace } = sandboxFor({
    replies: { explorer_read: { kind: "text", size: 3, text: "a\nb\n", path: "/x" } },
  });
  workspace.panes = ["s1"];
  sandbox.openFilePreview("/r/a.rs");
  assert.deepEqual(workspace.panes, ["s1", "view:file:/r/a.rs"]);
  sandbox.openFilePreview("/r/b.rs");
  assert.deepEqual(workspace.panes, ["s1", "view:file:/r/b.rs"]);
  assert.ok(!sandbox.state.open.has("view:file:/r/a.rs"), "transient preview was replaced");
  sandbox.openFilePreview("/r/b.rs", { keep: true });
  sandbox.openFilePreview("/r/c.rs");
  assert.ok(sandbox.state.open.has("view:file:/r/b.rs"), "kept preview survives");
  assert.deepEqual(workspace.panes, ["s1", "view:file:/r/c.rs"]);
  await new Promise((r) => setTimeout(r, 0));
});

test("every preview kind renders without throwing", async () => {
  for (const p of [
    { kind: "text", size: 5, text: "fn main() {}\n" },
    { kind: "markdown", size: 5, text: "# hi" },
    { kind: "image", size: 5, base64: "AAAA", mime: "image/png" },
    { kind: "binary", size: 5 },
    { kind: "too-large", size: 5e7 },
  ]) {
    const { sandbox } = sandboxFor({ replies: { explorer_read: p } });
    await sandbox.renderFilePreview(stubEl(), "/r/main.rs");
  }
  const { sandbox } = sandboxFor({
    replies: {
      explorer_read: () => {
        throw "gone";
      },
    },
  });
  await sandbox.renderFilePreview(stubEl(), "/r/missing.rs");
});

test("finder results render, and mention types into the running session", async () => {
  const { sandbox, els, calls, menus } = sandboxFor({
    sessions: [{ id: "s1", name: "work", cwd: "/r/clash" }],
    replies: {
      explorer_find: { matches: [{ rel: "src/app.rs", score: 9, positions: [4, 5, 6] }], capped: true },
      send_input: null,
    },
  });
  sandbox.filesPanel.root = "/r/clash";
  sandbox.filesPanel.query = "app";
  await sandbox.runFilesFind();
  const rows = els.get("files-list").children;
  assert.equal(rows[0].dataset.rel, "src/app.rs");
  assert.equal(sandbox.filesPanel.selected, "src/app.rs");

  sandbox.state.activeTab = "s1";
  sandbox.state.open.set("s1", { kind: "claude", term: {}, el: stubEl() });
  await sandbox.mentionInSession("/r/clash/src/app.rs");
  assert.equal(
    JSON.stringify(calls.find((c) => c[0] === "send_input")),
    JSON.stringify(["send_input", { sessionId: "s1", text: "@src/app.rs " }])
  );

  sandbox.fileContextMenu({ ...entry("src", true) }, 0, 0);
  sandbox.fileContextMenu(entry("src/app.rs"), 0, 0);
  assert.ok(menus[1].some((it) => it && it.label === "Mention in session"));
});

test("a mention with no running session explains instead of failing", async () => {
  const { sandbox, calls } = sandboxFor();
  await sandbox.mentionInSession("/r/a.rs");
  assert.ok(!calls.some((c) => c[0] === "send_input"));
});
