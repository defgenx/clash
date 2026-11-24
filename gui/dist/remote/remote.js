// The phone remote page (docs/remote.md): the desktop's sidebar, inbox,
// sessions and workflow decisions, laid out for a phone and painted with the
// Mac's own look. Decisions live in remote-model.js, attention.js (the inbox),
// status-info.js and wf-decisions.js — the same modules the desktop runs.
(function () {
  "use strict";

  const $ = (id) => document.getElementById(id);
  const store = (() => {
    try {
      return window.localStorage;
    } catch (_) {
      return null;
    }
  })();

  let machines = loadInstances(store);
  const polls = new Map(); // url → poll record
  let selected = null; // url
  let data = null; // { url, sessions, workflows, at } of the selected machine
  let lookKey = ""; // the last look applied, as JSON
  let termLook = null; // the desktop terminal's palette and font
  let view = { kind: "inbox" }; // | { kind: "session", id } | { kind: "item", key }
  const inboxFilter = { filter: "all", query: "" };
  let sessionQuery = "";

  // A pairing link opened on the phone: remember this machine, then drop the
  // token from the address bar so it is not left in history or a screenshot.
  const token = pairingToken(location.hash);
  if (token) {
    machines = addInstance(machines, { url: location.origin, token });
    saveInstances(store, machines);
    selected = location.origin;
    try {
      history.replaceState(null, "", location.pathname);
    } catch (_) {}
  }
  if (!selected && machines.length) selected = machines[0].url;

  const current = () => machines.find((x) => x.url === selected);

  async function call(machine, path, body) {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), 4000);
    try {
      const headers = { Authorization: `Bearer ${machine.token}` };
      if (body !== undefined) headers["Content-Type"] = "application/json";
      const res = await fetch(machine.url + path, {
        method: body === undefined ? "GET" : "POST",
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        cache: "no-store",
        signal: ctl.signal,
      });
      if (res.status === 401) return { unauthorized: true, status: 401 };
      if (!res.ok) return { error: `HTTP ${res.status}`, status: res.status };
      if (res.status === 204) return { ok: true, body: null };
      return { ok: true, body: await res.json() };
    } catch (e) {
      return { error: e && e.name === "AbortError" ? "timed out" : "no answer" };
    } finally {
      clearTimeout(timer);
    }
  }

  async function pollOne(machine) {
    const r = await call(machine, "/api/status");
    const result = r.ok ? { ok: true, status: r.body } : r;
    polls.set(machine.url, recordPoll(polls.get(machine.url), result, Date.now()));
  }

  async function pollSelected() {
    const m = current();
    if (!m) return;
    const [r, look] = await Promise.all([call(m, "/api/state"), call(m, "/api/appearance")]);
    if (look.ok) applyLook(look.body);
    if (!r.ok) {
      polls.set(m.url, recordPoll(polls.get(m.url), r, Date.now()));
      return;
    }
    polls.set(m.url, recordPoll(polls.get(m.url), { ok: true, status: r.body.status }, Date.now()));
    data = { url: m.url, sessions: r.body.sessions || [], workflows: r.body.workflows || [], at: Date.now() };
  }

  async function tick() {
    await Promise.all(machines.filter((m) => m.url !== selected).map(pollOne));
    await pollSelected();
    render();
  }

  // ── The Mac's look ──────────────────────────────────────────────────

  /// Paint the page with the selected machine's theme, exactly as its window
  /// shows it: the same custom properties on the same stylesheet.
  function applyLook(look) {
    const key = JSON.stringify(look || {});
    if (key === lookKey) return;
    lookKey = key;
    const root = document.documentElement;
    for (const [k, v] of lookVars(look)) root.style.setProperty(k, v);
    root.classList.toggle("theme-light", !!(look && look.light));
    const bar = getComputedStyle(root).getPropertyValue("--bg-sidebar").trim();
    if (bar) $("theme-color").setAttribute("content", bar);
    termLook = (look && look.terminal) || null;
    if (term) {
      dropTerminal();
      if (view.kind === "session") renderView(true);
    }
  }

  // ── Building blocks: the desktop's own markup ───────────────────────

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  function button(label, cls, onclick) {
    const b = el("button", cls || null, label);
    b.type = "button";
    if (onclick) b.onclick = onclick;
    return b;
  }

  /// A sidebar session row, as the desktop's sidebar builds it.
  function sessionRow(s) {
    const item = el("div", `session-item${view.kind === "session" && view.id === s.id ? " selected" : ""}`);
    item.append(el("div", `status-ring ${statusClass(s)}`));
    const meta = el("div", "session-meta");
    meta.append(el("div", "session-name", displayName(s)));
    const sub = el("div", "session-sub");
    sub.append(el("span", "agent-badge", (AGENT_BADGES[s.agent] || AGENT_BADGES.claude).short));
    const st = statusInfo(s);
    sub.append(el("span", `status-label ${st.cls}`, `${st.icon} ${st.label}`));
    if (s.git_branch) sub.append(el("span", "branch", s.git_branch));
    meta.append(sub);
    item.append(meta);
    item.onclick = () => go({ kind: "session", id: s.id });
    return item;
  }

  /// A sidebar workflow row, as the desktop's WORKFLOWS list builds it.
  function itemRow(item) {
    const key = `${item.project}/${item.slug}`;
    const info = wfStatusInfo(item.meta && item.meta.status);
    const row = el("div", `session-item wf-item${view.kind === "item" && view.key === key ? " selected" : ""}`);
    row.append(el("div", `status-ring wf-ring ${info.cls}`));
    const meta = el("div", "session-meta");
    meta.append(el("span", "session-name", (item.meta && item.meta.title) || item.slug));
    const sub = el("span", "session-sub");
    sub.append(el("span", `status-label ${info.cls}`, info.label));
    sub.append(el("span", "proj", item.project));
    const open = openDecisions(item).length;
    if (open) sub.append(el("span", "wf-chip wf-changes", `❓ ${open}`));
    meta.append(sub);
    row.append(meta);
    row.onclick = () => go({ kind: "item", key });
    return row;
  }

  function go(next) {
    view = next;
    document.body.classList.remove("drawer-open");
    renderView(true);
    renderSidebar();
    if (next.kind === "session") startScreen();
    else stopScreen();
  }

  // ── Sidebar ─────────────────────────────────────────────────────────

  function render() {
    renderSidebar();
    renderTopbar();
    renderView(false);
  }

  function rows() {
    return data && data.url === selected ? attentionRows({ sessions: data.sessions, workflows: data.workflows }) : [];
  }

  function renderSidebar() {
    const now = Date.now();
    const box = $("r-machines");
    box.replaceChildren();
    if (!machines.length)
      box.append(el("p", "hint", "No machine yet. In clash on your Mac: Settings → Remote, then scan its QR code or paste its pairing link."));
    for (const m of machines) {
      const v = instanceView(m, polls.get(m.url), now);
      const card = el("div", `mx-card mx-${v.state}${v.url === selected ? " r-selected" : ""}`);
      const chip = { connected: "wf-done", reconnecting: "wf-active", checking: "wf-active" }[v.state] || "mx-lost";
      const top = el("div", "mx-top");
      top.append(el("span", "mx-name", v.name), el("span", `wf-chip ${chip}`, v.state));
      if (v.needsYou != null) top.append(el("span", `mx-needs${v.needsYou ? " hot" : ""}`, `${v.needsYou} need you`));
      const forget = button("Forget", "mx-forget", (e) => {
        e.stopPropagation();
        machines = removeInstance(machines, v.url);
        saveInstances(store, machines);
        polls.delete(v.url);
        if (selected === v.url) selected = machines.length ? machines[0].url : null;
        data = null;
        go({ kind: "inbox" });
        tick();
      });
      top.append(forget);
      card.append(top, el("div", "mx-detail", v.detail));
      if (v.facts.length) card.append(el("div", "mx-facts", v.facts.join(" · ")));
      card.onclick = () => {
        if (selected === v.url) return;
        selected = v.url;
        data = null;
        lookKey = "";
        go({ kind: "inbox" });
        tick();
      };
      box.append(card);
    }

    const all = rows();
    $("r-inbox-count").textContent = String(all.length);
    $("r-inbox-nav").classList.toggle("selected", view.kind === "inbox");

    const list = $("session-list");
    list.replaceChildren();
    const wf = $("wf-list");
    wf.replaceChildren();
    if (!data || data.url !== selected) return;
    const q = sessionQuery.trim().toLowerCase();
    const sessions = data.sessions.filter(
      (s) => !q || `${displayName(s)} ${s.git_branch || ""} ${s.project_path || ""}`.toLowerCase().includes(q)
    );
    for (const g of sessionSections(sessions, sectionOf)) {
      const head = el("div", "section-label");
      head.textContent = g.label;
      head.append(el("span", "count", String(g.rows.length)));
      list.append(head);
      for (const s of g.rows) list.append(sessionRow(s));
    }
    if (!sessions.length) list.append(el("div", "list-empty", q ? "No session matches." : "No sessions."));

    const items = data.workflows.filter((i) => !["done", "abandoned"].includes(i.meta && i.meta.status));
    const head = el("div", "section-label", "WORKFLOWS");
    head.append(el("span", "count", String(items.length)));
    wf.append(head);
    for (const i of items) wf.append(itemRow(i));
  }

  function renderTopbar() {
    const v = current() ? instanceView(current(), polls.get(selected), Date.now()) : null;
    const ind = connectionIndicator(v);
    const conn = $("conn");
    conn.replaceChildren();
    const ring = { connected: "running", reconnecting: "waiting", lost: "errored", none: "done" }[ind.state] || "done";
    conn.append(el("span", `status-ring ${ring}`), el("span", null, ind.text));

    let title = "Inbox";
    let dot = "";
    if (view.kind === "session") {
      const s = data && data.sessions.find((x) => x.id === view.id);
      title = s ? displayName(s) : "Session";
      dot = s ? statusClass(s) : "";
    } else if (view.kind === "item") {
      const it = findItem(view.key);
      title = it ? (it.meta && it.meta.title) || it.slug : "Workflow item";
    }
    $("r-tab-title").textContent = title;
    $("r-tab-dot").className = `tab-dot ${dot}`;
    $("r-tab-dot").classList.toggle("hidden", view.kind !== "session");
  }

  // ── The main area ───────────────────────────────────────────────────

  let shellFor = ""; // which view the main area's DOM was built for

  function renderView(rebuild) {
    renderTopbar();
    const id = view.kind === "session" ? `s:${view.id}` : view.kind === "item" ? `i:${view.key}` : "inbox";
    const host = $("r-view");
    if (rebuild || shellFor !== id) {
      shellFor = id;
      host.replaceChildren();
      host.scrollTop = 0;
      if (view.kind === "session") buildSession(host);
    }
    if (!current()) {
      host.replaceChildren(el("div", "r-empty", "Pair a machine to start: in clash on your Mac open Settings → Remote, then scan its QR code."));
      shellFor = "";
      return;
    }
    if (view.kind === "inbox") renderInbox(host);
    else if (view.kind === "item") renderItem(host);
    else renderSession();
  }

  // The inbox: the desktop's inbox tab (attention.js decides every row).
  function renderInbox(host) {
    if (!host._inbox) {
      host.replaceChildren();
      const root = el("div", "wf-board-view");
      const head = el("div", "wf-board-head inbox-head");
      const top = el("div", "wf-board-top");
      const summary = el("div", "wf-board-summary");
      const search = el("input", "wf-board-search");
      search.type = "search";
      search.placeholder = "Filter by title, project or reason…";
      search.value = inboxFilter.query;
      search.oninput = () => {
        inboxFilter.query = search.value;
        renderInbox(host);
      };
      top.append(el("h2", null, "Inbox"), summary, search);
      const filters = el("div", "wf-waves-modes inbox-filters");
      head.append(top, filters);
      const body = el("div", "inbox");
      root.append(head, body);
      host.append(root);
      host._inbox = { summary, filters, body };
    }
    const shell = host._inbox;
    if (!data || data.url !== selected) {
      shell.body.replaceChildren(el("p", "hint", "Loading…"));
      return;
    }
    const v = inboxView(rows(), { ...inboxFilter, snoozes: {}, now: Date.now() });
    shell.summary.textContent = v.total ? `${v.total} waiting on you${v.blocking ? ` · ${v.blocking} blocked` : ""}` : "all clear";
    shell.filters.replaceChildren();
    for (const fid of INBOX_FILTERS.filter((f) => f !== "snoozed")) {
      const label = fid === "all" ? "All" : (INBOX_GROUPS.find((g) => g.id === fid) || {}).label || fid;
      const b = button("", `wf-seg${v.filter === fid ? " on" : ""}`, () => {
        inboxFilter.filter = fid;
        renderInbox(host);
      });
      b.append(document.createTextNode(`${label} `), el("span", "wf-seg-n", String(v.counts[fid] || 0)));
      shell.filters.append(b);
    }
    shell.body.replaceChildren();
    if (!v.groups.length) {
      shell.body.append(el("p", "hint", v.total ? "Nothing in this section." : "Nothing is waiting on you — every session has been answered and no workflow item is parked on a decision."));
      return;
    }
    for (const g of v.groups) {
      const label = el("div", "inbox-group", `${g.label} `);
      label.append(el("span", "wf-seg-n", String(g.rows.length)));
      shell.body.append(label);
      for (const r of g.rows) {
        const row = el("div", `inbox-row band-${r.band}`);
        const s = r.kind === "session" && data.sessions.find((x) => x.id === r.sessionId);
        const info = r.kind === "session" ? statusInfo(s || { status: r.status }) : wfStatusInfo(r.status);
        row.append(el("span", `${r.kind === "session" ? "status-ring" : "status-ring wf-ring"} ${info.cls}`));
        const main = el("span", "inbox-main");
        main.append(el("span", "inbox-title", r.title));
        const sub = el("span", "inbox-sub");
        sub.append(el("span", `status-label ${info.cls}`, info.label), el("span", "dim", r.context || ""));
        main.append(sub, el("span", "inbox-why", r.reasons.join(" · ")));
        row.append(main, el("span", "inbox-age dim", wfAgo(r.since)));
        row.onclick = () =>
          r.kind === "session" ? go({ kind: "session", id: r.sessionId }) : go({ kind: "item", key: `${r.project}/${r.slug}` });
        shell.body.append(row);
      }
    }
  }

  // ── A session: its terminal, its prompt, keys, a follow-up, stash ──────

  let screen = null; // { lines, prompt, view, at }
  let sessionError = "";
  let armed = null; // { id, at } — the button waiting for its second tap
  let screenTimer = null;
  let term = null; // { t, cols, rows, ansi, box, scale }
  let actualSize = false;
  let note = "";

  const sessionPath = (what) => `/api/sessions/${encodeURIComponent(view.id)}/${what}`;

  function startScreen() {
    screen = null;
    sessionError = "";
    armed = null;
    note = "";
    pollScreen();
    clearInterval(screenTimer);
    screenTimer = setInterval(pollScreen, SCREEN_POLL_MS);
  }

  function stopScreen() {
    clearInterval(screenTimer);
    screenTimer = null;
    dropTerminal();
  }

  function dropTerminal() {
    if (term) term.t.dispose();
    term = null;
  }

  async function pollScreen() {
    const m = current();
    if (!m || view.kind !== "session" || document.visibilityState !== "visible") return;
    const id = view.id;
    const r = await call(m, sessionPath("screen"));
    if (view.kind !== "session" || view.id !== id) return;
    if (r.ok) {
      screen = { lines: r.body.lines || [], prompt: r.body.prompt || null, view: r.body.view || null, at: Date.now() };
      sessionError = "";
    } else {
      sessionError = actionError(r.status);
    }
    renderSession();
  }

  let parts = null; // the session view's DOM

  function buildSession(host) {
    dropTerminal();
    const err = el("p", "r-error hidden");
    err.setAttribute("role", "alert");
    const prompt = el("section", "wf-decision open hidden");
    prompt.style.margin = "10px";
    const question = el("div", "wf-decision-head");
    const choices = el("div", "wf-decision-options");
    prompt.append(question, choices);

    const box = el("div", "r-term-box");
    const scale = el("div", "r-term-scale");
    box.append(scale);
    const tools = el("div", "r-term-tools");
    const age = el("span", "dim");
    const zoom = button("", "icon-btn wide", () => {
      actualSize = !actualSize;
      fitTerminal();
      renderSession();
    });
    tools.append(age, el("span", "spacer"), zoom);

    const keys = el("div", "r-keys");

    const follow = el("section", "r-section");
    follow.append(el("div", "section-label", "FOLLOW-UP"));
    const free = el("div", "wf-decision-free");
    const ta = el("textarea");
    ta.rows = 3;
    ta.placeholder = "Sent when the session is next idle at its prompt";
    free.append(ta);
    const send = button("Queue follow-up", "primary r-wide", () => sendFollowup(ta));
    follow.append(free, send);
    const noteEl = el("p", "hint r-note");

    const stash = button("Stash session", "danger r-wide", () => stashNow());
    const stashWrap = el("section", "r-section");
    stashWrap.append(stash, el("p", "hint r-note", "Anything that answers, interrupts or stashes takes a second tap."));

    host.append(err, prompt, box, tools, keys, follow, noteEl, stashWrap);
    parts = { err, prompt, question, choices, box, scale, age, zoom, keys, noteEl, stash };
  }

  /// The terminal, drawn by the desktop's xterm with the desktop's palette and
  /// font, then shrunk to the phone's width (or shown at the desktop's size).
  function paintTerminal(v) {
    if (!v || !v.ansi || !window.Terminal) return;
    const cols = v.cols || 80;
    const rowsN = v.rows || 24;
    if (!term) {
      const t = new Terminal({
        cols,
        rows: rowsN,
        disableStdin: true,
        cursorBlink: false,
        scrollback: 0,
        // The Mac's font first; a phone may not have it under that name.
        fontFamily: `${(termLook && termLook.fontFamily) || "SF Mono"}, Menlo, "SF Mono", ui-monospace, monospace`,
        fontSize: (termLook && termLook.fontSize) || 13,
        fontWeight: (termLook && termLook.fontWeight) || "normal",
        lineHeight: (termLook && termLook.lineHeight) || 1,
        letterSpacing: (termLook && termLook.letterSpacing) || 0,
        theme: (termLook && termLook.theme) || undefined,
      });
      parts.scale.replaceChildren();
      t.open(parts.scale);
      parts.box.style.background = (termLook && termLook.theme && termLook.theme.background) || "";
      term = { t, cols, rows: rowsN, ansi: "" };
    }
    if (term.cols !== cols || term.rows !== rowsN) {
      term.t.resize(cols, rowsN);
      term.cols = cols;
      term.rows = rowsN;
      term.ansi = "";
    }
    if (term.ansi === v.ansi) return;
    term.ansi = v.ansi;
    const bin = atob(v.ansi);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const [cr, cc] = v.cursor || [0, 0];
    term.t.write("\x1b[0m\x1b[H\x1b[2J");
    term.t.write(bytes);
    term.t.write(`\x1b[${cr + 1};${cc + 1}H\x1b[?25${v.cursor_hidden ? "l" : "h"}`, fitTerminal);
  }

  function fitTerminal() {
    if (!term || !parts) return;
    const screenEl = parts.scale.querySelector(".xterm-screen");
    const w = screenEl ? screenEl.offsetWidth + 12 : 0;
    const k = actualSize ? 1 : fitScale(w, parts.box.clientWidth);
    parts.scale.style.transform = k < 1 ? `scale(${k})` : "";
    parts.box.classList.toggle("actual", actualSize);
    parts.box.style.height = `${Math.ceil(parts.scale.offsetHeight * k)}px`;
  }

  function renderSession() {
    if (view.kind !== "session" || !parts) return;
    renderTopbar();
    parts.err.textContent = sessionError;
    parts.err.classList.toggle("hidden", !sessionError);

    const choices = choiceButtons(screen && screen.prompt);
    parts.prompt.classList.toggle("hidden", !choices.length);
    parts.question.replaceChildren(el("span", "wf-decision-text", (screen && screen.prompt && screen.prompt.question) || "Choose an option"));
    parts.choices.replaceChildren(
      ...choices.map((c) => {
        const id = `choice-${c.key}`;
        const live = armed && armed.id === id && Date.now() - armed.at < ARM_MS;
        const b = button("", `wf-decision-option${live ? " armed" : ""}`, () => press(c, id));
        b.append(el("span", "wf-decision-key", c.key), el("span", "wf-decision-option-text", live ? `Tap again: ${c.label.replace(/^\d · /, "")}` : c.label.replace(/^\d · /, "")));
        return b;
      })
    );

    if (screen && screen.view && screen.view.ansi) paintTerminal(screen.view);
    else if (!term) parts.scale.replaceChildren(el("p", "hint", screen ? screenTail(screen.lines).join("\n") || "Empty screen." : "Loading…"));
    parts.age.textContent = screen ? `Screen ${wfAgo(screen.at)}` : "";
    parts.zoom.textContent = actualSize ? "Fit width" : "Actual size";

    parts.keys.replaceChildren(
      ...KEY_BAR.map((k) => {
        const id = `key-${k.key}`;
        const live = armed && armed.id === id && Date.now() - armed.at < ARM_MS;
        return button(live ? `Tap again: ${k.label}` : k.label, live ? "armed" : null, () => press(k, id));
      })
    );

    const live = armed && armed.id === "stash" && Date.now() - armed.at < ARM_MS;
    parts.stash.classList.toggle("armed", !!live);
    parts.stash.textContent = live ? "Tap again to stash" : "Stash session";
    parts.noteEl.textContent = note;
  }

  /// A tap on a key or choice: armed buttons send on the second tap.
  async function press(b, id) {
    const step = tapButton(b, armed, id, Date.now());
    armed = step.armed;
    if (step.do === "arm") {
      renderSession();
      setTimeout(renderSession, ARM_MS + 50);
      return;
    }
    const m = current();
    if (!m) return;
    const r = await call(m, sessionPath("keys"), { key: b.key });
    sessionError = r.ok ? "" : actionError(r.status);
    renderSession();
    setTimeout(pollScreen, 250);
  }

  async function sendFollowup(ta) {
    const text = ta.value.trim();
    const m = current();
    if (!text || !m) return;
    const r = await call(m, sessionPath("followup"), { text });
    if (r.ok) {
      ta.value = "";
      note = `Queued (${r.body.queued} waiting): sent when the session is next idle at its prompt.`;
      sessionError = "";
    } else {
      sessionError = actionError(r.status);
    }
    renderSession();
  }

  async function stashNow() {
    const step = tapButton({ confirm: true }, armed, "stash", Date.now());
    armed = step.armed;
    if (step.do === "arm") {
      renderSession();
      setTimeout(renderSession, ARM_MS + 50);
      return;
    }
    const m = current();
    if (!m) return;
    const r = await call(m, sessionPath("stash"), {});
    if (r.ok) {
      go({ kind: "inbox" });
      tick();
    } else {
      sessionError = actionError(r.status);
      renderSession();
    }
  }

  // ── A workflow item: where it stands, and its decisions ────────────────

  function findItem(key) {
    return data && data.workflows.find((i) => `${i.project}/${i.slug}` === key);
  }

  let itemError = "";
  let itemKey = ""; // what the item view last drew, to skip repaints while typing

  function renderItem(host) {
    const item = findItem(view.key);
    if (!item) {
      host.replaceChildren(el("div", "r-empty", data ? "That workflow item is gone." : "Loading…"));
      return;
    }
    const typing = document.activeElement && document.activeElement.tagName === "TEXTAREA" && host.contains(document.activeElement);
    const key = JSON.stringify([item.meta, item.decisions, itemError]);
    if (typing || key === itemKey) return;
    itemKey = key;
    host.replaceChildren();
    const info = wfStatusInfo(item.meta.status);
    const head = el("div", "r-item-head");
    head.append(el("h2", null, item.meta.title || item.slug));
    const sub = el("div", "r-item-sub");
    sub.append(el("span", `wf-chip ${info.cls}`, info.label), el("span", "proj", item.project));
    if (item.meta.pr && item.meta.pr.url) sub.append(el("span", "pr-chip", `⇄ PR #${String(item.meta.pr.url).split("/").pop()}`));
    head.append(sub);
    const why = workflowReasons(item).map((r) => r.text);
    if (why.length) head.append(el("p", "hint", why.join(" · ")));
    host.append(head);
    if (itemError) host.append(el("p", "r-error", itemError));

    const m = decisionsModel(item);
    const sum = decisionsSummary(m);
    const dh = el("div", "wf-dec-head");
    const line = el("div", "wf-dec-headline");
    line.append(el("span", "wf-dec-title", sum.headline));
    dh.append(line);
    host.append(dh);
    const list = el("div", "wf-decisions");
    host.append(list);
    if (!m.open.length && !m.settled.length) {
      list.append(el("p", "hint", "No decisions — the agents haven't asked anything here."));
      return;
    }
    const iteration = (item.meta && item.meta.iteration) || 0;
    for (const d of m.open) list.append(openCard(item, d, iteration));
    for (const d of m.settled.slice(0, 8)) {
      const card = el("section", `wf-decision settled${d.status === "dropped" ? " dropped" : ""}`);
      const h = el("div", "wf-decision-head");
      h.append(el("span", "wf-decision-id", d.id), el("span", "wf-decision-text", d.text));
      card.append(h, el("div", "wf-decision-meta", d.status === "dropped" ? "dropped" : `${d.answer} — ${d.state}`));
      list.append(card);
    }
  }

  function openCard(item, d, iteration) {
    const card = el("section", "wf-decision open");
    const h = el("div", "wf-decision-head");
    h.append(el("span", "wf-decision-id", d.id), el("span", "wf-decision-text", d.text));
    card.append(h);
    const age = decisionAge(d, iteration);
    if (age.text) card.append(el("div", `wf-decision-meta${age.stale ? " wf-decision-stale" : ""}`, age.text));
    const answer = (text, b) => decide(item, d.id, text, b);
    if (d.options.length) {
      const opts = el("div", "wf-decision-options");
      for (const o of d.options) {
        const b = button("", `wf-decision-option${o.recommended ? " recommended" : ""}`);
        b.append(el("span", "wf-decision-key", o.key), el("span", "wf-decision-option-text", o.text));
        if (o.recommended) b.append(el("span", "wf-chip wf-done", "recommended"));
        b.onclick = () => answer(o.label, b);
        opts.append(b);
      }
      card.append(opts);
    }
    if (d.recommendedText) {
      const rec = el("div", "wf-decision-rec");
      rec.append(el("span", "wf-chip wf-done", "recommended"), document.createTextNode(` ${d.recommendedText}`));
      card.append(rec);
    }
    const free = el("div", "wf-decision-free");
    const ta = el("textarea");
    ta.rows = 2;
    ta.placeholder = d.options.length ? "Or answer in your own words…" : "Your answer…";
    free.append(ta);
    const actions = el("div", "wf-inline-actions");
    const own = button("Answer", "primary", (e) => ta.value.trim() && answer(ta.value.trim(), e.currentTarget));
    const leave = button("Let the agent decide", "wf-dec-delegate", (e) => answer(AGENT_DECIDES, e.currentTarget));
    actions.append(own, leave);
    card.append(free, actions);
    return card;
  }

  /// Answer a decision — the second tap sends, like every answer here.
  async function decide(item, id, answer, b) {
    const armId = `dec-${id}-${answer}`;
    const step = tapButton({ confirm: true }, armed, armId, Date.now());
    armed = step.armed;
    if (step.do === "arm") {
      b.classList.add("armed");
      setTimeout(() => b.classList.remove("armed"), ARM_MS);
      return;
    }
    const m = current();
    if (!m) return;
    const path = `/api/workflows/${encodeURIComponent(item.project)}/${encodeURIComponent(item.slug)}/decisions/${encodeURIComponent(id)}`;
    const r = await call(m, path, { answer });
    if (r.ok) {
      item.decisions = r.body;
      itemError = "";
    } else {
      itemError = r.status === 404 ? "That workflow item is gone." : actionError(r.status);
    }
    itemKey = "";
    renderView(false);
  }

  // ── Wiring ──────────────────────────────────────────────────────────

  $("r-menu").onclick = () => document.body.classList.add("drawer-open");
  $("r-close").onclick = () => document.body.classList.remove("drawer-open");
  $("r-scrim").onclick = () => document.body.classList.remove("drawer-open");
  $("r-inbox-nav").onclick = () => go({ kind: "inbox" });
  $("search").addEventListener("input", (e) => {
    sessionQuery = e.target.value;
    renderSidebar();
  });
  window.addEventListener("resize", fitTerminal);

  $("pair-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const parsed = parsePairingLink($("pair-link").value);
    if (!parsed) {
      $("pair-note").textContent = "That is not a clash pairing link (it ends in #token=…).";
      return;
    }
    machines = addInstance(machines, parsed);
    saveInstances(store, machines);
    selected = parsed.url;
    $("pair-link").value = "";
    $("pair-note").textContent = "";
    data = null;
    lookKey = "";
    go({ kind: "inbox" });
    tick();
  });

  render();
  tick();
  setInterval(tick, REMOTE_POLL_MS);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState !== "visible") return;
    tick();
    if (view.kind === "session") pollScreen();
  });
  if (!machines.length) document.body.classList.add("drawer-open");
})();
