// node --test gui/tests/ — the workflow launch's agent-choice model.
//
// A start asks which agent runs it only under the `ask` setting; then what is
// greyed and what is pre-selected is the whole behaviour worth pinning.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  wfAgentChoices,
  wfAgentDefault,
  wfAgentAsks,
  wfFixedAgent,
} = require("../dist/wf-agent.js");

test("an agent whose binary is missing is listed, greyed, with the reason", () => {
  const choices = wfAgentChoices({ claudeAvailable: true, ompAvailable: false, ompBin: "/opt/omp" });
  assert.deepEqual(
    choices.map((c) => [c.value, c.available]),
    [
      ["claude", true],
      ["omp", false],
    ]
  );
  assert.match(choices[1].reason, /\/opt\/omp/);
  assert.equal(choices[0].reason, "");
});

test("missing settings grey nothing — the backend refuses a bad binary", () => {
  assert.ok(wfAgentChoices({}).every((c) => c.available));
});

test("the item's agent wins, then the global workflow agent", () => {
  const both = { claudeAvailable: true, ompAvailable: true, workflowAgent: "omp" };
  assert.equal(wfAgentDefault("claude", both), "claude");
  assert.equal(wfAgentDefault("", both), "omp");
  assert.equal(wfAgentDefault("", { claudeAvailable: true, ompAvailable: true }), "claude");
});

test("an unavailable preference falls back to one that is available", () => {
  assert.equal(
    wfAgentDefault("omp", { claudeAvailable: true, ompAvailable: false }),
    "claude"
  );
  assert.equal(
    wfAgentDefault("claude", { claudeAvailable: false, ompAvailable: true }),
    "omp"
  );
  // Nothing available: keep the preference so the refusal names its binary.
  assert.equal(wfAgentDefault("omp", { claudeAvailable: false, ompAvailable: false }), "omp");
});

test("a start asks only when the item's setting, else the global one, is ask", () => {
  assert.equal(wfAgentAsks("", { workflowAgent: "ask" }), true);
  assert.equal(wfAgentAsks("ask", { workflowAgent: "omp" }), true);
  assert.equal(wfAgentAsks("claude", { workflowAgent: "ask" }), false);
  assert.equal(wfAgentAsks("", { workflowAgent: "omp" }), false);
  // Settings that never arrived: the shipped default, which asks.
  assert.equal(wfAgentAsks("", {}), true);
});

test("an unasked start runs on the fixed setting", () => {
  assert.equal(wfFixedAgent("omp", { workflowAgent: "claude" }), "omp");
  assert.equal(wfFixedAgent("", { workflowAgent: "omp" }), "omp");
});

test("a global ask pre-selects the last agent, never the word ask", () => {
  const both = { claudeAvailable: true, ompAvailable: true, workflowAgent: "ask" };
  assert.equal(wfAgentDefault("omp", both), "omp");
  assert.equal(wfAgentDefault("", both), "claude");
});
