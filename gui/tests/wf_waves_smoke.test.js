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

function run(states, { status = "diff-review", issue = null } = {}) {
  const added = [];
  const calls = [];
  const notes = [];
  const sandbox = {
    add: (label, cls, fn, title, zone, act) => added.push({ label, cls, fn, act }),
    note: (text) => notes.push(text),
    wfApproveWaves: (it, root, opts) => calls.push({ fn: "approve", opts }),
    wfStartWaves: () => calls.push({ fn: "start" }),
    reviewPending: false,
    root: {},
    item: { meta: { status }, waves: issue ? { waves: [], issue } : board(states) },
  };
  sandbox.window = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(dist("wf-waves.js"), sandbox);
  vm.runInContext(extractArrow(BAR, "waveButtons"), sandbox);
  vm.runInContext("waveButtons();", sandbox);
  return { added, calls, notes };
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

test("the last waves get a sign-off of their own; an item without waves adds nothing", () => {
  // Without it a pr-ready item had no way to sign them off, and so no way to close.
  for (const status of ["diff-review", "pr-draft", "pr-ready"]) {
    const { added, calls } = run(["done", "done", "review"], { status });
    assert.deepEqual(added.map((b) => [b.label, b.act]), [["✓ Sign off Wave 3", "approve-waves"]], status);
    added[0].fn();
    assert.equal(JSON.stringify(calls), JSON.stringify([{ fn: "approve", opts: { next: false } }]));
  }
  assert.equal(run([]).added.length, 0);
});

test("an unreadable waves.json says how to repair it and offers no wave decision", () => {
  const { added, notes } = run([], { status: "pr-ready", issue: "the dependencies form a cycle" });
  assert.equal(added.length, 0);
  assert.deepEqual(notes, [
    "waves.json can't be read: the dependencies form a cycle — ask 💬 Discuss to repair it, or ↩ Move to… plan review.",
  ]);
});

test("the item's approvals are held while waves.json can't be read", () => {
  const { wavesHoldApproval } = require("../dist/wf-waves.js");
  assert.ok(wavesHoldApproval({ meta: { status: "diff-review" }, waves: { waves: [], issue: "x" } }));
});

test("recording a ready PR signs off nothing; plan review with every wave signed off approves as usual", () => {
  const tr = extractFunction(APP, "wfTransition");
  assert.match(tr, /\["done", "pr-draft"\]\.includes\(status\)/);
  assert.doesNotMatch(tr, /"pr-ready"\]\.includes/);
  assert.match(BAR, /if \(waveBoard\(item\) && !waveSummary\(item\)\.allDone\) \{/);
  // A broken waves.json at plan review: the note and a change request, no
  // approve button for the recommender to point at.
  const plan = BAR.slice(BAR.indexOf('case "plan-review":'), BAR.indexOf('case "changes-requested":'));
  const broken = plan.slice(plan.indexOf("if (wavesIssue(item)) {"), plan.indexOf("break;"));
  assert.match(broken, /note\(wavesIssueGuidance\(item\)\);/);
  assert.doesNotMatch(broken, /approve-plan/);
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
