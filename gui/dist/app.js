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
  openTeamPanel: null, // team name shown in the details panel (for live refresh)
  notes: [],
  notesOpen: false,
  notesExpanded: new Set(), // scratch folder ids (rel paths) expanded in the tree
  notesDragId: null, // id of the scratch entry currently being dragged
  renaming: null, // session id with an open inline-rename input
  prevStatuses: new Map(), // session id -> status (attention transitions)
  unread: new Set(), // session ids with unseen attention events
  missingStreak: new Map(), // session id -> consecutive refreshes absent (ownership prune)
  // Persisted with workspaces in gui-state.json. optionMeta: ⌥ sends
  // Esc (Meta) in terminals; off = ⌥ always composes characters.
  settings: {
    defaultCwd: "",
    fontSize: 13,
    fontFamily: "SF Mono, Menlo, monospace",
    scrollback: 10000,
    cursorStyle: "block", // block | bar | underline
    cursorBlink: false,
    copyOnSelect: false,
    optionMeta: true,
    linkOpen: "ask", // ask | embedded | external — how terminal links open
    notifications: true,
    tuiTerminal: "", // last terminal picked for the TUI launcher ("" = auto)
    termShell: "", // last shell picked for in-app terminals ("" = $SHELL)
  },
  homeDir: "", // resolved at startup — last-resort new-session prefill
};

const $ = (id) => document.getElementById(id);

/// Spin an icon button's glyph while an async task runs, so a click gives
/// immediate visible feedback. A minimum spin time keeps near-instant work
/// (e.g. re-listing scratches) perceptible; the class is always cleared, even
/// if the task throws. Reusable for any `.icon-btn` that fires async work.
async function spinButton(btn, work, minMs = 500) {
  if (!btn) return work();
  btn.classList.add("spinning");
  const start = performance.now();
  try {
    return await work();
  } finally {
    const wait = Math.max(0, minMs - (performance.now() - start));
    if (wait) await new Promise((r) => setTimeout(r, wait));
    btn.classList.remove("spinning");
  }
}

// ── SVG icon set ────────────────────────────────────────────────
// Feather-style stroke icons. Unicode glyphs render inconsistently
// across fonts; these inherit color via currentColor and scale crisply.

const ICONS = {
  pencil: '<path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/>',
  x: '<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>',
  pause: '<rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/>',
  info: '<circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/>',
  alert: '<path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>',
  pr: '<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M13 6h3a2 2 0 0 1 2 2v7"/><line x1="6" y1="9" x2="6" y2="21"/>',
  zap: '<polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>',
  kebab: '<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>',
  plus: '<line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>',
  minus: '<line x1="5" y1="12" x2="19" y2="12"/>',
  "arrow-left": '<line x1="19" y1="12" x2="5" y2="12"/><polyline points="12 19 5 12 12 5"/>',
  "arrow-right": '<line x1="5" y1="12" x2="19" y2="12"/><polyline points="12 5 19 12 12 19"/>',
  reload: '<polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>',
  "external-link": '<path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/>',
  copy: '<rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/>',
  columns: '<rect x="3" y="3" width="18" height="18" rx="2"/><line x1="12" y1="3" x2="12" y2="21"/>',
  square: '<rect x="3" y="3" width="18" height="18" rx="2"/>',
  terminal: '<polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/>',
  users:
    '<path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>',
  folder:
    '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>',
  file: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/>',
  chevron: '<polyline points="9 18 15 12 9 6"/>',
};

function svgIcon(name, size = 15) {
  const body = ICONS[name];
  if (!body) return "";
  return `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;
}

/// Swap the static buttons' unicode glyphs for SVG icons at boot.
function applyStaticIcons() {
  const map = {
    "new-ws-btn": "plus",
    "new-team-btn": "plus",
    "new-note-btn": "plus",
    "split-btn": "columns",
    "unsplit-btn": "square",
    "details-btn": "info",
    "new-term-btn": "terminal",
  };
  for (const [id, name] of Object.entries(map)) {
    const el = $(id);
    if (el) el.innerHTML = svgIcon(name);
  }
  $("stash-all-btn").innerHTML = `${svgIcon("pause", 13)}<span>all</span>`;
  // Labeled launcher, not a bare glyph — it must read as "click to get
  // the TUI" next to the GUI badge, not as a mystery toolbar icon.
  $("tui-btn").innerHTML = `${svgIcon("terminal", 12)}<span>TUI</span>`;
}

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
    // Native browser webviews paint over the DOM and would hide the
    // dialog — drop them while it's up; fitAll() brings them back.
    if (typeof hideBrowserWebviews === "function") hideBrowserWebviews();
    document.body.appendChild(backdrop);
    const done = (val) => {
      backdrop.remove();
      resolve(val);
      if (typeof fitAll === "function") fitAll();
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

/// A modal that asks the user to pick one of several labeled actions.
/// `choices` is [{ label, value, primary? }]; resolves to the chosen value,
/// or null if cancelled. `detail` renders on its own line under the message
/// (used to show the URL being opened, wrapped so long links don't overflow).
function uiChoice({ message, detail = null, choices }) {
  return new Promise((resolve) => {
    const backdrop = document.createElement("div");
    backdrop.className = "dialog-backdrop";
    const box = document.createElement("div");
    box.className = "dialog-box";
    const msg = document.createElement("p");
    msg.textContent = message;
    box.appendChild(msg);
    if (detail !== null) {
      const d = document.createElement("p");
      d.className = "dialog-detail";
      d.textContent = detail;
      box.appendChild(d);
    }
    const actions = document.createElement("div");
    actions.className = "modal-actions";
    const done = (val) => {
      backdrop.remove();
      resolve(val);
      if (typeof fitAll === "function") fitAll();
    };
    const cancel = document.createElement("button");
    cancel.textContent = "Cancel";
    cancel.onclick = () => done(null);
    actions.appendChild(cancel);
    let firstBtn = null;
    for (const c of choices) {
      const b = document.createElement("button");
      b.textContent = c.label;
      if (c.primary) b.className = "primary";
      b.onclick = () => done(c.value);
      actions.appendChild(b);
      if (!firstBtn || c.primary) firstBtn = b;
    }
    box.appendChild(actions);
    backdrop.appendChild(box);
    // Native browser webviews paint over the DOM and would hide the dialog —
    // drop them while it's up; fitAll() (in done) brings them back.
    if (typeof hideBrowserWebviews === "function") hideBrowserWebviews();
    document.body.appendChild(backdrop);
    backdrop.onclick = (e) => {
      if (e.target === backdrop) done(null);
    };
    backdrop.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Escape") done(null);
    });
    setTimeout(() => firstBtn && firstBtn.focus(), 0);
  });
}

/// Open a URL from terminal output per the "Open links" setting: inside the
/// embedded browser panel, in the system browser, or (default) by asking each
/// time. The per-open prompt is the requested behavior — a link could belong
/// in either place, so let the user choose at click time.
async function openLink(uri) {
  const embed = () => openBrowserTab(uri, "split");
  const external = () => invoke("open_external", { url: uri }).catch(() => {});
  const isHttp = /^https?:\/\//.test(uri);
  // Non-http(s) schemes (mailto:, tel:, file:, …) can't render in the panel —
  // always hand them to the OS regardless of the setting.
  if (!isHttp) return external();
  const mode = state.settings.linkOpen;
  if (mode === "embedded") return embed();
  if (mode === "external") return external();
  const choice = await uiChoice({
    message: "Open link",
    detail: uri,
    choices: [
      { label: "In clash", value: "embedded", primary: true },
      { label: "System browser", value: "external" },
    ],
  });
  if (choice === "embedded") embed();
  else if (choice === "external") external();
}

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
  const browserTabs = [];
  for (const [id, e] of state.open) {
    if (e.kind === "browser") {
      browserTabs.push({ id, url: e.url, name: e.name, renamed: !!e.renamed });
    }
  }
  return JSON.stringify({
    workspaces: state.workspaces.map((w) => ({
      name: w.name,
      panes: w.panes,
      sessions: w.sessions,
      colFracs: w.colFracs,
      rowFracs: w.rowFracs,
      // "Where we were": which pane was focused and whether it was zoomed, so a
      // relaunch restores the exact view — not just the set of open tabs.
      focused: w.focused,
      zoomed: w.zoomed,
    })),
    browserTabs,
    active: state.activeWs,
    settings: state.settings,
  });
}

/// Write the workspace/layout state to disk *now*, bypassing the debounce.
/// Called when clash loses focus / is hidden / is closing so the latest
/// "where we were" is never lost to a pending debounce timer.
function flushWorkspaces() {
  clearTimeout(saveTimer);
  const json = workspacesJson();
  try {
    localStorage.setItem("clash-workspaces", json);
  } catch (e) {
    void e;
  }
  invoke("save_gui_state", { stateJson: json }).catch(() => {});
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
  if (!data) return false;
  // Settings ride along with the workspaces blob but load independently —
  // a fresh install with no workspaces yet still gets its saved settings.
  // Per-key type checks so a stale/corrupt blob never poisons defaults.
  if (data.settings) {
    const s = data.settings;
    if (typeof s.defaultCwd === "string") state.settings.defaultCwd = s.defaultCwd;
    if (typeof s.fontSize === "number" && s.fontSize >= 9 && s.fontSize <= 24) {
      state.settings.fontSize = Math.round(s.fontSize);
    }
    if (typeof s.fontFamily === "string" && s.fontFamily.trim()) {
      state.settings.fontFamily = s.fontFamily.trim();
    }
    if (typeof s.scrollback === "number" && s.scrollback >= 0 && s.scrollback <= 200000) {
      state.settings.scrollback = Math.round(s.scrollback);
    }
    if (["block", "bar", "underline"].includes(s.cursorStyle)) {
      state.settings.cursorStyle = s.cursorStyle;
    }
    if (typeof s.cursorBlink === "boolean") state.settings.cursorBlink = s.cursorBlink;
    if (typeof s.copyOnSelect === "boolean") state.settings.copyOnSelect = s.copyOnSelect;
    if (typeof s.optionMeta === "boolean") state.settings.optionMeta = s.optionMeta;
    if (["ask", "embedded", "external"].includes(s.linkOpen)) {
      state.settings.linkOpen = s.linkOpen;
    } else if (typeof s.embedLinks === "boolean") {
      // Legacy boolean setting → map to the new three-way choice.
      state.settings.linkOpen = s.embedLinks ? "embedded" : "external";
    }
    if (typeof s.notifications === "boolean") state.settings.notifications = s.notifications;
    if (typeof s.tuiTerminal === "string") state.settings.tuiTerminal = s.tuiTerminal;
    if (typeof s.termShell === "string") state.settings.termShell = s.termShell;
  }
  if (!Array.isArray(data.workspaces) || !data.workspaces.length) return false;
  // Shell terminals die with the app (in-process daemon) — drop any
  // persisted from the previous run. Browser tabs survive: their URLs
  // are persisted and the webviews are recreated lazily.
  const livePane = (p) => (p && isShellTerm(p) ? null : p);
  state.workspaces = data.workspaces.map((w) => {
    const panes =
      Array.isArray(w.panes) && w.panes.length ? w.panes.map(livePane) : [null];
    // Restore the focused pane (clamped to the pane count) and zoom, so the
    // relaunched app lands on the same view we left.
    const focused =
      Number.isInteger(w.focused) && w.focused >= 0 && w.focused < panes.length
        ? w.focused
        : 0;
    return {
      name: w.name || "ws",
      panes,
      focused,
      zoomed: !!w.zoomed && panes.length > 1,
      sessions: Array.isArray(w.sessions) ? w.sessions.filter((id) => !isShellTerm(id)) : [],
      // Pane track sizes; renderPanes resets them if they no longer match the
      // grid shape (pane count changed since the layout was saved).
      colFracs: Array.isArray(w.colFracs) ? w.colFracs : undefined,
      rowFracs: Array.isArray(w.rowFracs) ? w.rowFracs : undefined,
    };
  });
  pendingBrowserTabs = Array.isArray(data.browserTabs) ? data.browserTabs : [];
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

/// Restore sessions referenced by saved workspace panes. Running sessions
/// re-attach immediately; stashed sessions reopen as deferred tabs that
/// resume (claude --resume) only when first focused/clicked. Sessions that
/// no longer exist on disk are cleared from their slots.
async function restoreWorkspaceSessions() {
  // A pane id saved before a `/clear` is stale — Claude re-keyed the
  // conversation to a new id. Resolve every saved id forward to its current
  // conversation id so we match list_sessions (and resume the latest), then
  // persist the rewrite. Unknown ids pass through unchanged.
  const saved = [];
  for (const w of state.workspaces)
    for (const p of w.panes) if (p && !isBrowserTab(p)) saved.push(p);
  if (saved.length) {
    try {
      const resolved = await invoke("resolve_session_ids", { ids: saved });
      const remap = new Map();
      saved.forEach((id, i) => {
        if (resolved[i] && resolved[i] !== id) remap.set(id, resolved[i]);
      });
      if (remap.size) {
        for (const w of state.workspaces) {
          w.panes = w.panes.map((p) => (p && remap.get(p)) || p);
          w.sessions = w.sessions.map((s) => remap.get(s) || s);
        }
        saveWorkspaces();
      }
    } catch (e) {
      console.error("resolve_session_ids failed:", e);
    }
  }

  const byId = new Map(state.sessions.map((s) => [s.id, s]));
  const savedActive = state.activeWs;
  // The restore loop drives w.focused pane-by-pane as it assigns sessions, so
  // remember the saved focus per workspace and restore it afterwards — that's
  // the pane we were actually on.
  const savedFocused = state.workspaces.map((w) => w.focused);
  for (let wi = 0; wi < state.workspaces.length; wi++) {
    const w = state.workspaces[wi];
    for (let pi = 0; pi < w.panes.length; pi++) {
      const sid = w.panes[pi];
      if (!sid) continue;
      if (isBrowserTab(sid)) continue; // restored separately
      const s = byId.get(sid);
      if (!s) {
        w.panes[pi] = null; // gone from disk — drop the empty slot
        continue;
      }
      state.activeWs = wi;
      w.focused = pi;
      await openSession(sid, null, { defer: !s.is_running });
    }
  }
  state.activeWs = savedActive;
  // Restore the focused pane we left off on (clamped), without focusing the
  // terminal — focusing a deferred/stashed tab would auto-resume it, and resume
  // should stay a deliberate click.
  state.workspaces.forEach((w, i) => {
    const f = savedFocused[i];
    w.focused = Number.isInteger(f) && f >= 0 && f < w.panes.length ? f : 0;
  });
  syncActiveToFocused();
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
      close.innerHTML = svgIcon("x", 11);
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
    chip.oncontextmenu = (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      // Only sessions that still exist (and aren't wild) are killable.
      const known = new Set(
        state.sessions.filter((s) => s.source !== "Wild").map((s) => s.id)
      );
      const ids = w.sessions.filter((sid) => known.has(sid));
      showContextMenu(ev.clientX, ev.clientY, [
        { label: "Rename workspace…", icon: "pencil", hint: "⌘⇧R", action: () => renameWorkspace(i) },
        ...(state.workspaces.length > 1
          ? [
              {
                label: "Close workspace",
                icon: "x",
                hint: "⌘⇧W",
                action: () => {
                  state.activeWs = i;
                  closeWorkspace();
                },
              },
            ]
          : []),
        ...(ids.length
          ? [
              null,
              {
                label: `Kill all ${ids.length} session${ids.length === 1 ? "" : "s"}…`,
                icon: "alert",
                danger: true,
                action: () =>
                  massKill(
                    ids,
                    `session${ids.length === 1 ? "" : "s"} in workspace "${w.name}"`
                  ),
              },
            ]
          : []),
      ]);
    };
    chips.appendChild(chip);
  });
}

function switchWorkspace(i) {
  if (i < 0 || i >= state.workspaces.length) return;
  state.activeWs = i;
  syncActiveToFocused();
  const sid = ws().panes[ws().focused];
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

// A session is "actively working" when a turn is in flight — its newest
// conversation id may not be persisted to disk yet, so restarting it would
// lose the exchange. Reload deliberately skips these (see `reloadSession`).
const WORKING_STATUSES = new Set(["Thinking", "Prompting", "Waiting", "Starting"]);
function isActivelyWorking(s) {
  return WORKING_STATUSES.has(s.status);
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

/// A small ✕ button for a section header that mass-kills `ids` after one
/// confirmation. `what` is the pluralized noun phrase shown in the dialog.
function sectionKillAllButton(ids, what, title) {
  const btn = document.createElement("button");
  btn.className = "icon-btn mini danger";
  btn.innerHTML = svgIcon("x", 13);
  btn.title = title;
  btn.onclick = (ev) => {
    ev.stopPropagation();
    massKill(ids, what);
  };
  return btn;
}

/// A small ⟳ button for a section header that hot-reloads every session in
/// the group (skipping any that are actively working) after one confirm.
function sectionReloadAllButton(ids, what, title) {
  const btn = document.createElement("button");
  btn.className = "icon-btn mini";
  btn.innerHTML = svgIcon("reload", 13);
  btn.title = title;
  btn.onclick = (ev) => {
    ev.stopPropagation();
    reloadAll(ids, what);
  };
  return btn;
}

function renderStatusSections(list, items) {
  const sections = { ACTIVE: [], FAILED: [], STASHED: [], DONE: [] };
  for (const s of items) sections[sectionOf(s)].push(s);
  for (const [label, group] of Object.entries(sections)) {
    if (group.length === 0) continue;
    const header = document.createElement("div");
    header.className = "section-label";
    header.innerHTML = `${label}<span class="count">${group.length}</span>`;
    // Every status section gets a kill-all on its header: one confirmation
    // clears the whole group instead of one kebab menu per row.
    const noun = label.toLowerCase();
    const ids = group.map((s) => s.id);
    const plural = `${noun} session${group.length === 1 ? "" : "s"}`;
    header.appendChild(
      sectionReloadAllButton(
        ids,
        plural,
        `Reload all ${noun} sessions on the latest Claude (skips any still working)`
      )
    );
    header.appendChild(
      sectionKillAllButton(ids, plural, `Kill all ${noun} sessions`)
    );
    list.appendChild(header);
    for (const s of group) list.appendChild(sessionItem(s));
  }
}

function renderExternalSection(list, items) {
  if (!items.length) return;
  const header = document.createElement("div");
  header.className = "section-label external";
  header.innerHTML = `⚡ EXTERNAL<span class="count">${items.length}</span>`;
  header.title = "Claude processes running outside clash — click to take over and attach";
  // Kill every associated (wild) claude process at once — each row's
  // dynamically-associated PID is signalled, same as a per-row kill.
  header.appendChild(
    sectionKillAllButton(
      items.map((s) => s.id),
      `associated claude process${items.length === 1 ? "" : "es"}`,
      "Kill all associated claude processes"
    )
  );
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
      const ids = unassigned.map((s) => s.id);
      const plural = `unassigned session${unassigned.length === 1 ? "" : "s"}`;
      header.appendChild(
        sectionReloadAllButton(
          ids,
          plural,
          "Reload all unassigned sessions on the latest Claude (skips any still working)"
        )
      );
      header.appendChild(
        sectionKillAllButton(ids, plural, "Kill all unassigned sessions")
      );
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
  // Wild claudes are owned by another process — clicking takes over
  // (one confirm: kill the outside process, resume its conversation
  // here, terminal opens). Synthetic PID-only rows (no conversation on
  // disk yet) fall back to details.
  item.onclick = () =>
    wild
      ? s.id.startsWith("wild-pid-")
        ? showDetails(s.id)
        : adoptWild(s)
      : openSession(s.id);

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
      openBrowserTab(pr, "split");
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

  // A single kebab replaces the row of hover buttons — every action
  // lives in the same menu UI as the tab/workspace context menus.
  // Right-clicking the row opens it too.
  const actions = document.createElement("div");
  actions.className = "session-actions";
  // Reload: restart this session on the latest Claude, resuming its
  // conversation. Wild (externally-owned) rows have nothing for us to
  // restart — take-over is their action instead.
  if (!wild) {
    const reload = document.createElement("button");
    reload.innerHTML = svgIcon("reload", 14);
    reload.title = "Reload — restart on the latest Claude, resuming the conversation";
    reload.onclick = (ev) => {
      ev.stopPropagation();
      reloadSessionInteractive(s);
    };
    actions.appendChild(reload);
  }
  const kebab = document.createElement("button");
  kebab.innerHTML = svgIcon("kebab", 16);
  kebab.title = "Actions";
  kebab.onclick = (ev) => {
    ev.stopPropagation();
    const r = kebab.getBoundingClientRect();
    sessionMenu(s, r.left, r.bottom + 4);
  };
  actions.appendChild(kebab);
  item.oncontextmenu = (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    sessionMenu(s, ev.clientX, ev.clientY);
  };

  item.appendChild(ring);
  item.appendChild(meta);
  item.appendChild(actions);
  return item;
}

/// Sidebar session action menu (kebab button / right-click on the row).
function sessionMenu(s, x, y) {
  const pr = state.prUrls.get(s.id);
  showContextMenu(x, y, [
    { label: "Rename session…", icon: "pencil", action: () => startRename(s.id) },
    { label: "Details", icon: "info", action: () => showDetails(s.id) },
    ...(s.source !== "Wild"
      ? [
          {
            label: "Reload (restart on latest Claude)",
            icon: "reload",
            action: () => reloadSessionInteractive(s),
          },
        ]
      : []),
    ...(pr
      ? [{ label: `Open PR #${pr.split("/").pop()}`, icon: "pr", action: () => openBrowserTab(pr, "split") }]
      : []),
    ...(s.source === "Wild" && !s.id.startsWith("wild-pid-")
      ? [{ label: "Take over wild claude", icon: "zap", action: () => adoptWild(s) }]
      : []),
    null,
    ...(s.is_running
      ? [
          {
            label: "Stash (stop, keep resumable)",
            icon: "pause",
            action: async () => {
              await invoke("stash_session", { sessionId: s.id }).catch(console.error);
              dropTerminal(s.id);
              refreshSessions();
            },
          },
        ]
      : []),
    {
      label: "Kill session…",
      icon: "alert",
      danger: true,
      action: async () => {
        if (!(await uiConfirm(`Kill session "${displayName(s)}"?`, "Kill"))) return;
        await invoke("kill_session", { sessionId: s.id }).catch(console.error);
        dropTerminal(s.id);
        refreshSessions();
      },
    },
  ]);
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
        // Shell terminals and browser tabs are never in the session list
        if (isShellTerm(id) || isBrowserTab(id)) continue;
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

    // Keep open-terminal labels (tabs, pane titles) in sync with the
    // authoritative names from the backend, so a rename made anywhere
    // (sidebar, tab menu, TUI) propagates to every view.
    let labelsChanged = false;
    for (const [id, entry] of state.open) {
      const s = sessions.find((x) => x.id === id);
      if (s && entry.term) {
        const label = displayName(s);
        if (entry.name !== label) {
          entry.name = label;
          labelsChanged = true;
        }
      }
    }
    if (labelsChanged) renderPanes();

    // While an inline rename is in progress, rebuilding the sidebar would
    // destroy the input mid-typing (value reset, focus stolen) — skip it;
    // the next tick after Enter/Escape repaints with fresh data.
    if (!state.renaming) renderSidebar();
    renderTabs();
    if (state.detailsFor) renderDetails();

    // Teams change on disk when Claude spawns/retires agents, and members go
    // live/idle as sessions come and go — keep the open section AND the open
    // detail panel live without an explicit refresh.
    if (state.teamsOpen || state.openTeamPanel) {
      invoke("list_teams")
        .then((teams) => {
          const changed = JSON.stringify(teams) !== JSON.stringify(state.teams);
          state.teams = teams;
          // Sidebar rollup depends on running sessions (refreshed each tick),
          // so re-render even when the on-disk config is unchanged.
          if (state.teamsOpen) renderTeams();
          // Re-render the open panel when the team config or the running-member
          // set changed — but never while a context menu is open (it would
          // vanish under the rebuild).
          if (state.openTeamPanel && !$("context-menu")) {
            const t = teams.find((x) => x.name === state.openTeamPanel);
            if (t) {
              const sig = teamRunSignature(t);
              if (changed || sig !== state._teamRunSig) {
                state._teamRunSig = sig;
                showTeamDetails(t);
              }
            }
          }
        })
        .catch(() => {});
    }
  } catch (e) {
    console.error("list_sessions failed:", e);
  }
}

