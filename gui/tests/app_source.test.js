// node --test gui/tests/ — invariants over the frontend source itself.
//
// `app.js` is a plain script against the Tauri globals, so it cannot be
// `require`d in isolation. These are the properties worth pinning anyway: they
// are the ones whose violation is silent at runtime and expensive to debug.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const APP_PATH = path.join(__dirname, "..", "dist", "app.js");
const APP = fs.readFileSync(APP_PATH, "utf8");
const APP_BYTES = fs.readFileSync(APP_PATH);

test("app.js contains no NUL bytes", () => {
  // A single NUL makes grep treat the whole file as binary: `grep -n BROWSE
  // app.js` silently returns nothing without `-a`, so every future reader loses
  // time to a search that appears to prove the symbol doesn't exist.
  //
  // This assertion is only safe now that the repo picker's "Browse…" sentinel is
  // a Symbol. Deleting the byte while the sentinel was still "\x00browse" would
  // have demoted it to the plausible-as-a-path string "browse" — a weaker
  // invariant locked in by a passing test.
  const indexes = [];
  for (let i = 0; i < APP_BYTES.length; i++) {
    if (APP_BYTES[i] === 0) indexes.push(i);
  }
  assert.deepEqual(indexes, [], `NUL byte(s) at offset(s) ${indexes.join(", ")}`);
});

test("the repo picker sentinel is a Symbol, not a string", () => {
  // A string sentinel can collide with something a user types or pastes; the
  // picker compares by identity, so a Symbol cannot.
  assert.match(APP, /const BROWSE = Symbol\("browse"\)/);
  assert.doesNotMatch(APP, /const BROWSE = "/);
});

test("cross-frontend settings are not persisted in the GUI blob", () => {
  // The 7 shared settings live in config.toml from the migration onward. If the
  // blob kept a copy, a stale gui-state.json — or WKWebView's localStorage,
  // which resolves the real user home even under an isolated HOME — would
  // overwrite a hand-edited config.toml on the next boot.
  assert.match(APP, /settings: guiLocalSettings\(\)/);
  assert.doesNotMatch(APP, /^\s+settings: state\.settings,$/m);
  assert.match(APP, /function guiLocalSettings\(\)/);
});

test("only a disk blob may seed the settings migration", () => {
  // localStorage is a same-session fallback and is not HOME-isolated, so it must
  // never be a migration source.
  assert.match(APP, /applyWorkspacesData\(JSON\.parse\(raw\), \{ migratable: true \}\)/);
  assert.match(APP, /applyWorkspacesData\(JSON\.parse\(raw\), \{ migratable: false \}\)/);
});

test("settings are resolved from the backend before the first paint", () => {
  // applyTheme and syncSettingsUi both read state.settings; loading settings
  // after them would flash the default palette.
  const loadSettings = APP.indexOf("await loadSettings();");
  const applyTheme = APP.indexOf("applyTheme(state.settings.theme);\n  applyMonoFont();");
  assert.ok(loadSettings > 0, "boot must await loadSettings()");
  assert.ok(applyTheme > 0, "boot must apply the theme");
  assert.ok(loadSettings < applyTheme, "loadSettings() must precede applyTheme()");
});

test("a config reload coalesces refits to one animation frame", () => {
  // A reload can push xterm options to every open terminal and refit each one.
  // At the default 200ms watcher debounce, an editor saving per keystroke would
  // otherwise become a burst of refits across every pane.
  assert.match(APP, /function queueRefit\(\)/);
  assert.match(APP, /requestAnimationFrame\(\(\) => \{\s*refitQueued = false;/);
  // And the refit path is entered only for the keys the backend flagged.
  assert.match(APP, /payload\.refit && payload\.refit\.length\) queueRefit\(\)/);
});

test("shared settings persist through config.toml, GUI-local ones through the blob", () => {
  assert.match(APP, /function persistSetting\(key\)/);
  assert.match(APP, /if \(!sharedSettingKeys\.has\(key\)\) \{\s*saveWorkspaces\(\);/);
  assert.match(APP, /invoke\("config_set", \{ key, value: state\.settings\[key\] \}\)/);
  // The generic binders route through it rather than writing the blob directly.
  assert.match(APP, /else persistSetting\(key\);/);
});

test("the hand-written per-key settings validation is gone", () => {
  // One Rust validator now covers both frontends; these were the JS duplicates
  // of the schema's own range checks.
  assert.doesNotMatch(APP, /function numSetting\(/);
  assert.doesNotMatch(APP, /typeof s\.confirmKill === "boolean"/);
  assert.doesNotMatch(APP, /s\.embedLinks \? "embedded" : "external"/);
});

test("an early flush cannot persist unresolved defaults", () => {
  // blur/pagehide fire on any click away, including before boot finishes. At
  // that point state.settings still holds the in-code defaults, so persisting
  // them would let the next boot migrate defaults over the user's real values.
  assert.match(APP, /if \(!settingsResolved\) return loadedSettingsBlob \|\| \{\};/);
  assert.match(APP, /settingsResolved = true;/);
});
