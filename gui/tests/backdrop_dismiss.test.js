// node --test "gui/tests/*.test.js" — behaviour of the dialog scrim guard.
//
// The bug this pins is invisible to a source regex: a `click` handler on the
// scrim also fires when the press started inside a field and the release
// landed outside the box, which cancelled a rename dialog while the user was
// selecting the old name. app.js can't be require()d, so the helper is
// extracted and run against a minimal event target.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");

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

/// The scrim: records listeners, replays them like the DOM would.
function fakeBackdrop() {
  const listeners = {};
  return {
    addEventListener(type, fn) {
      (listeners[type] || (listeners[type] = [])).push(fn);
    },
    fire(type, target) {
      for (const fn of listeners[type] || []) fn({ target });
    },
  };
}

function wired() {
  const sandbox = {};
  vm.createContext(sandbox);
  vm.runInContext(extractFunction(APP, "wireBackdropDismiss"), sandbox);
  const backdrop = fakeBackdrop();
  let dismissed = 0;
  sandbox.wireBackdropDismiss(backdrop, () => dismissed++);
  return { backdrop, hits: () => dismissed };
}

test("a click that starts and ends on the scrim dismisses", () => {
  const { backdrop, hits } = wired();
  backdrop.fire("mousedown", backdrop);
  backdrop.fire("mouseup", backdrop);
  assert.equal(hits(), 1);
});

test("a selection drag out of a field does not dismiss", () => {
  const { backdrop, hits } = wired();
  const field = {}; // the dialog's <input>
  backdrop.fire("mousedown", field);
  backdrop.fire("mouseup", backdrop); // released past the box edge
  assert.equal(hits(), 0);
});

test("a press on the scrim released inside the box does not dismiss", () => {
  const { backdrop, hits } = wired();
  const box = {};
  backdrop.fire("mousedown", backdrop);
  backdrop.fire("mouseup", box);
  assert.equal(hits(), 0);
  // And the aborted press does not arm the next release.
  backdrop.fire("mouseup", backdrop);
  assert.equal(hits(), 0);
});

test("a later real outside click still dismisses", () => {
  const { backdrop, hits } = wired();
  const field = {};
  backdrop.fire("mousedown", field);
  backdrop.fire("mouseup", backdrop);
  backdrop.fire("mousedown", backdrop);
  backdrop.fire("mouseup", backdrop);
  assert.equal(hits(), 1);
});
