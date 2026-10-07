// Check runs — the pure half.
//
// One launcher for every agent pass that judges or explains a workflow item
// without moving it: the stage's review, plan vs changes, the two
// explanations, answering PR comments, the self-review. Any subset runs as one
// check run (`run.json`, driven by the backend's `workflow_run_*` commands):
// explainers alongside, judging passes one after another over the same
// iteration, then at most one change round for all of their findings. One
// pass ticked is exactly the standalone round.
//
// This module owns the catalogue (what each pass is, what it costs, whether it
// posts), the gate (which passes the stage allows), the spec sent to the
// backend, the strip's summary of a run in flight and the combined apply
// note. `app.js` owns the dialog and the strip. Contract: docs/workflows.md →
// Check runs.
(function () {
  "use strict";

  const node = typeof module !== "undefined" && module.exports;
  const deps = node
    ? { ...require("./wf-compose.js"), ...require("./wf-plan.js"), ...require("./wf-review.js") }
    : null;
  const dep = (name) => (deps ? deps[name] : window[name]);

  /// Every pass, in launch order — mirrors `workflow_run::pass_rank`, so the
  /// list reads the way the run will go. `posts` passes are never pre-ticked
  /// and never started by autopilot: they speak on GitHub in your name.
  const PASSES = [
    { id: "explain-plan", kind: "explain", posts: false },
    { id: "explain-diff", kind: "explain", posts: false },
    { id: "review", kind: "judge", posts: false },
    { id: "drift", kind: "judge", posts: false },
    { id: "respond", kind: "judge", posts: true },
    { id: "self-review", kind: "judge", posts: true },
  ];
  const BY_ID = Object.fromEntries(PASSES.map((p) => [p.id, p]));
  const order = (id) => PASSES.findIndex((p) => p.id === id);

  function passLabel(id, ctx = {}) {
    switch (id) {
      case "review":
        return ctx.reviewTarget === "plan" ? "Plan review" : "Code review";
      case "drift":
        return "Plan vs changes";
      case "explain-plan":
        return "Explain plan";
      case "explain-diff":
        return "Explain changes";
      case "respond":
        // The thread count rides the label wherever it is known.
        return dep("answerCommentsLabel")(ctx.unanswered);
      case "self-review":
        return "Self-review";
      default:
        return id;
    }
  }

  /// What a pass costs, in the words a row shows next to its name.
  function passBadge(id) {
    switch (id) {
      case "explain-plan":
      case "explain-diff":
        return "tokens · runs alongside";
      case "respond":
        return "tokens · posts replies on GitHub";
      case "self-review":
        return "tokens · posts a verdict on GitHub";
      default:
        return "tokens";
    }
  }

  function passDetail(id, ctx = {}) {
    switch (id) {
      case "review":
        return ctx.reviewTarget === "plan"
          ? "An agent judges plan.md against the real code and reports findings."
          : "An agent reviews the diff against the real code and files its findings as diff comments.";
      case "drift":
        return "Reads plan.md and the diff together and grades every divergence: intended, harmless or a problem.";
      case "explain-plan":
        return "What the plan is going to do, before it exists — a walk-through plus a drawn overview. Judges nothing.";
      case "explain-diff":
        return "What the change does — a walk-through by functional part plus a drawn overview. Judges nothing.";
      case "respond":
        return "Reads the PR's review threads, fixes the small ones, and replies on each thread.";
      case "self-review":
        return "Reviews the PR until a full pass finds nothing new, then posts approve or request changes.";
      default:
        return "";
    }
  }

  /// The passes this stage allows, in launch order. `ctx` carries the bar's
  /// own gates: `{ status, canReview, reviewTarget, hasPlan, canExplainPlan,
  /// canExplainDiff, hasPrs, canSelfReview }`.
  function availablePasses(ctx) {
    const built = !["draft", "plan-review"].includes(ctx.status);
    const out = [];
    if (ctx.canExplainPlan) out.push("explain-plan");
    if (ctx.canExplainDiff) out.push("explain-diff");
    if (ctx.canReview) out.push("review");
    if (ctx.canReview && ctx.hasPlan && built) out.push("drift");
    if (ctx.canReview && ctx.hasPrs) out.push("respond");
    if (ctx.canSelfReview) out.push("self-review");
    return out;
  }

  /// The backend `PassSpec` for a pass and its options.
  ///
  /// The stage's own review (and answering comments, which is a mode of it)
  /// names the target the launcher will derive, `plan` at plan-review and
  /// `diff` elsewhere: the run recognises its round by target, so naming any
  /// other would leave the pass waiting for a round that never comes.
  function passSpec(id, opts = {}, ctx = {}) {
    const own = ctx.reviewTarget === "plan" ? "plan" : "diff";
    const urls = Array.isArray(opts.prUrls) ? opts.prUrls : [];
    switch (id) {
      case "review":
        return {
          target: own,
          depth: opts.depth || "standard",
          publish: opts.publish || "local",
          prUrls: urls,
          focus: "",
        };
      case "drift":
        return { target: "drift", depth: "standard", publish: "local", prUrls: [], focus: "" };
      case "explain-plan":
      case "explain-diff": {
        const focus = String(opts.focus || "").trim();
        // A focused explanation digs deeper into what it was pointed at.
        return { target: id, depth: focus ? "deep" : "standard", publish: "local", prUrls: [], focus };
      }
      case "respond":
        return { target: own, depth: "standard", publish: "respond-pr-comments", prUrls: urls, focus: "" };
      case "self-review":
        return {
          target: "self-review",
          depth: opts.depth || "standard",
          publish: "pr-comments",
          prUrls: urls,
          focus: "",
        };
      default:
        return null;
    }
  }

  /// The catalogue id of a pass recorded in `run.json`.
  function passIdOf(pass) {
    const t = (pass && pass.target) || "";
    if (t === "explain-plan" || t === "explain-diff" || t === "drift" || t === "self-review") return t;
    return pass && pass.publish === "respond-pr-comments" ? "respond" : "review";
  }

  /// The strip's chips for a recommendation: every pass the recommender
  /// wants, ticked unless it posts, plus a refresh of any explanation that
  /// describes an older iteration when the comparison is among them — the
  /// three documents are read side by side.
  function recommendedSelection(step, stale = []) {
    if (!step || step.kind !== "check") return [];
    const out = step.passes.map((p) => ({
      id: p.id,
      params: p.params || {},
      reason: p.reason || "",
      ticked: !BY_ID[p.id].posts,
    }));
    if (out.some((p) => p.id === "drift")) {
      for (const s of stale) {
        if (out.some((p) => p.id === s.target)) continue;
        out.push({
          id: s.target,
          params: {},
          reason: `The ${s.label} describes an earlier iteration`,
          ticked: true,
        });
      }
    }
    return out.sort((a, b) => order(a.id) - order(b.id));
  }

  /// What autopilot may start from a selection: the ticked passes that post
  /// nothing.
  function autopilotSelection(selection) {
    return selection.filter((p) => p.ticked && !BY_ID[p.id].posts);
  }

  const isExplain = (p) => p.target === "explain-plan" || p.target === "explain-diff";
  const busy = (p) => p.state === "launching" || p.state === "running";

  /// Where a run stands, for the strip: one line plus the handles its
  /// controls need. Pure over the `run.json` batch.
  function runSummary(batch, ctx = {}) {
    const rows = batch.passes.map((p, index) => ({
      index,
      id: passIdOf(p),
      label: passLabel(passIdOf(p), ctx),
      state: p.state || "queued",
      error: p.error || "",
      explain: isExplain(p),
    }));
    const judges = rows.filter((r) => !r.explain);
    const settled = (r) => ["done", "failed", "skipped"].includes(r.state);
    const running = judges.find((r) => busy({ state: r.state }));
    const next = judges.find((r) => r.state === "queued") || null;
    const alongside = rows.filter((r) => r.explain && busy({ state: r.state }));
    const failed = rows.filter((r) => r.state === "failed");
    const apply = batch.apply || { state: "none" };
    const parts = [`Checks · ${judges.filter(settled).length} of ${judges.length} done`];
    if (running) parts.push(`${running.label.toLowerCase()} running`);
    if (next && !batch.stopping) parts.push(`next: ${next.label.toLowerCase()}`);
    for (const a of alongside) parts.push(`${a.label.toLowerCase()} alongside`);
    if (batch.stopping && next) parts.push("stopping after this one");
    if (batch.paused) parts.push("paused — a check failed");
    if (apply.state === "pending" && !apply.auto) {
      const n = (apply.keys || []).length;
      parts.push(`${n} round${n === 1 ? "" : "s"} to apply`);
    }
    if (apply.state === "pending" && apply.auto) parts.push("applying the findings");
    if (apply.state === "applying") parts.push("starting the change round");
    return {
      text: parts.join(" · "),
      rows,
      failed,
      next,
      // Stopping only means something while a judge runs and another waits.
      canStop: !!running && !!next && !batch.stopping,
      apply,
    };
  }

  /// The note for a run's combined apply: each round's own note, in order,
  /// under one line that says they are one list of work. `md` is
  /// `agent-review.md`; `keys` are `"<target>:<round>"`.
  function combinedApplyNote(md, keys) {
    const rounds = dep("agentReviewRounds")(md);
    const canonical = dep("canonicalTarget");
    const parts = (keys || []).map((key) => {
      const cut = key.lastIndexOf(":");
      const target = key.slice(0, cut);
      const n = Number(key.slice(cut + 1));
      // The last match: a heading can repeat, and the newest is the one the
      // run launched.
      const hits = rounds.filter((r) => canonical(r.target) === target && r.round === n);
      const hit = hits[hits.length - 1];
      const findings = hit ? dep("roundFindingsAt")(md, hit.index) : null;
      const as = target === "plan" ? "plan" : target === "drift" ? "drift" : "diff";
      return dep("applyReviewNote")({ round: n, target }, findings, as).trim();
    });
    if (parts.length <= 1) return parts.length ? `${parts[0]}\n` : "";
    return (
      `This change round applies ${parts.length} review rounds from one check run. ` +
      "Treat them as one list of work: where two rounds raise the same thing, do it once.\n\n" +
      parts.join("\n\n---\n\n") +
      "\n"
    );
  }

  const api = {
    PASSES,
    passLabel,
    passBadge,
    passDetail,
    availablePasses,
    passSpec,
    passIdOf,
    recommendedSelection,
    autopilotSelection,
    runSummary,
    combinedApplyNote,
  };

  if (node) module.exports = api;
  else Object.assign(window, api);
})();
