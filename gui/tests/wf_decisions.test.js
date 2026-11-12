// node --test gui/tests/ — the decision queue, the pure half, and where it
// surfaces: the Decisions tab, the bar's recommendable button, the rail and
// the attention inbox. Contract: docs/workflows.md → Decisions.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const D = require("../dist/wf-decisions.js");
const { wfNextStep } = require("../dist/wf-next.js");
const { workflowReasons } = require("../dist/attention.js");
const { railModel } = require("../dist/wf-rail.js");
const O = require("../dist/wf-outline.js");

const decisions = [
  {
    id: "D3",
    text: "Keep the legacy flag?",
    options: [
      { key: "a", text: "keep it" },
      { key: "b", text: "drop it" },
    ],
    recommended: "a",
    firstSeenIteration: 2,
    status: "open",
  },
  { id: "D4", text: "Retry queue?", recommended: "a dedicated one", status: "open" },
  { id: "D1", text: "Old one", status: "answered", answer: "yes", answeredAt: 5, deliveredIteration: 0 },
  { id: "D2", text: "Older", status: "answered", answer: "no", answeredAt: 9, deliveredIteration: 2 },
  { id: "D0", text: "Gone", status: "dropped", answeredAt: 1 },
];
const item = (status, extra = {}) => ({
  project: "p",
  slug: "s",
  agentAlive: true,
  openAnnotations: 0,
  reviewRounds: {},
  decisions,
  ...extra,
  meta: { status, iteration: 2 },
});

test("open decisions with their options, the recommended one flagged", () => {
  const m = D.decisionsModel(item("plan-review"));
  assert.deepEqual(m.open.map((d) => d.id), ["D3", "D4"]);
  assert.deepEqual(
    m.open[0].options.map((o) => [o.label, o.recommended]),
    [
      ["a) keep it", true],
      ["b) drop it", false],
    ]
  );
  assert.equal(m.open[0].recommendedText, "");
  // A recommendation naming no option is shown as the agent's words.
  assert.equal(m.open[1].recommendedText, "a dedicated one");
  // Settled: newest answer first, each saying where its answer went.
  assert.deepEqual(m.settled.map((d) => d.id), ["D2", "D1", "D0"]);
  assert.equal(m.settled[0].state, "handed to this round (iteration 2)");
  assert.equal(m.settled[1].state, "goes to the next round");
  assert.equal(m.settled[2].state, "dropped");
  assert.equal(D.optionAnswer({ key: "b", text: "x" }), "b) x");
  // The composer names the answers the change request carries.
  assert.equal(D.unsentAnswersNote(item("plan-review")), " Your answer to D1 is appended under ### Decisions.");
  assert.equal(D.unsentAnswersNote(item("plan-review", { decisions: [] })), "");
});

test("the tab label counts what is open, and the outline files it under Feedback", () => {
  assert.equal(D.decisionsTabLabel(item("plan-review")), "❓ Decisions (2)");
  assert.equal(D.decisionsTabLabel(item("plan-review", { decisions: [] })), "❓ Decisions");
  const o = O.wfOutline([["review", "Change requests"], ["decisions", "❓ Decisions (2)"]], "");
  const entry = o.groups.find((g) => g.id === "feedback").items.find((i) => i.id === "decisions");
  assert.equal(entry.text, "Decisions");
  assert.equal(entry.badge, 2);
});

test("the inbox says how many wait, on the item's one row, only when parked", () => {
  const parked = workflowReasons(item("diff-review")).map((r) => r.text);
  assert.deepEqual(parked, ["parked on your decision", "2 decisions waiting"]);
  assert.deepEqual(workflowReasons(item("implementing")), []);
  assert.equal(D.decisionsReason(item("pr-ready")), "2 decisions waiting");
  assert.equal(D.decisionsReason(item("plan-review", { decisions: [decisions[2]] })), null);
});

test("answering comes before approving, and before applying a review", () => {
  const available = new Set(["decisions", "approve-plan", "apply-review", "approve-done", "approve-pr-draft"]);
  const pending = { round: 1, apply: true };
  for (const st of ["plan-review", "diff-review"]) {
    const step = wfNextStep(item(st), { available, pending, prs: [], hasPlan: true });
    assert.equal(step.id, "decisions", st);
    assert.equal(step.auto, false);
  }
  // Nothing open: the stage's own path.
  const none = wfNextStep(item("plan-review", { decisions: [] }), { available, pending, prs: [] });
  assert.equal(none.id, "apply-review");
});

test("the rail lists the open ids unless the Decisions tab is open", () => {
  assert.deepEqual(railModel(item("plan-review"), []).decisions, {
    open: 2,
    text: "2 questions only you can answer",
    ids: ["D3", "D4"],
  });
  assert.equal(railModel(item("plan-review"), [], { subView: "decisions" }).decisions, null);
  assert.equal(railModel(item("plan-review", { decisions: [] }), []).decisions, null);
});

