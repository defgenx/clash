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
  /// (there is nothing to talk to).
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
  } = {}) {
    const prName = prNumber ? `#${prNumber}` : "the PR";
    return {
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

  /// Map the composer's interaction choice to the backend's tri-state
  /// `interactive` parameter: null = unset (the skill asks in-session),
  /// true/false = the human already decided at launch.
  function interactiveParam(value) {
    if (value === "interactive") return true;
    if (value === "autonomous") return false;
    return null;
  }

  /// Label for the "Answer PR comments" action. `count` is the unanswered
  /// thread count from the last PR refresh — null/undefined when never
  /// fetched (gh unavailable, no refresh yet), in which case the label
  /// stays generic rather than claiming a number it doesn't have.
  function answerCommentsLabel(count) {
    if (count === 1) return "Answer 1 PR comment";
    if (typeof count === "number" && count > 1) return `Answer ${count} PR comments`;
    return "Answer PR comments";
  }

  /// Tooltip for the same action — says what the agent will do and what the
  /// count means, including the honest "none right now" case (the button
  /// stays: comments arrive between polls, and the agent re-fetches).
  function answerCommentsTitle(count, prName) {
    const base =
      `Spend tokens: launch an agent to read ${prName}'s review comments, ` +
      "fix the trivial ones, and reply on each thread. Non-trivial ones land " +
      "in this item's comment queue.";
    if (count === 0) return `${base} None unanswered at last check.`;
    return base;
  }

  /// Confirm copy before launching a respond round.
  function answerCommentsConfirm(prName, count) {
    const what =
      typeof count === "number" && count > 0
        ? `its ${count} unanswered review comment${count > 1 ? "s" : ""}`
        : "its review comments";
    return (
      `Launch an agent to read ${prName} and answer ${what}? ` +
      "It fixes trivial findings with commits, replies on each thread, and " +
      "mirrors the rest into this item's comment queue for your triage."
    );
  }

  const api = {
    reviewRoundModel,
    interactiveParam,
    answerCommentsLabel,
    answerCommentsTitle,
    answerCommentsConfirm,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
