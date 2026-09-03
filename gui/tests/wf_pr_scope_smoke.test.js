// node --test gui/tests/ — smoke-run the PR-scope surfaces out of app.js.
//
// The scope question sits between a click and a `gh` call on a real
// repository, and every part of it lives in app.js, which cannot be
// `require`d. Without this, a dangling reference in the picker or the per-PR
// menu surfaces as a context menu that opens empty and a button that silently
// does nothing, while clash.log still shows a healthy boot line.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
const SCOPE = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-pr-scope.js"), "utf8");
const PRS = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-prs.js"), "utf8");

const ITEM = {
  project: "p",
  slug: "s",
  openAnnotations: 0,
  lastAgentReview: { round: 3 },
  meta: {
    status: "pr-draft",
    repoPath: "/repo",
    pr: { url: "https://github.com/o/api/pull/1", number: 1, draft: true, state: "OPEN" },
    linkedPrs: [
      { url: "https://github.com/o/web/pull/2", number: 2, draft: true, state: "OPEN" },
      { url: "https://github.com/o/dto/pull/3", number: 3, state: "MERGED" },
    ],
  },
};

/// A sandbox holding the real pure modules (wf-prs + wf-pr-scope publish onto
/// the window/global) and stubs for everything app.js reaches for.
function sandbox(over = {}) {
  const box = {
    console,
    document: {
      createElement: () => stubEl(),
      createTextNode: () => stubEl(),
      body: { appendChild() {} },
      querySelectorAll: () => [],
    },
    invoke: async () => ({ round: 3, posted: [], failed: [], flipped: [], meta: ITEM.meta }),
    uiAlert: () => {},
    uiConfirm: async () => true,
    uiChoice: async () => null,
    uiListChoice: async () => null,
    uiCheckChoice: async () => null,
    uiPrompt: async () => null,
    flashToast: () => {},
    openLink: () => {},
    openLinks: () => {},
    refreshWorkflows: async () => {},
    buildWorkflowView: () => {},
    showContextMenu: () => {},
    wfGhHint: () => null,
    wfPrRecovery: async () => false,
    wfCanReview: () => true,
    launchWfReview: () => {},
    launchWfReviewRespond: () => {},
    hideBrowserWebviews: () => {},
    fitAll: () => {},
    setTimeout: () => {},
    ...over,
  };
  box.window = box;
  vm.createContext(box);
  vm.runInContext(PRS, box);
  vm.runInContext(SCOPE, box);
  return box;
}

test("the scope picker asks once, over the rows the action can act on", async () => {
  let asked = null;
  const box = sandbox({
    // Answer with every offered row: the "all" case.
    uiCheckChoice: async (args) => {
      asked = args;
      return args.items.map((i) => i.value);
    },
  });
  vm.runInContext(`pickPrScope = ${extractFunction(APP, "pickPrScope")}`, box);

  // Three PRs, one of them merged: mark-ready asks over the two drafts only.
  const sel = await box.pickPrScope(ITEM, "markReady");
  assert.ok(asked, "a multi-draft item must be asked");
  assert.equal(asked.items.length, 2, "one row per draft, no fan-out row");
  assert.match(asked.allLabel, /All 2 drafts/, "the all/none toggle names the set");
  // Only the primary starts checked — never another repository.
  assert.deepEqual(
    [...asked.items.filter((i) => i.checked).map((i) => i.value)],
    ["https://github.com/o/api/pull/1"]
  );
  assert.deepEqual([...sel.urls], [
    "https://github.com/o/api/pull/1",
    "https://github.com/o/web/pull/2",
  ]);
  assert.equal(sel.all, true);

  // A subset of exactly one linked PR — the answer one-or-all could not give.
  const subset = sandbox({
    uiCheckChoice: async () => ["https://github.com/o/web/pull/2"],
  });
  vm.runInContext(`pickPrScope = ${extractFunction(APP, "pickPrScope")}`, subset);
  const two = await subset.pickPrScope(ITEM, "markReady");
  assert.deepEqual([...two.urls], ["https://github.com/o/web/pull/2"]);
  assert.equal(two.all, false);

  // A single candidate resolves with no dialog at all.
  asked = null;
  const single = {
    ...ITEM,
    meta: { ...ITEM.meta, linkedPrs: [{ url: "x", state: "MERGED" }] },
  };
  const one = await box.pickPrScope(single, "markReady");
  assert.equal(asked, null, "nothing to choose must not open a dialog");
  assert.deepEqual([...one.urls], ["https://github.com/o/api/pull/1"]);

  // Cancelling resolves null, so callers stop instead of acting on a default.
  const cancelled = sandbox({ uiCheckChoice: async () => null });
  vm.runInContext(`pickPrScope = ${extractFunction(APP, "pickPrScope")}`, cancelled);
  assert.equal(await cancelled.pickPrScope(ITEM, "markReady"), null);
});

