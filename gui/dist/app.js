// clash GUI frontend — cmux-style sidebar + split terminal panes.
// No build step: plain JS against the Tauri global API (withGlobalTauri).

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = {
  sessions: [],
  query: "",
  open: new Map(), // session id -> { term, fitAddon, el, name }
  panes: [null], // pane slots -> session id or null
  focusedPane: 0,
  activeTab: null, // session id highlighted in the tab bar
  detailsFor: null, // session id shown in the details panel, or null
  teams: [],
  teamsOpen: false,
  renaming: null, // session id with an open inline-rename input
  prevStatuses: new Map(), // session id -> status (attention transitions)
};

const $ = (id) => document.getElementById(id);

// ── Session helpers ─────────────────────────────────────────────

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

function renderSidebar() {
  const list = $("session-list");
  const sections = { ACTIVE: [], FAILED: [], STASHED: [], DONE: [] };
  for (const s of visibleSessions()) sections[sectionOf(s)].push(s);

  list.innerHTML = "";
  for (const [label, items] of Object.entries(sections)) {
    if (items.length === 0) continue;
    const header = document.createElement("div");
    header.className = "section-label";
    header.textContent = label;
    list.appendChild(header);
    for (const s of items) list.appendChild(sessionItem(s));
  }

  const n = state.sessions.length;
  $("session-count").textContent = `${n} session${n === 1 ? "" : "s"}`;
}

function sessionItem(s) {
  const item = document.createElement("div");
  item.className =
    "session-item" + (s.id === state.activeTab ? " selected" : "");
  item.onclick = () => openSession(s.id);

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
        if (!confirm(`Kill session "${displayName(s)}"?`)) return;
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
    renderSidebar();
    if (state.detailsFor) renderDetails();
  } catch (e) {
    console.error("list_sessions failed:", e);
  }
}

// ── Tabs ────────────────────────────────────────────────────────

