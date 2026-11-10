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

  async function call(machine, path) {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), 4000);
    try {
      const res = await fetch(machine.url + path, {
        headers: { Authorization: `Bearer ${machine.token}` },
        cache: "no-store",
        signal: ctl.signal,
      });
      if (res.status === 401) return { unauthorized: true };
      if (!res.ok) return { error: `HTTP ${res.status}` };
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
      const card = el("div", `card row band-${r.band}`);
      card.append(el("span", "title", r.title));
      if (r.context) card.append(el("span", "ctx", r.context));
      card.append(el("span", "why", r.reasons.join(" · ")));
      box.append(card);
    }
    if (current.state !== "connected") box.append(el("p", "note", `As of ${remoteAgo(now - inbox.at)}.`));
  }

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
    if (document.visibilityState === "visible") tick();
  });
})();
