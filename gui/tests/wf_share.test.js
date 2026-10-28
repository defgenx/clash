// node --test gui/tests/ — the share dialog's pure half.
//
// The share system's core promise is "the preview IS the payload"; what the
// model gets wrong, the dialog shows wrong. Pin the presets, the availability
// rules, and the HTML shell's self-containment.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const {
  SHARE_SECTIONS,
  SHARE_PRESETS,
  presetSections,
  shareModel,
  shareDestination,
  sectionsFromChecks,
  shareHtmlDocument,
  detectTicketKey,
  SHARE_EXPLAIN_PAIRS,
  explainLevelsToShare,
  shareAnchor,
  shareExplanationEntries,
  explainOutline,
} = require("../dist/wf-share.js");
const { extractFunction } = require("./extract-fn.js");
const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");

// A tree as `get_workflow_explain_tree` returns it: overview, part 1 opened,
// its sub-part 1.1 opened, part 2 never opened.
const TREE = [
  {
    node: "",
    nodes: [
      { id: "1", title: "Retry", summary: "wraps calls", critical: true },
      { id: "2", title: "Docs" },
    ],
  },
  { node: "1", stale: true, nodes: [{ id: "1.1", title: "Backoff" }] },
  { node: "1.1", nodes: [] },
];
const DIFF = SHARE_EXPLAIN_PAIRS.find((p) => p.which === "diff");

test("the explanation depth cuts the tree, and a part goes only with its parent", () => {
  const ids = (d) => explainLevelsToShare(TREE, d, true).map((l) => l.node);
  assert.deepEqual(ids("all"), ["", "1", "1.1"]);
  assert.deepEqual(ids("parts"), ["", "1"]);
  assert.deepEqual(ids("overview"), [""]);
  assert.deepEqual(ids("none"), []);
  // An orphaned level (its parent missing) stays out.
  assert.deepEqual(explainLevelsToShare([TREE[0], TREE[2]], "all", true).map((l) => l.node), [""]);
  // Written before drill-down: no index, so the overview alone.
  assert.deepEqual(explainLevelsToShare([], "all", true).map((l) => l.node), [""]);
  assert.deepEqual(explainLevelsToShare([], "all", false), []);
});

test("entries carry breadcrumbs and link only the parts the page holds", () => {
  const levels = explainLevelsToShare(TREE, "parts", true);
  const [root, one] = shareExplanationEntries(DIFF, levels, { "": { bodyHtml: "<p>w</p>", frameHtml: "<div>d</div>" } });
  assert.equal(root.anchor, "x-explain-diff");
  assert.equal(root.title, "Changes explained");
  assert.equal(root.parts[0].anchor, "x-explain-diff-1");
  assert.equal(root.parts[1].anchor, null, "never opened");
  assert.equal(one.title, "1 Retry");
  assert.equal(one.stale, true);
  assert.equal(one.parts[0].anchor, null, "1.1 was cut at this depth");
  assert.deepEqual(one.crumbs.map((c) => c.anchor), ["x-explain-diff", "x-explain-diff-1"]);
  assert.equal(shareAnchor("drift", "3.2.1"), "x-drift-3-2-1");
  assert.deepEqual(explainOutline([{ label: "Changes explained", entries: [root, one] }]), [
    "Changes explained — overview + 1 part",
  ]);
});

