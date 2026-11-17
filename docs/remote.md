# Seeing clash from elsewhere

clash can tell you, from your phone or from another machine's clash, whether an
instance is running and what waits on you there — and, once you allow it, let
you **answer it from your phone**: open a session, see its screen, pick an
option of the prompt it is waiting on, press Enter/Esc/⌃C, queue a follow-up or
stash it.

## The model: instances

Everything here is built on one idea, an **instance**: a clash GUI you can see.

- **local** — the clash you are looking at. Its status comes from its own
  state: no network, no token.
- **remote** — another clash, reached over HTTP with the pairing token it
  handed out. It may run on another machine, or on the same one.

Status, polling and the inbox work the same for both. Only the transport
differs. One model, `gui/dist/remote-model.js`, drives both clients:

| Client | Where | Instances | Transport |
|---|---|---|---|
| **Machines view** | inside the clash GUI (the monitor icon beside the inbox) | this clash first, then every paired one | the GUI's backend: it polls peers with their tokens, so the webview never holds one |
| **Remote page** | any browser, typically your phone: `http://<address>/remote/` | the instances paired on that phone | `fetch` from the page |

Each instance shows as:

- **connected**: the last poll answered. You see its version, uptime, running
  sessions and how many things wait on you.
- **reconnecting**: it answered before, but the last one or two polls did not.
- **unreachable · last seen …**: no answer for three polls (15 s), or never.
- **not paired**: it answers, but refuses the token (it was rotated).

Polls run every 5 s. The remote page also shows a connection indicator on every
screen (connected / reconnecting / lost) and, for the selected instance, its
inbox. The inbox rows are the desktop inbox's own (`attention.js`).

## Answering a session from your phone

Turn on **Settings → Remote → Allow actions from paired phones**
(`remote.actions`, off by default). Then, on the phone page, tap a session in
*Waiting on you*:

- **Its screen**, re-read every 1.5 s: the session's visible terminal as text,
  the bottom of it first.
- **The prompt on it**, when there is one — a permission dialog, a numbered
  question — as one big button per option (`1 · Yes`, `2 · Yes, and don't ask
  again…`, `3 · No…`), with the question above them.
- **A key bar**: ↑ ↓ ⇧⇥ to move a selection, ⏎ Enter, Esc, ⌃C.
- **Follow-up**: a message queued exactly like the desktop's *Queue
  follow-up…* — delivered when the session is next idle at its prompt, never
  typed into a turn in flight.
- **Stash session**: stop it and keep it resumable.

Anything that answers, interrupts or stops (an option, Enter, Esc, ⌃C, Stash)
takes **a second tap** within 3 seconds: the first one arms the button and says
*Tap again*. Moving the selection goes at once.

With actions off, the page still shows the inbox; tapping a session says the
actions are off on that clash.

## Setting it up

1. On the machine whose clash you want to see: **Settings → Trackers & sharing →
   Remote**, tick **Serve the remote page**.
2. Choose the address it listens on (`remote.bind`):
   - `127.0.0.1:7878` (the default): this machine only. Another clash on the
     same machine can pair with it; your phone cannot reach it.
   - your **Tailscale** address, e.g. `100.101.102.103:7878`: every device on
     your tailnet, including your phone (with the Tailscale app connected).
     Find it with `tailscale ip -4`.
   - a LAN address works too, but then anyone on that network can try the
     token, and the traffic is not encrypted (see below).
3. **Copy pairing link**, then:
   - on your phone, open it in Safari (send it to yourself through something
     private). The page remembers the instance and drops the token from the
     address bar. *Add to Home Screen* makes it an app-like icon.
   - in another clash, paste it in **Machines → Paste a pairing link**.

The server starts, moves or stops as soon as you change the setting, whether in
Settings or by hand in `config.toml`. No restart is needed.

## Security model

- **Off by default.** Nothing listens until you enable it, and the default
  address is loopback. A repository's `.clash/config.toml` can't turn it on:
  `remote.*` is not project-overridable.
- **One token per instance.** It is 32 random bytes from the OS, stored in
  `~/.claude/clash/remote-token` with mode 0600, created when the server is first
  enabled, and never logged. Every `/api/*` call must send
  `Authorization: Bearer <token>`, compared in constant time. The page itself
  (HTML/JS) is public; it shows nothing without a token.
- **The token never rides a URL.** The pairing link carries it in the fragment
  (`#token=…`), which browsers don't send to the server, so it can't land in a
  server or proxy log. The page sends it only as a header.
