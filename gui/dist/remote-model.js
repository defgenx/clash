// clash instances — the pure half of the Machines view and the phone page
// (docs/remote.md).
//
// An instance is a clash GUI you can see: `{ kind, name, url, token }`.
// `kind: "local"` is the clash this page runs inside (read from its own state,
// no HTTP, no token); `kind: "remote"` is another clash reached over HTTP with
// its pairing token. Status, polling and inbox logic are the same for both —
// only the transport differs, and the transport is the caller's: the phone
// page fetches, the desktop GUI asks its backend. Drawn by `remote/remote.js`
// (phone) and the *Machines* section of app.js (desktop).
(function () {
  "use strict";

  /// How often an instance is polled.
  const REMOTE_POLL_MS = 5000;
  /// Failed polls in a row before an instance that was connected reads as
  /// unreachable rather than reconnecting.
  const REMOTE_LOST_AFTER = 3;
  const REMOTE_STORAGE_KEY = "clash.remote.instances";

  /// The token a pairing link carries in its fragment (`#token=…`).
  function pairingToken(hash) {
    const m = /(?:^#|&)token=([0-9a-fA-F]{64})(?:&|$)/.exec(String(hash || ""));
    return m ? m[1].toLowerCase() : null;
  }

  /// A pasted pairing link → `{ url, token }` (url = the origin), or null.
  function parsePairingLink(text) {
    try {
      const u = new URL(String(text || "").trim());
      if (u.protocol !== "http:" && u.protocol !== "https:") return null;
      const token = pairingToken(u.hash);
      return token ? { url: u.origin, token } : null;
    } catch (_) {
      return null;
    }
  }

  /// The local instance. It has no url and no token: nothing about it goes
  /// over the network.
  function localInstance(name) {
    return { kind: "local", name: name || "This machine", url: "", token: "" };
  }

  /// One key per instance: the local one is unique, remote ones are their origin.
  function instanceKey(inst) {
    return inst && inst.kind === "local" ? "local" : (inst && inst.url) || "";
  }

  /// Add or re-pair a remote instance (same origin = same instance; the new
  /// token wins).
  function addInstance(list, { url, token, name }) {
    const all = list || [];
    const old = all.find((i) => i.kind !== "local" && i.url === url);
    const rest = all.filter((i) => !(i.kind !== "local" && i.url === url));
    let host = url;
    try {
      host = new URL(url).hostname;
    } catch (_) {}
    return [...rest, { kind: "remote", url, token: token || "", name: name || (old && old.name) || host }];
  }

  function removeInstance(list, url) {
    return (list || []).filter((i) => i.kind === "local" || i.url !== url);
  }

  /// Local first, then remote instances by name — "this one" is always on top.
  function orderInstances(list) {
    return [...(list || [])].sort((a, b) => {
      if ((a.kind === "local") !== (b.kind === "local")) return a.kind === "local" ? -1 : 1;
      return String(a.name).localeCompare(String(b.name));
    });
  }

  /// Read the paired remote instances. Storage can be missing or throw
  /// (private mode); an unreadable list is an empty one.
  function loadInstances(storage) {
    try {
      const raw = storage && storage.getItem(REMOTE_STORAGE_KEY);
      const list = raw ? JSON.parse(raw) : [];
      return Array.isArray(list)
        ? list
            .filter((i) => i && typeof i.url === "string" && typeof i.token === "string")
            .map((i) => ({ kind: "remote", url: i.url, token: i.token, name: i.name || i.url }))
        : [];
    } catch (_) {
      return [];
    }
  }

  /// Persist the remote instances only — the local one is never stored.
  function saveInstances(storage, list) {
    try {
      const remote = (list || []).filter((i) => i.kind !== "local");
      if (storage) storage.setItem(REMOTE_STORAGE_KEY, JSON.stringify(remote));
      return true;
    } catch (_) {
      return false;
    }
  }

  /// The record kept per instance: what the last answer was and when.
  function emptyPoll() {
    return { status: null, okAt: 0, failures: 0, unauthorized: false, error: "" };
  }

  /// Fold one poll result into the record. `result` is `{ ok, status }`,
  /// `{ unauthorized: true }` or `{ error }`; `now` is this page's clock.
  function recordPoll(poll, result, now) {
    if (result && result.ok && result.status && result.status.app === "clash") {
      return { status: result.status, okAt: now, failures: 0, unauthorized: false, error: "" };
    }
    const p = { ...(poll || emptyPoll()) };
    p.failures += 1;
    p.unauthorized = !!(result && result.unauthorized);
    p.error = (result && result.error) || (p.unauthorized ? "pairing token refused" : "no answer");
    return p;
  }

  function remoteAgo(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    if (s < 60) return "just now";
    const min = Math.floor(s / 60);
    if (min < 60) return `${min} min ago`;
    const h = Math.floor(min / 60);
    if (h < 24) return `${h} h ago`;
    return `${Math.floor(h / 24)} d ago`;
  }

  function remoteDuration(ms) {
    const min = Math.max(0, Math.floor(ms / 60000));
    if (min < 60) return `${min} min`;
    const h = Math.floor(min / 60);
    if (h < 24) return `${h} h ${min % 60} min`;
    return `${Math.floor(h / 24)} d ${h % 24} h`;
  }

  /// What is said about one instance:
  /// - `connected` — the last poll answered;
  /// - `reconnecting` — it answered before, and the last one or two did not;
  /// - `unreachable` — no answer for `REMOTE_LOST_AFTER` polls, or never;
  /// - `unpaired` — it answers but refuses the token (rotated, or wrong);
  /// - `checking` — nothing heard yet.
  function instanceView(inst, poll, now) {
    const p = poll || emptyPoll();
    const st = p.status;
    let state;
    if (p.unauthorized) state = "unpaired";
    else if (!p.failures && p.okAt) state = "connected";
    else if (!p.failures) state = "checking";
    else if (p.okAt && p.failures < REMOTE_LOST_AFTER) state = "reconnecting";
    else state = "unreachable";
    const local = inst.kind === "local";
    const name = (st && st.hostname) || inst.name || inst.url;
    const facts = [];
    if (st && state !== "unreachable") {
      facts.push(`v${st.version}`);
      facts.push(`up ${remoteDuration(st.now - st.startedAt)}`);
      facts.push(`${st.sessionsRunning} session${st.sessionsRunning === 1 ? "" : "s"} running`);
    }
    const detail = {
      connected: local ? "this clash" : "clash is running",
      checking: "checking…",
      reconnecting: `reconnecting… last answer ${remoteAgo(now - p.okAt)}`,
      unreachable: p.okAt
        ? `unreachable — last seen ${remoteAgo(now - p.okAt)}`
        : "unreachable — never answered",
      unpaired: "not paired — open a new pairing link from that clash's Settings",
    }[state];
    return {
      key: instanceKey(inst),
      kind: local ? "local" : "remote",
      url: inst.url,
      name,
      state,
      detail,
      facts,
      needsYou: st && state !== "unreachable" ? st.needsYou : null,
    };
  }

  /// The indicator every screen shows, for the instance being looked at.
  function connectionIndicator(view) {
    if (!view) return { state: "none", text: "No instance paired" };
    if (view.state === "connected")
      return { state: "connected", text: view.kind === "local" ? `${view.name} · this clash` : `Connected to ${view.name}` };
    if (view.state === "reconnecting" || view.state === "checking")
      return { state: "reconnecting", text: `Reconnecting to ${view.name}…` };
    return { state: "lost", text: `Lost ${view.name}` };
  }

  const api = {
    REMOTE_POLL_MS,
    REMOTE_LOST_AFTER,
    REMOTE_STORAGE_KEY,
    pairingToken,
    parsePairingLink,
    localInstance,
    instanceKey,
    addInstance,
    removeInstance,
    orderInstances,
    loadInstances,
    saveInstances,
    emptyPoll,
    recordPoll,
    instanceView,
    connectionIndicator,
    remoteAgo,
    remoteDuration,
  };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
