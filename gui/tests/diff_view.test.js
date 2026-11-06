// node --test gui/tests/ — the diff viewer's pure half (gui/dist/diff-view.js).
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const D = require("../dist/diff-view.js");
const { parseUnifiedDiff } = require("../dist/wf-diff.js");

const DIFF = `diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,3 +1,3 @@
 fn main() {
-    let x = compute(1, 2);
+    let x = compute(1, 3);
 }
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
diff --git a/old.txt b/old.txt
deleted file mode 100644
--- a/old.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/logo.png b/logo.png
new file mode 100644
Binary files /dev/null and b/logo.png differ
diff --git a/x.rs b/y.rs
similarity index 90%
rename from x.rs
rename to y.rs
`;

test("files get git's status, including the content-less ones", () => {
  const files = parseUnifiedDiff(DIFF);
  assert.deepEqual(files.map(D.fileStatus), ["modified", "added", "deleted", "added", "renamed"]);
  assert.equal(files[3].binary, true);
  assert.deepEqual(D.fileStats(files[0]), { adds: 1, dels: 1 });
  assert.equal(D.STATUS_LETTERS.renamed, "R");
});

test("the change bar always shows both kinds when both exist", () => {
  assert.deepEqual(D.statBlocks(0, 0), ["none", "none", "none", "none", "none"]);
  assert.deepEqual(D.statBlocks(10, 0), ["add", "add", "add", "add", "add"]);
  assert.deepEqual(D.statBlocks(100, 1), ["add", "add", "add", "add", "del"]);
  assert.deepEqual(D.statBlocks(1, 100), ["add", "del", "del", "del", "del"]);
  assert.equal(D.statBlocks(3, 3).length, 5);
});

test("split rows pair deletions with the additions that follow them", () => {
  const L = (kind) => ({ kind, text: kind });
  const lines = [L("ctx"), L("del"), L("del"), L("add"), L("ctx"), L("add")];
  assert.deepEqual(D.splitRows(lines), [
    { left: 0, right: 0 },
    { left: 1, right: 3 },
    { left: 2, right: null },
    { left: 4, right: 4 },
    { left: null, right: 5 },
  ]);
});

test("word marks cover only what changed, and give up on rewrites", () => {
  const [a, b] = D.intralineRanges("let x = compute(1, 2);", "let x = compute(1, 3);");
  assert.deepEqual(a, [[19, 20]]);
  assert.deepEqual(b, [[19, 20]]);
  assert.equal(D.intralineRanges("same", "same"), null);
  assert.equal(D.intralineRanges("alpha beta gamma", "totally different words here"), null);
  const marks = D.hunkWordMarks(parseUnifiedDiff(DIFF)[0].hunks[0].lines);
  assert.deepEqual([...marks.keys()], [1, 2]);
  assert.equal(
    D.markedHtml("a <b> c", [[2, 5]]),
    'a <mark class="dv-word">&lt;b&gt;</mark> c'
  );
  assert.equal(D.markedHtml("<x>", null), "&lt;x&gt;");
});

test("the navigator is a folder tree with single-child chains folded", () => {
  const rows = D.navRows(["src/app/ui/a.js", "src/app/ui/b.js", "README.md", "src/lib.rs"]);
  assert.deepEqual(
    rows.map((r) => [r.kind, r.name, r.depth, r.index]),
    [
      ["dir", "src", 0, undefined],
      ["dir", "app/ui", 1, undefined],
      ["file", "a.js", 2, 0],
      ["file", "b.js", 2, 1],
      ["file", "lib.rs", 1, 3],
      ["file", "README.md", 0, 2],
    ]
  );
  assert.equal(rows[1].path, "src/app/ui");
  assert.ok(D.matchesQuery("src/App/ui.js", "app UI"));
  assert.ok(!D.matchesQuery("src/app.js", "lib"));
});

