// Drill-down explanations — the pure half of the explain tab's navigation.
// The tree itself (which parts are expanded, stale, diveable) is derived in
// Rust (`application::explain_tree`) and arrives as a list of levels; this
// module only answers display questions about it. No DOM here: app.js
// renders, gui/tests/wf_explain_tree.test.js exercises it.
(function () {
  /// The tree target a document pair drills into.
  function explainTreeTarget(which) {
    return { plan: "explain-plan", diff: "explain-diff", drift: "drift" }[which] || null;
  }

  function explainLevel(tree, node) {
    return (tree || []).find((l) => l.node === (node || "")) || null;
  }

  /// `3.2.1` → `3.2`, `3` → `""` (the overview).
  function explainParentOf(node) {
    const i = String(node || "").lastIndexOf(".");
    return i < 0 ? "" : node.slice(0, i);
  }

  /// Overview › 3 Retry wrapper › 3.2 Backoff — one crumb per level from the
  /// overview down to `node`, titled by how the parent level lists the part.
  function explainBreadcrumb(tree, node) {
    const crumbs = [{ node: "", label: "Overview" }];
    if (!node) return crumbs;
    const parts = node.split(".");
    for (let i = 1; i <= parts.length; i++) {
      const id = parts.slice(0, i).join(".");
      const parent = explainLevel(tree, explainParentOf(id));
      const listed = parent && (parent.nodes || []).find((n) => n.id === id);
      crumbs.push({ node: id, label: `${id} ${(listed && listed.title) || ""}`.trim() });
    }
    return crumbs;
  }

  /// The dive in flight on `node` of `target`, or null.
  function runningDive(item, target, node) {
    const list = (item && item.explainers) || [];
    return list.find((e) => e.target === target && e.node === node && e.running) || null;
  }

  /// What a part's chip offers: `open` (its level is there), `stale` (there,
  /// but drawn before its parent or from another basis), `diving` (being
  /// written), `dive` (can be written) or `none` (a leaf).
  function explainPartState(item, target, part) {
    if (runningDive(item, target, part.id)) return "diving";
    if (part.expanded) return part.stale ? "stale" : "open";
    return part.diveable ? "dive" : "none";
  }

  /// The parts of a level worth opening first: critical, diveable, and not
  /// already open or being written.
  function criticalToDive(item, target, level) {
    return ((level && level.nodes) || []).filter(
      (n) => n.critical && explainPartState(item, target, n) === "dive"
    );
  }

  /// A part id at the start of a heading (`### 3. Retry wrapper`,
  /// `### 3.2 Backoff`) — how the write-up's sections are tied to parts.
  function headingPartId(text) {
    const m = /^\s*(\d+(?:\.\d+){0,2})\b\.?/.exec(String(text || ""));
    return m ? m[1] : null;
  }

  /// A part id carried by a link fragment in the drawn page (`#node-3.2`).
  function fragmentPartId(hash) {
    const m = /^#node-(\d+(?:\.\d+){0,2})$/.exec(String(hash || ""));
    return m ? m[1] : null;
  }

  /// The launch dialog's detail choices, the setting's value first.
  function explainDetailChoices(current) {
    const all = [
      {
        value: "critical",
        label: "Critical parts",
        detail: "The overview, plus the inside of the parts it flags critical. Everything else is a click away.",
      },
      {
        value: "overview",
        label: "Overview only",
        detail: "Cheapest: just the overview. Every part is a dive you start by clicking it.",
      },
      {
        value: "full",
        label: "Every part",
        detail: "The overview and the inside of every part, up front. Costs the most.",
      },
    ];
    const first = all.find((c) => c.value === current);
    return first ? [first, ...all.filter((c) => c !== first)] : all;
  }

  /// The local repository a PR belongs to: the candidate whose origin is the
  /// PR's `owner/repo`, compared case-insensitively. Null when none matches,
  /// or when two do — guessing between checkouts of one repository is the
  /// human's call.
  function matchRepoForPr(ownerRepo, candidates) {
    const want = String(ownerRepo || "").toLowerCase();
    if (!want) return null;
    const hits = (candidates || []).filter((c) => String(c.origin || "").toLowerCase() === want);
    return hits.length === 1 ? hits[0] : null;
  }

  /// `owner/repo` and the number from a PR URL, or null. Any host, so a
  /// GitHub Enterprise link resolves like a github.com one.
  function parsePrUrl(url) {
    const m = /^https?:\/\/[^/]+\/([^/]+\/[^/]+)\/pull\/(\d+)/i.exec(String(url || "").trim());
    return m ? { repo: m[1], number: Number(m[2]) } : null;
  }

  /// What one level deeper adds, by the depth of the level it would be — the
  /// altitudes of `skills/clash-explain` → *The tree*.
  const DEEPER = {
    1: "what is inside it — its components, the data and control flow between them, and the contract with the parts around it",
    2: "the mechanism — the key functions, state transitions, invariants and edge cases, with file:line",
    3: "an annotated walk-through of the code itself, with short excerpts",
  };

  /// Everything already known about a part that has no level of its own yet,
  /// gathered from the level that lists it: what it is, what it exchanges
  /// with its neighbours, where it lives, and what explaining it would add.
  /// Null when no written level lists the part.
  function explainPartPreview(tree, id) {
    const parentId = explainParentOf(id);
    const parent = explainLevel(tree, parentId);
    const part = parent && (parent.nodes || []).find((n) => n.id === id);
    if (!part) return null;
    const title = (pid) => {
      const n = (parent.nodes || []).find((x) => x.id === pid);
      return n ? n.title || pid : pid;
    };
    const edges = parent.edges || [];
    const depth = String(id).split(".").length;
    return {
      part,
      parentId,
      parentTitle: parent.title || (parentId ? parentId : "Overview"),
      depth,
      incoming: edges
        .filter((e) => e.to === id && e.from !== id)
        .map((e) => ({ id: e.from, title: title(e.from), label: e.label || "" })),
      outgoing: edges
        .filter((e) => e.from === id && e.to !== id)
        .map((e) => ({ id: e.to, title: title(e.to), label: e.label || "" })),
      siblings: (parent.nodes || []).filter((n) => n.id !== id),
      files: (part.files || []).filter(Boolean),
      deeper: DEEPER[depth] || "",
    };
  }

  /// Starting points for a dive's focus — the questions people bring to an
  /// unfamiliar part. Each becomes the dive's `Focus:`.
  function explainFocusSuggestions(preview) {
    const out = [
      "How does it work, step by step?",
      "What could break, and what happens then?",
    ];
    if (preview && (preview.incoming.length || preview.outgoing.length))
      out.push("Draw the flow through it, in order");
    if (preview && preview.part && preview.part.isNew) out.push("Why is it needed — what did we do before?");
    else out.push("What changes compared to today?");
    out.push("How is it tested?");
    return out;
  }

  /// `src/x.rs:40` → `src/x.rs`: a part's file reference without its line.
  function explainFilePath(ref) {
    return String(ref || "").replace(/:\d+(?::\d+)?$/, "");
  }

  const VIEW_GLYPHS = {
    flow: "→",
    state: "◎",
    data: "▤",
    "before-after": "⇄",
    rollout: "⏱",
    dependency: "⋔",
  };

  /// The tabs of one level, in the order the reader meets them: the main
  /// drawing, each extra view, then the write-up. Only what exists is offered.
  function explainViewTabs(forms, views) {
    const tabs = [];
    if (forms && forms.html) {
      tabs.push({
        key: "graphic",
        label: "◫ Diagram",
        hint: "The drawn overview — boxes, arrows, the repos and features it touches",
      });
    }
    for (const v of views || []) {
      tabs.push({
        key: `view:${v.id}`,
        label: `${VIEW_GLYPHS[v.kind || v.id] || "◇"} ${v.title || v.id}`,
        hint: v.why || `Another drawing of the same level: ${v.title || v.id}`,
      });
    }
    if (forms && forms.md) {
      tabs.push({
        key: "text",
        label: "☰ Write-up",
        hint: "The written walk-through, with diagrams where they help",
      });
    }
    return tabs;
  }

  /// Which tab to show: the one asked for when this level has it, else the
  /// main drawing, else the first tab there is. A view chosen on one level is
  /// not carried to a level that has no such view.
  function explainResolveView(requested, forms, views) {
    const tabs = explainViewTabs(forms, views);
    if (tabs.some((t) => t.key === requested)) return requested;
    if (tabs.some((t) => t.key === "graphic")) return "graphic";
    return tabs.length ? tabs[0].key : "text";
  }

  /// The view id of a `view:<id>` tab key, or null for the main forms.
  function explainViewId(key) {
    const m = /^view:([a-z][a-z0-9-]{0,23})$/.exec(String(key || ""));
    return m ? m[1] : null;
  }

  /// Is a frame still showing the document clash gave it? Anything else —
  /// the app's own page, a website — means a link navigated it away.
  function explainFrameIntact(href) {
    return /^about:srcdoc(#.*)?$/.test(String(href || ""));
  }

  const api = {
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
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