function renderTabs() {
  const tabs = $("tabs");
  tabs.innerHTML = "";
  for (const [id, entry] of state.open) {
    const tab = document.createElement("div");
    tab.className = "tab" + (id === state.activeTab ? " active" : "");
    tab.onclick = () => assignToFocusedPane(id);

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
  host.className = `layout-${state.panes.length}`;

  // Detach term elements first so re-appending doesn't destroy them
  for (const entry of state.open.values()) entry.el.remove();
  host.querySelectorAll(".pane").forEach((p) => p.remove());

  const anyOpen = state.open.size > 0;
  $("empty-state").style.display = anyOpen ? "none" : "flex";

  state.panes.forEach((sid, i) => {
    const pane = document.createElement("div");
    pane.className = "pane" + (i === state.focusedPane ? " focused" : "");
    pane.onclick = () => {
      state.focusedPane = i;
      if (state.panes[i]) {
        state.activeTab = state.panes[i];
        focusTerm(state.panes[i]);
      }
      renderPanes();
      renderTabs();
      renderSidebar();
    };

    const entry = sid ? state.open.get(sid) : null;
    if (entry) {
      if (state.panes.length > 1) {
        const title = document.createElement("div");
        title.className = "pane-title";
        title.textContent = entry.name;
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
    for (const sid of state.panes) {
      const entry = sid && state.open.get(sid);
      if (entry) entry.fitAddon.fit();
    }
  });
}

function focusTerm(sid) {
  const entry = state.open.get(sid);
  if (entry) setTimeout(() => entry.term.focus(), 0);
}

function addPane() {
  if (state.panes.length >= 4) return;
  state.panes.push(null);
  state.focusedPane = state.panes.length - 1;
  renderPanes();
}

function removePane() {
  if (state.panes.length <= 1) return;
  state.panes.pop();
  state.focusedPane = Math.min(state.focusedPane, state.panes.length - 1);
  renderPanes();
}

function assignToFocusedPane(sid) {
  // If already visible in a pane, just focus that pane
  const existing = state.panes.indexOf(sid);
  if (existing >= 0) {
    state.focusedPane = existing;
  } else {
    state.panes[state.focusedPane] = sid;
  }
  state.activeTab = sid;
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

  focusTerm(sid);
}

/// Detach (keep session running in the backend).
async function detachSession(sid) {
  try {
    await invoke("close_session", { sessionId: sid });
  } catch (e) {
    console.error("close_session failed:", e);
  }
  dropTerminal(sid);
}

/// Remove the local terminal for a session (after detach/stash/kill/exit).
function dropTerminal(sid) {
  const entry = state.open.get(sid);
  if (!entry) return;
  entry.term.dispose();
  entry.el.remove();
  state.open.delete(sid);
  state.panes = state.panes.map((p) => (p === sid ? null : p));
  if (state.activeTab === sid) {
    const next = state.open.keys().next();
    state.activeTab = next.done ? null : next.value;
  }
  renderPanes();
  renderTabs();
  renderSidebar();
}

// ── Details panel ───────────────────────────────────────────────

function showDetails(sid) {
  state.detailsFor = sid;
  $("details").classList.remove("hidden");
  renderDetails();
  fitAll();
}

function hideDetails() {
  state.detailsFor = null;
  $("details").classList.add("hidden");
  fitAll();
}

function kv(k, v) {
  return `<div class="kv"><span class="k">${k}</span><span class="v">${escapeHtml(
    v || "—"
  )}</span></div>`;
}

function escapeHtml(s) {
  return String(s)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function renderDetails() {
  const body = $("details-body");
  const s = state.sessions.find((x) => x.id === state.detailsFor);
  if (!s) {
    body.innerHTML = "<p>Session not found.</p>";
    return;
  }
  body.innerHTML = `
    <h3>${escapeHtml(displayName(s))}</h3>
    ${kv("Status", s.is_running ? s.status + " (running)" : s.status)}
    ${kv("Branch", s.git_branch)}
    ${kv("Project", s.worktree_project || s.project)}
    ${kv("Worktree", s.worktree)}
    ${kv("CWD", s.cwd || s.project_path)}
    ${kv("Agents", s.subagent_count > 0 ? String(s.subagent_count) : "—")}
    ${kv("Modified", s.last_modified)}
    ${kv("ID", s.id)}
    <h4>SUMMARY</h4>
    <div class="kv"><span class="v">${escapeHtml(s.summary || s.first_prompt || "—")}</span></div>
    <div class="actions">
      <button id="d-diff">Show diff</button>
      <button id="d-close">Close panel</button>
    </div>
    <div id="d-diff-out"></div>
  `;
  $("d-close").onclick = hideDetails;
  $("d-diff").onclick = async () => {
    const out = $("d-diff-out");
    out.innerHTML = `<h4>GIT DIFF (HEAD)</h4><div class="diff">loading…</div>`;
    try {
      const diff = await invoke("get_diff", { sessionId: s.id });
      out.querySelector(".diff").innerHTML = renderDiff(diff);
    } catch (e) {
      out.querySelector(".diff").textContent = `diff failed: ${e}`;
    }
  };
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
  state.detailsFor = null;
  const body = $("details-body");
  let tasks = [];
  try {
    tasks = await invoke("list_tasks", { team: team.name });
  } catch (e) {
    console.error("list_tasks failed:", e);
  }
  const members = (team.members || [])
    .map((m) => kv(m.name, m.agent_type + (m.model ? ` · ${m.model}` : "")))
    .join("");
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
    <h4>MEMBERS</h4>
    ${members || "<p class='hint'>none</p>"}
    <h4>TASKS</h4>
    ${taskRows}
    <div class="actions"><button id="d-close">Close panel</button></div>
  `;
  $("d-close").onclick = hideDetails;
  fitAll();
}

// ── New session modal ───────────────────────────────────────────

function showNewSessionModal() {
  $("ns-error").classList.add("hidden");
  $("modal-backdrop").classList.remove("hidden");
  setTimeout(() => $("ns-name").focus(), 0);
}

function hideNewSessionModal() {
  $("modal-backdrop").classList.add("hidden");
}

async function createSession() {
  const name = $("ns-name").value;
  const cwd = $("ns-cwd").value;
  try {
    const sid = await invoke("create_new_session", {
      name,
      cwd,
      cols: 120,
      rows: 40,
    });
    hideNewSessionModal();
    $("ns-name").value = "";
    await refreshSessions();
    openSession(sid);
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
  if (e.metaKey && e.key === "w") {
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

// ── Boot ────────────────────────────────────────────────────────

refreshSessions();
renderPanes();
setInterval(refreshSessions, 2000);
