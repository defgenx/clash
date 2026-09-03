// PR action scope — the pure half.
//
// Multi-repo work lands as several PRs on one item (a backend, a frontend, a
// contract), so every PR-shaped action carries a second question: which of
// them. Answering it by default is what this module exists to stop — "Open
// PR" opened all three at once, and "Mark ready" flipped the primary plus
// every linked draft or nothing, which turns three separate decisions into
// one button.
//
// The answer is not the same for every action, and the difference is not
// cosmetic:
//
//   - **open** and **post** fan out freely — three browser tabs, or the same
//     round comment on three PRs, are all one intent.
//   - **markReady** fans out too, but only over PRs that are actually drafts:
//     offering "mark ready" for a merged PR is offering a failure.
//   - **review** and **respond** cannot fan out at all. A round parks the
//     item in `reviewing` with one agent session, so two rounds at once is
//     two agents editing one item's files. Their "all" is a *different
//     answer*, not several: for a review it is the item's whole local diff
//     (no PR scope), and for a respond round there is none — answering
//     reviewers happens per PR, one PR at a time.
//
// `app.js` holds the dialog; the same shape as `wf-prs.js`, which this module
// takes its PR records from (`itemPrs`).
(function () {
  "use strict";

  /// A PR is still live when it is neither merged nor closed. Actions that
  /// change a PR (mark ready) only make sense on those; actions that read or
  /// annotate one (open, review, post a round) stay available afterwards —
  /// "what shipped" is a legitimate thing to open.
  function prLive(pr) {
    return pr.state !== "MERGED" && pr.state !== "CLOSED";
  }

  /// Per-action rules. `all` is whether the action can act on several PRs in
  /// one go; `allLabel`/`allDetail` name that option; `candidates` filters
  /// the PRs the action can act on at all; `empty` is what to say when the
  /// filter leaves nothing.
  const PR_ACTIONS = {
    open: {
      all: true,
      message: "Open which pull request?",
      allLabel: (n) => `All ${n} pull requests`,
      allDetail: "One split pane, the rest as background tabs",
      candidates: () => true,
    },
    markReady: {
      all: true,
      message: "Mark which pull request ready for review?",
      allLabel: (n) => `All ${n} drafts`,
      allDetail: "Flips every draft this item tracks — each repository's change goes up for review",
      // A non-draft is already ready, and a merged or closed PR cannot go
      // back: both would be a `gh` failure dressed up as a choice.
      candidates: (pr) => pr.draft && prLive(pr),
      empty: "None of this item's pull requests is still a draft.",
    },
    post: {
      all: true,
      message: "Post the round to which pull request?",
      allLabel: (n) => `All ${n} pull requests`,
      allDetail: "The same round comment on each — the round covered the whole change",
      candidates: () => true,
    },
    review: {
      // One agent per item: a round is a session, not a batch.
      all: false,
      message: "Review which pull request?",
      candidates: () => true,
    },
    respond: {
      all: false,
      message: "Answer review comments on which pull request?",
      candidates: () => true,
    },
  };

  /// The PRs one action can act on, in `itemPrs` order (primary first).
  function prActionCandidates(prs, action) {
    const rule = PR_ACTIONS[action];
    if (!rule) return [];
    return (prs || []).filter(rule.candidates);
  }

  /// What the scope picker should show for an action.
  ///
  /// `needed` is false when there is nothing to decide — zero or one
  /// candidate — and `only` then carries the answer straight through, so an
  /// item with a single PR never sees a dialog it cannot answer differently.
  /// Selections are `{ all, urls }`: `all` is the fan-out (and, for a review,
  /// the "whole change, no PR scope" answer, which is why its `urls` is
  /// empty), `urls` the PRs picked.
  function prScopeModel(prs, action, opts = {}) {
    const rule = PR_ACTIONS[action] || {};
    const candidates = prActionCandidates(prs, action);
    const rows = [];
    if (rule.all && candidates.length > 1) {
      rows.push({
        label: rule.allLabel(candidates.length),
        detail: rule.allDetail || "",
        value: { all: true, urls: candidates.map((p) => p.url) },
      });
    }
    for (const pr of candidates) {
      rows.push({
        label: prRowLabel(pr, opts),
        detail: prRowDetail(pr),
        value: { all: false, urls: [pr.url] },
      });
    }
    return {
      action,
      message: rule.message || "Which pull request?",
      empty: rule.empty || "This item has no pull request.",
      candidates,
      rows,
      // Nothing to ask when the action has one candidate — or none, in which
      // case `only` is null and the caller says why.
      needed: candidates.length > 1,
      only: candidates.length === 1 ? { all: false, urls: [candidates[0].url] } : null,
    };
  }

  /// Row label: the repo-scoped chip name (`api#42`), marked so the primary —
  /// the only PR that drives the item's status — is never mistaken for one of
  /// the linked ones.
  function prRowLabel(pr, opts = {}) {
    const name = typeof opts.chipLabel === "function" ? opts.chipLabel(pr) : prScopeChip(pr);
    return pr.primary ? `${name} · primary` : name;
  }

  /// Row detail: state, unanswered threads and the URL — everything needed to
  /// tell two PRs of the same item apart without opening either.
  function prRowDetail(pr) {
    const bits = [];
    const st = prScopeState(pr);
    if (st) bits.push(st);
    if (pr.unanswered) bits.push(`${pr.unanswered} unanswered`);
    bits.push(pr.url);
    return bits.join(" · ");
  }

  /// Local fallbacks for the two `wf-prs.js` formatters, so this module is
  /// loadable on its own (node tests) and the rows read the same either way.
  function prScopeChip(pr) {
    if (typeof prChipLabel === "function") return prChipLabel(pr);
    const name = String(pr.repo || "").split("/")[1] || "";
    return name ? `${name}#${pr.number || "?"}` : `#${pr.number || "?"}`;
  }

  function prScopeState(pr) {
    if (typeof prStateLabel === "function") return prStateLabel(pr);
    if (pr.state === "MERGED") return "merged";
    if (pr.state === "CLOSED") return "closed";
    if (pr.draft) return "draft";
    return pr.state === "OPEN" ? "open" : "";
  }

  /// How a completed action names what it acted on: a count when it fanned
  /// out, the PR itself when it didn't. Used in toasts and confirmations —
  /// "PR ready" on a three-repo item said nothing about which repo.
  function prScopeSummary(sel, prs) {
    const urls = (sel && sel.urls) || [];
    if (urls.length > 1) return `${urls.length} pull requests`;
    const pr = (prs || []).find((p) => p.url === urls[0]);
    return pr ? prScopeChip(pr) : urls[0] || "the PR";
  }

  /// Suffix for an action button whose click will ask which PR: the ellipsis
  /// convention the rest of the action bar uses for "opens a dialog".
  function prScopeSuffix(prs, action) {
    return prActionCandidates(prs, action).length > 1 ? "…" : "";
  }

  const api = {
    PR_ACTIONS,
    prLive,
    prActionCandidates,
    prScopeModel,
    prScopeSummary,
    prScopeSuffix,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
