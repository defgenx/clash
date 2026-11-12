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
  waveName,
  waveList,
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
  assert.equal(waveSummary(item(["ready", "blocked", "blocked", "blocked"])).text, "Waves 0/4 · Wave 1 ready");
  // People read "Wave 2", never the waves.json id.
  assert.equal(waveName("w2"), "Wave 2");
  assert.equal(waveName({ id: "W10" }), "Wave 10");
  assert.equal(waveName("api"), "api");
  assert.equal(waveList(["w2"]), "Wave 2");
  assert.equal(waveList(["w2", "w3", "w5"]), "Waves 2, 3 and 5");
  assert.equal(waveList(["w2", "api"]), "Wave 2 and api");
  assert.equal(waveSummary(item(["done", "active", "active", "blocked"])).text, "Waves 1/4 · Waves 2 and 3 in progress");
  assert.equal(waveSummary(item(["done", "review", "review", "blocked"])).text, "Waves 1/4 · Waves 2 and 3 to sign off");
  assert.equal(waveSummary(item(["done", "done", "done", "done"])).text, "Waves 4/4 · all signed off");
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
  assert.equal(rows[1].reason, "Can't run with Wave 2", "people read wave names, never ids");
  assert.deepEqual(toggleWavePick(it, ["w2"], "w3"), ["w2"]);
  // A wave without areas says why it runs alone.
  it.waves.waves[2].areas = [];
  assert.match(wavePickRows(it, [])[1].detail, /runs alone/);
});

test("the board's columns follow dependency levels", () => {
  const cols = waveColumns(item(["done", "ready", "ready", "blocked"]));
  assert.deepEqual(cols.map((c) => c.waves.map((x) => x.id)), [["w1"], ["w2", "w3"], ["w4"]]);
  assert.equal(cols[0].title, "Step 1");
  assert.equal(cols[2].title, "Step 3");
});

