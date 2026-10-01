// The next-step recommender — the pure half.
//
// One question, asked of every workflow item: of the buttons on its action
// bar, which one should the human press next, and why? Each stage has an
// ordered list of checks; the first one that fires wins (cheapest, most
// urgent first), and every check that did not fire is reported as what is
// already settled. The agents feed in through `**Next:**` (a reviewer round's
// last line, the executor's `handoff.md`), but only where clash's own checks
// are silent — an agent never outranks an unapplied review or an open
// comment.
//
// It only recommends an action whose button the bar actually rendered
// (`facts.available`), so the suggestion is always clickable and the gates
// stay owned by the bar. Contract: docs/workflows.md → Next-step assist.
(function () {
  "use strict";

  /// The `**Next:**` vocabulary. Mirrors `application::workflow::NEXT_ACTIONS`.
  const NEXT_ACTIONS = [
    "apply",
    "approve",
    "plan-review",
    "code-review",
    "deep-review",
    "drift",
    "answer-comments",
  ];

  /// Actions autopilot may start without a click: each one produces a report
  /// and decides nothing. An approval, a PR flip, a change request (whose note
  /// is the human's) and anything posted on GitHub always waits for a person.
  const AUTO_ACTIONS = new Set(["apply-review", "plan-review", "code-review", "drift"]);

  /// Has `target` had a round launched against the item's current iteration
  /// that has also finished? `meta.reviewMarks` says which iteration the
  /// latest launch read, the per-target tally says whether it landed. Items
  /// predating the marks count any past round as current: nagging every old
  /// item for a review it may well have had is worse than missing one.
  function reviewedThisIteration(item, target) {
    const meta = (item && item.meta) || {};
    const tally = ((item && item.reviewRounds) || {})[target] || 0;
    const mark = (meta.reviewMarks || {})[target];
    if (!mark) return tally > 0;
    return (mark.iteration || 0) === (meta.iteration || 0) && tally >= (mark.round || 0);
  }

  /// The agents' own recommendation, freshest source first: the latest
  /// judging round when it is about this stage's artifact, else the
  /// executor's hand-off. Unknown words were already dropped by the parser.
  function agentHint(item) {
    const r = item && item.lastAgentReview;
    const meta = (item && item.meta) || {};
    const stagePlan = meta.status === "plan-review";
    const fits = r && r.next && (stagePlan ? r.target === "plan" : r.target !== "plan");
    if (fits) return { next: r.next, reason: r.nextReason || "", from: `review round ${r.round}` };
    const h = item && item.handoff;
    if (h && h.next) return { next: h.next, reason: h.nextReason || "", from: "the agent's hand-off" };
    return null;
  }

  const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;

  /// The ordered checks for `item`'s stage. Each is `{ id, fires, reason,
  /// settled, params? }` — `settled` is what the check reads as when it does
  /// not fire, shown under "why this one".
  function checks(item, facts) {
    const meta = item.meta || {};
    const st = meta.status;
    const pending = facts.pending || null;
    const prs = facts.prs || [];
    const hint = agentHint(item);
    const said = (what) => (hint && hint.next === what ? hint : null);
    const because = (h, fallback) =>
      h ? `${h.from} recommends it${h.reason ? `: ${h.reason}` : ""}` : fallback;
    const open = item.openAnnotations || 0;
    const unanswered = prs.reduce((n, p) => n + (p.unanswered > 0 ? p.unanswered : 0), 0);
    const it = meta.iteration || 0;

    const applyCheck = {
      id: "apply-review",
      fires: !!pending && pending.apply !== false,
      reason: pending
        ? `Review round ${pending.round} came back with findings that are not applied yet` +
          (pending.applyReason ? ` — ${pending.applyReason}` : "")
        : "",
      settled: "no review waiting to be applied",
      auto: !!pending && pending.apply === true,
    };

    switch (st) {
      case "draft":
        return [
          {
            id: "start-planning",
            fires: true,
            reason: "Nothing is planned yet — an agent explores the repo and writes plan.md",
          },
        ];
      case "planning":
      case "implementing":
        return [
          {
            id: "relaunch",
            fires: item.agentAlive === false,
            reason: "The agent's session is gone — start a fresh one on the same phase",
            settled: "the agent is running",
          },
          {
            id: "open-session",
            fires: true,
            reason: "An agent is working — watch it, or answer its questions",
          },
        ];
      case "reviewing":
        return [
          {
            id: "end-round",
            fires: item.agentAlive === false,
            reason: "The reviewer's session is gone — end the round to unlock the item",
            settled: "the reviewer is running",
          },
          { id: "open-session", fires: true, reason: "A review round is running" },
        ];
      case "changes-requested":
        return [
          {
            id: "launch-round",
            fires: true,
            reason: "Your change request is recorded, but no agent is applying it yet",
          },
        ];
      case "plan-review":
        return [
          applyCheck,
          {
            id: "plan-review",
            fires: !reviewedThisIteration(item, "plan"),
            reason: because(
              said("plan-review"),
              it > 1
                ? "This revision of the plan has not been reviewed — the last round changed it"
                : "No agent has reviewed this plan yet — a plan is the cheapest place to catch a mistake"
            ),
            settled: "the plan was reviewed at this iteration",
            auto: true,
          },
          {
            id: "approve-plan",
            fires: true,
            reason: because(
              said("approve"),
              "The plan was reviewed at this iteration and nothing is waiting on it"
            ),
          },
        ];
    }

    if (!["diff-review", "pr-draft", "pr-ready"].includes(st)) return [];

    const deep = !!said("deep-review");
    const lastDiff = meta.review && meta.review.target === "diff" ? meta.review : null;
    const neverDrift = !((item.reviewRounds || {}).drift > 0);
    const code = [
      applyCheck,
      {
        id: "request-changes",
        fires: open > 0,
        reason: `${plural(open, "comment is", "comments are")} still open — they become the next fix round`,
        settled: "no open comments",
      },
      {
        id: "answer-comments",
        fires: unanswered > 0,
        reason: `${plural(unanswered, "PR thread is", "PR threads are")} waiting on an answer`,
        settled: "no PR thread waiting on you",
      },
      {
        id: "code-review",
        fires: !reviewedThisIteration(item, "diff"),
        reason: because(
          said("deep-review") || said("code-review"),
          it > 1
            ? "The code changed since its last review — the fix round has not been checked"
            : "No agent has reviewed this code yet"
        ),
        settled: "the code was reviewed at this iteration",
        params: { depth: deep ? "deep" : "standard" },
        auto: true,
      },
      {
        // A standard round already ran at this iteration, but something said
        // the change warrants a deep one.
        id: "code-review",
        fires: deep && !!lastDiff && lastDiff.depth !== "deep",
        reason: because(said("deep-review"), ""),
        settled: "no deep review asked for",
        params: { depth: "deep" },
        auto: true,
      },
      {
        id: "drift",
        fires:
          !!facts.hasPlan &&
          !reviewedThisIteration(item, "drift") &&
          (neverDrift || !!said("drift")),
        reason: because(
          said("drift"),
          "Nothing has compared the change with the plan yet — a diff can pass review and still build something else"
        ),
        settled: facts.hasPlan ? "the change was compared with the plan" : "no plan to compare against",
        auto: true,
      },
    ];

    const settledReason = "Reviewed at this iteration, with nothing open or waiting";
    if (st === "diff-review") {
      return [
        ...code,
        {
          id: "approve-pr-draft",
          fires: true,
          reason: because(said("approve"), `${settledReason} — move on to the PR`),
        },
        { id: "approve-done", fires: true, reason: because(said("approve"), settledReason) },
        { id: "create-pr", fires: true, reason: "Approved code still needs a PR to land" },
      ];
    }
    if (st === "pr-draft") {
      return [
        ...code,
        {
          id: "pr-is-ready",
          fires: true,
          reason: "The PR is already ready for review on GitHub — record it",
        },
        {
          id: "mark-ready",
          fires: true,
          reason: `${settledReason} — flip the draft to ready for review`,
        },
        { id: "attach-pr", fires: true, reason: "No PR is recorded for this item" },
        { id: "mark-done", fires: true, reason: "Nothing left to do here" },
      ];
    }
    return [
      ...code,
      {
        id: "open-prs",
        fires: true,
        reason: "Waiting on merge — the item closes itself when the PR merges",
      },
    ];
  }

  /// The recommendation for `item`: `{ id, reason, auto, params, settled }`,
  /// or null when the stage has nothing to recommend (a finished item).
  ///
  /// `facts` carries what the action bar already knows:
  /// - `available` — Set of action ids the bar rendered;
  /// - `pending` — `pendingReviewRound(item)`;
  /// - `prs` — `itemPrs(item.meta)`;
  /// - `hasPlan` — the item has a plan phase and a written plan.
  function wfNextStep(item, facts) {
    if (!item || !item.meta) return null;
    const available = (facts && facts.available) || new Set();
    const settled = [];
    for (const c of checks(item, facts || {})) {
      if (c.fires && available.has(c.id)) {
        return {
          id: c.id,
          reason: c.reason,
          auto: !!c.auto && AUTO_ACTIONS.has(c.id),
          params: c.params || {},
          settled,
        };
      }
      if (!c.fires && c.settled) settled.push(c.settled);
    }
    return null;
  }

  /// Which existing explanations describe an older iteration than the item's
  /// current one — the ones worth refreshing alongside a plan-vs-changes
  /// round, so the three documents you read side by side agree. An
  /// explanation with no record predates the tracking and counts as stale; a
  /// running one is already being refreshed. `canExplain(target)` is the
  /// bar's own gate.
  function staleExplanations(item, canExplainFn) {
    const meta = (item && item.meta) || {};
    const it = meta.iteration || 0;
    const out = [];
    for (const [target, forms, label] of [
      ["explain-plan", item && item.planExplain, "plan explanation"],
      ["explain-diff", item && item.diffExplain, "changes explanation"],
    ]) {
      if (!(forms && (forms.md || forms.html))) continue;
      if (canExplainFn && !canExplainFn(target)) continue;
      const rec = ((item && item.explainers) || []).find((e) => e.target === target);
      if (rec && rec.running) continue;
      if (rec && rec.finished && (rec.iteration || 0) === it) continue;
      out.push({ target, label });
    }
    return out;
  }

  const api = { NEXT_ACTIONS, AUTO_ACTIONS, wfNextStep, reviewedThisIteration, staleExplanations };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
