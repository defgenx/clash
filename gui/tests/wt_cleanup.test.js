// node --test gui/tests/ — "Clean up worktrees…" and the done-item toast.
//
// What is safe is the backend's call; what is pinned here is that the dialog
// never offers to tick a kept worktree, lists what can go first, and says
// what a run actually did.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { wtBytes, wtRel, wtCleanupDialog, wtRunReport, wtDoneToast } = require("../dist/wt-cleanup.js");

const root = "/w";
const cand = (path, removable, extra = {}) => ({
  repo: "/w/repo",
  path,
  branch: "feat/x",
  removable,
  reason: removable ? "clean and pushed" : "3 commits on no remote",
  bytes: 300 << 20,
  ...extra,
});

test("sizes and paths read the way a person checks them", () => {
  assert.equal(wtBytes(null), "");
  assert.equal(wtBytes(340 << 20), "340 MB");
  assert.equal(wtBytes(3 * (1 << 30)), "3.0 GB");
  assert.equal(wtRel("/w/repo-worktrees/a", "/w/"), "repo-worktrees/a");
  assert.equal(wtRel("/elsewhere/a", "/w"), "/elsewhere/a");
});

test("kept worktrees are listed with their reason and can never be ticked", () => {
  const d = wtCleanupDialog({
    root,
    candidates: [cand("/w/z-worktrees/kept", false), cand("/w/a-worktrees/go", true, { branch: null })],
  });
  assert.equal(d.empty, null);
  assert.deepEqual(
    d.items.map((i) => [i.label, i.checked, i.disabled]),
    [
      ["a-worktrees/go", true, false],
      ["z-worktrees/kept", false, true],
    ],
    "what can go comes first, pre-ticked; the rest is disabled"
  );
  assert.match(d.items[0].detail, /^detached · 300 MB · clean and pushed$/);
  assert.match(d.items[1].detail, /no remote/);
  assert.match(d.detail, /1 can go \(300 MB\)/);
  assert.match(d.detail, /1 kept/);
  assert.match(d.detail, /Branches always stay/);
});

test("nothing removable is a message, not an empty dialog", () => {
  assert.match(wtCleanupDialog({ root, candidates: [cand("/w/a", false)] }).empty, /every one of its 1 worktree is kept/);
  assert.match(wtCleanupDialog({ root, candidates: [] }).empty, /No git worktrees in \/w/);
});

test("a run says what went, what changed meanwhile and what failed", () => {
  const preview = { root, candidates: [cand("/w/a", true), cand("/w/b", true), cand("/w/c", true)] };
  const r = wtRunReport({ removed: ["/w/a"], kept: [["/w/b", "1 uncommitted change"]], failed: [["/w/c", "busy"]] }, preview);
  assert.equal(r.toast, "Removed 1 worktree (300 MB) · 1 kept — changed since the preview · 1 failed");
  assert.match(r.failures, /^Some worktrees could not be removed:\n\nc: busy$/);
  assert.equal(wtRunReport({ removed: ["/w/a", "/w/b"], kept: [], failed: [] }, preview).failures, null);
});

test("the done toast names the item and every worktree it kept", () => {
  assert.equal(
    wtDoneToast({
      item: "p/s",
      title: "Auth",
      outcome: { removed: ["/w/a"], kept: [["/w/b", "3 commits on no remote"]], failed: [] },
    }),
    "“Auth” is done — removed 1 worktree · kept 1: 3 commits on no remote"
  );
  assert.equal(wtDoneToast({ item: "p/s", outcome: { removed: [], kept: [], failed: [] } }), null);
  assert.match(wtDoneToast({ item: "p/s", outcome: { removed: ["/w/a"] } }), /^“p\/s” is done/);
});
