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
    // Who posts each family of destination, and the skill the session route
    // uses when one is named. Both come straight from Settings: this model
    // renders the human's choice, it does not make one.
    jiraTransport = "agent",
    chatTransport = "agent",
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
        shareDestination("slack", "Send to Slack", {
          transport: chatTransport,
          skill: chatSkill,
          clientConfigured: slackConfigured,
        }),
        shareDestination("discord", "Send to Discord", {
          transport: chatTransport,
          skill: chatSkill,
          clientConfigured: discordConfigured,
        }),
        shareDestination("jira", "Post to Jira…", {
          transport: jiraTransport,
          skill: jiraSkill,
          clientConfigured: jiraConfigured,
        }),
      ],
    };
  }

  /// One outward destination, with the route it will actually take.
  ///
  /// **Two routes, and the human picks which** — `transport` is a setting, not
  /// something inferred from what happens to be filled in:
  ///
  /// - `agent` (the default) — a Claude Code session posts it: through the
  ///   named `skill` when there is one, otherwise with whatever tooling that
  ///   session has connected (an MCP server for the destination, say). A skill
  ///   *refines* this route; it is not a route of its own, and its absence
  ///   never means "unavailable".
  /// - `clash` — clash posts it itself over HTTPS with the configured webhook
  ///   or API token.
  ///
  /// Neither is a fallback for the other. When `clash` is chosen and its
  /// credentials are missing, the destination is disabled and says so: silently
  /// launching a session instead would be clash deciding which system talks to
  /// your tracker, which is exactly the decision the setting exists to make.
  /// The route is always on the label — a share leaving by a path you did not
  /// expect is worse than either path.
  function shareDestination(id, label, { transport = "agent", skill = "", clientConfigured = false } = {}) {
    const viaClash = transport === "clash";
    const named = (skill || "").trim();
    if (viaClash) {
      return {
        id,
        label,
        route: "clash",
        skill: "",
        enabled: clientConfigured,
        hint: clientConfigured
          ? "clash posts it directly"
          : "clash posts this one directly, but its credentials are missing — fill them in, or switch this destination to an agent session in Settings → Workflows",
      };
    }
    return {
      id,
      label,
      route: "agent",
      skill: named,
      enabled: true,
      hint: named
        ? `in an agent session, via the ${named} skill — spends tokens`
        : "in an agent session, using the tools it has connected — spends tokens",
    };
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

  // ── Explanations in the saved page ──────────────────────────────────
  //
  // The `.html` export can also carry the item's explanation trees: each
  // level's drawn page and write-up, linked overview → part → sub-part. Only
  // the page carries them — every other destination sends the markdown alone.

  /// The three explanation trees, in page order.
  const SHARE_EXPLAIN_PAIRS = [
    { which: "plan", target: "explain-plan", label: "Plan explained" },
    { which: "diff", target: "explain-diff", label: "Changes explained" },
    { which: "drift", target: "drift", label: "Plan vs changes" },
  ];

  /// How deep the trees go into the page. `parts` = the overview plus the
  /// first level of every part someone opened.
  const SHARE_EXPLAIN_DEPTHS = [
    { value: "all", label: "Every level" },
    { value: "parts", label: "Overview + parts" },
    { value: "overview", label: "Overview only" },
    { value: "none", label: "None" },
  ];

  const nodeDepth = (node) => (node ? String(node).split(".").length : 0);
  const parentOf = (node) => {
    const i = String(node || "").lastIndexOf(".");
    return i < 0 ? "" : node.slice(0, i);
  };

  /// The levels of one tree (`get_workflow_explain_tree`'s answer, overview
  /// first) that go into the page at `depth`. A part's level goes only with
  /// the level above it, so every breadcrumb leads somewhere. An explanation
  /// with no index (written before drill-down) is its overview alone.
  function explainLevelsToShare(tree, depth, hasOverview) {
    if (depth === "none") return [];
    const max = depth === "overview" ? 0 : depth === "parts" ? 1 : 3;
    const levels = tree && tree.length ? tree : hasOverview ? [{ node: "", title: "", nodes: [] }] : [];
    const kept = [];
    for (const l of levels) {
      if (nodeDepth(l.node) > max) continue;
      if (l.node && !kept.some((k) => k.node === parentOf(l.node))) continue;
      kept.push(l);
    }
    return kept;
  }

  /// `x-explain-diff-3-2` — the page anchor of one level.
  function shareAnchor(target, node) {
    return `x-${target}${node ? "-" + String(node).replace(/\./g, "-") : ""}`;
  }

  /// One page section per kept level: title, breadcrumb, the parts it lists
  /// (linked when their own level is in the page) and its two forms.
  /// `docs[node]` = `{ bodyHtml, frameHtml }` — the rendered write-up and the
  /// agent's drawn page.
  function shareExplanationEntries(pair, levels, docs) {
    const listed = (node) => {
      const parent = levels.find((l) => l.node === parentOf(node));
      const n = parent && (parent.nodes || []).find((x) => x.id === node);
      return `${node} ${(n && n.title) || ""}`.trim();
    };
    return levels.map((l) => {
      const crumbs = [{ anchor: shareAnchor(pair.target, ""), label: pair.label }];
      if (l.node) {
        const segs = l.node.split(".");
        for (let i = 1; i <= segs.length; i++) {
          const id = segs.slice(0, i).join(".");
          crumbs.push({ anchor: shareAnchor(pair.target, id), label: listed(id) });
        }
      }
      const d = (docs && docs[l.node]) || {};
      return {
        anchor: shareAnchor(pair.target, l.node),
        title: l.node ? listed(l.node) : pair.label,
        depth: nodeDepth(l.node),
        stale: !!l.stale,
        crumbs,
        parts: (l.nodes || []).map((n) => ({
          id: n.id,
          title: n.title || "",
          summary: n.summary || "",
          critical: !!n.critical,
          anchor: levels.some((k) => k.node === n.id) ? shareAnchor(pair.target, n.id) : null,
        })),
        bodyHtml: d.bodyHtml || "",
        frameHtml: d.frameHtml || "",
      };
    });
  }

  const escText = (s) =>
    String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");

  /// The agent's drawn page as a frame document. It goes in `srcdoc` of a
  /// frame sandboxed without scripts, never into the export's own DOM.
  function shareFrameDoc(html) {
    const head =
      '<meta charset="utf-8"><style>:root{color-scheme:light dark}' +
      "body{margin:0;padding:16px;font:13px/1.55 -apple-system,'Segoe UI',system-ui,sans-serif}" +
      "svg{max-width:100%;height:auto}.box,.node{border:1px solid rgba(128,128,128,.4);border-radius:8px;padding:8px 10px}" +
      "</style>";
    return /<html[\s>]/i.test(html)
      ? html.replace(/<head(\s[^>]*)?>/i, (m) => m + head)
      : `<!doctype html><html><head>${head}</head><body>${html}</body></html>`;
  }

  function explanationHtml(e) {
    const crumbs =
      e.crumbs.length > 1
        ? `<nav class="crumbs">${e.crumbs
            .map((c) => (c.anchor === e.anchor ? escText(c.label) : `<a href="#${c.anchor}">${escText(c.label)}</a>`))
            .join(" › ")}</nav>`
        : "";
    const parts = e.parts.length
      ? `<ul class="parts">${e.parts
          .map(
            (p) =>
              `<li>${p.anchor ? `<a href="#${p.anchor}">` : ""}<b>${escText(p.id)} ${escText(p.title)}</b>${
                p.anchor ? " →</a>" : ""
              }${p.critical ? ' <span class="crit">where attention pays</span>' : ""}${
                p.summary ? `<br><span class="dim">${escText(p.summary)}</span>` : ""
              }</li>`
          )
          .join("")}</ul>`
      : "";
    const frame = e.frameHtml.trim()
      ? `<iframe sandbox="allow-same-origin" title="${escText(e.title)}" srcdoc="${escText(shareFrameDoc(e.frameHtml))}"></iframe>`
      : "";
    const body = e.bodyHtml.trim()
      ? frame
        ? `<details class="writeup"><summary>Write-up</summary>${e.bodyHtml}</details>`
        : e.bodyHtml
      : "";
    const Tag = e.depth ? "h3" : "h2";
    return `<section class="explain" id="${e.anchor}">${crumbs}<${Tag}>${escText(e.title)}${
      e.stale ? ' <span class="stale">out of date</span>' : ""
    }</${Tag}>${parts}${frame}${body}</section>`;
  }

  /// What the saved page will hold beyond the document, one line per tree —
  /// shown under the preview, since the preview pane renders the markdown.
  function explainOutline(groups) {
    return (groups || [])
      .filter((g) => g.entries.length)
      .map((g) => {
        const parts = g.entries.length - 1;
        return parts ? `${g.label} — overview + ${parts} part${parts === 1 ? "" : "s"}` : `${g.label} — overview`;
      });
  }

  /// Wrap rendered markdown in a self-contained HTML document — no external
  /// requests, no script, prints cleanly, readable in both OS themes. Mermaid
  /// diagrams arrive already rendered to inline SVG. `explanations` is
  /// `[{ label, entries }]` from `shareExplanationEntries`; with any, the page
  /// opens on a contents list and links the levels by anchor.
  function shareHtmlDocument(title, bodyHtml, explanations = []) {
    const esc = String(title || "clash export")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;");
    const groups = (explanations || []).filter((g) => g.entries && g.entries.length);
    const toc = groups.length
      ? `<nav class="toc"><a href="#x-document">Document</a>${groups
          .map((g) => ` · <a href="#${g.entries[0].anchor}">${escText(g.label)}</a>`)
          .join("")}</nav>`
      : "";
    const explained = groups.map((g) => g.entries.map(explanationHtml).join("\n")).join("\n");
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
  .toc { position: sticky; top: 0; padding: .5rem 0; font-size: .9em;
         background: Canvas; border-bottom: 1px solid rgba(128,128,128,.35); }
  .explain { margin-top: 2.5rem; scroll-margin-top: 3rem; }
  .explain h3 { font-size: 1.2em; }
  .crumbs { font-size: .85em; opacity: .75; }
  .parts { padding-left: 1.1rem; }
  .parts li { margin: .25rem 0; }
  .dim { opacity: .7; font-size: .92em; }
  .crit { font-size: .8em; border: 1px solid currentColor; border-radius: 999px; padding: 0 .5em; }
  .stale { font-size: .6em; font-weight: normal; padding: .1em .6em; border-radius: 999px;
           background: rgba(212,167,44,.2); vertical-align: middle; }
  .explain iframe { display: block; width: 100%; height: 75vh; border: 1px solid rgba(128,128,128,.35);
                    border-radius: 8px; background: Canvas; }
  .writeup { margin-top: .75rem; }
  .writeup > summary { cursor: pointer; opacity: .8; }
  .wf-mermaid { background: #fff; border-radius: 6px; padding: .5rem; text-align: center; overflow-x: auto; }
  @media print { body { margin: 0 auto; } .toc { position: static; } .explain iframe { height: 900px; } }
</style>
</head>
<body>
${toc}<div id="x-document">
${bodyHtml}
</div>
${explained}
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
    SHARE_EXPLAIN_PAIRS,
    SHARE_EXPLAIN_DEPTHS,
    explainLevelsToShare,
    shareAnchor,
    shareExplanationEntries,
    shareFrameDoc,
    explainOutline,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
