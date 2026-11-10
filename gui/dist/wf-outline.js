// Workflow item views — pure. An item tab is shown either as `tabs` (one row
// of sub-view tabs, the action bar at the bottom) or as `workbench` (a grouped
// outline beside the content, the next step on top of it). Both read the same
// sub-view list app.js builds; this module only groups and labels it, so the
// two views can never disagree about which documents an item has. Tested by
// gui/tests/wf_outline.test.js.
(function (root) {
  "use strict";

  const WF_ITEM_VIEWS = [
    { id: "workbench", label: "Workbench", detail: "A grouped outline beside the content, the next step on top of it." },
    { id: "tabs", label: "Tabs", detail: "One row of tabs above the content, the actions below it." },
  ];
  const DEFAULT_ITEM_VIEW = "workbench";

  function itemViewOf(id) {
    return WF_ITEM_VIEWS.find((v) => v.id === id) || WF_ITEM_VIEWS.find((v) => v.id === DEFAULT_ITEM_VIEW);
  }

  /// The outline's groups: what is being built, what was said about it, what
  /// explains it, and how it got here. Settings stand apart, at the bottom.
  const OUTLINE_GROUPS = [
    { id: "build", label: "Build", ids: ["waves", "plan", "diff", "prs"] },
    { id: "feedback", label: "Feedback", ids: ["review", "agentReview", "handoff"] },
    { id: "explained", label: "Explained", ids: ["explainPlan", "explainDiff", "drift"] },
    { id: "history", label: "History", ids: ["revisions", "history"] },
  ];

  /// A tab label as an outline entry: the leading glyph dropped (the group
  /// says what kind of thing it is), a count or open-comment badge lifted out
  /// (`Agent reviews (2)` → `Agent reviews` + 2), `·empty` as a dim flag.
  function outlineEntry(id, label) {
    let text = String(label || "");
    let badge = 0;
    let hot = false;
    const comments = /\s*💬\s*(\d+)/.exec(text);
    if (comments) {
      badge = Number(comments[1]);
      hot = true;
      text = text.replace(comments[0], "");
    }
    const count = /\s*\((\d+)\)\s*$/.exec(text);
    if (count) {
      badge = badge || Number(count[1]);
      text = text.replace(count[0], "");
    }
    const empty = /\s*·empty\s*$/.test(text);
    text = text.replace(/\s*·empty\s*$/, "");
    text = text.replace(/^[^\p{L}\p{N}]+/u, "").trim();
    return { id, text, badge, hot, empty };
  }

  /// Group the item's sub-views (`[[id, label], …]`, the list the tab bar is
  /// made of) into the outline. Every sub-view lands in exactly one place: an
  /// id no group names goes to the group of the last known one before it —
  /// better a misplaced entry than a document nobody can open.
  function wfOutline(subViews, active) {
    const groups = OUTLINE_GROUPS.map((g) => ({ id: g.id, label: g.label, items: [] }));
    let settings = null;
    let last = groups[0];
    for (const [id, label] of subViews || []) {
      const entry = { ...outlineEntry(id, label), active: id === active };
      if (id === "settings") {
        settings = entry;
        continue;
      }
      const g = groups.find((x) => OUTLINE_GROUPS.find((o) => o.id === x.id).ids.includes(id)) || last;
      g.items.push(entry);
      last = g;
    }
    for (const g of groups) {
      const order = OUTLINE_GROUPS.find((o) => o.id === g.id).ids;
      g.items.sort((a, b) => {
        const ia = order.indexOf(a.id);
        const ib = order.indexOf(b.id);
        return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
      });
    }
    return { groups: groups.filter((g) => g.items.length), settings };
  }

  /// The item zone's buttons as a "More" menu: label and danger flag per
  /// button, in order. Pure over `{ text, danger }` records.
  function moreMenuItems(buttons) {
    // Indexed before filtering: the index is the button the entry clicks.
    return (buttons || [])
      .map((b, index) => ({ index, label: String((b && b.text) || "").trim(), danger: !!(b && b.danger) }))
      .filter((it) => it.label);
  }

  const api = {
    WF_ITEM_VIEWS,
    DEFAULT_ITEM_VIEW,
    itemViewOf,
    OUTLINE_GROUPS,
    outlineEntry,
    wfOutline,
    moreMenuItems,
  };
  if (typeof module === "object" && module.exports) module.exports = api;
  else root.WfOutline = api;
})(typeof globalThis !== "undefined" ? globalThis : this);
