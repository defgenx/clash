// node --test gui/tests/ — the PR dashboard's pure half.
//
// itemPrs feeds three surfaces (item header, Open-PRs action, dashboard);
// its ordering and skip rules are contracts, as is the dashboard's sort.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const {
  prRepo,
  prChipLabel,
  prStateLabel,
  itemPrs,
  prDashboardModel,
} = require("../dist/wf-prs.js");

test("prRepo extracts owner/repo from PR URLs only", () => {
  assert.equal(prRepo("https://github.com/acme/clash/pull/42"), "acme/clash");
  assert.equal(prRepo("https://github.com/acme/clash/pull/42/files"), "acme/clash");
  assert.equal(prRepo("https://github.com/acme/clash/issues/42"), "");
  assert.equal(prRepo("https://gitlab.com/g/p/-/merge_requests/1"), "");
  assert.equal(prRepo(""), "");
  assert.equal(prRepo(null), "");
});

test("chip labels name the repo for linked PRs and survive URL-only records", () => {
  assert.equal(
    prChipLabel({ url: "https://github.com/acme/front/pull/12", number: 12 }),
    "front#12"
  );
  // URL-only (agent-written) record: the number comes from the URL.
  assert.equal(prChipLabel({ url: "https://github.com/acme/front/pull/12", number: 0 }), "front#12");
  assert.equal(prChipLabel({ url: "", number: 7 }), "#7");
});

test("state labels: merged/closed win over draft, unknown stays quiet", () => {
  assert.equal(prStateLabel({ state: "MERGED", draft: true }), "merged");
  assert.equal(prStateLabel({ state: "CLOSED" }), "closed");
  assert.equal(prStateLabel({ state: "OPEN", draft: true }), "draft");
  assert.equal(prStateLabel({ state: "OPEN" }), "open");
  assert.equal(prStateLabel({ state: "", draft: false }), "");
});

test("itemPrs lists the primary first, skips empty records, maps the unanswered count", () => {
  const meta = {
    pr: { url: "https://github.com/o/r/pull/7", number: 7, unansweredComments: 2 },
    linkedPrs: [
      { url: "https://github.com/o/front/pull/12", number: 12, draft: true },
      { url: "", number: 0 }, // placeholder — never a PR
    ],
  };
  const prs = itemPrs(meta);
  assert.equal(prs.length, 2);
  assert.equal(prs[0].primary, true);
  assert.equal(prs[0].repo, "o/r");
  assert.equal(prs[0].unanswered, 2);
  assert.equal(prs[1].primary, false);
  assert.equal(prs[1].unanswered, null); // never fetched ≠ zero
  // No PR block at all → empty list, and a meta-less item doesn't throw.
  assert.deepEqual(itemPrs({}), []);
  assert.deepEqual(itemPrs(null), []);
});

test("the dashboard shows only PR-bearing items, decisions first, settled last", () => {
  const mk = (slug, status, updatedAt, pr, linked = []) => ({
    project: "p",
    slug,
    meta: { title: slug, status, updatedAt, pr, linkedPrs: linked },
  });
  const rows = prDashboardModel([
    mk("no-pr", "diff-review", 99, null),
    mk("settled", "done", 50, { url: "https://github.com/o/r/pull/1", state: "MERGED" }),
    mk("older-open", "implementing", 10, { url: "https://github.com/o/r/pull/2", state: "OPEN" }),
    mk("newer-open", "implementing", 20, { url: "https://github.com/o/r/pull/3", state: "OPEN" }),
    mk("decision", "pr-draft", 5, { url: "https://github.com/o/r/pull/4", state: "OPEN" }),
  ]);
  assert.deepEqual(
    rows.map((r) => r.slug),
    ["decision", "newer-open", "older-open", "settled"]
  );
  assert.equal(rows[0].needsDecision, true);
  assert.equal(rows[3].allSettled, true);
});

test("the dashboard sums unanswered comments across an item's PRs", () => {
  const rows = prDashboardModel([
    {
      project: "p",
      slug: "s",
      meta: {
        status: "pr-ready",
        updatedAt: 1,
        pr: { url: "https://github.com/o/r/pull/1", unansweredComments: 2 },
        linkedPrs: [{ url: "https://github.com/o/f/pull/2", unansweredComments: 3 }],
      },
    },
  ]);
  assert.equal(rows[0].unanswered, 5);
  assert.equal(rows[0].prs.length, 2);
});

test("browser branch publishes every global app.js reads", () => {
  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-prs.js"), "utf8");
  const sandbox = { window: {} };
  vm.createContext(sandbox);
  vm.runInContext(src, sandbox);
  for (const name of ["prRepo", "prChipLabel", "prStateLabel", "itemPrs", "prDashboardModel"]) {
    assert.ok(name in sandbox.window, `${name} must be published to window`);
  }
});
