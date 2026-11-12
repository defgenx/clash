// node --test gui/tests/ — the item cockpit's context rail, the pure half.
//
// What the rail shows beside whatever tab is open: the waves story, the PRs
// at a glance and the item's agents. Pinned here so the rail never repeats
// the tab it sits beside and never hides what needs work behind what is done.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { railModel, RAIL_PR_CHIPS } = require("../dist/wf-rail.js");

const gh = (r, n) => `https://github.com/o/${r}/pull/${n}`;
const item = (extra = {}) => ({
  project: "p",
  slug: "s",
  prCheckedAt: 1000,
  prErrors: { [gh("api", 2)]: "HTTP 502" },
  meta: {
    status: "pr-ready",
    pr: { url: gh("api", 1), number: 1, state: "MERGED" },
    linkedPrs: [
      { url: gh("api", 2), number: 2, state: "OPEN" },
      { url: gh("web", 3), number: 3, state: "OPEN", draft: true },
    ],
  },
  ...extra,
});

test("PRs at a glance: counts, freshness, open ones first", () => {
  const m = railModel(item(), [], { now: 1000 + 120000 });
  assert.equal(m.prs.text, "3 PRs · 1 merged · 2 open");
  assert.equal(m.prs.fresh, "checked 2 min ago");
  assert.deepEqual(m.prs.chips.map((p) => p.number), [2, 3, 1]);
  assert.equal(m.prs.chips[0].error, "HTTP 502");
  assert.equal(m.prs.errors, 1);
  assert.equal(m.prs.many, true);
  assert.equal(m.prs.more, 0);
});

test("a long PR list is cut, and says how many it left out", () => {
  const linkedPrs = Array.from({ length: RAIL_PR_CHIPS + 3 }, (_, i) => ({
    url: gh("r", i + 10),
    number: i + 10,
    state: "OPEN",
  }));
  const m = railModel(item({ meta: { status: "pr-ready", linkedPrs } }), []);
  assert.equal(m.prs.chips.length, RAIL_PR_CHIPS);
  assert.equal(m.prs.more, 3);
});

test("the rail never repeats the tab it sits beside", () => {
  const withWaves = item({
    waves: { waves: [{ id: "w1", title: "Schema", state: "ready", blockedBy: [], compatibleWith: [], prs: [] }] },
  });
  const m = railModel(withWaves, []);
  assert.equal(m.waves.track[0].label, "Wave 1 · Schema");
  assert.match(m.waves.now, /Ready to start: Wave 1/);
  assert.equal(railModel(withWaves, [], { subView: "waves" }).waves, null);
  assert.equal(railModel(withWaves, [], { subView: "prs" }).prs, null);
  assert.equal(railModel(item(), []).waves, null, "no waves, no card");
  assert.equal(railModel(item({ waves: { waves: [] } }), []).waves, null, "an empty board, no blank card");
  assert.match(railModel(item({ waves: { waves: [], issue: "a cycle" } }), []).waves.now, /can't be used: a cycle/);
});

test("the item's agents, running first", () => {
  const sessions = [
    { id: "a", name: "S · implement", status: "Stashed", is_running: false, workflow_item: "p/s" },
    { id: "b", name: "S · code review r3", status: "Thinking", is_running: true, workflow_item: "p/s" },
    { id: "c", name: "other", is_running: true, workflow_item: "p/other" },
    { id: "d", name: "plain", is_running: true },
  ];
  const m = railModel(item(), sessions);
  assert.deepEqual(m.agents.map((a) => a.id), ["b", "a"]);
  assert.equal(m.agents[0].status, "thinking");
});

test("the browser branch publishes what app.js calls", () => {
  const win = {};
  for (const f of ["wf-prs.js", "wf-waves.js", "wf-rail.js"])
    vm.runInNewContext(fs.readFileSync(path.join(__dirname, "../dist", f), "utf8"), { window: win });
  assert.equal(typeof win.railModel, "function");
});