test("the browser branch publishes every name app.js calls", () => {
  const src = fs.readFileSync(path.join(__dirname, "../dist/wf-waves.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });
  for (const name of [
    "waveBoard",
    "wavesIssue",
    "waveName",
    "waveLabel",
    "waveList",
    "waveSummary",
    "waveBarState",
    "wavesHoldApproval",
    "wavesIssueGuidance",
    "wavePickRows",
    "defaultWavePick",
    "toggleWavePick",
    "waveColumns",
    "waveStateLabel",
    "wavePrs",
    "waveNarrative",
    "waveBlockers",
    "waveActions",
    "waveLive",
    "waveFocus",
  ])
    assert.ok(win[name], name);
});

const { wavePrs, waveNarrative } = require("../dist/wf-waves.js");

// The roadmap follows the stage: these are at a parked code stage.
const at = (it, status) => ({ ...it, meta: { ...(it.meta || {}), status } });

test("the roadmap says what is happening and what to do next", () => {
  const it = at(item(["review", "blocked", "blocked", "blocked"]), "diff-review");
  it.waves.waves[0].prs = ["u1", "u2", "u3"];
  it.waves.waves[1].blockedBy = ["w1"];
  it.waves.waves[2].blockedBy = ["w1"];
  it.waves.waves[3].blockedBy = ["w2", "w3"];
  const prs = [
    { url: "u1", state: "MERGED" },
    { url: "u2", state: "OPEN" },
    { url: "u3", state: "OPEN", draft: true },
  ];
  assert.deepEqual(
    { ...wavePrs(it.waves.waves[0], prs), prs: undefined },
    { prs: undefined, total: 3, merged: 1, closed: 0, open: 2 }
  );
  const n = waveNarrative(it, prs);
  assert.match(n.now, /Wave 1 · w1 title is waiting for your sign-off\. 1 of 3 PRs merged, 2 still open\./);
  assert.match(n.next, /get its PRs merged .* then sign it off — that unlocks Waves 2 and 3\./);
  // Merged: just approve.
  const merged = waveNarrative(it, prs.map((p) => ({ ...p, state: "MERGED" })));
  assert.equal(merged.next, "Sign it off — that unlocks Waves 2 and 3.");
  // Ready siblings that can share a round are offered together.
  const ready = waveNarrative(at(item(["done", "ready", "ready", "blocked"]), "diff-review"), []);
  assert.match(ready.now, /1 of 4 waves signed off\. Ready to start: Waves 2 and 3\./);
  assert.match(ready.next, /or Waves 2 and 3 together/);
  assert.match(waveNarrative(item(["done", "active", "blocked", "blocked"]), []).now, /implementing Wave 2/);
  assert.match(waveNarrative(item(["done", "done", "done", "done"]), []).now, /All 4 waves are signed off/);
  assert.match(waveNarrative({ waves: { waves: [], issue: "a cycle" } }, []).now, /can't be used: a cycle/);
  assert.equal(waveNarrative({}, []), null);
});

test("steps say which of their waves can share a round", () => {
  const cols = waveColumns(item(["done", "ready", "ready", "blocked"]));
  assert.equal(cols[1].title, "Step 2");
  assert.equal(cols[1].detail, "These can run in the same round.");
  assert.equal(cols[0].detail, "");
});

const { waveBlockers, waveActions, waveLive, waveFocus } = require("../dist/wf-waves.js");

test("a wave's blockers: a dependency stops it, PR facts only precede approval", () => {
  const it = item(["review", "blocked", "blocked", "blocked"]);
  it.waves.waves[0].prs = ["u1", "u2", "u3"];
  it.waves.waves[1].blockedBy = ["w1"];
  const prs = [
    { url: "u1", state: "MERGED" },
    { url: "u2", state: "OPEN", unanswered: 2 },
    { url: "u3", state: "OPEN", draft: true, error: "HTTP 502" },
  ];
  const b1 = waveBlockers(it, it.waves.waves[0], prs);
  assert.deepEqual(b1.map((b) => [b.kind, b.hard]), [["prs", false], ["threads", false], ["error", false]]);
  assert.equal(b1[0].text, "2 PRs not merged yet (1 draft)");
  assert.deepEqual(b1[0].urls, ["u2", "u3"]);
  assert.equal(b1[1].text, "2 PR threads waiting on you");
  const b2 = waveBlockers(it, it.waves.waves[1], prs);
  assert.deepEqual(b2, [{ kind: "wave", hard: true, wave: "w1", text: "Waits for Wave 1 · w1 title — to sign off" }]);
  assert.deepEqual(waveBlockers({}, it.waves.waves[0], prs), []);
});

test("a wave's actions are the bar's decisions, offered only where the bar offers them", () => {
  const it = { ...item(["review", "blocked", "blocked", "blocked"]), meta: { status: "pr-ready" } };
  it.waves.waves[0].prs = ["u1", "u2"];
  const prs = [{ url: "u1", state: "OPEN" }, { url: "u2", state: "MERGED" }];
  const ids = (acts) => acts.map((a) => a.id);
  assert.deepEqual(ids(waveActions(it, it.waves.waves[0], prs)), ["approve-next", "approve", "review", "open-prs"]);
  assert.deepEqual(waveActions(it, it.waves.waves[0], prs)[2].urls, ["u1"], "a review reads what is still open");
  assert.deepEqual(ids(waveActions(it, it.waves.waves[1], prs)), [], "a waiting wave decides nothing");
  // An agent at work: nothing to decide, only its session.
  const busy = { ...item(["done", "active", "active", "blocked"]), meta: { status: "implementing" } };
  assert.deepEqual(ids(waveActions(busy, busy.waves.waves[1], [])), ["session"]);
  // Waves the plan allows in parallel: together or alone is your call.
  const ready = { ...item(["done", "ready", "ready", "blocked"]), meta: { status: "diff-review" } };
  const both = waveActions(ready, ready.waves.waves[1], []);
  assert.deepEqual(both.map((a) => [a.label, a.ids, !!a.primary]), [
    ["▶ Start Waves 2 and 3 together…", ["w2", "w3"], true],
    ["▶ Start Wave 2 alone…", ["w2"], false],
  ]);
  // Waves that cannot share a round start one by one.
  const apart = { ...item(["done", "ready", "ready", "blocked"]), meta: { status: "diff-review" } };
  apart.waves.waves[1].compatibleWith = [];
  apart.waves.waves[2].compatibleWith = [];
  assert.deepEqual(waveActions(apart, apart.waves.waves[1], []).map((a) => [a.label, a.ids]), [["▶ Start Wave 2…", ["w2"]]]);
  // At plan review, starting is approving the plan.
  const plan = { ...item(["ready", "blocked", "blocked", "blocked"]), meta: { status: "plan-review" } };
  assert.equal(waveActions(plan, plan.waves.waves[0], [])[0].label, "✓ Approve plan → start Wave 1…");
});

test("live state: waves in parallel share one agent, and its state is said", () => {
  const it = {
    project: "p",
    slug: "s",
    meta: { status: "implementing", sessionId: "a" },
    ...item(["done", "active", "active", "blocked"]),
  };
  const now = new Date(2026, 10, 10, 12, 0).getTime();
  const sessions = [
    { id: "a", name: "S · implement", status: "Thinking", is_running: true, workflow_item: "p/s", last_modified: "2026-11-10 11:57" },
    { id: "b", name: "other", status: "Thinking", is_running: true, workflow_item: "p/x" },
  ];
  const m = waveLive(it, sessions, now);
  assert.equal(m.title, "Waves 2 and 3 running together");
  assert.match(m.shared, /One round, one agent, one worktree/);
  assert.equal(m.kind, "working");
  assert.equal(m.activity, "last activity 3 min ago");
  assert.equal(m.agentId, "a");
  assert.equal(waveLive(it, [{ ...sessions[0], status: "Prompting" }], now).kind, "attention");
  assert.equal(waveLive({ ...it, agentResumable: true }, [], now).kind, "stopped");
  assert.equal(waveLive({ ...it, agentAlive: false }, [], now).kind, "gone");
  assert.equal(waveLive(item(["review", "blocked", "blocked", "blocked"]), sessions, now), null, "nothing running, no panel");
});

test("one at a time opens on the wave that needs you, and keeps a valid pick", () => {
  assert.equal(waveFocus(item(["done", "review", "ready", "blocked"])), "w2");
  assert.equal(waveFocus(item(["done", "done", "active", "blocked"])), "w3");
  assert.equal(waveFocus(item(["done", "done", "done", "done"])), "w4");
  assert.equal(waveFocus(item(["done", "review", "ready", "blocked"]), "w4"), "w4");
  assert.equal(waveFocus(item(["done", "review", "ready", "blocked"]), "nope"), "w2");
});

const { signOffModel } = require("../dist/wf-waves.js");

test("a sign-off says what it does before it does it", () => {
  // w2 and w3 in review; signing them off unlocks w4.
  const it = { ...item(["done", "review", "review", "blocked"]), openAnnotations: 2, meta: { status: "diff-review" } };
  it.waves.waves[3].blockedBy = ["w2", "w3"];
  it.waves.waves[1].prs = ["u1", "u2"];
  const prs = [{ url: "u1", state: "OPEN", draft: true }, { url: "u2", state: "MERGED" }];
  const m = signOffModel(it, prs);
  assert.equal(m.title, "Sign off Wave 2 · w2 title, Wave 3 · w3 title?");
  assert.equal(m.okLabel, "Sign off Waves 2 and 3");
  assert.match(m.effects[0], /Seals their work at the current commit — later agent rounds are told it is accepted/);
  assert.equal(m.effects[1], "Unlocks Wave 4 · w4 title.");
  assert.deepEqual(m.warnings, ["2 comments are still open on the diff.", "Wave 2: 1 PR not merged yet (1 draft)."]);
  // The one effect that leaves clash is offered, never assumed.
  assert.deepEqual(m.drafts, [{ url: "u1", wave: "w2", waves: ["w2"] }]);
  // A PR shared by both waves of the round is offered once, naming both.
  it.waves.waves[2].prs = ["u1"];
  assert.deepEqual(signOffModel(it, prs).drafts, [{ url: "u1", wave: "w2", waves: ["w2", "w3"] }]);
  // The last waves hand over to the item's own approval.
  const last = { ...item(["done", "done", "done", "review"]), meta: { status: "diff-review" } };
  assert.match(signOffModel(last, []).effects[1], /last waves: the item's own approval comes next/);
  assert.equal(signOffModel(item(["done", "active", "blocked", "blocked"]), []), null);
});

test("a sign-off checks what it would seal, per repository", () => {
  const it = { ...item(["done", "review", "review", "blocked"]), meta: { status: "diff-review" } };
  const seals = [
    {
      wave: "w2",
      repos: [
        { repo: "api", branch: "item-w2", head: "aaa1111222", unpushed: 2, dirty: 0 },
        { repo: "web", branch: "item-w2", head: "bbb3333444", unpushed: 0, dirty: 3 },
      ],
    },
    // Started on the item branch and never moved, no PR: nothing to seal.
    { wave: "w3", repos: [{ repo: "api", branch: "item", head: "ccc", unpushed: 0, dirty: 0, sinceStart: 0 }] },
  ];
  const m = signOffModel(it, [], seals);
  assert.match(m.effects[0], /Seals their work \(api @ aaa1111, web @ bbb3333, api @ ccc\)/);
  assert.ok(m.warnings.includes("Wave 2: 2 commits on api (item-w2) not pushed."), m.warnings.join(" | "));
  assert.ok(m.warnings.includes("Wave 2: uncommitted changes in web (3 files)."));
  assert.ok(m.warnings.some((w) => /^Wave 3: nothing to seal/.test(w)));
  assert.equal(m.confirmEmpty, "Sign off Wave 3 anyway");
  // A wave with a PR, or with commits on its branch, needs no confirmation.
  it.waves.waves[2].prs = ["u9"];
  assert.equal(signOffModel(it, [{ url: "u9", state: "OPEN" }], seals).confirmEmpty, null);
  // No preview (the command failed): nothing extra is claimed or required.
  assert.equal(signOffModel(it, [], null).confirmEmpty, null);
});

const { wavesIssueGuidance } = require("../dist/wf-waves.js");

test("an unreadable waves.json holds the approvals and says how to repair it, by stage", () => {
  const broken = (status) => ({ meta: { status }, waves: { waves: [], issue: "a cycle" } });
  assert.ok(wavesHoldApproval(broken("pr-ready")));
  assert.equal(wavesIssueGuidance(broken("pr-ready")), "waves.json can't be read: a cycle — ask 💬 Discuss to repair it, or ↩ Move to… plan review.");
  assert.match(wavesIssueGuidance(broken("plan-review")), /request changes on the plan/);
  assert.equal(wavesIssueGuidance(item(["ready", "blocked", "blocked", "blocked"])), null);
  // At a code stage the planner is not who fixes it.
  assert.doesNotMatch(waveNarrative(broken("diff-review"), []).next, /Request changes on the plan/);
  assert.match(waveNarrative(broken("diff-review"), []).next, /💬 Discuss/);
  assert.match(waveNarrative(broken("plan-review"), []).next, /Request changes on the plan/);
});

test("the roadmap follows the item's stage", () => {
  const review = item(["done", "review", "blocked", "blocked"]);
  // An agent is working: nothing to sign off yet.
  const reviewing = { ...review, meta: { status: "reviewing", review: { returnStatus: "pr-ready" } } };
  assert.match(waveNarrative(reviewing, []).now, /A review round is running on Wave 2/);
  assert.doesNotMatch(waveNarrative(reviewing, []).next, /^Sign it off/);
  // Finished items ask for nothing.
  for (const status of ["done", "abandoned"]) {
    const n = waveNarrative(at(item(["done", "done", "ready", "blocked"]), status), []);
    assert.doesNotMatch(`${n.now} ${n.next}`, /Finish the item|Start|ready/, status);
    assert.match(n.next, /^Nothing/);
  }
  // At plan review starting is approving the plan, like the button says.
  const plan = waveNarrative(at(item(["ready", "blocked", "blocked", "blocked"]), "plan-review"), []);
  assert.match(plan.next, /^Approve the plan and start Wave 1/);
  const allDone = waveNarrative(at(item(["done", "done", "done", "done"]), "plan-review"), []);
  assert.equal(allDone.next, "Approve the plan to implement it, or request changes.");
  // A queued fix round is launched from the bar.
  const queued = waveNarrative(at(item(["done", "active", "blocked", "blocked"]), "changes-requested"), []);
  assert.match(queued.now, /A fix round is queued for Wave 2/);
  assert.equal(queued.next, "Launch it from the bar.");
  // Sign-off is suggested only at a parked code stage.
  for (const status of ["implementing", "planning", "draft", "plan-review", "changes-requested", "reviewing"])
    assert.doesNotMatch(waveNarrative(at(review, status), []).next, /sign (it|the wave) off/i, status);
  assert.match(waveNarrative(at(review, "pr-ready"), []).next, /Sign it off/);
  // The last wave in review: the item does not close by itself.
  assert.match(waveNarrative(at(item(["done", "done", "done", "review"]), "pr-ready"), []).next, /closes only once every wave is signed off/);
});

test("a PR in several waves of the round counts once in the story", () => {
  const it = at(item(["done", "review", "review", "blocked"]), "diff-review");
  it.waves.waves[1].prs = ["u1", "u2"];
  it.waves.waves[2].prs = ["u1"];
  const n = waveNarrative(it, [{ url: "u1", state: "OPEN" }, { url: "u2", state: "MERGED" }]);
  assert.match(n.now, /1 of 2 PRs merged, 1 still open\./);
});

test("a queued fix round shows no session to resume", () => {
  const it = { project: "p", slug: "s", meta: { status: "changes-requested", sessionId: "a" }, ...item(["done", "active", "blocked", "blocked"]) };
  const m = waveLive(it, [{ id: "a", status: "Stashed", is_running: false, workflow_item: "p/s" }]);
  assert.equal(m.kind, "queued");
  assert.equal(m.line, "A fix round is queued — launch it from the bar.");
  assert.equal(m.agentId, "");
  assert.doesNotMatch(m.title, /in progress/);
});

test("waves are signed off only at a code stage", () => {
  const it = at(item(["done", "review", "blocked", "blocked"]), "plan-review");
  assert.deepEqual(waveActions(it, it.waves.waves[1], []).filter((a) => /approve/.test(a.id)), []);
  const last = at(item(["done", "done", "done", "review"]), "pr-ready");
  assert.deepEqual(waveActions(last, last.waves.waves[3], []).map((a) => [a.id, a.label]), [["approve", "✓ Sign off Wave 4"]]);
});
