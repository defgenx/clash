// node --test gui/tests/ — the change-request composer's pure half.
//
// This text becomes the agent's instructions for the next round (review.md's
// latest `## Iteration` section), so the rules about what may be sent and how
// the annotations are summarised are worth pinning.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  changeRequestTemplate,
  composerPlaceholder,
  annotationsMarkdown,
  canSubmitChangeRequest,
  draftKey,
} = require("../dist/wf-compose.js");

test("the template carries the three things a change request needs", () => {
  const diff = changeRequestTemplate("diff");
  assert.match(diff, /^## What to change$/m);
  assert.match(diff, /^## Why$/m);
  assert.match(diff, /^## Out of scope$/m);
  // The prompts are HTML comments, so a half-filled template still renders
  // cleanly in review.md.
  assert.match(diff, /<!--/);
});

test("the plan template asks about the plan, not the code", () => {
  const plan = changeRequestTemplate("plan");
  assert.match(plan, /^## What to change in the plan$/m);
  assert.doesNotMatch(plan, /^## What to change$/m);
});

test("the placeholder tells you it is markdown and where it goes", () => {
  for (const target of ["plan", "diff"]) {
    const text = composerPlaceholder(target, 0);
    assert.match(text, /[Mm]arkdown/);
    assert.match(text, /next round/);
  }
});

test("with open comments the placeholder asks for framing, not a restatement", () => {
  const withOpen = composerPlaceholder("diff", 3);
  assert.match(withOpen, /framing/);
  // Without any, it has to ask for the substance instead.
  assert.match(composerPlaceholder("diff", 0), /What should change/);
});

test("annotations render as the same list review.md will contain", () => {
  const md = annotationsMarkdown([
    { file: "src/lib.rs", line: 12, body: "  rename this  " },
    { file: "gui/dist/app.js", line: 400, body: "dead branch" },
  ]);
  assert.equal(
    md,
    "- `src/lib.rs:12` — rename this\n- `gui/dist/app.js:400` — dead branch"
  );
});

test("annotationsMarkdown tolerates junk rather than throwing in a dialog", () => {
  assert.equal(annotationsMarkdown([]), "");
  assert.equal(annotationsMarkdown(null), "");
  assert.equal(annotationsMarkdown(undefined), "");
  // A line of 0 is a real line number, not a missing one.
  assert.equal(annotationsMarkdown([{ file: "a", line: 0, body: "x" }]), "- `a:0` — x");
  assert.equal(annotationsMarkdown([{}]), "- `?:?` — ");
});

test("a round with nothing to act on cannot be sent", () => {
  // Neither a note nor a comment would burn an agent session and come back
  // unchanged.
  const empty = canSubmitChangeRequest({ note: "   ", openCount: 0, target: "diff" });
  assert.equal(empty.ok, false);
  assert.match(empty.reason, /at least one diff comment or a note/);

  // Either one alone is enough.
  assert.equal(canSubmitChangeRequest({ note: "fix it", openCount: 0 }).ok, true);
  assert.equal(canSubmitChangeRequest({ note: "", openCount: 1 }).ok, true);
});

test("a plan round always needs a note — there are no plan annotations", () => {
  const plan = canSubmitChangeRequest({ note: "", openCount: 5, target: "plan" });
  assert.equal(plan.ok, false, "diff comments must not satisfy a plan revision");
  assert.match(plan.reason, /change in the plan/);
  assert.equal(canSubmitChangeRequest({ note: "rethink step 3", target: "plan" }).ok, true);
});

test("canSubmitChangeRequest defaults to refusing an empty call", () => {
  assert.equal(canSubmitChangeRequest().ok, false);
});

test("drafts are keyed per item so parallel reviews don't share a buffer", () => {
  assert.equal(draftKey("clash", "thing"), "clash/thing");
  assert.notEqual(draftKey("a", "x"), draftKey("b", "x"));
});

test("the browser branch publishes every name app.js calls", () => {
  // app.js is a plain script that reads these off `window`, so a rename here
  // (or a missing `<script>` tag) fails at click time, not at boot — the
  // frontend logs a clean "booted" line and the button just throws. This is the
  // cheapest way to catch it without a DOM.
  const fs = require("node:fs");
  const path = require("node:path");
  const vm = require("node:vm");

  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-compose.js"), "utf8");
  const win = {};
  // No `module` in scope → the IIFE takes its browser branch.
  vm.runInNewContext(src, { window: win });

  const app = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
  const used = [
    "changeRequestTemplate",
    "composerPlaceholder",
    "annotationsMarkdown",
    "canSubmitChangeRequest",
    "draftKey",
  ];
  for (const name of used) {
    assert.equal(typeof win[name], "function", `${name} must be on window`);
    assert.ok(app.includes(name), `app.js is expected to use ${name}`);
  }

  // And the script is actually loaded before app.js.
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(
    html.indexOf("wf-compose.js") < html.indexOf("app.js"),
    "wf-compose.js must be loaded before app.js"
  );
});