/// Kill a batch of sessions after a single confirmation — used by the
/// workspace-chip context menu and the UNASSIGNED header. `what` is the
/// already-pluralized noun phrase shown in the confirm dialog.
async function massKill(ids, what) {
  if (!ids.length) return;
  if (
    !(await uiConfirm(
      `Kill ${ids.length} ${what}? This removes them from clash.`,
      "Kill all"
    ))
  )
    return;
  const results = await Promise.allSettled(
    ids.map((sid) => invoke("kill_session", { sessionId: sid }))
  );
  for (const sid of ids) dropTerminal(sid);
  // Drop workspace ownership now instead of waiting for the 3-refresh prune.
  for (const w of state.workspaces) {
    w.sessions = w.sessions.filter((sid) => !ids.includes(sid));
  }
  saveWorkspaces();
  const failed = results.filter((r) => r.status === "rejected").length;
  if (failed) uiAlert(`${failed} of ${ids.length} kills failed.`);
  refreshSessions();
}

/// Hot-reload one session: stop its current process (kept resumable, waiting
/// for it to actually exit) and reopen it resuming its latest conversation id —
/// so it comes back on the newest `claude` binary without losing the
/// conversation. The backend `reload_session` does the stop-and-wait; then
/// `open_session` resolves the lineage forward (`resolve_resume_id`) and starts
/// fresh when no transcript survives, so "reopen on the latest id" is free.
/// Reopens in place for an open tab; opens (resumes) a currently-closed one.
async function reloadSession(sid) {
  const wasOpen = state.open.has(sid);
  const entry = wasOpen ? state.open.get(sid) : null;
  if (entry && entry.term) {
    entry.term.writeln("\r\n\x1b[90m⟳ reloading on the latest Claude…\x1b[0m");
  }
  try {
    await invoke("reload_session", { sessionId: sid });
  } catch (e) {
    console.error("reload failed", e);
    if (entry && entry.term) entry.term.writeln(`\x1b[31mReload failed: ${e}\x1b[0m`);
    return;
  }
  if (wasOpen) dropTerminal(sid);
  openSession(sid);
}

/// Reload one session with the "actively working" confirm guard — shared by
/// the sidebar row button, the tab button, the context menus, and the ⌘R
/// shortcut. No-ops on wild rows (take-over is their action) and returns
/// without reloading if the user cancels the confirm.
async function reloadSessionInteractive(s) {
  if (!s || s.source === "Wild") return;
  if (
    isActivelyWorking(s) &&
    !(await uiConfirm(
      `"${displayName(s)}" is working right now. Reload anyway? The in-flight turn may be lost.`,
      "Reload"
    ))
  )
    return;
  reloadSession(s.id);
}

/// Reload every non-actively-working session in `ids` after one confirm.
/// Actively-working sessions (a turn in flight) are skipped, per design.
/// `what` is the pluralized noun phrase for the dialog.
async function reloadAll(ids, what) {
  const sessions = ids
    .map((id) => state.sessions.find((s) => s.id === id))
    .filter(Boolean);
  const todo = sessions.filter((s) => !isActivelyWorking(s));
  const skipped = sessions.length - todo.length;
  if (!todo.length) {
    uiAlert(
      skipped
        ? `All ${skipped} ${what} are working right now — nothing reloaded.`
        : `No ${what} to reload.`
    );
    return;
  }
  const skipNote = skipped
    ? ` ${skipped} working ${skipped === 1 ? "session is" : "sessions are"} left alone.`
    : "";
  if (
    !(await uiConfirm(
      `Reload ${todo.length} ${what}? Each restarts on the latest Claude, ` +
        `resuming its conversation.${skipNote}`,
      "Reload all"
    ))
  )
    return;
  // Sequential so we don't stampede the daemon with concurrent spawns.
  for (const s of todo) await reloadSession(s.id);
  refreshSessions();
}

// ── Context menu ────────────────────────────────────────────────

function hideContextMenu() {
  const menu = $("context-menu");
  if (menu) {
    menu.remove();
    // Restore browser webviews hidden while the menu was up (they are
    // native views that would otherwise paint over the menu).
    fitAll();
  }
}

