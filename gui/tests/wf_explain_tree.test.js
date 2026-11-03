// node --test gui/tests/ — the explain tab's drill-down navigation, pure half.
//
// What is expanded, stale or diveable is derived in Rust; these pin how the
// tab reads that answer: the crumbs back up, what each part's chip offers,
// which parts the "explain the critical parts" batch picks, and how a part is
// recognised in the write-up and in the drawn page.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  explainPartPreview,
  explainFocusSuggestions,
  explainFilePath,
  explainViewTabs,
  explainResolveView,
  explainViewId,
  explainFrameIntact,
  explainTreeTarget,
  explainLevel,
  explainParentOf,
  explainBreadcrumb,
  runningDive,
  explainPartState,
  criticalToDive,
  headingPartId,
  fragmentPartId,
  explainDetailChoices,
  matchRepoForPr,
  parsePrUrl,
} = require("../dist/wf-explain-tree.js");

const part = (id, over = {}) => ({ id, title: `P${id}`, diveable: true, expanded: false, stale: false, ...over });
const TREE = [
  { node: "", nodes: [part("1", { expanded: true }), part("2", { critical: true })] },
  { node: "1", nodes: [part("1.1", { title: "Backoff" })] },
];

test("each document pair drills into its own tree", () => {
  assert.equal(explainTreeTarget("plan"), "explain-plan");
  assert.equal(explainTreeTarget("diff"), "explain-diff");
  assert.equal(explainTreeTarget("drift"), "drift");
  assert.equal(explainTreeTarget("review"), null);
});

test("levels and parents", () => {
  assert.equal(explainLevel(TREE, "").node, "");
  assert.equal(explainLevel(TREE, undefined).node, "", "no node is the overview");
  assert.equal(explainLevel(TREE, "9"), null);
  assert.equal(explainParentOf("3.2.1"), "3.2");
  assert.equal(explainParentOf("3"), "");
  assert.equal(explainParentOf(""), "");
});

test("the breadcrumb walks back to the overview, titled by the parent's listing", () => {
  assert.deepEqual(explainBreadcrumb(TREE, ""), [{ node: "", label: "Overview" }]);
  assert.deepEqual(
    explainBreadcrumb(TREE, "1.1").map((c) => [c.node, c.label]),
    [
      ["", "Overview"],
      ["1", "1 P1"],
      ["1.1", "1.1 Backoff"],
    ]
  );
  // A level the parent no longer lists still gets a crumb — just its id.
  assert.equal(explainBreadcrumb(TREE, "7")[1].label, "7");
});

test("a part's chip says what clicking it does", () => {
  const item = (explainers = []) => ({ explainers });
  const t = "explain-diff";
  assert.equal(explainPartState(item(), t, part("1", { expanded: true })), "open");
  assert.equal(explainPartState(item(), t, part("1", { expanded: true, stale: true })), "stale");
  assert.equal(explainPartState(item(), t, part("2")), "dive");
  assert.equal(explainPartState(item(), t, part("3", { diveable: false })), "none");
  // A dive in flight on that part, and only that part, of that tree.
  const diving = item([{ target: t, node: "2", running: true }]);
  assert.equal(explainPartState(diving, t, part("2")), "diving");
  assert.equal(explainPartState(diving, "drift", part("2")), "dive");
  assert.equal(explainPartState(item([{ target: t, node: "2", running: false }]), t, part("2")), "dive");
  assert.ok(runningDive(diving, t, "2"));
  assert.equal(runningDive(diving, t, "1"), null);
});

test("the critical batch picks critical parts that are not open or being written", () => {
  const level = {
    nodes: [
      part("1", { critical: true }),
      part("2", { critical: true, expanded: true }),
      part("3", { critical: false }),
      part("4", { critical: true, diveable: false }),
      part("5", { critical: true }),
    ],
  };
  const item = { explainers: [{ target: "explain-diff", node: "5", running: true }] };
  assert.deepEqual(
    criticalToDive(item, "explain-diff", level).map((p) => p.id),
    ["1"]
  );
  assert.deepEqual(criticalToDive({}, "explain-diff", null), []);
});

test("a part is recognised at the start of a heading and in a page fragment", () => {
  assert.equal(headingPartId("3. Retry wrapper"), "3");
  assert.equal(headingPartId("3.2 Backoff"), "3.2");
  assert.equal(headingPartId("  1.2.3. Leaf"), "1.2.3");
  assert.equal(headingPartId("Risks & review focus"), null);
  assert.equal(headingPartId("2026 roadmap"), "2026", "a number is a number — matched to a part only if listed");
  assert.equal(fragmentPartId("#node-3.2"), "3.2");
  assert.equal(fragmentPartId("#node-1.2.3.4"), null, "beyond the depth cap");
  assert.equal(fragmentPartId("#node-../x"), null);
  assert.equal(fragmentPartId("#other"), null);
  assert.equal(fragmentPartId(""), null);
});

