// node --test gui/tests/ — the Waves roadmap and the Pull requests tab, run.
//
// Both views live in app.js and are reached only by clicking a tab, so a
// dangling reference in either surfaces as an empty tab while clash.log
// shows a healthy boot. This runs the real render functions against the real
// pure modules over a ten-PR, multi-wave item and reads back what they drew.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

const dist = (f) => fs.readFileSync(path.join(__dirname, "..", "dist", f), "utf8");
const APP = dist("app.js");

const gh = (repo, n) => `https://github.com/o/${repo}/pull/${n}`;
const linked = [
  ["saturn", 454, "OPEN"],
  ["saturn", 455, "OPEN"],
  ["saturn", 456, "OPEN"],
  ["k8s", 9313, "MERGED"],
  ["helm-charts", 1902, "MERGED"],
  ["api", 977, "OPEN"],
  ["api", 978, "OPEN"],
  ["api", 979, "CLOSED"],
  ["web", 39797, "OPEN"],
].map(([r, n, state]) => ({ url: gh(r, n), number: n, state }));
const wave = (id, state, level, prs, extra = {}) => ({
  id,
  title: `${id} title`,
  summary: `${id} summary`,
  dependsOn: level ? [`w${level}`] : [],
  areas: [`${id}/`],
  state,
  blockedBy: state === "blocked" ? [`w${level}`] : [],
  level,
  compatibleWith: [],
  tickets: [`TT-${level}`],
  ticketUrls: { [`TT-${level}`]: `https://jira/browse/TT-${level}` },
  prs,
  ...extra,
});
const ITEM = {
  project: "p",
  slug: "s",
  prCheckedAt: Date.now() - 5 * 60000,
  prErrors: { [gh("api", 978)]: "HTTP 502" },
  meta: {
    status: "pr-ready",
    pr: { url: gh("saturn", 457), number: 457, state: "OPEN", headRef: "x", baseRef: "y" },
    linkedPrs: linked,
  },
  waves: {
    waves: [
      wave("w1", "review", 0, linked.filter((p) => p.number !== 456).map((p) => p.url)),
      wave("w2", "blocked", 1, [gh("saturn", 456), gh("saturn", 457)]),
      wave("w3", "blocked", 2, []),
    ],
  },
};

function sandbox() {
  const box = {
    console,
    Date,
    CSS: { escape: (s) => s },
    document: { createElement: (t) => stubEl(t), body: { appendChild() {} } },
    invoke: async () => ({}),
    busyButton: (b, fn) => fn(),
    openLink: () => {},
    openLinks: () => {},
    showContextMenu: () => {},
    wfPrMenu: () => [],
    wfApproveWaves: () => {},
    wfStartWaves: () => {},
    refreshWorkflows: async () => {},
    wfItem: () => ITEM,
    flashToast: () => {},
  };
  box.window = box;
  vm.createContext(box);
  for (const f of ["wf-prs.js", "wf-waves.js"]) vm.runInContext(dist(f), box);
  for (const fn of ["wfPrStateChip", "wfWaveDecisionButtons", "renderWfWavesView", "renderWfPrsView"])
    vm.runInContext(extractFunction(APP, fn), box);
  return box;
}

const texts = (root) => descendants(root).map((e) => e.textContent).filter(Boolean);

test("the roadmap opens on what is happening, then walks the waves with their PRs", () => {
  const box = sandbox();
  const body = stubEl();
  box.renderWfWavesView(body, {}, ITEM);
  const t = texts(body);
  assert.ok(t.some((x) => /w1 · w1 title is waiting for your approval\. 2 of 8 PRs merged, 5 still open\./.test(x)), t.join("\n"));
  assert.ok(t.some((x) => /^Next: .*unlocks w2/.test(x)));
  assert.ok(t.includes("✓ Approve w1 → next waves…"), "the decision sits on the roadmap");
  assert.ok(t.includes("Step 1") && t.includes("Step 3"));
  assert.ok(t.some((x) => /to approve · 2\/8 PRs merged/.test(x)));
  assert.ok(t.includes("TT-0"), "tickets are shown");
  assert.ok(t.includes("saturn#456 · open"), "a wave's PRs are chips");
  assert.ok(t.includes("api#979 · closed"));
});

test("the Pull requests tab lists every PR by wave, with freshness and failures", async () => {
  const box = sandbox();
  const body = stubEl();
  await box.renderWfPrsView(body, {}, ITEM);
  const t = texts(body);
  assert.ok(t.some((x) => /^10 PRs · 2 merged · 1 closed · 7 open · checked 5 min ago$/.test(x)), t.join("\n"));
  assert.ok(t.some((x) => / · 1 not reached on the last refresh/.test(x)));
  assert.ok(t.includes("↻ Refresh all") && t.includes("Open 7 open PRs"));
  // Groups follow the waves; a wave with nothing yet is left out.
  const heads = t.filter((x) => /^w\d · /.test(x));
  assert.deepEqual(heads, ["w1 · w1 title", "w2 · w2 title"]);
  assert.ok(t.includes("not refreshed: HTTP 502"));
  assert.ok(t.some((x) => /^primary · x → y$/.test(x)));
});

test("past a handful of PRs the header summarizes and points at the tab", () => {
  const view = extractFunction(APP, "buildWorkflowView");
  assert.match(APP, /const WF_HEAD_PR_LIMIT = 3;/);
  assert.match(view, /const manyPrs = headPrs\.length > WF_HEAD_PR_LIMIT;/);
  assert.match(view, /for \(const pr of manyPrs \? headPrs\.filter\(\(p\) => p\.primary\) : headPrs\)/);
  assert.match(view, /ts\.subView = "prs";/);
  // A project in waves opens on its roadmap.
  assert.match(view, /if \(!ts\.subView && item\.waves\) ts\.subView = "waves";/);
});
