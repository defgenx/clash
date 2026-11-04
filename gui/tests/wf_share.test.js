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
  shareHeadingSlug,
  shareHeadingSlugs,
  shareHtmlDocument,
  detectTicketKey,
  SHARE_EXPLAIN_PAIRS,
  explainLevelsToShare,
  shareAnchor,
  shareExplanationChapter,
  explainOutline,
  shareViewer,
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

test("a chapter holds every kept level, its drawings framed and its parts described", () => {
  const tree = [
    { ...TREE[0], edges: [{ from: "1", to: "2", label: "retries" }], views: [{ id: "flow", title: "Request flow" }] },
    TREE[1],
    TREE[2],
  ];
  const levels = explainLevelsToShare(tree, "parts", true);
  const ch = shareExplanationChapter(DIFF, levels, {
    "": { bodyHtml: "<p>w</p>", frameHtml: "<div>d</div>", views: { flow: "<div>f</div>" } },
  });
  assert.equal(ch.id, "explain-diff");
  assert.deepEqual(Object.keys(ch.levels).sort(), ["", "1"], "1.1 was cut at this depth; part 2 never opened");
  const root = ch.levels[""];
  assert.equal(root.title, "Changes explained");
  assert.deepEqual(root.parts.map((p) => [p.id, p.critical, p.summary]), [["1", true, "wraps calls"], ["2", false, ""]]);
  assert.deepEqual(root.edges, [{ from: "1", to: "2", label: "retries" }]);
  // Drawings travel as ready frame documents: the base that keeps their
  // `#node-` links inside the frame is already there.
  assert.match(root.drawing, /^<!doctype html><html><head><base href="about:srcdoc">/);
  assert.deepEqual(root.views.map((v) => [v.id, v.title]), [["flow", "Request flow"]]);
  assert.match(root.views[0].html, /<base href="about:srcdoc">[\s\S]*<div>f<\/div>/);
  assert.equal(ch.levels["1"].title, "Retry", "a part's level is named by its parent");
  assert.equal(ch.levels["1"].stale, true);
  assert.equal(ch.levels["1"].drawing, "", "no drawing: the explorer draws a map of the parts");
  assert.deepEqual(explainOutline([ch]), ["Changes explained — overview + 1 part"]);
});

const scriptsOf = (html) => [...html.matchAll(/<script([^>]*)>([\s\S]*?)<\/script>/g)].map((m) => ({ attrs: m[1], body: m[2] }));

