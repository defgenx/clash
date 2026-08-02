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

  const api = {
    changeRequestTemplate,
    composerPlaceholder,
    annotationsMarkdown,
    canSubmitChangeRequest,
    draftKey,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
