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
  prStackLabel,
  prHealthBadges,
  prHealthTitle,
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

test("a stack is spliced in base to tip where its first layer sat", () => {
  const u = (r, n) => `https://github.com/o/${r}/pull/${n}`;
  const meta = {
    pr: { url: u("api", 2) },
    linkedPrs: [{ url: u("web", 9) }, { url: u("api", 1) }],
  };
  const prs = itemPrs(meta, { layers: [{ url: u("api", 1) }, { url: u("api", 2) }] });
  assert.deepEqual(
    prs.map((p) => p.url.split("/").pop()),
    ["1", "2", "9"]
  );
  assert.deepEqual(prs[1].stack, { pos: 2, of: 2, parent: u("api", 1) });
  assert.equal(prStackLabel(prs[1]), "2/2");
  assert.equal(prs[2].stack, null);
  assert.equal(prStackLabel(prs[2]), "");
  // Without a stack (or with a stale one) the order is the item's own.
  assert.deepEqual(
    itemPrs(meta).map((p) => p.url.split("/").pop()),
    ["2", "9", "1"]
  );
  const stale = { layers: [{ url: u("api", 1) }, { url: u("api", 7) }] };
  assert.ok(itemPrs(meta, stale).every((p) => p.stack === null));
});

test("the dashboard orders a stacked item's chips and names an unclear stack", () => {
  const u = (n) => `https://github.com/o/api/pull/${n}`;
  const rows = prDashboardModel([
    {
      project: "p",
      slug: "s",
      meta: { status: "pr-draft", pr: { url: u(2) }, linkedPrs: [{ url: u(1) }] },
      prStack: { layers: [{ url: u(1) }, { url: u(2) }] },
    },
    {
      project: "p",
      slug: "t",
      meta: { status: "pr-draft", pr: { url: u(5) } },
      prStackIssue: "api#5 and api#6 target each other in a loop",
    },
  ]);
  assert.deepEqual(
    rows[0].prs.map((p) => p.url.split("/").pop()),
    ["1", "2"]
  );
  assert.equal(rows[0].stackIssue, null);
  assert.match(rows[1].stackIssue, /loop/);
});

test("browser branch publishes every global app.js reads", () => {
  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-prs.js"), "utf8");
  const sandbox = { window: {} };
  vm.createContext(sandbox);
  vm.runInContext(src, sandbox);
  for (const name of [
    "prRepo",
    "prChipLabel",
    "prStateLabel",
    "itemPrs",
    "prStackLabel",
    "prDashboardModel",
    "prDashboardView",
  ]) {
    assert.ok(name in sandbox.window, `${name} must be published to window`);
  }
});

const { prCounts, prCountsText, prBoard, checkedAgo } = require("../dist/wf-prs.js");

test("many PRs read as one summary and a board grouped by wave", () => {
  const meta = {
    pr: { url: "https://github.com/o/saturn/pull/457", number: 457, state: "OPEN", headRef: "main-branch" },
    linkedPrs: [
      { url: "https://github.com/o/saturn/pull/454", number: 454, state: "OPEN", unansweredComments: 2 },
      { url: "https://github.com/o/k8s/pull/9313", number: 9313, state: "MERGED" },
      { url: "https://github.com/o/api/pull/979", number: 979, state: "CLOSED" },
      { url: "https://github.com/o/api/pull/977", number: 977, state: "OPEN", draft: true },
    ],
  };
  const c = prCounts(itemPrs(meta));
  assert.deepEqual(c, { total: 5, merged: 1, closed: 1, open: 3, draft: 1, threads: 2 });
  assert.equal(prCountsText(c), "5 PRs · 1 merged · 1 closed · 3 open · 2 threads to answer");
  assert.equal(prCountsText(prCounts([])), "No PR");

  const item = {
    meta,
    prCheckedAt: 1000,
    prErrors: { "https://github.com/o/api/pull/977": "HTTP 502" },
    waves: {
      waves: [
        { id: "w1", title: "Foundations", state: "review", prs: [meta.linkedPrs[0].url, meta.linkedPrs[1].url, meta.linkedPrs[3].url] },
        { id: "w2", title: "Pipeline", state: "blocked", prs: [meta.pr.url] },
        { id: "w3", title: "Later", state: "blocked", prs: [] },
      ],
    },
  };
  const b = prBoard(item);
  // A wave with no PRs and nothing going on is left out; unclaimed PRs last.
  assert.deepEqual(b.groups.map((g) => g.key), ["w1", "w2", ""]);
  assert.equal(b.groups[0].title, "Wave 1 · Foundations");
  // Open before settled inside a group.
  assert.deepEqual(b.groups[0].prs.map((p) => p.number), [454, 977, 9313]);
  assert.equal(b.groups[0].prs[1].error, "HTTP 502");
  assert.equal(b.groups[2].title, "Not in a wave");
  assert.equal(b.errors, 1);
  assert.equal(b.checkedAt, 1000);
  // A PR two waves list sits under both, and the item still counts it once.
  item.waves.waves[1].prs = [meta.pr.url, meta.linkedPrs[0].url];
  const shared = prBoard(item);
  assert.deepEqual(shared.groups[1].prs.map((p) => p.number), [457, 454]);
  assert.ok(shared.groups[0].prs.some((p) => p.number === 454));
  assert.equal(shared.counts.total, 5);
  // A URL listed twice by the item is one PR.
  assert.equal(itemPrs({ ...meta, linkedPrs: [...meta.linkedPrs, { url: meta.pr.url }] }).length, 5);
  // Without waves: one group.
  assert.equal(prBoard({ meta }).groups.length, 1);
  assert.equal(itemPrs(meta)[0].headRef, "main-branch");
});

