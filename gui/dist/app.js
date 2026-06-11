// clash GUI frontend — cmux-style sidebar + split terminal panes.
// No build step: plain JS against the Tauri global API (withGlobalTauri).

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = {
  sessions: [],
  query: "",
  open: new Map(), // session id -> { term, fitAddon, el, name }
  // cmux-style workspaces: each owns its pane layout AND its sessions —
  // the sidebar is scoped to the active workspace's sessions.
  workspaces: [{ name: "main", panes: [null], focused: 0, zoomed: false, sessions: [] }],
  activeWs: 0,
  activeTab: null, // session id highlighted in the tab bar
  detailsFor: null, // session id shown in the details panel, or null
  teams: [],
  teamsOpen: false,
  renaming: null, // session id with an open inline-rename input
  prevStatuses: new Map(), // session id -> status (attention transitions)
  unread: new Set(), // session ids with unseen attention events
  missingStreak: new Map(), // session id -> consecutive refreshes absent (ownership prune)
};

const $ = (id) => document.getElementById(id);

// ── In-app dialogs ──────────────────────────────────────────────
// wry's WKWebView does not implement native alert/confirm/prompt —
// they silently return undefined — so modal equivalents are built in-page.

function uiDialog({ message, input = null, okLabel = "OK", cancelable = true, danger = false }) {
  return new Promise((resolve) => {
    const cancelValue = input !== null ? null : false;
    const backdrop = document.createElement("div");
    backdrop.className = "dialog-backdrop";
    const box = document.createElement("div");
    box.className = "dialog-box";
    const msg = document.createElement("p");
    msg.textContent = message;
    box.appendChild(msg);
    let field = null;
    if (input !== null) {
      field = document.createElement("input");
      field.type = "text";
      field.value = input;
      field.spellcheck = false;
      box.appendChild(field);
    }
    const actions = document.createElement("div");
    actions.className = "modal-actions";
    const ok = document.createElement("button");
    ok.textContent = okLabel;
    ok.className = danger ? "danger-primary" : "primary";
    if (cancelable) {
      const cancel = document.createElement("button");
      cancel.textContent = "Cancel";
      cancel.onclick = () => done(cancelValue);
      actions.appendChild(cancel);
    }
    actions.appendChild(ok);
    box.appendChild(actions);
    backdrop.appendChild(box);
    document.body.appendChild(backdrop);
    const done = (val) => {
      backdrop.remove();
      resolve(val);
    };
    ok.onclick = () => done(input !== null ? field.value : true);
    backdrop.onclick = (e) => {
      if (e.target === backdrop && cancelable) done(cancelValue);
    };
    backdrop.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") done(input !== null ? field.value : true);
      else if (e.key === "Escape" && cancelable) done(cancelValue);
    });
    setTimeout(() => (field || ok).focus(), 0);
  });
}

const uiConfirm = (message, okLabel = "Confirm") =>
  uiDialog({ message, okLabel, danger: true });
const uiPrompt = (message, def = "") => uiDialog({ message, input: def });
const uiAlert = (message) => uiDialog({ message, cancelable: false });

/// The active workspace.
function ws() {
  return state.workspaces[state.activeWs];
}

// ── Workspace persistence (layout + session ownership) ─────────
// Primary store is a disk file via the backend (gui-state.json) — the
// bare-binary WKWebView's localStorage is not reliably persisted across
// restarts. localStorage is kept as a same-session fallback only.

let saveTimer = null;

function workspacesJson() {
  return JSON.stringify({
    workspaces: state.workspaces.map((w) => ({
      name: w.name,
      panes: w.panes,
      sessions: w.sessions,
    })),
    active: state.activeWs,
  });
}

function saveWorkspaces() {
  const json = workspacesJson();
  try {
    localStorage.setItem("clash-workspaces", json);
  } catch (e) {
    console.error("saveWorkspaces (localStorage) failed:", e);
  }
  // Debounced disk write — frequent calls during drag/assign collapse to one
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    invoke("save_gui_state", { stateJson: workspacesJson() }).catch((e) =>
      console.error("save_gui_state failed:", e)
    );
  }, 250);
}

function applyWorkspacesData(data) {
  if (!data || !Array.isArray(data.workspaces) || !data.workspaces.length) return false;
  state.workspaces = data.workspaces.map((w) => ({
    name: w.name || "ws",
    panes: Array.isArray(w.panes) && w.panes.length ? w.panes : [null],
    focused: 0,
    zoomed: false,
    sessions: Array.isArray(w.sessions) ? w.sessions : [],
  }));
  state.activeWs = Math.min(data.active || 0, state.workspaces.length - 1);
  return true;
}

async function loadWorkspaces() {
  try {
    const raw = await invoke("load_gui_state");
    if (raw && applyWorkspacesData(JSON.parse(raw))) return;
  } catch (e) {
    console.error("load_gui_state failed:", e);
  }
  try {
    const raw = localStorage.getItem("clash-workspaces");
    if (raw) applyWorkspacesData(JSON.parse(raw));
  } catch (e) {
    console.error("loadWorkspaces (localStorage) failed:", e);
  }
}

/// Re-attach running sessions referenced by restored workspace panes.
/// Stashed/dead sessions are cleared from their slots (no surprise resumes).
async function restoreWorkspaceSessions() {
  const running = new Set(
    state.sessions.filter((s) => s.is_running).map((s) => s.id)
  );
  const savedActive = state.activeWs;
  for (let wi = 0; wi < state.workspaces.length; wi++) {
    const w = state.workspaces[wi];
    for (let pi = 0; pi < w.panes.length; pi++) {
      const sid = w.panes[pi];
      if (!sid) continue;
      if (!running.has(sid)) {
        w.panes[pi] = null;
        continue;
      }
      state.activeWs = wi;
      w.focused = pi;
      await openSession(sid);
    }
  }
  state.activeWs = savedActive;
  renderAll();
}

function renderAll() {
  renderWorkspaceBar();
  renderPanes();
  renderTabs();
  renderSidebar();
}

// ── Workspace bar ───────────────────────────────────────────────

function renderWorkspaceBar() {
  const chips = $("workspace-chips");
  chips.innerHTML = "";
  state.workspaces.forEach((w, i) => {
    const chip = document.createElement("div");
    chip.className = "ws-chip" + (i === state.activeWs ? " active" : "");
    chip.title = `${w.name} — ⌘${i + 1}`;
    chip.innerHTML = `<span class="n">${i + 1}</span><span class="label">${escapeHtml(
      w.name
    )}</span>`;
    if (state.workspaces.length > 1) {
      const close = document.createElement("span");
      close.className = "ws-close";
      close.textContent = "×";
      close.title = "Close workspace (⌘⇧W)";
      close.onclick = (ev) => {
        ev.stopPropagation();
        state.activeWs = i;
        closeWorkspace();
      };
      chip.appendChild(close);
    }
    chip.onclick = () => switchWorkspace(i);
    chip.ondblclick = () => renameWorkspace(i);
    chips.appendChild(chip);
  });
}

function switchWorkspace(i) {
  if (i < 0 || i >= state.workspaces.length) return;
  state.activeWs = i;
  const sid = ws().panes[ws().focused];
  state.activeTab = sid || state.activeTab;
  saveWorkspaces();
  renderAll();
  if (sid) focusTerm(sid);
}

function newWorkspace() {
  state.workspaces.push({
    name: `ws-${state.workspaces.length + 1}`,
    panes: [null],
    focused: 0,
    zoomed: false,
    sessions: [],
  });
  switchWorkspace(state.workspaces.length - 1);
}

