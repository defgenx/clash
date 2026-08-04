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
  for (const name of ["wfPipelineModel"]) {
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
