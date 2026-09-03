// node --test gui/tests/ — the PR action scope model.
//
// The properties here are the ones whose violation is a wrong action on a
// real repository: a "mark ready" offered for a merged PR, a fan-out offered
// for an action that can only run once, or a dialog asked when there is
// nothing to choose.
const { test } = require("node:test");
const assert = require("node:assert/strict");

const {
  prActionCandidates,
  prScopeModel,
  prScopeSelection,
  prScopeSummary,
  prScopeSuffix,
  prLive,
} = require("../dist/wf-pr-scope.js");

const pr = (over = {}) => ({
  url: "https://github.com/o/api/pull/7",
  number: 7,
  repo: "o/api",
  draft: false,
  state: "OPEN",
  unanswered: null,
  primary: false,
  ...over,
});

const THREE = [
  pr({ url: "https://github.com/o/api/pull/1", number: 1, repo: "o/api", primary: true, draft: true }),
  pr({ url: "https://github.com/o/web/pull/2", number: 2, repo: "o/web", draft: true }),
  pr({ url: "https://github.com/o/dto/pull/3", number: 3, repo: "o/dto", state: "MERGED" }),
];

test("mark-ready only ever offers PRs that are actually drafts", () => {
  // A non-draft is already ready and a merged one cannot go back: offering
  // either is offering a `gh` failure as a choice.
  const c = prActionCandidates(THREE, "markReady");
  assert.deepEqual(
    c.map((p) => p.number),
    [1, 2]
  );
  // Reading actions stay available on settled PRs — "what shipped" is a
  // legitimate thing to open or post a round to.
  assert.equal(prActionCandidates(THREE, "open").length, 3);
  assert.equal(prActionCandidates(THREE, "post").length, 3);
  assert.equal(prLive(THREE[2]), false);
});

test("every action's answer is a set — agent rounds included", () => {
  // An earlier version excluded review/respond on the grounds that "a round
  // is one agent session parked on one item". That is a constraint on agent
  // concurrency, not on how many PRs one round may read — and a cross-repo
  // review is the case that needs the set most, since judging the API PR
  // without the web PR that consumes it leaves the contract unchecked.
  for (const action of ["open", "markReady", "post", "review", "respond"]) {
    const m = prScopeModel(THREE, action);
    const candidates = prActionCandidates(THREE, action);
    assert.equal(m.rows.length, candidates.length, `${action} rows`);
    // Every row is individually selectable, and "all" is expressible.
    const all = prScopeSelection(m, candidates.map((p) => p.url));
    assert.equal(all.all, true, `${action} must be able to cover every candidate`);
    // …and so is a subset of exactly two, which one-or-all could not say.
    if (candidates.length > 2) {
      const two = prScopeSelection(m, [candidates[0].url, candidates[2].url]);
      assert.equal(two.urls.length, 2);
      assert.equal(two.all, false);
    }
  }
});

test("a selection keeps the item's own PR order, not the click order", () => {
  // A fan-out must not depend on which box was ticked first: the primary
  // leads, then the linked PRs as the item records them.
  const m = prScopeModel(THREE, "open");
  const sel = prScopeSelection(m, [THREE[2].url, THREE[0].url]);
  assert.deepEqual(sel.urls, [THREE[0].url, THREE[2].url]);
  // An unknown URL contributes nothing rather than travelling to the backend.
  assert.deepEqual(prScopeSelection(m, ["https://x/y"]).urls, []);
  assert.equal(prScopeSelection(m, []).all, false);
});

test("mark-ready's rows and 'all' label cover exactly the drafts", () => {
  // Flipping a merged PR is an error, and the count on the label is what the
  // human is agreeing to.
  const m = prScopeModel(THREE, "markReady");
  assert.deepEqual(
    m.rows.map((r) => r.url),
    [THREE[0].url, THREE[1].url]
  );
  assert.match(m.allLabel, /All 2 drafts/);
});

test("the default selection is the answer the button already promised", () => {
  // Never a linked repository: announcing another repo is precisely the thing
  // the human has to choose deliberately.
  for (const action of ["markReady", "post", "respond"]) {
    const m = prScopeModel(THREE, action);
    const checked = m.rows.filter((r) => r.checked).map((r) => r.url);
    assert.deepEqual(checked, [THREE[0].url], `${action} must pre-check the primary alone`);
  }
  // Open is the exception: read-only, and its label promises every PR.
  const open = prScopeModel(THREE, "open");
  assert.ok(open.rows.every((r) => r.checked), "open pre-checks everything");

  // A caller that already knows the selection seeds it instead.
  const seeded = prScopeModel(THREE, "open", { selected: [THREE[1].url] });
  assert.deepEqual(
    seeded.rows.filter((r) => r.checked).map((r) => r.url),
    [THREE[1].url]
  );
});

