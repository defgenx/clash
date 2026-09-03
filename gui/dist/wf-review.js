// Agent-review launcher — the pure half.
//
// A review round has three dimensions: what is reviewed (derived from the
// item's status), how deep, and where the findings go. They used to be asked
// as two stacked dialogs, which meant the full shape of the round was never
// on screen at once — and "answer the PR's review comments", a different job
// entirely, hid as the third option of the second dialog. The model here
// drives one composer dialog, and the respond job gets its own action.
//
// Same shape as `wf-compose.js`: everything testable without a DOM lives
// here, the dialogs themselves are in `app.js` with the other modals.
(function () {
  "use strict";

  /// Everything the review-round composer shows, derived from the item.
  /// `depth` is null for a plan (the plan-review engine has one depth —
  /// there are no hunks to read harder); `publish` is null without a PR
  /// (there is nothing to talk to); `prScope` is null unless the item carries
  /// several PRs, in which case *which change* is a real dimension of the
  /// round and belongs on the same screen as the other three.
  ///
  /// `prs` is `itemPrs(meta)` — primary first. The rows are built here rather
  /// than in the dialog so the whole shape of a round stays testable.
  function reviewRoundModel({
    round = 1,
    target = "diff",
    hasPr = false,
    prNumber = 0,
    // A draft PR cannot take a *review* (GitHub rejects one), but it takes
    // comments — and reviewing a draft is the normal case, not an edge one.
    // The choice stays offered; only its wording changes, so nobody discovers
    // the difference from a failure at the end of a round.
    prDraft = false,
    interactionDefault = "",
    autoApplyDefault = false,
    prs = [],
    // A pre-picked scope (a launch that started from one PR's own menu) —
    // pre-checks those rows instead of "this repository's own diff".
    prUrls = null,
  } = {}) {
    // A plan is one document however many repos implement it, so a plan round
    // has no PR scope. A code round on a multi-repo item does — and its
    // answer is a **set**: a cross-repo change is one change, and reviewing
    // the API PR without the web PR that consumes it is how a contract break
    // survives review. Checkboxes rather than radios for exactly that reason.
    //
    // Nothing checked means the item's own diff, which is what a code round
    // read before the scope existed. That state is rendered as its own row
    // instead of being left implicit: "leave everything unchecked" is not a
    // thing anyone reads off a dialog.
    const seeded = (prUrls || []).filter((u) => (prs || []).some((p) => p.url === u));
    const prScope =
      target === "plan" || (prs || []).length < 2
        ? null
        : {
            legend: "Which change?",
            // The local row and the PR rows are one group: picking a PR is
            // saying "read that repository instead of / as well as mine".
            local: {
              value: "",
              label: "This repository's own diff",
              detail:
                "Every file on the branch — findings land as diff comments you can triage here.",
              checked: seeded.length === 0,
            },
            choices: (prs || []).map((pr) => ({
              value: pr.url,
              label: prScopeRowLabel(pr),
              detail: prScopeRowDetail(pr),
              checked: seeded.includes(pr.url),
              // A linked PR's files are in another repository, so its
              // findings cannot be annotations on this item's diff — they
              // belong on that PR.
              linked: !pr.primary,
            })),
          };
    // With a scope choice on screen, naming the primary in the publish option
    // is a lie for most of the answers: it says where the findings go, which
    // is whichever PRs the round is about.
    const prName = prScope ? "the PRs under review" : prNumber ? `#${prNumber}` : "the PR";
    return {
      prScope,
      title: `Agent review — round ${round}`,
      intro:
        (target === "plan"
          ? "Reviews this item's plan against the real codebase, then returns the item here."
          : "Reviews this item's code diff, then returns the item here.") +
        " Findings land in this item either way. Spends tokens.",
      launchLabel: `Launch round ${round}`,
      depth:
        target === "plan"
          ? null
          : {
              legend: "How deep?",
              default: "deep",
              choices: [
                {
                  value: "deep",
                  label: "Deep review",
                  detail:
                    "Traces every subsystem the change touches — callers, invariants, existing tests — before judging it.",
                },
                {
                  value: "standard",
                  label: "Standard review",
                  detail: "Reviews the diff in context. Faster, lighter.",
                },
              ],
            },
      publish: !hasPr
        ? null
        : {
            legend: "Where do the findings go?",
            default: "local",
            choices: [
              {
                value: "local",
                label: "Keep local",
                detail: "Findings stay in this item. Nothing leaves the machine.",
              },
              {
                value: "pr-comments",
                label: `Also post to ${prName}`,
                detail: prDraft
                  ? `Line comments on ${prName} plus a summary comment — a draft cannot take a formal review, so the summary goes as a comment. Never approves or requests changes; that stays yours.`
                  : `One review with line comments on ${prName}. Never approves or requests changes — that stays yours.`,
              },
            ],
          },
      interaction: {
        legend: "How does the round run?",
        // The item's Settings-tab default pre-selects; the human still sees
        // and can change it — an explicit choice always leaves this dialog.
        default: ["interactive", "autonomous"].includes(interactionDefault)
          ? interactionDefault
          : "ask",
        choices: [
          {
            value: "ask",
            label: "Ask me when it starts",
            detail: "The reviewer opens its session by asking interactive vs autonomous — decide there.",
          },
          {
            value: "interactive",
            label: "Interactive",
            detail:
              "Findings are triaged with you in the session; fixes and PR posts are confirmed before they happen.",
          },
          {
            value: "autonomous",
            label: "Autonomous",
            detail: "No questions — the reviewer decides alone and reports at the end.",
          },
        ],
      },
      // The round's own findings can become the next change round without a
      // second trip through the UI. Pre-authorization, not automation: the
      // reviewer still declares whether anything is worth applying, and a
      // round that answers "no" applies nothing however this is set.
      autoApply: {
        label:
          target === "plan"
            ? "Apply the findings to the plan when the round finishes"
            : "Start a fix round when this one finishes",
        detail:
          "The round decides whether its findings are worth it — cosmetic ones are reported, not applied. " +
          (target === "plan"
            ? "The current plan is frozen as a version first, so you can diff what the round changed."
            : "The current diff is frozen first, so the Timeline keeps what you reviewed.") +
          " Leave it off to read the findings and decide yourself.",
        default: autoApplyDefault,
      },
    };
  }

  /// Composer row for one PR: the chip name, marked when it is the primary —
  /// the only PR whose state moves the item.
  function prScopeRowLabel(pr) {
    const chip =
      typeof prChipLabel === "function"
        ? prChipLabel(pr)
        : `${String(pr.repo || "").split("/")[1] || ""}#${pr.number || "?"}`;
    return pr.primary ? `${chip} · primary` : chip;
  }

  /// Composer detail for one PR: enough to tell two of an item's PRs apart,
  /// plus what reviewing *that* PR means — its own diff, read from the forge.
  function prScopeRowDetail(pr) {
    const st =
      typeof prStateLabel === "function"
        ? prStateLabel(pr)
        : pr.draft
          ? "draft"
          : String(pr.state || "").toLowerCase();
    const bits = [];
    if (st) bits.push(st);
    bits.push(
      pr.primary ? "this repository's PR diff" : "that repository's diff, read from GitHub"
    );
    return `${bits.join(" · ")} — ${pr.url}`;
  }

  /// Map the composer's interaction choice to the backend's tri-state
  /// `interactive` parameter: null = unset (the skill asks in-session),
  /// true/false = the human already decided at launch.
  function interactiveParam(value) {
    if (value === "interactive") return true;
    if (value === "autonomous") return false;
    return null;
  }

  /// Label for the "Answer PR comments" action. `count` is the number of open
  /// threads at the last PR refresh — null/undefined when it was never fetched
  /// (gh unavailable, no refresh yet), in which case the label stays generic
  /// rather than claiming a number it doesn't have.
  ///
  /// "Open" means the thread does not yet end with a reply of ours: a
  /// reviewer's remark nobody answered, *and* a finding an earlier round of
  /// this item published with no decision posted under it. Both need a
  /// published answer; the second is the one a "count only other people's
  /// comments" rule hid, leaving a PR with seven remarks and no outcomes.
  function answerCommentsLabel(count) {
    if (count === 0) return "Answer PR comments · all answered";
    if (count === 1) return "Answer 1 open PR thread";
    if (typeof count === "number" && count > 1) return `Answer ${count} open PR threads`;
    return "Answer PR comments";
  }

  /// Tooltip for the same action — what the agent will do, and what the count
  /// means, including the honest "nothing open" case (the button stays:
  /// comments arrive between polls, and the agent re-fetches).
  function answerCommentsTitle(count, prName) {
    const base =
      `Spends tokens: launches an agent to work through every open thread on ` +
      `${prName} — fixing what the comment asks for where the change is ` +
      "bounded, queueing the rest as diff comments for your triage — and to " +
      "publish a reply in every thread saying what was decided, whether the " +
      "comment was right or wrong.";
    if (count === 0)
      return (
        `Every thread on ${prName} already ends with a reply from you (last ` +
        "check), so there is nothing to answer. Launch it anyway to re-check " +
        "GitHub now, or use “Post round to PR” to publish this item's findings."
      );
    return base;
  }

  /// Confirm copy before launching a respond round.
  function answerCommentsConfirm(prName, count) {
    if (count === 0)
      return (
        `Every thread on ${prName} was already answered at the last check. ` +
        "Launch an agent anyway to re-check GitHub now? It spends tokens, and " +
        "a round that finds nothing open says so and stops."
      );
    const what =
      typeof count === "number" && count > 0
        ? `${count} open thread${count > 1 ? "s" : ""}`
        : "every open thread";
    return (
      `Launch an agent to work through ${what} on ${prName}? It fixes what the ` +
      "comments ask for where the change is bounded (commits and pushes), " +
      "queues the rest as diff comments for your triage, and publishes a reply " +
      "in every thread saying what was decided — including the ones it " +
      "declines, with the reason."
    );
  }

  const api = {
    reviewRoundModel,
    prScopeRowLabel,
    prScopeRowDetail,
    interactiveParam,
    answerCommentsLabel,
    answerCommentsTitle,
    answerCommentsConfirm,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