test("a respond round takes the whole picked set, in one round", async () => {
  // Answering the reviewers of a cross-repo change is one pass, not one pass
  // per repository — and the confirmation counts the threads of exactly the
  // PRs picked, not of the item.
  const spawned = [];
  const confirms = [];
  const box = sandbox({
    uiCheckChoice: async (args) => args.items.map((i) => i.value),
    uiConfirm: async (m) => {
      confirms.push(m);
      return true;
    },
    spawnWfReview: async (...args) => spawned.push(args),
    answerCommentsConfirm: (name, count) => `answer ${count} on ${name}?`,
    prChipLabel: (pr) => `chip${pr.number}`,
  });
  vm.runInContext(`pickPrScope = ${extractFunction(APP, "pickPrScope")}`, box);
  vm.runInContext(
    `launchWfReviewRespond = ${extractFunction(APP, "launchWfReviewRespond")}`,
    box
  );
  const withThreads = {
    ...ITEM,
    meta: {
      ...ITEM.meta,
      pr: { ...ITEM.meta.pr, unansweredComments: 2 },
      linkedPrs: [{ url: "https://github.com/o/web/pull/2", number: 2, unansweredComments: 3 }],
    },
  };
  await box.launchWfReviewRespond(withThreads, null);
  assert.match(confirms[0], /answer 5 on 2 pull requests\?/);
  const [, , depth, publish, opts] = spawned[0];
  assert.equal(depth, "standard");
  assert.equal(publish, "respond-pr-comments");
  assert.deepEqual([...opts.prUrls], [
    "https://github.com/o/api/pull/1",
    "https://github.com/o/web/pull/2",
  ]);
});

test("a no-candidate action says why instead of doing nothing", async () => {
  const alerts = [];
  const box = sandbox({ uiAlert: (m) => alerts.push(m) });
  vm.runInContext(`pickPrScope = ${extractFunction(APP, "pickPrScope")}`, box);
  const settled = {
    ...ITEM,
    meta: { pr: { url: "u", state: "MERGED" }, linkedPrs: [] },
  };
  assert.equal(await box.pickPrScope(settled, "markReady"), null);
  assert.equal(alerts.length, 1, "the human must be told there is no draft");
  assert.match(alerts[0], /draft/);
});

