// node --test gui/tests/ — the workflow timeline's pure half.
//
// The model merges four sources (review.md iterations, agent-review rounds,
// history snapshots, item creation) into one newest-first feed; the ordering
// and the join rules are exactly what a DOM test could not pin.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  timelineModel,
  parseStamp,
  parseReviewHeading,
} = require("../dist/wf-timeline.js");

test("stamps parse the clash/skill format and nothing else", () => {
  const t = parseStamp("2026-08-01 10:30");
  assert.ok(typeof t === "number" && t > 0);
  // The T-separated form works too; garbage and absence yield null.
  assert.ok(parseStamp("2026-08-01T10:30") > 0);
  assert.equal(parseStamp("last tuesday"), null);
  assert.equal(parseStamp(""), null);
  assert.equal(parseStamp(undefined), null);
});

test("review headings split into target · depth · date", () => {
  assert.deepEqual(parseReviewHeading("diff · deep · 2026-08-04 17:27"), {
    target: "diff",
    depth: "deep",
    date: "2026-08-04 17:27",
  });
  // Missing pieces degrade to empty strings — the heading is agent prose.
  assert.deepEqual(parseReviewHeading("plan"), { target: "plan", depth: "", date: "" });
  assert.deepEqual(parseReviewHeading(""), { target: "", depth: "", date: "" });
});

test("events interleave by date, newest first, creation last", () => {
  const events = timelineModel({
    iterations: [
      { iteration: 1, heading: "2026-08-01 10:00", note: "tighten", annotations: [] },
      { iteration: 2, heading: "2026-08-03 09:00", note: "again", annotations: [] },
    ],
    reviews: [
      { round: 1, heading: "plan · deep · 2026-08-02 12:00", verdict: "ok", published: [] },
    ],
    history: [1, 2],
    planSnapshots: [1],
    createdAt: parseStamp("2026-07-30 08:00"),
  });
  assert.deepEqual(
    events.map((e) => e.kind),
    ["change-round", "agent-review", "change-round", "created"]
  );
  assert.equal(events[0].iteration, 2);
  assert.equal(events[1].round, 1);
  assert.equal(events[2].iteration, 1);
});

test("change rounds join their snapshots and plan copies by iteration", () => {
  const [round] = timelineModel({
    iterations: [{ iteration: 3, heading: "2026-08-01 10:00", note: "n", annotations: ["`f:1` — x"] }],
    history: [3],
    planSnapshots: [3],
    hasPlanPhase: true,
  });
  assert.equal(round.kind, "change-round");
  assert.ok(round.hasCodeDiff);
  assert.ok(round.hasPlanDiff);
  assert.ok(round.hasPlanSnapshot);
  assert.deepEqual(round.annotations, ["`f:1` — x"]);
});

test("review-only items never offer a plan diff", () => {
  const [round] = timelineModel({
    iterations: [{ iteration: 1, heading: "2026-08-01 10:00", note: "", annotations: [] }],
    history: [1],
    planSnapshots: [1],
    hasPlanPhase: false,
  });
  assert.equal(round.hasPlanDiff, false);
});

test("a snapshot without a review.md section still gets a card", () => {
  // Legacy note-less rounds: the diff must stay reachable from the timeline.
  const events = timelineModel({ iterations: [], history: [1, 2], planSnapshots: [] });
  const rounds = events.filter((e) => e.kind === "change-round");
  assert.deepEqual(
    rounds.map((r) => r.iteration),
    [2, 1] // newest first
  );
  assert.ok(rounds.every((r) => r.hasCodeDiff && r.note === ""));
});

test("undated events keep their file order and creation stays oldest", () => {
  const events = timelineModel({
    iterations: [
      { iteration: 1, heading: "", note: "", annotations: [] },
      { iteration: 2, heading: "", note: "", annotations: [] },
    ],
    reviews: [],
    history: [],
    createdAt: parseStamp("2026-07-30 08:00"),
  });
  assert.deepEqual(
    events.map((e) => e.kind),
    ["change-round", "change-round", "created"]
  );
  // Newest first: iteration 2 (later in the file) above iteration 1.
  assert.equal(events[0].iteration, 2);
  assert.equal(events[1].iteration, 1);
});

test("the model tolerates an empty call", () => {
  const events = timelineModel();
  assert.equal(events.length, 1);
  assert.equal(events[0].kind, "created");
});

test("the browser branch publishes every name app.js calls", () => {
  const fs = require("node:fs");
  const path = require("node:path");
  const vm = require("node:vm");

  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-timeline.js"), "utf8");
  const win = {};
  vm.runInNewContext(src, { window: win });

  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  for (const name of ["timelineModel"]) {
    assert.equal(typeof win[name], "function", `${name} must be on window`);
    assert.ok(app.includes(name), `app.js is expected to use ${name}`);
  }

  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(
    html.indexOf("wf-timeline.js") < html.indexOf("app.js"),
    "wf-timeline.js must be loaded before app.js"
  );
});
