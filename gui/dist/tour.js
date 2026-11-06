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
      body: {
        classic:
          "Group your sessions and tabs into workspaces — one per project or per train of thought. " +
          "⌘N creates one; ⌘1-9 switches; drag one to reorder, right-click to rename or close it. " +
          "Layout and open tabs survive a restart.",
        rail:
          "Each workspace is a button on the rail, with its initials — one per project or per train " +
          "of thought. ＋ or ⌘N creates one, ⌘1-9 switches, drag one to reorder, right-click renames " +
          "or closes it. Layout and open tabs survive a restart.",
      },
    },
    {
      id: "new-session",
      target: "new-session-btn",
      title: "Start a session",
      body:
        "Launches a Claude Code or OMP session in any directory (⌘T) — pick the agent at the top " +
        "of the dialog. Tick “git worktree” to give it an isolated branch and folder.",
    },
    {
      id: "sessions",
      target: "session-list",
      title: "Sessions",
      body: {
        classic:
          "Every session, grouped by state. Click to open its terminal, right-click for " +
          "actions, ⟳ hot-restarts it on the newest claude binary. Sessions started " +
          "elsewhere show up as “wild” — one click adopts them.",
        rail:
          "The Sessions view: every session, grouped by state. Click to open its terminal, " +
          "right-click for actions, ⟳ hot-restarts it. The rail switches between Sessions, " +
          "Workflows, Scratches and Teams; clicking the open one folds the panel (⌘B).",
        tabbed:
          "The Sessions view: every session, grouped by state. Click to open its terminal, " +
          "right-click for actions, ⟳ hot-restarts it. The tabs above switch between Sessions, " +
          "Workflows, Scratches and Teams, each taking the sidebar's full height.",
        flyout:
          "The Sessions view slides out from the rail: every session, grouped by state. Pick one " +
          "and the panel slides away so the terminals keep the whole width; Esc or a click " +
          "outside does the same, and ⌘B brings it back.",
      },
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
      body: {
        classic:
          "A plan → review → implement → PR pipeline per work item, driven by agents and gated " +
          "by your decisions. An item opens as a Workbench: its documents grouped on the left, " +
          "the next step on top. ⊞ opens the board, ⇄ the PR dashboard across all projects.",
        rail:
          "A plan → review → implement → PR pipeline per work item, driven by agents and gated " +
          "by your decisions. Its rail icon counts the items waiting on you. An item opens as a " +
          "Workbench: documents on the left, the next step on top. ⊞ board, ⇄ PR dashboard.",
      },
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
      id: "files",
      target: "files-btn",
      title: "Files",
      body: {
        classic:
          "A file explorer for the focused session's folder (⌘E), with git status on every " +
          "file. ⌘P finds a file; click to preview it, right-click for its changes, history, " +
          "blame or an @-mention. The commit button lists only changed files. It docks on the " +
          "right, beside the Inspector if that is open too.",
        rail:
          "A file explorer for the focused session's folder (⌘E), with git status on every " +
          "file and ⌘P to find one; right-click a file for its changes, history or blame. It " +
          "docks on the right one panel at a time: its header switches between Files and the " +
          "Inspector.",
        flyout:
          "A file explorer for the focused session's folder (⌘E), with git status on every " +
          "file and ⌘P to find one; right-click a file for its changes, history or blame. It " +
          "opens as a drawer over the terminals; Esc or a click outside closes it.",
      },
    },
    {
      id: "inspector",
      target: "details-btn",
      title: "Inspector",
      body: {
        classic:
          "ⓘ opens the focused session at a glance: status, agent, branch, where it runs " +
          "(click a path to copy it), and one-click conversation, subagents, diff, IDE and " +
          "ports. It docks on the right and can stay open beside Files.",
        rail:
          "ⓘ opens the focused session at a glance: status, agent, branch, where it runs " +
          "(click a path to copy it), and one-click conversation, subagents, diff, IDE and " +
          "ports. It shares the right side with Files, one at a time.",
        flyout:
          "ⓘ opens the focused session at a glance: status, agent, branch, where it runs " +
          "(click a path to copy it), and one-click conversation, subagents, diff, IDE and " +
          "ports — as a drawer over the terminals, like Files.",
      },
    },
    {
      id: "tabs",
      target: "topbar",
      title: "Tabs & panes",
      body:
        "Sessions, terminals and browser tabs live up here. ⌘D splits the view into " +
        "panes, or drag a tab onto a pane: the highlighted half is where it splits. " +
        "The buttons on the right are grouped: new tab, stash all, panes, then the " +
        "Files and Inspector panels, lit while open.",
    },
    {
      id: "terminal",
      target: "terminal-host",
      title: "The terminal",
      body:
        "Sessions render here. ⌥-drag selects text even while the agent is using the mouse, " +
        "⌘C copies, links are clickable. Everything else is your terminal as usual.",
    },
    {
      id: "settings",
      target: "sidebar-footer",
      title: "Settings",
      body: {
        classic:
          "Quick settings live here; ⚙ All settings… (⌘,) opens every setting by topic — " +
          "theme, terminal, paths, workflows, models, trackers. Appearance → Layout rearranges " +
          "this window (Classic, Rail, Flyout); Workflow item view picks Workbench or Tabs. " +
          "Rerun this tour from Settings → General.",
        rail:
          "Quick settings open from the ⚙ at the foot of the rail; ⚙ All settings… (⌘,) opens " +
          "every setting by topic — theme, terminal, paths, workflows, models, trackers. " +
          "Appearance → Layout rearranges this window (Classic, Rail, Flyout); Workflow item " +
          "view picks Workbench or Tabs. Rerun this tour from Settings → General.",
      },
    },
  ];

  /// The window tour for one sidebar layout (layout.js ids). A step's body
  /// is a string or `{ classic, rail, flyout }`; Flyout falls back to Rail's
  /// text where it says nothing of its own (it is a rail layout too).
  function windowTourSteps(layout) {
    const pick = (v) => {
      if (!v || typeof v !== "object") return v;
      return v[layout] || (layout === "flyout" ? v.rail : null) || v.classic;
    };
    return TOUR_STEPS.map((s) => ({ id: s.id, target: s.target, title: s.title, body: pick(s.body) }));
  }

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

  /// The tour of a workflow item's tab — run the first time an item opens,
  /// replayable from the **?** in its header and Settings → General. Anchors
  /// are selectors inside the item tab (several items can be open, so ids
  /// would collide). A step may differ by item view: `selector` and `body`
  /// are either a string or `{ workbench, tabs }`; a step naming `views`
  /// appears only in those. A missing anchor skips its step (no review strip
  /// on an item never reviewed).
  const ITEM_TOUR_STEPS = [
    {
      id: "item-head",
      selector: ".wf-head",
      title: "The item",
      body:
        "Its title (double-click to rename), its stage, and its pull requests — right-click " +
        "a PR chip for that PR's own actions: open it, mark it ready, post a review.",
    },
    {
      id: "item-pipeline",
      selector: ".wf-pipeline",
      title: "Where it stands",
      body:
        "Every stage this item goes through. A ✓ stage behind it is clickable: it moves the " +
        "item back there, deleting nothing. The chips count its change rounds and agent reviews.",
    },
    {
      id: "item-documents",
      selector: { workbench: ".wf-outline", tabs: ".wf-subbar" },
      title: "Its documents",
      body: {
        workbench:
          "Grouped by what they are: Build (the plan, the diff), Feedback (your change " +
          "requests, the agent reviews, the hand-off), Explained, History. Badges count rounds " +
          "and open comments; a dim entry is not written yet.",
        tabs:
          "One tab per document: the plan, your change requests, the agent reviews, the " +
          "explanations, the diff with its comments, the timeline. Counts ride on the labels.",
      },
    },
    {
      id: "item-next",
      selector: ".wf-actions",
      title: "What now",
      body: {
        workbench:
          "The suggested next step and why, this step's actions (🔍 Check… runs reviews and " +
          "explanations), then the decisions that move the item. Back, share and abandon are " +
          "under More.",
        tabs:
          "The suggested next step and why, this step's actions (🔍 Check… runs reviews and " +
          "explanations), the decisions that move the item, and on the right back, share and " +
          "abandon.",
      },
    },
    {
      id: "item-review",
      selector: ".wf-review-strip",
      title: "The last review",
      body:
        "The latest agent review's verdict, and whether it was applied or published yet. " +
        "Click it for the full report; ↻ Apply review turns its findings into the next round.",
    },
    {
      id: "item-content",
      selector: ".wf-body",
      title: "The content",
      body:
        "The document or diff you picked. In the diff, click a line to comment: open " +
        "comments become the next round's work when you request changes.",
    },
    {
      id: "item-settings",
      selector: { workbench: ".wf-outline > .wf-outline-item:last-child", tabs: ".wf-subtab-end" },
      title: "This item's settings",
      body:
        "Its own knobs: title, diff base, agent, PR skill, ticket, how its rounds run. " +
        "Settings → Appearance → Workflow item view switches between Workbench and Tabs.",
    },
  ];

  /// The item tour for one view, every per-view field resolved.
  function itemTourSteps(view) {
    const pick = (v) => (v && typeof v === "object" ? v[view] : v);
    return ITEM_TOUR_STEPS.filter((s) => !s.views || s.views.includes(view))
      .map((s) => ({ id: s.id, selector: pick(s.selector), title: s.title, body: pick(s.body) }))
      .filter((s) => s.selector && s.body);
  }

  const api = { TOUR_STEPS, windowTourSteps, ITEM_TOUR_STEPS, itemTourSteps, tourPlacement };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
