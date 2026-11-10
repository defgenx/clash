// node --test gui/tests/ — waves, the pure half.
//
// The backend decides each wave's state and which waves may run together;
// what is pinned here is how the bar, the board and the picker read that
// answer: waves go in order by default, compatible ones may be ticked
// together, and the item's own approval waits until every wave is approved.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const {
  waveBoard,
  wavesIssue,
  waveSummary,
  waveBarState,
  wavesHoldApproval,
  wavePickRows,
  defaultWavePick,
  toggleWavePick,
  waveColumns,
} = require("../dist/wf-waves.js");

const w = (id, state, level, compatibleWith = [], extra = {}) => ({
  id,
  title: `${id} title`,
  summary: "",
  dependsOn: [],
  areas: [`${id}/`],
  state,
  blockedBy: [],
  level,
  compatibleWith,
  ...extra,
});

// w1 → (w2, w3) → w4, where w2 and w3 are compatible.
const item = (states) => ({
  waves: {
    waves: [
      w("w1", states[0], 0),
      w("w2", states[1], 1, ["w3"]),
      w("w3", states[2], 1, ["w2"]),
      w("w4", states[3], 2),
    ],
  },
});

test("an item without waves, or with an unusable file, has no board", () => {
  assert.equal(waveBoard({}), null);
  assert.equal(waveSummary({}), null);
  const broken = { waves: { waves: [], issue: "the waves' dependencies form a cycle" } };
  assert.equal(waveBoard(broken), null);
  assert.match(wavesIssue(broken), /cycle/);
});

test("the summary says where the waves stand", () => {
  assert.equal(waveSummary(item(["ready", "blocked", "blocked", "blocked"])).text, "Waves 0/4 · w1 ready");
  assert.equal(waveSummary(item(["done", "active", "active", "blocked"])).text, "Waves 1/4 · w2, w3 in progress");
  assert.equal(waveSummary(item(["done", "review", "review", "blocked"])).text, "Waves 1/4 · w2, w3 to approve");
  assert.equal(waveSummary(item(["done", "done", "done", "done"])).text, "Waves 4/4 · all approved");
});

test("the bar approves a round, then starts the next, and lets go at the end", () => {
  assert.equal(waveBarState(item(["done", "review", "review", "blocked"])).kind, "approve");
  assert.equal(waveBarState(item(["done", "done", "done", "ready"])).kind, "start");
  // The last round's waves: the item's own approvals take over.
  assert.equal(waveBarState(item(["done", "done", "done", "review"])).kind, "approve-last");
  assert.equal(wavesHoldApproval(item(["done", "done", "done", "review"])), false);
  assert.equal(waveBarState(item(["done", "done", "done", "done"])), null);
  // An agent on it: nothing to decide.
  assert.equal(waveBarState(item(["done", "active", "ready", "blocked"])), null);
  // Final approvals wait while waves remain.
  assert.ok(wavesHoldApproval(item(["done", "review", "review", "blocked"])));
  assert.ok(wavesHoldApproval(item(["done", "done", "done", "ready"])));
  assert.ok(!wavesHoldApproval({}));
});

test("waves go in order by default, and compatible ones may join", () => {
  const it = item(["done", "ready", "ready", "blocked"]);
  assert.deepEqual(defaultWavePick(it), ["w2"]);
  const rows = wavePickRows(it, ["w2"]);
  assert.deepEqual(rows.map((r) => r.id), ["w2", "w3"]);
  assert.equal(rows[1].disabled, false, "w3 may run with w2");
  assert.deepEqual(toggleWavePick(it, ["w2"], "w3"), ["w2", "w3"]);
  assert.deepEqual(toggleWavePick(it, ["w2", "w3"], "w2"), ["w3"]);
});

test("a wave that cannot run with the pick is disabled, with the reason", () => {
  const it = item(["done", "ready", "ready", "blocked"]);
  it.waves.waves[1].compatibleWith = [];
  it.waves.waves[2].compatibleWith = [];
  const rows = wavePickRows(it, ["w2"]);
  assert.equal(rows[1].disabled, true);
  assert.match(rows[1].reason, /Can't run with w2/);
  assert.deepEqual(toggleWavePick(it, ["w2"], "w3"), ["w2"]);
  // A wave without areas says why it runs alone.
  it.waves.waves[2].areas = [];
  assert.match(wavePickRows(it, [])[1].detail, /runs alone/);
});

test("the board's columns follow dependency levels", () => {
  const cols = waveColumns(item(["done", "ready", "ready", "blocked"]));
  assert.deepEqual(cols.map((c) => c.waves.map((x) => x.id)), [["w1"], ["w2", "w3"], ["w4"]]);
  assert.equal(cols[0].title, "First");
  assert.equal(cols[2].title, "Then (after w2, w3)");
});

test("the browser branch publishes every name app.js calls", () => {
  const src = fs.readFileSync(path.join(__dirname, "../dist/wf-waves.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });
  for (const name of [
    "waveBoard",
    "wavesIssue",
    "waveLabel",
    "waveList",
    "waveSummary",
    "waveBarState",
    "wavesHoldApproval",
    "wavePickRows",
    "defaultWavePick",
    "toggleWavePick",
    "waveColumns",
    "waveStateLabel",
  ])
    assert.ok(win[name], name);
});
