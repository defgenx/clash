// node --test gui/tests/ — smoke-render the document-pair reader against a
// stub DOM, once per pair.
//
// The three pairs (plan explained / changes explained / plan vs changes) share
// one renderer and differ only by a row in `WF_DOC_PAIRS`. That is the point of
// the table, and it is also the failure mode: a pair whose row is missing a
// field renders a tab with a blank tooltip, an empty state naming the wrong
// action, or a fetch of `undefined` — all at click time, in a webview with no
// console, long after a healthy `frontend booted:` line.
//
// Reachability is a different question and lives in app_source.test.js: this
// file would happily pass on a renderer nothing ever calls.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");

/// The pair table, lifted out of app.js and evaluated on its own — the
/// renderer reads it as a global.
function docPairs() {
  const start = APP.indexOf("const WF_DOC_PAIRS = {");
  assert.ok(start >= 0, "WF_DOC_PAIRS must exist in app.js");
  const end = APP.indexOf("\n};", start) + 3;
  const sandbox = {};
  vm.runInNewContext(`${APP.slice(start, end)}\nthis.out = WF_DOC_PAIRS;`, sandbox);
  return sandbox.out;
}

function sandboxFor(docs, tree = []) {
  const calls = [];
  const sandbox = {
    invoke: async (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "get_workflow_doc") return docs[args.doc] ?? "";
      if (cmd === "get_workflow_explain_tree") return tree;
      if (cmd === "get_workflow_explain_node")
        return docs[`${args.target}/${args.node}.${args.form}`] ?? "";
      throw new Error(`unexpected command ${cmd}`);
    },
    setInterval: () => 0,
    clearInterval: () => {},
    ...require("../dist/wf-explain-tree.js"),
    document: { createElement: (t) => stubEl(t), createTextNode: () => stubEl("text") },
    console,
    Object,
    String,
    svgIcon: () => "<svg/>",
    escapeHtml: (v) => String(v),
    renderMarkdown: (el, text) => {
      el.rendered = text;
    },
    renderMermaidIn: () => {},
    openScratchInEditor: () => {},
    buildWorkflowView: () => {},
    wfExplainFrameDoc: (t) => `<html><body>${t}</body></html>`,
  };
  vm.createContext(sandbox);
  const start = APP.indexOf("const WF_DOC_PAIRS = {");
  vm.runInContext(APP.slice(start, APP.indexOf("\n};", start) + 3), sandbox);
  vm.runInContext(extractFunction(APP, "wfExplainFormsFor"), sandbox);
  vm.runInContext(extractFunction(APP, "renderWfExplainView"), sandbox);
  for (const fn of ["renderWfExplainNav", "wfWatchExplainFrame", "wfMarkExplainHeadings", "wfExplainGo"])
    vm.runInContext(extractFunction(APP, fn), sandbox);
  sandbox.__calls = calls;
  return sandbox;
}

const ITEM = {
  project: "p",
  slug: "s",
  path: "/items/s",
  meta: { iteration: 3 },
  planExplain: { md: true, html: true },
  diffExplain: { md: true, html: true },
  driftReport: { md: true, html: true },
};

const PAIRS = [
  ["plan", "explain-plan.md", "explain-plan.html"],
  ["diff", "explain-diff.md", "explain-diff.html"],
  ["drift", "drift.md", "drift.html"],
];

test("every pair's row carries all five facts", () => {
  const pairs = docPairs();
  assert.deepEqual(Object.keys(pairs).sort(), ["diff", "drift", "plan"]);
  for (const [which] of PAIRS) {
    const row = pairs[which];
    for (const field of ["action", "md", "html", "caption"]) {
      assert.ok(row[field], `${which} is missing ${field}`);
    }
    // `legacy` is deliberately null for a pair that never had another name,
    // so it is checked for presence of the key, not for a value.
    assert.ok("legacy" in row, `${which} must declare whether it has a legacy name`);
    assert.match(row.md, /\.md$/);
    assert.match(row.html, /\.html$/);
  }
  // No two pairs share a file, or one tab would overwrite another's document.
  const files = PAIRS.flatMap(([w]) => [pairs[w].md, pairs[w].html]);
  assert.equal(new Set(files).size, files.length, "two pairs share a file");
});

