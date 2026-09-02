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
  assert.match(unconfigured.hint, /switch this destination to a Claude session/);
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
  ]) {
    assert.ok(name in sandbox.window, `${name} must be published to window`);
  }
});
