// Sidebar layouts — pure. The app speaks in layout-neutral actions (show a
// view, toggle a section, sync the badges, paint the workspaces); a layout
// decides how each one looks. This module holds the catalogue of views and
// layouts and the decisions; app.js's `sidebarLayout()` adapter applies them
// to the DOM. Tested by gui/tests/layout.test.js.
(function (root) {
  "use strict";

  /// Everything the sidebar can show. `elements` are the DOM ids a view owns;
  /// `openKey` is the `state` flag a collapsible section keeps (Classic
  /// collapses sections, the rail layouts never do).
  const SIDEBAR_VIEWS = [
    { id: "sessions", label: "Sessions", icon: "terminal", elements: ["new-session-btn", "search-wrap", "session-list"] },
    { id: "workflows", label: "Workflows", icon: "pr", elements: ["wf-section"], resizer: "wf-resizer", openKey: "wfOpen" },
    { id: "scratches", label: "Scratches", icon: "pencil", elements: ["notes-section"], resizer: "notes-resizer", openKey: "notesOpen" },
    { id: "teams", label: "Teams", icon: "users", elements: ["teams-section"], resizer: "teams-resizer", openKey: "teamsOpen" },
  ];

  /// The layouts a user can pick (Settings → Appearance → Layout).
  ///
  /// - `exclusive`: one section is open at a time and takes the room.
  /// - `rail`: workspaces and views live in an icon rail; the panel shows one
  ///   view at a time, full height.
  /// - `tabs`: the sidebar keeps its workspace chips and switches views with
  ///   a tab strip at its top (the exclusive rule without a rail).
  /// - `overlay`: the panel floats over the terminals and closes on its own,
  ///   so the terminals keep the window's whole width.
  /// - `panels`: how the right-hand panels (Files ⌘E, Inspector ⓘ) open —
  ///   `stack`: docked, any together; `single`: docked, one at a time (the
  ///   rail's one-view rule, on the right); `overlay`: a drawer over the
  ///   panes, one at a time, closing like Flyout's panel.
  /// - `quickSettings`: where the sidebar's quick settings live — `footer`
  ///   (a collapsible section under the lists) or `popover` (behind the ⚙ at
  ///   the rail's foot, so they never read as part of the open view).
  const LAYOUTS = [
    {
      id: "classic",
      label: "Classic",
      detail: "One sidebar: workspaces, sessions, then collapsible Workflows, Scratches and Teams.",
      exclusive: false,
      rail: false,
      overlay: false,
      panels: "stack",
      quickSettings: "footer",
    },
    {
      id: "rail",
      label: "Rail",
      detail: "An icon rail for workspaces and views; the panel beside it shows one view, full height.",
      exclusive: true,
      rail: true,
      overlay: false,
      panels: "single",
      quickSettings: "popover",
    },
    {
      id: "tabbed",
      label: "Tabbed",
      detail: "One sidebar with workspace chips; a tab strip at its top switches between the views, each full height.",
      exclusive: true,
      rail: false,
      tabs: true,
      overlay: false,
      panels: "stack",
      quickSettings: "footer",
    },
    {
      id: "flyout",
      label: "Flyout",
      detail: "Only the icon rail stays; a view slides over the terminals when you open it, so they keep the full width.",
      exclusive: true,
      rail: true,
      overlay: true,
      panels: "overlay",
      quickSettings: "popover",
    },
  ];

  const DEFAULT_LAYOUT = "classic";

  function layoutOf(id) {
    return LAYOUTS.find((l) => l.id === id) || LAYOUTS.find((l) => l.id === DEFAULT_LAYOUT);
  }

  function viewOf(id) {
    return SIDEBAR_VIEWS.find((v) => v.id === id) || null;
  }

  /// What the sidebar shows: per view, whether it is on screen (`shown`), its
  /// list expanded (`expanded`) and stretched to the free height (`fill`),
  /// plus whether the panel itself is visible.
  ///
  /// `active` is the view the user last chose (the rail layouts); `open` the
  /// Classic collapse flags; `panelOpen` whether a Flyout panel is out.
  function sidebarPlan(layoutId, { active = "sessions", open = {}, panelOpen = false } = {}) {
    const layout = layoutOf(layoutId);
    const current = viewOf(active) ? active : "sessions";
    const views = SIDEBAR_VIEWS.map((v) => {
      if (!layout.exclusive) {
        // Classic: everything stacked, each section as its own flag says.
        return { id: v.id, shown: true, expanded: v.openKey ? !!open[v.openKey] : true, fill: false };
      }
      // Rail / Flyout: the active view alone, full height.
      const on = v.id === current;
      return { id: v.id, shown: on, expanded: on, fill: on };
    });
    return {
      layout: layout.id,
      active: current,
      views,
      rail: layout.rail,
      viewTabs: !!layout.tabs,
      panel: layout.overlay ? !!panelOpen : true,
      workspaceChips: !layout.rail,
      panels: layout.panels || "stack",
      quickSettings: layout.quickSettings || "footer",
    };
  }

  /// A workspace's rail badge: up to two initials of its name (`Saturn Core`
  /// → `SC`, `main` → `MA`), its number when the name has no letters.
  function workspaceInitials(name, index) {
    const words = String(name || "")
      .split(/[\s\-_./]+/)
      .map((w) => w.replace(/[^\p{L}\p{N}]/gu, ""))
      .filter(Boolean);
    if (!words.length) return String(index + 1);
    const letters = words.length > 1 ? words[0][0] + words[1][0] : words[0].slice(0, 2);
    return letters.toUpperCase();
  }

  /// The view switcher's buttons — the rail's icons or Tabbed's tab strip —
  /// with their badges (`badges[id]` = a count).
  function railItems(active, badges = {}) {
    return SIDEBAR_VIEWS.map((v) => ({
      id: v.id,
      label: v.label,
      icon: v.icon,
      active: v.id === active,
      badge: badges[v.id] > 0 ? badges[v.id] : 0,
    }));
  }

  /// The right-hand panels, in the order they sit left to right.
  const RIGHT_PANELS = ["files", "details"];

  /// Which right panels stay open when `which` opens, given the layout's
  /// `panels` mode and the ones open now: `stack` keeps the others, `single`
  /// and `overlay` show one at a time.
  function rightPanelsAfterOpen(mode, open, which) {
    const now = (open || []).filter((p) => RIGHT_PANELS.includes(p));
    if (mode === "stack") return RIGHT_PANELS.filter((p) => p === which || now.includes(p));
    return [which];
  }

  const api = {
    RIGHT_PANELS,
    rightPanelsAfterOpen,
    SIDEBAR_VIEWS,
    LAYOUTS,
    DEFAULT_LAYOUT,
    layoutOf,
    viewOf,
    sidebarPlan,
    workspaceInitials,
    railItems,
  };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.SidebarLayout = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