test("the browser build publishes what app.js reads, and app.js wires it", () => {
  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-decisions.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });
  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  for (const name of [
    "DECISION_STAGES",
    "openDecisions",
    "decisionsModel",
    "decisionsTabLabel",
    "unsentAnswersNote",
  ]) {
    assert.ok(name in win, `${name} is not published`);
    assert.ok(app.includes(name), `${name} is no longer read by app.js`);
  }
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(html.indexOf("wf-decisions.js") < html.indexOf("app.js"));
  // The bar button carries the act id the recommender needs.
  assert.match(app, /"step",\s*"decisions"\s*\)/);
  // Both commands exist and are registered.
  const rs = fs.readFileSync(path.join(__dirname, "..", "src-tauri", "src", "main.rs"), "utf8");
  for (const cmd of ["answer_workflow_decision", "drop_workflow_decision"]) {
    assert.ok(app.includes(`"${cmd}"`), `${cmd} is never invoked`);
    assert.ok(rs.includes(`workflows::${cmd},`), `${cmd} is not registered`);
  }
});

const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

test("the header counts the decisions and offers the recommendations in one click", () => {
  const item = { meta: { iteration: 4 }, decisions };
  const m = D.decisionsModel(item);
  const s = D.decisionsSummary(m);
  assert.equal(s.open, 2);
  assert.equal(s.answered, 2);
  assert.equal(s.dropped, 1);
  assert.equal(s.percent, 60);
  assert.equal(s.headline, "2 waiting on you");
  assert.match(s.next, /^1 answer will reach the next round/);
  assert.equal(s.acceptable, 1, "only D3 has a recommended option");
  // Waiting rounds: D3 was asked at iteration 2, it is now 4.
  const d3 = m.open.find((d) => d.id === "D3");
  const age = D.decisionAge(d3, 4);
  assert.equal(age.rounds, 2);
  assert.ok(age.stale);
  assert.match(age.text, /asked in iteration 2 · waiting 2 rounds/);
  assert.equal(D.decisionAge({ firstSeen: 4 }, 4).text, "asked this round (iteration 4)");
  // Filters and search.
  assert.deepEqual(D.filterDecisions(m, "open", "").map((d) => d.id), ["D3", "D4"]);
  assert.deepEqual(D.filterDecisions(m, "answered", "").map((d) => d.status).sort(), ["answered", "answered", "dropped"]);
  assert.deepEqual(D.filterDecisions(m, "all", "drop it").map((d) => d.id), ["D3"], "options are searched");
  // A discussion starts from the question itself.
  assert.match(D.decisionTopic(d3), /^Decision D3: Keep the legacy flag\? Options: a\) keep it; b\) drop it\. The agent recommends a\./);
  // Nothing at all: says where decisions come from.
  assert.match(D.decisionsSummary(D.decisionsModel({ meta: {}, decisions: [] })).headline, /No decisions yet/);
});

test("the Decisions view renders its cards and wires every button", async () => {
  const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  const calls = [];
  const sandbox = {
    ...D,
    document: { createElement: (t) => stubEl(t) },
    escapeHtml: (s) => String(s),
    invoke: async (cmd, args) => calls.push([cmd, args]),
    busyButton: (_b, fn) => fn(),
    flashToast: () => {},
    uiAlert: () => {},
    uiConfirm: async () => true,
    uiTextPrompt: async () => null,
    refreshWorkflows: async () => {},
    buildWorkflowView: () => {},
    wfDiscuss: async (_item, opts) => calls.push(["discuss", opts]),
    wfSelfWriteAt: 0,
  };
  vm.createContext(sandbox);
  vm.runInContext(extractFunction(APP, "wfInlineMd"), sandbox);
  vm.runInContext(extractFunction(APP, "renderWfDecisionsView"), sandbox);
  const item = { project: "p", slug: "s", meta: { iteration: 4 }, decisions };
  const body = stubEl();
  const ts = {};
  await sandbox.renderWfDecisionsView(body, stubEl(), item, ts);
  const all = descendants(body);
  const buttons = all.filter((e) => e.tagName === "BUTTON");
  const label = (t) => buttons.find((b) => (b.textContent || b.innerHTML || "").includes(t));
  assert.equal(ts.decFilter, "open", "opens on what is waiting");
  assert.ok(all.some((e) => (e.className || "").includes("wf-decision-option recommended")), "options are cards");
  // Every button does something.
  for (const b of buttons) assert.equal(typeof b.onclick, "function", `${b.textContent || b.innerHTML} has no handler`);
  await label("Take the recommendation").onclick();
  assert.equal(JSON.stringify(calls.at(-1)), JSON.stringify(["answer_workflow_decision", { project: "p", slug: "s", id: "D3", answer: "a) keep it" }]));
  await label("Discuss").onclick();
  assert.equal(calls.at(-1)[0], "discuss");
  assert.match(calls.at(-1)[1].topic, /^Decision D3/);
  await label("Accept the recommendation").onclick();
  assert.equal(calls.at(-1)[1].id, "D3");
});

test("every button is styled, even one nobody gave a class", () => {
  const css = fs.readFileSync(path.join(__dirname, "..", "dist", "style.css"), "utf8");
  assert.match(css, /:where\(button:not\(\[class\]\), button\[class="primary"\]\) \{[^}]*background: var\(--bg-hover\)/);
  assert.match(css, /:where\(button\[class="primary"\]\) \{[^}]*var\(--accent\)/);
});
