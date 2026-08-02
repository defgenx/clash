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
  workflows: [], // workflow items (plan → review → implement → PR pipeline)
  wfOpen: false,
  wfUnread: new Set(), // "project/slug" keys with unseen attention events
  wfDoneOpen: false, // DONE/ABANDONED sidebar group expanded
  renaming: null, // session id with an open inline-rename input
  prevStatuses: new Map(), // session id -> status (attention transitions)
  unread: new Set(), // session ids with unseen attention events
  missingStreak: new Map(), // session id -> consecutive refreshes absent (ownership prune)
  // Persisted with workspaces in gui-state.json. optionMeta: ⌥ sends
  // Esc (Meta) in terminals; off = ⌥ always composes characters.
  settings: {
    theme: "clash-dark", // key into THEMES — drives the chrome and the terminals
    defaultCwd: "",
    fontSize: 13,
    fontFamily: "SF Mono, Menlo, monospace",
    fontWeight: "normal", // 300 | normal | 500 | 600 | bold
    fontWeightBold: "bold",
    lineHeight: 1,
    letterSpacing: 0,
    scrollback: 10000,
    cursorStyle: "block", // block | bar | underline
    cursorInactiveStyle: "outline", // outline | block | bar | underline | none
    cursorWidth: 1, // bar-cursor thickness in px
    cursorBlink: false,
    minimumContrast: 1, // 1 = off; xterm's minimumContrastRatio
    brightBold: false, // draw bold text in bright colors
    scrollSpeed: 1, // lines per wheel notch (xterm scrollSensitivity)
    smoothScroll: 0, // ms; 0 = instant
    copyOnSelect: false,
    rightClickWord: true, // right-click selects the word under the pointer
    optionMeta: true,
    bellToast: false, // surface a terminal bell as an in-app toast
    linkOpen: "ask", // ask | embedded | external — how terminal links open
    notifications: true,
    titleAttention: true, // "clash (2!)" window title when sessions need input
    confirmKill: true, // confirm dialog before a kill (never for stash)
    refreshSecs: 2, // session-list poll cadence
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
  search: '<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>',
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
    "new-wf-btn": "plus",
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

function uiDialog({
  message,
  input = null,
  okLabel = "OK",
  cancelable = true,
  danger = false,
  multiline = false,
  browse = false,
}) {
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
      // A pasted plan is many lines: same dialog, textarea instead of input,
      // and Enter types a newline (⌘/Ctrl+Enter submits — see keydown below).
      field = document.createElement(multiline ? "textarea" : "input");
      if (!multiline) field.type = "text";
      else field.rows = 14;
      field.value = input;
      field.spellcheck = false;
      if (browse && !multiline) {
        // Path prompts get the same native folder picker as the new-session
        // modal's cwd field — typing an absolute path from memory is the
        // fallback, not the only way in.
        const row = document.createElement("div");
        row.className = "input-with-btn";
        row.appendChild(field);
        const pick = document.createElement("button");
        pick.type = "button";
        pick.className = "icon-btn";
        pick.title = "Browse for a directory";
        pick.innerHTML = svgIcon("folder", 14);
        pick.onclick = async () => {
          const dir = await pickDirectory(field.value);
          if (dir) field.value = dir;
          field.focus();
        };
        row.appendChild(pick);
        box.appendChild(row);
      } else {
        box.appendChild(field);
      }
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
      if (e.key === "Enter" && (!multiline || e.metaKey || e.ctrlKey))
        done(input !== null ? field.value : true);
      else if (e.key === "Escape" && cancelable) done(cancelValue);
    });
    setTimeout(() => (field || ok).focus(), 0);
  });
}

const uiConfirm = (message, okLabel = "Confirm") =>
  uiDialog({ message, okLabel, danger: true });
/// Confirm a single-session kill, unless the user turned that prompt off in
/// Settings. Batch kills (`massKill`) always ask — one click taking out several
/// sessions is a different order of mistake.
const confirmKillSession = (message, okLabel = "Kill") =>
  state.settings.confirmKill ? uiConfirm(message, okLabel) : Promise.resolve(true);
const uiPrompt = (message, def = "") => uiDialog({ message, input: def });
/// Prompt for a directory: text field plus the native folder picker.
const uiPathPrompt = (message, def = "") =>
  uiDialog({ message, input: def, browse: true });
const uiAlert = (message) => uiDialog({ message, cancelable: false });
/// Multi-line prompt for text that doesn't fit one line (a pasted plan).
const uiTextPrompt = (message, def = "", okLabel = "OK") =>
  uiDialog({ message, input: def, multiline: true, okLabel });

/// A modal that picks one entry from a (possibly long) list: scrollable
/// rows with a label and an optional dim detail line. Resolves to the
/// picked value, or null when cancelled. Use this instead of uiChoice when
/// the options are data (repos, branches…) rather than 2-3 actions.
function uiListChoice({ message, items }) {
  return new Promise((resolve) => {
    const backdrop = document.createElement("div");
    backdrop.className = "dialog-backdrop";
    const box = document.createElement("div");
    box.className = "dialog-box";
    const msg = document.createElement("p");
    msg.textContent = message;
    box.appendChild(msg);
    const done = (val) => {
      backdrop.remove();
      resolve(val);
      if (typeof fitAll === "function") fitAll();
    };
    const list = document.createElement("div");
    list.className = "dialog-list";
    for (const it of items) {
      const row = document.createElement("div");
      row.className = "dialog-list-row";
      const label = document.createElement("div");
      label.className = "dialog-list-label";
      label.textContent = it.label;
      row.appendChild(label);
      if (it.detail) {
        const d = document.createElement("div");
        d.className = "dialog-list-detail";
        d.textContent = it.detail;
        row.appendChild(d);
      }
      row.onclick = () => done(it.value);
      list.appendChild(row);
    }
    box.appendChild(list);
    const actions = document.createElement("div");
    actions.className = "modal-actions";
    const cancel = document.createElement("button");
    cancel.textContent = "Cancel";
    cancel.onclick = () => done(null);
    actions.appendChild(cancel);
    box.appendChild(actions);
    backdrop.appendChild(box);
    if (typeof hideBrowserWebviews === "function") hideBrowserWebviews();
    document.body.appendChild(backdrop);
    backdrop.onclick = (e) => {
      if (e.target === backdrop) done(null);
    };
    backdrop.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Escape") done(null);
    });
    setTimeout(() => cancel.focus(), 0);
  });
}

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
  const viewTabs = [];
  for (const [id, e] of state.open) {
    if (e.kind === "browser") {
      browserTabs.push({ id, url: e.url, name: e.name, renamed: !!e.renamed });
    } else if (
      id === "view:wfboard" ||
      id === "view:skills" ||
      id.startsWith("view:workflow:")
    ) {
      // Workflow tabs are recreatable from their key alone (files on disk);
      // persist the name + active sub-view so restore lands where we were.
      const ts = id.startsWith("view:workflow:")
        ? wfTabState.get(id.slice("view:workflow:".length))
        : null;
      viewTabs.push({ id, name: e.name, subView: ts ? ts.subView : null });
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
    viewTabs,
    active: state.activeWs,
    // Only the GUI-local settings. The cross-frontend ones live in config.toml
    // from the migration onward; keeping a copy here would let this blob — or
    // WKWebView's localStorage, which is not HOME-isolated — quietly overwrite
    // a hand-edited config.toml on the next boot (plan Finding 5).
    settings: guiLocalSettings(),
  });
}

/// The settings blob minus everything that now lives in config.toml.
function guiLocalSettings() {
  // Until loadSettings() resolves, `state.settings` still holds the in-code
  // defaults — the persisted values are parked, not applied. A flush in that
  // window (blur/pagehide fire on any click away, and they run before boot
  // finishes) would write defaults into the blob, and the next boot would
  // migrate those defaults straight over the user's real config.toml values.
  // Echo back what we read instead, verbatim.
  if (!settingsResolved) return loadedSettingsBlob || {};
  const out = {};
  for (const [key, value] of Object.entries(state.settings)) {
    if (!sharedSettingKeys.has(key)) out[key] = value;
  }
  return out;
}

