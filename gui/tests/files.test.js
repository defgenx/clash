// node --test gui/tests/ — the Files panel's pure half.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const {
  fileTreeRows,
  statusIndex,
  markFor,
  markGlyph,
  hljsLanguage,
  mentionText,
  joinPath,
  highlightMatch,
  formatSize,
  foldersToRefresh,
} = require("../dist/files.js");

const e = (rel, isDir = false) => ({ rel, name: rel.split("/").pop(), isDir });

test("tree rows descend only into expanded, loaded folders", () => {
  const children = {
    "": [e("src", true), e("docs", true), e("README.md")],
    src: [e("src/app", true), e("src/main.rs")],
    "src/app": [e("src/app/mod.rs")],
    docs: [e("docs/a.md")],
  };
  const rows = fileTreeRows(children, new Set(["src", "src/app"]));
  assert.deepEqual(
    rows.map((r) => [r.rel, r.depth]),
    [
      ["src", 0],
      ["src/app", 1],
      ["src/app/mod.rs", 2],
      ["src/main.rs", 1],
      ["docs", 0],
      ["README.md", 0],
    ]
  );
  // Expanded but not yet listed: the folder shows, nothing under it.
  assert.equal(fileTreeRows({ "": [e("x", true)] }, new Set(["x"])).length, 1);
});

test("git marks: own status, untracked ancestors, dirty folders", () => {
  const idx = statusIndex({
    entries: [
      { rel: "src/app.rs", mark: "modified" },
      { rel: "new/", mark: "untracked" },
      { rel: "a/b/c.rs", mark: "added" },
    ],
  });
  assert.equal(markFor(idx, "src/app.rs", false), "modified");
  assert.equal(markFor(idx, "src", true), "dirty");
  assert.equal(markFor(idx, "new", true), "untracked");
  assert.equal(markFor(idx, "new/deep/x.txt", false), "untracked");
  assert.equal(markFor(idx, "a", true), "dirty");
  assert.equal(markFor(idx, "a/b", true), "dirty");
  assert.equal(markFor(idx, "src/other.rs", false), null);
  // A file is never "dirty" — only folders summarize their contents.
  assert.equal(markFor(idx, "newer", false), null);
  assert.equal(markFor(null, "x", false), null);
});

test("every mark the backend emits has a glyph", () => {
  for (const m of ["modified", "added", "deleted", "renamed", "untracked", "conflicted", "dirty"]) {
    assert.ok(markGlyph(m)?.letter, m);
  }
  assert.equal(markGlyph("nope"), null);
});

test("highlighter languages exist in the vendored bundle", () => {
  const ctx = {};
  vm.createContext(ctx);
  vm.runInContext(
    fs.readFileSync(path.join(__dirname, "../dist/vendor/highlight.min.js"), "utf8"),
    ctx
  );
  const shipped = new Set(ctx.hljs.listLanguages());
  const src = fs.readFileSync(path.join(__dirname, "../dist/files.js"), "utf8");
  const langs = [...src.matchAll(/:\s*"([a-z]+)"/g)].map((m) => m[1]);
  for (const l of langs) assert.ok(shipped.has(l), `${l} is not in highlight.min.js`);

  assert.equal(hljsLanguage("src/main.rs"), "rust");
  assert.equal(hljsLanguage("gui/dist/APP.JS"), "javascript");
  assert.equal(hljsLanguage("Makefile"), "makefile");
  assert.equal(hljsLanguage("config.toml"), "ini");
  assert.equal(hljsLanguage("LICENSE"), null);
  assert.equal(hljsLanguage(".gitignore"), null);
});

test("mentions are relative to the session and quoted when needed", () => {
  assert.equal(mentionText("/r/clash", "/r/clash/src/a.rs"), "@src/a.rs ");
  assert.equal(mentionText("/r/clash/", "/r/clash/src/a.rs"), "@src/a.rs ");
  assert.equal(mentionText("/r/other", "/r/clash/a.rs"), "@/r/clash/a.rs ");
  assert.equal(mentionText("/r", "/r/my file.md"), '@"my file.md" ');
  // A sibling folder sharing the prefix is not "under" the cwd.
  assert.equal(mentionText("/r/clash", "/r/clash-old/a"), "@/r/clash-old/a ");
});

test("path helpers", () => {
  assert.equal(joinPath("/r/", "a/b"), "/r/a/b");
  assert.equal(joinPath("/r", ""), "/r");
  assert.equal(highlightMatch("a<b", [0, 1]), "<b>a</b><b>&lt;</b>b");
  assert.equal(formatSize(10), "10 B");
  assert.equal(formatSize(2048), "2.0 KB");
  assert.equal(formatSize(3 * 1024 * 1024), "3.0 MB");
});

test("refresh lists the root and every visible expanded folder", () => {
  assert.deepEqual(foldersToRefresh(new Set()), [""]);
  assert.deepEqual(
    foldersToRefresh(new Set(["src/app", "src", "docs/x"])),
    ["", "src", "src/app"]
  );
});

test("the browser branch publishes every helper app.js reads", () => {
  const ctx = { window: {} };
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(path.join(__dirname, "../dist/files.js"), "utf8"), ctx);
  const app = fs.readFileSync(path.join(__dirname, "../dist/app.js"), "utf8");
  for (const name of Object.keys(require("../dist/files.js"))) {
    assert.equal(typeof ctx.window[name], "function", name);
    assert.ok(app.includes(`${name}(`), `app.js never calls ${name}`);
  }
});