test("a respond round pre-ticks the PRs that actually have threads waiting", () => {
  // The button advertises the item's whole unanswered count, so pre-ticking
  // the primary alone would open the dialog agreeing to 2 of the 5 it just
  // promised. A reply is not a release — nothing here is announced by accident.
  const waiting = [
    pr({ url: "https://github.com/o/api/pull/1", primary: true, unanswered: 0 }),
    pr({ url: "https://github.com/o/web/pull/2", unanswered: 3 }),
    pr({ url: "https://github.com/o/dto/pull/3", unanswered: 2 }),
  ];
  assert.deepEqual(
    prScopeModel(waiting, "respond").rows.filter((r) => r.checked).map((r) => r.url),
    [waiting[1].url, waiting[2].url]
  );
  // Mark-ready is unaffected: a waiting thread says nothing about readiness.
  const drafts = waiting.map((p) => ({ ...p, draft: true }));
  assert.deepEqual(
    prScopeModel(drafts, "markReady").rows.filter((r) => r.checked).map((r) => r.url),
    [drafts[0].url]
  );

  // Counts unfetched (gh unavailable, no refresh yet) is not "zero waiting":
  // fall back to the primary rather than open with nothing ticked.
  const unknown = [
    pr({ url: "https://github.com/o/api/pull/1", primary: true }),
    pr({ url: "https://github.com/o/web/pull/2" }),
  ];
  assert.deepEqual(
    prScopeModel(unknown, "respond").rows.filter((r) => r.checked).map((r) => r.url),
    [unknown[0].url]
  );
});

test("nothing is asked when there is nothing to choose", () => {
  // A single-PR item must behave exactly as it did before the scope existed:
  // no dialog, the one candidate resolved straight through.
  const one = prScopeModel([THREE[0]], "markReady");
  assert.equal(one.needed, false);
  assert.deepEqual(one.only, { all: true, urls: [THREE[0].url] });
  assert.equal(prScopeSuffix([THREE[0]], "markReady"), "");
  // …and the ellipsis appears exactly when the click will ask.
  assert.equal(prScopeSuffix(THREE, "markReady"), "…");
  assert.equal(prScopeSuffix(THREE, "open"), "…");

  // No candidate at all: `only` is null and the caller has copy to explain
  // why — an action that silently does nothing is worse than one that says so.
  const settled = prScopeModel([THREE[2]], "markReady");
  assert.equal(settled.needed, false);
  assert.equal(settled.only, null);
  assert.match(settled.empty, /draft/);
});

test("rows name the primary, since it is the only PR that moves the item", () => {
  const m = prScopeModel(THREE, "open");
  const primaryRow = m.rows.find((r) => r.url === THREE[0].url);
  assert.match(primaryRow.label, /primary/);
  assert.doesNotMatch(m.rows.find((r) => r.url === THREE[1].url).label, /primary/);
  // Details tell two PRs of one item apart without opening either.
  assert.match(primaryRow.detail, /draft/);
  assert.match(primaryRow.detail, /pull\/1/);
});

test("unanswered threads ride the row, so a respond pick is informed", () => {
  const withThreads = [
    pr({ url: "https://github.com/o/api/pull/1", primary: true, unanswered: 0 }),
    pr({ url: "https://github.com/o/web/pull/2", unanswered: 4 }),
  ];
  const m = prScopeModel(withThreads, "respond");
  assert.doesNotMatch(m.rows[0].detail, /unanswered/); // 0 is not a count worth showing
  assert.match(m.rows[1].detail, /4 unanswered/);
});

test("an outcome names what it acted on, not just 'the PR'", () => {
  // "PR ready" on a three-repo item said nothing about which repo.
  // "all 3" and "3 of 4" are different agreements — the confirmation says which.
  assert.equal(
    prScopeSummary({ all: true, urls: THREE.map((p) => p.url) }, THREE),
    "all 3 pull requests"
  );
  assert.equal(
    prScopeSummary({ all: false, urls: [THREE[0].url, THREE[1].url] }, THREE),
    "2 pull requests"
  );
  assert.equal(prScopeSummary({ urls: [THREE[1].url] }, THREE), "web#2");
  // An unknown URL still produces something a human can read.
  assert.equal(prScopeSummary({ urls: ["https://x/y"] }, THREE), "https://x/y");
  assert.equal(prScopeSummary({ urls: [] }, THREE), "the PR");
});

test("an unknown action acts on nothing rather than on everything", () => {
  // A typo'd action name must not silently resolve to "all PRs" — that is a
  // fan-out nobody asked for.
  assert.deepEqual(prActionCandidates(THREE, "markRedy"), []);
  const m = prScopeModel(THREE, "markRedy");
  assert.equal(m.rows.length, 0);
  assert.equal(m.needed, false);
  assert.equal(m.only, null);
  assert.deepEqual(prActionCandidates(undefined, "open"), []);
});

test("the browser branch publishes every name app.js calls", () => {
  // app.js is a plain script that reads these off `window`, so a rename here
  // (or a missing `<script>` tag) fails at click time, not at boot — the
  // frontend logs a clean "booted" line and the button just throws.
  const fs = require("node:fs");
  const path = require("node:path");
  const vm = require("node:vm");

  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-pr-scope.js"), "utf8");
  const win = {};
  // No `module` in scope → the IIFE takes its browser branch.
  vm.runInNewContext(src, { window: win });

  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  for (const name of [
    "prScopeModel",
    "prScopeSelection",
    "prScopeSuffix",
    "prScopeSummary",
    "prActionCandidates",
    "prLive",
  ]) {
    assert.equal(typeof win[name], "function", `${name} must be on window`);
    assert.ok(app.includes(name), `app.js is expected to use ${name}`);
  }

  // The row formatters lean on wf-prs.js's chip/state labels when they are
  // there, so that script must load first — and both before app.js.
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(
    html.indexOf("wf-prs.js") < html.indexOf("wf-pr-scope.js"),
    "wf-prs.js must be loaded before wf-pr-scope.js"
  );
  assert.ok(
    html.indexOf("wf-pr-scope.js") < html.indexOf("app.js"),
    "wf-pr-scope.js must be loaded before app.js"
  );
});
