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
    { id: "toc", label: "Contents & links", detail: "A linked contents list, ↑ back-links, comments linked to their file's diff" },
    { id: "summary", label: "Summary", detail: "Title, status, branch, iteration counters, PR links" },
    { id: "handoff", label: "Where it stands", detail: "The agent's latest hand-off note: done, left, next" },
    { id: "plan", label: "Plan", detail: "plan.md, verbatim" },
    { id: "explainPlan", label: "Plan explained", detail: "What the plan will do, written for a reader of the code" },
    { id: "explainDiff", label: "Changes explained", detail: "What the change does, part by part" },
    { id: "drift", label: "Plan vs changes", detail: "Where the built change differs from the plan, graded" },
    { id: "timeline", label: "Change rounds", detail: "Your change-request notes, one per iteration" },
    { id: "reviews", label: "Agent reviews", detail: "Each round's verdict and what it published" },
    { id: "annotations", label: "Open comments", detail: "Unresolved diff comments" },
    { id: "diff", label: "Diff", detail: "The current diff, one block per file", heavy: true },
  ];

  /// Sizes and purposes. "Review packet" is the default: everything a
  /// colleague needs to weigh in, without the raw diff. The chat-sized ones
  /// leave the contents out — anchors are dead text in a channel.
  const SHARE_PRESETS = [
    { id: "summary", label: "Summary", sections: ["summary"] },
    {
      id: "status",
      label: "Status update",
      sections: ["summary", "handoff", "reviews", "annotations"],
    },
    {
      id: "explain",
      label: "Explain it",
      sections: ["toc", "summary", "explainPlan", "explainDiff"],
    },
    {
      id: "packet",
      label: "Review packet",
      sections: ["toc", "summary", "plan", "explainDiff", "drift", "timeline", "reviews", "annotations"],
    },
    {
      id: "handover",
      label: "Hand-over",
      sections: ["toc", "summary", "handoff", "plan", "explainDiff", "timeline", "annotations"],
    },
    {
      id: "dossier",
      label: "Full dossier",
      sections: SHARE_SECTIONS.map((s) => s.id),
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
    // Which agent-written documents exist; an unwritten one is listed but
    // disabled, like a planless item's plan.
    has = {},
    slackConfigured = false,
    discordConfigured = false,
    // Who posts each family of destination, and the skill the session route
    // uses when one is named. Both come straight from Settings: this model
    // renders the human's choice, it does not make one.
    chatTransport = "agent",
    chatSkill = "",
    // `{ primary, jira: {...}, linear: {...} }` — see `trackerDestinations`.
    trackers = {},
    preset = "packet",
  } = {}) {
    const checked = presetSections(preset);
    return {
      presets: SHARE_PRESETS.map((p) => ({ id: p.id, label: p.label })),
      preset: SHARE_PRESETS.some((p) => p.id === preset) ? preset : "packet",
      sections: SHARE_SECTIONS.map((s) => {
        const missing =
          s.id === "plan" ? (hasPlan ? null : "This item has no plan phase") : sectionMissing(s.id, has);
        return {
          ...s,
          checked: checked[s.id] && !missing,
          disabled: !!missing,
          detail: missing || s.detail,
        };
      }),
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
        ...trackerDestinations(trackers),
      ],
    };
  }

  /// The ticket destinations: one per tracker that is configured (its site
  /// or workspace set) or is the primary one — so a fresh install still
  /// offers "Post to Jira…" through an agent session, as before trackers.
  /// Each tracker is `{ configured, clientConfigured, transport, skill }`;
  /// `primary` is `jira` | `linear` | `none`.
  function trackerDestinations({ primary = "jira", jira = {}, linear = {} } = {}) {
    const kinds = [
      ["jira", "Jira", jira],
      ["linear", "Linear", linear],
    ];
    return kinds
      .filter(([id, , t]) => t.configured || id === primary)
      .map(([id, label, t]) =>
        shareDestination(id, `Post to ${label}…`, {
          transport: t.transport || "agent",
          skill: t.skill || "",
          clientConfigured: !!t.clientConfigured,
        })
      );
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
          : "clash posts this one directly, but its credentials are missing — fill them in, or switch this destination to an agent session in Settings → Trackers & sharing",
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

  /// The first ticket key found in any of `texts` (PROJ-123, any case —
  /// branch names carry `ps-1234`), uppercased. Pre-fills the ticket prompt
  /// from the item's title/branch/slug; the backend re-validates.
  function detectTicketKey(...texts) {
    for (const t of texts) {
      const m = /(?:^|[^A-Za-z0-9])([A-Za-z]{2,}[A-Za-z0-9]*-\d+)(?![A-Za-z0-9])/.exec(t || "");
      if (m) return m[1].toUpperCase();
    }
    return "";
  }

  /// Why an agent-written section has nothing to carry, or null when it does.
  /// `has` is unspecified-means-present, so a caller that knows nothing about
  /// a document never hides it.
  function sectionMissing(id, has) {
    const why = {
      handoff: "No hand-off note written yet",
      explainPlan: "The plan has not been explained yet",
      explainDiff: "The changes have not been explained yet",
      drift: "Plan vs changes has not been run yet",
    }[id];
    return why && has && has[id] === false ? why : null;
  }

  /// GitHub's heading anchor, mirroring `heading_slug` in
  /// `application::workflow_share` — the share document's links are written
  /// with that rule, so the page's ids must be made with this one.
  function shareHeadingSlug(text) {
    return String(text || "")
      .toLowerCase()
      .replace(/[^\p{Alphabetic}\p{N}\-_ ]/gu, "")
      .replace(/ /g, "-");
  }

  /// Slugs for a document's headings in order, repeats numbered the way
  /// GitHub does (`plan`, `plan-1`…).
  function shareHeadingSlugs(texts) {
    const seen = new Map();
    return (texts || []).map((t) => {
      const base = shareHeadingSlug(t);
      let slug = base;
      while (seen.has(slug)) {
        seen.set(base, seen.get(base) + 1);
        slug = `${base}-${seen.get(base)}`;
      }
      seen.set(slug, 0);
      return slug;
    });
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

  /// The agent's drawn page as a frame document. The viewer puts it in the
  /// `srcdoc` of a frame sandboxed without scripts, never into its own DOM.
  function shareFrameDoc(html) {
    // The base keeps the drawing's `#node-3` links inside its frame; without
    // it they resolve against the export and load the export into the frame.
    const head =
      '<base href="about:srcdoc"><meta charset="utf-8"><style>:root{color-scheme:light dark}' +
      "body{margin:0;padding:18px;font:13px/1.55 -apple-system,'Segoe UI',system-ui,sans-serif}" +
      "svg{max-width:100%;height:auto}.box,.node{border:1px solid rgba(128,128,128,.4);border-radius:8px;padding:8px 10px}" +
      'a[href^="#node-"]{color:inherit;text-decoration:none;cursor:zoom-in}' +
      'a[href^="#node-"]:hover>*{filter:brightness(1.15)}' +
      "</style>";
    const page = /<head(\s[^>]*)?>/i.test(html)
      ? html.replace(/<head(\s[^>]*)?>/i, (m) => m + head)
      : /<html[\s>]/i.test(html)
        ? html.replace(/<html(\s[^>]*)?>/i, (m) => `${m}<head>${head}</head>`)
        : `<!doctype html><html><head>${head}</head><body>${html}</body></html>`;
    return withBreathingRoom(page);
  }

  /// Appended *last* to every drawn page — in clash's tab and in the saved
  /// page — so it beats the page's own width cap: a page written for a 900px
  /// column otherwise stays a column in a 1600px pane. Prose and drawings
  /// alike take the whole width; the large drawings (marked by
  /// `markWideDrawings`) scale to it, as tall as the pane allows.
  const FRAME_BREATHE_STYLE =
    "html,body{max-width:none!important;width:auto!important}" +
    // A drawn page's tables keep content-sized columns at any width: fixed
    // layouts and stretched widths spread short columns across a wide pane.
    "table{table-layout:auto!important;max-width:100%}td,th{vertical-align:top}" +
    "svg[data-clash-wide]{display:block;width:100%!important;max-width:none!important;" +
    "height:auto!important;max-height:calc(100vh - 36px)}";

  function withBreathingRoom(html) {
    const style = `<style data-clash-breathe>${FRAME_BREATHE_STYLE}</style>`;
    const at = html.search(/<\/body>(?![\s\S]*<\/body>)/i);
    return at < 0 ? html + style : html.slice(0, at) + style + html.slice(at);
  }

  /// Mark a drawn page's large drawings so FRAME_BREATHE_STYLE scales them to
  /// the pane: an outermost `<svg>` whose viewBox is at least 400 units wide.
  /// Icons and inline glyphs stay their size. The saved page's explorer
  /// carries its own copy (`shareViewer` cannot import this one).
  function markWideDrawings(doc) {
    if (!doc) return 0;
    let marked = 0;
    for (const svg of doc.querySelectorAll("svg[viewBox]")) {
      if (svg.parentElement && svg.parentElement.closest("svg")) continue;
      const w = Number(String(svg.getAttribute("viewBox")).trim().split(/[\s,]+/)[2]);
      if (w >= 400) {
        svg.setAttribute("data-clash-wide", "");
        marked++;
      }
    }
    return marked;
  }

  /// One explanation tree as the saved page's explorer reads it:
  /// `{ id, label, levels: { "<node>": level } }`, the overview at `""`.
  /// A part listed by a level but without a level of its own is not stored:
  /// the explorer draws its card from what the level above says about it.
  /// `docs[node]` = `{ bodyHtml, frameHtml, views: { <viewId>: html } }`.
  function shareExplanationChapter(pair, levels, docs) {
    const byNode = {};
    for (const l of levels) {
      const d = (docs && docs[l.node]) || {};
      const parent = levels.find((p) => p.node === parentOf(l.node));
      const listed = parent && (parent.nodes || []).find((n) => n.id === l.node);
      byNode[l.node] = {
        title: l.node ? (listed && listed.title) || l.title || "" : pair.label,
        stale: !!l.stale,
        parts: (l.nodes || []).map((n) => ({
          id: n.id,
          title: n.title || "",
          summary: n.summary || "",
          critical: !!n.critical,
          why: n.why || "",
          isNew: !!n.isNew,
          leaf: !!n.leaf,
          files: n.files || [],
        })),
        edges: (l.edges || []).map((e) => ({ from: e.from, to: e.to, label: e.label || "" })),
        drawing: d.frameHtml && d.frameHtml.trim() ? shareFrameDoc(d.frameHtml) : "",
        views: (l.views || [])
          .map((v) => ({
            id: v.id,
            title: v.title || v.kind || v.id,
            why: v.why || "",
            html: (d.views && d.views[v.id]) || "",
          }))
          .filter((v) => v.html.trim())
          .map((v) => ({ ...v, html: shareFrameDoc(v.html) })),
        writeup: d.bodyHtml || "",
      };
    }
    return { id: pair.target, label: pair.label, levels: byNode };
  }

  /// What the saved page will hold beyond the document, one line per tree —
  /// shown under the preview, since the preview pane renders the markdown.
  function explainOutline(chapters) {
    return (chapters || [])
      .filter((c) => Object.keys(c.levels || {}).length)
      .map((c) => {
        const parts = Object.keys(c.levels).length - 1;
        return parts ? `${c.label} — overview + ${parts} part${parts === 1 ? "" : "s"}` : `${c.label} — overview`;
      });
  }

  /// The saved page's explorer. Serialized into the page with `toString`, so
  /// it may use nothing from this module: it reads the `#x-data` JSON and the
  /// page's fixed ids. A level's drawing fills the stage; a click on one of
  /// its parts zooms into that part (its own level, or a card of what is
  /// known about it), and zooming out shrinks it back into its box.
  function shareViewer() {
    "use strict";
    var doc = document;
    var root = doc.documentElement;
    var DATA = JSON.parse(doc.getElementById("x-data").textContent);
    var chapters = DATA.chapters || [];
    if (!chapters.length) return;
    root.classList.add("js");
    var reduced = !!(window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches);
    var $ = function (id) {
      return doc.getElementById(id);
    };
    var esc = function (s) {
      return String(s == null ? "" : s).replace(/[&<>"']/g, function (c) {
        return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
      });
    };
    var parentOf = function (n) {
      var i = String(n || "").lastIndexOf(".");
      return i < 0 ? "" : n.slice(0, i);
    };
    var depthOf = function (n) {
      return n ? String(n).split(".").length : 0;
    };
    var partIdOf = function (t) {
      var m = /^\s*(\d+(?:\.\d+){0,2})\b\.?/.exec(String(t || ""));
      return m ? m[1] : null;
    };
    var chapterOf = function (id) {
      return chapters.filter(function (c) {
        return c.id === id;
      })[0] || null;
    };
    function partOf(ch, id) {
      var p = ch.levels[parentOf(id)];
      return p ? (p.parts || []).filter(function (x) {
        return x.id === id;
      })[0] || null : null;
    }
    function titleOf(ch, id) {
      if (!id) return ch.label;
      var p = partOf(ch, id);
      return id + " " + ((p && p.title) || (ch.levels[id] && ch.levels[id].title) || "");
    }

    var stage = $("x-stage");
    var panel = $("x-panel");
    var docEl = $("x-document");
    var explorer = $("x-explorer");
    var state = null;
    var current = null; // the layer on stage
    var seq = 0;
    var pendingRect = null;

    // ---- theme -------------------------------------------------------------
    var themes = ["auto", "light", "dark"];
    var theme = "auto";
    var themeBtn = $("x-theme");
    // A page exported in one clash theme keeps it: there is nothing to toggle.
    var fixedPalette = root.hasAttribute("data-palette");
    function setTheme(t) {
      theme = t;
      if (t === "auto") root.removeAttribute("data-theme");
      else root.setAttribute("data-theme", t);
      themeBtn.textContent = t === "light" ? "☀" : t === "dark" ? "☾" : "◐";
      themeBtn.title = "Theme: " + t;
      try {
        localStorage.setItem("clash-share-theme", t);
      } catch (e) {
        /* storage blocked: the pick lasts this visit */
      }
    }
    if (fixedPalette) {
      themeBtn.hidden = true;
    } else {
      try {
        theme = localStorage.getItem("clash-share-theme") || "auto";
      } catch (e) {
        theme = "auto";
      }
      setTheme(themes.indexOf(theme) < 0 ? "auto" : theme);
      themeBtn.onclick = function () {
        setTheme(themes[(themes.indexOf(theme) + 1) % themes.length]);
      };
    }

    // ---- chapters ----------------------------------------------------------
    var chapNav = $("x-chapters");
    [{ id: "document", label: "Document" }].concat(chapters).forEach(function (c) {
      var b = doc.createElement("button");
      b.type = "button";
      b.dataset.chapter = c.id;
      b.textContent = c.label;
      b.onclick = function () {
        go({ chapter: c.id, node: "", angle: "" });
      };
      chapNav.appendChild(b);
    });

    $("x-read").onclick = function () {
      root.classList.toggle("panel-closed");
    };
    $("x-out").onclick = function () {
      zoomOut();
    };

    // ---- routing -----------------------------------------------------------
    function hashOf(t) {
      if (t.chapter === "document") return "document";
      return t.chapter + (t.node ? "/" + t.node : "") + (t.angle ? "~" + t.angle : "");
    }
    function parse(hash) {
      var raw = decodeURIComponent(String(hash || "").replace(/^#/, ""));
      var m = /^([a-z-]+)(?:\/([\d.]+))?(?:~([a-z0-9-]+))?$/.exec(raw);
      var ch = m && chapterOf(m[1]);
      if (ch) {
        var node = m[2] || "";
        if (node && !ch.levels[node] && !partOf(ch, node)) node = "";
        return { chapter: ch.id, node: node, angle: m[3] || "" };
      }
      if (raw === "document") return { chapter: "document", node: "", angle: "" };
      if (raw && docEl.querySelector("#" + CSS.escape(raw))) return { chapter: "document", node: "", angle: "", anchor: raw };
      return { chapter: chapters[0].id, node: "", angle: "" };
    }
    function go(t, rect) {
      pendingRect = rect || null;
      var h = "#" + hashOf(t);
      if (location.hash === h) show(t);
      else location.hash = h;
    }
    window.addEventListener("hashchange", function () {
      show(parse(location.hash));
    });

    function zoomOut() {
      if (!state || state.chapter === "document" || !state.node) return;
      go({ chapter: state.chapter, node: parentOf(state.node), angle: "" });
    }
    function zoomTo(id, rect) {
      go({ chapter: state.chapter, node: id, angle: "" }, rect);
    }
    function sibling(step) {
      if (!state || !state.node) return;
      var ch = chapterOf(state.chapter);
      var parts = (ch.levels[parentOf(state.node)] || {}).parts || [];
      var ids = parts.map(function (p) {
        return p.id;
      });
      var next = ids[ids.indexOf(state.node) + step];
      if (next) go({ chapter: state.chapter, node: next, angle: "" });
    }

    // ---- the stage's layers ------------------------------------------------
    function layer(kind) {
      var l = doc.createElement("div");
      l.className = "layer " + kind;
      l.style.opacity = "0";
      stage.appendChild(l);
      l.ready = Promise.resolve();
      return l;
    }
    function frameLayer(html, title) {
      var l = layer("frame");
      var f = doc.createElement("iframe");
      f.setAttribute("sandbox", "allow-same-origin");
      f.setAttribute("title", title);
      l.appendChild(f);
      l.frame = f;
      l.ready = new Promise(function (resolve) {
        var done = false;
        var finish = function () {
          if (done) return;
          done = true;
          hookFrame(f);
          resolve();
        };
        f.addEventListener("load", finish);
        setTimeout(finish, 1200);
      });
      f.srcdoc = html;
      return l;
    }
    // The drawing's parts are links (`#node-3`): a click zooms into that part.
    function hookFrame(f) {
      var d = null;
      try {
        d = f.contentDocument;
      } catch (e) {
        d = null;
      }
      if (!d || d.__clashHooked) return;
      d.__clashHooked = true;
      // The page's large drawings take the stage's width (same rule as
      // markWideDrawings in this module, which this script cannot import).
      Array.prototype.forEach.call(d.querySelectorAll("svg[viewBox]"), function (svg) {
        if (svg.parentElement && svg.parentElement.closest("svg")) return;
        var w = Number(String(svg.getAttribute("viewBox")).trim().split(/[\s,]+/)[2]);
        if (w >= 400) svg.setAttribute("data-clash-wide", "");
      });
      d.addEventListener(
        "click",
        function (ev) {
          var a = ev.target && ev.target.closest ? ev.target.closest("a") : null;
          if (!a) return;
          var raw = a.getAttribute("href") || a.getAttribute("xlink:href") || "";
          var m = /^#node-(\d+(?:\.\d+){0,2})$/.exec(raw);
          if (m) {
            ev.preventDefault();
            var ch = chapterOf(state.chapter);
            if (ch.levels[m[1]] || partOf(ch, m[1])) zoomTo(m[1], rectIn(f, a));
            return;
          }
          if (raw.charAt(0) === "#") return;
          ev.preventDefault();
          if (/^(https?:|mailto:)/i.test(raw)) window.open(raw, "_blank", "noopener");
        },
        true
      );
      d.addEventListener("keydown", onKey);
    }
    function stageRect() {
      return stage.getBoundingClientRect();
    }
    function rectIn(f, el) {
      var fr = f.getBoundingClientRect();
      var r = el.getBoundingClientRect();
      var s = stageRect();
      return { x: fr.left - s.left + r.left, y: fr.top - s.top + r.top, w: r.width, h: r.height };
    }
    function rectOfPart(l, id) {
      if (!l) return null;
      var el = null;
      if (l.frame) {
        try {
          el = l.frame.contentDocument.querySelector('a[href="#node-' + id + '"]');
        } catch (e) {
          el = null;
        }
        if (el) {
          var r = rectIn(l.frame, el);
          return r.w && r.h ? r : null;
        }
        return null;
      }
      el = l.querySelector('[data-part="' + id + '"]');
      if (!el) return null;
      var a = el.getBoundingClientRect();
      var s = stageRect();
      return { x: a.left - s.left, y: a.top - s.top, w: a.width, h: a.height };
    }
    function highlight(id, on) {
      if (!current || !current.frame) {
        var card = current && current.querySelector('[data-part="' + id + '"]');
        if (card) card.classList.toggle("hot", on);
        return;
      }
      try {
        var el = current.frame.contentDocument.querySelector('a[href="#node-' + id + '"]');
        if (el) el.style.filter = on ? "drop-shadow(0 0 5px #e8a33d) drop-shadow(0 0 2px #e8a33d)" : "";
      } catch (e) {
        /* no access: no highlight */
      }
    }

    // Zoom: the old layer grows until the part's box fills the stage while the
    // new one grows out of that box — and the reverse on the way out.
    function swap(next, how, token) {
      var prev = current;
      return next.ready.then(function () {
        if (token !== seq) {
          next.remove();
          return;
        }
        current = next;
        var S = stageRect();
        var W = S.width;
        var H = S.height;
        var r = how && how.rect;
        if (how && how.type === "out" && how.child) r = rectOfPart(next, how.child);
        var animate = prev && !reduced && how && how.type !== "none";
        if (!animate) {
          next.style.opacity = "1";
          next.style.transform = "none";
          if (prev) prev.remove();
          return;
        }
        if (!r || !r.w || !r.h) r = { x: W * 0.35, y: H * 0.35, w: W * 0.3, h: H * 0.3 };
        var k = Math.min(W / r.w, H / r.h);
        var big =
          "translate(" + (W / 2 - (r.x + r.w / 2) * k) + "px," + (H / 2 - (r.y + r.h / 2) * k) + "px) scale(" + k + ")";
        var s = Math.min(r.w / W, r.h / H);
        var small = "translate(" + (r.x + r.w / 2 - (W / 2) * s) + "px," + (r.y + r.h / 2 - (H / 2) * s) + "px) scale(" + s + ")";
        var from = "none";
        var prevTo = "none";
        if (how.type === "in") {
          from = small;
          prevTo = big;
        } else if (how.type === "out") {
          from = big;
          prevTo = small;
        } else {
          from = "translateX(" + (how.dir || 1) * 40 + "px)";
          prevTo = "translateX(" + -(how.dir || 1) * 40 + "px)";
        }
        stage.classList.add("moving");
        next.style.transition = "none";
        next.style.transform = from;
        next.style.opacity = "0";
        void next.offsetWidth;
        next.style.transition = "";
        prev.style.transition = "";
        requestAnimationFrame(function () {
          next.style.transform = "none";
          next.style.opacity = "1";
          prev.style.transform = prevTo;
          prev.style.opacity = "0";
        });
        setTimeout(function () {
          prev.remove();
          stage.classList.remove("moving");
        }, 560);
      });
    }

    // ---- what a level and a part look like ---------------------------------
    function badges(p) {
      return (
        (p.critical ? '<span class="badge crit" title="' + esc(p.why) + '">● where attention pays</span>' : "") +
        (p.isNew ? '<span class="badge new">new</span>' : "")
      );
    }
    function mapLayer(ch, node, lvl) {
      var l = layer("map");
      var parts = lvl.parts || [];
      var cards = parts
        .map(function (p) {
          var deeper = !!ch.levels[p.id];
          var to = (lvl.edges || [])
            .filter(function (e) {
              return e.from === p.id;
            })
            .map(function (e) {
              return e.to;
            });
          return (
            '<button type="button" class="map-card' +
            (p.critical ? " crit" : "") +
            (p.isNew ? " new" : "") +
            (deeper ? " deeper" : "") +
            '" data-part="' +
            esc(p.id) +
            '"><span class="num">' +
            esc(p.id) +
            "</span><b>" +
            esc(p.title) +
            "</b><span>" +
            esc(p.summary) +
            "</span>" +
            (to.length ? '<i>→ ' + esc(to.join(", ")) + "</i>" : "") +
            "</button>"
          );
        })
        .join("");
      l.innerHTML =
        '<div class="map-wrap"><h2>' +
        esc(node ? titleOf(ch, node) : ch.label) +
        "</h2>" +
        (cards ? '<div class="map-grid">' + cards + "</div>" : '<p class="dim">Nothing is drawn at this level — the write-up says it all.</p>') +
        "</div>";
      l.addEventListener("click", function (ev) {
        var b = ev.target.closest ? ev.target.closest("[data-part]") : null;
        if (b) zoomTo(b.dataset.part, rectOfPart(l, b.dataset.part));
      });
      return l;
    }
    function connections(ch, id) {
      var lvl = ch.levels[parentOf(id)] || {};
      var edges = lvl.edges || [];
      var name = function (x) {
        var p = partOf(ch, x);
        return esc(x) + " " + esc((p && p.title) || "");
      };
      var rows = edges
        .filter(function (e) {
          return e.to === id && e.from !== id;
        })
        .map(function (e) {
          return '<button type="button" class="edge" data-go="' + esc(e.from) + '"><b>' + name(e.from) + "</b> <i>→ " + esc(e.label || "talks to it") + " →</i> this part</button>";
        })
        .concat(
          edges
            .filter(function (e) {
              return e.from === id && e.to !== id;
            })
            .map(function (e) {
              return '<button type="button" class="edge" data-go="' + esc(e.to) + '">this part <i>→ ' + esc(e.label || "talks to") + " →</i> <b>" + name(e.to) + "</b></button>";
            })
        );
      return rows.join("");
    }
    function partLayer(ch, id) {
      var p = partOf(ch, id) || { id: id, title: "", summary: "", files: [] };
      var l = layer("card");
      var links = connections(ch, id);
      l.innerHTML =
        '<div class="part-card">' +
        '<div class="kicker">Part of ' +
        esc(titleOf(ch, parentOf(id))) +
        "</div><h2>" +
        esc(id) +
        " · " +
        esc(p.title || "Untitled part") +
        "</h2>" +
        '<div class="badges">' +
        badges(p) +
        '<span class="badge">' +
        (p.leaf ? "nothing deeper" : "not explained on its own") +
        "</span></div>" +
        '<p class="lead">' +
        esc(p.summary || "The level above gives it no summary.") +
        "</p>" +
        (p.critical && p.why ? "<p><b>Why it matters:</b> " + esc(p.why) + "</p>" : "") +
        (links ? "<h3>How it connects</h3>" + links : "") +
        ((p.files || []).length
          ? "<h3>Where it lives</h3><ul class=\"files\">" +
            p.files
              .map(function (f) {
                return "<li><code>" + esc(f) + "</code></li>";
              })
              .join("") +
            "</ul>"
          : "") +
        '<p class="dim">' +
        (p.leaf
          ? "The explainer judged it small enough that the level above says all there is."
          : "Nobody explained this part on its own; the panel shows what the write-up above says about it.") +
        "</p></div>";
      l.addEventListener("click", function (ev) {
        var b = ev.target.closest ? ev.target.closest("[data-go]") : null;
        if (b) go({ chapter: ch.id, node: b.dataset.go, angle: "" });
      });
      return l;
    }
    // The write-up's section about one part: its heading down to the next
    // heading of the same rank or above.
    function writeupSection(html, id) {
      var box = doc.createElement("div");
      box.innerHTML = html || "";
      var heads = box.querySelectorAll("h2, h3, h4");
      for (var i = 0; i < heads.length; i++) {
        if (partIdOf(heads[i].textContent) !== id) continue;
        var rank = Number(heads[i].tagName.slice(1));
        var out = doc.createElement("div");
        var n = heads[i];
        do {
          out.appendChild(n.cloneNode(true));
          n = n.nextElementSibling;
        } while (n && !(/^H[1-6]$/.test(n.tagName) && Number(n.tagName.slice(1)) <= rank));
        return out.innerHTML;
      }
      return "";
    }

    function renderPanel(ch, node) {
      var lvl = ch.levels[node];
      var p = node ? partOf(ch, node) : null;
      var html = '<div class="kicker">' + esc(ch.label) + (node ? " · level " + depthOf(node) : " · overview") + "</div>";
      html += "<h2>" + esc(node ? titleOf(ch, node) : ch.label) + "</h2>";
      if (lvl && lvl.stale) html += '<span class="badge stale" title="Drawn before the level above was redrawn, or from an older basis">⟳ may be out of date</span>';
      // A part with no level shows its summary on the stage already.
      if (lvl && p && p.summary) html += '<p class="lead">' + esc(p.summary) + "</p>";
      if (lvl) {
        var parts = lvl.parts || [];
        if (parts.length) {
          html += '<h3>Inside <small>click to zoom in</small></h3><ul class="plist">';
          parts.forEach(function (x) {
            var deeper = !!ch.levels[x.id];
            html +=
              '<li><button type="button" data-part="' +
              esc(x.id) +
              '" class="' +
              (x.critical ? "crit" : "") +
              '"><span class="num">' +
              esc(x.id) +
              "</span><span class=\"txt\"><b>" +
              esc(x.title) +
              "</b>" +
              (x.critical ? ' <span class="dot" title="' + esc(x.why) + '">●</span>' : "") +
              (x.isNew ? ' <span class="badge new">new</span>' : "") +
              "<small>" +
              esc(x.summary) +
              "</small></span><em>" +
              (deeper ? "⤢" : "·") +
              "</em></button></li>";
          });
          html += "</ul>";
        }
        if (lvl.writeup) html += '<details class="writeup" open><summary>The write-up</summary><div class="md">' + lvl.writeup + "</div></details>";
      } else {
        var sec = writeupSection((ch.levels[parentOf(node)] || {}).writeup, node);
        html += sec
          ? '<details class="writeup" open><summary>What the write-up says</summary><div class="md">' + sec + "</div></details>"
          : '<p class="dim">The write-up above has no section of its own about this part.</p>';
      }
      panel.innerHTML = html;
      panel.scrollTop = 0;
      Array.prototype.forEach.call(panel.querySelectorAll(".plist button"), function (b) {
        var id = b.dataset.part;
        b.onmouseenter = function () {
          highlight(id, true);
        };
        b.onmouseleave = function () {
          highlight(id, false);
        };
        b.onclick = function () {
          highlight(id, false);
          zoomTo(id, rectOfPart(current, id));
        };
      });
      // A write-up section about a part gets a way into it.
      if (lvl)
        Array.prototype.forEach.call(panel.querySelectorAll(".md h2, .md h3"), function (h) {
          var id = partIdOf(h.textContent);
          if (!id || !(lvl.parts || []).some(function (x) {
            return x.id === id;
          })) return;
          var b = doc.createElement("button");
          b.type = "button";
          b.className = "inline";
          b.textContent = "⤢ zoom in";
          b.onclick = function () {
            zoomTo(id, rectOfPart(current, id));
          };
          h.appendChild(b);
        });
    }

    function renderPath(ch, node) {
      var path = $("x-path");
      path.innerHTML = "";
      var ids = [""];
      if (node) {
        var segs = node.split(".");
        for (var i = 1; i <= segs.length; i++) ids.push(segs.slice(0, i).join("."));
      }
      ids.forEach(function (id, i) {
        if (i) path.appendChild(Object.assign(doc.createElement("i"), { textContent: "›" }));
        var b = doc.createElement("button");
        b.type = "button";
        b.textContent = id ? titleOf(ch, id) : "Overview";
        if (id === node) b.className = "on";
        else
          b.onclick = function () {
            go({ chapter: ch.id, node: id, angle: "" });
          };
        path.appendChild(b);
      });
      $("x-out").disabled = !node;
    }

    function renderAngles(ch, node, lvl, angle) {
      var box = $("x-angles");
      box.innerHTML = "";
      var views = (lvl && lvl.views) || [];
      if (!views.length) return;
      [{ id: "", title: "Map", why: "The parts and how they connect" }].concat(views).forEach(function (v) {
        var b = doc.createElement("button");
        b.type = "button";
        b.textContent = v.title;
        b.title = v.why || "";
        if (v.id === angle) b.className = "on";
        b.onclick = function () {
          go({ chapter: ch.id, node: node, angle: v.id });
        };
        box.appendChild(b);
      });
    }

    function show(t) {
      var prev = state;
      state = t;
      Array.prototype.forEach.call(chapNav.children, function (b) {
        b.classList.toggle("on", b.dataset.chapter === t.chapter);
      });
      if (t.chapter === "document") {
        explorer.hidden = true;
        docEl.hidden = false;
        doc.title = DATA.title;
        var a = t.anchor && docEl.querySelector("#" + CSS.escape(t.anchor));
        if (a) a.scrollIntoView({ block: "start" });
        else window.scrollTo(0, 0);
        return;
      }
      docEl.hidden = true;
      explorer.hidden = false;
      var ch = chapterOf(t.chapter);
      var lvl = ch.levels[t.node] || null;
      var view = lvl && t.angle ? (lvl.views || []).filter(function (v) {
        return v.id === t.angle;
      })[0] : null;
      if (!view) t.angle = "";
      doc.title = (t.node ? titleOf(ch, t.node) : ch.label) + " — " + DATA.title;
      renderPath(ch, t.node);
      renderAngles(ch, t.node, lvl, t.angle);
      renderPanel(ch, t.node);

      var how = { type: "none" };
      if (prev && prev.chapter === t.chapter && prev.chapter !== "document") {
        if (prev.node === t.node) how = { type: "fade", dir: 1 };
        else if (t.node.indexOf(prev.node ? prev.node + "." : "") === 0 && depthOf(t.node) > depthOf(prev.node)) {
          var step = t.node.split(".").slice(0, depthOf(prev.node) + 1).join(".");
          how = { type: "in", rect: pendingRect || rectOfPart(current, step) };
        } else if (prev.node.indexOf(t.node ? t.node + "." : "") === 0 && depthOf(prev.node) > depthOf(t.node)) {
          how = { type: "out", child: prev.node.split(".").slice(0, depthOf(t.node) + 1).join(".") };
        } else {
          how = { type: "fade", dir: prev.node < t.node ? 1 : -1 };
        }
      }
      pendingRect = null;
      var token = ++seq;
      var label = t.node ? titleOf(ch, t.node) : ch.label;
      var next = view
        ? frameLayer(view.html, label + " — " + view.title)
        : lvl && lvl.drawing
          ? frameLayer(lvl.drawing, label)
          : lvl
            ? mapLayer(ch, t.node, lvl)
            : partLayer(ch, t.node);
      swap(next, how, token);
      if (prev && (prev.node !== t.node || prev.chapter !== t.chapter)) root.classList.add("zoomed");
    }

    // ---- keys --------------------------------------------------------------
    function onKey(ev) {
      if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
      var tag = (ev.target && ev.target.tagName) || "";
      if (/^(INPUT|TEXTAREA|SELECT)$/.test(tag)) return;
      if (!state || state.chapter === "document") return;
      if (ev.key === "Escape" || ev.key === "Backspace" || ev.key === "-") {
        ev.preventDefault();
        zoomOut();
      } else if (ev.key === "ArrowRight") {
        sibling(1);
      } else if (ev.key === "ArrowLeft") {
        sibling(-1);
      } else if (ev.key === "r") {
        root.classList.toggle("panel-closed");
      }
    }
    doc.addEventListener("keydown", onKey);
    window.addEventListener("resize", function () {
      if (current) current.style.transform = "none";
    });

    // The document's contents rail follows the reading: the section reaching
    // the top of the window lights its entry.
    var toc = $("x-toc");
    if (toc && window.IntersectionObserver) {
      var links = {};
      Array.prototype.forEach.call(toc.querySelectorAll("a"), function (a) {
        links[a.getAttribute("href").slice(1)] = a;
      });
      var lit = null;
      var spy = new IntersectionObserver(
        function (entries) {
          entries.forEach(function (e) {
            var a = e.isIntersecting && links[e.target.id];
            if (!a || a === lit) return;
            if (lit) lit.classList.remove("on");
            a.classList.add("on");
            lit = a;
          });
        },
        { rootMargin: "-80px 0px -70% 0px" }
      );
      Array.prototype.forEach.call(docEl.querySelectorAll("h2[id], h3[id]"), function (h) {
        if (links[h.id]) spy.observe(h);
      });
    }

    // The drawing comes first: on a window where the panel would leave it
    // under 1100px, the panel starts closed (☰ Read / r opens it).
    if (window.innerWidth && window.innerWidth - 340 < 1100 && window.innerWidth > 900) root.classList.add("panel-closed");
    show(parse(location.hash));
    setTimeout(function () {
      var hint = stage.querySelector(".hint");
      if (hint) hint.classList.add("gone");
    }, 8000);
  }

  /// The explorer as an inline script body. `</` is escaped so no string in
  /// it can close the script element.
  function shareViewerScript() {
    return `(${shareViewer.toString()})();`.replace(/<\//g, "<\\/");
  }

  /// JSON that is safe inside `<script type="application/json">`: no `<` at
  /// all, so nothing in it can close the element.
  function scriptJson(value) {
    return JSON.stringify(value)
      .replace(/</g, "\\u003c")
      .replace(/\u2028/g, "\\u2028")
      .replace(/\u2029/g, "\\u2029");
  }

  /// Colours, base typography and the top bar — shared by both page shapes.
  const THEME_STYLE = `
  :root { color-scheme: light; --bg: #f7f6f3; --bg-panel: #ffffff; --bg-raised: #ffffff; --bg-hover: #efece6;
          --border: #e0dbd2; --fg: #26241f; --fg-dim: #6e6960; --accent: #b0741a; --on-accent: #fff;
          --green: #17803d; --warn: #96700f; --stage: #eeece7; --code: rgba(0,0,0,.05); --shadow: 0 10px 30px rgba(0,0,0,.12); }
  :root[data-theme="dark"] { color-scheme: dark; --bg: #141414; --bg-panel: #1b1b1d; --bg-raised: #1f1f23; --bg-hover: #26262b;
          --border: #2c2c31; --fg: #d4d4d8; --fg-dim: #8a8a93; --accent: #e8a33d; --on-accent: #141414;
          --green: #4ec975; --warn: #d2be78; --stage: #0f1013; --code: rgba(255,255,255,.06); --shadow: 0 10px 30px rgba(0,0,0,.55); }
  @media (prefers-color-scheme: dark) {
    :root:not([data-theme="light"]) { color-scheme: dark; --bg: #141414; --bg-panel: #1b1b1d; --bg-raised: #1f1f23; --bg-hover: #26262b;
          --border: #2c2c31; --fg: #d4d4d8; --fg-dim: #8a8a93; --accent: #e8a33d; --on-accent: #141414;
          --green: #4ec975; --warn: #d2be78; --stage: #0f1013; --code: rgba(255,255,255,.06); --shadow: 0 10px 30px rgba(0,0,0,.55); }
  }
  * { box-sizing: border-box; }
  html, body { margin: 0; background: var(--bg); color: var(--fg); }
  body { font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif; }
  button { font: inherit; color: inherit; }
  a { color: var(--accent); }
  code, pre { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: .9em; }
  pre { padding: .75rem 1rem; border-radius: 6px; overflow-x: auto; background: var(--code); }
  blockquote { margin: 0; padding-left: 1rem; border-left: 3px solid var(--border); color: var(--fg-dim); }
  table { border-collapse: collapse; }
  th, td { border: 1px solid var(--border); padding: .3rem .6rem; vertical-align: top; }
  img, svg { max-width: 100%; }
  .dim { color: var(--fg-dim); }
  [hidden] { display: none !important; }

  .top { position: sticky; top: 0; z-index: 5; display: flex; align-items: center; gap: 14px; height: 52px; padding: 0 16px;
         background: var(--bg-panel); border-bottom: 1px solid var(--border); }
  .top .brand { color: var(--accent); font-weight: 700; font-size: 16px; }
  .top h1 { margin: 0; font-size: 15px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 34vw; }
  .chapters { display: flex; gap: 4px; overflow-x: auto; }
  .chapters button { background: none; border: 0; border-radius: 8px; padding: 6px 12px; cursor: pointer; color: var(--fg-dim); white-space: nowrap; }
  .chapters button:hover { background: var(--bg-hover); color: var(--fg); }
  .chapters button.on { background: var(--accent); color: var(--on-accent); }
  .spacer { flex: 1; }
  .icon { background: none; border: 1px solid var(--border); border-radius: 8px; min-width: 32px; height: 30px; cursor: pointer; }
  .icon:hover { border-color: var(--accent); }
`;

  /// The shared document: the full window wide, a sticky contents rail
  /// beside it, sections that breathe. The mermaid figure rules mirror
  /// `.wf-mmd-*` in style.css (the GUI draws the same markup).
  const DOC_STYLE = `
  .doc { --doc-top: 76px; padding: clamp(24px, 4vw, 56px) clamp(18px, 5vw, 80px) 120px; }
  .doc-plain { --doc-top: 24px; }
  .doc-wrap { display: grid; grid-template-columns: minmax(0, 1fr); gap: clamp(28px, 3.5vw, 64px); align-items: start; }
  .doc-wrap.has-toc { grid-template-columns: 250px minmax(0, 1fr); }
  .doc-toc { position: sticky; top: var(--doc-top); max-height: calc(100vh - var(--doc-top) - 24px); overflow-y: auto;
             padding: 14px 10px; background: var(--bg-panel); border: 1px solid var(--border); border-radius: 14px;
             box-shadow: var(--shadow); font-size: 13px; line-height: 1.4; }
  .doc-toc b { display: block; padding: 2px 10px 8px; font-size: 11px; text-transform: uppercase; letter-spacing: .07em; color: var(--fg-dim); }
  .doc-toc a { display: block; padding: 6px 10px; border-radius: 8px; color: var(--fg-dim); text-decoration: none; border-left: 2px solid transparent; }
  .doc-toc a:hover { background: var(--bg-hover); color: var(--fg); }
  .doc-toc a.l3 { padding-left: 24px; font-size: 12.5px; }
  .doc-toc a.on { color: var(--accent); border-left-color: var(--accent); background: var(--bg-hover); }
  .doc-body { min-width: 0; font-size: 15.5px; line-height: 1.75; }
  .doc-body > :first-child { margin-top: 0; }
  .doc-body > h1:first-child { font-size: clamp(28px, 3.2vw, 42px); line-height: 1.15; letter-spacing: -.01em;
             padding: clamp(24px, 3vw, 40px) clamp(22px, 3vw, 44px); margin: 0 0 36px; border-radius: 20px;
             border: 1px solid var(--border); box-shadow: var(--shadow);
             background: linear-gradient(135deg, color-mix(in srgb, var(--accent) 20%, var(--bg-panel)) 0%, var(--bg-panel) 65%); }
  .doc-body > h1:first-child::before { content: "◫ clash · shared from a workflow item"; display: block; margin-bottom: 12px;
             font-size: 12px; font-weight: 600; letter-spacing: .08em; text-transform: uppercase; color: var(--accent); }
  .doc-body h1, .doc-body h2, .doc-body h3, .doc-body h4 { line-height: 1.25; scroll-margin-top: calc(var(--doc-top) + 8px); }
  .doc-body h2 { font-size: 22px; margin: 64px 0 18px; padding: 12px 18px; border-left: 4px solid var(--accent);
             border-radius: 0 12px 12px 0; background: linear-gradient(90deg, var(--bg-hover), transparent); }
  .doc-body h3 { font-size: 17px; margin: 36px 0 10px; }
  .doc-body h4 { font-size: 15px; margin: 26px 0 8px; color: var(--fg-dim); }
  .doc-body p, .doc-body ul, .doc-body ol { margin: 14px 0; }
  .doc-body li { margin: 7px 0; }
  .doc-body li::marker { color: var(--accent); }
  .doc-body :not(pre) > code { background: var(--code); border: 1px solid var(--border); border-radius: 6px; padding: .08em .4em; }
  .doc-body pre { padding: 18px 22px; border: 1px solid var(--border); border-radius: 14px; line-height: 1.55; }
  .doc-body blockquote { margin: 22px 0; padding: 14px 20px; color: var(--fg); border-left: 4px solid var(--accent); border-radius: 0 12px 12px 0;
             background: color-mix(in srgb, var(--accent) 9%, var(--bg-panel)); }
  .doc-body blockquote > :first-child { margin-top: 0; }
  .doc-body blockquote > :last-child { margin-bottom: 0; }
  /* Columns size to their content: the table is only as wide as it needs
     (never stretched to the page, which spread short columns apart), and a
     table wider than the page scrolls on its own instead of squeezing. */
  .doc-body table { display: block; width: fit-content; max-width: 100%; overflow-x: auto; margin: 22px 0;
             border-collapse: separate; border-spacing: 0; border: 1px solid var(--border); border-radius: 14px;
             font-size: 14px; }
  .doc-body th, .doc-body td { border: 0; border-bottom: 1px solid var(--border); padding: 10px 14px; text-align: left;
             vertical-align: top; }
  .doc-body th + th, .doc-body td + td { border-left: 1px solid var(--border); }
  .doc-body th { white-space: nowrap; }
  .doc-body th { background: var(--bg-hover); font-weight: 600; }
  .doc-body tr:last-child td { border-bottom: 0; }
  .doc-body tbody tr:nth-child(even) td { background: color-mix(in srgb, var(--bg-hover) 45%, transparent); }
  .doc-body hr { border: 0; height: 1px; background: var(--border); margin: 48px 0; }
  .doc-body img { border-radius: 12px; }
  .doc-foot { margin-top: 72px; padding-top: 18px; border-top: 1px solid var(--border); font-size: 12px; color: var(--fg-dim); }

  .wf-mmd { position: relative; margin: 24px 0; border: 1px solid var(--border); border-radius: 16px; overflow: hidden;
            background: var(--bg-panel); box-shadow: var(--shadow); }
  .wf-mmd > input { position: absolute; opacity: 0; pointer-events: none; }
  .wf-mmd-bar { display: flex; gap: 4px; padding: 8px 10px; border-bottom: 1px solid var(--border); background: var(--bg-hover); }
  .wf-mmd-bar label { padding: 3px 12px; border-radius: 999px; font-size: 12px; cursor: pointer; color: var(--fg-dim); }
  .wf-mmd-bar label:hover { color: var(--fg); }
  .wf-mmd > .wf-mmd-d:checked ~ .wf-mmd-bar .wf-mmd-ld,
  .wf-mmd > .wf-mmd-s:checked ~ .wf-mmd-bar .wf-mmd-ls { background: var(--accent); color: var(--on-accent); }
  .wf-mmd > .wf-mmd-d:focus-visible ~ .wf-mmd-bar .wf-mmd-ld,
  .wf-mmd > .wf-mmd-s:focus-visible ~ .wf-mmd-bar .wf-mmd-ls { outline: 2px solid var(--accent); outline-offset: 1px; }
  .wf-mmd .wf-mermaid { margin: 0; padding: clamp(14px, 2vw, 28px); background: var(--bg-panel); text-align: center; overflow-x: auto; }
  /* Each diagram is drawn in both mermaid themes; the page's theme shows one. */
  .wf-mmd .wf-mmd-dark { display: none; }
  :root[data-theme="dark"] .wf-mmd .wf-mmd-dark { display: block; }
  :root[data-theme="dark"] .wf-mmd .wf-mmd-light { display: none; }
  @media (prefers-color-scheme: dark) {
    :root:not([data-theme="light"]) .wf-mmd .wf-mmd-dark { display: block; }
    :root:not([data-theme="light"]) .wf-mmd .wf-mmd-light { display: none; }
  }
  .wf-mmd .wf-mmd-src { display: none; margin: 0; border: 0; border-radius: 0; }
  .wf-mmd > .wf-mmd-s:checked ~ .wf-mermaid { display: none !important; }
  .wf-mmd > .wf-mmd-s:checked ~ .wf-mmd-src { display: block; }

  @media (max-width: 900px) {
    .doc-wrap.has-toc { grid-template-columns: minmax(0, 1fr); }
    .doc-toc { position: static; max-height: none; }
  }
  @media print {
    .top, .doc-toc { display: none !important; }
    .doc-wrap.has-toc { grid-template-columns: minmax(0, 1fr); }
    .doc-body > h1:first-child, .doc-toc, .wf-mmd { box-shadow: none; }
  }
`;

  /// The h2/h3 headings of a rendered document that carry an id, in order:
  /// `[{ id, level, text }]` with `text` still HTML-escaped, tags removed.
  function shareDocOutline(bodyHtml) {
    const out = [];
    const re = /<h([23])\b[^>]*\bid="([^"]+)"[^>]*>([\s\S]*?)<\/h\1>/gi;
    let m;
    while ((m = re.exec(String(bodyHtml || "")))) {
      const text = m[3].replace(/<[^>]*>/g, "").replace(/\s+/g, " ").trim();
      if (text) out.push({ id: m[2], level: Number(m[1]), text });
    }
    return out;
  }

  /// The document section both page shapes share: a contents rail when the
  /// document has at least two sections, the body beside it.
  function shareDocSection(bodyHtml, { plain = false } = {}) {
    const outline = shareDocOutline(bodyHtml);
    const toc =
      outline.length >= 2
        ? `<nav class="doc-toc" id="x-toc" aria-label="Contents"><b>Contents</b>${outline
            .map((h) => `<a class="l${h.level}" href="#${h.id.replace(/&/g, "&amp;")}">${h.text}</a>`)
            .join("")}</nav>`
        : "";
    const foot = plain ? `<footer class="doc-foot">Exported from a clash workflow item.</footer>` : "";
    return (
      `<section class="doc${plain ? " doc-plain" : ""}" id="x-document"><div class="doc-wrap${toc ? " has-toc" : ""}">` +
      `${toc}<article class="md doc-body">\n${bodyHtml}\n${foot}</article></div></section>`
    );
  }

  /// The explorer around the explanations — only on a page that has some.
  const EXPLORER_STYLE = `
  .explorer { display: flex; flex-direction: column; height: calc(100vh - 52px); }
  .toolbar { display: flex; align-items: center; gap: 10px; padding: 8px 14px; border-bottom: 1px solid var(--border); background: var(--bg-panel); flex-wrap: wrap; }
  .tb { background: var(--bg-raised); border: 1px solid var(--border); border-radius: 8px; padding: 4px 12px; cursor: pointer; }
  .tb:hover:not(:disabled) { border-color: var(--accent); }
  .tb:disabled { opacity: .4; cursor: default; }
  .path { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; min-width: 0; }
  .path button { background: none; border: 0; padding: 3px 6px; border-radius: 6px; cursor: pointer; color: var(--accent); }
  .path button:hover { background: var(--bg-hover); }
  .path button.on { color: var(--fg); font-weight: 600; cursor: default; background: none; }
  .path i { font-style: normal; color: var(--fg-dim); }
  .angles { display: inline-flex; border: 1px solid var(--border); border-radius: 8px; overflow: hidden; }
  .angles:empty { display: none; }
  .angles button { background: var(--bg-raised); border: 0; border-right: 1px solid var(--border); padding: 4px 12px; cursor: pointer; font-size: 12px; }
  .angles button:last-child { border-right: 0; }
  .angles button.on { background: var(--accent); color: var(--on-accent); }

  .body { flex: 1; display: flex; min-height: 0; }
  .stage { position: relative; flex: 1; min-width: 0; overflow: hidden; background: var(--stage); }
  .layer { position: absolute; inset: 0; transform-origin: 0 0;
           transition: transform .5s cubic-bezier(.4,0,.2,1), opacity .5s cubic-bezier(.4,0,.2,1); overflow: auto; }
  .stage.moving .layer { pointer-events: none; }
  .layer iframe { display: block; width: 100%; height: 100%; border: 0; background: transparent; }
  .hint { position: absolute; left: 14px; bottom: 12px; z-index: 2; font-size: 12px; color: var(--fg-dim);
          background: var(--bg-panel); border: 1px solid var(--border); border-radius: 999px; padding: 3px 12px;
          box-shadow: var(--shadow); transition: opacity .6s; pointer-events: none; }
  .zoomed .hint, .hint.gone { opacity: 0; }
  .map-wrap, .part-card { max-width: 60rem; margin: 0 auto; padding: 28px 24px; }
  .map-wrap h2 { margin: 0 0 16px; font-size: 20px; }
  .map-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 14px; }
  .map-card { text-align: left; background: var(--bg-raised); border: 1px solid var(--border); border-radius: 12px;
              padding: 12px 14px; cursor: zoom-in; display: flex; flex-direction: column; gap: 4px; box-shadow: var(--shadow); }
  .map-card:hover, .map-card.hot { border-color: var(--accent); }
  .map-card.crit { border-width: 2px; border-color: var(--accent); }
  .map-card.new b { color: var(--green); }
  .map-card .num { font-size: 11px; color: var(--fg-dim); font-weight: 600; }
  .map-card span { color: var(--fg-dim); font-size: 12.5px; }
  .map-card i { font-style: normal; font-size: 11px; color: var(--fg-dim); }
  .part-card { max-width: 44rem; margin-top: 4vh; background: var(--bg-raised); border: 1px solid var(--border); border-radius: 16px; box-shadow: var(--shadow); }
  .part-card h2 { margin: 2px 0 8px; font-size: 22px; }
  .part-card h3 { font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--fg-dim); margin: 18px 0 6px; }
  .kicker { font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--fg-dim); }
  .lead { font-size: 15px; }
  .badges { display: flex; flex-wrap: wrap; gap: 6px; }
  .badge { font-size: 11px; border: 1px solid var(--border); border-radius: 999px; padding: 1px 8px; color: var(--fg-dim); white-space: nowrap; }
  .badge.crit { border-color: var(--accent); color: var(--accent); }
  .badge.new { border-color: var(--green); color: var(--green); }
  .badge.stale { border-color: var(--warn); color: var(--warn); }
  .edge { display: block; width: 100%; text-align: left; background: none; border: 1px solid var(--border); border-radius: 8px;
          padding: 6px 10px; margin: 4px 0; cursor: pointer; }
  .edge:hover { border-color: var(--accent); }
  .edge i { font-style: normal; color: var(--fg-dim); }
  .files { margin: 0; padding-left: 1.1rem; }

  .panel { width: 340px; flex: none; overflow-y: auto; padding: 18px 20px 40px; background: var(--bg-panel); border-left: 1px solid var(--border); }
  .panel-closed .panel { display: none; }
  .panel h2 { margin: 2px 0 6px; font-size: 18px; line-height: 1.3; }
  .panel > h3 { font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--fg-dim); margin: 18px 0 6px; }
  .panel > h3 small { text-transform: none; letter-spacing: 0; font-weight: normal; }
  .plist { list-style: none; margin: 0; padding: 0; }
  .plist button { display: flex; gap: 10px; width: 100%; text-align: left; background: none; border: 1px solid transparent;
                  border-radius: 10px; padding: 7px 8px; cursor: zoom-in; align-items: flex-start; }
  .plist button:hover { background: var(--bg-hover); border-color: var(--border); }
  .plist .num { min-width: 30px; font-size: 12px; font-weight: 700; color: var(--fg-dim); padding-top: 1px; }
  .plist .crit .num, .plist .dot { color: var(--accent); }
  .plist .txt { flex: 1; min-width: 0; }
  .plist small { display: block; color: var(--fg-dim); font-size: 12px; line-height: 1.45; }
  .plist em { font-style: normal; color: var(--accent); }
  .writeup { margin-top: 18px; border-top: 1px solid var(--border); padding-top: 10px; }
  .writeup > summary { cursor: pointer; font-size: 11px; text-transform: uppercase; letter-spacing: .06em; color: var(--fg-dim); }
  .md { font-size: 13.5px; overflow-wrap: anywhere; }
  .md pre code { overflow-wrap: normal; }
  .md h1 { font-size: 17px; }
  .md h2 { font-size: 15px; margin-top: 1.4em; }
  .md h3 { font-size: 14px; }
  .md table { display: block; width: fit-content; max-width: 100%; overflow-x: auto; }
  .md th, .md td { vertical-align: top; }
  .inline { margin-left: 8px; font-size: 11px; font-weight: normal; border: 1px solid var(--border); border-radius: 10px;
            background: none; padding: 0 8px; cursor: zoom-in; color: var(--accent); vertical-align: middle; }
  footer { display: none; }
  .noscript { max-width: 52rem; margin: 16px auto 0; padding: 10px 24px; color: var(--fg-dim); }

  @media (max-width: 900px) {
    .top h1 { display: none; }
    .explorer { height: auto; }
    .body { flex-direction: column; }
    .stage { height: 70vh; flex: none; }
    .panel { width: auto; border-left: 0; border-top: 1px solid var(--border); }
    .panel-closed .panel { display: block; }
  }
  @media (prefers-reduced-motion: reduce) { .layer { transition: none; } }
  @media print {
    .top, .explorer { display: none !important; }
    .doc { display: block !important; }
  }`;

  const PAGE_STYLE = THEME_STYLE + DOC_STYLE + EXPLORER_STYLE;

  const hexRgb = (hex) => {
    const m = /^#?([0-9a-f]{6})$/i.exec(String(hex || "").trim());
    if (!m) return null;
    const n = parseInt(m[1], 16);
    return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
  };
  const hexMix = (hex, amount) => {
    const rgb = hexRgb(hex);
    if (!rgb) return hex;
    const target = amount > 0 ? 255 : 0;
    const t = Math.abs(amount);
    return `#${rgb.map((v) => Math.round(v + (target - v) * t).toString(16).padStart(2, "0")).join("")}`;
  };
  const hexLuma = (hex) => {
    const rgb = hexRgb(hex);
    return rgb ? (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) / 255 : 0;
  };

  /// A saved page's colours in one clash theme. `theme` is a `THEMES` entry of
  /// app.js (`{ id, label, dark, ui, ansi? }`); the result is what
  /// `shareHtmlDocument`'s `palette` option takes. Same derivations as
  /// app.js `applyTheme` (`--on-accent` from the accent's luminance).
  function shareThemePalette(id, theme) {
    const u = (theme && theme.ui) || {};
    const dark = !!(theme && theme.dark);
    const bg = u.bg || (dark ? "#141414" : "#fbfaf8");
    const accent = u.accent || (dark ? "#e8a33d" : "#b0741a");
    return {
      id: String(id || "theme"),
      label: (theme && theme.label) || String(id || "theme"),
      dark,
      vars: {
        bg,
        "bg-panel": u["bg-sidebar"] || bg,
        "bg-raised": u["bg-raised"] || bg,
        "bg-hover": u["bg-hover"] || bg,
        border: u.border || (dark ? "#2c2c31" : "#e0dbd2"),
        fg: u.fg || (dark ? "#d4d4d8" : "#26241f"),
        "fg-dim": u["fg-dim"] || (dark ? "#8a8a93" : "#6e6960"),
        accent,
        "on-accent": hexLuma(accent) > 0.55 ? "#14161a" : "#ffffff",
        green: u.green || (dark ? "#4ec975" : "#17803d"),
        warn: u["st-waiting"] || (theme && theme.ansi && theme.ansi[3]) || accent,
        stage: hexMix(bg, dark ? -0.25 : -0.04),
        code: dark ? "rgba(255,255,255,.06)" : "rgba(0,0,0,.05)",
        shadow: dark ? "0 10px 30px rgba(0,0,0,.55)" : "0 10px 30px rgba(0,0,0,.12)",
      },
    };
  }

  // Values land inside a <style>: anything that could close the rule or the
  // element is dropped rather than escaped.
  const CSS_VALUE = /^[#\w\s.,()%-]+$/;

  /// The `<html>` attributes and the style rule of a fixed palette, or
  /// nothing (the page then follows the reader's OS and offers its toggle).
  /// `data-theme` picks the matching mermaid variant and the frames'
  /// colour scheme; the rule comes after PAGE_STYLE so it wins.
  function paletteParts(palette) {
    if (!palette || !palette.vars) return { attrs: "", style: "" };
    const decls = Object.entries(palette.vars)
      .filter(([k, v]) => /^[a-z-]+$/.test(k) && CSS_VALUE.test(String(v)))
      .map(([k, v]) => `--${k}: ${v};`)
      .join(" ");
    const scheme = palette.dark ? "dark" : "light";
    const id = String(palette.id || "theme").replace(/[^\w-]/g, "");
    return {
      attrs: ` data-palette="${id}" data-theme="${scheme}"`,
      style: `\n  html:root[data-palette] { color-scheme: ${scheme}; ${decls} }`,
    };
  }

  /// Wrap rendered markdown in a self-contained HTML document — no external
  /// requests, readable in both OS themes. Mermaid diagrams arrive already
  /// rendered to inline SVG. `chapters` come from `shareExplanationChapter`;
  /// with any, the page opens on an explorer of the explanations — the
  /// drawing fills the screen and a click zooms into the part — driven by an
  /// inline script of its own (the drawings stay script-less frames). Without
  /// explanations it is a plain document with no script. `palette` (from
  /// `shareThemePalette`) fixes the page to one clash theme; without it the
  /// page follows the reader's OS.
  function shareHtmlDocument(title, bodyHtml, chapters = [], { palette = null } = {}) {
    const esc = String(title || "clash export")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;");
    const kept = (chapters || []).filter((c) => c && Object.keys(c.levels || {}).length);
    const pal = paletteParts(palette);
    if (!kept.length) return plainHtmlDocument(esc, bodyHtml, pal);
    const data = { title: String(title || "clash export"), chapters: kept };
    return `<!doctype html>
<html lang="en"${pal.attrs}>
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${esc}</title>
<style>${PAGE_STYLE}${pal.style}
</style>
</head>
<body>
<header class="top"><span class="brand">◫</span><h1>${esc}</h1><nav class="chapters" id="x-chapters"></nav><span class="spacer"></span><button type="button" class="icon" id="x-theme" aria-label="Theme">◐</button></header>
<noscript><p class="noscript">The explanations of this page are explored with JavaScript, which is off — the document below reads without it.</p></noscript>
<div class="explorer" id="x-explorer" hidden>
<div class="toolbar"><button type="button" class="tb" id="x-out" title="Zoom out (Esc)">⤺ Zoom out</button><nav class="path" id="x-path"></nav><span class="spacer"></span><div class="angles" id="x-angles"></div><button type="button" class="tb" id="x-read" title="Show or hide the reading panel (r)">☰ Read</button></div>
<div class="body"><div class="stage" id="x-stage"><div class="hint">Click a box to zoom in · Esc zooms out · ← → next part</div></div><aside class="panel" id="x-panel"></aside></div>
</div>
${shareDocSection(bodyHtml)}
<script type="application/json" id="x-data">${scriptJson(data)}</script>
<script>${shareViewerScript()}</script>
</body>
</html>
`;
  }

  /// The document alone, no script: the same full-width design as the
  /// explorer page's document chapter, its theme following the OS unless a
  /// palette fixes it.
  function plainHtmlDocument(esc, bodyHtml, pal) {
    return `<!doctype html>
<html lang="en"${pal.attrs}>
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>${esc}</title>
<style>${THEME_STYLE + DOC_STYLE}${pal.style}
</style>
</head>
<body>
${shareDocSection(bodyHtml, { plain: true })}
</body>
</html>
`;
  }

  const api = {
    shareDestination,
    trackerDestinations,
    SHARE_SECTIONS,
    SHARE_PRESETS,
    presetSections,
    shareModel,
    sectionsFromChecks,
    shareHeadingSlug,
    shareHeadingSlugs,
    shareHtmlDocument,
    shareThemePalette,
    shareDocOutline,
    shareDocSection,
    detectTicketKey,
    SHARE_EXPLAIN_PAIRS,
    SHARE_EXPLAIN_DEPTHS,
    explainLevelsToShare,
    shareAnchor,
    shareExplanationChapter,
    shareFrameDoc,
    withBreathingRoom,
    markWideDrawings,
    explainOutline,
    shareViewer,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