/// items: [{ label, action, danger? }] — null entries become separators.
function showContextMenu(x, y, items) {
  hideContextMenu();
  const menu = document.createElement("div");
  menu.id = "context-menu";
  // The icon column only exists when at least one item carries an icon,
  // so icon-less menus don't render an empty gutter.
  const hasIcons = items.some((it) => it && it.icon);
  for (const it of items) {
    if (!it) {
      const sep = document.createElement("div");
      sep.className = "ctx-sep";
      menu.appendChild(sep);
      continue;
    }
    const row = document.createElement("div");
    row.className = "ctx-item" + (it.danger ? " danger" : "");
    if (hasIcons) {
      const icon = document.createElement("span");
      icon.className = "ctx-icon";
      if (it.icon) icon.innerHTML = svgIcon(it.icon, 14);
      row.appendChild(icon);
    }
    const label = document.createElement("span");
    label.className = "ctx-label";
    label.textContent = it.label;
    row.appendChild(label);
    if (it.hint) {
      const hint = document.createElement("span");
      hint.className = "ctx-hint";
      hint.textContent = it.hint;
      row.appendChild(hint);
    }
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
  // Native browser webviews paint over all DOM — drop them while the
  // menu is open so it stays visible; hideContextMenu restores them.
  hideBrowserWebviews();
}

document.addEventListener("click", hideContextMenu);
window.addEventListener("blur", hideContextMenu);

/// Brief, non-blocking confirmation toast (bottom-center, auto-dismiss).
let _toastTimer = null;
function flashToast(msg) {
  let el = $("gui-toast");
  if (!el) {
    el = document.createElement("div");
    el.id = "gui-toast";
    document.body.appendChild(el);
  }
  el.textContent = msg;
  el.classList.add("show");
  if (_toastTimer) clearTimeout(_toastTimer);
  _toastTimer = setTimeout(() => el.classList.remove("show"), 1600);
}

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

/// Rename any tab. Claude sessions go through the registry (rename_session,
/// kept in sync with the TUI); shell/view/browser tabs are display-only —
/// shellterms die with the app, browser names persist via gui-state.
async function renameTabDialog(id) {
  const entry = state.open.get(id);
  if (!entry) return;
  if (entry.kind === "claude") return renameSessionDialog(id);
  const name = await uiPrompt("Tab name:", entry.name || "");
  if (!name || !name.trim()) return;
  entry.name = name.trim();
  if (entry.kind === "browser") {
    entry.renamed = true;
    saveWorkspaces();
  }
  renderTabs();
  renderPanes();
}

function tabContextMenu(ev, sid) {
  ev.preventDefault();
  ev.stopPropagation();
  const entry = state.open.get(sid);
  if (isShellTerm(sid)) {
    showContextMenu(ev.clientX, ev.clientY, [
      { label: "Rename terminal…", icon: "pencil", action: () => renameTabDialog(sid) },
      { label: "Close terminal", icon: "x", action: () => detachSession(sid) },
    ]);
    return;
  }
  if (entry && entry.kind === "browser") {
    showContextMenu(ev.clientX, ev.clientY, [
      { label: "Rename tab…", icon: "pencil", action: () => renameTabDialog(sid) },
      { label: "Copy URL", icon: "copy", action: () => navigator.clipboard?.writeText(entry.url).catch(() => {}) },
      {
        label: "Open in system browser",
        icon: "external-link",
        action: () => invoke("open_external", { url: entry.url }).catch(console.error),
      },
      null,
      { label: "Zoom in", icon: "plus", hint: "⌘+", action: () => browserZoom(entry, 0.1) },
      { label: "Zoom out", icon: "minus", hint: "⌘-", action: () => browserZoom(entry, -0.1) },
      { label: "Reset zoom", icon: "square", hint: "⌘0", action: () => browserZoom(entry, 0) },
      null,
      { label: "Open DevTools", icon: "terminal", action: () => invoke("browser_devtools", { tab: entry.tabId }).catch(() => {}) },
      null,
      { label: "Close tab", icon: "x", hint: "⌘W", action: () => detachSession(sid) },
    ]);
    return;
  }
  if (entry && !entry.term) {
    // Content tab (conversation/subagents/diff) — renamable + closable
    showContextMenu(ev.clientX, ev.clientY, [
      { label: "Rename tab…", icon: "pencil", action: () => renameTabDialog(sid) },
      { label: "Close tab", icon: "x", action: () => dropTerminal(sid) },
    ]);
    return;
  }
  const pr = state.prUrls.get(sid);
  showContextMenu(ev.clientX, ev.clientY, [
    { label: "Rename session…", icon: "pencil", action: () => renameSessionDialog(sid) },
    {
      label: "Reload (restart on latest Claude)",
      icon: "reload",
      action: () => reloadSession(sid),
    },
    { label: "Close tab (stash)", icon: "x", hint: "⌘W", action: () => closeTab(sid) },
    {
      label: "Detach (keep running)",
      icon: "external-link",
      action: () => detachSession(sid),
    },
    ...(pr
      ? [{ label: `Open PR #${pr.split("/").pop()}`, icon: "pr", action: () => openBrowserTab(pr, "split") }]
      : []),
    null,
    {
      label: "Stash (stop, keep resumable)",
      icon: "pause",
      action: async () => {
        await invoke("stash_session", { sessionId: sid }).catch(console.error);
        dropTerminal(sid);
        refreshSessions();
      },
    },
    {
      label: "Kill session…",
      icon: "alert",
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
    { label: "Details", icon: "info", action: () => showDetails(sid) },
  ]);
}

// ── Tabs ────────────────────────────────────────────────────────

/// Utility shell terminals (GUI "new terminal") — daemon PTYs in the
/// shellterm- namespace; tabs/panes only, never Claude sessions.
function isShellTerm(id) {
  return id.startsWith("shellterm-");
}

/// Session id behind a tab entry — view tabs (`view:conv:<sid>` …) belong
/// to the session in their key's last segment.
function tabSession(id) {
  return id.startsWith("view:") ? id.slice(id.lastIndexOf(":") + 1) : id;
}

function renderTabs() {
  const tabs = $("tabs");
  tabs.innerHTML = "";
  for (const [id, entry] of state.open) {
    // The tab strip is scoped to the active workspace (like the sidebar):
    // tabs owned by another workspace stay hidden until you switch back.
    // Unassigned sessions remain visible so they're always reachable.
    const owner = sessionWorkspace(tabSession(id));
    if (owner !== -1 && owner !== state.activeWs) continue;
    const tab = document.createElement("div");
    tab.className = "tab" + (id === state.activeTab ? " active" : "");
    tab.onclick = () => assignToFocusedPane(id);
    tab.oncontextmenu = (ev) => tabContextMenu(ev, id);
    tab.onauxclick = (ev) => {
      // Middle-click closes the tab (Claude → stash), like a browser.
      if (ev.button === 1) {
        ev.preventDefault();
        closeTab(id);
      }
    };

    const s = state.sessions.find((x) => x.id === id);
    if (s) {
      const dot = document.createElement("span");
      dot.className = `tab-dot ${statusClass(s)}`;
      dot.title = statusInfo(s).label;
      tab.appendChild(dot);
    }

    const label = document.createElement("span");
    label.textContent = entry.name;
    label.title = "Double-click to rename";
    label.ondblclick = (ev) => {
      ev.stopPropagation();
      renameTabDialog(id);
    };

    tab.appendChild(label);

    // Reload (Claude tabs only): restart on the latest Claude, resuming the
    // conversation. Shells/browsers/views have nothing to resume.
    if (entry.kind === "claude") {
      const reload = document.createElement("span");
      reload.className = "reload";
      reload.innerHTML = svgIcon("reload", 12);
      reload.title = "Reload — restart on the latest Claude, resuming the conversation";
      reload.onclick = (ev) => {
        ev.stopPropagation();
        // `s` is the session for this claude tab; if it's briefly missing
        // from the list (just removed) reload by id directly.
        if (s) reloadSessionInteractive(s);
        else reloadSession(id);
      };
      tab.appendChild(reload);
    }

    const close = document.createElement("span");
    close.className = "close";
    close.innerHTML = svgIcon("x", 13);
    close.title =
      entry.kind === "claude"
        ? "Close tab (stash — keeps resumable)"
        : "Close tab";
    close.onclick = (ev) => {
      ev.stopPropagation();
      closeTab(id);
    };

    tab.appendChild(close);
    tabs.appendChild(tab);
  }

  // "+" ghost tab — the same unified menu as the topbar button, where
  // the eye already is when looking at tabs.
  const plus = document.createElement("div");
  plus.className = "tab new-tab";
  plus.title = "New tab — terminal, browser, or Claude session";
  plus.innerHTML = svgIcon("plus", 13);
  plus.onclick = (ev) => {
    ev.stopPropagation();
    const r = plus.getBoundingClientRect();
    showNewTabMenu(r.left, r.bottom + 4);
  };
  tabs.appendChild(plus);
}

// ── Panes (split layout) ────────────────────────────────────────

function renderPanes() {
  const host = $("terminal-host");
  const w = ws();
  const visible = w.zoomed ? [w.panes[w.focused] ?? null] : w.panes;
  // Balanced grid for any pane count (no fixed cap): columns grow first,
  // rows follow — 2 → 2x1, 3-4 → 2x2, 5-6 → 3x2, 7-9 → 3x3, …
  const cols = Math.ceil(Math.sqrt(visible.length));
  const rows = Math.ceil(visible.length / cols);
  // Resizable grid tracks: per-workspace column/row fractions, reset to equal
  // whenever the grid shape changes (pane added/removed) or a single cell is
  // shown (zoom / one pane). Draggable gutters between tracks edit these.
  const resizable = !w.zoomed && visible.length > 1;
  if (resizable) {
    const valid = (a, n) =>
      Array.isArray(a) && a.length === n && a.every((f) => typeof f === "number" && f > 0);
    if (!valid(w.colFracs, cols)) w.colFracs = Array(cols).fill(1);
    if (!valid(w.rowFracs, rows)) w.rowFracs = Array(rows).fill(1);
    host.style.gridTemplateColumns = w.colFracs.map((f) => f + "fr").join(" ");
    host.style.gridTemplateRows = w.rowFracs.map((f) => f + "fr").join(" ");
  } else {
    host.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;
    host.style.gridTemplateRows = `repeat(${rows}, 1fr)`;
  }

  // Detach term elements first so re-appending doesn't destroy them
  for (const entry of state.open.values()) entry.el.remove();
  host.querySelectorAll(".pane, .pane-gutter").forEach((p) => p.remove());

  const anyAssigned = w.panes.some((p) => p);
  // The centered #empty-state welcome overlay spans the whole host, so it only
  // makes sense when there's a single, unfilled pane — otherwise it would paint
  // over (and clutter) the empty-pane placeholders. In every other empty case
  // the per-pane placeholder is the surface instead.
  const soleEmpty = !anyAssigned && visible.length === 1;
  $("empty-state").style.display = soleEmpty ? "flex" : "none";

  visible.forEach((sid, vi) => {
    const i = w.zoomed ? w.focused : vi;
    const pane = document.createElement("div");
    pane.className = "pane" + (i === w.focused ? " focused" : "");
    pane.onclick = () => {
      w.focused = i;
      syncActiveToFocused();
      if (w.panes[i]) focusTerm(w.panes[i]);
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
        title.title = "Double-click to zoom (⌘⇧↩)";
        title.ondblclick = toggleZoom;
        pane.appendChild(title);
      }
      pane.appendChild(entry.el);
    } else if (soleEmpty) {
      // Fresh workspace: let the #empty-state welcome overlay (spanning the
      // whole host, painted beneath the panes) be the visible, interactive
      // quick-start surface. Make this lone empty pane click-through so its
      // clicks/right-clicks reach the overlay underneath.
      pane.style.pointerEvents = "none";
    } else {
      const empty = document.createElement("div");
      empty.className = "pane-empty";
      empty.textContent = "click to focus · right-click to start";
      // Quick-start: right-clicking an empty pane opens the unified new-tab
      // menu (terminal / browser / Claude session) and whatever you pick lands
      // right here. The menu's actions target the focused pane (via
      // assignToFocusedPane), so focus this pane first. Left-click keeps its
      // existing meaning (focus the pane, then assign from the sidebar/tabs).
      empty.oncontextmenu = (ev) => {
        ev.preventDefault();
        ev.stopPropagation();
        const w2 = ws();
        w2.focused = i;
        syncActiveToFocused();
        showNewTabMenu(ev.clientX, ev.clientY);
      };
      pane.appendChild(empty);
    }
    host.appendChild(pane);
  });

  if (resizable) addPaneGutters(host, w, cols, rows);

  fitAll();
}

/// Add draggable gutters between the grid's column and row tracks. Positioned
/// (and repositioned on resize) by `repositionGutters` from the fractions, so
/// no pane layout needs to be read.
function addPaneGutters(host, w, cols, rows) {
  for (let k = 1; k < cols; k++) {
    const g = document.createElement("div");
    g.className = "pane-gutter col";
    g.title = "Drag to resize columns";
    makeGutterDraggable(g, host, w, "col", k);
    host.appendChild(g);
  }
  for (let j = 1; j < rows; j++) {
    const g = document.createElement("div");
    g.className = "pane-gutter row";
    g.title = "Drag to resize rows";
    makeGutterDraggable(g, host, w, "row", j);
    host.appendChild(g);
  }
  // Place immediately (host is already laid out) to avoid a one-frame flash at
  // the origin; fitAll's rAF repositions again once terms reflow.
  repositionGutters(host, w);
}

/// Place each gutter at its track boundary, computed from the fractions (the
/// column/row gap is 1px — negligible, so we ignore it). Gutters are appended
/// in track order, matching the cumulative-fraction walk.
function repositionGutters(host, w) {
  const colG = host.querySelectorAll(".pane-gutter.col");
  const rowG = host.querySelectorAll(".pane-gutter.row");
  if (!colG.length && !rowG.length) return;
  const cs = w.colFracs || [];
  const rs = w.rowFracs || [];
  const ctot = cs.reduce((a, b) => a + b, 0) || 1;
  const rtot = rs.reduce((a, b) => a + b, 0) || 1;
  const width = host.clientWidth;
  const height = host.clientHeight;
  let acc = 0;
  colG.forEach((g, i) => {
    acc += cs[i] || 0;
    g.style.left = (acc / ctot) * width + "px";
  });
  let accr = 0;
  rowG.forEach((g, i) => {
    accr += rs[i] || 0;
    g.style.top = (accr / rtot) * height + "px";
  });
}

/// Wire a gutter to redistribute the fraction between the two tracks it sits
/// between. `k` is the higher track index (boundary between k-1 and k).
function makeGutterDraggable(g, host, w, axis, k) {
  g.addEventListener("mousedown", (e) => {
    e.preventDefault();
    e.stopPropagation();
    g.classList.add("dragging");
    document.body.style.cursor = axis === "col" ? "col-resize" : "row-resize";
    const fracs = axis === "col" ? w.colFracs : w.rowFracs;
    const start = [...fracs];
    const total = start.reduce((a, b) => a + b, 0);
    const size = axis === "col" ? host.clientWidth : host.clientHeight;
    const startPos = axis === "col" ? e.clientX : e.clientY;
    const MIN = 0.15; // keep every track at least ~15% of an equal share
    const onMove = (ev) => {
      const pos = axis === "col" ? ev.clientX : ev.clientY;
      let d = ((pos - startPos) / size) * total;
      // Clamp so neither adjacent track shrinks below MIN.
      d = Math.max(-(start[k - 1] - MIN), Math.min(start[k] - MIN, d));
      fracs[k - 1] = start[k - 1] + d;
      fracs[k] = start[k] - d;
      const tpl = fracs.map((f) => f + "fr").join(" ");
      if (axis === "col") host.style.gridTemplateColumns = tpl;
      else host.style.gridTemplateRows = tpl;
      fitAll();
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      g.classList.remove("dragging");
      document.body.style.cursor = "";
      saveWorkspaces();
      fitAll();
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  });
}

function fitAll() {
  requestAnimationFrame(() => {
    for (const sid of ws().panes) {
      const entry = sid && state.open.get(sid);
      if (entry && entry.fitAddon) entry.fitAddon.fit();
    }
    if (typeof syncBrowserWebviews === "function") syncBrowserWebviews();
    repositionGutters($("terminal-host"), ws());
  });
}

function focusTerm(sid) {
  const entry = state.open.get(sid);
  if (!entry) return;
  if (entry.deferred) {
    resumeDeferred(sid);
    return;
  }
  if (entry.term) setTimeout(() => entry.term.focus(), 0);
}

/// Invariant enforced across every pane mutation: the active tab IS the
/// content of the focused pane (null when the focused pane is empty).
/// Tabs and panes stop drifting apart — clicking a tab fills the focused
/// pane, focusing a pane activates its tab.
function syncActiveToFocused() {
  const w = ws();
  state.activeTab = w.panes[w.focused] || null;
}

function addPane() {
  const w = ws();
  w.panes.push(null);
  w.focused = w.panes.length - 1;
  w.zoomed = false;
  syncActiveToFocused();
  saveWorkspaces();
  renderPanes();
  renderTabs();
}

/// Close the FOCUSED pane (not the last one). Its content survives as a
/// tab — closing a split never loses a session.
function removePane() {
  const w = ws();
  if (w.panes.length <= 1) return;
  w.panes.splice(w.focused, 1);
  w.focused = Math.min(w.focused, w.panes.length - 1);
  if (w.panes.length === 1) w.zoomed = false;
  syncActiveToFocused();
  saveWorkspaces();
  renderPanes();
  renderTabs();
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
  syncActiveToFocused();
  const sid = w.panes[w.focused];
  if (sid) focusTerm(sid);
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

/// Take over a wild claude in one step: confirm, kill the outside
/// process, resume its (dynamically associated, latest) conversation
/// under our daemon, then open the terminal — same flow as the TUI's
/// `a` on a wild row.
async function adoptWild(s) {
  if (
    !(await uiConfirm(
      `Take over "${displayName(s)}"? The outside claude (PID ${s.wild_pid}) is killed and its conversation resumes here.`,
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
    uiAlert(`Take over failed: ${e}`);
    refreshSessions();
    return;
  }
  refreshSessions();
  openSession(s.id);
}

async function openSession(sid, label, opts = {}) {
  // Sessions are workspace-scoped: owned elsewhere → switch there first;
  // unowned → the active workspace claims it.
  const owner = sessionWorkspace(sid);
  if (owner >= 0 && owner !== state.activeWs) switchWorkspace(owner);
  claimSession(sid);

  // `defer`: restore a stashed session as a placeholder tab (no process)
  // that resumes on first focus — see resumeDeferred / focusTerm.
  const defer = !!opts.defer;

  if (state.open.has(sid)) {
    assignToFocusedPane(sid); // focusing a deferred tab resumes it
    return;
  }

  const el = document.createElement("div");
  el.className = "term-wrap";

  const term = new Terminal({
    fontFamily: state.settings.fontFamily,
    fontSize: state.settings.fontSize,
    theme: TERM_THEME,
    scrollback: state.settings.scrollback,
    cursorStyle: state.settings.cursorStyle,
    cursorBlink: state.settings.cursorBlink,
    macOptionIsMeta: state.settings.optionMeta,
    // Claude Code turns on mouse tracking, so plain mouse drags are reported to
    // it as mouse events and never produce a text selection — making ⌘C / copy
    // impossible (and any stray partial selection copies garbled text). Match
    // the native macOS terminal convention (iTerm2 / Terminal.app): hold ⌥ while
    // dragging to force a real text selection that ⌘C and copy-on-select grab.
    // (On non-mac, xterm already lets Shift+drag force selection.) This is
    // mouse-only and independent of macOptionIsMeta, so it never affects typing
    // ⌥-composed glyphs (brackets/braces on AZERTY, etc.).
    macOptionClickForcesSelection: true,
    // Right-click selects the word under the pointer (parity with double-click),
    // a quick native affordance for grabbing a token to copy.
    rightClickSelectsWord: true,
    // OSC 8 hyperlinks (Claude Code emits these) — routed through openLink,
    // which asks / embeds / opens externally per the "Open links" setting.
    linkHandler: {
      activate: (_e, uri) => openLink(uri),
    },
  });
  // International layouts (e.g. AZERTY) type brackets/braces with Option
  // (⌥( = {, ⌥⇧( = [ …). macOptionIsMeta would turn those into ESC
  // sequences, making the characters impossible to type. Bypassing xterm
  // isn't enough either: WKWebView fires no keypress for Option combos,
  // and xterm's input-event fallback drops any insertText preceded by a
  // keydown (`!e.composed || !this._keyDownSeen`), so the glyph would be
  // silently swallowed. Send the composed character to the PTY directly.
  // With optionMeta on, Alt+letter stays Meta for readline word jumps
  // (⌥B/⌥F); with it off, ⌥ always composes — letters included.
  term.attachCustomKeyEventHandler((e) => {
    // Copy / paste. WKWebView has no native edit menu and xterm's canvas
    // selection isn't a DOM selection, so ⌘C/⌘V (macOS) and Ctrl+Shift+C/V
    // (Linux) never reach the clipboard on their own — handle them here via
    // the backend clipboard. Plain Ctrl+C (no Shift/Meta) is deliberately
    // left for xterm to forward to the PTY as SIGINT.
    const clipMod = e.metaKey || (e.ctrlKey && e.shiftKey);
    if (e.type === "keydown" && clipMod && (e.key === "c" || e.key === "C")) {
      // Only intercept when there's a selection to copy; otherwise let the
      // keystroke through (e.g. bare ⌘C with no selection is a no-op).
      if (term.hasSelection()) {
        const sel = term.getSelection();
        if (sel) invoke("clipboard_write_text", { text: sel }).catch(console.error);
        e.preventDefault();
        return false;
      }
    }
    if (e.type === "keydown" && clipMod && (e.key === "v" || e.key === "V")) {
      // term.paste() respects bracketed-paste mode, so multi-line pastes
      // don't auto-execute in the shell / Claude's input.
      invoke("clipboard_read_text")
        .then((text) => {
          if (text) term.paste(text);
        })
        .catch(console.error);
      e.preventDefault();
      return false;
    }
    // Shift+Enter inserts a newline in Claude sessions instead of
    // submitting. xterm encodes Enter and Shift+Enter identically (\r);
    // Claude Code treats ESC+CR as "insert newline" (the same sequence
    // its /terminal-setup binds in iTerm/VS Code). Claude sessions only:
    // in shells ESC+CR is readline M-RET and would surprise.
    if (
      e.type === "keydown" &&
      e.key === "Enter" &&
      e.shiftKey &&
      !e.metaKey &&
      !e.ctrlKey &&
      !e.altKey &&
      !isShellTerm(sid)
    ) {
      invoke("send_input", { sessionId: sid, text: "\x1b\r" }).catch(console.error);
      e.preventDefault();
      return false;
    }
    if (
      e.type === "keydown" &&
      e.altKey &&
      !e.metaKey &&
      !e.ctrlKey &&
      e.key.length === 1 &&
      (!/[a-zA-Z]/.test(e.key) || !state.settings.optionMeta)
    ) {
      invoke("send_input", { sessionId: sid, text: e.key }).catch(console.error);
      e.preventDefault();
      return false;
    }
    return true;
  });

  const fitAddon = new FitAddon.FitAddon();
  term.loadAddon(fitAddon);

  const s = state.sessions.find((x) => x.id === sid);
  state.open.set(sid, {
    kind: isShellTerm(sid) ? "shell" : "claude",
    term,
    fitAddon,
    el,
    name: label || (s ? displayName(s) : sid.slice(0, 8)),
    deferred: defer,
  });

  // Deferred restores keep their saved pane slot and must not steal focus
  // (focusing would resume them); live opens claim the focused pane.
  if (!defer) assignToFocusedPane(sid);
  term.open(el);
  // GPU-accelerated rendering. The default DOM renderer repaints cells as
  // styled <span>s and, under Claude Code's rapid streaming output (spinners,
  // progressive tokens), leaves stale/half-refreshed glyphs — the "not native,
  // badly refreshed text" symptom. The WebGL renderer draws the whole grid to
  // one GPU-backed canvas each frame, so it stays crisp and consistent. If the
  // WebGL context is lost (GPU pressure, tab backgrounded in WKWebView) the
  // addon emits onContextLoss; we dispose it and xterm falls back to the DOM
  // renderer automatically. Loading is best-effort: any failure keeps the DOM
  // renderer rather than leaving a blank terminal.
  try {
    if (window.WebglAddon) {
      const webgl = new WebglAddon.WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      term.loadAddon(webgl);
    }
  } catch (e) {
    console.warn("WebGL renderer unavailable, using DOM renderer:", e);
  }
  fitAddon.fit();

  if (defer) {
    term.writeln("\x1b[90m○ stashed — click to resume\x1b[0m");
  } else {
    try {
      await invoke("open_session", {
        sessionId: sid,
        cols: term.cols,
        rows: term.rows,
      });
    } catch (e) {
      term.writeln(`\x1b[31mFailed to open session: ${e}\x1b[0m`);
    }
  }

  term.onData((data) => {
    // First keystroke on a stashed tab resumes it instead of being lost.
    const en = state.open.get(sid);
    if (en && en.deferred) {
      resumeDeferred(sid);
      return;
    }
    invoke("send_input", { sessionId: sid, text: data }).catch(console.error);
  });
  term.onResize(({ cols, rows }) => {
    invoke("resize_session", { sessionId: sid, cols, rows }).catch(() => {});
  });
  // Copy-on-select (off by default). The bare WKWebView's navigator.clipboard
  // is unreliable (no secure-context/edit-menu plumbing), so route through the
  // backend clipboard plugin — same path as ⌘C — instead of navigator.clipboard,
  // which silently dropped the copy. Failures are ignored (⌘C still works).
  term.onSelectionChange(() => {
    if (!state.settings.copyOnSelect || !term.hasSelection()) return;
    const text = term.getSelection();
    if (text) invoke("clipboard_write_text", { text }).catch(() => {});
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
          activate: (_e, uri) => openLink(uri),
        });
      }
      cb(links.length ? links : undefined);
    },
  });

  if (!defer) focusTerm(sid);
}

/// Resume a deferred (restored-stashed) tab: spawn `claude --resume` and let
/// the daemon replay history + stream output into the existing terminal.
async function resumeDeferred(sid) {
  const entry = state.open.get(sid);
  if (!entry || !entry.deferred) return;
  entry.deferred = false;
  entry.term.clear();
  entry.fitAddon?.fit();
  try {
    await invoke("open_session", {
      sessionId: sid,
      cols: entry.term.cols,
      rows: entry.term.rows,
    });
  } catch (e) {
    entry.term.writeln(`\x1b[31mFailed to resume: ${e}\x1b[0m`);
  }
  refreshSessions();
}

/// Close a tab from the top strip. A Claude session is STASHED (process
/// stopped, conversation kept resumable) so that closing its tab and
/// stashing from the sidebar are the same action regardless of origin —
/// they stay linked. Shells are killed (nothing to resume), browser and
/// content tabs just close. For the "leave it running in the background"
/// case, use Detach from the tab context menu.
async function closeTab(sid) {
  const entry = state.open.get(sid);
  if (entry && entry.kind === "claude") {
    // A deferred (not-yet-resumed) tab has no live process — it's already
    // stashed on disk, so just drop the placeholder.
    if (!entry.deferred) {
      await invoke("stash_session", { sessionId: sid }).catch(console.error);
    }
    dropTerminal(sid);
    refreshSessions();
    return;
  }
  await detachSession(sid);
}

/// Detach (keep session running in the backend). Shell terminals are
/// killed instead — a detached shell has nothing to resume. View tabs
/// just close.
async function detachSession(sid) {
  const entry = state.open.get(sid);
  if (entry && entry.kind === "browser") {
    // Closing a browser tab destroys its webview — nothing to keep alive.
    if (entry.created) await invoke("browser_close_tab", { tab: entry.tabId }).catch(() => {});
  } else if (entry && entry.term) {
    try {
      if (isShellTerm(sid)) await invoke("close_terminal", { sessionId: sid });
      else await invoke("close_session", { sessionId: sid });
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
    // Shell terminals and browser tabs leave ownership on close — the
    // session prune intentionally skips them, so nothing else would.
    if (isShellTerm(sid) || isBrowserTab(sid)) {
      w.sessions = w.sessions.filter((x) => x !== sid);
    }
  }
  saveWorkspaces();
  if (state.activeTab === sid) syncActiveToFocused();
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
  state.open.set(key, { kind: "view", el, name });
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
    el.innerHTML = `<h4>SUBAGENTS (${subs.length})</h4>`;
    if (!subs.length) {
      el.innerHTML += "<p class='hint'>no subagents — they appear when this session spawns Task agents</p>";
      return;
    }
    for (const sub of subs) {
      const row = document.createElement("div");
      row.className = "row-item";
      row.innerHTML = `<span class="team-icon">${svgIcon("zap", 12)}</span><span>${escapeHtml(
        sub.agent_type || sub.id
      )}</span><span class="dim">${escapeHtml(sub.summary || "")}</span>`;
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
  $("details-btn").classList.add("on");
  renderDetails();
  fitAll();
}

function hideDetails() {
  state.detailsFor = null;
  state.openTeamPanel = null;
  detailsShellFor = null;
  $("details").classList.add("hidden");
  $("details-resizer").classList.add("hidden");
  $("details-btn").classList.remove("on");
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
      <button id="d-browser">Open in browser</button>
    </div>
    <div id="d-out"></div>
    <div class="kv dim-id" title="${escapeHtml(s.id)}"><span class="k">ID</span><span class="v">${escapeHtml(s.id)}</span></div>
  `;
  $("d-browser").onclick = () => showBrowserOpenPicker(s);
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
        row.onclick = () => openBrowserTab(`http://localhost:${row.dataset.port}`, "split");
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

/// "Open in browser" tool — pick what to show in the embedded browser
/// panel: the diff on GitHub, the session's PR, or the repository on
/// its forge. (The local diff lives in an in-app tab, not here.)
async function showBrowserOpenPicker(s) {
  const out = $("d-out");
  out.innerHTML = "<h4>OPEN IN BROWSER</h4>";
  const addRow = (label, desc, onclick) => {
    const row = document.createElement("div");
    row.className = "row-item";
    row.innerHTML = `<span>${escapeHtml(label)}</span><span class="dim">${escapeHtml(desc)}</span>`;
    row.onclick = onclick;
    out.appendChild(row);
  };
  const pr = state.prUrls.get(s.id);
  let repo = null;
  try {
    repo = await invoke("get_repo_url", { sessionId: s.id });
  } catch {
    /* no origin remote — skip the forge rows */
  }
  // GitHub diff first: the PR's files view, else a compare view of the
  // session branch against the default branch (pushed commits only).
  if (pr) {
    addRow("± Diff on GitHub", `PR #${pr.split("/").pop()} files`, () =>
      openBrowserTab(`${pr}/files`, "split"),
    );
  } else if (repo && repo.includes("github.com") && s.git_branch) {
    const branch = s.git_branch;
    addRow("± Diff on GitHub", `compare …${branch}`, async () => {
      const base = await invoke("get_default_branch", { sessionId: s.id }).catch(() => "main");
      if (base === branch) {
        uiAlert(`Branch ${branch} is the default branch — nothing to compare on GitHub.`);
        return;
      }
      openBrowserTab(
        `${repo}/compare/${encodeURIComponent(base)}...${encodeURIComponent(branch)}`,
        "split",
      );
    });
  }
  if (pr) addRow(`⇄ Pull request #${pr.split("/").pop()}`, pr, () => openBrowserTab(pr, "split"));
  if (repo) {
    const url =
      s.git_branch && repo.includes("github.com")
        ? `${repo}/tree/${encodeURIComponent(s.git_branch)}`
        : repo;
    addRow("⌂ Repository", url, () => openBrowserTab(url, "split"));
  }
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
  applySectionHeight("teams-section", "teams-resizer", state.teamsOpen, "teamsHeight");
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

/// A member is "running" when a live session shares its working directory
/// (same cwd match the core uses for `is_active`). Returns that session, or null.
function runningSessionForMember(m) {
  const norm = (p) => (p || "").replace(/\/+$/, "");
  const mcwd = norm(m && m.cwd);
  if (!mcwd) return null;
  return (
    state.sessions.find(
      (s) =>
        s.is_running &&
        (norm(s.cwd) === mcwd || norm(s.project_path) === mcwd)
    ) || null
  );
}

/// Active / total member counts for a team's sidebar rollup. "Active" is
/// session-derived (a live session shares the member's cwd) OR the flag stored
/// in config.json — the GUI's list_teams doesn't run the runtime cross-check.
function teamActivity(t) {
  const members = t.members || [];
  const active = members.filter((m) => runningSessionForMember(m) || m.isActive).length;
  return { active, total: members.length };
}

/// Compact signature of which members are currently running — used to decide
/// whether the open team panel needs a live re-render.
function teamRunSignature(t) {
  return (t.members || []).map((m) => (runningSessionForMember(m) ? "1" : "0")).join("");
}

function renderTeams() {
  const list = $("teams-list");
  list.innerHTML = "";
  if (state.teams.length === 0) {
    const empty = document.createElement("div");
    empty.className = "list-empty";
    empty.textContent = "no teams — + to create one";
    list.appendChild(empty);
    return;
  }
  for (const t of state.teams) {
    const item = document.createElement("div");
    item.className = "team-item";
    if (state.openTeamPanel === t.name) item.classList.add("active");
    const { active, total } = teamActivity(t);
    // Rollup: a live dot + "n/m" when any agent is running, else a plain count.
    const rollup =
      active > 0
        ? `<span class="member-dot active"></span><span class="count">${active}/${total}</span>`
        : `<span class="count">${total} agent${total === 1 ? "" : "s"}</span>`;
    item.innerHTML = `<span class="team-icon">${svgIcon("users", 13)}</span><span class="team-name">${escapeHtml(
      t.name
    )}</span>${rollup}`;
    item.onclick = () => showTeamDetails(t);
    item.oncontextmenu = (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      showContextMenu(ev.clientX, ev.clientY, [
        { label: "Details", icon: "info", action: () => showTeamDetails(t) },
        { label: "Rename team…", icon: "pencil", action: () => renameTeamPrompt(t.name) },
        null,
        {
          label: "Delete team…",
          icon: "alert",
          danger: true,
          action: () => deleteTeamConfirm(t.name),
        },
      ]);
    };
    list.appendChild(item);
  }
}

async function renameTeamPrompt(name) {
  const next = await uiPrompt("Rename team to:", name);
  if (next === null || !next.trim() || next.trim() === name) return;
  try {
    await invoke("rename_team", { old: name, new: next.trim() });
  } catch (e) {
    uiAlert(`Rename failed: ${e}`);
    return;
  }
  state.teams = await invoke("list_teams");
  if (state.openTeamPanel === name) state.openTeamPanel = next.trim();
  renderTeams();
  const fresh = state.teams.find((t) => t.name === next.trim());
  if (fresh && $("details") && !$("details").classList.contains("hidden")) showTeamDetails(fresh);
}

async function deleteTeamConfirm(name) {
  if (!(await uiConfirm(`Delete team "${name}" and all its tasks?`, "Delete"))) return;
  try {
    await invoke("delete_team", { name });
    hideDetails();
    state.teams = await invoke("list_teams");
    renderTeams();
  } catch (e) {
    uiAlert(`Delete failed: ${e}`);
  }
}

// ── Notes (scratch) ─────────────────────────────────────────────

async function toggleNotes() {
  state.notesOpen = !state.notesOpen;
  $("notes-caret").textContent = state.notesOpen ? "▾" : "▸";
  $("notes-list").classList.toggle("hidden", !state.notesOpen);
  applySectionHeight("notes-section", "notes-resizer", state.notesOpen, "notesHeight");
  if (state.notesOpen) await refreshNotes();
}

async function refreshNotes() {
  try {
    state.notes = await invoke("list_scratch_notes");
  } catch (e) {
    console.error("list_scratch_notes failed:", e);
    state.notes = [];
  }
  renderNotes();
}

/// Which scratch entries are visible right now: everything except entries
/// nested under a collapsed folder. `state.notes` is a depth-first pre-order
/// flattening (folders first), so a collapsed folder hides the contiguous run
/// of deeper-depth entries that follow it. Mirrors the core's
/// `visible_scratch_indices`, keeping the GUI tree in step with the TUI.
function visibleNotes() {
  const out = [];
  let collapsedDepth = null;
  for (const n of state.notes) {
    if (collapsedDepth !== null) {
      if (n.depth > collapsedDepth) continue;
      collapsedDepth = null;
    }
    out.push(n);
    if (n.isDir && !state.notesExpanded.has(n.id)) collapsedDepth = n.depth;
  }
  return out;
}

function renderNotes() {
  const list = $("notes-list");
  list.innerHTML = "";
  if (state.notes.length === 0) {
    const empty = document.createElement("div");
    empty.className = "list-empty";
    empty.textContent = "no scratches — + to create one";
    list.appendChild(empty);
    return;
  }
  for (const n of visibleNotes()) {
    list.appendChild(buildNoteRow(n));
  }
}

/// One tree row — a file or a folder — with indentation, caret, drag source,
/// drop target (folders), click-to-open / click-to-toggle, and a context menu.
function buildNoteRow(n) {
  const item = document.createElement("div");
  item.className = "team-item note-item" + (n.isDir ? " note-dir" : "");
  item.style.paddingLeft = `${8 + n.depth * 14}px`;

  const caret = n.isDir
    ? `<span class="note-caret ${
        state.notesExpanded.has(n.id) ? "open" : ""
      }">${svgIcon("chevron", 12)}</span>`
    : `<span class="note-caret note-caret-spacer"></span>`;
  const icon = svgIcon(n.isDir ? "folder" : "file", 13);
  item.innerHTML = `${caret}<span class="team-icon">${icon}</span><span class="team-name">${escapeHtml(
    n.title
  )}</span>`;

  item.onclick = (ev) => {
    if (n.isDir) toggleNoteDir(n.id);
    else openScratchInEditor(n, ev.clientX, ev.clientY);
  };
  item.oncontextmenu = (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    noteContextMenu(n, ev.clientX, ev.clientY);
  };

  // Drag source: every entry can be moved.
  item.draggable = true;
  item.addEventListener("dragstart", (ev) => {
    state.notesDragId = n.id;
    ev.dataTransfer.effectAllowed = "move";
    try {
      ev.dataTransfer.setData("text/plain", n.id);
    } catch (_) {}
    item.classList.add("note-dragging");
  });
  item.addEventListener("dragend", () => {
    state.notesDragId = null;
    item.classList.remove("note-dragging");
    document
      .querySelectorAll(".note-drop-hover")
      .forEach((el) => el.classList.remove("note-drop-hover"));
  });
  // Drop target: folders accept a move into themselves.
  if (n.isDir) wireNoteDropTarget(item, n.id);
  return item;
}

/// Make `el` accept drops that move the dragged scratch entry into the folder
/// `targetId` (`""` = root). Rejects no-op and cycle moves up front.
function wireNoteDropTarget(el, targetId) {
  el.addEventListener("dragover", (ev) => {
    if (!canDropNote(targetId)) return;
    ev.preventDefault();
    ev.dataTransfer.dropEffect = "move";
    if (el.classList.contains("team-item")) el.classList.add("note-drop-hover");
  });
  el.addEventListener("dragleave", () => el.classList.remove("note-drop-hover"));
  el.addEventListener("drop", async (ev) => {
    el.classList.remove("note-drop-hover");
    const dragId = state.notesDragId;
    if (!canDropNote(targetId)) return;
    ev.preventDefault();
    ev.stopPropagation();
    await moveNote(dragId, targetId);
  });
}

/// Whether the in-flight drag may drop into folder `targetId`: not onto its own
/// current parent (no-op), and not into itself or a descendant (cycle).
function canDropNote(targetId) {
  const dragId = state.notesDragId;
  if (!dragId) return false;
  const dragged = state.notes.find((n) => n.id === dragId);
  if (!dragged) return false;
  if (dragged.parent === targetId) return false; // already there
  if (targetId === dragId || targetId.startsWith(dragId + "/")) return false; // cycle
  return true;
}

async function moveNote(id, newParent) {
  try {
    const moved = await invoke("move_scratch", { id, newParent });
    if (moved && moved.parent) state.notesExpanded.add(moved.parent);
    await refreshNotes();
  } catch (e) {
    uiAlert(`Move failed: ${e}`);
  }
}

function toggleNoteDir(id) {
  if (state.notesExpanded.has(id)) state.notesExpanded.delete(id);
  else state.notesExpanded.add(id);
  renderNotes();
}

/// Last path segment of an OS path (handles `/` and `\`); `""` if empty.
function baseName(p) {
  const parts = String(p || "").split(/[\\/]/);
  return parts[parts.length - 1] || "";
}

/// Copy `text` to the clipboard via the backend plugin (the bare WKWebView's
/// navigator.clipboard is unreliable — same reasoning as ⌘C) and flash a toast.
function copyScratchPath(text, kind) {
  if (!text) return;
  invoke("clipboard_write_text", { text })
    .then(() => flashToast(`Copied ${kind}`))
    .catch((e) => uiAlert(`Copy failed: ${e}`));
}

/// Context menu for a scratch entry. Folders also offer new file/folder
/// inside them; everything offers copy-path, rename, and delete.
function noteContextMenu(n, x, y) {
  const items = [];
  if (n.isDir) {
    items.push({
      label: "New scratch…",
      icon: "plus",
      action: () => newNotePrompt(x, y, n.id),
    });
    items.push({
      label: "New folder…",
      icon: "folder",
      action: () => newFolderPrompt(n.id),
    });
  } else {
    items.push({
      label: "Open in editor…",
      icon: "pencil",
      action: () => openScratchInEditor(n, x, y),
    });
  }
  // Copy path/reference — mirrors the TUI's `y` picker (absolute / relative /
  // name) so a path can be pasted straight into a Claude session.
  items.push(null);
  items.push({
    label: "Copy absolute path",
    icon: "copy",
    action: () => copyScratchPath(n.path, "absolute path"),
  });
  items.push({
    label: "Copy relative path",
    icon: "copy",
    action: () => copyScratchPath(n.id, "relative path"),
  });
  items.push({
    label: n.isDir ? "Copy folder name" : "Copy file name",
    icon: "copy",
    action: () => copyScratchPath(baseName(n.path) || n.title, "name"),
  });
  items.push(null);
  items.push({
    label: "Rename…",
    icon: "pencil",
    action: () => renameNotePrompt(n),
  });
  items.push({
    label: n.isDir ? "Delete folder…" : "Delete scratch…",
    icon: "alert",
    danger: true,
    action: () => deleteNoteConfirm(n),
  });
  showContextMenu(x, y, items);
}

/// Create a note inside `parent` (`""` = root). Opens the editor picker on the
/// new note when created from the root `+` button (x/y position the picker).
async function newNotePrompt(x, y, parent = "") {
  const where = parent ? ` in ${parent}` : "";
  const title = await uiPrompt(`New scratch title${where}`, "");
  if (title === null) return;
  const trimmed = (title || "").trim();
  if (!trimmed) return;
  try {
    const note = await invoke("create_scratch_note", { parent, title: trimmed });
    if (parent) state.notesExpanded.add(parent);
    await refreshNotes();
    openScratchInEditor(note, x, y);
  } catch (e) {
    uiAlert(`Create scratch failed: ${e}`);
  }
}

async function newFolderPrompt(parent = "") {
  const where = parent ? ` in ${parent}` : "";
  const name = await uiPrompt(`New folder name${where}`, "");
  if (name === null) return;
  const trimmed = (name || "").trim();
  if (!trimmed) return;
  try {
    const dir = await invoke("create_scratch_dir", { parent, name: trimmed });
    if (parent) state.notesExpanded.add(parent);
    if (dir && dir.id) state.notesExpanded.add(dir.id);
    await refreshNotes();
  } catch (e) {
    uiAlert(`Create folder failed: ${e}`);
  }
}

async function renameNotePrompt(n) {
  // Pre-fill with the on-disk name (file name with extension, or folder name).
  const current = n.id.includes("/") ? n.id.slice(n.id.lastIndexOf("/") + 1) : n.id;
  const name = await uiPrompt(`Rename "${current}" to`, current);
  if (name === null) return;
  const trimmed = (name || "").trim();
  if (!trimmed || trimmed === current) return;
  try {
    await invoke("rename_scratch", { id: n.id, newName: trimmed });
    await refreshNotes();
  } catch (e) {
    uiAlert(`Rename failed: ${e}`);
  }
}

async function deleteNoteConfirm(note) {
  const msg = note.isDir
    ? `Delete folder "${note.title}" and everything inside it?`
    : `Delete scratch "${note.title}"?`;
  if (!(await uiConfirm(msg, "Delete"))) return;
  try {
    await invoke("delete_scratch_note", { id: note.id });
    state.notesExpanded.delete(note.id);
    await refreshNotes();
  } catch (e) {
    uiAlert(`Delete failed: ${e}`);
  }
}

/// Open a scratch via the editor picker — the GUI equivalent of the TUI's
/// editor-picker flow. Terminal editors (vim/nvim/emacs/nano…) open in an
/// in-app terminal tab; GUI editors (VS Code/Cursor/Zed…) launch alongside,
/// like opening a project. (x, y) position the picker menu near the click.
async function openScratchInEditor(note, x, y) {
  let editors = [];
  try {
    editors = await invoke("detect_editors");
  } catch (e) {
    console.error("detect_editors failed:", e);
  }
  if (!editors.length) {
    uiAlert(
      "No editors detected. Install a terminal editor (vim, nano) or a GUI editor (VS Code, Cursor, Zed)."
    );
    return;
  }
  const px = typeof x === "number" ? x : 220;
  const py = typeof y === "number" ? y : 220;
  showContextMenu(
    px,
    py,
    editors.map((ed) => ({
      label: ed.label,
      hint: ed.description,
      icon: ed.value.startsWith("terminal:") ? "terminal" : "external-link",
      action: () => launchScratchEditor(note, ed.value),
    }))
  );
}

/// Launch the chosen editor on a scratch file. Terminal editors get an in-app
/// PTY tab (spawned via the daemon, dies when you quit the editor); GUI
/// editors are launched externally via open_in_ide on the note's file path.
async function launchScratchEditor(note, value) {
  try {
    if (value.startsWith("terminal:")) {
      const cmd = value.slice("terminal:".length);
      const sid = await invoke("open_scratch_terminal_editor", {
        editor: cmd,
        path: note.path,
        cols: 120,
        rows: 40,
      });
      await openSession(sid, `📝 ${note.title}`);
    } else {
      await invoke("open_in_ide", { value, projectDir: note.path });
    }
  } catch (e) {
    uiAlert(`Open failed: ${e}`);
  }
}

const TASK_STATES = ["pending", "in_progress", "completed", "blocked"];

async function showTeamDetails(team) {
  $("details").classList.remove("hidden");
  $("details-resizer").classList.remove("hidden");
  $("details-btn").classList.add("on");
  state.detailsFor = null;
  state.openTeamPanel = team.name; // enables live refresh while open
  detailsShellFor = null; // team view replaces the session shell
  const body = $("details-body");
  let tasks = [];
  try {
    tasks = await invoke("list_tasks", { team: team.name });
  } catch (e) {
    console.error("list_tasks failed:", e);
  }
  const members = team.members || [];
  const { active } = teamActivity(team);
  const activeNote =
    active > 0
      ? `<span class="dim" style="font-weight:400">— ${active} running</span>`
      : "";
  body.innerHTML = `
    <h3>${svgIcon("users", 13)} <span id="d-team-name" title="Click to rename">${escapeHtml(
      team.name
    )}</span></h3>
    <p class="hint" id="d-team-desc" title="Click to edit description">${
      team.description ? escapeHtml(team.description) : "<span class='dim'>no description — click to add</span>"
    } ✎</p>
    <h4>MEMBERS (${members.length}) ${activeNote}</h4>
    <div id="d-members"></div>
    <button id="d-member-add" class="ghost-action">＋ Add member</button>
    <h4>TASKS (${tasks.length}) <button id="d-task-add" class="mini-add" title="New task">＋</button></h4>
    <div id="d-tasks"></div>
    <div class="actions">
      <button id="d-team-rename">Rename</button>
      <button id="d-team-delete" class="danger">Delete team</button>
      <button id="d-close">Close panel</button>
    </div>
    <div id="d-out"></div>
  `;

  // ── Members ──────────────────────────────────────────────────
  const membersEl = $("d-members");
  if (members.length === 0) {
    membersEl.innerHTML =
      "<p class='hint'>none yet — add one, or agents join when Claude spawns them into this team</p>";
  }
  for (const m of members) {
    const sess = runningSessionForMember(m);
    const running = !!sess;
    const row = document.createElement("div");
    row.className = "row-item member-row" + (running ? " is-running" : "");
    // Member serializes camelCase (serde rename_all) — agentType/isActive.
    const dot = running || m.isActive ? "active" : "idle";
    const openHint = running
      ? `<span class="member-open" title="Open running session">▶</span>`
      : "";
    row.innerHTML =
      `<span class="member-dot ${dot}"></span>` +
      `<span class="member-name">${escapeHtml(m.name)}</span>` +
      `<span class="dim">${escapeHtml(m.agentType || "")}</span>` +
      (m.model ? `<span class="mini-chip">${escapeHtml(m.model)}</span>` : "") +
      openHint;
    // Left-click: jump to the running session if there is one, else the inbox.
    row.onclick = () => {
      if (running) openSession(sess.id);
      else showInbox(team.name, m.name);
    };
    row.oncontextmenu = (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      const items = [];
      if (running)
        items.push({
          label: "Open session",
          icon: "external-link",
          action: () => openSession(sess.id),
        });
      items.push({ label: "Inbox", icon: "info", action: () => showInbox(team.name, m.name) });
      items.push(null);
      items.push({
        label: "Change model…",
        icon: "terminal",
        action: () => editMember(team.name, m, "model"),
      });
      items.push({
        label: "Change type…",
        icon: "pencil",
        action: () => editMember(team.name, m, "type"),
      });
      items.push({
        label: "Edit prompt…",
        icon: "pencil",
        action: () => editMember(team.name, m, "prompt"),
      });
      items.push({
        label: "Rename member…",
        icon: "pencil",
        action: () => editMember(team.name, m, "rename"),
      });
      items.push(null);
      items.push({
        label: "Remove member…",
        icon: "alert",
        danger: true,
        action: async () => {
          if (!(await uiConfirm(`Remove "${m.name}" from "${team.name}"?`, "Remove"))) return;
          await teamMutation(team.name, () =>
            invoke("remove_team_member", { team: team.name, member: m.name })
          );
        },
      });
      showContextMenu(ev.clientX, ev.clientY, items);
    };
    membersEl.appendChild(row);
  }

  // ── Tasks ────────────────────────────────────────────────────
  const tasksEl = $("d-tasks");
  if (tasks.length === 0) tasksEl.innerHTML = "<p class='hint'>no tasks — ＋ to add one</p>";
  for (const t of tasks) {
    const st = String(t.status || "").toLowerCase().replace(/\s+/g, "_");
    const row = document.createElement("div");
    row.className = "task-item";
    row.innerHTML =
      `<span class="task-status ${st}" title="Click to cycle status">${escapeHtml(
        String(t.status)
      )}</span>` +
      `<span class="task-subject">${escapeHtml(t.subject || t.id)}</span>` +
      (t.owner ? `<span class="mini-chip">${escapeHtml(t.owner)}</span>` : "");
    // Click the status badge to cycle it.
    row.querySelector(".task-status").onclick = (ev) => {
      ev.stopPropagation();
      taskMutation(team.name, () =>
        invoke("cycle_task_status", { team: team.name, taskId: t.id })
      );
    };
    row.oncontextmenu = (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      const statusItems = TASK_STATES.map((s) => ({
        label: s === st ? `● ${s.replace("_", " ")}` : `  ${s.replace("_", " ")}`,
        action: () =>
          taskMutation(team.name, () =>
            invoke("set_task_status", { team: team.name, taskId: t.id, status: s })
          ),
      }));
      showContextMenu(ev.clientX, ev.clientY, [
        ...statusItems,
        null,
        {
          label: "Assign owner…",
          icon: "users",
          action: () => assignTaskOwner(team, t),
        },
        null,
        {
          label: "Delete task…",
          icon: "alert",
          danger: true,
          action: async () => {
            if (!(await uiConfirm(`Delete task "${t.subject || t.id}"?`, "Delete"))) return;
            await taskMutation(team.name, () =>
              invoke("delete_task", { team: team.name, taskId: t.id })
            );
          },
        },
      ]);
    };
    tasksEl.appendChild(row);
  }

  // ── Actions / edit affordances ───────────────────────────────
  $("d-team-name").onclick = () => renameTeamPrompt(team.name);
  $("d-team-desc").onclick = async () => {
    const description = await uiPrompt("Team description:", team.description || "");
    if (description === null) return;
    await teamMutation(team.name, () =>
      invoke("update_team_description", { name: team.name, description: description.trim() })
    );
  };
  $("d-member-add").onclick = async () => {
    const name = await uiPrompt("Member name:");
    if (!name || !name.trim()) return;
    const agentType = await uiPrompt("Agent type:", "general-purpose");
    if (agentType === null) return;
    const model = await uiPrompt("Model (empty = inherit):");
    if (model === null) return;
    await teamMutation(team.name, () =>
      invoke("add_team_member", {
        team: team.name,
        name: name.trim(),
        agentType: agentType.trim(),
        model: model.trim(),
      })
    );
  };
  $("d-task-add").onclick = async (ev) => {
    ev.stopPropagation();
    const subject = await uiPrompt("Task subject:");
    if (!subject || !subject.trim()) return;
    const description = (await uiPrompt("Description (optional):")) || "";
    await taskMutation(team.name, () =>
      invoke("create_task", { team: team.name, subject: subject.trim(), description })
    );
  };
  $("d-team-rename").onclick = () => renameTeamPrompt(team.name);
  $("d-close").onclick = hideDetails;
  $("d-team-delete").onclick = () => deleteTeamConfirm(team.name);
  fitAll();
}

/// Edit one field of a member via a prompt, then persist + refresh.
async function editMember(teamName, m, field) {
  if (field === "model") {
    const model = await uiPrompt(`Model for "${m.name}" (empty = inherit):`, m.model || "");
    if (model === null) return;
    return teamMutation(teamName, () =>
      invoke("set_team_member_model", { team: teamName, member: m.name, model: model.trim() })
    );
  }
  if (field === "type") {
    const t = await uiPrompt(`Agent type for "${m.name}":`, m.agentType || "general-purpose");
    if (t === null) return;
    return teamMutation(teamName, () =>
      invoke("set_team_member_type", { team: teamName, member: m.name, agentType: t.trim() })
    );
  }
  if (field === "prompt") {
    const p = await uiPrompt(`System prompt for "${m.name}":`, m.prompt || "");
    if (p === null) return;
    return teamMutation(teamName, () =>
      invoke("set_team_member_prompt", { team: teamName, member: m.name, prompt: p })
    );
  }
  if (field === "rename") {
    const next = await uiPrompt(`Rename "${m.name}" to:`, m.name);
    if (next === null || !next.trim() || next.trim() === m.name) return;
    return teamMutation(teamName, () =>
      invoke("rename_team_member", { team: teamName, old: m.name, new: next.trim() })
    );
  }
}

/// Assign a task's owner via a picker of the team's members (blank = clear).
async function assignTaskOwner(team, task) {
  const members = team.members || [];
  const items = [
    { label: "(unassigned)", action: () => setOwner("") },
    ...members.map((m) => ({ label: m.name, icon: "users", action: () => setOwner(m.name) })),
  ];
  function setOwner(owner) {
    taskMutation(team.name, () =>
      invoke("set_task_owner", { team: team.name, taskId: task.id, owner })
    );
  }
  // Anchor near the panel; a simple centered menu is fine here.
  const r = $("details").getBoundingClientRect();
  showContextMenu(r.left + 20, r.top + 60, items);
}

/// Run a team mutation, then reload teams and re-open the details panel
/// so the change is visible immediately.
async function teamMutation(teamName, run) {
  try {
    await run();
  } catch (e) {
    uiAlert(`Team update failed: ${e}`);
    return;
  }
  await refreshTeamPanel(teamName);
}

/// Task mutations reuse the same reload-and-reopen path (showTeamDetails
/// re-fetches the task list).
async function taskMutation(teamName, run) {
  try {
    await run();
  } catch (e) {
    uiAlert(`Task update failed: ${e}`);
    return;
  }
  await refreshTeamPanel(teamName);
}

/// Reload teams and, if the panel is still on this team, re-render it.
async function refreshTeamPanel(teamName) {
  try {
    state.teams = await invoke("list_teams");
    renderTeams();
    const fresh = state.teams.find((t) => t.name === teamName);
    if (fresh && state.openTeamPanel === teamName) await showTeamDetails(fresh);
  } catch (e) {
    console.error("team refresh failed:", e);
  }
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
  // Prefill cwd fresh on every open — a stale value from a previous open
  // is never kept. The configured default directory (settings) wins, then
  // the focused session's project, then home. Never leaves the field empty.
  const cur = state.sessions.find((x) => x.id === state.activeTab);
  $("ns-cwd").value =
    state.settings.defaultCwd ||
    (cur && (cur.cwd || cur.project_path)) ||
    state.homeDir ||
    "";
  if ($("ns-cwd").value) loadPresetsForCwd();
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
  if (isShellTerm(session_id)) {
    // `exit` in a shell terminal closes its tab, like a real terminal.
    dropTerminal(session_id);
    return;
  }
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

// Fresh workspace: the "no session" overlay is a quick-start surface — click
// (or right-click) anywhere on it to open the unified new-tab menu and launch
// a terminal, browser, or Claude session straight into the focused pane.
{
  const quickStart = (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    showNewTabMenu(ev.clientX, ev.clientY);
  };
  $("empty-state").addEventListener("click", quickStart);
  $("empty-state").addEventListener("contextmenu", quickStart);
}
$("modal-backdrop").addEventListener("click", (e) => {
  if (e.target === $("modal-backdrop")) hideNewSessionModal();
});
// Enter submits from ANY field in the modal (name, working directory, preset,
// or the worktree checkbox) — not just the working-directory input, which was
// the only field that used to react. Escape is handled by the global keydown
// handler (hideNewSessionModal). Modifier-Enter combos (e.g. ⌘⇧↩ zoom) fall
// through to the global handler untouched.
$("new-session-modal").addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey) {
    e.preventDefault();
    createSession();
  }
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

$("notes-toggle").onclick = toggleNotes;
// Manual refresh (re-list the scratch dir). A backend watcher also pushes
// `scratch-changed` on external edits, but the button is an always-works
// fallback (and expands the section if collapsed).
$("refresh-notes-btn").innerHTML = svgIcon("reload", 13);
$("refresh-notes-btn").onclick = (e) => {
  e.stopPropagation();
  spinButton($("refresh-notes-btn"), async () => {
    if (!state.notesOpen) await toggleNotes(); // opening already refreshes
    else await refreshNotes();
  });
};
// The list itself is a drop target → moving an entry to the scratch root.
// Wired once (the element persists across re-renders; rows are rebuilt each time).
wireNoteDropTarget($("notes-list"), "");
// Backend scratch-directory watcher: auto-refresh the tree when files change
// outside clash (an editor saving, the TUI, git…). Only re-list when the
// section is open — a collapsed section refreshes on next expand anyway.
listen("scratch-changed", () => {
  if (state.notesOpen) refreshNotes();
});
$("new-note-btn").onclick = (e) => {
  e.stopPropagation();
  // Make sure the section is expanded so the new entry is visible after refresh.
  if (!state.notesOpen) toggleNotes();
  // Offer a new note or a new folder at the scratch root.
  showContextMenu(e.clientX, e.clientY, [
    {
      label: "New scratch…",
      icon: "plus",
      action: () => newNotePrompt(e.clientX, e.clientY, ""),
    },
    {
      label: "New folder…",
      icon: "folder",
      action: () => newFolderPrompt(""),
    },
  ]);
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
      uiDialog({
        message: `clash v${version} installed. Restart now? Running sessions will be closed.`,
        okLabel: "Restart",
      }).then((restart) => {
        if (restart) invoke("restart_app").catch((e) => uiAlert(`Restart failed: ${e}`));
      });
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

// SETTINGS section collapses like TEAMS; the choice persists. Collapsed
// by default — the footer rows (session count, version) stay visible.
function toggleSettings(open) {
  const want = open ?? $("settings-body").classList.contains("hidden");
  $("settings-body").classList.toggle("hidden", !want);
  $("settings-caret").textContent = want ? "▾" : "▸";
  try {
    localStorage.setItem("clash-settings-open", want ? "1" : "0");
  } catch (e) {
    void e;
  }
}
$("settings-toggle").onclick = () => toggleSettings();
try {
  if (localStorage.getItem("clash-settings-open") === "1") toggleSettings(true);
} catch (e) {
  void e;
}

// Font-family autocomplete: offer the monospace fonts actually installed
// (document.fonts.check resolves real families without loading anything).
// Free typing still works — the datalist only suggests.
function populateFontOptions() {
  const candidates = [
    "SF Mono", "Menlo", "Monaco", "JetBrains Mono", "Fira Code", "Fira Mono",
    "Hack", "Source Code Pro", "IBM Plex Mono", "Cascadia Code", "Consolas",
    "Inconsolata", "Ubuntu Mono", "DejaVu Sans Mono", "Roboto Mono",
    "Iosevka", "Victor Mono", "Geist Mono", "Berkeley Mono", "MesloLGS NF",
    "Liberation Mono", "PT Mono", "Space Mono", "Noto Sans Mono",
    "Andale Mono", "Courier New",
  ];
  const list = $("font-options");
  list.innerHTML = "";
  for (const f of candidates) {
    let available = false;
    try {
      available = document.fonts.check(`12px "${f}"`);
    } catch (e) {
      void e;
    }
    if (!available) continue;
    const opt = document.createElement("option");
    opt.value = f;
    list.appendChild(opt);
  }
}
populateFontOptions();

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
  if (e.metaKey && e.shiftKey && e.key.toLowerCase() === "t") {
    e.preventDefault();
    openShellTerminal(state.settings.termShell || "");
    return;
  }
  if (e.metaKey && e.key === "t") {
    e.preventDefault();
    showNewSessionModal();
    return;
  }
  if (e.metaKey && e.key.toLowerCase() === "d") {
    e.preventDefault();
    if (e.shiftKey) removePane();
    else addPane();
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
    openBrowserTab(); // blank tab, address bar focused
    return;
  }
  // Browser-pane shortcuts (apply when the focused pane holds a browser
  // tab; keystrokes inside the native page never reach this handler).
  {
    const focusedEntry = state.open.get(ws().panes[ws().focused]);
    if (focusedEntry?.kind === "browser" && e.metaKey && !e.shiftKey && !e.altKey) {
      if (e.key.toLowerCase() === "l") {
        e.preventDefault();
        focusedEntry.urlInput?.focus();
        return;
      }
      if (e.key === "=" || e.key === "+") {
        e.preventDefault();
        browserZoom(focusedEntry, 0.1);
        return;
      }
      if (e.key === "-") {
        e.preventDefault();
        browserZoom(focusedEntry, -0.1);
        return;
      }
      if (e.key === "0") {
        e.preventDefault();
        browserZoom(focusedEntry, 0);
        return;
      }
      if (e.key.toLowerCase() === "r") {
        e.preventDefault();
        invoke("browser_reload", { tab: focusedEntry.tabId }).catch(() => {});
        return;
      }
    }
  }
  // ⌘R — reload (restart) the focused session on the latest Claude, resuming
  // its conversation: the ⟳ button as a shortcut. Browser panes handled their
  // own ⌘R above. preventDefault unconditionally so ⌘R never reloads the whole
  // GUI webview; it's a no-op when the focused pane isn't a Claude session.
  if (e.metaKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "r") {
    e.preventDefault();
    const focusKey = ws().panes[ws().focused];
    const entry = focusKey && state.open.get(focusKey);
    if (entry && entry.kind === "claude") {
      reloadSessionInteractive(state.sessions.find((x) => x.id === focusKey));
    }
    return;
  }
  if (e.metaKey && e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
    e.preventDefault();
    focusPaneDelta(e.key === "ArrowRight" ? 1 : -1);
    return;
  }
  if (e.metaKey && !e.shiftKey && e.key === "w") {
    e.preventDefault();
    if (state.activeTab) closeTab(state.activeTab);
    return;
  }
  if (e.metaKey && e.key === "f") {
    e.preventDefault();
    $("search").focus();
    return;
  }
  if (e.metaKey && !e.shiftKey && e.key === "k") {
    e.preventDefault();
    const entry = state.activeTab && state.open.get(state.activeTab);
    if (entry && entry.term) entry.term.clear();
    return;
  }
  if (e.key === "/" && !inInput) {
    e.preventDefault();
    $("search").focus();
  }
});

window.addEventListener("resize", fitAll);

// Persist "where we were" the moment clash loses focus / is hidden / closes —
// the debounced saveWorkspaces might not have flushed the latest layout yet,
// and Tauri's async IPC can't reliably complete during teardown, so we write
// eagerly on these signals (blur fires well before a Cmd+Q quit).
window.addEventListener("blur", flushWorkspaces);
window.addEventListener("pagehide", flushWorkspaces);
window.addEventListener("beforeunload", flushWorkspaces);
document.addEventListener("visibilitychange", () => {
  if (document.hidden) flushWorkspaces();
});

// ── Browser tabs (first-class tabs, one child webview each) ──────
// A browser tab is a regular `state.open` entry living in panes and
// workspaces like terminals do. Its page is a native child webview the
// backend positions over the tab's .b-slot rect; the frontend owns
// visibility (created lazily the first time the tab becomes visible).

let browserNextTabId = 1; // monotonic: webview labels are never reused
let browserUrlPoll = null;

/// Forward frontend diagnostics to clash.log (the webview console is
/// invisible in release builds). Uncaught errors and unhandled promise
/// rejections always go through here.
function dlog(...a) {
  invoke("gui_log", { msg: a.map((x) => (typeof x === "object" ? JSON.stringify(x) : String(x))).join(" ") }).catch(() => {});
}
window.addEventListener("error", (e) => dlog("uncaught error:", e.message, e.filename + ":" + e.lineno));
window.addEventListener("unhandledrejection", (e) => dlog("unhandled rejection:", e.reason && e.reason.stack ? e.reason.stack : e.reason));
let pendingBrowserTabs = []; // persisted tabs awaiting restore at boot

function isBrowserTab(id) {
  return id.startsWith("browser-");
}

function hostnameOf(url) {
  try {
    return new URL(url).hostname.replace(/^www\./, "") || url;
  } catch {
    return url || "tab";
  }
}

/// Address-bar input → navigable URL. Explicit schemes pass through,
/// host-looking strings get https:// (http:// for localhost), anything
/// else becomes a web search.
function normalizeBrowserInput(raw) {
  const s = raw.trim();
  if (!s) return null;
  if (/^[a-z][a-z0-9+.-]*:/i.test(s)) return s; // explicit scheme
  if (s === "localhost" || /^localhost[:/]/.test(s)) return "http://" + s;
  if (/^(\d{1,3}\.){3}\d{1,3}(:\d+)?(\/|$)/.test(s)) return "http://" + s;
  if (!/\s/.test(s) && /^[\w-]+(\.[\w-]+)+/.test(s)) return "https://" + s;
  return "https://duckduckgo.com/?q=" + encodeURIComponent(s);
}

function browserNavigate(entry, url) {
  entry.url = url;
  if (!entry.renamed) entry.name = hostnameOf(url);
  renderTabs();
  saveWorkspaces();
  if (entry.created) {
    invoke("browser_navigate", { tab: entry.tabId, url }).catch((err) =>
      uiAlert(`Navigate failed: ${err}`),
    );
  }
}

function browserZoom(entry, delta) {
  entry.zoom = delta === 0 ? 1 : Math.min(5, Math.max(0.25, (entry.zoom || 1) + delta));
  invoke("browser_set_zoom", { tab: entry.tabId, factor: entry.zoom }).catch(() => {});
}

/// Per-pane chrome strip — back/forward, reload⇄stop, address bar
/// (URL or search), copy-URL, open-external — above the .b-slot div
/// the native webview covers.
function buildBrowserPaneEl(entry) {
  const el = document.createElement("div");
  el.className = "browser-pane";

  const chrome = document.createElement("div");
  chrome.className = "b-chrome";
  // Clicks inside the native webview never reach the DOM — clicking the
  // chrome strip is how a browser pane takes focus.
  chrome.addEventListener("mousedown", () => {
    const w = ws();
    const i = w.panes.indexOf(entry.id);
    if (i >= 0 && w.focused !== i) {
      w.focused = i;
      syncActiveToFocused();
      renderPanes();
      renderTabs();
    }
  });

  const btn = (icon, title, fn) => {
    const b = document.createElement("button");
    b.className = "icon-btn";
    b.title = title;
    b.innerHTML = svgIcon(icon);
    b.onclick = fn;
    chrome.appendChild(b);
    return b;
  };
  btn("arrow-left", "Back", () =>
    invoke("browser_history", { tab: entry.tabId, delta: -1 }).catch(() => {}),
  );
  btn("arrow-right", "Forward", () =>
    invoke("browser_history", { tab: entry.tabId, delta: 1 }).catch(() => {}),
  );
  const navBtn = btn("reload", "Reload", () => {
    if (entry.loading) invoke("browser_stop", { tab: entry.tabId }).catch(() => {});
    else invoke("browser_reload", { tab: entry.tabId }).catch(() => {});
  });
  // Reload ⇄ Stop, driven by browser-nav page-load events.
  entry.setNavState = () => {
    navBtn.innerHTML = svgIcon(entry.loading ? "x" : "reload");
    navBtn.title = entry.loading ? "Stop" : "Reload";
    navBtn.classList.toggle("loading", !!entry.loading);
  };

  const urlInput = document.createElement("input");
  urlInput.type = "text";
  urlInput.className = "b-url";
  urlInput.spellcheck = false;
  urlInput.placeholder = "Search or enter address";
  urlInput.value = entry.url === "about:blank" ? "" : entry.url;
  urlInput.addEventListener("focus", () => urlInput.select());
  urlInput.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      urlInput.value = entry.url === "about:blank" ? "" : entry.url;
      urlInput.blur();
      return;
    }
    if (e.key !== "Enter") return;
    const url = normalizeBrowserInput(urlInput.value);
    if (!url) return;
    browserNavigate(entry, url);
    urlInput.blur();
  });
  chrome.appendChild(urlInput);
  entry.urlInput = urlInput;

  btn("copy", "Copy URL", () => {
    navigator.clipboard?.writeText(entry.url).catch(() => {});
  });
  btn("external-link", "Open in system browser", () =>
    invoke("open_external", { url: entry.url }).catch(console.error),
  );

  const slot = document.createElement("div");
  slot.className = "b-slot";

  el.appendChild(chrome);
  el.appendChild(slot);
  entry.slot = slot;
  return el;
}

/// Build the `state.open` entry for a browser tab (no webview yet —
/// `syncBrowserWebviews` materializes it on first visibility).
function makeBrowserEntry(id, url, name, renamed) {
  const entry = {
    kind: "browser",
    id,
    tabId: id.slice("browser-".length),
    url,
    name: name || (url === "about:blank" ? "New tab" : hostnameOf(url)),
    renamed: !!renamed,
    created: false,
    creating: false,
    loading: false,
    zoom: 1,
    el: null,
    slot: null,
    urlInput: null,
    setNavState: null,
  };
  entry.el = buildBrowserPaneEl(entry);
  return entry;
}

/// Open `url` as a first-class tab in the active workspace. Reuses an
/// existing tab showing the same URL instead of duplicating it. Without
/// a URL, opens a blank tab with the address bar focused (browser-like).
/// If the focused pane already holds something, push a fresh split pane
/// and focus it — so a browser open lands beside the current session
/// instead of replacing it. No-op when the focused pane is empty.
function ensureFreePane() {
  const w = ws();
  if (w.panes[w.focused] != null) {
    w.panes.push(null);
    w.focused = w.panes.length - 1;
    w.zoomed = false;
  }
}

/// Open a URL in a clash browser tab. `mode` controls placement:
///   - undefined: take over the focused pane (the blank "new tab" command).
///   - "split": open in a NEW split pane beside the current session, which
///     stays visible — used by PR/link/port/repo opens so a browser never
///     evicts the session you're working in.
///   - "background": always create a fresh tab in the strip/sidebar without
///     stealing focus — used by link clicks (target="_blank", window.open)
///     inside the embedded browser; no dedup, no pane takeover, no switch.
function openBrowserTab(url, mode) {
  const blank = !url;
  if (blank) url = "about:blank";
  const w = ws();
  const background = mode === "background";
  const split = mode === "split";
  if (!blank && !background) {
    for (const [id, entry] of state.open) {
      if (entry.kind === "browser" && entry.url === url && w.sessions.includes(id)) {
        // Already open here — surface it (in its own split if split-mode
        // and it isn't already in a pane) rather than spawning a duplicate.
        if (split && w.panes.indexOf(id) < 0) ensureFreePane();
        assignToFocusedPane(id);
        return;
      }
    }
  }
  const id = "browser-" + browserNextTabId++;
  const entry = makeBrowserEntry(id, url);
  state.open.set(id, entry);
  claimSession(id);
  if (background) {
    // Surface the new tab in the strip/sidebar without stealing focus.
    saveWorkspaces();
    renderTabs();
    renderSidebar();
  } else {
    if (split) ensureFreePane();
    assignToFocusedPane(id);
    saveWorkspaces();
  }
  if (!browserUrlPoll) browserUrlPoll = setInterval(syncBrowserUrls, 1500);
  if (blank) setTimeout(() => entry.urlInput?.focus(), 50);
}

/// Recreate persisted browser tabs (entries only — webviews are lazy).
function restoreBrowserTabs() {
  const owned = new Set(state.workspaces.flatMap((w) => w.sessions));
  let maxId = 0;
  for (const t of pendingBrowserTabs) {
    if (!t || typeof t.id !== "string" || !isBrowserTab(t.id)) continue;
    const n = parseInt(t.id.slice("browser-".length), 10);
    if (Number.isFinite(n)) maxId = Math.max(maxId, n);
    if (!owned.has(t.id) || typeof t.url !== "string" || !t.url) continue;
    state.open.set(t.id, makeBrowserEntry(t.id, t.url, t.name, t.renamed));
  }
  browserNextTabId = maxId + 1;
  pendingBrowserTabs = [];
  if (state.open.size && !browserUrlPoll) {
    browserUrlPoll = setInterval(syncBrowserUrls, 1500);
  }
}

/// Single source of truth for webview geometry/visibility — runs after
/// every layout change (fitAll). A browser tab's webview is shown iff
/// the tab sits in a visible pane of the active workspace.
function syncBrowserWebviews() {
  const w = ws();
  const visible = new Set(
    (w.zoomed ? [w.panes[w.focused]] : w.panes).filter(
      (id) => id && isBrowserTab(id),
    ),
  );
  for (const [id, entry] of state.open) {
    if (entry.kind !== "browser") continue;
    // Pre-creation states are the diagnostic gold: why a webview did or
    // didn't materialize. Quiet once created (set_bounds churn is noise).
    if (!entry.created)
      dlog("browser sync:", id, "visible=" + visible.has(id), "connected=" + !!(entry.slot && entry.slot.isConnected), "creating=" + entry.creating);
    if (visible.has(id) && entry.slot && entry.slot.isConnected) {
      const r = entry.slot.getBoundingClientRect();
      if (!entry.created)
        dlog("browser sync rect:", id, JSON.stringify({ x: r.x, y: r.y, w: r.width, h: r.height }));
      if (r.width <= 0 || r.height <= 0) continue; // layout not settled yet
      const rect = { x: r.x, y: r.y, w: r.width, h: r.height };
      if (!entry.created) {
        if (entry.creating) continue;
        entry.creating = true;
        invoke("browser_open", { tab: entry.tabId, url: entry.url, ...rect })
          .then(() => {
            entry.created = true;
            if (entry.zoom && entry.zoom !== 1) {
              invoke("browser_set_zoom", { tab: entry.tabId, factor: entry.zoom }).catch(() => {});
            }
          })
          .catch((e) => dlog("browser_open failed:", entry.tabId, e))
          .finally(() => {
            entry.creating = false;
          });
      } else {
        invoke("browser_set_bounds", { tab: entry.tabId, ...rect }).catch(() => {});
        invoke("browser_set_visible", { tab: entry.tabId, visible: true }).catch(() => {});
      }
    } else if (entry.created) {
      invoke("browser_set_visible", { tab: entry.tabId, visible: false }).catch(() => {});
    }
  }
}

/// Native webviews paint over the DOM — hide them while a modal dialog
/// is up so it stays visible and clickable. fitAll() restores them.
function hideBrowserWebviews() {
  for (const entry of state.open.values()) {
    if (entry.kind === "browser" && entry.created) {
      invoke("browser_set_visible", { tab: entry.tabId, visible: false }).catch(() => {});
    }
  }
}

/// Keep tab labels and URL bars in sync with in-page navigation.
async function syncBrowserUrls() {
  const w = ws();
  const visible = w.zoomed ? [w.panes[w.focused]] : w.panes;
  for (const id of visible) {
    const entry = id && state.open.get(id);
    if (!entry || entry.kind !== "browser" || !entry.created) continue;
    try {
      const url = await invoke("browser_get_url", { tab: entry.tabId });
      if (url && url !== entry.url) {
        entry.url = url;
        if (!entry.renamed) entry.name = hostnameOf(url);
        renderTabs();
        saveWorkspaces();
      }
      if (entry.urlInput && document.activeElement !== entry.urlInput) {
        entry.urlInput.value = entry.url === "about:blank" ? "" : entry.url;
      }
    } catch (e) {
      void e;
    }
  }
}

// Page-load lifecycle from the backend — spinner/stop state, plus
// instant URL-bar and tab-label updates (the poll only covers SPAs).
listen("browser-nav", (event) => {
  const { tab, event: phase, url } = event.payload;
  for (const entry of state.open.values()) {
    if (entry.kind !== "browser" || entry.tabId !== tab) continue;
    entry.loading = phase === "started";
    if (url && url !== entry.url) {
      entry.url = url;
      if (!entry.renamed) {
        entry.name = url === "about:blank" ? "New tab" : hostnameOf(url);
      }
      renderTabs();
      saveWorkspaces();
    }
    if (entry.urlInput && document.activeElement !== entry.urlInput) {
      entry.urlInput.value = entry.url === "about:blank" ? "" : entry.url;
    }
    entry.setNavState?.();
    break;
  }
});

// A link inside the embedded browser that wants a new window/tab
// (target="_blank", window.open) opens a new clash browser tab in the
// background — the tab the user is reading stays focused.
listen("browser-open-tab", (event) => {
  const url = event.payload;
  if (typeof url === "string" && /^https?:\/\//.test(url)) openBrowserTab(url, "background");
});

// Pane-area geometry changes that bypass renderPanes (details panel
// open/close, sidebar drag) still move the slots — observe the host.
new ResizeObserver(() => {
  syncBrowserWebviews();
  repositionGutters($("terminal-host"), ws());
}).observe($("terminal-host"));

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
loadPanelSizes();

// ── Sidebar section heights (TEAMS / SCRATCHES) ─────────────────
// The collapsible lower sidebar sections get a draggable divider on top so
// the user can trade vertical space between them and the session list (which
// flexes to absorb the difference). Heights persist alongside panel widths.

const MIN_SECTION_H = 56; // section label + one row
const MIN_SESSION_H = 80; // keep the session list usable

function panelSize(key) {
  try {
    return JSON.parse(localStorage.getItem("clash-panel-sizes") || "{}")[key];
  } catch (e) {
    void e;
    return undefined;
  }
}

/// Apply a section's persisted height when it's expanded (clamped so the
/// session list keeps a usable minimum), or clear it and hide the divider when
/// collapsed. With no saved height the section keeps its default content-sized,
/// CSS-capped look. Re-run on toggle and on window resize (to re-clamp).
function applySectionHeight(sectionId, resizerId, open, key) {
  const section = $(sectionId);
  const resizer = $(resizerId);
  if (!open) {
    section.style.height = "";
    section.style.maxHeight = "";
    resizer.classList.add("hidden");
    return;
  }
  resizer.classList.remove("hidden");
  const want = panelSize(key);
  if (!want) {
    // No saved height: keep the default content-sized, 35%-capped look.
    section.style.height = "";
    section.style.maxHeight = "";
    return;
  }
  // Sidebar hidden (⌘B collapse): keep the current inline height as-is —
  // re-clamping against a zero-height layout would wrongly shrink it.
  if ($("sidebar").offsetHeight === 0) return;
  // Reset before measuring so the clamp reads the true available give.
  section.style.height = "";
  section.style.maxHeight = "";
  const give = Math.max(0, $("session-list").offsetHeight - MIN_SESSION_H);
  const h = Math.max(MIN_SECTION_H, Math.min(section.offsetHeight + give, want));
  section.style.height = h + "px";
  section.style.maxHeight = "none";
}

function reapplySectionHeights() {
  applySectionHeight("teams-section", "teams-resizer", state.teamsOpen, "teamsHeight");
  applySectionHeight("notes-section", "notes-resizer", state.notesOpen, "notesHeight");
}

/// Drag the divider to set the section height. The session list (the only
/// flexing item above) gives up its space, so the amount available to grow is
/// fixed at mousedown — a simple delta model, no live anchor tracking.
function initSectionResizer(handleId, sectionId, key) {
  const handle = $(handleId);
  const section = $(sectionId);
  handle.addEventListener("mousedown", (e) => {
    e.preventDefault();
    handle.classList.add("dragging");
    document.body.style.cursor = "row-resize";
    const startY = e.clientY;
    const startH = section.offsetHeight;
    const maxH = startH + Math.max(0, $("session-list").offsetHeight - MIN_SESSION_H);
    section.style.maxHeight = "none"; // allow growth past the CSS cap
    const onMove = (ev) => {
      const h = Math.max(MIN_SECTION_H, Math.min(maxH, startH + (startY - ev.clientY)));
      section.style.height = h + "px";
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      handle.classList.remove("dragging");
      document.body.style.cursor = "";
      savePanelSize(key, section.offsetHeight);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  });
}

initSectionResizer("teams-resizer", "teams-section", "teamsHeight");
initSectionResizer("notes-resizer", "notes-section", "notesHeight");
reapplySectionHeights();
window.addEventListener("resize", reapplySectionHeights);

$("new-ws-btn").onclick = newWorkspace;

// ── Settings (sidebar footer) ───────────────────────────────────

$("default-cwd").addEventListener("change", () => {
  state.settings.defaultCwd = $("default-cwd").value.trim();
  saveWorkspaces();
});

/// Scratch directory lives in the shared config.toml (not gui-state) so the
/// TUI sees it too — persisted via the backend, which echoes back the
/// resolved absolute path. An empty value resets to the default.
$("set-scratch-dir").addEventListener("change", async () => {
  const el = $("set-scratch-dir");
  try {
    el.value = await invoke("set_scratch_dir", { path: el.value.trim() });
    if (state.notesOpen) await refreshNotes();
  } catch (e) {
    uiAlert(`Scratch directory: ${e}`);
    try {
      el.value = await invoke("get_scratch_dir");
    } catch (_) {}
  }
});

/// Reflect the current scratch directory (from config) into the field at boot.
async function loadScratchDir() {
  try {
    $("set-scratch-dir").value = await invoke("get_scratch_dir");
  } catch (e) {
    console.error("get_scratch_dir failed:", e);
  }
}

/// Reflect persisted settings into the footer controls.
function syncSettingsUi() {
  $("set-fontsize").value = state.settings.fontSize;
  $("set-fontfamily").value = state.settings.fontFamily;
  $("set-scrollback").value = state.settings.scrollback;
  $("set-cursor-style").value = state.settings.cursorStyle;
  $("set-cursor-blink").checked = state.settings.cursorBlink;
  $("set-copy-select").checked = state.settings.copyOnSelect;
  $("set-option-meta").checked = state.settings.optionMeta;
  $("set-link-open").value = state.settings.linkOpen;
  $("set-notify").checked = state.settings.notifications;
}

/// Live-apply an xterm option to every open terminal, then persist.
function applyTermOption(key, value) {
  for (const entry of state.open.values()) {
    if (entry.term) entry.term.options[key] = value;
  }
  saveWorkspaces();
}

$("set-fontsize").addEventListener("change", () => {
  const v = Math.round(Number($("set-fontsize").value));
  if (!Number.isFinite(v) || v < 9 || v > 24) {
    $("set-fontsize").value = state.settings.fontSize;
    return;
  }
  state.settings.fontSize = v;
  // Live-apply to every open terminal; refit so cols/rows track the metrics.
  for (const entry of state.open.values()) {
    if (entry.term) entry.term.options.fontSize = v;
  }
  fitAll();
  saveWorkspaces();
});

$("set-fontfamily").addEventListener("change", () => {
  const v = $("set-fontfamily").value.trim();
  if (!v) {
    $("set-fontfamily").value = state.settings.fontFamily;
    return;
  }
  state.settings.fontFamily = v;
  applyTermOption("fontFamily", v);
  fitAll();
});

$("set-scrollback").addEventListener("change", () => {
  const v = Math.round(Number($("set-scrollback").value));
  if (!Number.isFinite(v) || v < 0 || v > 200000) {
    $("set-scrollback").value = state.settings.scrollback;
    return;
  }
  state.settings.scrollback = v;
  applyTermOption("scrollback", v);
});

$("set-cursor-style").addEventListener("change", () => {
  state.settings.cursorStyle = $("set-cursor-style").value;
  applyTermOption("cursorStyle", state.settings.cursorStyle);
});

$("set-cursor-blink").addEventListener("change", () => {
  state.settings.cursorBlink = $("set-cursor-blink").checked;
  applyTermOption("cursorBlink", state.settings.cursorBlink);
});

$("set-copy-select").addEventListener("change", () => {
  state.settings.copyOnSelect = $("set-copy-select").checked;
  saveWorkspaces();
});

$("set-option-meta").addEventListener("change", () => {
  state.settings.optionMeta = $("set-option-meta").checked;
  applyTermOption("macOptionIsMeta", state.settings.optionMeta);
});

$("set-link-open").addEventListener("change", () => {
  state.settings.linkOpen = $("set-link-open").value;
  saveWorkspaces();
});

$("set-notify").addEventListener("change", () => {
  state.settings.notifications = $("set-notify").checked;
  invoke("set_notifications_enabled", { enabled: state.settings.notifications }).catch(console.error);
  saveWorkspaces();
});

// ── TUI launcher (sidebar header) ───────────────────────────────
// Gold when a clash TUI process is running somewhere, grey when not.
// Click opens a picker of terminals detected on the OS (plus Auto);
// the choice is remembered as the menu's "last used" marker.

let detectedTerminals = []; // populated at boot from list_terminals

async function refreshTuiIndicator() {
  try {
    const on = await invoke("tui_running");
    $("tui-btn").classList.toggle("on", !!on);
    const tip = on
      ? "clash TUI is running — click to open another"
      : "Launch the clash TUI in a terminal";
    $("tui-btn").title = tip;
    $("tui-btn").dataset.tip = tip;
  } catch (e) {
    void e;
  }
}

async function launchTui(terminalId) {
  state.settings.tuiTerminal = terminalId;
  saveWorkspaces();
  try {
    await invoke("launch_tui", { terminal: terminalId || null });
  } catch (e) {
    uiAlert(`Launch TUI failed: ${e}`);
  }
  setTimeout(refreshTuiIndicator, 1500);
}

$("tui-btn").onclick = (ev) => {
  ev.stopPropagation(); // the same click would bubble to hideContextMenu
  const r = $("tui-btn").getBoundingClientRect();
  const last = state.settings.tuiTerminal || "";
  showContextMenu(r.left, r.bottom + 4, [
    {
      label: "Auto — split pane or default terminal",
      icon: "columns",
      hint: last === "" ? "last used" : "",
      action: () => launchTui(""),
    },
    ...(detectedTerminals.length ? [null] : []),
    ...detectedTerminals.map((t) => ({
      label: t.name,
      icon: "terminal",
      hint: last === t.id ? "last used" : "",
      action: () => launchTui(t.id),
    })),
  ]);
};

// ── In-app shell terminals (topbar) ─────────────────────────────
// Full terminals inside the GUI: a daemon PTY running a login shell,
// rendered like any session pane. The picker lists the machine's shells
// (/etc/shells + $SHELL); the last choice is remembered.

let detectedShells = []; // populated at boot from list_shells

async function openShellTerminal(shell) {
  state.settings.termShell = shell;
  saveWorkspaces();
  // Open where you're working: focused session's project, then the
  // configured default directory, then home (backend fallback).
  const cur = state.sessions.find((x) => x.id === state.activeTab);
  const cwd =
    (cur && (cur.cwd || cur.project_path)) || state.settings.defaultCwd || null;
  try {
    const sid = await invoke("create_terminal", {
      shell: shell || null,
      cwd,
      cols: 120,
      rows: 40,
    });
    const base = (shell || detectedShells[0] || "shell").split("/").pop();
    await openSession(sid, `$ ${base}`);
  } catch (e) {
    uiAlert(`New terminal failed: ${e}`);
  }
}

/// Unified new-tab menu: a terminal (per detected shell), a browser tab,
/// or a Claude session — everything a pane can hold, in one place.
function showNewTabMenu(x, y) {
  const last = state.settings.termShell || "";
  showContextMenu(x, y, [
    ...detectedShells.map((sh) => ({
      label: sh,
      icon: "terminal",
      hint: last === sh ? "last used" : "",
      action: () => openShellTerminal(sh),
    })),
    ...(detectedShells.length
      ? []
      : [{ label: "Default shell", icon: "terminal", action: () => openShellTerminal("") }]),
    null,
    {
      label: "New browser tab",
      icon: "external-link",
      hint: "⌘⇧B",
      action: () => openBrowserTab(),
    },
    null,
    {
      label: "New Claude session…",
      icon: "plus",
      hint: "⌘T",
      action: showNewSessionModal,
    },
  ]);
}

$("new-term-btn").onclick = (ev) => {
  ev.stopPropagation(); // the same click would bubble to hideContextMenu
  const r = $("new-term-btn").getBoundingClientRect();
  showNewTabMenu(r.left, r.bottom + 4);
};

// ── Icon button hover labels ────────────────────────────────────
// Instant tooltip for .icon-btn, replacing the native title tooltip
// (slow and unreliable in WKWebView). Delegated so dynamically created
// buttons (kill-all, browser new-tab) are covered. The label is moved
// from title to data-tip on first hover to suppress the native one.

const iconTip = document.createElement("div");
iconTip.id = "icon-tip";

document.addEventListener("mouseover", (e) => {
  const btn = e.target.closest?.(".icon-btn");
  if (!btn) return;
  if (btn.title) {
    btn.dataset.tip = btn.title;
    btn.removeAttribute("title");
  }
  const tip = btn.dataset.tip;
  if (!tip) return;
  iconTip.textContent = tip;
  document.body.appendChild(iconTip);
  const b = btn.getBoundingClientRect();
  const t = iconTip.getBoundingClientRect();
  let left = Math.min(Math.max(4, b.left + b.width / 2 - t.width / 2), window.innerWidth - t.width - 4);
  let top = b.bottom + 6;
  // Flip above when the label would fall off-screen or over an embedded
  // browser webview (native child webviews cover in-app DOM).
  let overlapsSlot = false;
  for (const s of document.querySelectorAll(".browser-pane .b-slot")) {
    const sr = s.getBoundingClientRect();
    if (sr.width <= 0 || sr.height <= 0) continue;
    if (top + t.height > sr.top && top < sr.bottom && left + t.width > sr.left && left < sr.right) {
      overlapsSlot = true;
      break;
    }
  }
  if (top + t.height > window.innerHeight - 4 || overlapsSlot) top = b.top - t.height - 6;
  iconTip.style.left = `${left}px`;
  iconTip.style.top = `${top}px`;
});

document.addEventListener("mouseout", (e) => {
  if (e.target.closest?.(".icon-btn")) iconTip.remove();
});
// Buttons often re-render the DOM under the cursor, which can swallow
// the mouseout — drop the label on any click.
document.addEventListener("click", () => iconTip.remove(), true);

// ── Boot ────────────────────────────────────────────────────────

(async () => {
  applyStaticIcons(); // before first paint — never show the unicode fallbacks
  await loadWorkspaces(); // disk-backed — must complete before first render
  restoreBrowserTabs(); // entries only — webviews materialize on first visibility
  $("default-cwd").value = state.settings.defaultCwd;
  syncSettingsUi();
  loadScratchDir();
  if (!state.settings.notifications) {
    invoke("set_notifications_enabled", { enabled: false }).catch(console.error);
  }
  state.homeDir = await invoke("get_home_dir").catch(() => "");
  if (state.homeDir) $("default-cwd").placeholder = state.homeDir;
  renderAll();
  setVersionLabel();
  refreshTuiIndicator();
  setInterval(refreshTuiIndicator, 5000);
  invoke("list_terminals")
    .then((t) => (detectedTerminals = t))
    .catch(() => {});
  invoke("list_shells")
    .then((s) => (detectedShells = s))
    .catch(() => {});
  await refreshSessions();
  await restoreWorkspaceSessions();
  setInterval(refreshSessions, 2000);
})();