for (const [which, md, html] of PAIRS) {
  test(`the ${which} pair renders its drawing, and the toggle offers the prose`, async () => {
    const sandbox = sandboxFor({ [md]: "# doc", [html]: "<h1>drawn</h1>" });
    const body = stubEl();
    await sandbox.renderWfExplainView(body, stubEl(), ITEM, {}, which);
    const all = descendants(body);
    // Both forms exist, so the picture is the headline and the frame holds it.
    const frame = all.find((e) => (e.className || "").includes("wf-explain-frame"));
    assert.ok(frame, "the graphical form must render in a frame");
    assert.match(frame.srcdoc, /drawn/);
    // Inert: generated markup must not share a DOM with the Tauri bridge, and
    // must not be able to run at all. Omitting `allow-scripts` is what makes
    // inline handlers and `<img onerror>` dead, while same-origin still lets
    // the parent measure the content.
    assert.equal(frame.attrs.sandbox, "allow-same-origin");
    assert.ok(!/allow-scripts/.test(frame.attrs.sandbox));
    assert.ok(!all.some((e) => e.innerHTML === "<h1>drawn</h1>"), "never injected inline");
    // The fetch asked for this pair's own file, not another's.
    assert.ok(
      sandbox.__calls.some(([c, a]) => c === "get_workflow_doc" && a.doc === html),
      `expected a fetch of ${html}, got ${JSON.stringify(sandbox.__calls)}`
    );
    // The toggle is present with both forms, and the Edit button names the
    // file actually on screen.
    const seg = all.find((e) => (e.className || "").includes("wf-seg"));
    assert.ok(seg, "two forms means a toggle");
    assert.ok(
      all.some((e) => (e.innerHTML || "").includes(`Edit ${html}`)),
      `the edit button must name ${html}`
    );
  });

  test(`the ${which} pair falls back to the prose when there is no drawing`, async () => {
    const item = { ...ITEM };
    const forms = { md: true, html: false };
    if (which === "plan") item.planExplain = forms;
    if (which === "diff") item.diffExplain = forms;
    if (which === "drift") item.driftReport = forms;
    const sandbox = sandboxFor({ [md]: "# the write-up" });
    const body = stubEl();
    await sandbox.renderWfExplainView(body, stubEl(), item, {}, which);
    const all = descendants(body);
    assert.ok(!all.some((e) => (e.className || "").includes("wf-explain-frame")));
    assert.ok(all.some((e) => e.rendered === "# the write-up"), "the prose must render");
    // One form means no toggle to pick between.
    assert.ok(!all.some((e) => (e.className || "").includes("wf-seg")));
  });

  test(`an unwritten ${which} pair names the action that would write it`, async () => {
    const sandbox = sandboxFor({});
    const body = stubEl();
    await sandbox.renderWfExplainView(body, stubEl(), ITEM, { explainView: "text" }, which);
    // The empty state used to hardcode "◫ Explain plan/changes", so the third
    // pair would have told the reader to run the wrong action.
    const said = String(body.__html || "");
    assert.match(said, /nothing written yet/);
    assert.ok(
      said.includes(docPairs()[which].action),
      `empty state must name ${docPairs()[which].action}, said: ${said}`
    );
  });
}

