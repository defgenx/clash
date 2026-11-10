// node --test gui/tests/ — the Waves roadmap and the Pull requests tab, run.
//
// Both views live in app.js and are reached only by clicking a tab, so a
// dangling reference in either surfaces as an empty tab while clash.log
// shows a healthy boot. This runs the real render functions against the real
// pure modules over a ten-PR, multi-wave item and reads back what they drew.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { extractFunction, stubEl, descendants } = require("./extract-fn.js");

const dist = (f) => fs.readFileSync(path.join(__dirname, "..", "dist", f), "utf8");
const APP = dist("app.js");

const gh = (repo, n) => `https://github.com/o/${repo}/pull/${n}`;
const linked = [
  ["saturn", 454, "OPEN"],
  ["saturn", 455, "OPEN"],
  ["saturn", 456, "OPEN"],
  ["k8s", 9313, "MERGED"],
  ["helm-charts", 1902, "MERGED"],
  ["api", 977, "OPEN"],
  ["api", 978, "OPEN"],
  ["api", 979, "CLOSED"],
  ["web", 39797, "OPEN"],
].map(([r, n, state]) => ({ url: gh(r, n), number: n, state }));
const wave = (id, state, level, prs, extra = {}) => ({
  id,
  title: `${id} title`,
  summary: `${id} summary`,
  dependsOn: level ? [`w${level}`] : [],
  areas: [`${id}/`],
  state,
  blockedBy: state === "blocked" ? [`w${level}`] : [],
  level,
  compatibleWith: [],
  tickets: [`TT-${level}`],
  ticketUrls: { [`TT-${level}`]: `https://jira/browse/TT-${level}` },
  prs,
  ...extra,
});
const ITEM = {
  project: "p",
  slug: "s",
  prCheckedAt: Date.now() - 5 * 60000,
  prErrors: { [gh("api", 978)]: "HTTP 502" },
  meta: {
    status: "pr-ready",
    pr: { url: gh("saturn", 457), number: 457, state: "OPEN", headRef: "x", baseRef: "y" },
    linkedPrs: linked,
  },
  waves: {
    waves: [
      wave("w1", "review", 0, linked.filter((p) => p.number !== 456).map((p) => p.url)),
      wave("w2", "blocked", 1, [gh("saturn", 456), gh("saturn", 457)]),
      wave("w3", "blocked", 2, []),
    ],
  },
};

function sandbox() {
  const box = {
    console,
    Date,
    CSS: { escape: (s) => s },
    document: { createElement: (t) => stubEl(t), body: { appendChild() {} } },
    invoke: async () => ({}),
    busyButton: (b, fn) => fn(),
    openLink: () => {},
    openLinks: () => {},
    showContextMenu: () => {},
    wfPrMenu: () => [],
    wfApproveWaves: () => {},
    wfStartWaves: () => {},
    refreshWorkflows: async () => {},
    wfItem: () => ITEM,
    wfKey: (p, s) => `${p}/${s}`,
    wfTabState: new Map(),
    state: { sessions: [] },
    openSession: () => {},
    wfSid: (_, id) => id,
    wfComposeReviewRound: () => {},
    setTimeout,
    flashToast: () => {},
    svgIcon: (name) => `<svg data-icon="${name}"></svg>`,
  };
  box.window = box;
  vm.createContext(box);
  for (const f of ["wf-prs.js", "wf-waves.js"]) vm.runInContext(dist(f), box);
  for (const fn of ["wfPrStateChip", "wfWaveDecisionButtons", "wfWaveCard", "wfStepTogether", "wfRenderWaveLive", "renderWfWavesView", "renderWfPrsView"])
    vm.runInContext(extractFunction(APP, fn), box);
  return box;
}

const texts = (root) => descendants(root).map((e) => e.textContent).filter(Boolean);

