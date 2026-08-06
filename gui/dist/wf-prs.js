// Cross-project PR dashboard — the pure half.
//
// One item can carry several PRs since linked PRs exist (a backend/frontend/
// contract split lands as several repos). This module answers two questions
// without a DOM: "which PRs does this item have?" (also used by the item
// header and the Open-PRs action) and "what does the dashboard show, in what
// order?". The tab itself is in `app.js` (the `wf-timeline.js` precedent).
(function () {
  "use strict";

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
  /// state, unanswered, primary }`. Empty-URL records are skipped — they are
  /// placeholders, not PRs.
  function itemPrs(meta) {
    const out = [];
    const push = (pr, primary) => {
      if (!pr || !pr.url) return;
      out.push({
        url: pr.url,
        number: pr.number || 0,
        repo: prRepo(pr.url),
        draft: !!pr.draft,
        state: pr.state || "",
        unanswered: typeof pr.unansweredComments === "number" ? pr.unansweredComments : null,
        primary,
      });
    };
    push(meta && meta.pr, true);
    for (const pr of (meta && meta.linkedPrs) || []) push(pr, false);
    return out;
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
      const prs = itemPrs(item.meta || {});
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

  const api = { prRepo, prChipLabel, prStateLabel, itemPrs, prDashboardModel };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