test("a pair with no legacy name never fetches `undefined` as a document", async () => {
  // The two explanations fall back to their pre-rename filename when the
  // current one is empty. The drift report has no such name, and asking the
  // backend for a null document is a rejected command, not an empty string.
  const sandbox = sandboxFor({});
  await sandbox.renderWfExplainView(
    stubEl(),
    stubEl(),
    ITEM,
    { explainView: "text" },
    "drift"
  );
  const docs = sandbox.__calls.filter(([c]) => c === "get_workflow_doc").map(([, a]) => a.doc);
  assert.deepEqual(docs, ["drift.md"], `unexpected fetches: ${JSON.stringify(docs)}`);
  // …while the diff pair does try its legacy name, which is the behaviour
  // this must not have broken.
  const legacy = sandboxFor({ "structure.md": "# old" });
  await legacy.renderWfExplainView(
    stubEl(),
    stubEl(),
    ITEM,
    { explainView: "text" },
    "diff"
  );
  assert.ok(
    legacy.__calls.some(([c, a]) => c === "get_workflow_doc" && a.doc === "structure.md"),
    "the diff pair must still read its pre-rename filename"
  );
});

// ── Drill-down ──────────────────────────────────────────────────────────

const TREE = [
  {
    node: "",
    title: "Overview",
    stale: false,
    md: true,
    html: true,
    nodes: [
      { id: "1", title: "Parser", critical: true, why: "every read", expanded: true, diveable: true, stale: false },
      { id: "2", title: "Cache", critical: true, why: "invalidation", expanded: false, diveable: true, stale: false },
      { id: "3", title: "Docs", critical: false, expanded: false, diveable: false, stale: false },
    ],
    edges: [],
  },
  { node: "1", title: "Parser", stale: false, md: true, html: true, nodes: [], edges: [] },
];

test("an explanation with a tree shows its parts, each a way down", async () => {
  const sandbox = sandboxFor({ "explain-diff.html": "<h1>map</h1>" }, TREE);
  const body = stubEl();
  await sandbox.renderWfExplainView(body, stubEl(), ITEM, {}, "diff");
  const all = descendants(body);
  assert.ok(all.some((e) => (e.className || "").includes("wf-explain-nav")), "the strip renders");
  const chips = all.filter((e) => /(^| )wf-explain-part( |$)/.test(e.className || ""));
  assert.equal(chips.length, 3);
  // Open, dive, and a leaf that offers nothing.
  assert.match(chips[0].className, /\bopen\b/);
  assert.match(chips[1].className, /\bdive\b/);
  assert.ok(chips[2].disabled, "a leaf is not clickable");
  // Only the critical part not yet written is offered as a batch.
  const batch = all.find((e) => (e.className || "").includes("wf-explain-critical"));
  assert.ok(batch && /\(1\)/.test(batch.textContent), "one critical part to dive");
  // The page stays inert at every level.
  const frame = all.find((e) => (e.className || "").includes("wf-explain-frame"));
  assert.equal(frame.attrs.sandbox, "allow-same-origin");
});

test("a part's level is read from its own files, never the overview's", async () => {
  const sandbox = sandboxFor({ "explain-diff/1.html": "<h1>inside the parser</h1>" }, TREE);
  const body = stubEl();
  await sandbox.renderWfExplainView(body, stubEl(), ITEM, { explainNodes: { diff: "1" } }, "diff");
  const nodeReads = sandbox.__calls.filter(([c]) => c === "get_workflow_explain_node");
  assert.deepEqual(
    nodeReads.map(([, a]) => [a.target, a.node, a.form]),
    [["explain-diff", "1", "html"]]
  );
  assert.ok(!sandbox.__calls.some(([c]) => c === "get_workflow_doc"), "the overview is not fetched");
  const all = descendants(body);
  const frame = all.find((e) => (e.className || "").includes("wf-explain-frame"));
  assert.match(frame.srcdoc, /inside the parser/);
  // The edit button names the part's file.
  assert.ok(all.some((e) => (e.innerHTML || "").includes("Edit explain-diff/1.html")));
});

test("a level that vanished falls back to the overview", async () => {
  const sandbox = sandboxFor({ "explain-diff.md": "# overview" }, TREE);
  const ts = { explainNodes: { diff: "9" }, explainView: "text" };
  await sandbox.renderWfExplainView(stubEl(), stubEl(), ITEM, ts, "diff");
  assert.equal(ts.explainNodes.diff, "");
  assert.ok(sandbox.__calls.some(([c, a]) => c === "get_workflow_doc" && a.doc === "explain-diff.md"));
});
