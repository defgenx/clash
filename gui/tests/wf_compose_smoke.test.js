// node --test gui/tests/ — smoke-open the change-request composer.
//
// app.js can't be require()d, so its dialog builders are normally only
// executed by a click in the real webview — where a dangling reference
// (`openCount` after a refactor, the bug this pins) surfaces as an unhandled
// rejection in clash.log and a button that silently does nothing, while the
// boot line still looks healthy. This test extracts wfComposeChangeRequest
// from the source and runs its open path against a stub DOM: any
// ReferenceError at open time fails here instead of at click time.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");

/// Extract a top-level `function name(...) {...}` by brace matching, starting
/// at the BODY brace (the parameter list may hold a destructuring `{…}` of
/// its own, which a count-from-first-brace would mistake for the body).
function extractFunction(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, `${name} must exist in app.js`);
  const body = /\)\s*\{/.exec(source.slice(start));
  assert.ok(body, `${name} must have a body`);
  let i = start + body.index + body[0].length - 1;
  let depth = 0;
  for (; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) break;
  }
  return source.slice(start, i + 1);
}

/// A DOM-element stand-in that absorbs any property set, hands out child
/// stubs, and no-ops the methods the dialog builders call.
function stubEl() {
  const target = {};
  return new Proxy(target, {
    get(t, p) {
      if (p === "classList") return { add() {}, remove() {}, toggle() {} };
      if (p === "style" || p === "dataset") return t[p] || (t[p] = {});
      if (
        [
          "appendChild",
          "append",
          "remove",
          "focus",
          "setSelectionRange",
          "addEventListener",
          "dispatchEvent",
        ].includes(p)
      )
        return () => {};
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

test("the change-request composer opens without throwing", async () => {
  const src = extractFunction(APP, "wfComposeChangeRequest");
  const placeholderCalls = [];
  const sandbox = {
    document: {
      createElement: () => stubEl(),
      createTextNode: () => stubEl(),
      body: { appendChild() {} },
      querySelectorAll: () => [],
    },
    wfDrafts: new Map(),
    draftKey: (a, b) => `${a}/${b}`,
    composerPlaceholder: (t, c) => {
      placeholderCalls.push([t, c]);
      return "";
    },
    changeRequestTemplate: () => "",
    annotationsMarkdown: () => "",
    canSubmitChangeRequest: () => ({ ok: true }),
    agentReviewRounds: () => [],
    roundFindings: () => null,
    interactiveParam: () => null,
    renderMarkdown: () => {},
    invoke: async () => ({}),
    uiConfirm: async () => false,
    uiChoice: async () => null,
    uiAlert: () => {},
    flashToast: () => {},
    dlog: () => {},
    hideBrowserWebviews: () => {},
    fitAll: () => {},
    setTimeout: () => {},
    CSS: { escape: (s) => s },
  };
  vm.createContext(sandbox);
  vm.runInContext(`opened = ${src}`, sandbox);

  // The open path must not throw for any target / annotation mix — a
  // rejection here is the "button silently does nothing" bug.
  for (const [target, annotations] of [
    ["diff", [{ id: "a1", file: "f.rs", line: 3, body: "x", status: "open" }]],
    ["diff", []],
    ["plan", []],
  ]) {
    let rejected = null;
    const p = sandbox.opened({
      item: {
        project: "p",
        slug: "s",
        meta: { iteration: 1, reviewRound: 0 },
        hasAgentReview: false,
      },
      target,
      annotations,
      onJump: () => {},
    });
    p.catch((e) => (rejected = e));
    await new Promise((r) => process.nextTick(r));
    assert.equal(rejected, null, `composer open rejected for ${target}: ${rejected}`);
  }
  // And the placeholder got a real count, not an undefined leftover.
  assert.ok(placeholderCalls.length >= 3);
  for (const [, count] of placeholderCalls) {
    assert.equal(typeof count, "number", "placeholder count must be a number");
  }
});
