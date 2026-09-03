// node --test gui/tests/ — the workflow stepper's pure model.
//
// The stepper is the "where am I, what happened" answer of the item view, so
// what each status renders as — and that looping statuses anchor to the node
// they produce work for — is worth pinning.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  wfPipelineNodes,
  wfPipelineAnchor,
  wfPipelineModel,
  wfRewindTargets,
  wfStageBlurb,
  wfNodeRewindStatus,
} = require("../dist/wf-pipeline.js");

test("each mode gets its own main line", () => {
  assert.deepEqual(
    wfPipelineNodes("full").map((n) => n.id),
    ["planning", "plan-review", "implementing", "diff-review", "pr", "done"]
  );
  // from-plan starts at plan review — nothing to plan.
  assert.equal(wfPipelineNodes("from-plan")[0].id, "plan-review");
  // review-only is just the review loop and the exit.
  assert.deepEqual(
    wfPipelineNodes("review-only").map((n) => n.id),
    ["diff-review", "done"]
  );
});

test("looping statuses anchor to the node they produce work for", () => {
  assert.equal(wfPipelineAnchor("changes-requested", {}), "implementing");
  assert.equal(wfPipelineAnchor("reviewing", { returnStatus: "pr-draft" }), "pr");
  assert.equal(wfPipelineAnchor("reviewing", {}), "diff-review");
  // review-only has no Implement node — the loop hangs off diff-review.
  assert.equal(
    wfPipelineAnchor("implementing", { mode: "review-only" }),
    "diff-review"
  );
});

test("states split around the current node", () => {
  const m = wfPipelineModel({ status: "diff-review", mode: "full" });
  const states = Object.fromEntries(m.nodes.map((n) => [n.id, n.state]));
  assert.equal(states["planning"], "done");
  assert.equal(states["implementing"], "done");
  assert.equal(states["diff-review"], "current");
  assert.equal(states["pr"], "future");
  assert.equal(states["done"], "future");
});

test("a review round in flight names itself on the anchor node", () => {
  const m = wfPipelineModel({
    status: "reviewing",
    mode: "full",
    reviewReturnStatus: "pr-ready",
    reviewRoundInFlight: 5,
  });
  const pr = m.nodes.find((n) => n.id === "pr");
  assert.equal(pr.state, "current");
  assert.match(pr.sub, /round 5 in flight/);
});

test("chips count change rounds and reviews; unknown status still renders", () => {
  const m = wfPipelineModel({
    status: "pr-ready",
    reviewRound: 4,
    iteration: 3,
    prDraft: false,
  });
  assert.ok(m.chips.some((c) => /2 change rounds/.test(c)));
  assert.ok(m.chips.some((c) => /4 agent reviews/.test(c)));
  assert.ok(m.chips.some((c) => /PR ready/.test(c)));
  // Forward-compat: a status this build doesn't know must not blow up.
  assert.ok(wfPipelineModel({ status: "something-new" }).nodes.length > 0);
});

test("terminal items mark the whole line done, abandoned is flagged dead", () => {
  const done = wfPipelineModel({ status: "done" });
  assert.ok(done.nodes.every((n) => n.state === "done"));
  assert.equal(done.dead, false);
  assert.equal(wfPipelineModel({ status: "abandoned" }).dead, true);
});

test("rewind targets are the parked stages behind the item, nearest first", () => {
  // Mirrors WorkflowStatus::rewind_targets. The Rust side validates the move;
  // this decides what the picker and the stepper offer, so the two lists must
  // not drift.
  assert.deepEqual(wfRewindTargets("pr-ready", "full"), [
    "pr-draft",
    "diff-review",
    "plan-review",
    "draft",
  ]);
  assert.deepEqual(wfRewindTargets("diff-review", "full"), ["plan-review", "draft"]);
  // A finished item may go back anywhere — that is the whole ask.
  assert.deepEqual(wfRewindTargets("done", "full"), [
    "pr-ready",
    "pr-draft",
    "diff-review",
    "plan-review",
    "draft",
  ]);
  assert.deepEqual(wfRewindTargets("abandoned", "full"), wfRewindTargets("done", "full"));
  // Mode filters: from-plan never had a draft, review-only has no plan stages.
  assert.deepEqual(wfRewindTargets("diff-review", "from-plan"), ["plan-review"]);
  assert.deepEqual(wfRewindTargets("pr-draft", "review-only"), ["diff-review"]);
  // The first stage of a mode has nothing behind it.
  assert.deepEqual(wfRewindTargets("draft", "full"), []);
  assert.deepEqual(wfRewindTargets("diff-review", "review-only"), []);
  // Never *into* an agent's hands, and never while one is working.
  for (const st of ["planning", "implementing", "reviewing", "something-new"]) {
    assert.deepEqual(wfRewindTargets(st, "full"), [], st);
  }
  for (const st of ["plan-review", "diff-review", "pr-draft", "pr-ready", "done"]) {
    for (const t of wfRewindTargets(st, "full")) {
      assert.ok(!["planning", "implementing", "reviewing", "done"].includes(t), `${st} → ${t}`);
    }
  }
});

test("every rewind target explains itself, and only passed stages are clickable", () => {
  // A bare status name is a label; the picker needs a reason to pick it.
  for (const st of wfRewindTargets("done", "full")) {
    assert.ok(wfStageBlurb(st).length > 10, st);
  }
  assert.equal(wfStageBlurb("nope"), "");
  // The PR node covers two statuses and stands for whichever is behind us.
  const back = wfRewindTargets("done", "full");
  assert.equal(wfNodeRewindStatus("pr", back), "pr-ready");
  assert.equal(wfNodeRewindStatus("diff-review", back), "diff-review");
  assert.equal(wfNodeRewindStatus("pr", ["pr-draft", "diff-review"]), "pr-draft");
  // The agent nodes have no parked equivalent: you cannot move an item into
  // an agent's hands by clicking a stepper node.
  assert.equal(wfNodeRewindStatus("planning", back), null);
  assert.equal(wfNodeRewindStatus("implementing", back), null);
  // Nothing is clickable when there is nowhere to go.
  assert.equal(wfNodeRewindStatus("diff-review", []), null);
});

test("the browser branch publishes every name app.js calls", () => {
  // app.js is a plain script that reads these off `window`, so a rename here
  // (or a missing `<script>` tag) fails at render time, not at boot. Same
  // guard as wf-compose.
  const fs = require("node:fs");
  const path = require("node:path");
  const vm = require("node:vm");

  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-pipeline.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });

  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  for (const name of ["wfPipelineModel", "wfRewindTargets", "wfStageBlurb", "wfNodeRewindStatus"]) {
    assert.equal(typeof win[name], "function", `${name} must be on window`);
    assert.ok(app.includes(name), `app.js is expected to use ${name}`);
  }

  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(
    html.indexOf("wf-pipeline.js") < html.indexOf("app.js"),
    "wf-pipeline.js must load before app.js"
  );
  assert.ok(html.indexOf("wf-pipeline.js") > 0, "index.html must include wf-pipeline.js");
});
