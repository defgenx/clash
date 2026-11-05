// node --test gui/tests/ — the Settings modal's structure in index.html.
//
// Every control's binding finds it by id, so a setting dropped outside the
// modal, or a quick setting mirroring an id that moved, fails silently: the
// row renders and nothing happens when it changes.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const HTML = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
const paneStart = HTML.indexOf('<div id="settings-pane">');
const paneEnd = HTML.indexOf('id="settings-empty"');
const pane = HTML.slice(paneStart, paneEnd);

test("every setting control lives in a tab of the Settings modal", () => {
  assert.ok(paneStart > 0 && paneEnd > paneStart, "the modal's pane exists");
  const ids = [...HTML.matchAll(/id="(set-[a-z0-9-]+)"/g)].map((m) => m[1]);
  assert.ok(ids.length > 40, "the settings were found");
  const outside = ids.filter((id) => !pane.includes(`id="${id}"`));
  assert.deepEqual(outside, []);
});

test("tabs are unique and labelled", () => {
  const tabs = [...pane.matchAll(/class="settings-tab" data-tab="([a-z]+)" data-label="([^"]+)"/g)];
  assert.ok(tabs.length >= 5);
  const keys = tabs.map((t) => t[1]);
  assert.equal(new Set(keys).size, keys.length, "duplicate tab");
  assert.ok(keys.includes("general"), "the modal opens on General by default");
});

test("every quick setting mirrors a control that exists in the modal", () => {
  const mirrors = [...HTML.matchAll(/data-mirror="([a-z0-9-]+)"/g)].map((m) => m[1]);
  assert.ok(mirrors.length >= 3);
  for (const id of mirrors) assert.ok(pane.includes(`id="${id}"`), `${id} is not in the modal`);
  // The quick panel holds mirrors only: a real control there would be a
  // second element with a binding nobody wired.
  const quick = HTML.slice(HTML.indexOf('<div id="settings-body"'), HTML.indexOf('<div class="footer-row">'));
  assert.doesNotMatch(quick, /id="set-/);
});
