// node --test gui/tests/ — every link obeys the linkOpen setting.
//
// The bug this pins: half the URL-opening sites called `openBrowserTab`
// directly — the workflow PR buttons among them — so the setting worked for
// terminal links and was silently ignored everywhere else. That is invisible
// until someone who set "System browser" clicks a PR and lands in the panel.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");

/// Run the real openLink/openLinks against recording stubs.
function sandboxFor({ linkOpen = "ask", answer = "embedded", open = new Map() } = {}) {
  const calls = { tabs: [], external: [], asked: [] };
  const sandbox = {
    state: { settings: { linkOpen }, open },
    ws: () => ({ sessions: [...open.keys()] }),
    Set,
    Map,
    openBrowserTab: (url, mode) => calls.tabs.push([url, mode]),
    invoke: async (cmd, args) => {
      if (cmd === "open_external") calls.external.push(args.url);
      return null;
    },
    uiChoice: async (o) => {
      calls.asked.push(o);
      return answer;
    },
  };
  vm.createContext(sandbox);
  for (const fn of ["resolveLinkMode", "openLink", "openLinks"]) {
    vm.runInContext(extractFunction(APP, fn), sandbox);
  }
  vm.runInContext('const openExternal = (uri) => invoke("open_external", { url: uri });', sandbox);
  sandbox.__calls = calls;
  return sandbox;
}

test("the setting decides, without asking, when it names a destination", async () => {
  const embedded = sandboxFor({ linkOpen: "embedded" });
  await embedded.openLink("https://example.com/pr/1");
  assert.deepEqual(embedded.__calls.tabs, [["https://example.com/pr/1", "split"]]);
  assert.equal(embedded.__calls.asked.length, 0, "a decided setting never asks");

  const external = sandboxFor({ linkOpen: "external" });
  await external.openLink("https://example.com/pr/1");
  assert.deepEqual(external.__calls.external, ["https://example.com/pr/1"]);
  assert.equal(external.__calls.tabs.length, 0);
});

test("ask pops one question and honours the answer", async () => {
  const yes = sandboxFor({ linkOpen: "ask", answer: "external" });
  await yes.openLink("https://example.com/x");
  assert.equal(yes.__calls.asked.length, 1);
  assert.equal(yes.__calls.asked[0].detail, "https://example.com/x");
  assert.deepEqual(yes.__calls.external, ["https://example.com/x"]);

  // Dismissed: nothing opens anywhere.
  const no = sandboxFor({ linkOpen: "ask", answer: null });
  await no.openLink("https://example.com/x");
  assert.equal(no.__calls.tabs.length + no.__calls.external.length, 0);
});

test("a non-http scheme always goes to the OS, whatever the setting says", async () => {
  // The panel cannot render mailto:/tel:/file:, so the setting has no say.
  for (const linkOpen of ["embedded", "ask", "external"]) {
    const s = sandboxFor({ linkOpen });
    await s.openLink("mailto:someone@example.com");
    assert.deepEqual(s.__calls.external, ["mailto:someone@example.com"], linkOpen);
    assert.equal(s.__calls.tabs.length, 0, linkOpen);
    assert.equal(s.__calls.asked.length, 0, "nothing to decide");
  }
});

test("a batch asks once and keeps the multi-PR layout", async () => {
  // Three PRs for one click must not pop the same question three times.
  const s = sandboxFor({ linkOpen: "ask", answer: "embedded" });
  await s.openLinks(["https://x/pr/1", "https://x/pr/2", "https://x/pr/3"], "3 pull requests");
  assert.equal(s.__calls.asked.length, 1);
  assert.equal(s.__calls.asked[0].detail, "3 pull requests");
  // First takes a split pane, the rest land in the background.
  assert.deepEqual(s.__calls.tabs, [
    ["https://x/pr/1", "split"],
    ["https://x/pr/2", "background"],
    ["https://x/pr/3", "background"],
  ]);
});

test("a batch sent outside opens every one, and dedupes the list", async () => {
  const s = sandboxFor({ linkOpen: "external" });
  await s.openLinks(["https://x/pr/1", "https://x/pr/2", "https://x/pr/1"]);
  assert.deepEqual(s.__calls.external, ["https://x/pr/1", "https://x/pr/2"]);
  // A single-link batch is just a link — same path, same question.
  const one = sandboxFor({ linkOpen: "embedded" });
  await one.openLinks(["https://x/pr/9"]);
  assert.deepEqual(one.__calls.tabs, [["https://x/pr/9", "split"]]);
  // Nothing to open is not an error.
  const none = sandboxFor({});
  await none.openLinks([]);
  await none.openLinks(null);
  assert.equal(none.__calls.asked.length, 0);
});

test("a PR already open in this workspace is not opened twice", async () => {
  const open = new Map([
    ["browser-1", { kind: "browser", url: "https://x/pr/2" }],
  ]);
  const s = sandboxFor({ linkOpen: "embedded", open });
  await s.openLinks(["https://x/pr/1", "https://x/pr/2", "https://x/pr/3"]);
  assert.deepEqual(s.__calls.tabs, [
    ["https://x/pr/1", "split"], // split mode dedupes itself
    ["https://x/pr/3", "background"],
  ]);
});

test("no URL bypasses the decision", () => {
  // Only openLink/openLinks may reach openBrowserTab with a URL. The blank-tab
  // action passes null, and a link clicked *inside* the embedded browser stays
  // in the panel — that is the browser's own behaviour, not clash's setting.
  const allowed = new Set(["null", "uri", "url"]);
  const offenders = [];
  for (const m of APP.matchAll(/openBrowserTab\(([^,)]+)/g)) {
    const arg = m[1].trim();
    if (allowed.has(arg)) continue;
    const before = APP.slice(0, m.index);
    const fn = [...before.matchAll(/^(?:async )?function (\w+)\(/gm)].pop();
    offenders.push(`${fn ? fn[1] : "?"}: openBrowserTab(${arg}`);
  }
  assert.deepEqual(offenders, [], "these open a URL without consulting linkOpen");
});