/// Index of the workspace owning a session, or -1 if unassigned.
function sessionWorkspace(sid) {
  return state.workspaces.findIndex((w) => w.sessions.includes(sid));
}

/// Claim a session for the active workspace if no workspace owns it yet.
function claimSession(sid) {
  if (sessionWorkspace(sid) === -1) {
    ws().sessions.push(sid);
    saveWorkspaces();
  }
}

async function renameWorkspace(i) {
  const name = await uiPrompt("Workspace name:", state.workspaces[i].name);
  if (name && name.trim()) {
    state.workspaces[i].name = name.trim();
    saveWorkspaces();
    renderWorkspaceBar();
  }
}

function closeWorkspace() {
  if (state.workspaces.length <= 1) return;
  state.workspaces.splice(state.activeWs, 1);
  switchWorkspace(Math.max(0, state.activeWs - 1));
}

// ── Session helpers ─────────────────────────────────────────────

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

// Subsequence fuzzy match: every char of q appears in order in hay.
function fuzzyMatch(q, hay) {
  q = q.toLowerCase();
  hay = (hay || "").toLowerCase();
  if (hay.includes(q)) return true;
  let i = 0;
  for (const c of hay) {
    if (c === q[i]) i++;
    if (i === q.length) return true;
  }
  return false;
}

function visibleSessions() {
  if (!state.query) return state.sessions;
  return state.sessions.filter((s) =>
    fuzzyMatch(
      state.query,
      `${displayName(s)} ${s.git_branch} ${s.worktree_project || s.project} ${s.summary}`
    )
  );
}

// ── Sidebar ─────────────────────────────────────────────────────

function renderStatusSections(list, items) {
  const sections = { ACTIVE: [], FAILED: [], STASHED: [], DONE: [] };
  for (const s of items) sections[sectionOf(s)].push(s);
  for (const [label, group] of Object.entries(sections)) {
    if (group.length === 0) continue;
    const header = document.createElement("div");
    header.className = "section-label";
    header.innerHTML = `${label}<span class="count">${group.length}</span>`;
    list.appendChild(header);
    for (const s of group) list.appendChild(sessionItem(s));
  }
}

function renderExternalSection(list, items) {
  if (!items.length) return;
  const header = document.createElement("div");
  header.className = "section-label external";
  header.innerHTML = `⚡ EXTERNAL<span class="count">${items.length}</span>`;
  header.title = "Claude processes running outside clash — use ⚡ to adopt";
  list.appendChild(header);
  for (const s of items) list.appendChild(sessionItem(s));
}

function renderSidebar() {
  const list = $("session-list");
  list.innerHTML = "";

  const visible = visibleSessions();
  // External (wild) claudes are segregated at the bottom, like the TUI's
  // EXTERNAL section — never interleaved with clash-managed rows.
  const wild = visible.filter((s) => s.source === "Wild");
  const managed = visible.filter((s) => s.source !== "Wild");

  if (state.query) {
    // Searching: global, across all workspaces. Items owned by another
    // workspace carry a ⌘n badge; clicking switches there and opens.
    renderStatusSections(list, managed);
    renderExternalSection(list, wild);
  } else {
    // Scoped: the active workspace's sessions, then sessions no
    // workspace has claimed yet. Other workspaces' sessions live in
    // their own workspace (switch via chips / ⌘1-9 / search).
    const mine = managed.filter((s) => ws().sessions.includes(s.id));
    const unassigned = managed.filter((s) => sessionWorkspace(s.id) === -1);
    renderStatusSections(list, mine);
    if (unassigned.length) {
      const header = document.createElement("div");
      header.className = "section-label unassigned";
      header.innerHTML = `UNASSIGNED<span class="count">${unassigned.length}</span>`;
      header.title = "Not in any workspace — opening one claims it for this workspace";
      list.appendChild(header);
      for (const s of unassigned) list.appendChild(sessionItem(s));
    }
    renderExternalSection(list, wild);
    if (mine.length === 0 && unassigned.length === 0 && wild.length === 0) {
      const empty = document.createElement("div");
      empty.className = "list-empty";
      empty.textContent = "no sessions in this workspace — / to search all";
      list.appendChild(empty);
    }
  }

  const scoped = state.query ? visible.length : null;
  const n = state.sessions.length;
  $("session-count").textContent =
    scoped !== null
      ? `${scoped} match${scoped === 1 ? "" : "es"}`
      : `${n} session${n === 1 ? "" : "s"}`;
}

function sessionItem(s) {
  const wild = s.source === "Wild";
  const item = document.createElement("div");
  item.className =
    "session-item" +
    (s.id === state.activeTab ? " selected" : "") +
    (wild ? " wild" : "");
  // Wild claudes are owned by another process — clicking shows details
  // (adopt with ⚡) instead of resuming a session something else holds.
  item.onclick = () => (wild ? showDetails(s.id) : openSession(s.id));

  const ring = document.createElement("div");
  ring.className = "status-ring " + statusClass(s);

  const meta = document.createElement("div");
  meta.className = "session-meta";

  const name = document.createElement("div");
  name.className = "session-name";
  if (state.renaming === s.id) {
    const input = document.createElement("input");
    input.value = s.name || "";
    input.onclick = (ev) => ev.stopPropagation();
    input.onkeydown = async (ev) => {
      if (ev.key === "Enter") {
        const v = input.value.trim();
        state.renaming = null;
        if (v) {
          try {
            await invoke("rename_session", { sessionId: s.id, name: v });
          } catch (e) {
            console.error("rename failed:", e);
          }
        }
        refreshSessions();
      } else if (ev.key === "Escape") {
        state.renaming = null;
        renderSidebar();
      }
    };
    input.onblur = () => {
      if (state.renaming === s.id) {
        state.renaming = null;
        renderSidebar();
      }
    };
    name.appendChild(input);
    setTimeout(() => input.focus(), 0);
  } else {
    name.textContent = displayName(s);
    name.ondblclick = (ev) => {
      ev.stopPropagation();
      startRename(s.id);
    };
  }

  const sub = document.createElement("div");
  sub.className = "session-sub";
  const st = statusInfo(s);
  const stLabel = document.createElement("span");
  stLabel.className = `status-label ${st.cls}`;
  stLabel.textContent = `${st.icon} ${st.label}`;
  sub.appendChild(stLabel);
  const owner = sessionWorkspace(s.id);
  if (owner >= 0 && owner !== state.activeWs) {
    const wsBadge = document.createElement("span");
    wsBadge.className = "ws-badge";
    wsBadge.textContent = `⌘${owner + 1} ${state.workspaces[owner].name}`;
    wsBadge.title = "Owned by another workspace — click opens it there";
    sub.appendChild(wsBadge);
  }
  const pr = state.prUrls.get(s.id);
  if (pr) {
    const chip = document.createElement("span");
    chip.className = "pr-chip";
    chip.textContent = `⇄ PR #${pr.split("/").pop()}`;
    chip.title = `${pr} — click to open in the browser panel`;
    chip.onclick = (ev) => {
      ev.stopPropagation();
      openBrowserPanel(pr);
    };
    sub.appendChild(chip);
  }
  if (s.git_branch) {
    const branch = document.createElement("span");
    branch.className = "branch";
    branch.textContent = s.git_branch;
    sub.appendChild(branch);
  }
  const proj = document.createElement("span");
  proj.className = "proj";
  proj.textContent = s.worktree_project || s.project;
  sub.appendChild(proj);

  meta.appendChild(name);
  meta.appendChild(sub);

  if (state.unread.has(s.id)) {
    const dot = document.createElement("div");
    dot.className = "unread-dot";
    dot.title = "Needs attention";
    meta.querySelector(".session-name").appendChild(dot);
  }

  const actions = document.createElement("div");
  actions.className = "session-actions";
  actions.appendChild(
    actionBtn("✎", "Rename", (ev) => {
      ev.stopPropagation();
      startRename(s.id);
    })
  );
  actions.appendChild(
    actionBtn("ⓘ", "Details", (ev) => {
      ev.stopPropagation();
      showDetails(s.id);
    })
  );
  if (s.source === "Wild") {
    actions.appendChild(
      actionBtn("⚡", "Adopt: take over this wild claude process", async (ev) => {
        ev.stopPropagation();
        if (
          !(await uiConfirm(
            `Take over wild session "${displayName(s)}"? Its current process is killed and resumed under clash.`,
            "Take over"
          ))
        )
          return;
        try {
          await invoke("takeover_wild", {
            sessionId: s.id,
            pid: s.wild_pid,
            cwd: s.cwd || s.project_path || "",
            cols: 120,
            rows: 40,
          });
        } catch (e) {
          uiAlert(`Adopt failed: ${e}`);
        }
        refreshSessions();
      })
    );
  }
  if (s.is_running) {
    actions.appendChild(
      actionBtn("⏸", "Stash (stop, keep resumable)", async (ev) => {
        ev.stopPropagation();
        await invoke("stash_session", { sessionId: s.id }).catch(console.error);
        dropTerminal(s.id);
        refreshSessions();
      })
    );
  }
  actions.appendChild(
    actionBtn(
      "✕",
      "Kill (remove from clash)",
      async (ev) => {
        ev.stopPropagation();
        if (!(await uiConfirm(`Kill session "${displayName(s)}"?`, "Kill"))) return;
        await invoke("kill_session", { sessionId: s.id }).catch(console.error);
        dropTerminal(s.id);
        refreshSessions();
      },
      true
    )
  );

  item.appendChild(ring);
  item.appendChild(meta);
  item.appendChild(actions);
  return item;
}

