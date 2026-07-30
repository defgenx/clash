// node --test gui/tests/ — zero-dependency tests for the workflow diff
// parser. Line numbering here feeds annotation anchors (the composer reads
// data-line off the rendered DOM), so a mis-parse corrupts review data, not
// just pixels.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  parseUnifiedDiff,
  diffFilePath,
  diffFileChangedLines,
} = require("../dist/wf-diff.js");

const SIMPLE = [
  "diff --git a/src/lib.rs b/src/lib.rs",
  "index 111..222 100644",
  "--- a/src/lib.rs",
  "+++ b/src/lib.rs",
  "@@ -10,4 +10,5 @@ fn ctx()",
  " keep one",
  "-old line",
  "+new line",
  "+added line",
  " keep two",
  "",
].join("\n");

test("numbers lines through mixed hunks", () => {
  const files = parseUnifiedDiff(SIMPLE);
  assert.equal(files.length, 1);
  assert.equal(diffFilePath(files[0]), "src/lib.rs");
  const lines = files[0].hunks[0].lines;
  assert.deepEqual(
    lines.map((l) => [l.kind, l.oldNo, l.newNo]),
    [
      ["ctx", 10, 10],
      ["del", 11, null],
      ["add", null, 11],
      ["add", null, 12],
      ["ctx", 12, 13],
    ]
  );
  assert.equal(lines[2].text, "new line");
  assert.equal(diffFileChangedLines(files[0]), 3);
});

test("multiple hunks and files keep independent counters", () => {
  const text =
    SIMPLE +
    [
      "diff --git a/b.rs b/b.rs",
      "--- a/b.rs",
      "+++ b/b.rs",
      "@@ -1 +1 @@",
      "-x",
      "+y",
      "@@ -7,2 +7,2 @@",
      " z",
      "-w",
      "+v",
      "",
    ].join("\n");
  const files = parseUnifiedDiff(text);
  assert.equal(files.length, 2);
  assert.equal(files[1].hunks.length, 2);
  assert.equal(files[1].hunks[1].lines[0].oldNo, 7);
  assert.equal(files[1].hunks[1].lines[1].oldNo, 8);
  assert.equal(files[1].hunks[1].lines[2].newNo, 8);
});

test("hunk headers without counts parse (single-line hunks)", () => {
  const files = parseUnifiedDiff("--- a/x\n+++ b/x\n@@ -3 +9 @@\n-a\n+b\n");
  const lines = files[0].hunks[0].lines;
  assert.equal(lines[0].oldNo, 3);
  assert.equal(lines[1].newNo, 9);
});

test("added and deleted files use /dev/null sides", () => {
  const text = [
    "diff --git a/new.rs b/new.rs",
    "--- /dev/null",
    "+++ b/new.rs",
    "@@ -0,0 +1,2 @@",
    "+one",
    "+two",
    "diff --git a/gone.rs b/gone.rs",
    "--- a/gone.rs",
    "+++ /dev/null",
    "@@ -1,2 +0,0 @@",
    "-one",
    "-two",
    "",
  ].join("\n");
  const files = parseUnifiedDiff(text);
  assert.equal(files[0].oldPath, "");
  assert.equal(diffFilePath(files[0]), "new.rs");
  assert.equal(files[0].hunks[0].lines[1].newNo, 2);
  assert.equal(files[1].newPath, "");
  assert.equal(diffFilePath(files[1]), "gone.rs");
});

test("rename headers are tracked", () => {
  const text = [
    "diff --git a/old.rs b/new.rs",
    "similarity index 90%",
    "rename from old.rs",
    "rename to new.rs",
    "--- a/old.rs",
    "+++ b/new.rs",
    "@@ -1 +1 @@",
    "-a",
    "+b",
    "",
  ].join("\n");
  const files = parseUnifiedDiff(text);
  assert.equal(files[0].renamedFrom, "old.rs");
  assert.equal(diffFilePath(files[0]), "new.rs");
});

test("metadata and binary notices never become content lines", () => {
  const text = [
    "diff --git a/img.png b/img.png",
    "Binary files a/img.png and b/img.png differ",
    "",
  ].join("\n");
  const files = parseUnifiedDiff(text);
  assert.equal(files.length, 1);
  assert.equal(files[0].hunks.length, 0);
});

test("a context line containing + or - is not add/del", () => {
  const files = parseUnifiedDiff("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a + b - c\n+real add\n");
  const lines = files[0].hunks[0].lines;
  assert.equal(lines[0].kind, "ctx");
  assert.equal(lines[1].kind, "add");
});

test("empty and garbage input yield no files", () => {
  assert.deepEqual(parseUnifiedDiff(""), []);
  assert.deepEqual(parseUnifiedDiff("hello\nworld"), []);
  assert.deepEqual(parseUnifiedDiff(null), []);
});