test("mark ready sends the picked URLs and reports what did not flip", async () => {
  const calls = [];
  const box = sandbox({
    uiCheckChoice: async (a) => a.items.map((i) => i.value), // every draft
    invoke: async (cmd, args) => {
      calls.push([cmd, args]);
      return {
        flipped: ["https://github.com/o/api/pull/1"],
        failed: ["https://github.com/o/web/pull/2: gh: not a draft"],
        meta: { ...ITEM.meta, status: "pr-ready" },
      };
    },
  });
  const alerts = [];
  box.uiAlert = (m) => alerts.push(m);
  const toasts = [];
  box.flashToast = (m) => toasts.push(m);
  vm.runInContext(`wfStatusInfo = () => ({ label: "PR DRAFT" })`, box);
  vm.runInContext(`pickPrScope = ${extractFunction(APP, "pickPrScope")}`, box);
  vm.runInContext(`wfMarkPrReady = ${extractFunction(APP, "wfMarkPrReady")}`, box);

  await box.wfMarkPrReady(ITEM, null);
  const [cmd, args] = calls[0];
  assert.equal(cmd, "mark_workflow_pr_ready");
  // The selection travels as URLs — the backend resolves them against the
  // item, so a stale frontend can never flip a PR nobody linked.
  assert.deepEqual([...args.prs], [
    "https://github.com/o/api/pull/1",
    "https://github.com/o/web/pull/2",
  ]);
  assert.equal(toasts.length, 1);
  assert.match(toasts[0], /api#1/);
  assert.equal(alerts.length, 1, "a partial failure must be reported");
  assert.match(alerts[0], /not a draft/);
});

test("a linked-only flip says the item stayed where it was", async () => {
  // Only the primary drives status; without saying so, a linked-only flip
  // reads as a stage transition that failed.
  const toasts = [];
  const box = sandbox({
    uiCheckChoice: async (a) => [a.items.at(-1).value], // the linked draft alone
    invoke: async () => ({
      flipped: ["https://github.com/o/web/pull/2"],
      failed: [],
      meta: { ...ITEM.meta, status: "pr-draft" },
    }),
    flashToast: (m) => toasts.push(m),
  });
  vm.runInContext(`wfStatusInfo = () => ({ label: "PR DRAFT" })`, box);
  vm.runInContext(`pickPrScope = ${extractFunction(APP, "pickPrScope")}`, box);
  vm.runInContext(`wfMarkPrReady = ${extractFunction(APP, "wfMarkPrReady")}`, box);
  await box.wfMarkPrReady(ITEM, null);
  assert.match(toasts[0], /stays at PR DRAFT/);
});

test("the per-PR menu builds for every PR without throwing", () => {
  const box = sandbox();
  vm.runInContext(`wfMarkPrReady = () => {}`, box);
  vm.runInContext(`publishWfReview = () => {}`, box);
  vm.runInContext(`wfPrMenu = ${extractFunction(APP, "wfPrMenu")}`, box);

  const prs = box.itemPrs(ITEM.meta);
  assert.equal(prs.length, 3);
  for (const pr of prs) {
    const entries = box.wfPrMenu(ITEM, pr, null).filter(Boolean);
    const labels = entries.map((e) => e.label);
    assert.ok(labels.includes("Open"), "every PR can be opened");
    assert.ok(labels.includes("Copy URL"));
    assert.ok(labels.includes("Refresh state"));
    // Mark ready only where it is a real act.
    const ready = labels.some((l) => l.startsWith("Mark "));
    assert.equal(ready, !!(pr.draft && pr.state !== "MERGED"), `mark-ready on ${pr.url}`);
    // Unlink is a linked PR's own lifecycle; the primary's is the pipeline.
    assert.equal(labels.includes("Unlink…"), !pr.primary);
    // Every entry can actually be invoked — a dangling reference in one is
    // exactly what this test exists to catch.
    for (const e of entries) assert.equal(typeof e.action, "function");
  }
});

test("the review composer opens with a scope group and pre-selects the pick", () => {
  const REVIEW = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-review.js"), "utf8");
  const box = sandbox({
    wfNextReviewRound: () => 2,
    wfReviewTarget: () => "diff",
    wfHasPr: () => true,
    wireBackdropDismiss: () => {},
    renderMarkdown: () => {},
  });
  vm.runInContext(REVIEW, box);
  vm.runInContext(`wfComposeReviewRound = ${extractFunction(APP, "wfComposeReviewRound")}`, box);

  // The model is what the dialog renders; assert on it directly for the two
  // cases that matter, then prove the dialog opens over them.
  const model = box.reviewRoundModel({
    target: "diff",
    hasPr: true,
    prs: box.itemPrs(ITEM.meta),
    prUrls: ["https://github.com/o/web/pull/2"],
  });
  assert.ok(model.prScope, "a multi-PR code round must offer a scope");
  assert.equal(model.prScope.choices.length, 3, "one row per PR");
  // The seeded PR is checked and the local-diff row is not — a scoped launch
  // must not also read the whole branch.
  assert.equal(model.prScope.local.checked, false);
  assert.deepEqual(
    [...model.prScope.choices.filter((c) => c.checked).map((c) => c.value)],
    ["https://github.com/o/web/pull/2"]
  );
  // Unscoped, the round reads the item's own diff — what it always read.
  const plain = box.reviewRoundModel({ target: "diff", prs: box.itemPrs(ITEM.meta) });
  assert.equal(plain.prScope.local.checked, true);
  assert.ok(plain.prScope.choices.every((c) => !c.checked));

  let rejected = null;
  const p = box.wfComposeReviewRound(ITEM, {
    prUrls: ["https://github.com/o/web/pull/2"],
  });
  p.catch((e) => (rejected = e));
  assert.equal(rejected, null, `the composer must open: ${rejected}`);
});
