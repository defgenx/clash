// Sidebar layouts — pure. The app speaks in layout-neutral actions (show a
// view, toggle a section, sync the badges, paint the workspaces); a layout
// decides how each one looks. This module holds the catalogue of views and
// layouts and the decisions; app.js's `sidebarLayout()` adapter applies them
// to the DOM. Tested by gui/tests/layout.test.js.
(function (root) {
  "use strict";

  /// Everything the sidebar can show. `elements` are the DOM ids a view owns;
  /// `body` the one of them that grows and shrinks (Stack's dividers size it);
  /// `min` the height below which it stops being usable; `openKey` is the
  /// `state` flag a collapsible section keeps (Classic collapses sections, the
  /// other layouts decide it themselves).
  const SIDEBAR_VIEWS = [
    { id: "sessions", label: "Sessions", icon: "terminal", elements: ["new-session-btn", "search-wrap", "session-list"], body: "session-list", min: 80 },
    { id: "workflows", label: "Workflows", icon: "pr", elements: ["wf-section"], body: "wf-section", min: 56, resizer: "wf-resizer", openKey: "wfOpen" },
    { id: "scratches", label: "Scratches", icon: "pencil", elements: ["notes-section"], body: "notes-section", min: 56, resizer: "notes-resizer", openKey: "notesOpen" },
    { id: "teams", label: "Teams", icon: "users", elements: ["teams-section"], body: "teams-section", min: 56, resizer: "teams-resizer", openKey: "teamsOpen" },
  ];

  /// The layouts a user can pick (Settings → Appearance → Layout).
  ///
  /// - `place`: where the sidebar sits — `left` (beside the terminals),
  ///   `bottom` (Dock: a panel under them, the views as columns), `split`
  ///   (Sessions on the left, the other views in a second sidebar on the
  ///   right) or `top` (Topbar: the rail runs across the window's top and a
  ///   view drops down under it).
  /// - `sections`: `collapsible` (each section keeps its own expanded flag),
  ///   `one` (one view at a time, full height), `some` (the views you stacked,
  ///   sharing the room) or `all` (every view open at once).
  /// - `rail`: workspaces and views live in an icon rail (vertical on the
  ///   left, horizontal at the top for Topbar).
  /// - `tabs`: the sidebar keeps its workspace chips and switches views with
  ///   a tab strip at its top (the one-view rule without a rail).
  /// - `overlay`: the panel floats over the terminals and closes on its own,
  ///   so the terminals keep the window's whole width.
  /// - `panels`: how the right-hand panels (Files ⌘E, Inspector ⓘ) open —
  ///   `stack`: docked, any together; `single`: docked, one at a time;
  ///   `overlay`: a drawer over the panes, one at a time, closing like an
  ///   overlay panel.
  /// - `quickSettings`: where the sidebar's quick settings live — `footer`
  ///   (a collapsible section under the lists) or `popover` (behind the rail's
  ///   ⚙, so they never read as part of the open view).
  const LAYOUTS = [
    {
      id: "classic",
      label: "Classic",
      detail: "One sidebar: workspaces, sessions, then collapsible Workflows, Scratches and Teams.",
      place: "left",
      sections: "collapsible",
      rail: false,
      overlay: false,
      panels: "stack",
      quickSettings: "footer",
    },
    {
      id: "rail",
      label: "Rail",
      detail: "An icon rail and Sessions on the left; a sidebar on the right shows Workflows, Scratches or Teams, one at a time.",
      place: "split",
      sections: "one",
      rail: true,
      overlay: false,
      panels: "single",
      quickSettings: "popover",
    },
    {
      id: "stack",
      label: "Stack",
      detail: "An icon rail whose icons stack views in one panel: click to add or remove one, ⌥-click to show it alone.",
      place: "left",
      sections: "some",
      rail: true,
      overlay: false,
      panels: "single",
      quickSettings: "popover",
    },
    {
      id: "tabbed",
      label: "Tabbed",
      detail: "One sidebar with workspace chips; a tab strip at its top switches between the views, each full height.",
      place: "left",
      sections: "one",
      rail: false,
      tabs: true,
      overlay: false,
      panels: "stack",
      quickSettings: "footer",
    },
    {
      id: "dock",
      label: "Dock",
      detail: "The terminals take the full width; Sessions, Workflows, Scratches and Teams sit side by side in a panel under them.",
      place: "bottom",
      sections: "all",
      rail: false,
      overlay: false,
      panels: "stack",
      quickSettings: "footer",
    },
    {
      id: "split",
      label: "Split",
      detail: "Sessions on the left; Workflows, Scratches and Teams in a second sidebar on the right.",
      place: "split",
      sections: "all",
      rail: false,
      overlay: false,
      panels: "single",
      quickSettings: "footer",
    },
    {
      id: "topbar",
      label: "Topbar",
      detail: "No sidebar: workspaces and views are one bar across the top, and a view drops down under it when you open it.",
      place: "top",
      sections: "one",
      rail: true,
      overlay: true,
      panels: "overlay",
      quickSettings: "popover",
    },
  ];

  /// Layouts that no longer exist, and the one a saved choice becomes.
  /// Flyout was Rail with an overlaid panel — too close to tell apart — and
  /// Rail Split became what Rail is.
  const RETIRED_LAYOUTS = { flyout: "rail", railsplit: "rail" };

  const DEFAULT_LAYOUT = "classic";

  function layoutOf(id) {
    const want = RETIRED_LAYOUTS[id] || id;
    return LAYOUTS.find((l) => l.id === want) || LAYOUTS.find((l) => l.id === DEFAULT_LAYOUT);
  }

  /// Whether the layout decides which sections are expanded (`one`, `all`)
  /// rather than leaving it to each section's own flag (`collapsible`).
  function managesSections(layout) {
    return layout.sections !== "collapsible";
  }

  function viewOf(id) {
    return SIDEBAR_VIEWS.find((v) => v.id === id) || null;
  }

  /// What the sidebar shows: per view, whether it is on screen (`shown`), its
  /// list expanded (`expanded`) and stretched to the free room (`fill`), plus
  /// whether the panel itself is visible.
  ///
  /// `active` is the view the user last chose (the one-view layouts); `open`
  /// the Classic collapse flags; `panelOpen` whether an overlay panel is out;
  /// `stacked` the views Stack shows.
  ///
  /// A one-view Split (Rail) keeps Sessions on the left at all times and
  /// shows one right-hand view: `active` when it is one, else the first.
  function sidebarPlan(layoutId, { active = "sessions", open = {}, panelOpen = false, stacked = DEFAULT_STACK } = {}) {
    const layout = layoutOf(layoutId);
    const current = viewOf(active) ? active : "sessions";
    const oneRight = layout.place === "split" && layout.sections === "one";
    const rightView = oneRight ? (SPLIT_RIGHT.includes(current) ? current : SPLIT_RIGHT[0]) : null;
    const views = SIDEBAR_VIEWS.map((v) => {
      if (layout.sections === "collapsible") {
        // Classic: everything stacked, each section as its own flag says.
        return { id: v.id, shown: true, expanded: v.openKey ? !!open[v.openKey] : true, fill: false };
      }
      if (layout.sections === "all") return { id: v.id, shown: true, expanded: true, fill: true };
      if (layout.sections === "some") {
        const on = normalizeStack(stacked).includes(v.id);
        return { id: v.id, shown: on, expanded: on, fill: on };
      }
      if (rightView) {
        const on = v.id === "sessions" || v.id === rightView;
        return { id: v.id, shown: on, expanded: on, fill: on };
      }
      // One view at a time: the active one alone, full height.
      const on = v.id === current;
      return { id: v.id, shown: on, expanded: on, fill: on };
    });
    return {
      layout: layout.id,
      active: current,
      views,
      place: layout.place,
      rightView,
      stacked: layout.sections === "some" ? normalizeStack(stacked) : null,
      rail: layout.rail,
      viewTabs: !!layout.tabs,
      panel: layout.overlay ? !!panelOpen : true,
      workspaceChips: !layout.rail,
      panels: layout.panels || "stack",
      quickSettings: layout.quickSettings || "footer",
    };
  }

  /// Which views a Split layout moves into its right-hand sidebar.
  const SPLIT_RIGHT = ["workflows", "scratches", "teams"];

  /// The sidebar a view sits in under a layout: `right` for Split's work
  /// lists, `left` for everything else (the side ⌘B and the rail fold).
  function sideOf(layoutId, viewId) {
    return layoutOf(layoutId).place === "split" && SPLIT_RIGHT.includes(viewId) ? "right" : "left";
  }

  /// The rail icons lit for a plan, given which sidebars are on screen: the
  /// open view in Topbar; in Rail, Sessions while the left panel shows and the
  /// right view while the right one does; in Stack, every stacked view.
  function railLit(plan, { left = true, right = true } = {}) {
    if (!plan.panel) return [];
    if (plan.place === "split") {
      return [left && "sessions", right && plan.rightView].filter(Boolean);
    }
    if (plan.stacked) return left ? plan.stacked : [];
    return left ? [plan.active] : [];
  }

  /// What Stack shows before you choose.
  const DEFAULT_STACK = ["sessions", "workflows"];

  /// A stack as the plan uses it: known views only, once each, in the
  /// catalogue's order — never the order they were clicked, so a view always
  /// lands in the same place. An empty or unreadable stack is the default.
  function normalizeStack(ids) {
    const want = new Set(Array.isArray(ids) ? ids : []);
    const out = SIDEBAR_VIEWS.map((v) => v.id).filter((id) => want.has(id));
    return out.length ? out : DEFAULT_STACK.slice();
  }

  /// Stack's share of the height per view, as flex weights: a missing, zero,
  /// negative or non-numeric weight is 1 (an equal share), and the rest are
  /// clamped so no view can be dragged to nothing or take everything.
  function normalizeStackWeights(weights) {
    const out = {};
    for (const v of SIDEBAR_VIEWS) {
      const w = Number(weights && weights[v.id]);
      out[v.id] = Number.isFinite(w) && w > 0 ? Math.min(20, Math.max(0.05, w)) : 1;
    }
    return out;
  }

  /// A drag of the divider between two stacked views by `delta` px: height
  /// moves from one to the other and nobody else changes. The pair keeps its
  /// combined weight, so the other views keep their share; each side stops at
  /// its `min`. Heights are what the two showed when the drag began.
  function stackResize(weights, above, below, { hAbove, hBelow, delta, minAbove = 0, minBelow = 0 }) {
    const w = normalizeStackWeights(weights);
    const total = hAbove + hBelow;
    if (!(total > 0) || total < minAbove + minBelow) return w;
    const a = Math.max(minAbove, Math.min(total - minBelow, hAbove + delta));
    const pair = w[above] + w[below];
    const wa = Math.round(((pair * a) / total) * 1000) / 1000;
    return normalizeStackWeights({ ...w, [above]: wa, [below]: Math.round((pair - wa) * 1000) / 1000 });
  }

  /// The dividers a stack shows: one between each pair of neighbours.
  function stackDividers(stacked) {
    const s = normalizeStack(stacked);
    return s.slice(1).map((below, i) => ({ above: s[i], below }));
  }

  /// A click on a Stack rail icon: adds the view, or removes it; `solo` (⌥)
  /// shows it alone. Removing the last view folds the panel instead of
  /// emptying it (`fold`), so the next click brings back what was there.
  function stackToggle(stacked, id, { solo = false } = {}) {
    const now = normalizeStack(stacked);
    if (!viewOf(id)) return { stacked: now, fold: false };
    if (solo) return { stacked: [id], fold: false };
    if (!now.includes(id)) return { stacked: normalizeStack([...now, id]), fold: false };
    if (now.length === 1) return { stacked: now, fold: true };
    return { stacked: now.filter((v) => v !== id), fold: false };
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
    RETIRED_LAYOUTS,
    DEFAULT_LAYOUT,
    SPLIT_RIGHT,
    sideOf,
    railLit,
    DEFAULT_STACK,
    normalizeStack,
    stackToggle,
    normalizeStackWeights,
    stackResize,
    stackDividers,
    layoutOf,
    managesSections,
    viewOf,
    sidebarPlan,
    workspaceInitials,
    railItems,
  };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.SidebarLayout = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
