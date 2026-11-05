// Sidebar layouts — pure. The app speaks in layout-neutral actions (show a
// view, toggle a section, sync the badges, paint the workspaces); a layout
// decides how each one looks. This module holds the catalogue of views and
// layouts and the decisions; app.js's `sidebarLayout()` adapter applies them
// to the DOM. Tested by gui/tests/layout.test.js.
(function (root) {
  "use strict";

  /// Everything the sidebar can show. `elements` are the DOM ids a view owns;
  /// `openKey` is the `state` flag a collapsible section keeps (Classic and
  /// Focus collapse sections, the rail layouts never do).
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
  /// - `overlay`: the panel floats over the terminals and closes on its own,
  ///   so the terminals keep the window's whole width.
  const LAYOUTS = [
    {
      id: "classic",
      label: "Classic",
      detail: "One sidebar: workspaces, sessions, then collapsible Workflows, Scratches and Teams.",
      exclusive: false,
      rail: false,
      overlay: false,
    },
    {
      id: "focus",
      label: "Focus",
      detail: "One sidebar, sessions on top and one section open at a time below them, filling the height.",
      exclusive: true,
      rail: false,
      overlay: false,
    },
    {
      id: "rail",
      label: "Rail",
      detail: "An icon rail for workspaces and views; the panel beside it shows one view, full height.",
      exclusive: true,
      rail: true,
      overlay: false,
    },
    {
      id: "flyout",
      label: "Flyout",
      detail: "Only the icon rail stays; a view slides over the terminals when you open it, so they keep the full width.",
      exclusive: true,
      rail: true,
      overlay: true,
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
  /// `active` is the view the user last chose (rail layouts, and the open
  /// section in Focus); `open` the Classic collapse flags; `panelOpen` whether
  /// a Flyout panel is out.
  function sidebarPlan(layoutId, { active = "sessions", open = {}, panelOpen = false } = {}) {
    const layout = layoutOf(layoutId);
    const current = viewOf(active) ? active : "sessions";
    const views = SIDEBAR_VIEWS.map((v) => {
      if (!layout.exclusive) {
        // Classic: everything stacked, each section as its own flag says.
        return { id: v.id, shown: true, expanded: v.openKey ? !!open[v.openKey] : true, fill: false };
      }
      if (!layout.rail) {
        // Focus: sessions always on top; of the sections, only the active one
        // is open, and it takes the rest of the height.
        if (v.id === "sessions") return { id: v.id, shown: true, expanded: true, fill: false };
        const on = v.id === current;
        return { id: v.id, shown: true, expanded: on, fill: on };
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
      panel: layout.overlay ? !!panelOpen : true,
      workspaceChips: !layout.rail,
    };
  }

  /// The section Focus opens when a user clicks a section header: the clicked
  /// one, or none when it was already the open one (sessions stay).
  function focusToggle(active, clicked) {
    return active === clicked ? "sessions" : clicked;
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

  /// The rail's view buttons with their badges (`badges[id]` = a count).
  function railItems(active, badges = {}) {
    return SIDEBAR_VIEWS.map((v) => ({
      id: v.id,
      label: v.label,
      icon: v.icon,
      active: v.id === active,
      badge: badges[v.id] > 0 ? badges[v.id] : 0,
    }));
  }

  const api = {
    SIDEBAR_VIEWS,
    LAYOUTS,
    DEFAULT_LAYOUT,
    layoutOf,
    viewOf,
    sidebarPlan,
    focusToggle,
    workspaceInitials,
    railItems,
  };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.SidebarLayout = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