test("the saved page is an explorer whose drawings stay script-less frames", () => {
  const levels = explainLevelsToShare(TREE, "all", true);
  const drawn = '<div onclick="x()">drawn "page"</div><script>alert(1)</script>';
  const ch = shareExplanationChapter(DIFF, levels, {
    "": { bodyHtml: "<p>write-up</p>", frameHtml: drawn },
    1: { bodyHtml: "<p>inside</p>", frameHtml: "" },
  });
  const html = shareHtmlDocument("T", "<p>doc</p>", [ch]);
  assert.ok(!/allow-scripts/.test(html));
  assert.ok(!/\b(src|href)\s*=\s*["'](https?:)?\/\//.test(html), "external reference found");
  // Two scripts: the data, which no string inside can close, and the
  // explorer, which compiles.
  const [data, viewer, ...rest] = scriptsOf(html);
  assert.equal(rest.length, 0);
  assert.equal(data.attrs, ' type="application/json" id="x-data"');
  assert.ok(!data.body.includes("<"), "a < in the data could close its element");
  const parsed = JSON.parse(data.body);
  assert.equal(parsed.chapters[0].levels[""].drawing.includes(drawn), true, "the drawing rides as data, never as markup");
  assert.ok(!html.replace(data.body, "").includes("drawn &quot;page&quot;") && !html.replace(data.body, "").includes('drawn "page"'));
  assert.doesNotThrow(() => new vm.Script(viewer.body));
  // The frames are made by the explorer, sandboxed without scripts.
  assert.match(viewer.body, /setAttribute\("sandbox", "allow-same-origin"\)/);
  // The document reads without script.
  assert.ok(html.includes('<section class="doc" id="x-document"><article class="md">\n<p>doc</p>'));
  // No explanations: the plain document, without any script.
  const plain = shareHtmlDocument("T", "<p>doc</p>");
  assert.ok(!/<script/.test(plain) && !plain.includes("x-explorer"));
});

test("the explorer is self-contained: it reads nothing from the module around it", () => {
  // It is shipped with toString(), so a closure variable would be a
  // ReferenceError on the recipient's machine and nowhere in clash.
  const src = shareViewer.toString();
  const free = ["shareFrameDoc", "shareAnchor", "parentOf(l", "nodeDepth", "PAGE_STYLE", "scriptJson", "api"].filter((name) =>
    new RegExp(`\\b${name.replace("(", "\\(")}`).test(src)
  );
  assert.deepEqual(free, []);
  assert.ok(!src.includes("</script"), "would close the page's script element");
});

test("smoke: the explorer zooms into a part and back out", async () => {
  // A tiny DOM, enough for the explorer to boot, route and swap layers.
  const levels = explainLevelsToShare(TREE, "all", true);
  const ch = shareExplanationChapter(DIFF, levels, {
    "": { bodyHtml: "<h3>1. Retry</h3><p>w</p>", frameHtml: "" },
    1: { bodyHtml: "<p>inside</p>", frameHtml: "" },
  });
  const html = shareHtmlDocument("T", "<p>doc</p>", [ch]);
  const [data, viewer] = scriptsOf(html);
  const dom = fakeDom(JSON.parse(data.body));
  vm.createContext(dom.window);
  vm.runInContext(viewer.body, dom.window);
  await dom.flush();
  assert.equal(dom.window.document.title, "Changes explained — T");
  assert.ok(dom.byId("x-panel").innerHTML.includes("Inside"));
  dom.go("#explain-diff/1");
  await dom.flush();
  assert.equal(dom.window.document.title, "1 Retry — T");
  assert.deepEqual(
    dom.byId("x-path").children.filter((c) => c.tagName === "BUTTON").map((c) => c.textContent),
    ["Overview", "1 Retry"]
  );
  dom.go("#explain-diff/2");
  await dom.flush();
  assert.ok(dom.byId("x-panel").innerHTML.includes("2 Docs"), "a part with no level still has a page");
  dom.go("#document");
  await dom.flush();
  assert.equal(dom.byId("x-document").hidden, false);
  assert.equal(dom.byId("x-explorer").hidden, true);
});

/// The smallest DOM the explorer boots in: elements are plain objects that
/// remember children, attributes and innerHTML; layout reads return zeros.
function fakeDom(data) {
  const els = {};
  const listeners = {};
  const make = (tag) => {
    const el = {
      tagName: String(tag).toUpperCase(),
      children: [],
      style: {},
      dataset: {},
      attrs: {},
      hidden: false,
      className: "",
      textContent: "",
      html: "",
      get innerHTML() { return this.html; },
      set innerHTML(v) { this.html = v; this.children = []; },
      classList: {
        set: new Set(),
        add(c) { this.set.add(c); },
        remove(c) { this.set.delete(c); },
        toggle(c, on) { (on === undefined ? !this.set.has(c) : on) ? this.set.add(c) : this.set.delete(c); },
        contains(c) { return this.set.has(c); },
      },
      appendChild(c) { this.children.push(c); c.parentNode = this; return c; },
      append(...cs) { cs.forEach((c) => this.appendChild(c)); },
      remove() { if (this.parentNode) this.parentNode.children = this.parentNode.children.filter((x) => x !== this); },
      setAttribute(k, v) { this.attrs[k] = v; },
      getAttribute(k) { return this.attrs[k]; },
      removeAttribute(k) { delete this.attrs[k]; },
      addEventListener() {},
      querySelector() { return null; },
      querySelectorAll() { return []; },
      getBoundingClientRect() { return { left: 0, top: 0, width: 800, height: 600 }; },
      scrollIntoView() {},
      cloneNode() { return make(tag); },
      get offsetWidth() { return 0; },
    };
    return el;
  };
  for (const id of ["x-data", "x-stage", "x-panel", "x-document", "x-explorer", "x-theme", "x-chapters", "x-read", "x-out", "x-path", "x-angles"]) {
    els[id] = make("div");
  }
  els["x-data"].textContent = JSON.stringify(data);
  const location = { hash: "" };
  const document = {
    title: "T",
    documentElement: make("html"),
    getElementById: (id) => els[id] || null,
    createElement: make,
    addEventListener() {},
  };
  const timers = [];
  const window = {
    document,
    location,
    JSON,
    Promise,
    Object,
    Array,
    String,
    Number,
    Math,
    CSS: { escape: (s) => s },
    localStorage: { getItem: () => null, setItem() {} },
    matchMedia: () => ({ matches: true }),
    addEventListener: (k, f) => { listeners[k] = f; },
    scrollTo() {},
    requestAnimationFrame: (f) => f(),
    setTimeout: (f) => { timers.push(f); return timers.length; },
    open() {},
  };
  window.window = window;
  Object.defineProperty(location, "hash", {
    get() { return this._h || ""; },
    set(v) { this._h = v.startsWith("#") ? v : `#${v}`; if (listeners.hashchange) listeners.hashchange(); },
  });
  return {
    window,
    byId: (id) => els[id],
    go: (h) => { location.hash = h; },
    flush: () => new Promise((r) => setImmediate(r)),
  };
}

test("the page export reads explanations only through the document readers", () => {
  const fn = extractFunction(APP, "wfShareExplanations");
  const cmds = [...fn.matchAll(/invoke\("([a-z_]+)"/g)].map((m) => m[1]);
  assert.deepEqual([...new Set(cmds)].sort(), ["get_workflow_doc", "get_workflow_explain_node", "get_workflow_explain_tree"]);
  assert.match(fn, /renderMarkdownForExport\(md, `\$\{anchor\}--`\)/, "write-up ids are kept apart from the document's");
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
      if (cmd === "get_workflow_explain_tree")
        return args.target === "explain-diff"
          ? TREE.map((l) => (l.node === "1.1" ? { ...l, views: [{ id: "flow", title: "Flow" }] } : l))
          : [];
      if (cmd === "get_workflow_doc") return docs[args.doc] || "";
      if (cmd === "get_workflow_explain_node") return `${args.node}.${args.form}`;
      throw new Error(`unexpected ${cmd}`);
    },
    renderMarkdownForExport: async (md) => `<rendered>${md}</rendered>`,
    dlog: () => {},
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
  const level = (node) => groups[0].levels[node];
  const root = level("");
  assert.equal(root.writeup, "<rendered># legacy write-up</rendered>");
  assert.match(root.drawing, /<body><div>drawn<\/div><style data-clash-breathe>/);
  assert.match(level("1").drawing, /<body>1\.html<style data-clash-breathe>/);
  // A level's extra views travel with it, read through the same node reader.
  assert.deepEqual(level("1.1").views.map((v) => [v.id, /<body>1\.1\.html<style/.test(v.html)]), [["flow", true]]);
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
    "shareHeadingSlugs",
    "shareHtmlDocument",
    "detectTicketKey",
    "SHARE_EXPLAIN_PAIRS",
    "SHARE_EXPLAIN_DEPTHS",
    "explainLevelsToShare",
    "shareExplanationChapter",
    "explainOutline",
  ]) {
    assert.ok(name in sandbox.window, `${name} must be published to window`);
  }
});