function actionBtn(label, title, onclick, danger = false) {
  const b = document.createElement("button");
  b.textContent = label;
  b.title = title;
  b.onclick = onclick;
  if (danger) b.className = "danger";
  return b;
}

function startRename(id) {
  state.renaming = id;
  renderSidebar();
}

async function refreshSessions() {
  try {
    const sessions = await invoke("list_sessions");
    // Attention transitions: flash title when a session starts prompting
    let attention = 0;
    for (const s of sessions) {
      const prev = state.prevStatuses.get(s.id);
      if (s.status === "Prompting" || s.status === "Waiting") attention++;
      state.prevStatuses.set(s.id, s.status);
      void prev;
    }
    document.title = attention > 0 ? `clash (${attention}!)` : "clash";
    state.sessions = sessions;

    // Prune workspace ownership of sessions gone from the list for 3
    // consecutive refreshes (killed/removed) — tolerates transient
    // daemon hiccups without orphaning the workspace's session list.
    const known = new Set(sessions.map((s) => s.id));
    let pruned = false;
    for (const w of state.workspaces) {
      for (const id of [...w.sessions]) {
        if (known.has(id)) {
          state.missingStreak.delete(id);
          continue;
        }
        const streak = (state.missingStreak.get(id) || 0) + 1;
        state.missingStreak.set(id, streak);
        if (streak >= 3) {
          w.sessions = w.sessions.filter((x) => x !== id);
          state.missingStreak.delete(id);
          pruned = true;
        }
      }
    }
    if (pruned) saveWorkspaces();

    renderSidebar();
    renderTabs();
    if (state.detailsFor) renderDetails();
  } catch (e) {
    console.error("list_sessions failed:", e);
  }
}

// ── Context menu ────────────────────────────────────────────────

function hideContextMenu() {
  const menu = $("context-menu");
  if (menu) menu.remove();
}

/// items: [{ label, action, danger? }] — null entries become separators.
function showContextMenu(x, y, items) {
  hideContextMenu();
  const menu = document.createElement("div");
  menu.id = "context-menu";
  for (const it of items) {
    if (!it) {
      const sep = document.createElement("div");
      sep.className = "ctx-sep";
      menu.appendChild(sep);
      continue;
    }
    const row = document.createElement("div");
    row.className = "ctx-item" + (it.danger ? " danger" : "");
    row.textContent = it.label;
    row.onclick = (ev) => {
      ev.stopPropagation();
      hideContextMenu();
      it.action();
    };
    menu.appendChild(row);
  }
  document.body.appendChild(menu);
  // Clamp to the viewport so the menu never opens off-screen
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(x, window.innerWidth - r.width - 4)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - r.height - 4)}px`;
}

document.addEventListener("click", hideContextMenu);
window.addEventListener("blur", hideContextMenu);

/// Rename a session via dialog — used by the tab context menu.
async function renameSessionDialog(sid) {
  const s = state.sessions.find((x) => x.id === sid);
  const entry = state.open.get(sid);
  const current = (s && s.name) || (entry && entry.name) || "";
  const name = await uiPrompt("Session name:", current);
  if (!name || !name.trim()) return;
  try {
    await invoke("rename_session", { sessionId: sid, name: name.trim() });
  } catch (e) {
    uiAlert(`Rename failed: ${e}`);
    return;
  }
  if (entry) entry.name = name.trim();
  renderTabs();
  refreshSessions();
}

function tabContextMenu(ev, sid) {
  ev.preventDefault();
  ev.stopPropagation();
  const entry = state.open.get(sid);
  if (entry && !entry.term) {
    // Content tab (conversation/subagents/diff) — only closable
    showContextMenu(ev.clientX, ev.clientY, [
      { label: "Close tab", action: () => dropTerminal(sid) },
    ]);
    return;
  }
  const pr = state.prUrls.get(sid);
  showContextMenu(ev.clientX, ev.clientY, [
    { label: "Rename session…", action: () => renameSessionDialog(sid) },
    { label: "Close tab (detach)", action: () => detachSession(sid) },
    ...(pr
      ? [{ label: `Open PR #${pr.split("/").pop()} ⇄`, action: () => openBrowserPanel(pr) }]
      : []),
    null,
    {
      label: "Stash (stop, keep resumable)",
      action: async () => {
        await invoke("stash_session", { sessionId: sid }).catch(console.error);
        dropTerminal(sid);
        refreshSessions();
      },
    },
    {
      label: "Kill session…",
      danger: true,
      action: async () => {
        const s = state.sessions.find((x) => x.id === sid);
        const label = s ? displayName(s) : sid.slice(0, 8);
        if (!(await uiConfirm(`Kill session "${label}"?`, "Kill"))) return;
        await invoke("kill_session", { sessionId: sid }).catch(console.error);
        dropTerminal(sid);
        refreshSessions();
      },
    },
    null,
    { label: "Details", action: () => showDetails(sid) },
  ]);
}

// ── Tabs ────────────────────────────────────────────────────────

function renderTabs() {
  const tabs = $("tabs");
  tabs.innerHTML = "";
  for (const [id, entry] of state.open) {
    const tab = document.createElement("div");
    tab.className = "tab" + (id === state.activeTab ? " active" : "");
    tab.onclick = () => assignToFocusedPane(id);
    tab.oncontextmenu = (ev) => tabContextMenu(ev, id);

    const s = state.sessions.find((x) => x.id === id);
    if (s) {
      const dot = document.createElement("span");
      dot.className = `tab-dot ${statusClass(s)}`;
      dot.title = statusInfo(s).label;
      tab.appendChild(dot);
    }

    const label = document.createElement("span");
    label.textContent = entry.name;

    const close = document.createElement("span");
    close.className = "close";
    close.textContent = "×";
    close.title = "Detach (session keeps running)";
    close.onclick = (ev) => {
      ev.stopPropagation();
      detachSession(id);
    };

    tab.appendChild(label);
    tab.appendChild(close);
    tabs.appendChild(tab);
  }
}

