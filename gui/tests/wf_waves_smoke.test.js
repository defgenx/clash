// node --test gui/tests/ — the action bar's wave buttons, run for real.
//
// `waveButtons` is extracted from `renderWfActions` and run against the real
// wf-waves.js, so what is asserted is what the bar renders: approve the round
// in review (and start the next), start the next when nothing waits, and
// hold the item's own approvals until no wave remains.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction } = require("./extract-fn.js");

const dist = (f) => fs.readFileSync(path.join(__dirname, "..", "dist", f), "utf8");
const APP = dist("app.js");
const BAR = extractFunction(APP, "renderWfActions");

function extractArrow(source, name) {
  const at = source.indexOf(`const ${name} = () => {`);
  assert.ok(at >= 0, `${name} must exist as a zero-arg arrow`);
  let i = source.indexOf("{", at);
  let depth = 0;
  for (; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) break;
  }
  return source.slice(at, i + 1);
}

const board = (states) => ({
  waves: states.map((state, i) => ({
    id: `w${i + 1}`,
    title: `T${i + 1}`,
    summary: "",
    dependsOn: [],
    areas: [`w${i + 1}`],
    state,
    blockedBy: [],
    level: i,
    compatibleWith: [],
  })),
});

function run(states) {
  const added = [];
  const calls = [];
  const sandbox = {
    add: (label, cls, fn, title, zone, act) => added.push({ label, cls, fn, act }),
    wfApproveWaves: (it, root, opts) => calls.push({ fn: "approve", opts }),
    wfStartWaves: () => calls.push({ fn: "start" }),
    reviewPending: false,
    root: {},
    item: { meta: { status: "diff-review" }, waves: board(states) },
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(dist("wf-waves.js"), sandbox);
  vm.runInContext(extractArrow(BAR, "waveButtons"), sandbox);
  vm.runInContext("waveButtons();", sandbox);
  return { added, calls };
}

test("a round in review is approved first, then the next waves start", () => {
  const { added, calls } = run(["done", "review", "blocked"]);
  assert.deepEqual(added.map((b) => b.act), ["approve-waves", undefined]);
  assert.match(added[0].label, /Sign off Wave 2 → next waves/);
  assert.equal(added[0].cls, "primary");
  added[1].fn();
  // Objects built inside the sandbox belong to another realm: compare by value.
  assert.equal(JSON.stringify(calls), JSON.stringify([{ fn: "approve", opts: { next: false } }]));
});

test("with nothing waiting, the next waves are offered", () => {
  const { added } = run(["done", "done", "ready"]);
  assert.deepEqual(added.map((b) => b.act), ["start-waves"]);
});

test("the last waves and an item without waves add nothing of their own", () => {
  assert.equal(run(["done", "done", "review"]).added.length, 0);
  assert.equal(run([]).added.length, 0);
});

test("the item's own approvals wait while waves remain", () => {
  assert.match(BAR, /const holdApproval = wavesHoldApproval\(item\);/);
  assert.match(BAR, /if \(holdApproval && FINAL_ACTS\.has\(act\)\) return b;/);
  for (const act of ["approve-done", "approve-pr-draft", "mark-done"])
    assert.match(BAR, new RegExp(`"${act}"`));
  // The last waves are approved by the item's own approval.
  assert.match(extractFunction(APP, "wfTransition"), /invoke\("approve_workflow_waves"/);
  // Every code stage offers the waves, and plan approval starts the first.
  assert.equal(BAR.split("waveButtons();").length - 1, 3);
  assert.match(BAR, /"✓ Approve plan → start waves…"/);
  // Launches carry the pick to the backend, which validates it.
  assert.match(extractFunction(APP, "launchWfAgent"), /waves: opts\.waves \|\| null,/);
  assert.match(extractFunction(APP, "wfStartWaves"), /launchWfAgent\(item, "implement", root, null, \{ waves: picks, home \}\)/);
});