test("the roadmap opens on what is happening, then walks the waves with their PRs", () => {
  const box = sandbox();
  const body = stubEl();
  box.renderWfWavesView(body, {}, ITEM);
  const t = texts(body);
  assert.ok(t.some((x) => /w1 · w1 title is waiting for your sign-off\. 2 of 8 PRs merged, 5 still open\./.test(x)), t.join("\n"));
  assert.ok(t.some((x) => /^Next: .*unlocks w2/.test(x)));
  assert.ok(t.includes("✓ Sign off w1 → next waves…"), "the decision sits on the roadmap");
  assert.ok(t.includes("Step 1") && t.includes("Step 3"));
  assert.ok(t.includes("2/8 PRs merged"));
  assert.ok(t.includes("TT-0"), "tickets are shown");
  assert.ok(t.includes("saturn#456 · open"), "a wave's PRs are chips");
  // The overview lists what still needs landing; the wave's Details, all.
  assert.ok(!t.includes("api#979 · closed"), "settled PRs wait in Details");
  box.wfTabState.set("p/s", { waveMode: "one", waveFocus: "w1" });
  const one = stubEl();
  box.renderWfWavesView(one, {}, ITEM);
  const t1 = texts(one);
  assert.ok(t1.includes("api#979 · closed"));
  assert.ok(t1.some((x) => /^Wave 1 of 3/.test(x)), t1.join("\n"));
});

test("a wave card says what blocks it and binds the bar's actions", () => {
  const box = sandbox();
  const body = stubEl();
  box.renderWfWavesView(body, {}, ITEM);
  const t = texts(body);
  // Soft blockers before approving, a waiting wave's dependency as one line.
  assert.ok(t.includes("Before you sign off"));
  assert.ok(t.some((x) => /^5 PRs not merged yet$/.test(x)), t.join("\n"));
  assert.ok(t.some((x) => /^1 PR not reached on the last refresh$/.test(x)));
  assert.ok(t.includes("Waits for w1 · w1 title — to sign off"));
  // The round's decisions and the wave's PR tools are on its card.
  assert.equal(t.filter((x) => x === "✓ Sign off w1 → next waves…").length, 2, "the roadmap's and the card's");
  assert.ok(t.includes("⌕ Review its 5 PRs…"));
  assert.ok(t.includes("Open 5 PRs"));
  assert.ok(["Flow", "List", "One at a time"].every((m) => t.includes(m)));
});

test("the Pull requests tab lists every PR by wave, with freshness and failures", async () => {
  const box = sandbox();
  const body = stubEl();
  await box.renderWfPrsView(body, {}, ITEM);
  const t = texts(body);
  assert.ok(t.some((x) => /^10 PRs · 2 merged · 1 closed · 7 open · checked 5 min ago$/.test(x)), t.join("\n"));
  assert.ok(t.some((x) => / · 1 not reached on the last refresh/.test(x)));
  const html = descendants(body).map((e) => e.innerHTML || "").join("\n");
  assert.match(html, /data-icon="reload".*Refresh all/);
  assert.match(html, /Open 7 open PRs/);
  // Groups follow the waves; a wave with nothing yet is left out.
  const heads = t.filter((x) => /^w\d · /.test(x));
  assert.deepEqual(heads, ["w1 · w1 title", "w2 · w2 title"]);
  assert.ok(t.includes("not refreshed: HTTP 502"));
  assert.ok(t.some((x) => /^primary · x → y$/.test(x)));
});