// ── Panes (split layout) ────────────────────────────────────────

function renderPanes() {
  const host = $("terminal-host");
  const w = ws();
  const visible = w.zoomed ? [w.panes[w.focused] ?? null] : w.panes;
  host.className = `layout-${visible.length}`;

  // Detach term elements first so re-appending doesn't destroy them
  for (const entry of state.open.values()) entry.el.remove();
  host.querySelectorAll(".pane").forEach((p) => p.remove());

  const anyAssigned = w.panes.some((p) => p);
  $("empty-state").style.display = anyAssigned ? "none" : "flex";

  visible.forEach((sid, vi) => {
    const i = w.zoomed ? w.focused : vi;
    const pane = document.createElement("div");
    pane.className = "pane" + (i === w.focused ? " focused" : "");
    pane.onclick = () => {
      w.focused = i;
      if (w.panes[i]) {
        state.activeTab = w.panes[i];
        focusTerm(w.panes[i]);
      }
      renderPanes();
      renderTabs();
      renderSidebar();
    };

    const entry = sid ? state.open.get(sid) : null;
    if (entry) {
      if (visible.length > 1 || w.zoomed) {
        const title = document.createElement("div");
        title.className = "pane-title";
        title.textContent = entry.name + (w.zoomed ? "  (zoomed)" : "");
        pane.appendChild(title);
      }
      pane.appendChild(entry.el);
    } else {
      const empty = document.createElement("div");
      empty.className = "pane-empty";
      empty.textContent = "click a session";
      pane.appendChild(empty);
    }
    host.appendChild(pane);
  });

  fitAll();
}

function fitAll() {
  requestAnimationFrame(() => {
    for (const sid of ws().panes) {
      const entry = sid && state.open.get(sid);
      if (entry && entry.fitAddon) entry.fitAddon.fit();
    }
    if (typeof syncBrowserBounds === "function") syncBrowserBounds();
  });
}

function focusTerm(sid) {
  const entry = state.open.get(sid);
  if (entry && entry.term) setTimeout(() => entry.term.focus(), 0);
}

function addPane() {
  const w = ws();
  if (w.panes.length >= 4) return;
  w.panes.push(null);
  w.focused = w.panes.length - 1;
  w.zoomed = false;
  saveWorkspaces();
  renderPanes();
}

function removePane() {
  const w = ws();
  if (w.panes.length <= 1) return;
  w.panes.pop();
  w.focused = Math.min(w.focused, w.panes.length - 1);
  saveWorkspaces();
  renderPanes();
}

function toggleZoom() {
  const w = ws();
  if (w.panes.length <= 1) return;
  w.zoomed = !w.zoomed;
  renderPanes();
}

function focusPaneDelta(delta) {
  const w = ws();
  if (w.panes.length <= 1) return;
  w.focused = (w.focused + delta + w.panes.length) % w.panes.length;
  const sid = w.panes[w.focused];
  if (sid) {
    state.activeTab = sid;
    focusTerm(sid);
  }
  renderPanes();
  renderTabs();
}

function assignToFocusedPane(sid) {
  const w = ws();
  // If already visible in a pane of this workspace, just focus that pane
  const existing = w.panes.indexOf(sid);
  if (existing >= 0) {
    w.focused = existing;
  } else {
    w.panes[w.focused] = sid;
  }
  state.activeTab = sid;
  state.unread.delete(sid);
  saveWorkspaces();
  renderPanes();
  renderTabs();
  renderSidebar();
  focusTerm(sid);
}

// ── Terminals ───────────────────────────────────────────────────

const TERM_THEME = {
  background: "#141414",
  foreground: "#d4d4d8",
  cursor: "#e8a33d",
  selectionBackground: "#3a3a40",
};

