// Cross-project PR dashboard — the pure half.
//
// One item can carry several PRs since linked PRs exist (a backend/frontend/
// contract split lands as several repos). This module answers two questions
// without a DOM: "which PRs does this item have?" (also used by the item
// header and the Open-PRs action) and "what does the dashboard show, in what
// order?". The tab itself is in `app.js` (the `wf-timeline.js` precedent).
(function () {
  "use strict";

  /// "Wave 2 · API" — wf-waves.js's waveLabel when loaded (wf-waves.js reads
  /// this module, so the lookup is lazy rather than a require cycle).
  function waveTitle(w) {
    const f =
      typeof module !== "undefined" && module.exports
        ? require("./wf-waves.js").waveLabel
        : typeof window !== "undefined" && window.waveLabel;
    return f ? f(w) : w.title ? `${w.id} · ${w.title}` : w.id;
  }

  // Mirrors WorkflowStatus::needs_attention — kept local so the module stays
  // loadable without app.js (node tests).
  const PR_DECISION_STATUSES = ["plan-review", "diff-review", "pr-draft"];

  /// `owner/repo` out of a GitHub PR URL; "" when it isn't one.
  function prRepo(url) {
    const m = /github\.com\/([^/]+)\/([^/]+)\/pull\/\d+/.exec(String(url || ""));
    return m ? `${m[1]}/${m[2]}` : "";
  }

  /// Short display name for a PR chip: `repo#42` (basename of the repo — the
  /// owner is noise inside one org), falling back to the URL tail.
  function prChipLabel(pr) {
    const repo = prRepo(pr.url);
    const name = repo ? repo.split("/")[1] : "";
    const num = pr.number || Number(String(pr.url || "").split("/").pop()) || 0;
    return name ? `${name}#${num || "?"}` : `#${num || "?"}`;
  }

  /// Lifecycle label for a PR record: merged/closed win over draft (a merged
  /// PR that was once a draft is merged), unknown state with a URL is "open"
  /// as far as anyone can tell — say "PR" and let the refresh firm it up.
  function prStateLabel(pr) {
    if (pr.state === "MERGED") return "merged";
    if (pr.state === "CLOSED") return "closed";
    if (pr.draft) return "draft";
    if (pr.state === "OPEN") return "open";
    return "";
  }

  /// Every PR of one item's meta, primary first: `{ url, number, repo, draft,
  /// state, unanswered, headRef, baseRef, health, primary, stack }`. Empty-URL records are skipped —
  /// they are placeholders, not PRs.
  ///
  /// `prStack` is the item's derived stack (`item.prStack`, base → tip). Its
  /// layers are spliced in stack order where the first of them sat, and each
  /// carries `stack: { pos, of, parent }` — so every fan-out walks a stack
  /// parent before child. PRs outside it keep their place and `stack: null`.
  function itemPrs(meta, prStack) {
    const out = [];
    const push = (pr, primary) => {
      // One record per URL, whatever lists it twice: item-wide counts read these.
      if (!pr || !pr.url || out.some((p) => p.url === pr.url)) return;
      out.push({
        url: pr.url,
        number: pr.number || 0,
        repo: prRepo(pr.url),
        draft: !!pr.draft,
        state: pr.state || "",
        unanswered: typeof pr.unansweredComments === "number" ? pr.unansweredComments : null,
        headRef: pr.headRef || "",
        baseRef: pr.baseRef || "",
        health: pr.health || null,
        primary,
        stack: null,
      });
    };
    push(meta && meta.pr, true);
    for (const pr of (meta && meta.linkedPrs) || []) push(pr, false);
    return stackOrder(out, prStack);
  }

  /// Annotate and reorder `prs` by a stack (see `itemPrs`). A stack naming a
  /// PR the item no longer tracks is stale, and is ignored whole.
  function stackOrder(prs, prStack) {
    const layers = ((prStack && prStack.layers) || []).map((l) => l.url);
    if (layers.length < 2 || !layers.every((u) => prs.some((p) => p.url === u))) return prs;
    const layered = layers.map((url, i) => ({
      ...prs.find((p) => p.url === url),
      stack: { pos: i + 1, of: layers.length, parent: i > 0 ? layers[i - 1] : null },
    }));
    const out = [];
    for (const p of prs) {
      if (!layers.includes(p.url)) out.push(p);
      else if (!out.some((q) => q.stack)) out.push(...layered);
    }
    return out;
  }

  const settledPr = (pr) => pr.state === "MERGED" || pr.state === "CLOSED";

  /// A PR's health (`pr.health`, written by the refresh) as compact badges:
  /// `[{ kind, text, title }]` — CI first (✓ / ✗ / ●, failing names in the
  /// title), then "conflicts", "behind base", the review decision. A merged or
  /// closed PR shows none: its CI no longer decides anything.
  function prHealthBadges(pr) {
    const h = pr && pr.health;
    if (!h || settledPr(pr)) return [];
    const out = [];
    const failing = (h.failing || []).join(", ");
    if (h.checks === "fail")
      out.push({ kind: "fail", text: "✗", title: `CI failing${failing ? `: ${failing}` : ""}` });
    else if (h.checks === "pending") out.push({ kind: "pending", text: "●", title: "CI running" });
    else if (h.checks === "pass") out.push({ kind: "pass", text: "✓", title: "CI passing" });
    if (h.mergeable === "conflicts")
      out.push({ kind: "conflicts", text: "conflicts", title: "Conflicts with its base branch" });
    if (h.behind)
      out.push({
        kind: "behind",
        text: "behind base",
        title: "Behind its base branch, which must be up to date to merge",
      });
    if (h.review === "approved") out.push({ kind: "approved", text: "approved", title: "Review approved" });
    else if (h.review === "changes")
      out.push({ kind: "changes", text: "changes requested", title: "A reviewer requested changes" });
    return out;
  }

  /// The badges' titles on one line, for a chip's tooltip ("" when none).
  function prHealthTitle(pr) {
    return prHealthBadges(pr)
      .map((b) => b.title)
      .join(" · ");
  }

  /// "2/3" for a stacked PR, "" otherwise.
  function prStackLabel(pr) {
    return pr && pr.stack ? `${pr.stack.pos}/${pr.stack.of}` : "";
  }

  /// Rows for the PR dashboard: every item holding at least one PR, sorted so
  /// the ones asking for something come first — decision states, then items
  /// with any non-merged/non-closed PR, then the rest; freshest last-touched
  /// first within each band. Terminal items with all PRs merged sink to the
  /// bottom instead of being hidden: "what shipped lately" is half the point
  /// of a PR list.
  function prDashboardModel(items) {
    const rows = [];
    for (const item of items || []) {
      const prs = itemPrs(item.meta || {}, item.prStack);
      if (!prs.length) continue;
      const open = prs.filter((p) => p.state !== "MERGED" && p.state !== "CLOSED");
      rows.push({
        project: item.project,
        slug: item.slug,
        title: (item.meta && item.meta.title) || item.slug,
        status: (item.meta && item.meta.status) || "unknown",
        needsDecision: PR_DECISION_STATUSES.includes(item.meta && item.meta.status),
        updatedAt: (item.meta && item.meta.updatedAt) || 0,
        unanswered: prs.reduce((n, p) => n + (p.unanswered || 0), 0),
        allSettled: open.length === 0,
        // Why stacking is on but the PRs do not order — said on the row,
        // since a silent fallback to "no stack" reads as stacking not working.
        stackIssue: (item.prStack ? null : item.prStackIssue) || null,
        prs,
      });
    }
    rows.sort((a, b) => {
      if (a.needsDecision !== b.needsDecision) return a.needsDecision ? -1 : 1;
      if (a.allSettled !== b.allSettled) return a.allSettled ? 1 : -1;
      return b.updatedAt - a.updatedAt;
    });
    return rows;
  }

  /// The dashboard's filters over `prDashboardModel` rows. `view` is
  /// `active` (items with a PR still open — the default), `attention`
  /// (an open PR waiting on you: unanswered threads, or the item parked on
  /// your decision) or `all`; `project` and `query` narrow further (every word
  /// of the query must appear in the title, project or a PR's repo/branch).
  /// Returns `{ rows, counts: { active, attention, all }, summary, projects }`
  /// — counts and projects over the project/query-filtered set, so a tab's
  /// number is what clicking it shows.
  function prDashboardView(rows, { view = "active", project = null, query = "" } = {}) {
    const words = String(query || "").toLowerCase().split(/\s+/).filter(Boolean);
    const hay = (r) =>
      [r.title, r.project, ...r.prs.map((p) => `${p.repo} #${p.number} ${p.headRef}`)]
        .join(" ")
        .toLowerCase();
    const counts = new Map();
    for (const r of rows || []) counts.set(r.project, (counts.get(r.project) || 0) + 1);
    const projects = [...counts]
      .map(([name, count]) => ({ name, count }))
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
    const base = (rows || []).filter(
      (r) => (!project || r.project === project) && words.every((w) => hay(r).includes(w))
    );
    const active = (r) => !r.allSettled;
    const attention = (r) => active(r) && (r.unanswered > 0 || r.needsDecision);
    const pick = view === "attention" ? attention : view === "all" ? () => true : active;
    const live = base.flatMap((r) => r.prs.filter((p) => p.state !== "MERGED" && p.state !== "CLOSED"));
    return {
      rows: base.filter(pick),
      counts: {
        active: base.filter(active).length,
        attention: base.filter(attention).length,
        all: base.length,
      },
      summary: {
        open: live.length,
        drafts: live.filter((p) => p.draft).length,
        threads: live.reduce((n, p) => n + (p.unanswered > 0 ? p.unanswered : 0), 0),
        merged: base.flatMap((r) => r.prs).filter((p) => p.state === "MERGED").length,
      },
      projects,
    };
  }

  /// Counts over a PR list, for the summaries that stand in for a row of
  /// chips: `{ total, merged, closed, open, draft, threads }` — `open`
  /// includes drafts, `threads` sums the known unanswered counts.
  function prCounts(prs) {
    const n = (f) => (prs || []).filter(f).length;
    return {
      total: (prs || []).length,
      merged: n((p) => p.state === "MERGED"),
      closed: n((p) => p.state === "CLOSED"),
      open: n((p) => p.state !== "MERGED" && p.state !== "CLOSED"),
      draft: n((p) => p.draft && p.state !== "MERGED" && p.state !== "CLOSED"),
      threads: (prs || []).reduce((t, p) => t + (p.unanswered > 0 ? p.unanswered : 0), 0),
    };
  }

  /// "10 PRs · 2 merged · 1 closed · 7 open" — the header's stand-in for a
  /// chip per PR, which stops being readable past a handful.
  function prCountsText(c) {
    if (!c.total) return "No PR";
    const parts = [`${c.total} PR${c.total === 1 ? "" : "s"}`];
    if (c.merged) parts.push(`${c.merged} merged`);
    if (c.closed) parts.push(`${c.closed} closed`);
    if (c.open) parts.push(`${c.open} open`);
    if (c.threads) parts.push(`${c.threads} thread${c.threads === 1 ? "" : "s"} to answer`);
    return parts.join(" · ");
  }

  /// The item's PRs grouped for its Pull requests tab: by wave when the item
  /// has waves (in wave order, then the PRs no wave claims), else one group.
  /// A PR several waves list appears under each; `counts` is item-wide and
  /// counts it once.
  /// Each row carries the last refresh's error for that PR, if any. Settled
  /// PRs sort after open ones inside a group, so what needs work reads first.
  function prBoard(item) {
    const prs = itemPrs((item && item.meta) || {}, item && item.prStack).map((p) => ({
      ...p,
      error: ((item && item.prErrors) || {})[p.url] || "",
    }));
    const settled = (p) => p.state === "MERGED" || p.state === "CLOSED";
    const order = (list) =>
      // Stable: the stack/primary order holds inside each half.
      [...list.filter((p) => !settled(p)), ...list.filter(settled)];
    const board = item && item.waves && !item.waves.issue ? item.waves : null;
    let groups;
    if (board && board.waves && board.waves.length) {
      groups = board.waves.map((w) => ({
        key: w.id,
        title: waveTitle(w),
        state: w.state,
        prs: order(prs.filter((p) => (w.prs || []).includes(p.url))),
      }));
      const loose = prs.filter((p) => !board.waves.some((w) => (w.prs || []).includes(p.url)));
      if (loose.length) groups.push({ key: "", title: "Not in a wave", state: "", prs: order(loose) });
      groups = groups.filter((g) => g.prs.length || g.state === "active" || g.state === "review");
    } else {
      groups = prs.length ? [{ key: "", title: "", state: "", prs: order(prs) }] : [];
    }
    return {
      groups,
      counts: prCounts(prs),
      errors: prs.filter((p) => p.error).length,
      checkedAt: (item && item.prCheckedAt) || 0,
    };
  }

  /// "checked 3 min ago" / "not checked yet".
  function checkedAgo(at, now) {
    if (!at) return "not checked yet";
    const s = Math.max(0, Math.round(((now || Date.now()) - at) / 1000));
    if (s < 60) return "checked just now";
    const m = Math.round(s / 60);
    if (m < 60) return `checked ${m} min ago`;
    const h = Math.round(m / 60);
    return `checked ${h} h ago`;
  }

  const api = {
    prRepo,
    prChipLabel,
    prStateLabel,
    itemPrs,
    prStackLabel,
    prHealthBadges,
    prHealthTitle,
    prDashboardModel,
    prDashboardView,
    prCounts,
    prCountsText,
    prBoard,
    checkedAgo,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
