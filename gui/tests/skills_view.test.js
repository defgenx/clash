// node --test gui/tests/ — the ☰ Skills tab's grouping model.
//
// One row per skill, one chip per installed agent CLI, and the harness the
// viewer opens on: those are the behaviours worth pinning.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { skillChips, skillNeedsAttention, skillViewerChoice, skillGroups } = require("../dist/skills-view.js");

const workflow = {
  name: "clash-workflow",
  managed: true,
  copies: [
    { agent: "hermes", state: "outdated", path: "/h" },
    { agent: "claude", state: "current", path: "/c" },
    { agent: "omp", state: "edited", path: "/o" },
  ],
};

test("chips follow harness order whatever order the copies arrive in", () => {
  assert.deepEqual(
    skillChips(workflow).map((c) => [c.short, c.glyph]),
    [
      ["CC", "✓"],
      ["OMP", "✎"],
      ["HM", "⚠"],
    ]
  );
  assert.match(skillChips(workflow)[2].title, /^Hermes: outdated/);
});

test("a harness that is not installed has no chip; a missing skill shows a dash", () => {
  const s = { managed: true, copies: [{ agent: "claude", state: "missing", path: "" }] };
  assert.deepEqual(
    skillChips(s).map((c) => [c.short, c.glyph]),
    [["CC", "—"]]
  );
});

test("attention is a managed copy that differs from what clash ships", () => {
  assert.equal(skillNeedsAttention(workflow), true);
  assert.equal(skillNeedsAttention({ managed: true, copies: [{ agent: "claude", state: "current" }] }), false);
  assert.equal(skillNeedsAttention({ managed: true, copies: [{ agent: "claude", state: "missing" }] }), false);
  assert.equal(skillNeedsAttention({ managed: false, copies: [{ agent: "claude", state: "present" }] }), false);
});

test("the viewer opens the remembered harness, else the one needing attention", () => {
  assert.deepEqual(skillViewerChoice(workflow, "hermes"), { agents: ["claude", "omp", "hermes"], pick: "hermes" });
  assert.equal(skillViewerChoice(workflow, "").pick, "omp");
  const clean = { managed: true, copies: [{ agent: "claude", state: "current" }, { agent: "omp", state: "current" }] };
  assert.equal(skillViewerChoice(clean, "").pick, "claude");
  // A remembered harness without a copy on disk is not offered.
  const partial = { managed: true, copies: [{ agent: "claude", state: "current" }, { agent: "hermes", state: "missing" }] };
  assert.deepEqual(skillViewerChoice(partial, "hermes"), { agents: ["claude"], pick: "claude" });
  assert.deepEqual(skillViewerChoice({ copies: [] }, "claude"), { agents: [], pick: "" });
});

test("clash's skills come first and empty groups are dropped", () => {
  const other = { name: "go-conventions", managed: false, copies: [] };
  assert.deepEqual(
    skillGroups([other, workflow]).map((g) => [g.title, g.skills.map((s) => s.name)]),
    [
      ["Managed by clash", ["clash-workflow"]],
      ["Other skills", ["go-conventions"]],
    ]
  );
  assert.deepEqual(skillGroups([other]).map((g) => g.title), ["Other skills"]);
});