test("the detail choices lead with the setting's value", () => {
  assert.deepEqual(
    explainDetailChoices("critical").map((c) => c.value),
    ["critical", "overview", "full"]
  );
  assert.equal(explainDetailChoices("full")[0].value, "full");
  assert.equal(explainDetailChoices("bogus").length, 3);
});

test("a PR URL finds its local checkout, and only an unambiguous one", () => {
  assert.deepEqual(parsePrUrl("https://github.com/acme/api/pull/42"), { repo: "acme/api", number: 42 });
  assert.deepEqual(parsePrUrl(" https://ghe.corp.io/team/web/pull/7/files "), { repo: "team/web", number: 7 });
  assert.equal(parsePrUrl("acme/api#42"), null);
  const repos = [
    { path: "/r/api", origin: "Acme/API" },
    { path: "/r/web", origin: "acme/web" },
    { path: "/r/web2", origin: "acme/web" },
  ];
  assert.equal(matchRepoForPr("acme/api", repos).path, "/r/api");
  assert.equal(matchRepoForPr("acme/web", repos), null, "two checkouts: the human picks");
  assert.equal(matchRepoForPr("acme/none", repos), null);
  assert.equal(matchRepoForPr("", repos), null);
});

test("a level's tabs: the drawing, each view, the write-up — only what exists", () => {
  const views = [
    { id: "flow", kind: "flow", title: "Request flow", why: "three services in a row" },
    { id: "life", kind: "state", title: "Lifecycle" },
    { id: "odd", title: "" },
  ];
  const tabs = explainViewTabs({ html: true, md: true }, views);
  assert.deepEqual(
    tabs.map((t) => t.key),
    ["graphic", "view:flow", "view:life", "view:odd", "text"]
  );
  assert.equal(tabs[1].label, "→ Request flow");
  assert.equal(tabs[1].hint, "three services in a row");
  assert.equal(tabs[2].label, "◎ Lifecycle");
  assert.equal(tabs[3].label, "◇ odd");
  assert.deepEqual(explainViewTabs({ md: true }, []).map((t) => t.key), ["text"]);
});

test("the tab shown falls back to the drawing when the level lacks the view", () => {
  const views = [{ id: "flow" }];
  assert.equal(explainResolveView("view:flow", { html: true }, views), "view:flow");
  assert.equal(explainResolveView("view:data", { html: true }, views), "graphic");
  assert.equal(explainResolveView("text", { html: true }, views), "graphic");
  assert.equal(explainResolveView("text", { html: true, md: true }, views), "text");
  assert.equal(explainResolveView(undefined, { md: true }, views), "view:flow");
  assert.equal(explainResolveView(undefined, {}, []), "text");
});

test("view keys carry a safe id", () => {
  assert.equal(explainViewId("view:before-after"), "before-after");
  assert.equal(explainViewId("graphic"), null);
  assert.equal(explainViewId("view:../plan"), null);
  assert.equal(explainViewId("view:Flow"), null);
});

test("a frame that navigated away from its drawing is noticed", () => {
  assert.ok(explainFrameIntact("about:srcdoc"));
  assert.ok(explainFrameIntact("about:srcdoc#node-3"));
  assert.ok(!explainFrameIntact("tauri://localhost/#node-3"));
  assert.ok(!explainFrameIntact("https://example.com/"));
  assert.ok(!explainFrameIntact(""));
});

test("a part with no level yet is previewed from the level that lists it", () => {
  const tree = [
    {
      node: "",
      title: "Rate limiting",
      nodes: [
        { id: "1", title: "Gateway" },
        { id: "2", title: "Counter", summary: "counts calls", critical: true, why: "every call", files: ["src/c.rs:40"], isNew: true },
        { id: "3", title: "Service" },
      ],
      edges: [
        { from: "1", to: "2", label: "api key" },
        { from: "2", to: "3", label: "forwards" },
        { from: "2", to: "2", label: "self" },
      ],
    },
  ];
  const p = explainPartPreview(tree, "2");
  assert.equal(p.part.title, "Counter");
  assert.equal(p.parentTitle, "Rate limiting");
  assert.equal(p.depth, 1);
  assert.deepEqual(p.incoming, [{ id: "1", title: "Gateway", label: "api key" }]);
  assert.deepEqual(p.outgoing, [{ id: "3", title: "Service", label: "forwards" }]);
  assert.deepEqual(p.siblings.map((n) => n.id), ["1", "3"]);
  assert.deepEqual(p.files, ["src/c.rs:40"]);
  assert.match(p.deeper, /inside it/);
  assert.equal(explainPartPreview(tree, "9"), null);
  assert.equal(explainPartPreview(tree, "2.1"), null, "its parent level is not written");

  const asks = explainFocusSuggestions(p);
  assert.ok(asks.includes("Draw the flow through it, in order"));
  assert.ok(asks.includes("Why is it needed — what did we do before?"));
  assert.equal(explainFilePath("src/c.rs:40"), "src/c.rs");
  assert.equal(explainFilePath("src/c.rs:40:2"), "src/c.rs");
  assert.equal(explainFilePath("README.md"), "README.md");
});