test("the saved page sandboxes the drawings and stays script-free", () => {
  const levels = explainLevelsToShare(TREE, "all", true);
  const drawn = '<div onclick="x()">drawn "page"</div><script>alert(1)</script>';
  const entries = shareExplanationEntries(DIFF, levels, {
    "": { bodyHtml: "<p>write-up</p>", frameHtml: drawn },
    1: { bodyHtml: "<p>inside</p>", frameHtml: "" },
  });
  const html = shareHtmlDocument("T", "<p>doc</p>", [{ label: "Changes explained", entries }]);
  // The agent's page is an escaped srcdoc of a script-less sandbox — never
  // markup of the page itself.
  assert.match(html, /<iframe sandbox="allow-same-origin" title="Changes explained" srcdoc="[^"]*&lt;script&gt;alert\(1\)/);
  assert.ok(!html.includes(drawn));
  assert.ok(!/allow-scripts/.test(html));
  assert.ok(!/<script/.test(html), "no script of the page's own");
  assert.ok(!/\b(src|href)\s*=\s*["'](https?:)?\/\//.test(html), "external reference found");
  // Navigation is anchors: a contents line, and each level's own section.
  assert.match(html, /<nav class="toc"><a href="#x-document">Document<\/a> · <a href="#x-explain-diff">/);
  for (const id of ["x-explain-diff", "x-explain-diff-1", "x-explain-diff-1-1"]) assert.ok(html.includes(`id="${id}"`), id);
  assert.match(html, /<a href="#x-explain-diff-1"><b>1 Retry<\/b> →<\/a>/);
  // With a drawing the write-up folds under it; without one it is the body.
  assert.match(html, /<details class="writeup"><summary>Write-up<\/summary><p>write-up<\/p>/);
  assert.ok(html.includes("<p>inside</p>") && !html.includes("<summary>Write-up</summary><p>inside"));
  // No explanations: the page is the old export, plus nothing.
  assert.ok(!shareHtmlDocument("T", "<p>doc</p>").includes('class="toc"'));
});

test("the page export reads explanations only through the document readers", () => {
  const fn = extractFunction(APP, "wfShareExplanations");
  const cmds = [...fn.matchAll(/invoke\("([a-z_]+)"/g)].map((m) => m[1]);
  assert.deepEqual([...new Set(cmds)].sort(), ["get_workflow_doc", "get_workflow_explain_node", "get_workflow_explain_tree"]);
  assert.match(fn, /renderMarkdownForExport\(md\)/);
  // Saves go through the native save dialog seeded with the backend's name,
  // then a toast that reveals the file or copies its path.
  const dialog = extractFunction(APP, "wfShareDialog");
  assert.match(dialog, /invoke\("default_share_export_path"/);
  assert.match(dialog, /await pickSavePath\(\s*seed/);
  assert.doesNotMatch(dialog, /pickDirectory\(/);
  assert.match(dialog, /invoke\("explorer_reveal", \{ path: written \}\)/);
  assert.match(dialog, /copyText\(written, "path"\)/);
  assert.match(extractFunction(APP, "pickSavePath"), /plugin:dialog\|save/);
});

test("smoke: gathering explanations for the page, with the legacy fallback", async () => {
  const docs = {
    "structure.md": "# legacy write-up",
    "explain-diff.html": "<div>drawn</div>",
  };
  const sandbox = {
    ...require("../dist/wf-share.js"),
    invoke: async (cmd, args) => {
      if (cmd === "get_workflow_explain_tree") return args.target === "explain-diff" ? TREE : [];
      if (cmd === "get_workflow_doc") return docs[args.doc] || "";
      if (cmd === "get_workflow_explain_node") return `${args.node}.${args.form}`;
      throw new Error(`unexpected ${cmd}`);
    },
    renderMarkdownForExport: async (md) => `<rendered>${md}</rendered>`,
  };
  const pairs = APP.slice(APP.indexOf("const WF_DOC_PAIRS = {"), APP.indexOf("\n};", APP.indexOf("const WF_DOC_PAIRS = {")) + 3);
  vm.createContext(sandbox);
  vm.runInContext(
    `${pairs}\n${extractFunction(APP, "wfExplainFormsFor")}\n${extractFunction(APP, "wfShareExplanations")}\nthis.fn = wfShareExplanations;`,
    sandbox
  );
  const item = { project: "p", slug: "s", diffExplain: { md: true, html: true } };
  // Across the vm realm, compare plain data.
  const groups = JSON.parse(JSON.stringify(await sandbox.fn(item, "all", true)));
  assert.deepEqual(groups.map((g) => g.label), ["Changes explained"]);
  const [root, one] = groups[0].entries;
  assert.equal(root.bodyHtml, "<rendered># legacy write-up</rendered>");
  assert.equal(root.frameHtml, "<div>drawn</div>");
  assert.equal(one.frameHtml, "1.html");
  assert.equal((await sandbox.fn(item, "none", false)).length, 0);
});

test("an export renders its diagrams on an attached stage", () => {
  // renderMermaidIn skips a detached node, so the export used to ship its
  // mermaid fences as bare code.
  const fn = extractFunction(APP, "renderMarkdownForExport");
  assert.match(fn, /document\.body\.appendChild\(stage\)/);
  assert.match(fn, /stage\.remove\(\)/);
  assert.match(fn, /renderMermaidIn\(stage, \{ theme: "default" \}\)/);
});

test("presets are honest sizes: summary ⊂ packet ⊂ dossier", () => {
  const ids = (p) => SHARE_PRESETS.find((x) => x.id === p).sections;
  for (const s of ids("summary")) assert.ok(ids("packet").includes(s));
  for (const s of ids("packet")) assert.ok(ids("dossier").includes(s));
  // The dossier is everything; the diff is dossier-only (it is the heavy one).
  assert.deepEqual([...ids("dossier")].sort(), SHARE_SECTIONS.map((s) => s.id).sort());
  assert.ok(!ids("packet").includes("diff"));
  assert.ok(ids("dossier").includes("diff"));
});

test("presetSections returns one flag per section, unknown preset falls back to packet", () => {
  const packet = presetSections("packet");
  assert.deepEqual(Object.keys(packet).sort(), SHARE_SECTIONS.map((s) => s.id).sort());
  assert.equal(packet.summary, true);
  assert.equal(packet.diff, false);
  assert.deepEqual(presetSections("nonsense"), packet);
});

test("a planless item gets a disabled, unchecked plan row — not a missing one", () => {
  const m = shareModel({ hasPlan: false, preset: "packet" });
  const plan = m.sections.find((s) => s.id === "plan");
  assert.equal(plan.disabled, true);
  assert.equal(plan.checked, false);
  assert.match(plan.detail, /no plan phase/);
  // Everything else keeps the preset's selection.
  assert.equal(m.sections.find((s) => s.id === "summary").checked, true);
});

test("the transport is a choice, and a skill only refines the session route", () => {
  // The correction that produced this shape: a skill is not a route of its own,
  // and clash posting directly is not a fallback. One setting picks who posts;
  // the skill only says how the session does it.
  const session = shareDestination("jira", "Post to Jira…", { transport: "agent" });
  assert.equal(session.route, "agent");
  assert.equal(session.enabled, true, "a session route needs no configuration");
  assert.match(session.hint, /tools it has connected/);

  const withSkill = shareDestination("jira", "l", { transport: "agent", skill: "myorg:jira-post" });
  assert.equal(withSkill.route, "agent", "a skill refines the route, it is not one");
  assert.equal(withSkill.skill, "myorg:jira-post");
  assert.match(withSkill.hint, /via the myorg:jira-post skill/);

  // Credentials present or not, the session route is unaffected: nothing about
  // this choice is inferred from what happens to be filled in.
  assert.equal(
    shareDestination("jira", "l", { transport: "agent", clientConfigured: true }).route,
    "agent"
  );
});

test("clash's own route fails loudly rather than falling back", () => {
  // Choosing "clash itself" and leaving the credentials empty must not quietly
  // launch a session instead — that would be clash deciding which system talks
  // to your tracker, which is the decision the setting exists to make.
  const ready = shareDestination("slack", "Send to Slack", {
    transport: "clash",
    clientConfigured: true,
  });
  assert.equal(ready.route, "clash");
  assert.equal(ready.enabled, true);
  assert.equal(ready.hint, "clash posts it directly");

  const unconfigured = shareDestination("slack", "Send to Slack", { transport: "clash" });
  assert.equal(unconfigured.route, "clash", "still the chosen route, not a silent swap");
  assert.equal(unconfigured.enabled, false);
  assert.match(unconfigured.hint, /credentials are missing/);
  assert.match(unconfigured.hint, /switch this destination to an agent session/);
  // A skill is irrelevant on this route and must not leak into it.
  assert.equal(
    shareDestination("slack", "l", { transport: "clash", skill: "myorg:x", clientConfigured: true })
      .skill,
    ""
  );
});

test("the model renders the transports it is given, per family", () => {
  const m = shareModel({
    jiraTransport: "clash",
    jiraConfigured: true,
    chatTransport: "agent",
    chatSkill: "myorg:chat",
  });
  const by = (id) => m.destinations.find((d) => d.id === id);
  assert.equal(by("jira").route, "clash");
  // Slack and Discord share one family, so one setting covers both.
  assert.equal(by("slack").route, "agent");
  assert.equal(by("discord").route, "agent");
  assert.equal(by("slack").skill, "myorg:chat");
  assert.equal(by("discord").skill, "myorg:chat");
  // The default is the session route, so a fresh install can share at once.
  const fresh = shareModel({});
  for (const id of ["jira", "slack", "discord"]) {
    assert.equal(fresh.destinations.find((d) => d.id === id).route, "agent", id);
    assert.equal(fresh.destinations.find((d) => d.id === id).enabled, true, id);
  }
  // Local destinations take no route at all.
  for (const id of ["clipboard", "md", "html"]) {
    assert.equal(fresh.destinations.find((d) => d.id === id).route, "local", id);
  }
});

test("detectTicketKey finds keys across title/branch/slug, uppercased, first hit wins", () => {
  assert.equal(detectTicketKey("Fix login PS-1234"), "PS-1234");
  assert.equal(detectTicketKey("", "fix/ps-987-login"), "PS-987");
  assert.equal(detectTicketKey(null, undefined, "dop-12-cleanup"), "DOP-12");
  assert.equal(detectTicketKey("no ticket", "nope", "still-nothing"), "");
  // Single-letter fragments are not tickets; "utf-8"-style false positives
  // are accepted — this only pre-fills an editable prompt.
  assert.equal(detectTicketKey("v-2 of the api"), "");
  assert.equal(detectTicketKey("utf-8 handling"), "UTF-8");
});

test("sectionsFromChecks normalizes to booleans over the full section list", () => {
  const s = sectionsFromChecks({ summary: 1, diff: undefined });
  assert.equal(s.summary, true);
  assert.equal(s.diff, false);
  assert.equal(s.plan, false);
  assert.deepEqual(Object.keys(s).sort(), SHARE_SECTIONS.map((x) => x.id).sort());
});

test("the HTML export is self-contained and escapes its title", () => {
  const html = shareHtmlDocument('Auth <"refactor"> & co', "<h1>body</h1>");
  assert.ok(html.startsWith("<!doctype html>"));
  assert.match(html, /<title>Auth &lt;"refactor"> &amp; co<\/title>/);
  assert.ok(html.includes("<h1>body</h1>"));
  // No external requests: no src/href pointing at a scheme or protocol-
  // relative URL (the artifact must open offline, from a file://).
  assert.ok(!/\b(src|href)\s*=\s*["'](https?:)?\/\//.test(html), "external reference found");
  assert.ok(!/<script/.test(html), "scripts must not ride along");
});

test("browser branch publishes every global app.js reads", () => {
  const src = fs.readFileSync(path.join(__dirname, "..", "dist", "wf-share.js"), "utf8");
  const sandbox = { window: {} };
  vm.createContext(sandbox);
  vm.runInContext(src, sandbox);
  for (const name of [
    "SHARE_SECTIONS",
    "SHARE_PRESETS",
    "presetSections",
    "shareModel",
    "sectionsFromChecks",
    "shareHtmlDocument",
    "detectTicketKey",
    "SHARE_EXPLAIN_PAIRS",
    "SHARE_EXPLAIN_DEPTHS",
    "explainLevelsToShare",
    "shareExplanationEntries",
    "explainOutline",
  ]) {
    assert.ok(name in sandbox.window, `${name} must be published to window`);
  }
});