/// Write the workspace/layout state to disk *now*, bypassing the debounce.
/// Called when clash loses focus / is hidden / is closing so the latest
/// "where we were" is never lost to a pending debounce timer. Returns the
/// disk-write promise so callers that must guarantee the write lands before
/// something drastic (e.g. an update re-exec) can await it; the always-.catch
/// means the promise resolves even on failure. Event-listener callers ignore
/// the return and stay fire-and-forget.
function flushWorkspaces() {
  clearTimeout(saveTimer);
  const json = workspacesJson();
  try {
    localStorage.setItem("clash-workspaces", json);
  } catch (e) {
    void e;
  }
  return invoke("save_gui_state", { stateJson: json }).catch(() => {});
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

/// The raw settings blob from the persisted state, awaiting validation.
///
/// Validation and migration happen in Rust (`config_migrate_gui_blob`), so the
/// range checks and legacy fixups exist once and the TUI can reuse them — this
/// used to be ~50 lines of hand-written per-key checks that duplicated the
/// schema's own constraints. `applyWorkspacesData` is synchronous and runs
/// before the first paint, so it parks the blob here and `loadSettings()`
/// awaits the backend.
let pendingSettingsBlob = null;
/// The same blob, kept after `loadSettings()` consumes it, so a flush that races
/// boot can echo it back instead of persisting unresolved defaults.
let loadedSettingsBlob = null;
/// Whether the parked blob may seed the one-shot migration into config.toml.
/// False for anything read from localStorage — see `loadWorkspaces`.
let pendingSettingsMigratable = false;
/// Set once `loadSettings()` has applied real values to `state.settings`.
let settingsResolved = false;
/// Setting keys that live in config.toml rather than gui-state.json. Filled
/// from the backend (the schema is the source of truth), so this never drifts.
let sharedSettingKeys = new Set();

function applyWorkspacesData(data, { migratable = false } = {}) {
  if (!data) return false;
  // Settings ride along with the workspaces blob but load independently — a
  // fresh install with no workspaces yet still gets its saved settings. First
  // source wins: loadWorkspaces tries disk before the localStorage fallback,
  // and the fallback must not override what disk already provided.
  if (data.settings && typeof data.settings === "object" && !pendingSettingsBlob) {
    pendingSettingsBlob = data.settings;
    loadedSettingsBlob = data.settings;
    pendingSettingsMigratable = migratable;
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
  pendingWorkflowTabs = Array.isArray(data.viewTabs) ? data.viewTabs : [];
  state.activeWs = Math.min(data.active || 0, state.workspaces.length - 1);
  return true;
}

let pendingWorkflowTabs = []; // persisted workflow tabs awaiting restore at boot

async function loadWorkspaces() {
  try {
    const raw = await invoke("load_gui_state");
    // `migratable`: only the disk blob may seed the one-shot settings migration.
    // WKWebView's localStorage resolves the real user home even when clash runs
    // under an isolated HOME, so a stale copy there must never be allowed to
    // write into config.toml (plan Finding 5).
    if (raw && applyWorkspacesData(JSON.parse(raw), { migratable: true })) return;
  } catch (e) {
    console.error("load_gui_state failed:", e);
  }
  try {
    const raw = localStorage.getItem("clash-workspaces");
    if (raw) applyWorkspacesData(JSON.parse(raw), { migratable: false });
  } catch (e) {
    console.error("loadWorkspaces (localStorage) failed:", e);
  }
}

/// Resolve settings through the backend, then apply them to `state.settings`.
///
/// Two halves, one schema: the cross-frontend keys come back from config.toml,
/// the GUI-local ones come back validated and normalised from the same Rust
/// property table (including the legacy `embedLinks` → `linkOpen` fixup). Must
/// be awaited before the first paint — `applyTheme` and `syncSettingsUi` both
/// read `state.settings`.
async function loadSettings() {
  const blob = pendingSettingsBlob;
  const migratable = pendingSettingsMigratable;
  pendingSettingsBlob = null;
  pendingSettingsMigratable = false;

  let result = null;
  try {
    result = migratable
      ? await invoke("config_migrate_gui_blob", { blob: blob || {} })
      : await invoke("config_get");
  } catch (e) {
    console.error("loading settings failed:", e);
  }
  if (!result) {
    // A backend that can't answer must not leave the window unstyled; the
    // in-code defaults already in state.settings stand in. Deliberately *not*
    // marked resolved: with no answer we don't know which keys are shared, so
    // the blob keeps being echoed back rather than rewritten from a guess.
    if (blob) Object.assign(state.settings, blob);
    return;
  }

  sharedSettingKeys = new Set(
    Object.keys(result.settings || {}).filter((k) => k in state.settings)
  );
  // A non-migratable (localStorage) blob still supplies the GUI-local half.
  if (blob && !migratable) {
    for (const [key, value] of Object.entries(blob)) {
      if (!sharedSettingKeys.has(key) && key in state.settings) state.settings[key] = value;
    }
  }
  Object.assign(state.settings, result.guiLocal || {});
  Object.assign(state.settings, result.settings || {});
  settingsResolved = true;

  for (const warning of result.warnings || []) dlog(`settings: ${warning}`);
  for (const issue of result.issues || []) dlog(`config: ${issue}`);
  if (result.error) showConfigError(result.error);
  if (result.applied && result.applied.length) {
    dlog(`settings migrated into config.toml: ${result.applied.join(", ")}`);
    // Rewrite the blob without the migrated keys straight away, so even one
    // crash later they cannot come back and overwrite config.toml.
    await flushWorkspaces();
  }
}

/// Persist one setting to wherever it lives.
///
/// Cross-frontend settings round-trip through config.toml (so the TUI agrees and
/// the schema validates them); GUI-local ones stay in gui-state.json.
function persistSetting(key) {
  if (!sharedSettingKeys.has(key)) {
    saveWorkspaces();
    return;
  }
  invoke("config_set", { key, value: state.settings[key] }).catch((e) => {
    console.error(`config_set ${key} failed:`, e);
    flashToast(`Could not save ${key}: ${e}`);
  });
}

/// Restore sessions referenced by saved workspace panes. Running sessions
/// re-attach immediately; stashed sessions reopen as deferred tabs that
/// resume (claude --resume) only when first focused/clicked. Sessions that
/// no longer exist on disk are cleared from their slots.
async function restoreWorkspaceSessions() {
  // A persisted id can be stale in two ways: `/clear` re-keyed the registry,
  // and `claude --resume` forks the conversation into a new transcript (healed
  // into the registry by `heal_registry_forks` at backend startup). Either way
  // list_sessions now reports a DIFFERENT id than we saved. Resolve every
  // persisted id forward — panes AND workspace ownership, not just panes: an
  // owned-but-not-open session whose id moved would otherwise be pruned from
  // `w.sessions` while its current id, owned by nobody, surfaces under
  // UNASSIGNED on every relaunch. Unknown ids pass through unchanged.
  const saved = persistedSessionIds(state.workspaces, isRealSessionId);
  if (saved.length) {
    try {
      const resolved = await invoke("resolve_session_ids", { ids: saved });
      const remap = new Map();
      saved.forEach((id, i) => {
        if (resolved[i] && resolved[i] !== id) remap.set(id, resolved[i]);
      });
      if (remapWorkspaceIds(state.workspaces, remap)) saveWorkspaces();
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
      if (sid.startsWith("view:")) {
        // Workflow tabs are rebuilt from disk state; other view tabs
        // (conversation / subagents / session diff) keep the historical
        // drop-on-restart behavior.
        if (
          sid === "view:wfboard" ||
          sid === "view:skills" ||
          sid.startsWith("view:workflow:")
        ) {
          state.activeWs = wi;
          w.focused = pi;
          restoreWorkflowTab(sid, pendingWorkflowTabs.find((t) => t && t.id === sid));
        } else {
          w.panes[pi] = null;
        }
        continue;
      }
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
  pendingWorkflowTabs = [];
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
        if (!(await confirmKillSession(`Kill session "${displayName(s)}"?`))) return;
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
    document.title =
      attention > 0 && state.settings.titleAttention ? `clash (${attention}!)` : "clash";
    state.sessions = sessions;

    // Prune workspace ownership of sessions gone from the list for 3
    // consecutive refreshes (killed/removed) — tolerates transient
    // daemon hiccups without orphaning the workspace's session list.
    const known = new Set(sessions.map((s) => s.id));
    const vanished = new Set();
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
        if (streak >= 3) vanished.add(id);
      }
    }
    // A vanished id is not necessarily a dead session: a `/clear` (hook re-key)
    // or a resume fork moves the conversation to a NEW id mid-run, and the old
    // one simply stops being listed. Resolve each forward before dropping it —
    // if it moved, transfer ownership to the new id so the session stays in its
    // workspace instead of resurfacing under UNASSIGNED. Rare, so the extra
    // round-trip costs nothing on a normal tick. A failed resolve defers the
    // whole prune to the next tick rather than guessing "dead" — dropping
    // ownership is the one outcome we can't undo.
    if (vanished.size) {
      const ids = [...vanished];
      try {
        const resolved = await invoke("resolve_session_ids", { ids });
        const owned = state.workspaces.flatMap((w) => w.sessions);
        const moved = ownershipTransfers(ids, resolved, known, owned);
        if (pruneOwnership(state.workspaces, vanished, moved)) saveWorkspaces();
        for (const id of ids) state.missingStreak.delete(id);
      } catch (e) {
        console.error("resolve_session_ids failed:", e);
      }
    }

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
        if (!(await confirmKillSession(`Kill session "${label}"?`))) return;
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

/// True for a real Claude conversation id — i.e. an id that `list_sessions`
/// can report and that the registry can resolve forward. Excludes every
/// synthetic pane/tab key clash also persists: browser tabs, shell terminals
/// and view tabs (workflow board, skills, per-session views).
function isRealSessionId(id) {
  return !!id && !isBrowserTab(id) && !isShellTerm(id) && !id.startsWith("view:");
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

// ── Themes ──────────────────────────────────────────────────────
// A theme drives the CSS custom properties in style.css *and* the xterm
// palette, so the chrome and the terminals always agree. Only the colours
// below are per-theme; everything derivable is derived (see applyTheme):
// the status palette from the semantic colours, `on-accent` from the
// accent's luminance, and the 8 bright ANSI slots from the 8 base ones.
//
// `ansi` is [black, red, green, yellow, blue, magenta, cyan, white]. Omit it
// to keep xterm's built-in palette (clash dark does, so its terminals look
// exactly as they always have).

const THEMES = {
  "clash-dark": {
    label: "clash dark",
    dark: true,
    ui: {
      bg: "#141414",
      "bg-sidebar": "#1b1b1d",
      "bg-hover": "#242428",
      "bg-active": "#2c2c31",
      "bg-raised": "#1f1f23",
      border: "#2a2a2e",
      fg: "#d4d4d8",
      "fg-dim": "#76767e",
      accent: "#e8a33d",
      green: "#4ec975",
      red: "#e5534b",
      blue: "#539bf5",
      purple: "#b083f0",
      // The one theme with a hand-tuned status palette (it mirrors the TUI's
      // theme.rs); every other theme derives these from its semantic colours.
      "st-running": "#82c396",
      "st-thinking": "#91afd2",
      "st-waiting": "#d2be78",
      "st-starting": "#a591d7",
      "st-prompting": "#d29187",
      "st-idle": "#5f5873",
      "st-error": "#c88287",
    },
    selection: "#3a3a40",
  },
  "clash-light": {
    label: "clash light",
    dark: false,
    ui: {
      bg: "#fbfaf8",
      "bg-sidebar": "#f3f1ed",
      "bg-hover": "#e9e5df",
      "bg-active": "#ded9d1",
      "bg-raised": "#ffffff",
      border: "#dcd6cd",
      fg: "#26241f",
      "fg-dim": "#6e6960",
      accent: "#b0741a",
      green: "#17803d",
      red: "#c02b28",
      blue: "#1f6feb",
      purple: "#7b3fd4",
    },
    ansi: ["#3b3833", "#c02b28", "#17803d", "#96700f", "#1f6feb", "#7b3fd4", "#0f7c8c", "#e9e5df"],
  },
  "solarized-dark": {
    label: "Solarized Dark",
    dark: true,
    ui: {
      bg: "#002b36",
      "bg-sidebar": "#01222b",
      "bg-hover": "#073642",
      "bg-active": "#0b4a59",
      "bg-raised": "#073642",
      border: "#0d4b59",
      fg: "#93a1a1",
      "fg-dim": "#657b83",
      accent: "#b58900",
      green: "#859900",
      red: "#dc322f",
      blue: "#268bd2",
      purple: "#6c71c4",
    },
    ansi: ["#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5"],
  },
  "solarized-light": {
    label: "Solarized Light",
    dark: false,
    ui: {
      bg: "#fdf6e3",
      "bg-sidebar": "#f4edda",
      "bg-hover": "#eee8d5",
      "bg-active": "#ded8c4",
      "bg-raised": "#fffdf6",
      border: "#ded8c6",
      fg: "#586e75",
      "fg-dim": "#93a1a1",
      accent: "#b58900",
      green: "#859900",
      red: "#dc322f",
      blue: "#268bd2",
      purple: "#6c71c4",
    },
    ansi: ["#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5"],
  },
  nord: {
    label: "Nord",
    dark: true,
    ui: {
      bg: "#2e3440",
      "bg-sidebar": "#292e39",
      "bg-hover": "#3b4252",
      "bg-active": "#434c5e",
      "bg-raised": "#3b4252",
      border: "#3b4252",
      fg: "#d8dee9",
      "fg-dim": "#7b88a1",
      accent: "#88c0d0",
      green: "#a3be8c",
      red: "#bf616a",
      blue: "#81a1c1",
      purple: "#b48ead",
    },
    ansi: ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0"],
  },
  "tokyo-night": {
    label: "Tokyo Night",
    dark: true,
    ui: {
      bg: "#1a1b26",
      "bg-sidebar": "#16161e",
      "bg-hover": "#24283b",
      "bg-active": "#2f334d",
      "bg-raised": "#1f2335",
      border: "#292e42",
      fg: "#c0caf5",
      "fg-dim": "#6b7396",
      accent: "#7aa2f7",
      green: "#9ece6a",
      red: "#f7768e",
      blue: "#7dcfff",
      purple: "#bb9af7",
    },
    ansi: ["#32344a", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5"],
  },
  "catppuccin-mocha": {
    label: "Catppuccin Mocha",
    dark: true,
    ui: {
      bg: "#1e1e2e",
      "bg-sidebar": "#181825",
      "bg-hover": "#313244",
      "bg-active": "#45475a",
      "bg-raised": "#232334",
      border: "#313244",
      fg: "#cdd6f4",
      "fg-dim": "#9399b2",
      accent: "#cba6f7",
      green: "#a6e3a1",
      red: "#f38ba8",
      blue: "#89b4fa",
      purple: "#cba6f7",
    },
    ansi: ["#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de"],
  },
  "catppuccin-latte": {
    label: "Catppuccin Latte",
    dark: false,
    ui: {
      bg: "#eff1f5",
      "bg-sidebar": "#e6e9ef",
      "bg-hover": "#dce0e8",
      "bg-active": "#ccd0da",
      "bg-raised": "#ffffff",
      border: "#ccd0da",
      fg: "#4c4f69",
      "fg-dim": "#7c7f93",
      accent: "#8839ef",
      green: "#40a02b",
      red: "#d20f39",
      blue: "#1e66f5",
      purple: "#8839ef",
    },
    ansi: ["#5c5f77", "#d20f39", "#40a02b", "#df8e1d", "#1e66f5", "#ea76cb", "#179299", "#acb0be"],
  },
  "gruvbox-dark": {
    label: "Gruvbox Dark",
    dark: true,
    ui: {
      bg: "#282828",
      "bg-sidebar": "#1d2021",
      "bg-hover": "#3c3836",
      "bg-active": "#504945",
      "bg-raised": "#32302f",
      border: "#3c3836",
      fg: "#ebdbb2",
      "fg-dim": "#a89984",
      accent: "#fabd2f",
      green: "#b8bb26",
      red: "#fb4934",
      blue: "#83a598",
      purple: "#d3869b",
    },
    ansi: ["#3c3836", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2"],
  },
  dracula: {
    label: "Dracula",
    dark: true,
    ui: {
      bg: "#282a36",
      "bg-sidebar": "#21222c",
      "bg-hover": "#343746",
      "bg-active": "#44475a",
      "bg-raised": "#2f3140",
      border: "#3a3d4d",
      fg: "#f8f8f2",
      "fg-dim": "#6272a4",
      accent: "#bd93f9",
      green: "#50fa7b",
      red: "#ff5555",
      blue: "#8be9fd",
      purple: "#ff79c6",
    },
    ansi: ["#44475a", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2"],
  },
  "one-dark": {
    label: "One Dark",
    dark: true,
    ui: {
      bg: "#282c34",
      "bg-sidebar": "#21252b",
      "bg-hover": "#2c313a",
      "bg-active": "#3a3f4b",
      "bg-raised": "#2c313a",
      border: "#3a3f4b",
      fg: "#abb2bf",
      "fg-dim": "#7f848e",
      accent: "#61afef",
      green: "#98c379",
      red: "#e06c75",
      blue: "#56b6c2",
      purple: "#c678dd",
    },
    ansi: ["#3f4451", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf"],
  },
  "github-light": {
    label: "GitHub Light",
    dark: false,
    ui: {
      bg: "#ffffff",
      "bg-sidebar": "#f6f8fa",
      "bg-hover": "#eaeef2",
      "bg-active": "#dfe3e8",
      "bg-raised": "#ffffff",
      border: "#d0d7de",
      fg: "#1f2328",
      "fg-dim": "#636c76",
      accent: "#0969da",
      green: "#1a7f37",
      red: "#cf222e",
      blue: "#0969da",
      purple: "#8250df",
    },
    ansi: ["#24292f", "#cf222e", "#1a7f37", "#9a6700", "#0969da", "#8250df", "#1b7c83", "#6e7781"],
  },
};

/// Mix `hex` toward white (amount > 0) or black (amount < 0) by that fraction.
/// Used for the bright ANSI slots and hover tints — a straight linear mix, which
/// is enough for palette variants and needs no colour-space machinery.
function mixColor(hex, amount) {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return hex;
  const n = parseInt(m[1], 16);
  const target = amount > 0 ? 255 : 0;
  const t = Math.abs(amount);
  const ch = (shift) => {
    const v = (n >> shift) & 0xff;
    return Math.round(v + (target - v) * t);
  };
  return `#${[ch(16), ch(8), ch(0)].map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}

/// Perceived brightness (sRGB luma, 0-1) — decides whether text on top of a
/// colour should be near-black or near-white.
function luma(hex) {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return 0;
  const n = parseInt(m[1], 16);
  return (0.2126 * ((n >> 16) & 0xff) + 0.7152 * ((n >> 8) & 0xff) + 0.0722 * (n & 0xff)) / 255;
}

/// The xterm palette currently in force. Reassigned by applyTheme and read when
/// a terminal is constructed, so new panes match the theme already on screen.
let TERM_THEME = {
  background: "#141414",
  foreground: "#d4d4d8",
  cursor: "#e8a33d",
  selectionBackground: "#3a3a40",
};

/// Paint a theme: CSS custom properties for the chrome, `TERM_THEME` (plus a
/// live update of every open terminal) for the panes. Unknown id → the default.
function applyTheme(id) {
  const t = THEMES[id] || THEMES["clash-dark"];
  const u = t.ui;
  const root = document.documentElement;
  for (const [key, value] of Object.entries(u)) root.style.setProperty(`--${key}`, value);
  // Derived: code/menu surface, text on filled accent, and the status palette
  // (unless the theme spelled it out, as clash dark does).
  root.style.setProperty("--bg-3", u["bg-raised"]);
  root.style.setProperty("--on-accent", luma(u.accent) > 0.55 ? "#14161a" : "#ffffff");
  const derived = {
    "st-running": u.green,
    "st-thinking": u.blue,
    "st-waiting": u.accent,
    "st-starting": u.purple,
    "st-prompting": u.red,
    "st-idle": u["fg-dim"],
    "st-error": u.red,
  };
  for (const [key, value] of Object.entries(derived)) {
    if (!u[key]) root.style.setProperty(`--${key}`, value);
  }
  // A few rules can't be expressed as a colour (icon filters, image blending).
  root.classList.toggle("theme-light", !t.dark);

  TERM_THEME = {
    background: u.bg,
    foreground: u.fg,
    cursor: u.accent,
    cursorAccent: u.bg,
    selectionBackground: t.selection || mixColor(u.bg, t.dark ? 0.16 : -0.12),
  };
  if (t.ansi) {
    const names = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];
    t.ansi.forEach((color, i) => {
      TERM_THEME[names[i].toLowerCase()] = color;
      // Bright slots: lighten on dark themes, deepen on light ones, so bold
      // text stays legible instead of washing into the background.
      TERM_THEME[`bright${names[i]}`] = mixColor(color, t.dark ? 0.22 : -0.18);
    });
  }
  for (const entry of state.open.values()) {
    if (entry.term) entry.term.options.theme = TERM_THEME;
  }
}

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
    fontWeight: state.settings.fontWeight,
    fontWeightBold: state.settings.fontWeightBold,
    lineHeight: state.settings.lineHeight,
    letterSpacing: state.settings.letterSpacing,
    theme: TERM_THEME,
    scrollback: state.settings.scrollback,
    cursorStyle: state.settings.cursorStyle,
    cursorInactiveStyle: state.settings.cursorInactiveStyle,
    cursorWidth: state.settings.cursorWidth,
    cursorBlink: state.settings.cursorBlink,
    minimumContrastRatio: state.settings.minimumContrast,
    drawBoldTextInBrightColors: state.settings.brightBold,
    scrollSensitivity: state.settings.scrollSpeed,
    smoothScrollDuration: state.settings.smoothScroll,
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
    // a quick native affordance for grabbing a token to copy. Off for anyone who
    // wants right-click to stay out of the selection.
    rightClickSelectsWord: state.settings.rightClickWord,
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
  // BEL (\a) — a script or a shell prompt signalling "done / attention". xterm
  // has no audible bell, so surface it as the toast the rest of the app uses,
  // naming the session so a bell from a background pane is still actionable.
  term.onBell(() => {
    if (!state.settings.bellToast) return;
    const s = state.sessions.find((x) => x.id === sid);
    flashToast(`🔔 ${s ? displayName(s) : label || sid.slice(0, 8)}`);
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

// Render markdown into an element. DOMPurify is load-bearing, not cosmetic:
// there is no CSP and withGlobalTauri is on, so unsanitized agent-generated
// markdown (plan.md!) could reach window.__TAURI__.core.invoke via an
// injected handler. Everything markdown goes through here.
function renderMarkdown(el, md) {
  let html;
  try {
    html = marked.parse(md ?? "", { async: false });
  } catch (e) {
    el.textContent = md ?? "";
    return;
  }
  el.innerHTML = DOMPurify.sanitize(html, { FORBID_TAGS: ["style", "form"] });
  // Neutralize link navigation inside the webview: markdown links open via
  // the OS/system browser path, never by navigating the app webview.
  for (const a of el.querySelectorAll("a[href]")) {
    a.addEventListener("click", (ev) => {
      ev.preventDefault();
      const href = a.getAttribute("href") || "";
      if (/^https?:/i.test(href)) invoke("open_external", { url: href });
    });
  }
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


// ── Workflows (plan → review → implement → PR pipeline) ────────

const wfKey = (project, slug) => `${project}/${slug}`;

// Status metadata — mirrors WorkflowStatus in src/domain/workflow.rs (the
// kebab-case strings are the shared vocabulary; keep both sites in sync).
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

// Mirrors WorkflowStatus::needs_attention — the decision-needed states.
const WF_DECISION = new Set(["plan-review", "diff-review", "pr-draft"]);

// Mirrors WorkflowMode::is_review_only / has_plan_phase. An absent mode (items
// created before entry modes existed) is the full pipeline.
const wfIsReviewOnly = (item) => item.meta.mode === "review-only";
const wfHasPlanPhase = (item) => !wfIsReviewOnly(item);

// Mirrors WorkflowStatus::can_request_review — the states holding an artifact
// worth reviewing while parked on a human decision. Reviews are unbounded, so
// this gates only *where* a round can start, never how many.
const WF_REVIEWABLE = new Set(["plan-review", "diff-review", "pr-draft", "pr-ready"]);
const wfCanReview = (item) =>
  WF_REVIEWABLE.has(item.meta.status) && !!(item.meta.repoPath || "").trim();

// Mirrors ReviewTarget::for_status — a plan review only makes sense where a
// plan exists to read.
const wfReviewTarget = (item) =>
  item.meta.status === "plan-review" && wfHasPlanPhase(item) ? "plan" : "diff";

const wfHasPr = (item) => !!(item.meta.pr && item.meta.pr.url);

function wfGroup(item) {
  const st = item.meta.status;
  if (st === "done" || st === "abandoned") return "DONE";
  if (WF_DECISION.has(st)) return "NEEDS DECISION";
  if (st === "pr-ready") return "PR READY";
  return "IN FLIGHT";
}

async function toggleWorkflows() {
  state.wfOpen = !state.wfOpen;
  $("wf-caret").textContent = state.wfOpen ? "▾" : "▸";
  $("wf-list").classList.toggle("hidden", !state.wfOpen);
  applySectionHeight("wf-section", "wf-resizer", state.wfOpen, "wfHeight");
  if (state.wfOpen) await refreshWorkflows();
}

async function refreshWorkflows() {
  try {
    state.workflows = await invoke("list_workflow_items");
  } catch (e) {
    console.error("list_workflow_items failed:", e);
    state.workflows = [];
  }
  renderWorkflows();
}

function wfItem(project, slug) {
  return state.workflows.find((w) => w.project === project && w.slug === slug);
}

/// Count chip on the section label — visible even when collapsed, so a
/// pending decision is never invisible.
function updateWfBadge() {
  const badge = $("wf-badge");
  const n = state.wfUnread.size;
  badge.classList.toggle("hidden", n === 0);
  badge.textContent = n > 0 ? String(n) : "";
}

function renderWorkflows() {
  updateWfBadge();
  const list = $("wf-list");
  list.innerHTML = "";
  if (state.workflows.length === 0) {
    const empty = document.createElement("div");
    empty.className = "list-empty";
    empty.textContent = "no workflows — + to create one";
    list.appendChild(empty);
    return;
  }
  const groups = new Map([
    ["NEEDS DECISION", []],
    ["IN FLIGHT", []],
    ["PR READY", []],
    ["DONE", []],
  ]);
  for (const item of state.workflows) groups.get(wfGroup(item)).push(item);
  for (const [name, items] of groups) {
    if (!items.length) continue;
    const label = document.createElement("div");
    label.className = "wf-group-label";
    if (name === "DONE") {
      // Finished items collapse by default — history on demand.
      label.textContent = `${state.wfDoneOpen ? "▾" : "▸"} DONE (${items.length})`;
      label.classList.add("clickable");
      label.onclick = () => {
        state.wfDoneOpen = !state.wfDoneOpen;
        renderWorkflows();
      };
      list.appendChild(label);
      if (!state.wfDoneOpen) continue;
    } else {
      label.textContent = name;
      list.appendChild(label);
    }
    for (const item of items) list.appendChild(buildWorkflowRow(item));
  }
}

function buildWorkflowRow(item) {
  const key = wfKey(item.project, item.slug);
  const info = wfStatusInfo(item.meta.status);
  const row = document.createElement("div");
  // Same skeleton as session rows: status ring column + two-line meta.
  row.className = "session-item wf-item";
  const bits = [escapeHtml(item.project), `it.${item.meta.iteration || 1}`];
  // Review-only items never planned or implemented from scratch — say so, or
  // their empty plan looks like a stalled full workflow.
  if (wfIsReviewOnly(item)) bits.push("review");
  if (item.openAnnotations > 0) bits.push(`💬${item.openAnnotations}`);
  if (item.meta.pr && item.meta.pr.url) bits.push(item.meta.pr.draft ? "PR·draft" : "PR");
  const warn =
    item.agentAlive === false
      ? `<span class="wf-warn" title="agent session is gone — relaunch from the item tab">⚠</span>`
      : "";
  const unread = state.wfUnread.has(key) ? `<span class="unread-dot"></span>` : "";
  row.innerHTML =
    `<span class="status-ring wf-ring ${info.cls}" title="${info.label}"></span>` +
    `<span class="session-meta">` +
    `<span class="session-name">${escapeHtml(item.meta.title || item.slug)}${warn}${unread}</span>` +
    `<span class="session-sub"><span class="status-label ${info.cls}">${info.label}</span>` +
    `<span class="dim">${bits.join(" · ")}</span></span>` +
    `</span>`;
  row.title = `${item.meta.title || item.slug} — ${info.label}`;
  row.onclick = () => openWorkflowTab(item);
  row.oncontextmenu = (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    workflowContextMenu(item, ev.clientX, ev.clientY);
  };
  return row;
}

function workflowContextMenu(item, x, y) {
  const items = [{ label: "Open", icon: "file", action: () => openWorkflowTab(item) }];
  if (item.meta.pr && item.meta.pr.url) {
    items.push({
      label: "Copy PR URL",
      icon: "copy",
      action: async () => {
        try {
          await invoke("clipboard_write_text", { text: item.meta.pr.url });
          flashToast("PR URL copied");
        } catch (e) {
          uiAlert(`Copy failed: ${e}`);
        }
      },
    });
  }
  items.push(null);
  if (item.meta.status !== "abandoned") {
    items.push({
      label: "Abandon…",
      icon: "x",
      danger: true,
      action: async () => {
        if (!(await uiConfirm(`Abandon "${item.meta.title || item.slug}"? The files stay on disk.`)))
          return;
        try {
          await invoke("update_workflow_status", {
            project: item.project,
            slug: item.slug,
            status: "abandoned",
            note: null,
          });
          refreshWorkflows();
        } catch (e) {
          uiAlert(`Abandon failed: ${e}`);
        }
      },
    });
  }
  items.push({
    label: "Delete…",
    icon: "x",
    danger: true,
    action: async () => {
      if (
        !(await uiConfirm(
          `Delete "${item.meta.title || item.slug}" and its whole history from disk?`,
          "Delete"
        ))
      )
        return;
      try {
        await invoke("delete_workflow_item", { project: item.project, slug: item.slug });
        refreshWorkflows();
      } catch (e) {
        uiAlert(`Delete failed: ${e}`);
      }
    },
  });
  showContextMenu(x, y, items);
}

/// Entry modes — mirrors WorkflowMode in src/domain/workflow.rs (the
/// kebab-case strings are the shared vocabulary; keep both sites in sync).
const WF_MODES = [
  {
    value: "full",
    label: "Full workflow",
    detail: "agent plans → you approve → agent implements → you review the diff → PR",
  },
  {
    value: "from-plan",
    label: "From a plan I already have",
    detail: "no planning agent — starts at plan review, one approval from implementation",
  },
  {
    value: "review-only",
    label: "Review only — an existing PR or branch",
    detail: "no plan, no first implementation: straight to diff review ⇄ changes requested",
  },
];

/// Sentinel value for the repo picker's "Browse…" row — a real repo path can
/// never collide with it.
///
/// A `Symbol` rather than a magic string: `uiListChoice` hands `item.value` back
/// through a closure (it never round-trips through the DOM), so identity
/// comparison is enough, and nothing a user could type can equal it. It used to
/// be `"\x00browse"`, whose literal NUL byte made `grep` treat this entire file
/// as binary — `grep -n BROWSE app.js` returned *nothing* without `-a`, which is
/// a debugging tax on every future reader. Plain `"browse"` would have been a
/// plausible path fragment, i.e. a weaker invariant than the one it replaced.
const BROWSE = Symbol("browse");

/// Folder name of a repo path, tolerating a trailing slash.
const repoBaseName = (p) => baseName(String(p || "").replace(/[\\/]+$/, ""));

/// Pick the repo + project an item belongs to (from existing workflow projects
/// and open sessions, browsed for, else typed in). Resolves to
/// `{ project, repoPath }`, or null when cancelled. Shared by every entry mode.
async function pickWorkflowRepo() {
  // Candidate repos, deduped by path (two repos can share a basename —
  // dedupe by name silently dropped one of them).
  const candidates = new Map(); // repo path -> project name
  for (const w of state.workflows) {
    const dir = w.meta.repoPath || "";
    if (dir && !candidates.has(dir)) candidates.set(dir, w.project);
  }
  for (const s of state.sessions || []) {
    const dir = s.cwd || s.project_path || "";
    const name = repoBaseName(dir);
    if (dir && name && !candidates.has(dir)) candidates.set(dir, name);
  }
  let project;
  let repoPath = "";
  if (candidates.size === 0) {
    project = null; // straight to manual entry
  } else {
    const items = [...candidates.entries()]
      .sort((a, b) => a[1].localeCompare(b[1]) || a[0].localeCompare(b[0]))
      .map(([dir, name]) => ({ label: name, detail: dir, value: dir }));
    items.push({ label: "Browse…", detail: "pick a repository folder", value: BROWSE });
    items.push({ label: "Other…", detail: "enter a project name and repo path", value: "" });
    const picked = await uiListChoice({
      message: "Repository for this workflow item",
      items,
    });
    if (picked === null) return null;
    if (picked === BROWSE) {
      const dir = await pickDirectory(
        state.settings.defaultCwd || state.homeDir || "",
        "Choose a repository folder"
      );
      if (!dir) return null;
      repoPath = dir;
      project = candidates.get(dir) || repoBaseName(dir);
    } else if (picked) {
      repoPath = picked;
      project = candidates.get(picked);
    } else {
      project = null;
    }
  }
  if (!project) {
    // Manual entry: the path first (with the folder picker, like the
    // new-session modal), then the project name pre-filled from the folder.
    repoPath = (
      (await uiPathPrompt(
        "Repository path (absolute)",
        repoPath || state.settings.defaultCwd || state.homeDir || ""
      )) || ""
    ).trim();
    if (!repoPath) return null;
    project = (
      (await uiPrompt("Project name (group under the workflows root)", repoBaseName(repoPath))) ||
      ""
    ).trim();
    if (!project) return null;
  }
  return { project, repoPath };
}

/// Ask where the plan comes from and resolve it to the backend's two seed
/// arguments: `plan` (pasted text) or `planFile` (a path the backend reads —
/// a markdown file, or a scratch note's file). Null = cancelled.
async function pickWorkflowPlan() {
  const src = await uiListChoice({
    message: "Where is the plan?",
    items: [
      { label: "Paste it", detail: "type or paste the plan markdown", value: "paste" },
      { label: "A markdown file", detail: "absolute path to a file on disk", value: "file" },
      { label: "A scratch note", detail: "pick one of your scratches", value: "note" },
    ],
  });
  if (src === null) return null;

  if (src === "paste") {
    const text = await uiTextPrompt("Plan (markdown) — ⌘⏎ / Ctrl+⏎ to save", "", "Use this plan");
    if (text === null || !text.trim()) return null;
    return { plan: text, planFile: null };
  }
  if (src === "file") {
    const path = ((await uiPrompt("Path to the plan file")) || "").trim();
    if (!path) return null;
    return { plan: null, planFile: path };
  }
  // Scratch note: the backend reads the note's file, so only the path travels.
  if (!state.notes || !state.notes.length) {
    try {
      state.notes = await invoke("list_scratch_notes");
    } catch (e) {
      uiAlert(`Could not list scratches: ${e}`);
      return null;
    }
  }
  const files = (state.notes || []).filter((n) => !n.isDir);
  if (!files.length) {
    uiAlert("No scratch notes yet — write one first, or paste the plan instead.");
    return null;
  }
  const path = await uiListChoice({
    message: "Which scratch note is the plan?",
    items: files.map((n) => ({ label: n.title, detail: n.id, value: n.path })),
  });
  if (path === null) return null;
  return { plan: null, planFile: path };
}

/// Review-only creation: pick the PR or branch, then let the backend resolve
/// it (gh) and materialize a checkout before the item exists.
async function newReviewWorkflow({ project, repoPath }) {
  const src = await uiListChoice({
    message: "What are you reviewing?",
    items: [
      { label: "A pull request", detail: "its GitHub URL or number", value: "pr" },
      { label: "A local branch", detail: "a branch in this repo", value: "branch" },
    ],
  });
  if (src === null) return;

  let pr = null;
  let branch = null;
  if (src === "pr") {
    pr = ((await uiPrompt("GitHub PR URL (or number)")) || "").trim();
    if (!pr) return;
  } else {
    let branches = [];
    try {
      branches = await invoke("list_repo_branches", { repoPath });
    } catch (e) {
      uiAlert(`Could not list branches: ${e}`);
      return;
    }
    if (!branches.length) {
      uiAlert("No local branches found in this repo.");
      return;
    }
    branch = await uiListChoice({
      message: "Which branch holds the feature?",
      items: branches.map((b) => ({
        label: b.name,
        detail: b.worktree ? `${b.lastCommit} · checked out in ${b.worktree}` : b.lastCommit,
        value: b.name,
      })),
    });
    if (branch === null) return;
  }

  // Resolving a PR and checking the branch out can take a few seconds (fetch).
  flashToast("Preparing the review…");
  try {
    const item = await invoke("create_workflow_review", {
      project,
      repoPath,
      pr,
      branch,
      title: null,
    });
    await refreshWorkflows();
    openWorkflowTab(item);
  } catch (e) {
    uiAlert(wfGhHint(e) || `Could not start the review: ${e}`);
  }
}

/// New-item flow: entry mode → repo/project → whatever that mode needs.
async function newWorkflowFlow() {
  if (!state.wfOpen) await toggleWorkflows();
  const mode = await uiListChoice({
    message: "How does this workflow item start?",
    items: WF_MODES.map((m) => ({ label: m.label, detail: m.detail, value: m.value })),
  });
  if (mode === null) return;

  const repo = await pickWorkflowRepo();
  if (!repo) return;

  // Review-only takes its title from the PR/branch, so it asks for neither.
  if (mode === "review-only") return newReviewWorkflow(repo);

  const title = await uiPrompt("Workflow item — title");
  if (!title || !title.trim()) return;

  let seed = { plan: null, planFile: null };
  if (mode === "from-plan") {
    const picked = await pickWorkflowPlan();
    if (!picked) return;
    seed = picked;
  }

  try {
    const item = await invoke("create_workflow_item", {
      project: repo.project,
      title: title.trim(),
      repoPath: repo.repoPath,
      mode,
      plan: seed.plan,
      planFile: seed.planFile,
    });
    await refreshWorkflows();
    openWorkflowTab(item);
  } catch (e) {
    uiAlert(`Create failed: ${e}`);
  }
}

/// Recreate a persisted workflow tab at boot (state.open entry + builder);
/// the saved sub-view is restored so the relaunch lands where we were.
function restoreWorkflowTab(key, saved) {
  if (key === "view:wfboard") {
    openWorkflowBoardTab();
    return;
  }
  if (key === "view:skills") {
    openSkillsTab();
    return;
  }
  const rest = key.slice("view:workflow:".length);
  const slash = rest.indexOf("/");
  if (slash < 0) return;
  const project = rest.slice(0, slash);
  const slug = rest.slice(slash + 1);
  if (saved && saved.subView) {
    wfTabState.set(wfKey(project, slug), { subView: saved.subView, iteration: null });
  }
  openViewTab(key, (saved && saved.name) || `⧉ ${slug}`, (el) =>
    buildWorkflowView(el, project, slug)
  );
}

/// Open (or focus) an item's detail tab and clear its unread badge.
function openWorkflowTab(item, opts = {}) {
  const key = wfKey(item.project, item.slug);
  state.wfUnread.delete(key);
  renderWorkflows();
  openViewTab(`view:workflow:${key}`, `⧉ ${item.meta.title || item.slug}`, (el) =>
    buildWorkflowView(el, item.project, item.slug, opts)
  );
}

/// Skills viewer: the Claude Code skills clash ships (managed, auto-updated
/// at startup) and any other skills under ~/.claude/skills. Singleton tab.
function openSkillsTab(selectName = null) {
  openViewTab("view:skills", "☰ Skills", (el) => buildSkillsView(el, selectName));
}

async function buildSkillsView(el, selectName) {
  el.classList.add("skills-view");
  el.innerHTML = "<p class='hint'>loading…</p>";
  let skills = [];
  try {
    skills = await invoke("list_skills");
  } catch (e) {
    el.innerHTML = `<h4>SKILLS</h4><p class='hint'>failed: ${escapeHtml(e)}</p>`;
    return;
  }
  el.innerHTML = "";
  const wrap = document.createElement("div");
  wrap.className = "skills-wrap";
  const list = document.createElement("div");
  list.className = "skills-list";
  const content = document.createElement("div");
  content.className = "skills-content";
  wrap.append(list, content);
  el.appendChild(wrap);

  if (!skills.length) {
    content.innerHTML =
      "<p class='hint'>no skills found under ~/.claude/skills — clash installs its own (clash-workflow) at startup</p>";
    return;
  }

  let activeRow = null;
  const show = async (skill, row) => {
    if (activeRow) activeRow.classList.remove("active");
    activeRow = row;
    row.classList.add("active");
    content.innerHTML = "<p class='hint'>loading…</p>";
    let text = "";
    try {
      text = await invoke("get_skill", { name: skill.name });
    } catch (e) {
      content.innerHTML = `<p class='hint'>failed: ${escapeHtml(e)}</p>`;
      return;
    }
    content.innerHTML = "";
    const tools = document.createElement("div");
    tools.className = "wf-doc-tools";
    if (skill.managed) {
      const note = document.createElement("span");
      note.className = "skills-managed-note";
      note.textContent = skill.upToDate
        ? "managed by clash — auto-updated at startup, local edits are overwritten"
        : "managed by clash — differs from the embedded version (refreshes on next launch)";
      tools.appendChild(note);
    }
    const open = document.createElement("button");
    open.className = "icon-btn wide";
    open.innerHTML = `${svgIcon("pencil", 12)}<span>Open in editor</span>`;
    open.onclick = (ev) =>
      openScratchInEditor({ path: skill.path, title: skill.name }, ev.clientX, ev.clientY);
    tools.appendChild(open);
    content.appendChild(tools);
    const md = document.createElement("div");
    md.className = "wf-md";
    renderMarkdown(md, text);
    content.appendChild(md);
  };

  let first = null;
  for (const skill of skills) {
    const row = document.createElement("div");
    row.className = "skills-row";
    const badge = skill.managed
      ? `<span class="skills-badge${skill.upToDate ? "" : " stale"}">${
          skill.upToDate ? "clash" : "clash·stale"
        }</span>`
      : "";
    row.innerHTML =
      `<div class="skills-row-name">${escapeHtml(skill.name)}${badge}</div>` +
      `<div class="skills-row-desc">${escapeHtml(
        (skill.description || "").slice(0, 140)
      )}</div>`;
    row.onclick = () => show(skill, row);
    list.appendChild(row);
    if (!first || skill.name === selectName) first = { skill, row };
    if (skill.name === selectName) first = { skill, row };
  }
  if (first) show(first.skill, first.row);
}

// Board columns: display name → statuses bucketed into it.
const WF_BOARD_COLUMNS = [
  ["DRAFT / PLANNING", ["draft", "planning"]],
  ["PLAN REVIEW", ["plan-review"]],
  // `reviewing` is an agent-working state like implementing: an item in a
  // review round is in flight, not waiting on the human.
  ["CHANGES / IMPL.", ["changes-requested", "implementing", "reviewing"]],
  ["DIFF REVIEW", ["diff-review"]],
  ["PR", ["pr-draft", "pr-ready"]],
  ["DONE", ["done", "abandoned"]],
];

/// Kanban-style board over all workflow items (singleton tab).
function openWorkflowBoardTab() {
  openViewTab("view:wfboard", "⧉ Workflows", async (el) => {
    el.classList.add("wf-board-wrap");
    el.innerHTML = "<p class='hint'>loading…</p>";
    if (!state.workflows.length) await refreshWorkflows();
    renderWorkflowBoard(el);
  });
}

function renderWorkflowBoard(el) {
  el.innerHTML = "";
  const board = document.createElement("div");
  board.className = "wf-board";
  for (const [name, statuses] of WF_BOARD_COLUMNS) {
    const items = state.workflows.filter((w) => statuses.includes(w.meta.status));
    const col = document.createElement("div");
    col.className = "wf-col";
    col.innerHTML = `<div class="wf-col-head">${name} <span class="dim">${items.length}</span></div>`;
    for (const item of items) col.appendChild(buildWorkflowCard(item));
    board.appendChild(col);
  }
  el.appendChild(board);
}

function buildWorkflowCard(item) {
  const info = wfStatusInfo(item.meta.status);
  const card = document.createElement("div");
  card.className = "wf-card";
  const bits = [`it.${item.meta.iteration || 1}`];
  if (item.openAnnotations > 0) bits.push(`💬${item.openAnnotations}`);
  if (item.meta.pr && item.meta.pr.url) bits.push(item.meta.pr.draft ? "PR·draft" : "PR");
  if (item.agentAlive === false) bits.push("⚠ agent gone");
  card.innerHTML =
    `<div class="wf-card-title"><span class="status-ring wf-ring ${info.cls}"></span>${escapeHtml(
      item.meta.title || item.slug
    )}</div>` +
    `<div class="wf-card-sub">${escapeHtml(item.project)} · ${bits.join(" · ")}</div>`;
  card.onclick = () => openWorkflowTab(item);
  card.oncontextmenu = (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    workflowContextMenu(item, ev.clientX, ev.clientY);
  };
  return card;
}

// Per-tab UI state (active sub-view, viewed iteration) — survives rebuilds
// triggered by `workflows-changed` so a refresh never yanks you elsewhere.
const wfTabState = new Map(); // "project/slug" -> { subView, iteration }

/// Item detail view: header strip + sub-view bar + content + action bar.
async function buildWorkflowView(el, project, slug) {
  const key = wfKey(project, slug);
  const ts = wfTabState.get(key) || { subView: null, iteration: null };
  wfTabState.set(key, ts);
  el.classList.add("wf-view");
  el.innerHTML = "<p class='hint'>loading…</p>";

  let item = wfItem(project, slug);
  if (!item) {
    try {
      state.workflows = await invoke("list_workflow_items");
    } catch (e) {
      console.error("list_workflow_items failed:", e);
    }
    item = wfItem(project, slug);
  }
  if (!item) {
    el.innerHTML = "<h4>WORKFLOW</h4><p class='hint'>item not found (deleted?)</p>";
    return;
  }

  // Landing sub-view depends on the mode: a review-only item has no plan, so
  // the diff is what it is about. Resolved here (not at tab-state creation)
  // because the mode is only known once the item is loaded. A restored tab can
  // also carry a "plan" sub-view the item no longer offers — coerce it, or the
  // view would render a doc with no tab to leave it by.
  if (!ts.subView || (ts.subView === "plan" && !wfHasPlanPhase(item)))
    ts.subView = wfHasPlanPhase(item) ? "plan" : "diff";
  // Same coercion for the agent-reviews tab, which only exists once a round has
  // run: a tab restored from a state where it did would otherwise render a doc
  // with no tab to leave it by.
  if (ts.subView === "agentReview" && !(item.hasAgentReview || item.meta.reviewRound))
    ts.subView = "diff";

  el.innerHTML = "";
  const info = wfStatusInfo(item.meta.status);

  // ── Header strip ──
  const head = document.createElement("div");
  head.className = "wf-head";
  const warn =
    item.agentAlive === false
      ? ` · <span class="wf-warn">⚠ agent gone</span>`
      : "";
  // Non-default entry modes get their own chip: it explains why the pipeline
  // is shorter than the canonical one (no plan, or a supplied plan).
  const modeChip = wfIsReviewOnly(item)
    ? `<span class="wf-chip wf-mode" title="Review-only item — no planning or first implementation phase">⌕ REVIEW ONLY</span>`
    : item.meta.mode === "from-plan"
      ? `<span class="wf-chip wf-mode" title="The plan was supplied, not written by an agent">◫ FROM PLAN</span>`
      : "";
  const branchBit = item.meta.branch ? ` · ${escapeHtml(item.meta.branch)}` : "";
  head.innerHTML =
    `<span class="wf-title">${escapeHtml(item.meta.title || item.slug)}</span>` +
    `<span class="wf-chip ${info.cls}">${info.icon} ${info.label}</span>` +
    modeChip +
    `<span class="wf-meta">${escapeHtml(item.project)}${branchBit} · it.${
      item.meta.iteration || 1
    }${warn}</span>` +
    `<span class="spacer"></span>`;
  if (item.meta.pr && item.meta.pr.url) {
    const prBtn = document.createElement("button");
    prBtn.className = "icon-btn wide";
    prBtn.innerHTML = `${svgIcon("pr", 12)}<span>#${item.meta.pr.number || "PR"}${item.meta.pr.draft ? " draft" : ""}</span>`;
    prBtn.title = item.meta.pr.url;
    prBtn.onclick = () => openWorkflowPr(item);
    head.appendChild(prBtn);
  }
  el.appendChild(head);

  // ── Sub-view bar ──
  const bar = document.createElement("div");
  bar.className = "wf-subbar";
  // Review-only items have no plan at all — an always-empty Plan tab would
  // read as a missing step rather than an absent phase.
  const subViews = [
    ...(wfHasPlanPhase(item) ? [["plan", `Plan${item.hasPlan ? "" : " ·empty"}`]] : []),
    ["review", `Review${item.hasReview ? "" : " ·empty"}`],
    // Agent review rounds accumulate, so the tab counts them — that count is
    // the item's review history and the reason a round is worth repeating.
    ...(item.hasAgentReview || item.meta.reviewRound
      ? [["agentReview", `Agent reviews${item.meta.reviewRound ? ` (${item.meta.reviewRound})` : ""}`]]
      : []),
    ["diff", item.openAnnotations > 0 ? `Diff 💬${item.openAnnotations}` : "Diff"],
    ["history", `History${item.historyIterations.length ? ` (${item.historyIterations.length})` : ""}`],
  ];
  for (const [id, label] of subViews) {
    const b = document.createElement("button");
    b.className = "wf-subtab" + (ts.subView === id ? " active" : "");
    b.textContent = label;
    b.onclick = () => {
      ts.subView = id;
      if (id !== "diff") ts.iteration = null;
      buildWorkflowView(el, project, slug);
    };
    bar.appendChild(b);
  }
  el.appendChild(bar);

  // ── Content ──
  const body = document.createElement("div");
  body.className = "wf-body";
  el.appendChild(body);

  // ── Action bar ──
  const actions = document.createElement("div");
  actions.className = "wf-actions";
  el.appendChild(actions);
  renderWfActions(actions, el, item);

  renderWfSubView(body, el, item, ts);
}

/// Open the item's PR in the embedded browser (split pane), falling back to
/// the system browser when the embedded panel is unavailable.
function openWorkflowPr(item) {
  const url = item.meta.pr && item.meta.pr.url;
  if (!url) return;
  if (typeof openBrowserTab === "function") openBrowserTab(url, "split");
  else invoke("open_external", { url });
}

/// Launch (or relaunch) the workflow agent for a phase and open its session
/// tab split next to the workflow tab.
async function launchWfAgent(item, phase, root, branch = null) {
  try {
    const sid = await invoke("start_workflow_agent", {
      project: item.project,
      slug: item.slug,
      phase,
      branch,
      cols: 120,
      rows: 40,
    });
    await refreshSessions();
    await openSession(sid, `wf-${item.slug}`);
    await refreshWorkflows();
    if (root) buildWorkflowView(root, item.project, item.slug);
  } catch (e) {
    // The default branch name (the slug) already exists in the repo — ask
    // what to call this workflow's branch and retry with it.
    const msg = String(e);
    if (msg.startsWith("branch-exists:")) {
      const taken = msg.slice("branch-exists:".length);
      const next = await uiPrompt(
        `Branch "${taken}" already exists in this repo — branch name for this workflow:`,
        `${taken}-2`
      );
      if (next === null) return;
      const name = next.trim();
      if (!name) return;
      return launchWfAgent(item, phase, root, name);
    }
    uiAlert(`Agent launch failed: ${e}`);
  }
}

/// Launch one agent review round: pick the depth, then what to do with the
/// findings, then spawn the reviewer. The *target* is not asked — it follows
/// from the item's status (plan at plan-review, code everywhere else), because
/// the other choice has nothing to read.
///
/// Rounds are unbounded by design: the reviewer returns the item to the status
/// it started in, so this same button is available again the moment it finishes.
async function launchWfReview(item, root) {
  const target = wfReviewTarget(item);
  const what = target === "plan" ? "plan" : "code";
  const round = (item.meta.reviewRound || 0) + 1;

  const depth = await uiChoice({
    message: `Review round ${round} — how deep?`,
    detail:
      target === "plan"
        ? "Standard reads the plan and the files it names. Deep goes and reads how the code actually works, then checks the plan against it."
        : "Standard reviews the diff in context. Deep traces every subsystem the change touches — callers, invariants, existing tests — before judging it.",
    choices: [
      { label: "Deep review", value: "deep", primary: true },
      { label: `Standard ${what} review`, value: "standard" },
    ],
  });
  if (!depth) return;

  // The publish question only exists once there is a PR to talk to.
  let publish = "local";
  if (wfHasPr(item)) {
    const n = item.meta.pr.number ? `#${item.meta.pr.number}` : "the PR";
    publish = await uiChoice({
      message: "What should this round do with its findings?",
      detail: `Findings always land in this item. These options additionally talk to ${n}.`,
      choices: [
        { label: "Keep local", value: "local", primary: true },
        { label: `Post findings to ${n}`, value: "pr-comments" },
        { label: `Answer ${n}'s review comments`, value: "respond-pr-comments" },
      ],
    });
    if (!publish) return;
  }

  try {
    const sid = await invoke("start_workflow_review_agent", {
      project: item.project,
      slug: item.slug,
      depth,
      publish,
      cols: 120,
      rows: 40,
    });
    await refreshSessions();
    await openSession(sid, `wf-${item.slug}`);
    await refreshWorkflows();
    if (root) buildWorkflowView(root, item.project, item.slug);
  } catch (e) {
    const msg = String(e);
    if (msg.startsWith("no-pr:")) {
      uiAlert("This item has no pull request yet — create one first, or keep the round local.");
      return;
    }
    uiAlert(`Review launch failed: ${e}`);
  }
}

/// Put a wedged review round back where it came from. The gated `reviewing`
/// state is the one place a dead agent could strand the item with its Approve
/// button disabled, so this escape hatch is always reachable.
async function cancelWfReview(item, root) {
  if (
    !(await uiConfirm(
      "End this review round and unlock the item? Anything the agent already wrote is kept.",
      "End round"
    ))
  )
    return;
  try {
    await invoke("cancel_workflow_review", { project: item.project, slug: item.slug });
    await refreshWorkflows();
    buildWorkflowView(root, item.project, item.slug);
  } catch (e) {
    uiAlert(`Could not end the round: ${e}`);
  }
}

/// Human status transition + tab/sidebar refresh.
async function wfTransition(item, root, status, note = null) {
  try {
    await invoke("update_workflow_status", {
      project: item.project,
      slug: item.slug,
      status,
      note,
    });
    await refreshWorkflows();
    buildWorkflowView(root, item.project, item.slug);
  } catch (e) {
    uiAlert(`Transition failed: ${e}`);
  }
}

/// In-memory change-request drafts, keyed by item.
///
/// Guards the real failure: a stray Esc or backdrop click destroying a paragraph
/// you just wrote. Deliberately not localStorage — that is documented as
/// unreliable in this webview, and a draft is not state worth promising to
/// persist across a relaunch.
const wfDrafts = new Map();

/// Compose a change request, then send it.
///
/// This is a prompt-authoring surface, not a form field: the note is appended
/// verbatim to `review.md` under `## Iteration N`, and the agent's first
/// instruction is to read that section. It replaced a one-line `uiPrompt` — in
/// which a multi-paragraph instruction was simply not typeable, and a newline was
/// impossible.
///
/// `target` is "plan" (plan-review) or "diff" (diff-review). One composer serves
/// both; they used to be two divergent inline prompts.
function wfComposeChangeRequest({ item, target, annotations }) {
  return new Promise((resolve) => {
    const openCount = annotations.length;
    const key = draftKey(item.project, item.slug);

    const backdrop = document.createElement("div");
    backdrop.className = "dialog-backdrop";
    const box = document.createElement("div");
    box.className = "dialog-box wf-compose";

    const title = document.createElement("p");
    title.textContent =
      target === "plan" ? "Request changes to the plan" : "Request changes";
    box.appendChild(title);

    const hint = document.createElement("p");
    hint.className = "dialog-detail";
    hint.textContent =
      `Markdown. Appended to review.md as iteration ${(item.meta.iteration || 0) + 1} and ` +
      "read by the agent as its instructions for the next round.";
    box.appendChild(hint);

    // Toolbar: template + preview. Both are opt-in so the one-sentence path
    // stays a one-sentence path.
    const tools = document.createElement("div");
    tools.className = "wf-compose-tools";
    const tmplBtn = document.createElement("button");
    tmplBtn.type = "button";
    tmplBtn.className = "icon-btn wide";
    tmplBtn.textContent = "Insert template";
    tmplBtn.title = "What to change / Why / Out of scope";
    tools.appendChild(tmplBtn);
    const previewBtn = document.createElement("button");
    previewBtn.type = "button";
    previewBtn.className = "icon-btn wide";
    previewBtn.textContent = "Preview";
    tools.appendChild(previewBtn);
    box.appendChild(tools);

    const field = document.createElement("textarea");
    field.className = "wf-compose-input";
    field.rows = 14;
    field.spellcheck = false;
    field.value = wfDrafts.get(key) || "";
    field.placeholder = composerPlaceholder(target, openCount);
    box.appendChild(field);

    // Rendered preview of exactly what will land in review.md.
    const preview = document.createElement("div");
    preview.className = "wf-md wf-compose-preview";
    preview.hidden = true;
    box.appendChild(preview);

    let previewing = false;
    previewBtn.onclick = () => {
      previewing = !previewing;
      previewBtn.textContent = previewing ? "Edit" : "Preview";
      field.hidden = previewing;
      preview.hidden = !previewing;
      if (previewing) {
        const body = field.value.trim();
        const annos = annotationsMarkdown(annotations);
        const full =
          body + (annos ? `${body ? "\n\n" : ""}### Open annotations\n\n${annos}` : "");
        renderMarkdown(preview, full || "_(nothing to send yet)_");
      } else {
        field.focus();
      }
    };
    tmplBtn.onclick = () => {
      const tmpl = changeRequestTemplate(target);
      // Prepend rather than replace — never destroy what is already typed.
      field.value = field.value.trim() ? `${tmpl}\n${field.value}` : tmpl;
      if (previewing) previewBtn.onclick();
      field.focus();
      field.setSelectionRange(0, 0);
    };

    // The comments already queued for this round, so the note can complement
    // them instead of repeating them. A bare count could never do that.
    if (openCount) {
      const label = document.createElement("p");
      label.className = "wf-compose-annos-label";
      label.textContent = `${openCount} open diff comment${openCount > 1 ? "s" : ""} sent with this round:`;
      box.appendChild(label);
      const list = document.createElement("div");
      list.className = "wf-compose-annos";
      renderMarkdown(list, annotationsMarkdown(annotations));
      box.appendChild(list);
    }

    const error = document.createElement("p");
    error.className = "wf-compose-error";
    error.hidden = true;
    box.appendChild(error);

    const actions = document.createElement("div");
    actions.className = "modal-actions";
    const cancel = document.createElement("button");
    cancel.textContent = "Cancel";
    actions.appendChild(cancel);
    const send = document.createElement("button");
    send.className = "primary";
    send.textContent = "Request changes";
    actions.appendChild(send);
    box.appendChild(actions);

    const done = (value) => {
      backdrop.remove();
      resolve(value);
      if (typeof fitAll === "function") fitAll();
    };
    const keepDraft = () => {
      const text = field.value;
      if (text.trim()) wfDrafts.set(key, text);
      else wfDrafts.delete(key);
    };
    const dismiss = () => {
      keepDraft();
      done(null);
    };
    const submit = () => {
      const note = field.value.trim();
      const check = canSubmitChangeRequest({ note, openCount, target });
      if (!check.ok) {
        error.textContent = check.reason;
        error.hidden = false;
        field.focus();
        return;
      }
      wfDrafts.delete(key);
      done(note);
    };

    cancel.onclick = dismiss;
    send.onclick = submit;
    backdrop.onclick = (e) => {
      if (e.target === backdrop) dismiss();
    };
    backdrop.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Escape") dismiss();
      // Enter types a newline; ⌘/Ctrl+Enter sends — same contract as the
      // multiline uiDialog, so the muscle memory carries over.
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        submit();
      }
    });

    if (typeof hideBrowserWebviews === "function") hideBrowserWebviews();
    document.body.appendChild(backdrop);
    setTimeout(() => {
      field.focus();
      // Cursor at the end, so a restored draft is continued rather than overwritten.
      field.setSelectionRange(field.value.length, field.value.length);
    }, 0);
  });
}

/// Fetch the open annotations for an item, so the composer can show them.
/// Failure is non-fatal: the composer still opens, just without the list.
async function wfOpenAnnotations(item) {
  try {
    const file = await invoke("get_workflow_annotations", {
      project: item.project,
      slug: item.slug,
    });
    const all = (file && file.annotations) || [];
    return all.filter((a) => a.status === "open");
  } catch (e) {
    dlog(`wfOpenAnnotations failed: ${e}`);
    return [];
  }
}

/// PR-button degradation: gh missing/unauthenticated disables PR flows with
/// a setup hint instead of a hard failure.
function wfGhHint(e) {
  const msg = String(e);
  if (msg.startsWith("gh-unavailable:"))
    return "GitHub CLI (gh) is not installed — `brew install gh`, then retry.";
  if (msg.startsWith("gh-unauthenticated:"))
    return "gh is not authenticated — run `gh auth login`, then retry.";
  return null;
}

/// Action bar — per-status primary/secondary actions. The Mark-ready confirm
/// IS the validation act (there is deliberately no separate 'validated'
/// state).
function renderWfActions(bar, root, item) {
  bar.innerHTML = "";
  const st = item.meta.status;
  const add = (label, cls, fn, title = "") => {
    const b = document.createElement("button");
    b.textContent = label;
    if (cls) b.className = cls;
    if (title) b.title = title;
    b.onclick = fn;
    bar.appendChild(b);
    return b;
  };
  const spacer = () => {
    const sp = document.createElement("span");
    sp.className = "spacer";
    bar.appendChild(sp);
  };
  const abandon = () =>
    add("Abandon", "", async () => {
      if (!(await uiConfirm(`Abandon "${item.meta.title || item.slug}"? The files stay on disk.`)))
        return;
      wfTransition(item, root, "abandoned");
    });

  const requestChanges = async () => {
    const annotations = await wfOpenAnnotations(item);
    const note = await wfComposeChangeRequest({ item, target: "diff", annotations });
    if (note === null) return;
    try {
      await invoke("workflow_request_changes", {
        project: item.project,
        slug: item.slug,
        note: note || null,
      });
      await refreshWorkflows();
      buildWorkflowView(root, item.project, item.slug);
    } catch (e) {
      uiAlert(`Request changes failed: ${e}`);
    }
  };

  // Available from every state holding a reviewable artifact, every time the
  // item lands back there — that is what makes rounds repeatable. The label
  // counts past rounds so it is obvious this is round N+1, not a one-shot.
  const reviewButton = () => {
    if (!wfCanReview(item)) return;
    const n = item.meta.reviewRound || 0;
    add(
      n ? `⌕ Review again (${n})` : "⌕ Agent review",
      "",
      () => launchWfReview(item, root),
      n
        ? `${n} agent review round${n > 1 ? "s" : ""} so far — run another`
        : `Have an agent review this item's ${
            wfReviewTarget(item) === "plan" ? "plan" : "code"
          } and report findings`
    );
  };

  switch (st) {
    case "draft":
      add("▶ Start planning", "primary", () => launchWfAgent(item, "plan", root));
      spacer();
      abandon();
      break;

    case "reviewing": {
      // Gated: no Approve while a round is in flight. The only ways out are the
      // agent finishing, ending the round, or abandoning the item.
      const r = item.meta.review || {};
      const via = r.returnStatus ? wfStatusInfo(r.returnStatus).label : "where it started";
      if (item.agentAlive === false) {
        add("⚠ End review round", "primary", () => cancelWfReview(item, root),
          "The reviewer session is gone — unlock the item");
      } else if (item.meta.sessionId) {
        add("Open review session", "primary", () => openSession(item.meta.sessionId),
          `Review round ${r.round || 1} — ${r.depth || "standard"} ${r.target || ""}`.trim());
      }
      add("End round", "", () => cancelWfReview(item, root), `Returns the item to ${via}`);
      spacer();
      abandon();
      break;
    }

    case "planning":
    case "implementing":
      if (item.agentAlive === false) {
        add("⚠ Relaunch agent", "primary", () =>
          launchWfAgent(item, st === "planning" ? "plan" : "revise", root)
        , "The recorded agent session is gone");
      } else if (item.meta.sessionId) {
        add("Open agent session", "primary", () => openSession(item.meta.sessionId));
      }
      spacer();
      abandon();
      break;

    case "plan-review":
      add("✓ Approve plan", "primary", async () => {
        if (!(await uiConfirm("Approve this plan and move to implementation?", "Approve"))) return;
        await wfTransition(item, root, "implementing");
        const go = await uiChoice({
          message: "Launch the implementation agent now?",
          choices: [{ label: "Launch agent", value: "go", primary: true }],
        });
        const fresh = wfItem(item.project, item.slug) || item;
        if (go === "go") launchWfAgent(fresh, "implement", root);
      });
      add("✎ Request changes", "", async () => {
        // Same composer as the diff path: a plan revision is just as much a
        // prompt for the next round, and it was the same one-line input.
        const note = await wfComposeChangeRequest({ item, target: "plan", annotations: [] });
        if (note === null) return;
        await wfTransition(item, root, "changes-requested", note);
      });
      reviewButton();
      spacer();
      abandon();
      break;

    case "changes-requested":
      // A review-only item reaches this state without any agent having run
      // yet, so "relaunch" would be a lie on its first round.
      add(
        item.meta.sessionId ? "▶ Relaunch agent" : "▶ Launch agent",
        "primary",
        () => launchWfAgent(item, "revise", root)
      );
      spacer();
      abandon();
      break;

    case "diff-review": {
      // Approval at diff-review means one thing: the human is satisfied with
      // the diff. What happens *next* is their choice, so a draft PR is never a
      // precondition — it used to be, and for anyone who merges straight to
      // their default branch that made the only primary action a dead end.
      //
      // `→ done` is therefore always offered. The PR path stays primary only
      // when clash already knows about a PR (the item is evidently in a PR
      // flow); otherwise it is an opt-in secondary.
      const openWarning = () =>
        item.openAnnotations > 0
          ? ` ${item.openAnnotations} comment${item.openAnnotations > 1 ? "s are" : " is"} still open.`
          : "";
      const hasPr = !!(item.meta.pr && item.meta.pr.url);

      const approveDone = () =>
        add("✓ Approve → done", hasPr ? "" : "primary", async () => {
          if (!(await uiConfirm(`Approve this diff and close the item?${openWarning()}`, "Approve")))
            return;
          wfTransition(item, root, "done");
        });

      if (hasPr) {
        // Review-only items track a PR clash doesn't own, so there is no
        // draft-PR ceremony to advance into — only the full pipeline has one.
        if (!wfIsReviewOnly(item)) {
          add("✓ Approve → PR draft", "primary", async () => {
            if (!(await uiConfirm(`Approve these changes?${openWarning()}`, "Approve"))) return;
            wfTransition(item, root, "pr-draft");
          });
        }
        approveDone();
        add("Open PR", "", () => openWorkflowPr(item));
      } else {
        approveDone();
        add("Create draft PR", "", async () => {
          if (!(await uiConfirm("Open a draft PR for this branch?", "Create"))) return;
          try {
            await invoke("workflow_create_pr", {
              project: item.project,
              slug: item.slug,
              title: null,
              body: null,
            });
            flashToast("Draft PR created");
            await refreshWorkflows();
            buildWorkflowView(root, item.project, item.slug);
          } catch (e) {
            uiAlert(wfGhHint(e) || `Create PR failed: ${e}`);
          }
        });
      }
      add("✎ Request changes", "", requestChanges);
      reviewButton();
      spacer();
      abandon();
      break;
    }

    case "pr-draft": {
      if (item.meta.pr && item.meta.pr.url) {
        add("✓ Mark PR ready", "primary", async () => {
          const warn =
            item.openAnnotations > 0
              ? ` ${item.openAnnotations} comment${item.openAnnotations > 1 ? "s are" : " is"} still open —`
              : "";
          if (
            !(await uiConfirm(
              `This is the validation step:${warn} flip PR #${item.meta.pr.number} to ready-for-review?`,
              "Mark ready"
            ))
          )
            return;
          try {
            const meta = await invoke("mark_workflow_pr_ready", {
              project: item.project,
              slug: item.slug,
            });
            flashToast(`PR ready: ${meta.pr ? meta.pr.url : ""}`);
            await refreshWorkflows();
            buildWorkflowView(root, item.project, item.slug);
          } catch (e) {
            uiAlert(wfGhHint(e) || `Mark ready failed: ${e}`);
          }
        });
        add("Open PR", "", () => openWorkflowPr(item));
      } else {
        add("Attach PR by URL…", "primary", async () => {
          const url = await uiPrompt("GitHub PR URL");
          if (!url || !url.trim()) return;
          try {
            await invoke("attach_workflow_pr", {
              project: item.project,
              slug: item.slug,
              url: url.trim(),
            });
            await refreshWorkflows();
            buildWorkflowView(root, item.project, item.slug);
          } catch (e) {
            uiAlert(`Attach failed: ${e}`);
          }
        });
      }
      add("↩ Back to review", "", () => wfTransition(item, root, "diff-review"));
      reviewButton();
      spacer();
      abandon();
      break;
    }

    case "pr-ready":
      if (item.meta.pr && item.meta.pr.url) add("Open PR", "primary", () => openWorkflowPr(item));
      add("✓ Mark done", "", async () => {
        if (!(await uiConfirm("Mark this workflow item as done?", "Done"))) return;
        wfTransition(item, root, "done");
      });
      reviewButton();
      break;

    case "done":
    case "abandoned":
      if (item.meta.pr && item.meta.pr.url) add("Open PR", "", () => openWorkflowPr(item));
      add("Reopen", "", () => wfTransition(item, root, "diff-review"));
      break;
  }
}

async function renderWfSubView(body, root, item, ts) {
  const { project, slug } = item;
  if (ts.subView === "plan" || ts.subView === "review" || ts.subView === "agentReview") {
    const doc =
      ts.subView === "plan"
        ? "plan.md"
        : ts.subView === "review"
          ? "review.md"
          : "agent-review.md";
    body.innerHTML = "<p class='hint'>loading…</p>";
    let text = "";
    try {
      text = await invoke("get_workflow_doc", { project, slug, doc });
    } catch (e) {
      body.innerHTML = `<p class='hint'>failed: ${escapeHtml(e)}</p>`;
      return;
    }
    body.innerHTML = "";
    const tools = document.createElement("div");
    tools.className = "wf-doc-tools";
    const edit = document.createElement("button");
    edit.className = "icon-btn wide";
    edit.innerHTML = `${svgIcon("pencil", 12)}<span>Edit ${doc}</span>`;
    edit.onclick = (ev) =>
      openScratchInEditor(
        { path: `${item.path}/${doc}`, title: `${slug} ${doc}` },
        ev.clientX,
        ev.clientY
      );
    tools.appendChild(edit);
    body.appendChild(tools);
    const md = document.createElement("div");
    md.className = "wf-md";
    if (text.trim()) renderMarkdown(md, text);
    else
      md.innerHTML = `<p class="hint">${
        ts.subView === "plan"
          ? "no plan yet — Start planning launches an agent that writes it"
          : ts.subView === "review"
            ? "no review notes yet — they accumulate when you request changes"
            : "no agent reviews yet — each round appends its findings here"
      }</p>`;
    body.appendChild(md);
    return;
  }

  if (ts.subView === "history") {
    body.innerHTML = "<p class='hint'>loading…</p>";
    let iters = [];
    try {
      iters = await invoke("list_workflow_history", { project, slug });
    } catch (e) {
      body.innerHTML = `<p class='hint'>failed: ${escapeHtml(e)}</p>`;
      return;
    }
    body.innerHTML = "";
    if (!iters.length) {
      body.innerHTML =
        "<p class='hint'>no snapshots yet — each Request-changes freezes the diff + annotations of the iteration</p>";
      return;
    }
    for (const it of [...iters].reverse()) {
      const row = document.createElement("div");
      row.className = "row-item";
      const current = item.meta.iteration || 1;
      row.innerHTML =
        `<span class="team-icon">${svgIcon("file", 12)}</span>` +
        `<span>Iteration ${it}</span>` +
        `<span class="dim">${it === current ? "current" : `superseded by it.${it + 1}`}</span>` +
        `<span class="spacer"></span><span class="dim">view diff →</span>`;
      row.onclick = () => {
        ts.subView = "diff";
        ts.iteration = it;
        buildWorkflowView(root, project, slug);
      };
      body.appendChild(row);
    }
    return;
  }

  // diff sub-view
  renderWfDiffView(body, root, item, ts);
}

// Collapse guards for big diffs: files above this many changed lines render
// collapsed; a whole diff above the total starts with every file collapsed.
const WF_FILE_COLLAPSE_LINES = 1500;
const WF_DIFF_COLLAPSE_TOTAL = 20000;

/// Diff sub-view: iteration switcher + per-file hunks with line numbers.
/// Annotation threads/composer hook in via the delegated listener (next
/// commit); this render is already annotation-aware in its data fetch.
async function renderWfDiffView(body, root, item, ts) {
  const { project, slug } = item;
  body.innerHTML = "<p class='hint'>loading diff…</p>";
  let text = "";
  let anchored = [];
  try {
    text = await invoke("get_workflow_diff", { project, slug, iteration: ts.iteration });
    anchored = await invoke("get_anchored_annotations", { project, slug, iteration: ts.iteration });
  } catch (e) {
    body.innerHTML = `<p class='hint'>diff failed: ${escapeHtml(e)}</p>`;
    return;
  }
  body.innerHTML = "";

  // Header controls: iteration switcher + unresolved count.
  const head = document.createElement("div");
  head.className = "wf-diff-head";
  const sel = document.createElement("select");
  const cur = document.createElement("option");
  cur.value = "";
  cur.textContent = `current (it.${item.meta.iteration || 1})`;
  sel.appendChild(cur);
  for (const it of [...item.historyIterations].reverse()) {
    const o = document.createElement("option");
    o.value = String(it);
    o.textContent = `iteration ${it}`;
    sel.appendChild(o);
  }
  sel.value = ts.iteration === null ? "" : String(ts.iteration);
  sel.onchange = () => {
    ts.iteration = sel.value === "" ? null : parseInt(sel.value, 10);
    buildWorkflowView(root, project, slug);
  };
  head.appendChild(sel);
  const openCount = anchored.filter((a) => a.annotation.status === "open").length;
  const count = document.createElement("span");
  count.className = "wf-diff-count";
  count.textContent = openCount > 0 ? `💬 ${openCount} unresolved` : "";
  head.appendChild(count);
  body.appendChild(head);

  // Phase lock (review A2): the agent owns annotations.json while it works;
  // editing re-opens when the item returns to a review state. The backend
  // enforces the same rule — this banner just explains it.
  const locked = wfAnnotationsLocked(item);
  if (locked) {
    const banner = document.createElement("div");
    banner.className = "wf-lock-banner";
    banner.textContent = "agent is working — comments are locked until it finishes";
    body.appendChild(banner);
  }

  const files = parseUnifiedDiff(text);
  if (!files.length) {
    body.appendChild(Object.assign(document.createElement("p"), {
      className: "hint",
      textContent: ts.iteration === null ? "no changes yet" : "empty snapshot",
    }));
    return;
  }
  const totalLines = files.reduce((n, f) => n + diffFileChangedLines(f), 0);
  const container = document.createElement("div");
  container.className = "wf-diff" + (locked ? " locked" : "");
  body.appendChild(container);
  buildWorkflowDiff(container, files, anchored, {
    item,
    ts,
    root,
    locked,
    collapseAll: totalLines > WF_DIFF_COLLAPSE_TOTAL,
  });

  // Restore the scroll position a rebuild-after-mutation saved.
  if (ts.scrollTop) {
    body.scrollTop = ts.scrollTop;
    ts.scrollTop = 0;
  }
}

/// The agent owns annotations.json in these phases (review A2). Mirrors
/// `ensure_annotations_unlocked` — a review round writes its findings as
/// annotations, so the human's editor is locked while one runs.
function wfAnnotationsLocked(item) {
  return (
    item.meta.status === "changes-requested" ||
    item.meta.status === "implementing" ||
    item.meta.status === "reviewing"
  );
}

/// Build the diff DOM: .wf-file > head + hunks > numbered lines. One
/// delegated click listener on the container handles collapse (and, next
/// commit, comment composition/threads) — no per-line listeners, so large
/// diffs stay cheap.
function buildWorkflowDiff(container, files, anchored, opts) {
  container.innerHTML = "";
  opts._files = files;
  opts._anchored = anchored;
  container.onclick = (ev) => wfDiffClick(container, ev, opts);

  for (const f of files) {
    const path = diffFilePath(f);
    const changed = diffFileChangedLines(f);
    const fileAnns = anchored.filter((a) => a.currentFile === path);
    const openHere = fileAnns.filter((a) => a.annotation.status === "open").length;

    const fileEl = document.createElement("div");
    fileEl.className = "wf-file";
    fileEl.dataset.file = path;

    const collapsed = opts.collapseAll || changed > WF_FILE_COLLAPSE_LINES;
    const head = document.createElement("div");
    head.className = "wf-file-head";
    head.dataset.wfAct = "toggle-file";
    let adds = 0;
    let dels = 0;
    for (const h of f.hunks)
      for (const l of h.lines) {
        if (l.kind === "add") adds++;
        else if (l.kind === "del") dels++;
      }
    head.innerHTML =
      `<span class="wf-file-caret">${collapsed ? "▸" : "▾"}</span>` +
      `<span class="wf-file-path">${escapeHtml(path)}</span>` +
      (f.renamedFrom ? `<span class="dim">← ${escapeHtml(f.renamedFrom)}</span>` : "") +
      `<span class="wf-file-stats"><span class="add">+${adds}</span> <span class="del">−${dels}</span>` +
      (openHere ? ` · 💬${openHere}` : "") +
      (collapsed && changed > WF_FILE_COLLAPSE_LINES ? ` · big file (${changed} lines)` : "") +
      `</span>`;
    fileEl.appendChild(head);

    const bodyEl = document.createElement("div");
    bodyEl.className = "wf-file-body" + (collapsed ? " hidden" : "");
    if (!collapsed) wfFillFileBody(bodyEl, f, fileAnns, opts);
    else bodyEl.dataset.lazy = "1";
    fileEl.appendChild(bodyEl);
    container.appendChild(fileEl);
  }
  wfRenderOrphans(container, files, anchored, opts);
}

/// Fill a file's hunks (lazily for collapsed big files).
function wfFillFileBody(bodyEl, f, fileAnns, opts) {
  const frag = document.createDocumentFragment();
  const path = diffFilePath(f);
  for (const h of f.hunks) {
    const hunkEl = document.createElement("div");
    hunkEl.className = "wf-hunk";
    const hh = document.createElement("div");
    hh.className = "wf-hunk-head";
    hh.textContent = h.header;
    hunkEl.appendChild(hh);
    for (const l of h.lines) {
      const side = l.kind === "del" ? "old" : "new";
      const lineNo = side === "old" ? l.oldNo : l.newNo;
      const row = document.createElement("div");
      row.className = `wf-line ${l.kind}`;
      row.dataset.file = path;
      row.dataset.side = side;
      row.dataset.line = String(lineNo ?? "");
      row.dataset.hunk = h.header;
      row.innerHTML =
        `<span class="wf-gut">${l.oldNo ?? ""}</span>` +
        `<span class="wf-gut">${l.newNo ?? ""}</span>` +
        `<span class="wf-plus" data-wf-act="compose" title="Comment on this line">+</span>` +
        `<span class="wf-text">${escapeHtml(l.text)}</span>`;
      hunkEl.appendChild(row);
      // Threads anchored to this exact line render right under it.
      for (const a of fileAnns) {
        if (a.orphaned || a.currentLine !== lineNo) continue;
        if ((a.annotation.side === "old" ? "old" : "new") !== side) continue;
        hunkEl.appendChild(wfBuildThread(a, opts));
      }
    }
    frag.appendChild(hunkEl);
  }
  bodyEl.appendChild(frag);
}

/// Annotations whose anchor no longer resolves — never dropped, rendered in
/// a tray at the end of the diff with their stored context.
function wfRenderOrphans(container, files, anchored, opts) {
  const orphans = anchored.filter((a) => a.orphaned);
  if (!orphans.length) return;
  const tray = document.createElement("div");
  tray.className = "wf-orphans";
  tray.innerHTML = `<div class="wf-orphans-head">💬 ${orphans.length} comment${
    orphans.length > 1 ? "s" : ""
  } no longer anchored to the current diff</div>`;
  for (const a of orphans) {
    const wrap = document.createElement("div");
    wrap.className = "wf-orphan";
    wrap.innerHTML = `<div class="wf-orphan-ctx">${escapeHtml(a.annotation.file)}:${
      a.annotation.line
    } — <code>${escapeHtml(a.annotation.lineContent || "")}</code></div>`;
    wrap.appendChild(wfBuildThread(a, opts));
    tray.appendChild(wrap);
  }
  container.appendChild(tray);
}

/// Thread rendering + composer land in the next commit; the read-only view
/// shows existing comments inline.
function wfBuildThread(a, opts) {
  const ann = a.annotation;
  const t = document.createElement("div");
  t.className = `wf-thread ${ann.status}`;
  t.dataset.annId = ann.id;
  const moved =
    !a.orphaned && a.currentLine !== null && a.currentLine !== ann.line
      ? `<span class="dim" title="re-anchored by content">moved from L${ann.line}</span>`
      : "";
  t.innerHTML =
    `<div class="wf-thread-head"><span class="wf-ann-state ${ann.status}">${escapeHtml(
      ann.status
    )}</span><span class="wf-ann-author">${escapeHtml(ann.author || "user")}</span>` +
    `<span class="dim">it.${ann.iteration || 1}</span>${moved}</div>`;
  const bodyEl = document.createElement("div");
  bodyEl.className = "wf-thread-body wf-md";
  renderMarkdown(bodyEl, ann.body || "");
  t.appendChild(bodyEl);
  for (const r of ann.replies || []) {
    const rep = document.createElement("div");
    rep.className = "wf-reply";
    const repBody = document.createElement("span");
    repBody.className = "wf-md";
    renderMarkdown(repBody, r.body || "");
    rep.innerHTML = `<span class="wf-ann-author">${escapeHtml(r.author || "")}</span>`;
    rep.appendChild(repBody);
    t.appendChild(rep);
  }
  if (!opts.locked) {
    const actions = document.createElement("div");
    actions.className = "wf-thread-actions";
    const btn = (act, label, danger = false) =>
      `<button data-wf-act="${act}" data-ann-id="${escapeHtml(ann.id)}"${
        danger ? ' class="danger"' : ""
      }>${label}</button>`;
    actions.innerHTML =
      btn("ann-reply", "Reply") +
      btn("ann-edit", "Edit") +
      (ann.status === "open"
        ? btn("ann-resolve", "Resolve ✓") + btn("ann-wontfix", "Wontfix")
        : btn("ann-reopen", "Reopen")) +
      btn("ann-delete", "Delete", true);
    t.appendChild(actions);
  }
  return t;
}

/// Delegated click handler for the whole diff container.
function wfDiffClick(container, ev, opts) {
  const act = ev.target.closest("[data-wf-act]");
  if (!act) return;
  const kind = act.dataset.wfAct;
  if (kind === "toggle-file") {
    const fileEl = act.closest(".wf-file");
    const bodyEl = fileEl.querySelector(".wf-file-body");
    const caret = act.querySelector(".wf-file-caret");
    const hidden = bodyEl.classList.toggle("hidden");
    caret.textContent = hidden ? "▸" : "▾";
    if (!hidden && bodyEl.dataset.lazy) {
      delete bodyEl.dataset.lazy;
      // Lazy-fill big files on first expand.
      const path = fileEl.dataset.file;
      const f = opts._files?.find((x) => diffFilePath(x) === path);
      if (f) wfFillFileBody(bodyEl, f, (opts._anchored || []).filter((a) => a.currentFile === path), opts);
    }
    return;
  }
  if (opts.locked) return;
  if (kind === "compose") {
    wfOpenComposer(container, act.closest(".wf-line"), opts);
    return;
  }
  if (kind.startsWith("ann-")) {
    const id = act.dataset.annId;
    const a = (opts._anchored || []).find((x) => x.annotation.id === id);
    if (a) wfThreadAction(container, kind, a, act.closest(".wf-thread"), opts);
  }
}

/// Rebuild the item tab after an annotation mutation, keeping the diff
/// sub-view scroll position (composer focus is gone by then by design).
function wfReloadAfterMutation(opts) {
  const body = opts.root.querySelector(".wf-body");
  if (body) opts.ts.scrollTop = body.scrollTop;
  refreshWorkflows();
  buildWorkflowView(opts.root, opts.item.project, opts.item.slug);
}

/// A single composer at a time, inserted right under the clicked line (or a
/// thread for replies/edits). `submit(text)` performs the save.
function wfComposer({ placeholder, initial = "", okLabel = "Comment", onSubmit }) {
  document.querySelectorAll(".wf-composer").forEach((el) => el.remove());
  const box = document.createElement("div");
  box.className = "wf-composer";
  const ta = document.createElement("textarea");
  ta.placeholder = placeholder;
  ta.value = initial;
  ta.rows = 3;
  const actions = document.createElement("div");
  actions.className = "wf-composer-actions";
  const ok = document.createElement("button");
  ok.className = "primary";
  ok.textContent = okLabel;
  const cancel = document.createElement("button");
  cancel.textContent = "Cancel";
  cancel.onclick = () => box.remove();
  ok.onclick = async () => {
    const text = ta.value.trim();
    if (!text) return;
    ok.disabled = true;
    try {
      await onSubmit(text);
    } catch (e) {
      ok.disabled = false;
      uiAlert(`Save failed: ${e}`);
      return;
    }
    box.remove();
  };
  ta.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) ok.click();
    else if (e.key === "Escape") box.remove();
  });
  actions.append(cancel, ok);
  box.append(ta, actions);
  setTimeout(() => ta.focus(), 0);
  return box;
}

/// New line-level comment on the clicked diff row.
function wfOpenComposer(container, row, opts) {
  if (!row) return;
  const { item } = opts;
  const box = wfComposer({
    placeholder: `Comment on ${row.dataset.file}:${row.dataset.line}…`,
    onSubmit: async (text) => {
      await invoke("save_workflow_annotation", {
        project: item.project,
        slug: item.slug,
        annotation: {
          id: "",
          file: row.dataset.file,
          side: row.dataset.side,
          line: parseInt(row.dataset.line || "0", 10),
          hunkHeader: row.dataset.hunk || "",
          lineContent: row.querySelector(".wf-text")?.textContent ?? "",
          lineContentHash: "", // backend computes — single hash impl in Rust
          body: text,
          status: "open",
          author: "user",
          iteration: opts.ts.iteration ?? item.meta.iteration ?? 1,
          createdAt: 0,
          updatedAt: 0,
          replies: [],
        },
      });
      wfReloadAfterMutation(opts);
    },
  });
  row.after(box);
}

/// Thread buttons: reply / edit / resolve / wontfix / reopen / delete.
async function wfThreadAction(container, kind, anchoredAnn, threadEl, opts) {
  const { item } = opts;
  const ann = anchoredAnn.annotation;
  const setStatus = async (status) => {
    await invoke("set_workflow_annotation_status", {
      project: item.project,
      slug: item.slug,
      id: ann.id,
      status,
    });
    wfReloadAfterMutation(opts);
  };
  switch (kind) {
    case "ann-resolve":
      return setStatus("addressed").catch((e) => uiAlert(`Update failed: ${e}`));
    case "ann-wontfix":
      return setStatus("wontfix").catch((e) => uiAlert(`Update failed: ${e}`));
    case "ann-reopen":
      return setStatus("open").catch((e) => uiAlert(`Update failed: ${e}`));
    case "ann-delete": {
      if (!(await uiConfirm("Delete this comment thread?", "Delete"))) return;
      try {
        await invoke("delete_workflow_annotation", {
          project: item.project,
          slug: item.slug,
          id: ann.id,
        });
        wfReloadAfterMutation(opts);
      } catch (e) {
        uiAlert(`Delete failed: ${e}`);
      }
      return;
    }
    case "ann-edit": {
      const box = wfComposer({
        placeholder: "Edit comment…",
        initial: ann.body || "",
        okLabel: "Save",
        onSubmit: async (text) => {
          await invoke("save_workflow_annotation", {
            project: item.project,
            slug: item.slug,
            annotation: { ...ann, body: text },
          });
          wfReloadAfterMutation(opts);
        },
      });
      threadEl.after(box);
      return;
    }
    case "ann-reply": {
      const box = wfComposer({
        placeholder: "Reply…",
        okLabel: "Reply",
        onSubmit: async (text) => {
          const replies = [...(ann.replies || []), { author: "user", body: text, createdAt: Date.now() }];
          await invoke("save_workflow_annotation", {
            project: item.project,
            slug: item.slug,
            annotation: { ...ann, replies },
          });
          wfReloadAfterMutation(opts);
        },
      });
      threadEl.after(box);
      return;
    }
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

/// Native folder picker (tauri-plugin-dialog) seeded from a starting path.
/// Returns the chosen absolute directory, or null when cancelled/unavailable.
async function pickDirectory(defaultPath, title = "Choose a working directory") {
  try {
    const picked = await invoke("plugin:dialog|open", {
      options: {
        directory: true,
        multiple: false,
        defaultPath: (defaultPath || "").trim() || state.homeDir || undefined,
        title,
      },
    });
    return typeof picked === "string" ? picked : null;
  } catch (e) {
    console.error("folder picker failed:", e);
    return null;
  }
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
$("ns-browse").innerHTML = svgIcon("folder", 14);
$("ns-browse").onclick = async () => {
  const dir = await pickDirectory($("ns-cwd").value);
  if (dir) {
    $("ns-cwd").value = dir;
    loadPresetsForCwd();
  }
};
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
      }).then(async (restart) => {
        if (!restart) return;
        // Persist "where we were" BEFORE the re-exec. app.restart() (see the
        // backend restart_app) replaces the process without firing any window
        // teardown events, so the blur/pagehide/beforeunload flushes never run
        // on this path — without this await, the restored layout is stale and
        // sessions don't come back where we left them.
        await flushWorkspaces();
        invoke("restart_app").catch((e) => uiAlert(`Restart failed: ${e}`));
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
  // Reopen shows the whole list — a leftover filter would look like the
  // settings had vanished.
  if (want && $("settings-filter").value) {
    $("settings-filter").value = "";
    filterSettings("");
  }
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

// ── Font picker ─────────────────────────────────────────────────
// The font field is read-only and opens a searchable list instead: typing a
// family name blind (and getting a silent fallback when it's misspelled) is the
// worst way to pick a font. Families come from the OS via `list_font_families`
// (AppKit on macOS); if that yields nothing, a curated set is probed with
// `document.fonts.check`, which confirms a family without loading it.

const FONT_CANDIDATES = [
  "SF Mono", "Menlo", "Monaco", "JetBrains Mono", "JetBrains Mono NL", "Fira Code",
  "Fira Mono", "Hack", "Source Code Pro", "IBM Plex Mono", "Cascadia Code",
  "Cascadia Mono", "Consolas", "Inconsolata", "Ubuntu Mono", "DejaVu Sans Mono",
  "Roboto Mono", "Iosevka", "Iosevka Term", "Victor Mono", "Geist Mono",
  "Berkeley Mono", "MesloLGS NF", "Liberation Mono", "PT Mono", "Space Mono",
  "Noto Sans Mono", "Andale Mono", "Courier New", "Courier", "Menlo Regular",
  "Operator Mono", "Dank Mono", "Comic Mono", "Anonymous Pro", "Monoid",
  "Terminus", "ProggyClean", "Go Mono", "Nimbus Mono PS", "Recursive Mono",
];

let fontFamilyCache = null; // [{ name, mono }] — resolved once per launch

/// Monospace test: in a fixed-pitch family every glyph advances the same, so a
/// narrow and a wide character measure identically. Canvas keeps this cheap
/// enough to run over a few hundred families.
function isMonospaceFamily(name, ctx) {
  ctx.font = `16px "${name}", serif`;
  const i = ctx.measureText("iiiiiiiiii").width;
  const w = ctx.measureText("WWWWWWWWWW").width;
  if (!i || !w) return false;
  return Math.abs(i - w) < 0.5;
}

/// Every family we can offer, monospace flagged. Cached: enumeration hops to the
/// main thread and the measuring loop is pure work — neither changes mid-launch.
///
/// The list is a union, because neither source alone is complete: AppKit knows
/// the installed families but omits system faces the webview can still render
/// (`SF Mono` — clash's own default — is absent from `availableFontFamilies`),
/// while the curated probe only ever finds fonts someone thought to list. The
/// configured family is added too, so the current choice is always in its list.
async function loadFontFamilies() {
  if (fontFamilyCache) return fontFamilyCache;
  let names = [];
  try {
    names = (await invoke("list_font_families")) || [];
  } catch (e) {
    console.error("list_font_families failed:", e);
  }
  const probed = FONT_CANDIDATES.filter((f) => {
    try {
      return document.fonts.check(`12px "${f}"`);
    } catch (e) {
      void e;
      return false;
    }
  });
  // A stack ("SF Mono, Menlo, monospace") isn't a family — it can't be measured
  // or previewed as one, and "Custom…" is where stacks are edited.
  const current = state.settings.fontFamily.includes(",") ? [] : [state.settings.fontFamily];
  const seen = new Set();
  const merged = [];
  for (const name of [...names, ...probed, ...current]) {
    const key = name.trim().toLowerCase();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    merged.push(name.trim());
  }
  merged.sort((a, b) => a.localeCompare(b));
  const ctx = document.createElement("canvas").getContext("2d");
  fontFamilyCache = merged.map((name) => ({
    name,
    mono: ctx ? isMonospaceFamily(name, ctx) : /mono|code|courier|consol/i.test(name),
  }));
  return fontFamilyCache;
}

/// Modal font list: search box, monospace families first (they are what a
/// terminal wants), each row previewed in its own face. Resolves to the chosen
/// family, or null when cancelled. "Custom…" hands over to a text prompt so a
/// full fallback stack ("SF Mono, Menlo, monospace") stays expressible.
async function pickFontFamily(current) {
  const families = await loadFontFamilies();
  return new Promise((resolve) => {
    const backdrop = document.createElement("div");
    backdrop.className = "dialog-backdrop";
    const box = document.createElement("div");
    box.className = "dialog-box font-picker";
    const msg = document.createElement("p");
    msg.textContent = "Terminal font";
    box.appendChild(msg);

    const search = document.createElement("input");
    search.type = "search";
    search.placeholder = "Search fonts…";
    search.spellcheck = false;
    box.appendChild(search);

    const monoOnly = document.createElement("label");
    monoOnly.className = "setting-check";
    const monoBox = document.createElement("input");
    monoBox.type = "checkbox";
    monoBox.checked = true;
    monoOnly.appendChild(monoBox);
    monoOnly.appendChild(document.createTextNode(" Monospace only"));
    box.appendChild(monoOnly);

    const list = document.createElement("div");
    list.className = "dialog-list";
    box.appendChild(list);

    const done = (val) => {
      backdrop.remove();
      resolve(val);
      if (typeof fitAll === "function") fitAll();
    };

    const render = () => {
      const q = search.value.trim().toLowerCase();
      list.innerHTML = "";
      const shown = families
        .filter((f) => (monoBox.checked ? f.mono : true))
        .filter((f) => !q || f.name.toLowerCase().includes(q))
        .sort((a, b) => Number(b.mono) - Number(a.mono) || a.name.localeCompare(b.name));
      if (!shown.length) {
        const empty = document.createElement("div");
        empty.className = "dialog-list-detail";
        empty.textContent = monoBox.checked
          ? "No monospace family matches — try unchecking “Monospace only”."
          : "No font matches.";
        list.appendChild(empty);
      }
      for (const f of shown) {
        const row = document.createElement("div");
        row.className = "dialog-list-row";
        if (f.name === current) row.classList.add("current");
        const label = document.createElement("div");
        label.className = "dialog-list-label";
        label.textContent = f.name;
        row.appendChild(label);
        // Preview in the face itself — the whole point of a picker.
        const sample = document.createElement("div");
        sample.className = "font-sample";
        sample.style.fontFamily = `"${f.name}", monospace`;
        sample.textContent = "if (x === 0) { i1lO0 —> ~/.claude }";
        row.appendChild(sample);
        row.onclick = () => done(f.name);
        list.appendChild(row);
      }
    };
    search.addEventListener("input", render);
    monoBox.addEventListener("change", render);
    render();

    const actions = document.createElement("div");
    actions.className = "modal-actions";
    const cancel = document.createElement("button");
    cancel.textContent = "Cancel";
    cancel.onclick = () => done(null);
    actions.appendChild(cancel);
    const custom = document.createElement("button");
    custom.textContent = "Custom…";
    custom.title = "Type a font stack by hand";
    custom.onclick = async () => {
      backdrop.remove();
      const typed = await uiPrompt("Font family (CSS font stack)", current);
      resolve(typed === null ? null : typed.trim() || null);
      if (typeof fitAll === "function") fitAll();
    };
    actions.appendChild(custom);
    box.appendChild(actions);

    backdrop.appendChild(box);
    if (typeof hideBrowserWebviews === "function") hideBrowserWebviews();
    document.body.appendChild(backdrop);
    backdrop.onclick = (e) => {
      if (e.target === backdrop) done(null);
    };
    backdrop.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Escape") done(null);
      // Enter takes the first row — search, Enter, done.
      else if (e.key === "Enter") {
        const first = list.querySelector(".dialog-list-row .dialog-list-label");
        if (first) done(first.textContent);
      }
    });
    setTimeout(() => search.focus(), 0);
  });
}

/// Point the app's `--mono` at the terminal font, so in-app monospace text
/// (markdown code, diffs, paths) is the same face as the terminals.
function applyMonoFont() {
  document.documentElement.style.setProperty(
    "--mono",
    `${state.settings.fontFamily}, monospace`
  );
}

async function openFontPicker() {
  const picked = await pickFontFamily(state.settings.fontFamily);
  if (!picked) return;
  state.settings.fontFamily = picked;
  $("set-fontfamily").value = picked;
  applyTermOption("fontFamily", picked);
  applyMonoFont();
  fitAll();
}

// Both the (read-only) field and its button open the picker. The button sits
// inside the row's <label>, so the click is stopped here — otherwise a browser
// that forwards label clicks to the labeled input would open two pickers.
$("set-fontfamily").onclick = openFontPicker;
$("set-fontfamily-pick").innerHTML = svgIcon("search", 14);
$("set-fontfamily-pick").onclick = (e) => {
  e.preventDefault();
  e.stopPropagation();
  openFontPicker();
};

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


// Workflows sidebar section.
$("wf-toggle").onclick = toggleWorkflows;
$("refresh-wf-btn").innerHTML = svgIcon("reload", 13);
$("refresh-wf-btn").onclick = (e) => {
  e.stopPropagation();
  spinButton($("refresh-wf-btn"), async () => {
    if (!state.wfOpen) await toggleWorkflows(); // opening already refreshes
    else await refreshWorkflows();
  });
};
$("new-wf-btn").onclick = (e) => {
  e.stopPropagation();
  newWorkflowFlow();
};
$("wf-board-btn").onclick = (e) => {
  e.stopPropagation();
  openWorkflowBoardTab();
};
$("wf-skills-btn").innerHTML = svgIcon("zap", 13);
$("wf-skills-btn").onclick = (e) => {
  e.stopPropagation();
  openSkillsTab();
};
// Backend workflows watcher: refresh the sidebar list and rebuild any open
// workflow tabs. The rebuild refetches only the active sub-view's data
// (buildWorkflowView is sub-view-scoped), so an agent write burst never
// spawns git subprocesses for invisible diffs.
listen("workflows-changed", async () => {
  await refreshWorkflows();
  rebuildOpenWorkflowTabs();
});

/// Rebuild every open workflow tab in place, preserving sub-view + scroll.
/// Skipped while a comment composer has focus — a rebuild would eat the
/// user's draft; a banner offers a manual refresh instead.
function rebuildOpenWorkflowTabs() {
  const board = state.open.get("view:wfboard");
  if (board) renderWorkflowBoard(board.el);
  for (const [key, entry] of state.open) {
    if (!key.startsWith("view:workflow:")) continue;
    const rest = key.slice("view:workflow:".length);
    const slash = rest.indexOf("/");
    if (slash < 0) continue;
    const project = rest.slice(0, slash);
    const slug = rest.slice(slash + 1);
    const el = entry.el;
    const composer = el.querySelector(".wf-composer");
    if (composer && composer.contains(document.activeElement)) {
      if (!el.querySelector(".wf-stale-banner")) {
        const banner = document.createElement("div");
        banner.className = "wf-stale-banner";
        banner.innerHTML = `content changed on disk — <a href="#">refresh</a>`;
        banner.querySelector("a").onclick = (ev) => {
          ev.preventDefault();
          buildWorkflowView(el, project, slug);
        };
        el.prepend(banner);
      }
      continue;
    }
    const body = el.querySelector(".wf-body");
    const ts = wfTabState.get(wfKey(project, slug));
    if (body && ts) ts.scrollTop = body.scrollTop;
    buildWorkflowView(el, project, slug);
  }
}

// Decision-needed transitions (agent writes only — clash's own clicks are
// suppressed backend-side by the AttentionLedger): badge the row unless the
// item's tab is visible and the window focused; always toast. The native
// desktop notification fires backend-side with the same suppression rules
// as sessions.
listen("workflow-attention", (event) => {
  const { project, slug, title, status } = event.payload;
  const key = wfKey(project, slug);
  const tabVisible =
    ws().panes.includes(`view:workflow:${key}`) && document.hasFocus();
  if (!tabVisible) {
    state.wfUnread.add(key);
    if (state.wfOpen) renderWorkflows();
    else updateWfBadge();
  }
  flashToast(`${title || slug}: ${wfStatusInfo(status).label} — decision needed`);
});

// Lazy PR polling: while any pr-draft/pr-ready item is on screen (sidebar
// section or open tab), refresh its recorded PR state once a minute. The
// backend throttles to one gh call per 30s per item and only writes meta on
// actual change, so this never churns the FS watcher.
setInterval(() => {
  for (const item of state.workflows) {
    const st = item.meta.status;
    if (st !== "pr-draft" && st !== "pr-ready") continue;
    if (!item.meta.pr || !item.meta.pr.url) continue;
    const key = wfKey(item.project, item.slug);
    const visible = state.wfOpen || state.open.has(`view:workflow:${key}`);
    if (!visible) continue;
    invoke("refresh_workflow_pr", {
      project: item.project,
      slug: item.slug,
      force: false,
    }).catch(() => {}); // gh absent → buttons already degrade; polling stays quiet
  }
}, 60_000);

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
  applySectionHeight("wf-section", "wf-resizer", state.wfOpen, "wfHeight");
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
initSectionResizer("wf-resizer", "wf-section", "wfHeight");
reapplySectionHeights();
window.addEventListener("resize", reapplySectionHeights);

$("new-ws-btn").onclick = newWorkspace;

// ── Settings (sidebar footer) ───────────────────────────────────

$("default-cwd").addEventListener("change", () => {
  state.settings.defaultCwd = $("default-cwd").value.trim();
  persistSetting("defaultCwd");
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

/// Same contract for the workflows directory (dedicated root, config.toml).
$("set-workflows-dir").addEventListener("change", async () => {
  const el = $("set-workflows-dir");
  try {
    el.value = await invoke("set_workflows_dir", { path: el.value.trim() });
    if (state.wfOpen) await refreshWorkflows();
  } catch (e) {
    uiAlert(`Workflows directory: ${e}`);
    try {
      el.value = await invoke("get_workflows_dir");
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
  try {
    $("set-workflows-dir").value = await invoke("get_workflows_dir");
  } catch (e) {
    console.error("get_workflows_dir failed:", e);
  }
}

/// Reflect persisted settings into the footer controls.
function syncSettingsUi() {
  const s = state.settings;
  const theme = $("set-theme");
  if (theme) {
    // Built once, from the theme table — dark first, then light, each group
    // alphabetical, with the two clash themes pinned to the top of theirs.
    if (!theme.options.length) {
      const rank = (id) => (id.startsWith("clash-") ? 0 : 1);
      const entries = Object.entries(THEMES).sort(
        (a, b) =>
          Number(b[1].dark) - Number(a[1].dark) ||
          rank(a[0]) - rank(b[0]) ||
          a[1].label.localeCompare(b[1].label)
      );
      for (const [id, t] of entries) {
        const o = document.createElement("option");
        o.value = id;
        o.textContent = `${t.label} ${t.dark ? "◐" : "◑"}`;
        theme.appendChild(o);
      }
    }
    theme.value = s.theme;
  }
  $("set-fontfamily").value = s.fontFamily;
  $("set-fontsize").value = s.fontSize;
  $("set-font-weight").value = s.fontWeight;
  $("set-font-weight-bold").value = s.fontWeightBold;
  $("set-line-height").value = s.lineHeight;
  $("set-letter-spacing").value = s.letterSpacing;
  $("set-cursor-style").value = s.cursorStyle;
  $("set-cursor-inactive").value = s.cursorInactiveStyle;
  $("set-cursor-width").value = s.cursorWidth;
  $("set-cursor-blink").checked = s.cursorBlink;
  $("set-min-contrast").value = s.minimumContrast;
  $("set-bright-bold").checked = s.brightBold;
  $("set-scrollback").value = s.scrollback;
  $("set-scroll-speed").value = s.scrollSpeed;
  $("set-smooth-scroll").value = s.smoothScroll;
  $("set-copy-select").checked = s.copyOnSelect;
  $("set-rclick-word").checked = s.rightClickWord;
  $("set-option-meta").checked = s.optionMeta;
  $("set-bell-toast").checked = s.bellToast;
  $("set-link-open").value = s.linkOpen;
  $("set-notify").checked = s.notifications;
  $("set-title-attention").checked = s.titleAttention;
  $("set-confirm-kill").checked = s.confirmKill;
  $("set-refresh-secs").value = s.refreshSecs;
  syncPickerSelects();
}

/// Fill the shell / TUI-terminal selects from what the backend detected. Called
/// again once detection lands, since it resolves after the first sync.
function syncPickerSelects() {
  const shell = $("set-term-shell");
  if (shell) {
    shell.innerHTML = "";
    for (const [value, label] of [["", "Default ($SHELL)"], ...detectedShells.map((s) => [s, s])]) {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = label;
      shell.appendChild(o);
    }
    shell.value = detectedShells.includes(state.settings.termShell)
      ? state.settings.termShell
      : "";
  }
  const tui = $("set-tui-terminal");
  if (tui) {
    tui.innerHTML = "";
    const opts = [["", "Auto — split pane or default"], ...detectedTerminals.map((t) => [t.id, t.name])];
    for (const [value, label] of opts) {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = label;
      tui.appendChild(o);
    }
    tui.value = detectedTerminals.some((t) => t.id === state.settings.tuiTerminal)
      ? state.settings.tuiTerminal
      : "";
  }
}

/// Filter the settings list as you type: hide rows whose label doesn't match,
/// and hide a group header when everything under it is hidden. Empty = show all.
function filterSettings(query) {
  const q = query.trim().toLowerCase();
  const rows = [...$("settings-body").children];
  let lastGroup = null;
  let groupHasMatch = false;
  const flushGroup = () => {
    if (lastGroup) lastGroup.classList.toggle("hidden", !groupHasMatch);
  };
  for (const el of rows) {
    if (el.classList.contains("settings-group")) {
      flushGroup();
      lastGroup = el;
      groupHasMatch = false;
      continue;
    }
    if (el.id === "settings-filter" || el.id === "update-btn") continue;
    const match = !q || el.textContent.toLowerCase().includes(q);
    el.classList.toggle("hidden", !match);
    if (match) groupHasMatch = true;
  }
  flushGroup();
}

/// Live-apply an xterm option to every open terminal, then persist.
// ── Live config reload ──────────────────────────────────────────
// config.toml is watched, so an edit by hand, by the TUI, or by another clash
// instance lands here without a restart.

/// Pending refit request, coalesced to one animation frame.
///
/// A reload fans out: re-read → diff → push xterm options → possibly refit every
/// open terminal. At the default 200 ms watcher debounce, an editor that saves on
/// every keystroke would otherwise turn one burst of saves into a burst of refits
/// across every pane. Only keys the schema marks `x-refit` (font, line height,
/// letter spacing) enter that path at all, and never more than once per frame.
let refitQueued = false;
function queueRefit() {
  if (refitQueued) return;
  refitQueued = true;
  requestAnimationFrame(() => {
    refitQueued = false;
    fitAll();
  });
}

/// Show a config load error as a toast. The backend keeps serving the last good
/// values, so this is the only signal that the file on disk is broken — the old
/// behaviour logged a warning nobody could see and silently used defaults.
function showConfigError(message) {
  flashToast(`Config error — ${message}`);
}

/// Apply a `config-changed` event: the changed dotted paths, the subset needing
/// a refit, and the new effective shared settings.
function applyConfigChange(payload) {
  const changed = (payload && payload.changed) || [];
  const settings = (payload && payload.settings) || {};
  if (!changed.length) return;

  Object.assign(state.settings, settings);
  syncSettingsUi();
  $("default-cwd").value = state.settings.defaultCwd;

  // Keys with a live effect beyond the settings panel.
  if (changed.includes("sessions.refresh_secs")) restartSessionPoll();
  if (changed.includes("notifications.enabled")) {
    invoke("set_notifications_enabled", { enabled: state.settings.notifications }).catch(
      console.error
    );
  }
  if (changed.includes("notifications.title_attention")) refreshSessions();

  if (payload && payload.refit && payload.refit.length) queueRefit();
  dlog(`config reloaded: ${changed.join(", ")}`);
}

function listenForConfigReload() {
  listen("config-changed", (e) => applyConfigChange(e.payload)).catch((err) =>
    console.error("config-changed listen failed:", err)
  );
  listen("config-error", (e) => showConfigError(e.payload)).catch((err) =>
    console.error("config-error listen failed:", err)
  );
}

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
  persistSetting("notifications");
});

// ── Settings: generic wiring ────────────────────────────────────
// Everything below is declarative: one row per control, so a new setting is a
// line here plus a line in syncSettingsUi — no bespoke handler each time.

/// Bind a numeric input to a setting. Out-of-range or non-numeric input snaps
/// back to the stored value. `option` live-applies to open terminals (xterm
/// option name); `refit` re-measures the grid when metrics changed.
function bindNumberSetting(id, key, { min, max, step = 1, option = null, refit = false }) {
  const el = $(id);
  if (!el) return;
  el.addEventListener("change", () => {
    const raw = Number(el.value);
    const v = step < 1 ? Math.round(raw / step) * step : Math.round(raw);
    if (!Number.isFinite(v) || v < min || v > max) {
      el.value = state.settings[key];
      return;
    }
    // Floating-point steps (line height 1.05) accumulate noise — pin to 2dp.
    state.settings[key] = step < 1 ? Number(v.toFixed(2)) : v;
    el.value = state.settings[key];
    if (option) applyTermOption(option, state.settings[key]);
    else persistSetting(key);
    if (refit) fitAll();
  });
}

/// Bind a checkbox or select to a setting; `option` live-applies to terminals.
function bindChoiceSetting(id, key, { option = null, refit = false, onChange = null } = {}) {
  const el = $(id);
  if (!el) return;
  el.addEventListener("change", () => {
    state.settings[key] = el.type === "checkbox" ? el.checked : el.value;
    if (option) applyTermOption(option, state.settings[key]);
    else persistSetting(key);
    if (refit) fitAll();
    if (onChange) onChange(state.settings[key]);
  });
}

// Appearance
bindChoiceSetting("set-theme", "theme", {
  onChange: (id) => {
    applyTheme(id);
    // Terminal colours changed under the WebGL renderer's texture atlas.
    for (const entry of state.open.values()) {
      if (entry.term) entry.term.clearTextureAtlas?.();
    }
  },
});
// Terminal · text (metrics changes need a refit so cols/rows track the cell box)
bindChoiceSetting("set-font-weight", "fontWeight", { option: "fontWeight", refit: true });
bindChoiceSetting("set-font-weight-bold", "fontWeightBold", {
  option: "fontWeightBold",
  refit: true,
});
bindNumberSetting("set-line-height", "lineHeight", {
  min: 1,
  max: 2,
  step: 0.05,
  option: "lineHeight",
  refit: true,
});
bindNumberSetting("set-letter-spacing", "letterSpacing", {
  min: -2,
  max: 5,
  step: 0.5,
  option: "letterSpacing",
  refit: true,
});
// Terminal · cursor
bindChoiceSetting("set-cursor-inactive", "cursorInactiveStyle", {
  option: "cursorInactiveStyle",
});
bindNumberSetting("set-cursor-width", "cursorWidth", { min: 1, max: 5, option: "cursorWidth" });
// Terminal · colors
bindNumberSetting("set-min-contrast", "minimumContrast", {
  min: 1,
  max: 21,
  step: 0.5,
  option: "minimumContrastRatio",
});
bindChoiceSetting("set-bright-bold", "brightBold", { option: "drawBoldTextInBrightColors" });
// Terminal · scroll & input
bindNumberSetting("set-scroll-speed", "scrollSpeed", {
  min: 1,
  max: 10,
  option: "scrollSensitivity",
});
bindNumberSetting("set-smooth-scroll", "smoothScroll", {
  min: 0,
  max: 500,
  option: "smoothScrollDuration",
});
bindChoiceSetting("set-rclick-word", "rightClickWord", { option: "rightClickSelectsWord" });
bindChoiceSetting("set-bell-toast", "bellToast");
// clash
bindChoiceSetting("set-title-attention", "titleAttention", {
  // Drop a stale "(2!)" immediately when the marker is switched off.
  onChange: () => refreshSessions(),
});
bindChoiceSetting("set-confirm-kill", "confirmKill");
bindNumberSetting("set-refresh-secs", "refreshSecs", { min: 1, max: 30 });
// Second listener, after the bind above has validated and stored the value.
$("set-refresh-secs").addEventListener("change", restartSessionPoll);
bindChoiceSetting("set-term-shell", "termShell");
bindChoiceSetting("set-tui-terminal", "tuiTerminal");

/// The `claude` binary lives in config.toml (shared with the TUI), so it round
/// trips through the backend, which validates an absolute path and echoes the
/// effective value back.
$("set-claude-bin").addEventListener("change", async () => {
  const el = $("set-claude-bin");
  try {
    el.value = await invoke("set_claude_bin", { path: el.value.trim() });
  } catch (e) {
    uiAlert(`Claude binary: ${e}`);
    try {
      el.value = await invoke("get_claude_bin");
    } catch (_) {}
  }
});

/// Folder pickers for every directory setting: fill the field, then fire the
/// same `change` handler typing would (so config-backed rows still persist).
for (const id of ["default-cwd", "set-scratch-dir", "set-workflows-dir"]) {
  const btn = $(`${id}-browse`);
  if (!btn) continue;
  btn.innerHTML = svgIcon("folder", 14);
  btn.onclick = async (e) => {
    e.preventDefault(); // inside the row's <label> — don't re-trigger the input
    e.stopPropagation();
    const dir = await pickDirectory($(id).value);
    if (!dir) return;
    $(id).value = dir;
    $(id).dispatchEvent(new Event("change"));
  };
}

$("settings-filter").addEventListener("input", () => filterSettings($("settings-filter").value));

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

/// (Re)start the session-list poll at the configured cadence. One handle, so
/// changing the interval in Settings never leaves two timers running.
let sessionPoll = null;
function restartSessionPoll() {
  clearInterval(sessionPoll);
  sessionPoll = setInterval(refreshSessions, state.settings.refreshSecs * 1000);
}

(async () => {
  applyStaticIcons(); // before first paint — never show the unicode fallbacks
  await loadWorkspaces(); // disk-backed — must complete before first render
  // Settings come from the backend (config.toml + the schema-validated
  // GUI-local half), so this must land before anything reads state.settings.
  await loadSettings();
  listenForConfigReload();
  // Theme and mono font before the first render, so the window never flashes
  // the default palette on the way to the chosen one.
  applyTheme(state.settings.theme);
  applyMonoFont();
  restoreBrowserTabs(); // entries only — webviews materialize on first visibility
  $("default-cwd").value = state.settings.defaultCwd;
  syncSettingsUi();
  loadScratchDir();
  invoke("get_claude_bin")
    .then((b) => ($("set-claude-bin").value = b))
    .catch(() => {});
  if (!state.settings.notifications) {
    invoke("set_notifications_enabled", { enabled: false }).catch(console.error);
  }
  state.homeDir = await invoke("get_home_dir").catch(() => "");
  if (state.homeDir) $("default-cwd").placeholder = state.homeDir;
  renderAll();
  setVersionLabel();
  refreshTuiIndicator();
  setInterval(refreshTuiIndicator, 5000);
  // Detection resolves after the first syncSettingsUi, so refill the selects
  // that are built from it once the lists land.
  invoke("list_terminals")
    .then((t) => {
      detectedTerminals = t;
      syncPickerSelects();
    })
    .catch(() => {});
  invoke("list_shells")
    .then((s) => {
      detectedShells = s;
      syncPickerSelects();
    })
    .catch(() => {});
  await refreshSessions();
  await restoreWorkspaceSessions();
  restartSessionPoll();
  // One line in clash.log per launch saying the webview got all the way through
  // boot. Without it, a frontend that dies early is indistinguishable from a
  // backend problem: WKWebView has no visible console, so the only symptom is a
  // blank or half-drawn window and a log that looks perfectly healthy.
  dlog(
    `frontend booted: ${state.workspaces.length} workspace(s), ` +
      `${state.sessions.length} session(s), theme ${state.settings.theme} ` +
      `(bg ${getComputedStyle(document.documentElement).getPropertyValue("--bg").trim()}), ` +
      `font "${state.settings.fontFamily}" ${state.settings.fontSize}px, ` +
      `refresh ${state.settings.refreshSecs}s`
  );
})();
