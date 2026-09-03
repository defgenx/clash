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

test("only the actions that can fan out offer an 'all' row", () => {
  // open/markReady/post act on several PRs in one go; a review or a respond
  // round is one agent session parked on one item, so their "all" would be
  // two agents editing the same item's files.
  for (const action of ["open", "markReady", "post"]) {
    const m = prScopeModel(THREE, action);
    assert.equal(m.rows[0].value.all, true, `${action} must offer a fan-out row`);
    assert.equal(m.rows[0].value.urls.length, prActionCandidates(THREE, action).length);
  }
  for (const action of ["review", "respond"]) {
    const m = prScopeModel(THREE, action);
    assert.ok(
      m.rows.every((r) => r.value.all === false),
      `${action} must never offer a fan-out row`
    );
    assert.equal(m.rows.length, 3);
  }
});

test("the 'all' row of mark-ready covers exactly the drafts", () => {
  // Not "every PR the item has": flipping a merged PR is an error, and the
  // count on the label is what the human is agreeing to.
  const m = prScopeModel(THREE, "markReady");
  assert.deepEqual(m.rows[0].value.urls, [THREE[0].url, THREE[1].url]);
  assert.match(m.rows[0].label, /All 2 drafts/);
});

test("nothing is asked when there is nothing to choose", () => {
  // A single-PR item must behave exactly as it did before the scope existed:
  // no dialog, the one candidate resolved straight through.
  const one = prScopeModel([THREE[0]], "markReady");
  assert.equal(one.needed, false);
  assert.deepEqual(one.only, { all: false, urls: [THREE[0].url] });
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
  const single = m.rows.filter((r) => !r.value.all);
  const primaryRow = single.find((r) => r.value.urls[0] === THREE[0].url);
  assert.match(primaryRow.label, /primary/);
  const linkedRow = single.find((r) => r.value.urls[0] === THREE[1].url);
  assert.doesNotMatch(linkedRow.label, /primary/);
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
  assert.equal(prScopeSummary({ urls: THREE.map((p) => p.url) }, THREE), "3 pull requests");
  assert.equal(prScopeSummary({ urls: [THREE[1].url] }, THREE), "web#2");
  // An unknown URL still produces something a human can read.
  assert.equal(prScopeSummary({ urls: ["https://x/y"] }, THREE), "https://x/y");
  assert.equal(prScopeSummary({ urls: [] }, THREE), "the PR");
});

test("an unknown action acts on nothing rather than on everything", () => {
  // A typo'd action name must not silently resolve to "all PRs" — that is a
  // fan-out nobody asked for.
  assert.deepEqual(prActionCandidates(THREE, "markRedy"), []);
  assert.equal(prScopeModel(THREE, "markRedy").rows.length, 0);
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
  for (const name of ["prScopeModel", "prScopeSuffix", "prScopeSummary", "prActionCandidates", "prLive"]) {
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
