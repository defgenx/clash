// Pure model of "Clean up worktrees…" and the done-item cleanup toast: what
// the dialog lists, in which order, what it says, and what a run reports. No
// DOM and no IO: app.js renders it (`wfCleanWorktrees`, the
// `workflow-worktrees-cleaned` listener), gui/tests/wt_cleanup.test.js
// exercises it. The verdicts are the backend's
// (`application::worktree_cleanup`); nothing here decides what is safe.
(function () {
  /// "1.2 GB" / "340 MB"; empty when unmeasured.
  function wtBytes(b) {
    if (b == null) return "";
    if (b >= 1 << 30) return `${(b / (1 << 30)).toFixed(1)} GB`;
    return `${Math.round(b / (1 << 20))} MB`;
  }

  /// A path shown relative to the scanned root when it is inside it.
  function wtRel(path, root) {
    const r = String(root || "").replace(/\/+$/, "");
    return r && path.startsWith(r + "/") ? path.slice(r.length + 1) : path;
  }

  const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

  /// The dialog for a preview (`worktree_cleanup_preview`): `empty` is the
  /// message to show instead when nothing can go; otherwise `message`,
  /// `detail` and `items` for `uiCheckChoice` — removable first, each
  /// pre-ticked, the rest listed with their reason and never tickable.
  function wtCleanupDialog(preview) {
    const cands = preview.candidates || [];
    const removable = cands.filter((c) => c.removable);
    if (!removable.length) {
      return {
        empty: cands.length
          ? `Nothing to clean up in ${preview.root}: every one of its ${plural(cands.length, "worktree")} ` +
            "is kept (in use, uncommitted changes or unpushed commits)."
          : `No git worktrees in ${preview.root}.`,
      };
    }
    const total = removable.reduce((n, c) => n + (c.bytes || 0), 0);
    const kept = cands.length - removable.length;
    const order = [...cands].sort(
      (a, b) => Number(b.removable) - Number(a.removable) || a.path.localeCompare(b.path),
    );
    return {
      empty: null,
      message: `Clean up worktrees in ${preview.root}`,
      detail:
        `${removable.length} can go (${wtBytes(total) || "size unknown"}): clean, pushed, and used by no open ` +
        `item or running session.${kept ? ` ${kept} kept, shown with why — they can't be ticked.` : ""} ` +
        "Branches always stay.",
      items: order.map((c) => ({
        label: wtRel(c.path, preview.root),
        detail: [c.branch || "detached", wtBytes(c.bytes), c.reason].filter(Boolean).join(" · "),
        value: c.path,
        checked: c.removable,
        disabled: !c.removable,
      })),
    };
  }

  /// What a run (`worktree_cleanup_run`) reports: one toast line, and the
  /// failures spelled out when there are any (null otherwise).
  function wtRunReport(out, preview) {
    const cands = preview.candidates || [];
    const freed = cands
      .filter((c) => out.removed.includes(c.path))
      .reduce((n, c) => n + (c.bytes || 0), 0);
    const size = wtBytes(freed);
    const bits = [`Removed ${plural(out.removed.length, "worktree")}${size ? ` (${size})` : ""}`];
    if (out.kept.length) bits.push(`${out.kept.length} kept — changed since the preview`);
    if (out.failed.length) bits.push(`${out.failed.length} failed`);
    return {
      toast: bits.join(" · "),
      failures: out.failed.length
        ? "Some worktrees could not be removed:\n\n" +
          out.failed.map(([p, e]) => `${wtRel(p, preview.root)}: ${e}`).join("\n")
        : null,
    };
  }

  /// The toast for an item that reached done and had its worktrees cleaned
  /// up (`workflow-worktrees-cleaned`); null when there is nothing to say.
  function wtDoneToast(payload) {
    const out = (payload && payload.outcome) || {};
    const removed = out.removed || [];
    const kept = out.kept || [];
    const failed = out.failed || [];
    const bits = [];
    if (removed.length) bits.push(`removed ${plural(removed.length, "worktree")}`);
    if (kept.length) bits.push(`kept ${kept.length}: ${kept.map(([, why]) => why).join("; ")}`);
    if (failed.length) bits.push(`${failed.length} could not be removed`);
    if (!bits.length) return null;
    return `“${payload.title || payload.item}” is done — ${bits.join(" · ")}`;
  }

  const api = { wtBytes, wtRel, wtCleanupDialog, wtRunReport, wtDoneToast };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