test("compare choices: local changes first, the default branch leads, the current branch is left out", () => {
  const groups = D.compareChoices({
    refs: { current: "feat", local: ["feat", "dev", "main"], remote: ["origin/dev", "origin/main"], tags: ["v1"] },
    defaultBranch: "main",
    commits: [{ sha: "a".repeat(40), short: "aaaaaaa", subject: "fix", author: "Ada" }],
  });
  assert.deepEqual(groups.map((g) => g.label), [
    "Local changes",
    "Since branched from",
    "Compare with branch",
    "Compare with tag",
    "Show a commit",
  ]);
  assert.deepEqual(groups[0].items.map((i) => i.target), ["head", "staged", "unstaged"]);
  assert.deepEqual(groups[1].items.map((i) => i.rev), ["main", "origin/main", "dev", "origin/dev"]);
  assert.equal(groups[4].items[0].target, "commit");
  assert.deepEqual(D.compareChoices({}).map((g) => g.label), ["Local changes"]);
  assert.equal(D.targetLabel("merge-base", "main"), "Since branched from main");
  assert.equal(D.targetLabel("commit", "abcdef1234"), "Commit abcdef12");
  assert.equal(D.targetLabel("head"), "Uncommitted changes");
});

test("blame rows name a commit where its run starts and bucket ages", () => {
  const a = "a".repeat(40);
  const b = "b".repeat(40);
  const z = "0".repeat(40);
  const now = 1_800_000_000_000;
  const rows = D.blameRows(
    {
      lines: [a, a, b, z],
      commits: { [a]: { author: "Ada", time: 1_000_000_000, summary: "old" }, [b]: { author: "Bo", time: 1_700_000_000, summary: "new" }, [z]: { time: 1_800_000_000 } },
    },
    now
  );
  assert.deepEqual(rows.map((r) => r.first), [true, false, true, true]);
  assert.equal(rows[0].age, 4);
  assert.equal(rows[2].age, 0);
  assert.equal(rows[3].uncommitted, true);
  assert.equal(rows[3].author, "You");
  assert.equal(rows[0].short, "aaaaaaa");
  assert.deepEqual(D.blameRows(null), []);
});

test("relative times and change stepping", () => {
  const now = 1_000_000 * 1000;
  assert.equal(D.relTime(1_000_000 - 30, now), "just now");
  assert.equal(D.relTime(1_000_000 - 7200, now), "2 h ago");
  assert.equal(D.relTime(1_000_000 - 86400 * 3, now), "3 d ago");
  assert.equal(D.stepChange(0, -1, 1), -1);
  assert.equal(D.stepChange(3, -1, 1), 0);
  assert.equal(D.stepChange(3, -1, -1), 2);
  assert.equal(D.stepChange(3, 2, 1), 0);
});

test("saved preferences are validated field by field", () => {
  assert.deepEqual(D.diffPrefsFrom(null), { ...D.DIFF_PREF_DEFAULTS });
  assert.deepEqual(
    D.diffPrefsFrom({ mode: "split", wrap: true, context: 25, untracked: "yes", nav: false }),
    { mode: "split", wrap: true, ignoreWhitespace: false, untracked: true, context: 25, nav: false }
  );
  assert.equal(D.diffPrefsFrom({ context: 7, mode: "weird" }).context, null);
  assert.equal(D.diffPrefsFrom({ mode: "weird" }).mode, "unified");
  assert.ok(D.hasWorkTree("head") && D.hasWorkTree("merge-base"));
  assert.ok(!D.hasWorkTree("staged") && !D.hasWorkTree("commit"));
});

test("the browser branch publishes DiffView", () => {
  const sandbox = {};
  sandbox.globalThis = sandbox;
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "..", "dist", "diff-view.js"), "utf8"), sandbox);
  assert.equal(typeof sandbox.DiffView.splitRows, "function");
  const html = fs.readFileSync(path.join(__dirname, "..", "dist", "index.html"), "utf8");
  assert.ok(html.indexOf('src="diff-view.js"') < html.indexOf('src="app.js"'), "loaded before app.js");
});
