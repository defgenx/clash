// clash GUI frontend — cmux-style sidebar + xterm.js terminal panes.
// No build step: plain JS against the Tauri global API (withGlobalTauri).

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = {
  sessions: [],
  selected: null, // session id shown in the terminal host
  open: new Map(), // session id -> { term, fitAddon, pane, name }
};

// ── Session sidebar ─────────────────────────────────────────────

// Mirror of SessionStatus serde values (Stashed -> "idle", Done -> "done").
function statusClass(s) {
  if (s.is_running) {
    switch (s.status) {
      case "Prompting":
      case "Waiting":
        return "prompting";
      case "Thinking":
        return "thinking";
      default:
        return "running";
    }
  }
  if (s.status === "Errored") return "errored";
  return "done";
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

function renderSidebar() {
  const list = document.getElementById("session-list");
  const sections = { ACTIVE: [], FAILED: [], STASHED: [], DONE: [] };
  for (const s of state.sessions) sections[sectionOf(s)].push(s);

  list.innerHTML = "";
  for (const [label, items] of Object.entries(sections)) {
    if (items.length === 0) continue;
    const header = document.createElement("div");
    header.className = "section-label";
    header.textContent = label;
    list.appendChild(header);

    for (const s of items) {
      const item = document.createElement("div");
      item.className = "session-item" + (s.id === state.selected ? " selected" : "");
      item.onclick = () => openSession(s);

      const ring = document.createElement("div");
      ring.className = "status-ring " + statusClass(s);

      const meta = document.createElement("div");
      meta.className = "session-meta";

      const name = document.createElement("div");
      name.className = "session-name";
      name.textContent = displayName(s);

      const sub = document.createElement("div");
      sub.className = "session-sub";
      if (s.git_branch) {
        const branch = document.createElement("span");
        branch.className = "branch";
        branch.textContent = s.git_branch;
        sub.appendChild(branch);
      }
      const proj = document.createElement("span");
      proj.textContent = s.worktree_project || s.project;
      sub.appendChild(proj);

      meta.appendChild(name);
      meta.appendChild(sub);
      item.appendChild(ring);
      item.appendChild(meta);
      list.appendChild(item);
    }
  }

  document.getElementById("session-count").textContent =
    `${state.sessions.length} session${state.sessions.length === 1 ? "" : "s"}`;
}

async function refreshSessions() {
  try {
    state.sessions = await invoke("list_sessions");
    renderSidebar();
  } catch (e) {
    console.error("list_sessions failed:", e);
  }
}

// ── Tabs ────────────────────────────────────────────────────────

function renderTabs() {
  const tabs = document.getElementById("tabs");
  tabs.innerHTML = "";
  for (const [id, entry] of state.open) {
    const tab = document.createElement("div");
    tab.className = "tab" + (id === state.selected ? " active" : "");
    tab.onclick = () => selectPane(id);

    const label = document.createElement("span");
    label.textContent = entry.name;

    const close = document.createElement("span");
    close.className = "close";
    close.textContent = "×";
    close.onclick = (ev) => {
      ev.stopPropagation();
      closePane(id);
    };

    tab.appendChild(label);
    tab.appendChild(close);
    tabs.appendChild(tab);
  }
}

function selectPane(id) {
  state.selected = id;
  for (const [sid, entry] of state.open) {
    entry.pane.classList.toggle("visible", sid === id);
  }
  document.getElementById("empty-state").style.display = state.open.size ? "none" : "flex";
  const entry = state.open.get(id);
  if (entry) {
    entry.fitAddon.fit();
    entry.term.focus();
  }
  renderTabs();
  renderSidebar();
}

async function closePane(id) {
  const entry = state.open.get(id);
  if (!entry) return;
  try {
    await invoke("close_session", { sessionId: id });
  } catch (e) {
    console.error("close_session failed:", e);
  }
  entry.term.dispose();
  entry.pane.remove();
  state.open.delete(id);
  if (state.selected === id) {
    const next = state.open.keys().next();
    state.selected = next.done ? null : next.value;
  }
  selectPane(state.selected);
}

// ── Terminal panes ──────────────────────────────────────────────

const TERM_THEME = {
  background: "#141414",
  foreground: "#d4d4d8",
  cursor: "#e8a33d",
  selectionBackground: "#3a3a40",
};

async function openSession(s) {
  if (state.open.has(s.id)) {
    selectPane(s.id);
    return;
  }

  const host = document.getElementById("terminal-host");
  const pane = document.createElement("div");
  pane.className = "term-pane";
  host.appendChild(pane);

  const term = new Terminal({
    fontFamily: "SF Mono, Menlo, monospace",
    fontSize: 13,
    theme: TERM_THEME,
    scrollback: 10000,
    macOptionIsMeta: true,
  });
  const fitAddon = new FitAddon.FitAddon();
  term.loadAddon(fitAddon);
  term.open(pane);
  fitAddon.fit();

  state.open.set(s.id, { term, fitAddon, pane, name: displayName(s) });

  try {
    await invoke("open_session", {
      sessionId: s.id,
      cols: term.cols,
      rows: term.rows,
    });
  } catch (e) {
    term.writeln(`\x1b[31mFailed to open session: ${e}\x1b[0m`);
  }

  term.onData((data) => {
    invoke("send_input", { sessionId: s.id, text: data }).catch(console.error);
  });
  term.onResize(({ cols, rows }) => {
    invoke("resize_session", { sessionId: s.id, cols, rows }).catch(() => {});
  });

  selectPane(s.id);
}

// Refit the visible terminal when the window resizes.
window.addEventListener("resize", () => {
  const entry = state.open.get(state.selected);
  if (entry) entry.fitAddon.fit();
});

// ── PTY event stream ────────────────────────────────────────────

function base64ToBytes(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

listen("pty-output", (event) => {
  const { session_id, data } = event.payload;
  const entry = state.open.get(session_id);
  if (entry) entry.term.write(base64ToBytes(data));
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

// ── Boot ────────────────────────────────────────────────────────

refreshSessions();
setInterval(refreshSessions, 2000);