test("heading anchors follow the same rule as the Rust builder", () => {
  // The same cases as `slugs_follow_github` in workflow_share.rs: the links
  // are written there, the ids are made here.
  assert.equal(shareHeadingSlug("Change rounds"), "change-rounds");
  assert.equal(shareHeadingSlug("Iteration 1 — 2026-08-01 10:00"), "iteration-1--2026-08-01-1000");
  assert.equal(shareHeadingSlug("src/a.rs"), "srcars");
  assert.equal(shareHeadingSlug("Use the API now"), "use-the-api-now");
  assert.equal(shareHeadingSlug("Déjà vu_2"), "déjà-vu_2");
  assert.deepEqual(shareHeadingSlugs(["Plan", "Plan", "Plan 1", "Plan"]), ["plan", "plan-1", "plan-1-1", "plan-2"]);
});

test("every preset names real sections, and the chat-sized ones carry no contents", () => {
  const ids = new Set(SHARE_SECTIONS.map((s) => s.id));
  for (const p of SHARE_PRESETS) for (const s of p.sections) assert.ok(ids.has(s), `${p.id}: ${s}`);
  const of = (id) => SHARE_PRESETS.find((p) => p.id === id).sections;
  assert.ok(!of("summary").includes("toc"));
  assert.ok(!of("status").includes("toc"));
  assert.ok(of("packet").includes("toc"));
  assert.ok(of("explain").includes("explainDiff"));
  assert.ok(of("handover").includes("handoff"));
});

