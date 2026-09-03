// Pure pipeline model for the workflow stepper — which stages this item's
// mode has, where the item currently is, and the loop/side-trip context
// (change rounds, agent review rounds). No DOM here: app.js renders the
// model, gui/tests/wf_pipeline.test.js exercises it.
//
// The stepper answers "where am I and what happened", not "what may I click"
// — actions stay in the action bar. Two deliberate simplifications keep it
// readable: pr-draft and pr-ready collapse into one optional PR node (the
// node's sublabel says which), and the looping statuses (changes-requested,
// implementing, reviewing) render as the node they do work FOR, with the loop
// spelled out in the sublabel.
(function () {
  /// Main-line nodes for an entry mode. `review-only` has no plan phases and
  /// no PR ceremony of its own; `from-plan` starts at plan review.
  function wfPipelineNodes(mode) {
    if (mode === "review-only")
      return [
        { id: "diff-review", label: "Diff review" },
        { id: "done", label: "Done" },
      ];
    const nodes = [
      { id: "planning", label: "Plan" },
      { id: "plan-review", label: "Plan review" },
      { id: "implementing", label: "Implement" },
      { id: "diff-review", label: "Diff review" },
      { id: "pr", label: "PR" },
      { id: "done", label: "Done" },
    ];
    return mode === "from-plan" ? nodes.slice(1) : nodes;
  }

  /// The node a status lives on. Looping statuses anchor to the node whose
  /// artifact they are producing; `reviewing` anchors to where the round will
  /// hand back (`returnStatus`).
  function wfPipelineAnchor(status, { returnStatus = null, mode = "full" } = {}) {
    switch (status) {
      case "draft":
      case "planning":
        return "planning";
      case "plan-review":
        return "plan-review";
      case "changes-requested":
      case "implementing":
        // In review-only there is no Implement node — the change loop hangs
        // off the diff-review node it returns to.
        return mode === "review-only" ? "diff-review" : "implementing";
      case "diff-review":
        return "diff-review";
      case "reviewing":
        return wfPipelineAnchor(returnStatus || "diff-review", { mode });
      case "pr-draft":
      case "pr-ready":
        return "pr";
      case "done":
      case "abandoned":
        return "done";
      default:
        return "diff-review";
    }
  }

  /// Build the full stepper model from the fields the GUI has on an item.
  /// Input: { status, mode, reviewRound, iteration, prDraft (bool|null),
  ///          reviewReturnStatus, reviewRoundInFlight }.
  /// Output: { nodes: [{id, label, state, sub}], chips: [string], dead: bool }
  ///   state ∈ "done" | "current" | "future".
  function wfPipelineModel({
    status = "draft",
    mode = "full",
    reviewRound = 0,
    iteration = 1,
    prDraft = null,
    reviewReturnStatus = null,
    reviewRoundInFlight = 0,
  } = {}) {
    const nodes = wfPipelineNodes(mode);
    const anchor = wfPipelineAnchor(status, { returnStatus: reviewReturnStatus, mode });
    let idx = nodes.findIndex((n) => n.id === anchor);
    if (idx < 0) idx = 0;
    const terminal = status === "done" || status === "abandoned";

    const out = nodes.map((n, i) => {
      const state = terminal || i < idx ? "done" : i === idx ? "current" : "future";
      let sub = null;
      if (i === idx && !terminal) {
        if (status === "reviewing")
          sub = `⌕ review round ${reviewRoundInFlight || reviewRound || 1} in flight`;
        else if (status === "changes-requested") sub = `✎ change round it.${iteration} queued`;
        else if (status === "implementing") sub = `agent working · it.${iteration}`;
        else if (status === "pr-draft") sub = "draft";
        else if (status === "pr-ready") sub = "ready";
        else if (status === "draft") sub = "not started";
      }
      return { id: n.id, label: n.label, state, sub };
    });

    const chips = [];
    if (iteration > 1) chips.push(`↻ ${iteration - 1} change round${iteration > 2 ? "s" : ""}`);
    if (reviewRound > 0)
      chips.push(`⌕ ${reviewRound} agent review${reviewRound > 1 ? "s" : ""}`);
    if (status !== "done" && prDraft !== null)
      chips.push(prDraft ? "PR draft open" : "PR ready");

    return { nodes: out, chips, dead: status === "abandoned" };
  }

  /// The stages this item may be sent **back** to, nearest first.
  ///
  /// Mirrors `WorkflowStatus::rewind_targets` in src/domain/workflow.rs, which
  /// is the authority — the command validates there, this decides what the
  /// picker and the stepper offer. Only the *parked* stages are candidates:
  /// the three agent-owned ones (`planning`, `implementing`, `reviewing`) are
  /// not somewhere a human can move an item to, and `changes-requested` is
  /// queued by requesting changes, which records the note the agent reads.
  function wfRewindTargets(status, mode) {
    const LINE = ["draft", "plan-review", "diff-review", "pr-draft", "pr-ready"];
    const at = {
      draft: 0,
      "plan-review": 1,
      "changes-requested": 2,
      "diff-review": 2,
      "pr-draft": 3,
      "pr-ready": 4,
      done: LINE.length,
      abandoned: LINE.length,
    }[status];
    if (at == null) return []; // planning / implementing / reviewing / unknown
    const hasPlan = mode !== "review-only";
    return LINE.slice(0, at)
      .reverse()
      .filter((s) =>
        s === "draft" ? hasPlan && mode !== "from-plan" : s === "plan-review" ? hasPlan : true
      );
  }

  /// One line about what a stage *is*, for the "move back to…" picker. A bare
  /// status name is a label; this is the reason to pick it.
  function wfStageBlurb(status) {
    return {
      draft: "Nothing written yet — planning starts from scratch",
      "plan-review": "Read and revise plan.md before any code is written",
      "diff-review": "Review the code as it stands, comment on lines, request changes",
      "pr-draft": "The draft PR is the thing under validation",
      "pr-ready": "The PR is open for review",
    }[status] || "";
  }

  /// The status a stepper node stands for, when clicking it can move the item
  /// there. The two agent nodes have no parked equivalent (you cannot move an
  /// item *into* an agent's hands), and the PR node covers two statuses — it
  /// stands for whichever of them is actually behind the item.
  function wfNodeRewindStatus(nodeId, targets) {
    if (nodeId === "pr") return targets.find((t) => t === "pr-draft" || t === "pr-ready") || null;
    return targets.includes(nodeId) ? nodeId : null;
  }

  const api = {
    wfPipelineNodes,
    wfPipelineAnchor,
    wfPipelineModel,
    wfRewindTargets,
    wfStageBlurb,
    wfNodeRewindStatus,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