- **No token, no server.** If the token can't be created, the server refuses
  to start, on any address.
- **New pairing token** replaces it. Every paired phone and clash then reads
  *not paired* until it pairs again. Use it if a link leaked.
- **Peers' tokens stay in the backend.** The Machines view's paired instances
  live in `~/.claude/clash/remote-peers.json` (0600). The webview only ever
  sees their names and URLs.
- **Read-only unless you allow actions.** With `remote.actions` off the API
  has two GET routes and no way to change anything. With it on, three POST
  routes act on a session — and only on a session the GUI lists — and the
  screen route shows its terminal. Every action route checks the token, then
  the setting, then the session, before it reads or does anything (a source
  test pins the order).
- **Named keys, never bytes.** The keys route accepts a fixed list — `1`–`9`,
  `enter`, `esc`, `ctrl-c`, `tab`, `shift-tab`, `up`, `down` — and nothing
  else, so a stolen token cannot type a command into a shell session. A
  follow-up is free text, but it goes through the prompt queue: delivered only
  at the session's idle prompt, as a bracketed paste with control bytes
  stripped.
- **Plain HTTP.** The server speaks HTTP, not HTTPS. Over Tailscale that is
  fine: the tailnet encrypts and authenticates every packet. On a LAN or
  anything else, the token and the status travel in clear, so prefer Tailscale.
- **CORS is open (`*`).** A phone page served by one instance polls the
  others, so the API answers any origin. The bearer token, never a cookie, is
  what authorizes a call, so a foreign page gains nothing without it.

## API

| Route | Auth | Answer |
|---|---|---|
| `GET /remote/` (+ `remote.js`, `remote-model.js`, `attention.js`) | none | the remote page |
| `GET /api/status` | token | `{ app: "clash", hostname, version, pid, startedAt, now, sessionsRunning, needsYou }` |
| `GET /api/state` | token | `{ status, sessions, workflows }`, the lists the GUI last built |
| `GET /api/sessions/{id}/screen` | token + actions | `{ lines, prompt }`: the visible rows, and `{ question, choices: [{ key, label }] }` when a numbered prompt is on screen (else `null`) |
| `POST /api/sessions/{id}/keys` | token + actions | `{ "key": "1" }` → 204; an unknown key is 400 |
| `POST /api/sessions/{id}/followup` | token + actions | `{ "text": "…" }` → `{ queued }` |
| `POST /api/sessions/{id}/stash` | token + actions | 204 |

Action routes answer 403 while `remote.actions` is off, 404 for a session the
GUI does not list, and 502 when the daemon refused (a stashed session has no
screen).

`needsYou` counts the inbox rows (`application::remote::needs_you` mirrors
`attentionRows`). `/api/state` reads the GUI's caches: the session list
`list_sessions` last merged and the workflow list `list_workflow_items` last
returned. It never rebuilds them, so a phone poll can't run the session merge
or drain the prompt queue. A source test pins that. The data is as fresh as
the GUI's own refresh.

## Where the code is

- `src/application/remote.rs`: the pure half (token shape and constant-time
  check, bearer parsing, bind parsing, pairing links, the peer list,
  `InstanceStatus`, `needs_you`, the named keys `key_bytes`, and
  `screen_prompt`, which finds a numbered prompt's options on a screen).
- The daemon's `Screen` request (`daemon/protocol.rs`, answered from the
  session's vt100 mirror): the screen as text.
- `gui/src-tauri/src/remote.rs`: the server (axum), the token file, the
  settings commands, the Machines view's commands, and the plain-HTTP client
  that polls peers.
- `gui/dist/remote-model.js`: the client model shared by the Machines view and
  the remote page, including the session view's buttons and the second-tap
  rule (`choiceButtons`, `KEY_BAR`, `tapButton`).
- `gui/dist/remote/`: the remote page.
- The *Machines* section of `gui/dist/app.js`: the desktop view.

## Not yet

- **Workflow decisions** from the phone: approving, signing off, requesting
  changes, answering an item's ❓ decisions.
- **A live terminal** on the phone (streamed output and free typing); the
  session view shows the screen and answers prompts, but does not stream.
- **Actions from the desktop Machines view** on a paired clash.
- **"Not running" vs "asleep"**: an always-on helper (a launchd agent) that
  answers when clash is closed and offers ▶ Start clash.
- **A native iOS app** with push notifications.
- **A relay** for reaching an instance without Tailscale.
