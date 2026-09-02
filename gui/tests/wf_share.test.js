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
  sectionsFromChecks,
  shareHtmlDocument,
  detectTicketKey,
} = require("../dist/wf-share.js");

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

test("webhook destinations exist only once configured, with a pointer to Settings", () => {
  const off = shareModel({});
  const slack = off.destinations.find((d) => d.id === "slack");
  assert.equal(slack.enabled, false);
  assert.match(slack.hint, /Settings → Workflows/);
  const on = shareModel({ slackConfigured: true, discordConfigured: true, jiraConfigured: true });
  assert.equal(on.destinations.find((d) => d.id === "slack").enabled, true);
  assert.equal(on.destinations.find((d) => d.id === "discord").enabled, true);
  assert.equal(on.destinations.find((d) => d.id === "jira").enabled, true);
  // Jira needs full config (URL + email + token) — partial reads as absent.
  const jiraOff = off.destinations.find((d) => d.id === "jira");
  assert.equal(jiraOff.enabled, false);
  assert.match(jiraOff.hint, /Settings → Workflows/);
  // Local destinations never need configuration.
  for (const id of ["clipboard", "md", "html"]) {
    assert.equal(off.destinations.find((d) => d.id === id).enabled, true, id);
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
  ]) {
    assert.ok(name in sandbox.window, `${name} must be published to window`);
  }
});

test("a named skill takes over a destination, and says so", () => {
  // The point of the skill transport: reach destinations clash has no client
  // for, through whatever the session's own tooling can reach. It has to be
  // visible on the button — a share leaving by a different route than you
  // expect is worse than either route.
  const withSkills = shareModel({ jiraSkill: "myorg:jira-post", chatSkill: "myorg:chat" });
  const by = (id) => withSkills.destinations.find((d) => d.id === id);
  for (const id of ["jira", "slack", "discord"]) {
    assert.equal(by(id).enabled, true, `${id} is available through its skill`);
    assert.match(by(id).hint, /in a Claude session/);
  }
  assert.equal(by("jira").skill, "myorg:jira-post");
  assert.equal(by("slack").skill, "myorg:chat");

  // Unconfigured either way: disabled, and the hint names both transports so
  // neither route looks like the only one.
  const bare = shareModel({});
  const bareBy = (id) => bare.destinations.find((d) => d.id === id);
  for (const id of ["jira", "slack", "discord"]) {
    assert.equal(bareBy(id).enabled, false);
    assert.equal(bareBy(id).skill, "");
    assert.match(bareBy(id).hint, /Settings → Workflows/);
    assert.match(bareBy(id).hint, /skill/);
  }

  // Credentials alone still work, with no skill claim in the hint.
  const creds = shareModel({ slackConfigured: true, jiraConfigured: true });
  assert.equal(creds.destinations.find((d) => d.id === "slack").enabled, true);
  assert.equal(creds.destinations.find((d) => d.id === "slack").hint, "");
  assert.equal(creds.destinations.find((d) => d.id === "discord").enabled, false);
});
