// Share & export — the pure half.
//
// The share dialog's whole promise is "the preview IS the payload": the
// backend's pure builder composes one markdown document, and every
// destination (clipboard, file, webhook) sends exactly that. What lives here
// is the model behind the dialog — which sections exist, what each preset
// selects, which destinations are available — plus the self-contained HTML
// shell an .html export wraps the rendered markdown in. The dialog itself is
// in `app.js` with the other modals (the `wf-compose.js` precedent).
(function () {
  "use strict";

  /// Every section the share document can carry, in the order the backend
  /// renders them. `heavy` marks the one worth a size warning (a diff can be
  /// tens of thousands of lines; webhooks truncate hard).
  const SHARE_SECTIONS = [
    { id: "summary", label: "Summary", detail: "Title, status, branch, iteration counters, PR links" },
    { id: "plan", label: "Plan", detail: "plan.md, verbatim" },
    { id: "timeline", label: "Change rounds", detail: "Your change-request notes, one per iteration" },
    { id: "reviews", label: "Agent reviews", detail: "Each round's verdict and what it published" },
    { id: "annotations", label: "Open comments", detail: "Unresolved diff comments" },
    { id: "diff", label: "Diff", detail: "The current diff, fenced", heavy: true },
  ];

  /// Three honest sizes. "Review packet" is the default: everything a
  /// colleague needs to weigh in, without the raw diff.
  const SHARE_PRESETS = [
    { id: "summary", label: "Summary", sections: ["summary"] },
    {
      id: "packet",
      label: "Review packet",
      sections: ["summary", "plan", "timeline", "reviews", "annotations"],
    },
    {
      id: "dossier",
      label: "Full dossier",
      sections: ["summary", "plan", "timeline", "reviews", "annotations", "diff"],
    },
  ];

  /// The `sections` object the backend's builder takes, for one preset.
  function presetSections(presetId) {
    const preset =
      SHARE_PRESETS.find((p) => p.id === presetId) || SHARE_PRESETS.find((p) => p.id === "packet");
    const out = {};
    for (const s of SHARE_SECTIONS) out[s.id] = preset.sections.includes(s.id);
    return out;
  }

  /// Everything the share dialog renders. A section that cannot have content
  /// (a review-only item has no plan) is still listed but disabled — an
  /// invisible option reads as a missing feature; a disabled one explains
  /// itself. Webhook destinations exist only once configured, with the hint
  /// saying where to configure them.
  function shareModel({
    hasPlan = true,
    slackConfigured = false,
    discordConfigured = false,
    jiraConfigured = false,
    // Skill transports: a named skill posts through a Claude Code session
    // (its own MCP servers and tooling), instead of clash's webhook/API
    // client. Non-empty wins, and the label says so — a share going out
    // through a different route than you expect is worse than either route.
    jiraSkill = "",
    chatSkill = "",
    preset = "packet",
  } = {}) {
    const checked = presetSections(preset);
    return {
      presets: SHARE_PRESETS.map((p) => ({ id: p.id, label: p.label })),
      preset: SHARE_PRESETS.some((p) => p.id === preset) ? preset : "packet",
      sections: SHARE_SECTIONS.map((s) => ({
        ...s,
        checked: checked[s.id] && (s.id !== "plan" || hasPlan),
        disabled: s.id === "plan" && !hasPlan,
        detail: s.id === "plan" && !hasPlan ? "This item has no plan phase" : s.detail,
      })),
      destinations: [
        { id: "clipboard", label: "Copy markdown", enabled: true, route: "local" },
        { id: "md", label: "Save .md…", enabled: true, route: "local" },
        { id: "html", label: "Save .html…", enabled: true, route: "local" },
        shareDestination("slack", "Send to Slack", chatSkill, slackConfigured),
        shareDestination("discord", "Send to Discord", chatSkill, discordConfigured),
        shareDestination("jira", "Post to Jira…", jiraSkill, jiraConfigured),
      ],
    };
  }

  /// One outward destination, with the route it will actually take.
  ///
  /// Three routes, in precedence order — explicit configuration first, and the
  /// session last because it is the one that spends tokens:
  ///
  /// - `skill` — a skill named in Settings posts it in a Claude session;
  /// - `client` — clash posts it itself (webhook URL, Jira API token);
  /// - `agent` — a Claude session with no skill named, using whatever tooling
  ///   it has connected (an MCP server for the destination, say).
  ///
  /// The `agent` route is why no destination is ever disabled: "clash has no
  /// credentials for this" is not the same as "this cannot be reached", and
  /// disabling the button on that basis hid a route the session already had.
  /// The route is on the label because a share leaving by a path you did not
  /// expect is worse than either path.
  function shareDestination(id, label, skill, clientConfigured) {
    const route = skill ? "skill" : clientConfigured ? "client" : "agent";
    const hint =
      route === "skill"
        ? `via the ${skill} skill, in a Claude session`
        : route === "client"
          ? ""
          : "in a Claude session, using the tools it has connected — spends tokens";
    return { id, label, enabled: true, route, skill: skill || "", hint };
  }

  /// The first Jira ticket key found in any of `texts` (PROJ-123, any case —
  /// branch names carry `ps-1234`), uppercased. Pre-fills the Jira ticket
  /// prompt from the item's title/branch/slug; the backend re-validates.
  function detectTicketKey(...texts) {
    for (const t of texts) {
      const m = /(?:^|[^A-Za-z0-9])([A-Za-z]{2,}[A-Za-z0-9]*-\d+)(?![A-Za-z0-9])/.exec(t || "");
      if (m) return m[1].toUpperCase();
    }
    return "";
  }

  /// The `sections` object from the dialog's live checkbox state.
  function sectionsFromChecks(checks) {
    const out = {};
    for (const s of SHARE_SECTIONS) out[s.id] = !!checks[s.id];
    return out;
  }

  /// Wrap rendered markdown in a self-contained HTML document — no external
  /// requests, prints cleanly, readable in both OS themes. Mermaid diagrams
  /// arrive already rendered to inline SVG (the dialog renders the markdown
  /// in a detached node first), so no script rides along.
  function shareHtmlDocument(title, bodyHtml) {
    const esc = String(title || "clash export")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;");
    return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${esc}</title>
<style>
  :root { color-scheme: light dark; }
  body { max-width: 56rem; margin: 2rem auto; padding: 0 1.25rem;
         font: 15px/1.6 -apple-system, "Segoe UI", system-ui, sans-serif; }
  h1, h2, h3 { line-height: 1.25; }
  h2 { margin-top: 2em; border-bottom: 1px solid rgba(128,128,128,.35); padding-bottom: .25em; }
  code, pre { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: .9em; }
  pre { padding: .75rem 1rem; border-radius: 6px; overflow-x: auto;
        background: rgba(128,128,128,.12); }
  blockquote { margin: 0; padding-left: 1rem; border-left: 3px solid rgba(128,128,128,.4); }
  table { border-collapse: collapse; }
  th, td { border: 1px solid rgba(128,128,128,.35); padding: .3rem .6rem; }
  img, svg { max-width: 100%; }
  a { color: inherit; }
  footer { margin-top: 3rem; font-size: .8em; opacity: .6; }
  @media print { body { margin: 0 auto; } }
</style>
</head>
<body>
${bodyHtml}
<footer>Exported from a clash workflow item.</footer>
</body>
</html>
`;
  }

  const api = {
    shareDestination,
    SHARE_SECTIONS,
    SHARE_PRESETS,
    presetSections,
    shareModel,
    sectionsFromChecks,
    shareHtmlDocument,
    detectTicketKey,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
