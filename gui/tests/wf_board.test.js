// node --test gui/tests/ — the workflow board, the pure half.
//
// The board's point is "what waits on me": pinned here so its needs-you rule
// stays the inbox's, finished work never asks for anything, and the lanes
// put what waits on you first.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { BOARD_LANES, DONE_SHOWN, boardCard, boardModel } = require("../dist/wf-board.js");

const NOW = Date.UTC(2026, 10, 10, 12, 0);
const it = (slug, status, extra = {}, meta = {}) => ({
  project: extra.project || "p",
  slug,
  agentAlive: true,
  openAnnotations: 0,
  ...extra,
  meta: { title: `${slug} title`, status, iteration: 2, updatedAt: NOW - 3600000, ...meta },
});

test("every status lands in exactly one lane", () => {
  const all = BOARD_LANES.flatMap((l) => l.statuses);
  for (const st of ["draft", "planning", "plan-review", "changes-requested", "implementing", "reviewing", "diff-review", "pr-draft", "pr-ready", "done", "abandoned"])
    assert.equal(all.filter((s) => s === st).length, 1, st);
});

test("a card says its stage, its age and why it waits on you", () => {
  const c = boardCard(
    it("a", "diff-review", { openAnnotations: 2 }, { pr: { url: "u", number: 1, state: "OPEN", unansweredComments: 3 } }),
    NOW
  );
  assert.equal(c.stage, "Code review");
  assert.equal(c.ago, "1h ago");
  assert.deepEqual(c.needs, ["your decision", "3 PR threads"]);
  assert.equal(c.comments, 2);
  assert.deepEqual(c.prs, { total: 1, open: 1, merged: 0, drafts: 0 });
  assert.equal(boardCard(it("b", "implementing"), NOW).working, true);
  assert.deepEqual(boardCard(it("b", "implementing", { agentAlive: false }), NOW).needs, ["agent gone"]);
  assert.deepEqual(boardCard(it("c", "changes-requested"), NOW).needs, ["launch the fix round"]);
  // A check run in flight decides for now; the stage waits on it, not on you.
  assert.deepEqual(boardCard(it("d", "plan-review", { run: { id: 1 } }), NOW).needs, []);
  // Finished work asks nothing, whatever its PRs still carry.
  assert.deepEqual(
    boardCard(it("e", "done", {}, { pr: { url: "u", number: 1, state: "MERGED", unansweredComments: 6 } }), NOW).needs,
    []
  );
});

test("lanes put what needs you first, then the most recently moved", () => {
  const items = [
    it("old", "plan-review", { run: { id: 1 } }, { updatedAt: NOW - 1000 }),
    it("mine", "plan-review", {}, { updatedAt: NOW - 9e6 }),
    it("newer", "plan-review", { run: { id: 2 } }, { updatedAt: NOW - 10 }),
  ];
  const lane = boardModel(items, {}, NOW).lanes.find((l) => l.id === "plan-review");
  assert.deepEqual(lane.cards.map((c) => c.key), ["p/mine", "p/newer", "p/old"]);
});

test("filters: project, search, and a Done lane that keeps its latest", () => {
  const items = [
    it("a", "diff-review", { project: "api" }, { branch: "feat/audit" }),
    it("b", "implementing", { project: "web" }),
    ...Array.from({ length: DONE_SHOWN + 3 }, (_, i) => it(`d${i}`, "done", { project: "api" }, { updatedAt: NOW - i })),
  ];
  const m = boardModel(items, {}, NOW);
  assert.deepEqual(m.projects, [{ name: "api", count: DONE_SHOWN + 4 }, { name: "web", count: 1 }]);
  assert.deepEqual(m.summary, { needsYou: 1, working: 1, inPr: 0, done: DONE_SHOWN + 3, total: DONE_SHOWN + 5 });
  const done = m.lanes.find((l) => l.id === "done");
  assert.equal(done.cards.length, DONE_SHOWN);
  assert.equal(done.hidden, 3);
  assert.equal(done.cards[0].key, "p/d0".replace("p/", "api/"), "latest first");
  assert.equal(boardModel(items, { showDone: true }, NOW).lanes.find((l) => l.id === "done").hidden, 0);
  assert.equal(boardModel(items, { project: "web" }, NOW).summary.total, 1);
  assert.deepEqual(
    boardModel(items, { query: "audit api" }, NOW).lanes.flatMap((l) => l.cards.map((c) => c.key)),
    ["api/a"],
    "every word must match: title, slug, project, branch, ticket"
  );
});

test("the browser branch publishes what app.js calls", () => {
  const win = {};
  for (const f of ["wf-prs.js", "wf-waves.js", "wf-board.js"])
    vm.runInNewContext(fs.readFileSync(path.join(__dirname, "../dist", f), "utf8"), { window: win });
  for (const n of ["boardModel", "boardCard", "DONE_SHOWN", "BOARD_LANES"]) assert.ok(win[n] !== undefined, n);
  const html = fs.readFileSync(path.join(__dirname, "../dist/index.html"), "utf8");
  assert.ok(html.indexOf("wf-board.js") > html.indexOf("wf-waves.js"), "loaded after its dependencies");
});
