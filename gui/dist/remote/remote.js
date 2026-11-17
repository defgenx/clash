// The phone remote page (docs/remote.md). Decisions live in remote-model.js;
// the inbox rows are attention.js's, the same ones the desktop inbox shows.
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
  let inbox = null; // { url, rows, at }

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

  async function pollInbox() {
    const m = machines.find((x) => x.url === selected);
    if (!m) return;
    const r = await call(m, "/api/state");
    if (!r.ok) return;
    polls.set(m.url, recordPoll(polls.get(m.url), { ok: true, status: r.body.status }, Date.now()));
    inbox = {
      url: m.url,
      rows: attentionRows({ sessions: r.body.sessions || [], workflows: r.body.workflows || [] }),
      at: Date.now(),
    };
  }

  async function tick() {
    await Promise.all(machines.filter((m) => m.url !== selected).map(pollOne));
    await pollInbox();
    const m = machines.find((x) => x.url === selected);
    // The inbox call doubles as the selected machine's status poll; when it
    // failed, record the failure through a status poll of its own.
    if (m && (!inbox || inbox.url !== m.url || Date.now() - inbox.at > REMOTE_POLL_MS / 2)) await pollOne(m);
    render();
  }

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }

  function render() {
    const now = Date.now();
    const views = machines.map((m) => instanceView(m, polls.get(m.url), now));
    const current = views.find((v) => v.url === selected) || null;

    const ind = connectionIndicator(current);
    const conn = $("conn");
    conn.className = `conn ${ind.state}`;
    conn.textContent = ind.text;

    const list = $("machines");
    list.replaceChildren();
    if (!views.length) list.append(el("p", "empty", "No machine yet. In clash on your Mac: Settings → Remote, then open or paste its pairing link here."));
    for (const v of views) {
      const card = el("div", `card machine${v.url === selected ? " selected" : ""}`);
      card.append(el("span", `dot ${v.state}`));
      card.append(el("span", "m-name", v.name));
      const badge = el("span", `badge${v.needsYou ? "" : " zero"}`, v.needsYou == null ? "–" : `${v.needsYou} need you`);
      card.append(badge);
      card.append(el("span", "m-detail", v.detail));
      if (v.facts.length) card.append(el("span", "m-facts", v.facts.join(" · ")));
      const forget = el("button", "link", "Forget");
      forget.type = "button";
      forget.onclick = (e) => {
        e.stopPropagation();
        machines = removeInstance(machines, v.url);
        saveInstances(store, machines);
        polls.delete(v.url);
        if (selected === v.url) selected = machines.length ? machines[0].url : null;
        inbox = null;
        render();
      };
      card.append(forget);
      card.onclick = () => {
        selected = v.url;
        inbox = null;
        render();
        tick();
      };
      list.append(card);
    }

    $("inbox-title").textContent = current ? `Waiting on you · ${current.name}` : "Waiting on you";
    const box = $("inbox");
    box.replaceChildren();
    if (!current) return;
    if (!inbox || inbox.url !== current.url) {
      box.append(el("p", "empty", current.state === "connected" ? "Loading…" : "Not connected."));
      return;
    }
    if (!inbox.rows.length) box.append(el("p", "empty", "Nothing is waiting on you."));
    for (const r of inbox.rows) {
      const tappable = r.kind === "session" && r.sessionId;
      const card = el("div", `card row band-${r.band}${tappable ? " tappable" : ""}`);
      card.append(el("span", "title", r.title));
      if (r.context) card.append(el("span", "ctx", r.context));
      card.append(el("span", "why", r.reasons.join(" · ")));
      if (tappable) card.onclick = () => openSession(current.url, r.sessionId, r.title);
      box.append(card);
    }
    if (current.state !== "connected") box.append(el("p", "note", `As of ${remoteAgo(now - inbox.at)}.`));
  }

  // ── A session: its screen, its prompt, keys, a follow-up, stash ─────────

  let session = null; // { url, id, title }
  let screen = null; // { lines, prompt, at } | null
  let sessionError = "";
  let armed = null; // { id, at } — the button waiting for its second tap
  let screenTimer = null;

  function sessionMachine() {
    return session && machines.find((m) => m.url === session.url);
  }

  function sessionPath(what) {
    return `/api/sessions/${encodeURIComponent(session.id)}/${what}`;
  }

  function openSession(url, id, title) {
    session = { url, id, title };
    screen = null;
    sessionError = "";
    armed = null;
    $("followup-text").value = "";
    $("s-note").textContent = "";
    document.body.classList.add("in-session");
    $("session").classList.remove("hidden");
    window.scrollTo(0, 0);
    renderSession();
    pollScreen();
    clearInterval(screenTimer);
    screenTimer = setInterval(pollScreen, SCREEN_POLL_MS);
  }

  function closeSession() {
    session = null;
    clearInterval(screenTimer);
    document.body.classList.remove("in-session");
    $("session").classList.add("hidden");
    render();
  }

  async function pollScreen() {
    const m = sessionMachine();
    if (!m || document.visibilityState !== "visible") return;
    const r = await call(m, sessionPath("screen"));
    if (!session) return;
    if (r.ok) {
      screen = { lines: r.body.lines || [], prompt: r.body.prompt || null, at: Date.now() };
      sessionError = "";
    } else {
      sessionError = actionError(r.status);
    }
    renderSession();
  }

  /// A tap on a key or choice: armed buttons send on the second tap.
  async function press(button, id) {
    const step = tapButton(button, armed, id, Date.now());
    armed = step.armed;
    if (step.do === "arm") {
      renderSession();
      setTimeout(renderSession, ARM_MS + 50);
      return;
    }
    const m = sessionMachine();
    if (!m) return;
    const r = await call(m, sessionPath("keys"), { key: button.key });
    sessionError = r.ok ? "" : actionError(r.status);
    renderSession();
    setTimeout(pollScreen, 250);
  }

  function actionButton(button, id, extra = "") {
    const live = armed && armed.id === id && Date.now() - armed.at < ARM_MS;
    const b = el("button", `${extra}${live ? " armed" : ""}`.trim(), live ? `Tap again: ${button.label}` : button.label);
    b.type = "button";
    b.onclick = () => press(button, id);
    return b;
  }

  function renderSession() {
    if (!session) return;
    $("s-title").textContent = session.title;
    const err = $("s-error");
    err.textContent = sessionError;
    err.classList.toggle("hidden", !sessionError);

    const choices = choiceButtons(screen && screen.prompt);
    $("s-prompt").classList.toggle("hidden", !choices.length);
    $("s-question").textContent = (screen && screen.prompt && screen.prompt.question) || "Choose an option";
    const list = $("s-choices");
    list.replaceChildren(...choices.map((c) => actionButton(c, `choice-${c.key}`)));

    const pre = $("s-screen");
    const atBottom = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 4;
    pre.textContent = screen ? screenTail(screen.lines).join("\n") : "Loading…";
    if (atBottom) pre.scrollTop = pre.scrollHeight;

    $("s-keys").replaceChildren(...KEY_BAR.map((k) => actionButton(k, `key-${k.key}`)));

    const stash = $("s-stash");
    const live = armed && armed.id === "stash" && Date.now() - armed.at < ARM_MS;
    stash.classList.toggle("armed", !!live);
    stash.textContent = live ? "Tap again to stash" : "Stash session";
  }

  $("s-back").onclick = closeSession;

  $("s-stash").onclick = async () => {
    const step = tapButton({ confirm: true }, armed, "stash", Date.now());
    armed = step.armed;
    if (step.do === "arm") {
      renderSession();
      setTimeout(renderSession, ARM_MS + 50);
      return;
    }
    const m = sessionMachine();
    if (!m) return;
    const r = await call(m, sessionPath("stash"), {});
    if (r.ok) {
      closeSession();
      tick();
    } else {
      sessionError = actionError(r.status);
      renderSession();
    }
  };

  $("followup-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const text = $("followup-text").value.trim();
    const m = sessionMachine();
    if (!text || !m) return;
    const r = await call(m, sessionPath("followup"), { text });
    if (r.ok) {
      $("followup-text").value = "";
      $("s-note").textContent = `Queued (${r.body.queued} waiting): sent when the session is next idle at its prompt.`;
      sessionError = "";
    } else {
      sessionError = actionError(r.status);
    }
    renderSession();
  });

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
    inbox = null;
    render();
    tick();
  });

  render();
  tick();
  setInterval(tick, REMOTE_POLL_MS);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState !== "visible") return;
    tick();
    if (session) pollScreen();
  });
})();