async function openSession(sid) {
  // Sessions are workspace-scoped: owned elsewhere → switch there first;
  // unowned → the active workspace claims it.
  const owner = sessionWorkspace(sid);
  if (owner >= 0 && owner !== state.activeWs) switchWorkspace(owner);
  claimSession(sid);

  if (state.open.has(sid)) {
    assignToFocusedPane(sid);
    return;
  }

  const el = document.createElement("div");
  el.className = "term-wrap";

  const term = new Terminal({
    fontFamily: "SF Mono, Menlo, monospace",
    fontSize: 13,
    theme: TERM_THEME,
    scrollback: 10000,
    macOptionIsMeta: true,
    // OSC 8 hyperlinks (Claude Code emits these) — open in the embedded
    // browser panel, not the system browser.
    linkHandler: {
      activate: (_e, uri) => {
        if (/^https?:\/\//.test(uri)) openBrowserPanel(uri);
        else invoke("open_external", { url: uri }).catch(() => {});
      },
    },
  });
  const fitAddon = new FitAddon.FitAddon();
  term.loadAddon(fitAddon);

  const s = state.sessions.find((x) => x.id === sid);
  state.open.set(sid, {
    term,
    fitAddon,
    el,
    name: s ? displayName(s) : sid.slice(0, 8),
  });

  assignToFocusedPane(sid);
  term.open(el);
  fitAddon.fit();

  try {
    await invoke("open_session", {
      sessionId: sid,
      cols: term.cols,
      rows: term.rows,
    });
  } catch (e) {
    term.writeln(`\x1b[31mFailed to open session: ${e}\x1b[0m`);
  }

  term.onData((data) => {
    invoke("send_input", { sessionId: sid, text: data }).catch(console.error);
  });
  term.onResize(({ cols, rows }) => {
    invoke("resize_session", { sessionId: sid, cols, rows }).catch(() => {});
  });

  // URLs in terminal output are clickable — they open in the embedded
  // browser panel (cmux-style).
  const URL_RE = /https?:\/\/[^\s"'`<>)\]]+/g;
  term.registerLinkProvider({
    provideLinks(y, cb) {
      const line = term.buffer.active.getLine(y - 1);
      if (!line) return cb(undefined);
      const text = line.translateToString(true);
      const links = [];
      URL_RE.lastIndex = 0;
      let m;
      while ((m = URL_RE.exec(text))) {
        links.push({
          range: {
            start: { x: m.index + 1, y },
            end: { x: m.index + m[0].length, y },
          },
          text: m[0],
          activate: (_e, uri) => openBrowserPanel(uri),
        });
      }
      cb(links.length ? links : undefined);
    },
  });

  focusTerm(sid);
}

/// Detach (keep session running in the backend). View tabs just close.
async function detachSession(sid) {
  const entry = state.open.get(sid);
  if (entry && entry.term) {
    try {
      await invoke("close_session", { sessionId: sid });
    } catch (e) {
      console.error("close_session failed:", e);
    }
  }
  dropTerminal(sid);
}

/// Remove the local terminal/view for a tab (after detach/stash/kill/exit).
function dropTerminal(sid) {
  const entry = state.open.get(sid);
  if (!entry) return;
  if (entry.term) entry.term.dispose();
  entry.el.remove();
  state.open.delete(sid);
  for (const w of state.workspaces) {
    w.panes = w.panes.map((p) => (p === sid ? null : p));
  }
  saveWorkspaces();
  if (state.activeTab === sid) {
    const next = state.open.keys().next();
    state.activeTab = next.done ? null : next.value;
  }
  renderPanes();
  renderTabs();
  renderSidebar();
}

// ── View tabs (conversation / subagents / diff in the main area) ──

/// Open (or focus) a non-terminal content tab. `build(el)` fills it.
function openViewTab(key, name, build) {
  if (state.open.has(key)) {
    // Rebuild content so reopening shows fresh data
    const entry = state.open.get(key);
    entry.el.innerHTML = "";
    assignToFocusedPane(key);
    build(entry.el);
    return;
  }
  const el = document.createElement("div");
  el.className = "view-wrap";
  state.open.set(key, { el, name });
  assignToFocusedPane(key);
  build(el);
}

function openConversationTab(s) {
  openViewTab(`view:conv:${s.id}`, `🗨 ${displayName(s)}`, async (el) => {
    el.innerHTML = "<h4>CONVERSATION</h4><p class='hint'>loading…</p>";
    try {
      const msgs = await invoke("get_conversation", {
        project: s.project,
        sessionId: s.id,
      });
      el.innerHTML = "<h4>CONVERSATION</h4>";
      renderChat(el, msgs);
    } catch (e) {
      el.innerHTML = `<h4>CONVERSATION</h4><p class='hint'>failed: ${escapeHtml(e)}</p>`;
    }
  });
}

function openSubagentsTab(s) {
  openViewTab(`view:subs:${s.id}`, `⛭ ${displayName(s)}`, (el) => buildSubagentsList(el, s));
}

async function buildSubagentsList(el, s) {
  el.innerHTML = "<h4>SUBAGENTS</h4><p class='hint'>loading…</p>";
  try {
    const subs = await invoke("get_subagents", {
      project: s.project,
      sessionId: s.id,
    });
    el.innerHTML = "<h4>SUBAGENTS</h4>";
    if (!subs.length) {
      el.innerHTML += "<p class='hint'>no subagents</p>";
      return;
    }
    for (const sub of subs) {
      const row = document.createElement("div");
      row.className = "row-item";
      row.innerHTML = `<span>${escapeHtml(sub.agent_type || sub.id)}</span><span class="dim">${escapeHtml(
        sub.summary || ""
      )}</span>`;
      row.onclick = async () => {
        el.innerHTML = `<div class="row-item back">← all subagents</div><h4>SUBAGENT · ${escapeHtml(
          sub.agent_type || sub.id
        )}</h4>`;
        el.querySelector(".back").onclick = () => buildSubagentsList(el, s);
        try {
          const msgs = await invoke("get_subagent_conversation", {
            project: s.project,
            sessionId: s.id,
            agentId: sub.id,
          });
          renderChat(el, msgs);
        } catch (e) {
          el.innerHTML += `<p class='hint'>failed: ${escapeHtml(e)}</p>`;
        }
      };
      el.appendChild(row);
    }
  } catch (e) {
    el.innerHTML = `<h4>SUBAGENTS</h4><p class='hint'>failed: ${escapeHtml(e)}</p>`;
  }
}

function openDiffTab(s) {
  openViewTab(`view:diff:${s.id}`, `± ${displayName(s)}`, async (el) => {
    el.innerHTML = "<h4>GIT DIFF (HEAD)</h4><div class='diff'>loading…</div>";
    try {
      const diff = await invoke("get_diff", { sessionId: s.id });
      el.querySelector(".diff").innerHTML = renderDiff(diff);
    } catch (e) {
      el.querySelector(".diff").textContent = `diff failed: ${e}`;
    }
  });
}

// ── Details panel ───────────────────────────────────────────────

// Session id whose shell is currently built in #details-body. The shell is
// rebuilt only when this changes; refresh cycles just update field values
// in place so #d-out (conversation, subagents, IDE picker…) is never wiped.
let detailsShellFor = null;

function showDetails(sid) {
  state.detailsFor = sid;
  $("details").classList.remove("hidden");
  $("details-resizer").classList.remove("hidden");
  renderDetails();
  fitAll();
}

function hideDetails() {
  state.detailsFor = null;
  detailsShellFor = null;
  $("details").classList.add("hidden");
  $("details-resizer").classList.add("hidden");
  fitAll();
}

function kv(k, v, id = "") {
  return `<div class="kv"><span class="k">${k}</span><span class="v"${
    id ? ` id="${id}"` : ""
  }>${escapeHtml(v || "—")}</span></div>`;
}

function escapeHtml(s) {
  return String(s)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function detailsStatusText(s) {
  return s.is_running ? s.status + " (running)" : s.status;
}

function renderDetails() {
  const body = $("details-body");
  const s = state.sessions.find((x) => x.id === state.detailsFor);
  if (!s) {
    body.innerHTML = "<p>Session not found.</p>";
    detailsShellFor = null;
    return;
  }
  if (detailsShellFor === s.id) {
    // Refresh tick: update the live fields without touching the DOM tree.
    const set = (id, v) => {
      const el = $(id);
      if (el) el.textContent = v || "—";
    };
    set("d-kv-name", displayName(s));
    set("d-kv-agents", s.subagent_count > 0 ? String(s.subagent_count) : "—");
    set("d-kv-modified", s.last_modified);
    set("d-kv-summary", s.summary || s.first_prompt || "—");
    const st = statusInfo(s);
    const stEl = $("d-kv-status");
    if (stEl) {
      stEl.className = `status-label ${st.cls}`;
      stEl.textContent = `${st.icon} ${st.label}`;
    }
    return;
  }
  detailsShellFor = s.id;
  const st = statusInfo(s);
  body.innerHTML = `
    <h3 id="d-kv-name">${escapeHtml(displayName(s))}</h3>
    <div class="kv"><span class="k">Status</span><span class="status-label ${st.cls}" id="d-kv-status">${st.icon} ${st.label}</span></div>
    ${kv("Branch", s.git_branch)}
    ${kv("Project", s.worktree_project || s.project)}
    ${s.worktree ? kv("Worktree", s.worktree) : ""}
    ${kv("CWD", s.cwd || s.project_path)}
    ${kv("Agents", s.subagent_count > 0 ? String(s.subagent_count) : "—", "d-kv-agents")}
    ${kv("Modified", s.last_modified, "d-kv-modified")}
    <h4>SUMMARY</h4>
    <div class="kv"><span class="v" id="d-kv-summary">${escapeHtml(s.summary || s.first_prompt || "—")}</span></div>
    <h4>OPEN AS TAB</h4>
    <div class="actions">
      <button id="d-conv">🗨 Conversation</button>
      <button id="d-subs">⛭ Subagents</button>
      <button id="d-diff">± Diff</button>
    </div>
    <h4>TOOLS</h4>
    <div class="actions">
      <button id="d-ports">Ports</button>
      <button id="d-ide">Open in IDE</button>
      <button id="d-close">Close panel</button>
    </div>
    <div id="d-out"></div>
    <div class="kv dim-id" title="${escapeHtml(s.id)}"><span class="k">ID</span><span class="v">${escapeHtml(s.id)}</span></div>
  `;
  $("d-close").onclick = hideDetails;
  $("d-diff").onclick = () => openDiffTab(s);
  $("d-conv").onclick = () => openConversationTab(s);
  $("d-subs").onclick = () => openSubagentsTab(s);
  $("d-ide").onclick = () => showIdePicker(s);
  $("d-ports").onclick = async () => {
    const out = $("d-out");
    out.innerHTML = "<h4>LISTENING PORTS</h4><p class='hint'>scanning…</p>";
    try {
      const ports = await invoke("get_session_ports", { sessionId: s.id });
      out.innerHTML =
        "<h4>LISTENING PORTS</h4>" +
        (ports.length
          ? ports
              .map(
                (p) =>
                  `<div class="row-item port" data-port="${escapeHtml(p)}"><span>:${escapeHtml(p)}</span><span class="dim">http://localhost:${escapeHtml(p)}</span></div>`
              )
              .join("")
          : "<p class='hint'>no listening ports</p>");
      out.querySelectorAll(".row-item.port").forEach((row) => {
        row.onclick = () => openBrowserPanel(`http://localhost:${row.dataset.port}`);
      });
    } catch (e) {
      out.innerHTML = `<h4>LISTENING PORTS</h4><p class='hint'>failed: ${escapeHtml(e)}</p>`;
    }
  };
}

function renderChat(out, msgs) {
  if (!msgs.length) {
    out.innerHTML += "<p class='hint'>empty conversation</p>";
    return;
  }
  const chat = document.createElement("div");
  chat.className = "chat";
  for (const m of msgs) {
    const div = document.createElement("div");
    div.className = `msg ${m.role === "user" ? "user" : "assistant"}`;
    const who = document.createElement("span");
    who.className = "who";
    who.textContent = m.role.toUpperCase();
    div.appendChild(who);
    div.appendChild(document.createTextNode(m.text));
    chat.appendChild(div);
  }
  out.appendChild(chat);
  chat.scrollTop = chat.scrollHeight;
}

async function showIdePicker(s) {
  const out = $("d-out");
  out.innerHTML = "<h4>OPEN IN IDE</h4>";
  const dir = s.worktree || s.cwd || s.project_path;
  const ides = await invoke("detect_ides").catch(() => []);
  if (!ides.length) {
    out.innerHTML += "<p class='hint'>no IDEs detected</p>";
    return;
  }
  for (const ide of ides) {
    const row = document.createElement("div");
    row.className = "row-item";
    row.innerHTML = `<span>${escapeHtml(ide.label)}</span><span class="dim">${escapeHtml(
      ide.description
    )}</span>`;
    row.onclick = async () => {
      try {
        await invoke("open_in_ide", { value: ide.value, projectDir: dir });
        out.innerHTML = `<p class='hint'>opened in ${escapeHtml(ide.label)}</p>`;
      } catch (e) {
        out.innerHTML += `<p class='hint'>failed: ${escapeHtml(e)}</p>`;
      }
    };
    out.appendChild(row);
  }
}

function renderDiff(text) {
  if (!text.trim()) return "no changes";
  return text
    .split("\n")
    .map((line) => {
      const esc = escapeHtml(line);
      if (line.startsWith("+++") || line.startsWith("---"))
        return `<span class="file">${esc}</span>`;
      if (line.startsWith("@@")) return `<span class="hunk">${esc}</span>`;
      if (line.startsWith("diff ")) return `<span class="file">${esc}</span>`;
      if (line.startsWith("+")) return `<span class="add">${esc}</span>`;
      if (line.startsWith("-")) return `<span class="del">${esc}</span>`;
      return esc;
    })
    .join("\n");
}

// ── Teams ───────────────────────────────────────────────────────

async function toggleTeams() {
  state.teamsOpen = !state.teamsOpen;
  $("teams-caret").textContent = state.teamsOpen ? "▾" : "▸";
  $("teams-list").classList.toggle("hidden", !state.teamsOpen);
  if (state.teamsOpen) {
    try {
      state.teams = await invoke("list_teams");
    } catch (e) {
      console.error("list_teams failed:", e);
      state.teams = [];
    }
    renderTeams();
  }
}

function renderTeams() {
  const list = $("teams-list");
  list.innerHTML = "";
  if (state.teams.length === 0) {
    const empty = document.createElement("div");
    empty.className = "team-item";
    empty.textContent = "no teams";
    list.appendChild(empty);
    return;
  }
  for (const t of state.teams) {
    const item = document.createElement("div");
    item.className = "team-item";
    item.innerHTML = `<span>${escapeHtml(t.name)}</span><span class="count">${
      (t.members || []).length
    } agents</span>`;
    item.onclick = () => showTeamDetails(t);
    list.appendChild(item);
  }
}

async function showTeamDetails(team) {
  $("details").classList.remove("hidden");
  $("details-resizer").classList.remove("hidden");
  state.detailsFor = null;
  detailsShellFor = null; // team view replaces the session shell
  const body = $("details-body");
  let tasks = [];
  try {
    tasks = await invoke("list_tasks", { team: team.name });
  } catch (e) {
    console.error("list_tasks failed:", e);
  }
  const taskRows = tasks.length
    ? tasks
        .map((t) => {
          const st = String(t.status || "").toLowerCase().replace(" ", "_");
          return `<div class="task-item"><span class="task-status ${st}">${escapeHtml(
            String(t.status)
          )}</span><span>${escapeHtml(t.subject || t.id)}</span></div>`;
        })
        .join("")
    : "<p class='hint'>no tasks</p>";
  body.innerHTML = `
    <h3>${escapeHtml(team.name)}</h3>
    <div class="kv"><span class="v">${escapeHtml(team.description || "")}</span></div>
    <h4>MEMBERS <span class="dim" style="font-weight:400">(click for inbox)</span></h4>
    <div id="d-members"></div>
    <h4>TASKS</h4>
    ${taskRows}
    <div class="actions">
      <button id="d-team-delete" class="danger">Delete team</button>
      <button id="d-close">Close panel</button>
    </div>
    <div id="d-out"></div>
  `;
  const membersEl = $("d-members");
  if ((team.members || []).length === 0) {
    membersEl.innerHTML = "<p class='hint'>none</p>";
  }
  for (const m of team.members || []) {
    const row = document.createElement("div");
    row.className = "row-item";
    row.innerHTML = `<span>${escapeHtml(m.name)}</span><span class="dim">${escapeHtml(
      m.agent_type + (m.model ? ` · ${m.model}` : "")
    )}</span>`;
    row.onclick = () => showInbox(team.name, m.name);
    membersEl.appendChild(row);
  }
  $("d-close").onclick = hideDetails;
  $("d-team-delete").onclick = async () => {
    if (!(await uiConfirm(`Delete team "${team.name}" and all its tasks?`, "Delete"))) return;
    try {
      await invoke("delete_team", { name: team.name });
      hideDetails();
      state.teams = await invoke("list_teams");
      renderTeams();
    } catch (e) {
      uiAlert(`Delete failed: ${e}`);
    }
  };
  fitAll();
}

async function showInbox(team, agent) {
  const out = $("d-out");
  out.innerHTML = `<h4>INBOX · ${escapeHtml(agent)}</h4>`;
  try {
    const msgs = await invoke("get_inbox", { team, agent });
    if (!msgs.length) {
      out.innerHTML += "<p class='hint'>empty inbox</p>";
      return;
    }
    for (const m of msgs) {
      const div = document.createElement("div");
      div.className = "inbox-msg" + (m.read ? "" : " unread");
      const who = document.createElement("div");
      who.className = "who";
      who.textContent = m.from || "?";
      div.appendChild(who);
      div.appendChild(document.createTextNode(m.text || ""));
      out.appendChild(div);
    }
  } catch (e) {
    out.innerHTML += `<p class='hint'>failed: ${escapeHtml(e)}</p>`;
  }
}

async function createTeamPrompt() {
  const name = await uiPrompt("Team name:");
  if (!name || !name.trim()) return;
  const description = (await uiPrompt("Description (optional):")) || "";
  try {
    await invoke("create_team", { name: name.trim(), description });
    state.teams = await invoke("list_teams");
    renderTeams();
  } catch (e) {
    uiAlert(`Create team failed: ${e}`);
  }
}

// ── New session modal ───────────────────────────────────────────

let nsPresets = [];

function showNewSessionModal() {
  $("ns-error").classList.add("hidden");
  $("modal-backdrop").classList.remove("hidden");
  // Prefill cwd from the focused session's project for fast iteration
  const cur = state.sessions.find((x) => x.id === state.activeTab);
  if (cur && !$("ns-cwd").value) {
    $("ns-cwd").value = cur.cwd || cur.project_path || "";
    loadPresetsForCwd();
  }
  setTimeout(() => $("ns-cwd").focus(), 0);
}

function hideNewSessionModal() {
  $("modal-backdrop").classList.add("hidden");
}

async function loadPresetsForCwd() {
  const cwd = $("ns-cwd").value.trim();
  const wrap = $("ns-preset-wrap");
  const select = $("ns-preset");
  select.innerHTML = `<option value="">— none —</option>`;
  nsPresets = [];
  if (!cwd) {
    wrap.classList.add("hidden");
    return;
  }
  try {
    nsPresets = await invoke("list_presets", { projectDir: cwd });
  } catch (e) {
    console.error("list_presets failed:", e);
  }
  if (nsPresets.length === 0) {
    wrap.classList.add("hidden");
    return;
  }
  nsPresets.forEach((p, i) => {
    const opt = document.createElement("option");
    opt.value = String(i);
    opt.textContent = p.description ? `${p.name} — ${p.description}` : p.name;
    select.appendChild(opt);
  });
  wrap.classList.remove("hidden");
}

function selectedPreset() {
  const v = $("ns-preset").value;
  return v === "" ? null : nsPresets[Number(v)];
}

async function createSession() {
  const name = $("ns-name").value;
  let cwd = $("ns-cwd").value.trim();
  const preset = selectedPreset();
  let worktree = $("ns-worktree").checked;

  if (preset) {
    if (preset.directory && preset.directory !== ".") {
      cwd = `${cwd.replace(/\/$/, "")}/${preset.directory.replace(/^\.\//, "")}`;
    }
    if (preset.worktree === true) worktree = true;
  }

  try {
    let sid;
    if (worktree) {
      const wtName = (name || (preset ? preset.name : "")).trim();
      sid = await invoke("create_worktree_session", {
        name: wtName,
        projectPath: cwd,
        cols: 120,
        rows: 40,
      });
    } else {
      sid = await invoke("create_new_session", {
        name: name || (preset ? preset.name : ""),
        cwd,
        cols: 120,
        rows: 40,
      });
    }
    hideNewSessionModal();
    $("ns-name").value = "";
    $("ns-worktree").checked = false;
    await refreshSessions();
    await openSession(sid);
    // Preset prompt: typed into the fresh session once Claude has started
    if (preset && preset.prompt) {
      setTimeout(() => {
        invoke("send_input", {
          sessionId: sid,
          text: preset.prompt + "\r",
        }).catch(console.error);
      }, 3000);
    }
  } catch (e) {
    const err = $("ns-error");
    err.textContent = String(e);
    err.classList.remove("hidden");
  }
}

// ── PTY event stream ────────────────────────────────────────────

function base64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

// PR sniffing: the newest GitHub PR URL seen in each session's output
// (sidebar chip + tab context menu, cmux-style). A rolling tail buffers
// URLs split across output chunks.
const PR_RE = /https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/pull\/\d+/g;
const prTails = new Map(); // session id -> recent output text
state.prUrls = new Map(); // session id -> last PR url

function sniffPrUrl(sid, bytes) {
  let text;
  try {
    text = new TextDecoder().decode(bytes);
  } catch {
    return;
  }
  const tail = ((prTails.get(sid) || "") + text).slice(-4096);
  prTails.set(sid, tail);
  PR_RE.lastIndex = 0;
  let m;
  let last = null;
  while ((m = PR_RE.exec(tail))) last = m[0];
  if (last && state.prUrls.get(sid) !== last) {
    state.prUrls.set(sid, last);
    renderSidebar();
  }
}

listen("pty-output", (event) => {
  const { session_id, data } = event.payload;
  const bytes = base64ToBytes(data);
  const entry = state.open.get(session_id);
  if (entry && entry.term) entry.term.write(bytes);
  sniffPrUrl(session_id, bytes);
});

listen("session-attention", (event) => {
  const { session_id } = event.payload;
  // Badge unless the session is in a visible pane of the active workspace
  // and the window is focused (cmux-style suppression).
  const visible = ws().panes.includes(session_id) && document.hasFocus();
  if (!visible) {
    state.unread.add(session_id);
    renderSidebar();
  }
});

listen("pty-exited", (event) => {
  const { session_id, exit_code } = event.payload;
  const entry = state.open.get(session_id);
  if (entry) {
    entry.term.writeln(
      `\r\n\x1b[33m── session exited (${exit_code ?? "?"}) ──\x1b[0m`
    );
  }
  refreshSessions();
});

// ── Wiring ──────────────────────────────────────────────────────

$("search").addEventListener("input", (e) => {
  state.query = e.target.value.trim();
  renderSidebar();
});

$("new-session-btn").onclick = showNewSessionModal;
$("ns-cancel").onclick = hideNewSessionModal;
$("ns-create").onclick = createSession;
$("modal-backdrop").addEventListener("click", (e) => {
  if (e.target === $("modal-backdrop")) hideNewSessionModal();
});
$("ns-cwd").addEventListener("keydown", (e) => {
  if (e.key === "Enter") createSession();
});
$("ns-cwd").addEventListener("blur", loadPresetsForCwd);
$("ns-preset").addEventListener("change", () => {
  const p = selectedPreset();
  if (p && !$("ns-name").value) $("ns-name").value = p.name;
  if (p && p.worktree === true) $("ns-worktree").checked = true;
});

$("stash-all-btn").onclick = async () => {
  if (!(await uiConfirm("Stash all running sessions?", "Stash all"))) return;
  try {
    const n = await invoke("stash_all");
    for (const sid of [...state.open.keys()]) dropTerminal(sid);
    refreshSessions();
    console.log(`stashed ${n} sessions`);
  } catch (e) {
    uiAlert(`Stash all failed: ${e}`);
  }
};

$("new-team-btn").onclick = (e) => {
  e.stopPropagation();
  createTeamPrompt();
};

$("update-btn").onclick = () => {
  $("version").textContent = "checking…";
  invoke("start_update").catch(console.error);
};

listen("update-phase", (event) => {
  const { phase, version, message } = event.payload;
  const v = $("version");
  switch (phase) {
    case "checking":
      v.textContent = "checking…";
      break;
    case "downloading":
      v.textContent = `downloading v${version}…`;
      break;
    case "extracting":
      v.textContent = "extracting…";
      break;
    case "installing":
      v.textContent = "installing…";
      break;
    case "done":
      v.textContent = `v${version} installed — restart`;
      break;
    case "failed":
      v.textContent = message || "update failed";
      setTimeout(setVersionLabel, 5000);
      break;
  }
});

async function setVersionLabel() {
  try {
    $("version").textContent = `v${await invoke("get_version")}`;
  } catch {
    $("version").textContent = "";
  }
}

$("split-btn").onclick = addPane;
$("unsplit-btn").onclick = removePane;
$("details-btn").onclick = () => {
  if ($("details").classList.contains("hidden")) {
    if (state.activeTab) showDetails(state.activeTab);
    else if (state.sessions[0]) showDetails(state.sessions[0].id);
  } else {
    hideDetails();
  }
};
$("teams-toggle").onclick = toggleTeams;

document.addEventListener("keydown", (e) => {
  const inInput =
    document.activeElement &&
    (document.activeElement.tagName === "INPUT" ||
      document.activeElement.classList.contains("xterm-helper-textarea"));

  if (e.key === "Escape") {
    hideNewSessionModal();
    if (document.activeElement === $("search")) {
      $("search").blur();
      state.query = "";
      $("search").value = "";
      renderSidebar();
    }
    return;
  }
  if (e.metaKey && e.key === "t") {
    e.preventDefault();
    showNewSessionModal();
    return;
  }
  if (e.metaKey && e.key === "d") {
    e.preventDefault();
    addPane();
    return;
  }
  // Workspace shortcuts (cmux layout: ⌘N new, ⌘1-9 switch, ⌘⇧R rename, ⌘⇧W close)
  if (e.metaKey && !e.shiftKey && e.key === "n") {
    e.preventDefault();
    newWorkspace();
    return;
  }
  if (e.metaKey && !e.shiftKey && e.key >= "1" && e.key <= "9") {
    e.preventDefault();
    switchWorkspace(Number(e.key) - 1);
    return;
  }
  if (e.metaKey && e.shiftKey && e.key.toLowerCase() === "r") {
    e.preventDefault();
    renameWorkspace(state.activeWs);
    return;
  }
  if (e.metaKey && e.shiftKey && e.key.toLowerCase() === "w") {
    e.preventDefault();
    closeWorkspace();
    return;
  }
  if (e.metaKey && e.key === "b") {
    e.preventDefault();
    $("sidebar").classList.toggle("collapsed");
    fitAll();
    return;
  }
  if (e.metaKey && e.shiftKey && e.key === "Enter") {
    e.preventDefault();
    toggleZoom();
    return;
  }
  if (e.metaKey && e.shiftKey && e.key.toLowerCase() === "b") {
    e.preventDefault();
    toggleBrowserPanel();
    return;
  }
  if (e.metaKey && e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
    e.preventDefault();
    focusPaneDelta(e.key === "ArrowRight" ? 1 : -1);
    return;
  }
  if (e.metaKey && !e.shiftKey && e.key === "w") {
    e.preventDefault();
    if (state.activeTab) detachSession(state.activeTab);
    return;
  }
  if (e.metaKey && e.key === "f") {
    e.preventDefault();
    $("search").focus();
    return;
  }
  if (e.key === "/" && !inInput) {
    e.preventDefault();
    $("search").focus();
  }
});

window.addEventListener("resize", fitAll);

// ── Embedded browser panel (cmux-style) ─────────────────────────
// A native child webview is positioned over #browser-slot by the
// backend; the frontend reports the slot's rect whenever layout moves.

let browserShown = false;
let browserLastUrl = null;
let browserUrlPoll = null;

function browserSlotRect() {
  const r = $("browser-slot").getBoundingClientRect();
  return { x: r.x, y: r.y, w: r.width, h: r.height };
}

async function openBrowserPanel(url) {
  browserLastUrl = url;
  $("browser").classList.remove("hidden");
  $("browser-resizer").classList.remove("hidden");
  fitAll();
  // Layout must settle before measuring the slot
  await new Promise((r) => requestAnimationFrame(r));
  const { x, y, w, h } = browserSlotRect();
  try {
    await invoke("browser_open", { url, x, y, w, h });
    browserShown = true;
    if (document.activeElement !== $("b-url")) $("b-url").value = url;
    if (!browserUrlPoll) browserUrlPoll = setInterval(syncBrowserUrl, 1500);
  } catch (e) {
    closeBrowserPanel();
    uiAlert(`Browser failed: ${e}`);
  }
}

function closeBrowserPanel() {
  browserShown = false;
  if (browserUrlPoll) {
    clearInterval(browserUrlPoll);
    browserUrlPoll = null;
  }
  $("browser").classList.add("hidden");
  $("browser-resizer").classList.add("hidden");
  invoke("browser_close").catch(() => {});
  fitAll();
}

function toggleBrowserPanel() {
  if (browserShown) closeBrowserPanel();
  else openBrowserPanel(browserLastUrl || "https://github.com");
}

async function syncBrowserUrl() {
  if (!browserShown) return;
  try {
    const url = await invoke("browser_get_url");
    browserLastUrl = url;
    if (document.activeElement !== $("b-url")) $("b-url").value = url;
  } catch (e) {
    void e;
  }
}

function syncBrowserBounds() {
  if (!browserShown) return;
  const { x, y, w, h } = browserSlotRect();
  invoke("browser_bounds", { x, y, w, h }).catch(() => {});
}

new ResizeObserver(syncBrowserBounds).observe($("browser-slot"));
window.addEventListener("resize", syncBrowserBounds);

$("b-back").onclick = () => invoke("browser_history", { delta: -1 }).catch(() => {});
$("b-fwd").onclick = () => invoke("browser_history", { delta: 1 }).catch(() => {});
$("b-reload").onclick = () => invoke("browser_reload").catch(() => {});
$("b-close").onclick = closeBrowserPanel;
$("b-ext").onclick = () => {
  if (browserLastUrl) invoke("open_external", { url: browserLastUrl }).catch(console.error);
};
$("b-url").addEventListener("keydown", (e) => {
  if (e.key !== "Enter") return;
  let url = $("b-url").value.trim();
  if (!url) return;
  if (!/^https?:\/\//.test(url)) url = "https://" + url;
  browserLastUrl = url;
  invoke("browser_navigate", { url }).catch((err) => uiAlert(`Navigate failed: ${err}`));
});

// ── Panel resizing (sidebar / details) ──────────────────────────

function loadPanelSizes() {
  try {
    const sizes = JSON.parse(localStorage.getItem("clash-panel-sizes") || "{}");
    const apply = (el, px) => {
      el.style.width = px + "px";
      el.style.minWidth = px + "px";
    };
    if (sizes.sidebar) apply($("sidebar"), sizes.sidebar);
    if (sizes.details) apply($("details"), sizes.details);
    if (sizes.browser) apply($("browser"), sizes.browser);
  } catch (e) {
    console.error("loadPanelSizes failed:", e);
  }
}

function savePanelSize(key, px) {
  try {
    const sizes = JSON.parse(localStorage.getItem("clash-panel-sizes") || "{}");
    sizes[key] = px;
    localStorage.setItem("clash-panel-sizes", JSON.stringify(sizes));
  } catch (e) {
    console.error("savePanelSize failed:", e);
  }
}

/// Horizontal drag-to-resize. `compute(clientX)` returns the new width.
function initResizer(handleId, panelId, storageKey, min, max, compute) {
  const handle = $(handleId);
  const panel = $(panelId);
  handle.addEventListener("mousedown", (e) => {
    e.preventDefault();
    handle.classList.add("dragging");
    document.body.style.cursor = "col-resize";
    const onMove = (ev) => {
      const w = Math.max(min, Math.min(max, compute(ev.clientX)));
      panel.style.width = w + "px";
      panel.style.minWidth = w + "px";
      fitAll();
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      handle.classList.remove("dragging");
      document.body.style.cursor = "";
      savePanelSize(storageKey, parseInt(panel.style.width, 10));
      fitAll();
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  });
}

initResizer("sidebar-resizer", "sidebar", "sidebar", 180, 480, (x) => x);
initResizer("details-resizer", "details", "details", 240, 640, (x) => window.innerWidth - x);
initResizer(
  "browser-resizer",
  "browser",
  "browser",
  300,
  1200,
  (x) => $("browser").getBoundingClientRect().right - x
);
loadPanelSizes();

$("new-ws-btn").onclick = newWorkspace;

// ── Boot ────────────────────────────────────────────────────────

(async () => {
  await loadWorkspaces(); // disk-backed — must complete before first render
  renderAll();
  setVersionLabel();
  await refreshSessions();
  await restoreWorkspaceSessions();
  setInterval(refreshSessions, 2000);
})();
