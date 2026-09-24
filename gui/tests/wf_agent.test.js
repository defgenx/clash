// node --test gui/tests/ — the workflow launch's agent-choice model.
//
// Every start asks which agent runs it, so what is greyed and what is
// pre-selected is the whole behaviour worth pinning.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { wfAgentChoices, wfAgentDefault } = require("../dist/wf-agent.js");

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
