// Change-request composer — the pure half.
//
// The note a human writes when requesting changes is not a form field: it is
// appended verbatim to `review.md` under `## Iteration N`, and the
// `clash-workflow` skill's first instruction is to read the latest `## Iteration`
// section. So this text *is* the prompt for the next agent round — the
// highest-leverage string in the pipeline, and it used to be typed into a
// one-line `<input>`.
//
// The template, the annotation summary and the submit rule live here so they are
// testable without a DOM (same shape as `wf-diff.js`). The dialog itself is in
// `app.js` with the other modals.
(function () {
  "use strict";

  /// Scaffold for a structured change request, per review target.
  ///
  /// Deliberately *inserted on demand* rather than pre-filled: most rounds are a
  /// sentence ("rename this, it shadows the outer binding"), and forcing three
  /// headings onto those would make `review.md` noisier rather than clearer. One
  /// click away is the right cost for the thorough path.
  ///
  /// The prompts are HTML comments, which markdown renders as nothing — so a
  /// half-filled template still reads cleanly in the rendered doc.
  function changeRequestTemplate(target) {
    const what =
      target === "plan"
        ? "What to change in the plan"
        : "What to change";
    return [
      `## ${what}`,
      "",
      "<!-- Be specific: the behaviour or code that must end up different. -->",
      "",
      "## Why",
      "",
      "<!-- The reasoning. This is what lets the agent make the right call on the",
      "     details you did not spell out. -->",
      "",
      "## Out of scope",
      "",
      "<!-- Anything it should NOT touch this round. -->",
      "",
    ].join("\n");
  }

  /// The dialog's opening line — the answer to "what do I write here, and
  /// must I write anything?", adapting to what the round already carries.
  /// The plumbing (review.md, iteration N) lives on a dim second line:
  /// humans decide with purpose, not file names.
  function composerIntro(target, openCount) {
    if (target === "plan") {
      return "Describe what should change in the plan. This becomes the agent's instructions for the next revision round.";
    }
    if (openCount === 1) {
      return "Your diff comment below is the work order for the next agent round — you can send it as is. Add a note only if it needs framing: priorities, context, what's out of scope.";
    }
    if (openCount > 1) {
      return `Your ${openCount} diff comments below are the work order for the next agent round — you can send them as is. Add a note only if they need framing: priorities, what ties them together, what's out of scope.`;
    }
    return "Describe what should change and why. This becomes the agent's instructions for the next round.";
  }

  /// Caption over the note field — it says "optional" out loud when the
  /// comments already carry the work, which is the answer to the blank
  /// intimidating textarea.
  function noteCaption(target, openCount) {
    return target !== "plan" && openCount > 0 ? "Note — optional" : "Note";
  }

  /// The submit button names its consequence and tracks the After-recording
  /// choice — the moment of commitment is where the outcome must be visible.
  function submitLabel(launchNow) {
    return launchNow ? "Record & launch agent" : "Record round";
  }

  /// Placeholder shown in an empty composer — the shape of a good request,
  /// without occupying the field.
  function composerPlaceholder(target, openCount) {
    if (target === "plan") {
      return "What should change in the plan, and why?\n\nMarkdown. This becomes the agent's instructions for the next round.";
    }
    if (openCount > 0) {
      return (
        "Optional framing for the comments below — what ties them together, what to\n" +
        "prioritise, what to leave alone.\n\n" +
        "Markdown. This becomes the agent's instructions for the next round."
      );
    }
    return "What should change, and why?\n\nMarkdown. This becomes the agent's instructions for the next round.";
  }

  /// The open annotations as a markdown list, matching how
  /// `append_review_iteration` will render them into `review.md`.
  ///
  /// Shown read-only beside the composer: the count alone ("3 comments will be
  /// sent") tells you nothing about whether your note duplicates them.
  function annotationsMarkdown(annotations) {
    if (!Array.isArray(annotations) || !annotations.length) return "";
    return annotations
      .map((a) => {
        const file = a && a.file ? String(a.file) : "?";
        const line = a && (a.line || a.line === 0) ? a.line : "?";
        const body = a && a.body ? String(a.body).trim() : "";
        return `- \`${file}:${line}\` — ${body}`;
      })
      .join("\n");
  }

  /// Whether a change request can be sent.
  ///
  /// A round with neither a note nor a single annotation gives the agent nothing
  /// to act on; it would burn a session and come back unchanged. `reason` is the
  /// message to show, so the caller never invents its own wording.
  function canSubmitChangeRequest({ note = "", openCount = 0, target = "diff" } = {}) {
    if (note.trim()) return { ok: true, reason: null };
    if (target !== "plan" && openCount > 0) return { ok: true, reason: null };
    return {
      ok: false,
      reason:
        target === "plan"
          ? "Describe what should change in the plan — the agent needs something to act on."
          : "Add at least one diff comment or a note — the agent needs something to act on.",
    };
  }

  /// Draft key for an item. Drafts are per item, so two items being reviewed in
  /// parallel don't share one buffer.
  function draftKey(project, slug) {
    return `${project}/${slug}`;
  }

  /// Every `## Review <n>` round of agent-review.md, in file order:
  /// `{ index, round, target, heading }`. Drives the composer's round picker,
  /// so a change request can pull findings from ANY round, not just the latest.
  ///
  /// `index` is the identity used for lookups, not `round`: round numbers
  /// restart per target, so "Review 1" appears once for the plan and once for
  /// the diff, and a by-number lookup would silently take the wrong one.
  function agentReviewRounds(md) {
    const out = [];
    for (const line of String(md || "").split("\n")) {
      const m = /^## Review (\d+)\b\s*[—-]?\s*(.*)$/.exec(line);
      if (!m) continue;
      const heading = m[2].trim();
      const target = (heading.split(/[·\s]+/)[0] || "").toLowerCase();
      out.push({ index: out.length, round: Number(m[1]), target, heading });
    }
    return out;
  }

  /// Human label for one round: the phase it belongs to and its number within
  /// that phase. "Round 1" on its own is ambiguous now that numbers restart.
  function roundLabel(r) {
    const n = (r && r.round) || 1;
    const t = (r && r.target) || "";
    if (t === "plan") return `Plan review ${n}`;
    if (t === "diff") return `Code review ${n}`;
    // Both explainer targets, under their current and original spellings:
    // naming only one left plan explanations reading as "Review 3", a label
    // that says a judgement was made when none was.
    if (t === "explain-diff" || t === "structure") return `Changes explained ${n}`;
    if (t === "explain-plan" || t === "blueprint") return `Plan explained ${n}`;
    return `Review ${n}`;
  }

  /// One round of agent-review.md, reshaped as change-request material — what
  /// "Insert round N findings" pastes into the composer. This is the bridge
  /// from a review round to applied work: the note is the next round's
  /// prompt, so the findings must be able to become that note without
  /// retyping.
  ///
  /// Subsections that are records rather than instructions are dropped:
  /// `### Published` (what left the machine), `### Fixed in this round`
  /// (already done), `### Dismissed in triage` (explicitly not work).
  /// Returns `{ round, text }`, or null when the round doesn't exist or has
  /// nothing usable. When several sections share the number (should not
  /// happen), the last one wins — same rule as the latest-round parser.
  function roundFindingsAt(md, index) {
    const lines = String(md || "").split("\n");
    let start = -1;
    let seen = -1;
    for (let i = 0; i < lines.length; i++) {
      if (!/^## Review (\d+)\b/.test(lines[i])) continue;
      seen += 1;
      if (seen === index) {
        start = i;
        break;
      }
    }
    if (start < 0) return null;
    let end = lines.length;
    for (let i = start + 1; i < lines.length; i++) {
      if (/^## /.test(lines[i])) {
        end = i;
        break;
      }
    }
    const OMIT = new Set([
      "### Published",
      "### Fixed in this round",
      "### Dismissed in triage",
    ]);
    const out = [];
    let skipping = false;
    for (const line of lines.slice(start + 1, end)) {
      if (/^###? /.test(line)) skipping = OMIT.has(line.trim());
      if (!skipping) out.push(line);
    }
    const text = out.join("\n").replace(/\n{3,}/g, "\n\n").trim();
    const meta = agentReviewRounds(md)[index] || {};
    return text ? { index, round: meta.round || 0, target: meta.target || "", text } : null;
  }

  /// The latest round's findings — the last section of the file. Kept as its
  /// own name because "insert the latest" is the common path, and because the
  /// round being applied is always this one.
  function latestAgentRoundFindings(md) {
    const rounds = agentReviewRounds(md);
    if (!rounds.length) return null;
    return roundFindingsAt(md, rounds.length - 1);
  }

  const api = {
    changeRequestTemplate,
    composerIntro,
    noteCaption,
    submitLabel,
    composerPlaceholder,
    annotationsMarkdown,
    canSubmitChangeRequest,
    draftKey,
    agentReviewRounds,
    roundLabel,
    roundFindingsAt,
    latestAgentRoundFindings,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
