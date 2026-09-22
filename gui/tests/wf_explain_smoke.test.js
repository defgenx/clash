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

function sandboxFor(docs) {
  const calls = [];
  const sandbox = {
    invoke: async (cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "get_workflow_doc") return docs[args.doc] ?? "";
      throw new Error(`unexpected command ${cmd}`);
    },
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