test("the new tabs are built from the app's own pieces", () => {
  // Decision rows use the action bar's buttons, states are .wf-chip, tools
  // are .icon-btn and the cards are the Timeline's — a hand-rolled look is
  // how these tabs ended up not matching the rest of the app.
  const box = sandbox();
  const roadmap = stubEl();
  box.renderWfWavesView(roadmap, {}, ITEM);
  const cls = (root) => descendants(root).map((e) => e.className || "");
  const r = cls(roadmap);
  assert.ok(r.some((c) => /\bwf-inline-actions\b/.test(c) && /\bwf-actions-btns\b/.test(c)));
  assert.ok(r.some((c) => /^wf-tl-card wf-wave\b/.test(c)));
  assert.ok(r.some((c) => /^wf-chip wf-pr-state wf-pr-state-open$/.test(c)));
  assert.ok(r.every((c) => !/wf-pr-chip|wf-waves-dot|wf-waves-actions/.test(c)), "no retired one-off classes");
  const prs = stubEl();
  box.renderWfPrsView(prs, {}, ITEM);
  const p = cls(prs);
  assert.ok(p.filter((c) => c === "icon-btn wide").length >= 2, "toolbar buttons are icon buttons");
  assert.ok(p.includes("icon-btn mini"), "the row menu is an icon button");
  assert.ok(p.some((c) => /^wf-tl-card wf-itemprs-group$/.test(c)));
  // The action bar and the inline rows share one button rule.
  const css = fs.readFileSync(path.join(__dirname, "..", "dist", "style.css"), "utf8");
  assert.match(css, /\.wf-actions button,\s*\.wf-inline-actions button \{/);
  assert.match(css, /\.wf-actions button\.primary,\s*\.wf-inline-actions button\.primary \{/);
});

test("the tabs use the width they are given", () => {
  // One column in a narrow tab, the now-panel beside the steps and groups in
  // columns in a wide one — sized by the tab (a container query), not the
  // window, because a tab in a split pane is narrow on a wide screen.
  const box = sandbox();
  const roadmap = stubEl();
  box.renderWfWavesView(roadmap, {}, ITEM);
  const top = roadmap.children[0];
  assert.match(roadmap.className, /\bwf-wide-view\b/);
  assert.equal(top.className, "wf-waves-layout");
  assert.deepEqual(top.children.map((c) => c.className), ["wf-waves-aside", "wf-waves-main"]);
  const cls = (root) => descendants(root).map((e) => e.className || "");
  assert.ok(cls(top.children[1]).includes("wf-waves-grid"), "a step's waves sit in a grid");
  assert.ok(cls(top.children[0]).some((c) => /wf-waves-stats/.test(c)), "PR counts beside the story");
  const prs = stubEl();
  box.renderWfPrsView(prs, {}, ITEM);
  assert.ok(cls(prs).includes("wf-itemprs-grid"));
  const css = fs.readFileSync(path.join(__dirname, "..", "dist", "style.css"), "utf8");
  assert.match(css, /\.wf-wide-view \{ container-type: inline-size;/);
  assert.match(css, /@container \(min-width: 980px\) \{[\s\S]*?\.wf-waves-layout \{ grid-template-columns: minmax\(280px, 360px\) minmax\(0, 1fr\); \}/);
  // The other lists stop at no fixed width either.
  assert.doesNotMatch(css, /\.wf-prs \{[^}]*max-width/);
  assert.match(css, /\.wf-md \{[^}]*max-width: none;/);
  // The dashboard's rows keep their own class: the item tab must not restyle them.
  assert.doesNotMatch(css, /\.wf-prs-row \{ display: flex; align-items: center; gap: 10px; padding: 4px 0/);
});

test("past a handful of PRs the header summarizes and points at the tab", () => {
  const view = extractFunction(APP, "buildWorkflowView");
  assert.match(APP, /const WF_HEAD_PR_LIMIT = 3;/);
  assert.match(view, /const manyPrs = headPrs\.length > WF_HEAD_PR_LIMIT;/);
  assert.match(view, /for \(const pr of manyPrs \? headPrs\.filter\(\(p\) => p\.primary\) : headPrs\)/);
  assert.match(view, /ts\.subView = "prs";/);
  // A project in waves opens on its roadmap.
  assert.match(view, /if \(!ts\.subView && item\.waves\) ts\.subView = "waves";/);
});

test("a plan with parallel waves opens on the Flow, and together-or-alone is yours", () => {
  // w2 and w3 may share a round and are both ready: the step offers them
  // together once, each card offers itself alone.
  const par = JSON.parse(JSON.stringify(ITEM));
  par.meta.status = "diff-review";
  par.waves.waves = [
    wave("w1", "done", 0, []),
    wave("w2", "ready", 1, [], { dependsOn: ["w1"], compatibleWith: ["w3"] }),
    wave("w3", "ready", 1, [], { dependsOn: ["w1"], compatibleWith: ["w2"] }),
  ];
  const box = sandbox();
  const body = stubEl();
  box.renderWfWavesView(body, {}, par);
  const t = texts(body);
  assert.ok(t.includes("⇉ parallel"), "the step says its waves can run in parallel");
  assert.equal(t.filter((x) => x === "▶ Start w2 + w3 together…").length, 1, "together is offered once, for the step");
  assert.ok(t.includes("▶ Start w2 alone…") && t.includes("▶ Start w3 alone…"));
  const cls = descendants(body).map((e) => e.className || "");
  assert.ok(cls.includes("wf-waves-flow"), "Flow is where a parallel plan opens");
  assert.ok(cls.includes("wf-flow-col parallel"));
  // A straight chain opens on the List.
  const chain = stubEl();
  sandbox().renderWfWavesView(chain, {}, ITEM);
  assert.ok(!descendants(chain).some((e) => e.className === "wf-waves-flow"));
});
