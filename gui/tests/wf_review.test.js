// node --test gui/tests/ — the agent-review launcher's pure half.
//
// The composer model decides which choices a round even offers, and the
// answer-comments copy is the only place the unanswered-count semantics are
// visible to the human — both worth pinning without a DOM.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  reviewRoundModel,
  interactiveParam,
  answerCommentsLabel,
  answerCommentsTitle,
  answerCommentsConfirm,
} = require("../dist/wf-review.js");

test("a code round with a PR offers both dimensions on one screen", () => {
  const m = reviewRoundModel({ round: 3, target: "diff", hasPr: true, prNumber: 42 });
  assert.equal(m.title, "Agent review — round 3");
  assert.equal(m.launchLabel, "Launch round 3");
  assert.deepEqual(
    m.depth.choices.map((c) => c.value),
    ["deep", "standard"]
  );
  assert.deepEqual(
    m.publish.choices.map((c) => c.value),
    ["local", "pr-comments"]
  );
  // The PR option names the actual PR, not an abstraction.
  assert.match(m.publish.choices[1].label, /#42/);
});

test("respond-pr-comments is not a composer choice — it has its own action", () => {
  const m = reviewRoundModel({ round: 1, target: "diff", hasPr: true, prNumber: 7 });
  for (const c of m.publish.choices) {
    assert.notEqual(c.value, "respond-pr-comments");
  }
});

test("a plan round asks no depth — the plan engine has one", () => {
  const m = reviewRoundModel({ round: 1, target: "plan", hasPr: false });
  assert.equal(m.depth, null);
  assert.match(m.intro, /plan/);
});

test("without a PR there is no publish question", () => {
  const m = reviewRoundModel({ round: 2, target: "diff", hasPr: false });
  assert.equal(m.publish, null);
  assert.notEqual(m.depth, null);
});

test("defaults match what the old dialogs made primary", () => {
  const m = reviewRoundModel({ round: 1, target: "diff", hasPr: true, prNumber: 1 });
  assert.equal(m.depth.default, "deep");
  assert.equal(m.publish.default, "local");
});

test("every round offers the interaction choice, defaulting to ask-in-session", () => {
  // Unlike depth/publish, the dimension always exists: plan rounds and
  // PR-less rounds still run interactively or not.
  for (const args of [
    { target: "plan", hasPr: false },
    { target: "diff", hasPr: true, prNumber: 3 },
    {},
  ]) {
    const m = reviewRoundModel(args);
    assert.equal(m.interaction.default, "ask");
    assert.deepEqual(
      m.interaction.choices.map((c) => c.value),
      ["ask", "interactive", "autonomous"]
    );
  }
});

test("the interaction choice maps to the backend tri-state", () => {
  // null = "the skill asks in-session" — the kickoff omits the field.
  assert.equal(interactiveParam("ask"), null);
  assert.equal(interactiveParam("interactive"), true);
  assert.equal(interactiveParam("autonomous"), false);
  assert.equal(interactiveParam(undefined), null);
});

test("the model tolerates an empty call", () => {
  const m = reviewRoundModel();
  assert.equal(m.title, "Agent review — round 1");
  assert.equal(m.publish, null);
});

test("the answer-comments label only claims a count it has", () => {
  assert.equal(answerCommentsLabel(3), "Answer 3 PR comments");
  assert.equal(answerCommentsLabel(1), "Answer 1 PR comment");
  // 0, null and undefined all stay generic: the count is advisory and stale
  // by up to a poll, so the button never asserts "nothing to do".
  assert.equal(answerCommentsLabel(0), "Answer PR comments");
  assert.equal(answerCommentsLabel(null), "Answer PR comments");
  assert.equal(answerCommentsLabel(undefined), "Answer PR comments");
});

test("a known-zero count is honest in the tooltip, not hidden", () => {
  assert.match(answerCommentsTitle(0, "#5"), /None unanswered at last check/);
  assert.doesNotMatch(answerCommentsTitle(2, "#5"), /None unanswered/);
  assert.match(answerCommentsTitle(null, "#5"), /#5/);
});

test("the confirm says what the agent will actually do", () => {
  const c = answerCommentsConfirm("#9", 2);
  assert.match(c, /#9/);
  assert.match(c, /2 unanswered review comments/);
  assert.match(c, /comment queue/);
  // Unknown count: no invented number.
  assert.match(answerCommentsConfirm("the PR", null), /its review comments/);
});

test("the browser branch publishes every name app.js calls", () => {
  // app.js is a plain script that reads these off `window`, so a rename here
  // (or a missing `<script>` tag) fails at click time, not at boot. Same
  // guard as wf-compose.js.
  const fs = require("node:fs");
  const path = require("node:path");
  const vm = require("node:vm");

  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-review.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });

  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  const used = [
    "reviewRoundModel",
    "interactiveParam",
    "answerCommentsLabel",
    "answerCommentsTitle",
    "answerCommentsConfirm",
  ];
  for (const name of used) {
    assert.equal(typeof win[name], "function", `${name} must be on window`);
    assert.ok(app.includes(name), `app.js is expected to use ${name}`);
  }

  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(
    html.indexOf("wf-review.js") < html.indexOf("app.js"),
    "wf-review.js must be loaded before app.js"
  );
});
