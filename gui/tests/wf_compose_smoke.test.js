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
const { extractFunction, stubEl } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");

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
    composerIntro: () => "",
    noteCaption: () => "Note",
    submitLabel: () => "Record round",
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
  // The scrim-dismissal helper is shared by every dialog builder: run the real
  // one rather than stubbing it, so a rename there still fails here.
  vm.runInContext(extractFunction(APP, "wireBackdropDismiss"), sandbox);
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
