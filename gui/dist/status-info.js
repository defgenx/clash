// Status vocabulary shared by the desktop GUI and the phone page
// (gui/dist/remote/), so a session or an item reads the same on both.
(function () {
  "use strict";

  // Mirror of SessionStatus serde values (Stashed -> "idle", Done -> "done").
  // Same status vocabulary as the TUI (src/adapters/format.rs).
  function statusInfo(s) {
    if (s.is_running || s.status === "Starting") {
      switch (s.status) {
        case "Prompting":
          return { cls: "prompting", icon: "◆", label: "PROMPTING" };
        case "Waiting":
          return { cls: "waiting", icon: "◉", label: "WAITING" };
        case "Thinking":
          return { cls: "thinking", icon: "◉", label: "THINKING" };
        case "Starting":
          return { cls: "starting", icon: "◔", label: "STARTING" };
        default:
          return { cls: "running", icon: "⟳", label: "RUNNING" };
      }
    }
    if (s.status === "Errored") return { cls: "errored", icon: "✗", label: "ERRORED" };
    if (s.status === "idle") return { cls: "stashed", icon: "○", label: "STASHED" };
    return { cls: "done", icon: "✓", label: "DONE" };
  }

  function statusClass(s) {
    return statusInfo(s).cls;
  }

  function sectionOf(s) {
    if (s.is_running || s.status === "Starting") return "ACTIVE";
    if (s.status === "Errored") return "FAILED";
    if (s.status === "idle") return "STASHED";
    return "DONE";
  }

  function displayName(s) {
    return s.name || s.summary || s.first_prompt || s.id.slice(0, 8);
  }

  function wfStatusInfo(status) {
    switch (status) {
      case "draft":
        return { cls: "wf-draft", icon: "○", label: "DRAFT" };
      case "planning":
        return { cls: "wf-active", icon: "◔", label: "PLANNING" };
      case "plan-review":
        return { cls: "wf-review", icon: "◆", label: "PLAN REVIEW" };
      case "changes-requested":
        return { cls: "wf-changes", icon: "↺", label: "CHANGES REQUESTED" };
      case "implementing":
        return { cls: "wf-active", icon: "⟳", label: "IMPLEMENTING" };
      case "reviewing":
        return { cls: "wf-active", icon: "⌕", label: "REVIEWING" };
      case "diff-review":
        return { cls: "wf-review", icon: "◆", label: "DIFF REVIEW" };
      case "pr-draft":
        return { cls: "wf-pr", icon: "⇄", label: "PR DRAFT" };
      case "pr-ready":
        return { cls: "wf-pr-ready", icon: "⇄", label: "PR READY" };
      case "done":
        return { cls: "wf-done", icon: "✓", label: "DONE" };
      case "abandoned":
        return { cls: "wf-dead", icon: "⌀", label: "ABANDONED" };
      default:
        return { cls: "wf-draft", icon: "?", label: String(status || "?").toUpperCase() };
    }
  }

  /// How each agent CLI (`Session.agent`) is named: the row badge, its
  /// tooltip, and the name used in prose.
  const AGENT_BADGES = {
    claude: { short: "CC", name: "Claude Code", title: "Runs on Claude Code" },
    omp: { short: "OMP", name: "OMP", title: "Runs on OMP (oh-my-pi)" },
    hermes: { short: "HM", name: "Hermes", title: "Runs on Hermes Agent" },
  };

  /// "just now" / "5m ago" / "3h ago" / "2d ago".
  function wfAgo(ms) {
    if (!ms) return "";
    const s = Math.max(0, (Date.now() - ms) / 1000);
    if (s < 90) return "just now";
    if (s < 5400) return `${Math.round(s / 60)}m ago`;
    if (s < 129600) return `${Math.round(s / 3600)}h ago`;
    return `${Math.round(s / 86400)}d ago`;
  }

  const api = { statusInfo, statusClass, sectionOf, displayName, wfStatusInfo, wfAgo, AGENT_BADGES };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
