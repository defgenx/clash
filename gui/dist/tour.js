// GUI guided tour — the pure half.
//
// The TUI has had a tour since day one (`tui/widgets/tour.rs`); this is the
// GUI's. What lives here is everything testable without a DOM: the step
// list (one entry per area of the window, each anchored to an element id)
// and the placement math that keeps the tooltip on screen wherever the
// anchor sits. The overlay itself — spotlight ring, tooltip, key handling —
// is `startGuiTour` in `app.js` with the other modals.
(function () {
  "use strict";

  /// One entry per area, in reading order. `target` is the element id the
  /// spotlight rings (resolved at show time — a missing element skips the
  /// step rather than pointing at nothing).
  const TOUR_STEPS = [
    {
      id: "workspaces",
      target: "workspace-bar",
      title: "Workspaces",
      body:
        "Group your sessions and tabs into workspaces — one per project or per train of thought. " +
        "⌘N creates one; ⌘1-9 switches. Layout and open tabs survive a restart.",
    },
    {
      id: "new-session",
      target: "new-session-btn",
      title: "Start a session",
      body:
        "Launches a Claude Code session in any directory (⌘T). " +
        "Tick “git worktree” to give it an isolated branch and folder.",
    },
    {
      id: "sessions",
      target: "session-list",
      title: "Sessions",
      body:
        "Every session, grouped by state. Click to open its terminal, right-click for " +
        "actions, ⟳ hot-restarts it on the newest claude binary. Sessions started " +
        "elsewhere show up as “wild” — one click adopts them.",
    },
    {
      id: "inbox",
      target: "inbox-btn",
      title: "Inbox",
      body:
        "One list of everything waiting on you (⌘I) — sessions asking for approval or a " +
        "next message, workflow items parked on a decision, PRs with unanswered comments. " +
        "Blocked work first, oldest first. The count turns red when something is blocked.",
    },
    {
      id: "workflows",
      target: "wf-section",
      title: "Workflows",
      body:
        "A plan → review → implement → PR pipeline per work item, driven by agents and " +
        "gated by your decisions. An agent review is applied in one click — it becomes " +
        "the next round and the plan is versioned, so you can diff what changed. ⊞ opens " +
        "the board, ⇄ the PR dashboard across all projects.",
    },
    {
      id: "scratches",
      target: "notes-section",
      title: "Scratches",
      body:
        "Free-form notes in a folder tree — drag to reorganize, right-click to copy a " +
        "path or open one in your editor. They live in ~/.claude/clash/scratch as plain files.",
    },
    {
      id: "teams",
      target: "teams-section",
      title: "Teams",
      body: "Agent teams from Claude Code: members, their tasks and inboxes, at a glance.",
    },
    {
      id: "tabs",
      target: "topbar",
      title: "Tabs & panes",
      body:
        "Sessions, terminals and browser tabs live up here. ⌘D splits the view into " +
        "panes (drag the gutters to resize), ⌘⇧T opens a shell, ⌘⇧B a browser tab.",
    },
    {
      id: "terminal",
      target: "terminal-host",
      title: "The terminal",
      body:
        "Sessions render here. ⌥-drag selects text even while Claude is using the mouse, " +
        "⌘C copies, links are clickable. Everything else is your terminal as usual.",
    },
    {
      id: "settings",
      target: "sidebar-footer",
      title: "Settings",
      body:
        "Theme, fonts, directories, workflow webhooks — shared settings persist in " +
        "config.toml, so the TUI sees them too. You can rerun this tour from here anytime.",
    },
  ];

  /// Where the tooltip goes relative to the anchor rectangle. Tries the
  /// preferred side, then the others (right → left → bottom → top), and
  /// clamps into the viewport so a corner anchor never pushes it off screen.
  /// All rects are `{ left, top, width, height }`; returns
  /// `{ left, top, side }` for the tooltip's top-left corner.
  function tourPlacement(anchor, tip, viewport, preferred = "right", gap = 12) {
    const fits = {
      right: anchor.left + anchor.width + gap + tip.width <= viewport.width,
      left: anchor.left - gap - tip.width >= 0,
      bottom: anchor.top + anchor.height + gap + tip.height <= viewport.height,
      top: anchor.top - gap - tip.height >= 0,
    };
    const order = [preferred, "right", "left", "bottom", "top"];
    const side = order.find((s) => fits[s]) || "bottom";
    let left;
    let top;
    if (side === "right" || side === "left") {
      left = side === "right" ? anchor.left + anchor.width + gap : anchor.left - gap - tip.width;
      top = anchor.top + anchor.height / 2 - tip.height / 2;
    } else {
      top = side === "bottom" ? anchor.top + anchor.height + gap : anchor.top - gap - tip.height;
      left = anchor.left + anchor.width / 2 - tip.width / 2;
    }
    // Clamp with a small margin; the viewport wins over the anchor.
    const m = 8;
    left = Math.min(Math.max(left, m), Math.max(m, viewport.width - tip.width - m));
    top = Math.min(Math.max(top, m), Math.max(m, viewport.height - tip.height - m));
    return { left, top, side };
  }

  const api = { TOUR_STEPS, tourPlacement };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
