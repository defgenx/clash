// PR action scope — the pure half.
//
// Multi-repo work lands as several PRs on one item (a backend, a frontend, a
// contract), so every PR-shaped action carries a second question: which of
// them. Answering it by default is what this module exists to stop — "Open
// PR" opened all three, "Mark ready" flipped the primary plus every linked
// draft or nothing, and a code review could only ever read the local diff.
//
// **The answer is a set, for every action.** An earlier version of this
// module said review and respond rounds "cannot fan out", reasoning that a
// round is one agent session parked on one item. That is a constraint on
// agent *concurrency* — two rounds at once would be two agents editing one
// item's files — and it says nothing about how many PRs one round may read.
// One session reads three diffs perfectly well, and a cross-repo change is
// exactly where it must: reviewing the API PR without the web PR that
// consumes it is how a contract break survives review. So a round over
// several PRs is one round, and the set is the unit everywhere.
//
// What remains genuinely per-action is only two things:
//
//   - **candidates** — which PRs the action can act on at all. Mark-ready
//     offers drafts only; a merged PR is a `gh` failure dressed as a choice.
//   - **the default selection** — pre-checked when the dialog opens. The rule
//     is "the answer the button already promised", because that is the right
//     answer for the single-PR case and the one nobody has to think about. In
//     practice that means the primary alone (the item's own PR, and the only
//     one whose state moves the item), with two deliberate exceptions: `open`
//     is read-only and its label promises every PR, and `respond` advertises
//     the item's whole unanswered count, so it pre-checks every PR that has
//     threads waiting. It is never a *linked* repository by default outside
//     those two — announcing a second repo is what you tick deliberately.
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

  /// Per-action rules: what it can act on, what it says, and what starts
  /// checked. `defaultAll` is the deliberate exception to "primary only".
  const PR_ACTIONS = {
    open: {
      message: "Open which pull requests?",
      allLabel: (n) => `All ${n} pull requests`,
      okLabel: "Open",
      detail: "The first opens in a split pane, the rest as background tabs.",
      candidates: () => true,
      // Read-only, and "Open PRs (3)" promises three: pre-checking them all
      // costs nothing and keeps the click count where it was.
      defaultAll: true,
    },
    markReady: {
      message: "Mark which pull requests ready for review?",
      allLabel: (n) => `All ${n} drafts`,
      okLabel: "Mark ready",
      detail:
        "Each repository's change goes up for review. Only the primary PR moves this item to PR READY.",
      // A non-draft is already ready, and a merged or closed PR cannot go
      // back: both would be a `gh` failure dressed up as a choice.
      candidates: (pr) => pr.draft && prLive(pr),
      empty: "None of this item's pull requests is still a draft.",
    },
    post: {
      message: "Post the round to which pull requests?",
      allLabel: (n) => `All ${n} pull requests`,
      okLabel: "Post",
      detail: "The same round comment on each. No agent, no tokens.",
      candidates: () => true,
    },
    review: {
      message: "Review which pull requests?",
      allLabel: (n) => `All ${n} pull requests`,
      okLabel: "Continue",
      detail:
        "One round reads every PR you pick — a cross-repo change is one change, and reviewing half of it misses the contract between the halves.",
      candidates: () => true,
    },
    respond: {
      message: "Answer review comments on which pull requests?",
      allLabel: (n) => `All ${n} pull requests`,
      okLabel: "Launch",
      detail:
        "One round works through every open thread on every PR you pick, replying in each.",
      candidates: () => true,
      // The odd one out, and for a reason: this action's whole subject is
      // threads waiting on you, and the button that opens it advertises the
      // item's total ("Answer 5 open PR threads"). Pre-ticking the primary
      // alone would open the dialog agreeing to 2 of the 5 it just promised.
      // A reply is not a release, so there is nothing here to announce
      // accidentally.
      defaultWaiting: true,
    },
  };

  /// The PRs one action can act on, in `itemPrs` order (primary first).
  function prActionCandidates(prs, action) {
    const rule = PR_ACTIONS[action];
    if (!rule) return [];
    return (prs || []).filter(rule.candidates);
  }

  /// Everything the scope dialog shows for an action.
  ///
  /// `needed` is false when there is nothing to decide — zero or one
  /// candidate — and `only` then carries the answer straight through, so an
  /// item with a single PR never sees a dialog it cannot answer differently.
  /// `rows` are checkbox rows: `checked` is the default selection.
  function prScopeModel(prs, action, opts = {}) {
    const rule = PR_ACTIONS[action] || {};
    const candidates = prActionCandidates(prs, action);
    // A caller that already knows the selection (a per-PR menu, a re-open of
    // a dialog) seeds the checkboxes instead of the default.
    const seed = Array.isArray(opts.selected) ? opts.selected : null;
    // "The PRs with threads waiting" only exists when some PR is known to
    // have any: an unfetched count is not zero, and falling back to the
    // primary beats opening a dialog with nothing ticked.
    const waiting = rule.defaultWaiting && candidates.some((p) => p.unanswered > 0);
    const rows = candidates.map((pr) => ({
      url: pr.url,
      label: prRowLabel(pr, opts),
      detail: prRowDetail(pr),
      checked: seed
        ? seed.includes(pr.url)
        : rule.defaultAll
          ? true
          : waiting
            ? pr.unanswered > 0
            : !!pr.primary,
    }));
    return {
      action,
      message: rule.message || "Which pull requests?",
      detail: rule.detail || "",
      okLabel: rule.okLabel || "OK",
      allLabel: rule.allLabel ? rule.allLabel(candidates.length) : `All ${candidates.length}`,
      empty: rule.empty || "This item has no pull request.",
      candidates,
      rows,
      // Nothing to ask when the action has one candidate — or none, in which
      // case `only` is null and the caller says why.
      needed: candidates.length > 1,
      only: candidates.length === 1 ? { all: true, urls: [candidates[0].url] } : null,
    };
  }

  /// Turn a set of picked URLs into a selection, in the model's own row order
  /// — the primary first, then the linked PRs as the item records them, so a
  /// fan-out never depends on the order the boxes were clicked in.
  function prScopeSelection(model, urls) {
    const want = new Set(urls || []);
    const picked = model.rows.filter((r) => want.has(r.url)).map((r) => r.url);
    return { all: picked.length === model.rows.length && picked.length > 0, urls: picked };
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

  /// How a completed action names what it acted on: a count when it covered
  /// several, the PR itself when it covered one. Used in toasts and
  /// confirmations — "PR ready" on a three-repo item said nothing about which
  /// repo.
  function prScopeSummary(sel, prs) {
    const urls = (sel && sel.urls) || [];
    // "all 3" and "3" are different agreements when the item has four PRs, and
    // the confirmation is where that difference has to be visible.
    if (urls.length > 1)
      return sel && sel.all ? `all ${urls.length} pull requests` : `${urls.length} pull requests`;
    const pr = (prs || []).find((p) => p.url === urls[0]);
    return pr ? prScopeChip(pr) : urls[0] || "the PR";
  }

  /// Suffix for an action button whose click will ask which PRs: the ellipsis
  /// convention the rest of the action bar uses for "opens a dialog".
  function prScopeSuffix(prs, action) {
    return prActionCandidates(prs, action).length > 1 ? "…" : "";
  }

  const api = {
    PR_ACTIONS,
    prLive,
    prActionCandidates,
    prScopeModel,
    prScopeSelection,
    prScopeSummary,
    prScopeSuffix,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