test("an unwritten agent document is listed, disabled and says why", () => {
  const m = shareModel({ preset: "dossier", has: { explainPlan: false, drift: false } });
  const row = (id) => m.sections.find((s) => s.id === id);
  assert.equal(row("explainPlan").disabled, true);
  assert.equal(row("explainPlan").checked, false);
  assert.match(row("explainPlan").detail, /not been explained/);
  assert.equal(row("drift").disabled, true);
  // Unknown means present: a caller that says nothing hides nothing.
  assert.equal(row("explainDiff").disabled, false);
  assert.equal(row("explainDiff").checked, true);
  assert.equal(row("handoff").checked, true);
});

test("drawn pages breathe: the page's own width cap is lifted, last", () => {
  const { shareFrameDoc, withBreathingRoom, markWideDrawings } = require("../dist/wf-share.js");
  const capped = "<html><head><style>body{max-width:920px}</style></head><body><svg viewBox='0 0 900 470'></svg></body></html>";
  const framed = shareFrameDoc(capped);
  // After the page's own styles, so it wins; still exactly one base, first.
  assert.ok(framed.indexOf("data-clash-breathe") > framed.indexOf("max-width:920px"));
  assert.match(framed, /max-width:none!important[\s\S]*<\/style><\/body><\/html>$/);
  assert.equal(framed.match(/<base /g).length, 1);
  assert.match(withBreathingRoom("<div>fragment</div>"), /^<div>fragment<\/div><style data-clash-breathe>/);
  // A page with <html> but no <head> still gets the base.
  assert.match(shareFrameDoc("<html><body>x</body></html>"), /^<html><head><base href="about:srcdoc">/);
  // Only an outermost, large drawing is scaled — icons keep their size.
  const svg = (vb, inner = false) => ({
    attrs: {},
    parentElement: { closest: () => (inner ? {} : null) },
    getAttribute() { return vb; },
    setAttribute(k, v) { this.attrs[k] = v; },
  });
  const big = svg("0 0 1200 600");
  const icon = svg("0 0 16 16");
  const nested = svg("0 0 900 400", true);
  assert.equal(markWideDrawings({ querySelectorAll: () => [big, icon, nested] }), 1);
  assert.ok("data-clash-wide" in big.attrs && !("data-clash-wide" in icon.attrs) && !("data-clash-wide" in nested.attrs));
  // clash's own tab applies the same room and the same marking.
  assert.match(extractFunction(APP, "wfExplainFrameDoc"), /return withBreathingRoom\(wfFrameWithHead\(html, head\)\);/);
  assert.match(extractFunction(APP, "wfGuardExplainFrame"), /markWideDrawings\(doc\);/);
});