test("freshness is said in words", () => {
  assert.equal(checkedAgo(0, 5), "not checked yet");
  assert.equal(checkedAgo(1000, 20000), "checked just now");
  assert.equal(checkedAgo(0 + 1, 1 + 5 * 60000), "checked 5 min ago");
  assert.equal(checkedAgo(1, 1 + 3 * 3600000), "checked 3 h ago");
});

const { prDashboardView } = require("../dist/wf-prs.js");

test("the dashboard filters: active by default, what needs you, everything", () => {
  const row = (slug, extra) => ({
    project: "p",
    slug,
    title: `${slug} title`,
    status: "pr-ready",
    needsDecision: false,
    unanswered: 0,
    allSettled: false,
    prs: [{ url: `u-${slug}`, repo: "o/r", number: 1, state: "OPEN", headRef: `feat/${slug}`, unanswered: 0 }],
    ...extra,
  });
  const rows = [
    row("quiet"),
    row("threads", { unanswered: 2, prs: [{ url: "u2", repo: "o/api", number: 7, state: "OPEN", headRef: "fix/x", unanswered: 2, draft: true }] }),
    row("decide", { needsDecision: true, project: "web" }),
    row("merged", { allSettled: true, prs: [{ url: "u3", repo: "o/r", number: 3, state: "MERGED", headRef: "" }] }),
  ];
  const v = prDashboardView(rows);
  assert.deepEqual(v.rows.map((r) => r.slug), ["quiet", "threads", "decide"], "active hides settled items");
  assert.deepEqual(v.counts, { active: 3, attention: 2, all: 4 });
  assert.deepEqual(v.summary, { open: 3, drafts: 1, threads: 2, merged: 1 });
  assert.deepEqual(prDashboardView(rows, { view: "attention" }).rows.map((r) => r.slug), ["threads", "decide"]);
  assert.equal(prDashboardView(rows, { view: "all" }).rows.length, 4);
  // Project and search narrow, and the tab counts follow them.
  assert.deepEqual(prDashboardView(rows, { project: "web" }).counts, { active: 1, attention: 1, all: 1 });
  assert.deepEqual(prDashboardView(rows, { query: "api #7" }).rows.map((r) => r.slug), ["threads"]);
  assert.deepEqual(prDashboardView(rows, { query: "feat/quiet" }).rows.map((r) => r.slug), ["quiet"]);
  assert.deepEqual(v.projects, [{ name: "p", count: 3 }, { name: "web", count: 1 }]);
});

test("PR health reads as compact badges, failing checks named in the tooltip", () => {
  const pr = (health, extra = {}) => ({ url: "u", state: "OPEN", health, ...extra });
  const red = pr({ checks: "fail", failing: ["lint", "unit"], mergeable: "conflicts", behind: true, review: "changes" });
  assert.deepEqual(
    prHealthBadges(red).map((b) => [b.kind, b.text]),
    [["fail", "✗"], ["conflicts", "conflicts"], ["behind", "behind base"], ["changes", "changes requested"]]
  );
  assert.match(prHealthTitle(red), /^CI failing: lint, unit · /);
  assert.deepEqual(
    prHealthBadges(pr({ checks: "pass", mergeable: "yes", review: "approved" })).map((b) => b.text),
    ["✓", "approved"]
  );
  assert.deepEqual(prHealthBadges(pr({ checks: "pending" })).map((b) => b.text), ["●"]);
  // Nothing to say: no checks, unknown mergeability, no review rule.
  assert.deepEqual(prHealthBadges(pr({ checks: "none", mergeable: "unknown" })), []);
  // Never fetched, or a PR whose CI no longer decides anything.
  assert.deepEqual(prHealthBadges(pr(undefined)), []);
  assert.deepEqual(prHealthBadges(pr(red.health, { state: "MERGED" })), []);
  assert.equal(prHealthTitle(pr(undefined)), "");
});

test("itemPrs carries each PR's health, null when never fetched", () => {
  const meta = {
    pr: { url: "https://github.com/o/r/pull/7", health: { checks: "fail", failing: ["t"] } },
    linkedPrs: [
      { url: "https://github.com/o/a/pull/1", health: { checks: "pass", behind: true } },
      { url: "https://github.com/o/b/pull/2", state: "MERGED", health: { checks: "fail" } },
      { url: "https://github.com/o/c/pull/3" },
    ],
  };
  const prs = itemPrs(meta);
  assert.deepEqual(prs[0].health, { checks: "fail", failing: ["t"] });
  assert.deepEqual(prs[1].health, { checks: "pass", behind: true });
  assert.equal(prs[3].health, null);
});
