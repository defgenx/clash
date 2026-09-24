# Status hooks

clash shows a session's live state — THINKING, WAITING, PROMPTING, STASHED —
from three layers, in increasing order of latency:

1. **Hooks** (this document) — Claude Code calls a script on its own lifecycle
   events, so the state changes the instant it happens.
2. **Daemon screen analysis** — the PTY's vt100 mirror, read by the daemon.
3. **JSONL parsing** — `detect_session_status` over the transcript tail.

Only the hook layer can report two things the other two cannot infer: a
session **blocked on a permission prompt** (`PermissionRequest`), and a session
that **ended** (`SessionEnd`). Everything downstream of that — the prompt
queue refusing to type into a tool-approval dialog, the attention inbox's
"blocked tool call" band, a row going STASHED when its agent exits — depends
on this layer working.

## How registration works

Everything lives in clash's own data directory:

```
~/.claude/clash/hooks/
├── status-hook.sh     # the script: reads the event JSON on stdin, writes ../status/<id>
└── settings.json      # registers the script for 7 events
```

The daemon appends `--settings ~/.claude/clash/hooks/settings.json` to every
`claude` it spawns (`PtySession::spawn`). `--settings` **merges** with the
settings Claude Code loads on its own rather than replacing them, so the
user's own hooks, permissions and env are unaffected, and clash's file names
only clash's handlers.

Three properties of that injection are load-bearing:

- **It happens at the spawn, not at the call sites.** `PtySession::spawn` is
  the one place every claude spawn passes through — a new launcher cannot
  forget it. (There are already five: new session, resume, workflow executor,
  workflow reviewer, attach.)
- **It is agent-only.** The daemon also spawns shells (`shellterm-*`), which
  would reject the flag. Decided by `wants_hook_flag`, on the binary's
  basename: `claude` gets `--settings`, `omp` gets `-e` (see
  [OMP sessions](#omp-sessions)).
- **It is existence-guarded.** claude refuses to start at all on a
  `--settings` path it cannot read (`Error: Settings file not found`), so a
  missing hooks file must cost the status hooks and never the session.
  `install_hooks` runs at startup in **both** binaries for the same reason —
  the GUI used to rely on the TUI having run once, which left a GUI-only
  machine with no hooks at all.

## Why not the user's settings files

Registration used to be merged into `~/.claude/settings.local.json`, on the
reasoning that it was the one file inside `~/.claude/` that clash could
politely write. **Claude Code does not read that file.** The settings it
loads are:

```
~/.claude/settings.json
<project>/.claude/settings.json
<project>/.claude/settings.local.json
```

`settings.local.json` is a *project*-scoped concept; there is no user-scoped
equivalent. So the hook was registered somewhere nothing looked, and the
failure was silent in the worst way: `Phase 3`'s hook overlay applies
`starting` with **no expiry** and forces `is_running = true`, so every row
stayed pinned at the `starting` clash itself writes at spawn — a session that
had long since gone idle still read as live, `prompting` never appeared at
all, and the only status that ever moved was whatever clash wrote by hand.
Reloading a session appeared to fix it, because a reload rewrites the status
file.

`~/.claude/settings.json` would work, and is where the user's own global hooks
live, but Claude Code writes that file too — approving a permission appends to
it — so a clash startup write can clobber a concurrent one. clash's config
subsystem takes an advisory lock for exactly this hazard
(`docs/configuration.md`); a settings file clash does not own has no such
protocol available. Passing the path explicitly avoids the question: nothing
clash writes is a file anyone else writes, and *which* files Claude Code
chooses to search can never break it again.

`install_hooks` withdraws the old registration from
`~/.claude/settings.local.json` on first launch (`strip_clash_hooks`), pruning
emptied groups and events and the `hooks` key itself, and leaving every other
key byte-identical. A file it cannot parse is left alone: rewriting settings
clash does not understand would be worse than a stale entry that does nothing.

## The events, and what they mean

| Event | Status written | Why clash needs it |
|---|---|---|
| `SessionStart` | `starting` | A session exists before its transcript does. Also re-keys the registry after `/clear` (see below). |
| `UserPromptSubmit` | `thinking` | The turn began. |
| `PostToolUse` / `PostToolUseFailure` | `thinking` | Still working — keeps a long turn from reading as idle. |
| `Stop` | `waiting` | Turn finished; the prompt queue may now deliver. |
| `PermissionRequest` | `prompting` | **Blocked on a human.** Not inferable from the transcript. |
| `SessionEnd` | `idle` | The session is over — the row goes STASHED. |

`SessionStart` additionally carries clash's `/clear` handling: the hook
inherits the previous session's name and re-keys `sessions.json` to the new
conversation id, pushing the old key into the entry's `previous_ids`. Without
it a `/clear` leaves clash pointing at the pre-clear conversation — see the
resume-fork gotcha in `CLAUDE.md`.

Two properties of that re-key are load-bearing.

**Which entry.** The payload names the *new* conversation and nothing else, so
the entry is found by `cwd` — and several sessions can share one. The hook
picks the cwd match whose status file (written by this same hook on every
event) was touched most recently, because the session that just ran `/clear`
is the one the user was interacting with; the registry key breaks ties into a
stable answer. Taking the first cwd match instead always re-keyed the same
entry whatever was cleared, handing one session's conversation to another
entry — which then reads as one session vanishing and another gaining a
conversation it never had.

**The lineage is the session.** An entry's key, its `claude_session_id` and
its `previous_ids` are one session at different points in its life, and which
of them a caller holds is an accident of when it was handed the id: a GUI
pane, a workspace ownership list, and a PTY spawned before the `/clear` all
keep older ones. So anything that *finds* a session resolves through the
lineage (`resolve_resume_id`), anything that *suppresses* one covers all of it
(`session_aliases` → the kill guard, the idle status write, `unregister`), and
the refresh pipeline collapses the ids to one row per entry (Phase 5.75 in
`session_refresh.rs`). Each of those was a real bug: matching only the current
ids let a deleted session come back under a sibling id, and listing them
separately put a live session's second row under UNASSIGNED, owned by no
workspace.

## Consequence: sessions clash did not spawn

A `claude` the user started in their own terminal gets no `--settings`, so it
writes no status file. Those rows are the EXTERNAL/wild section: they are
correlated by process scan and their status comes from the transcript tail,
which is what has always driven them. The loss is `prompting` on a session
clash cannot interact with anyway.

## Verifying it works

The positive signal is a status file that moves:

```sh
id=$(uuidgen | tr 'A-Z' 'a-z')
claude -p "say ok" --session-id "$id" \
  --settings ~/.claude/clash/hooks/settings.json
cat ~/.claude/clash/status/"$id"     # → {"status":"idle",...}
```

No file means the hooks are not firing. To see which settings files Claude
Code is actually loading — the check that found this bug — run any claude
command with `--debug` and look for the `Watching for changes in setting
files` line.

## OMP sessions

OMP (oh-my-pi, `omp`) has no settings-file hooks; it has an **extension**
event bus. clash's counterpart of the script + settings pair is one
extension it owns outright:

```
~/.claude/clash/hooks/omp-status.js   # written by install_hooks, only when its bytes change
```

The daemon **prepends** `-e <that file>` to every `omp` spawn (same
basename rule, same existence guard). Prepended, not appended: a workflow
launch passes its kickoff prompt as the last positional argument, and
anything placed after it could read as message text.

The extension maps omp's events onto the same status vocabulary and writes
the same `status/<id>` files:

| omp event | status |
|---|---|
| `session_start` | `waiting` (omp is at its prompt the moment it starts) |
| `agent_start`, `tool_approval_resolved` | `thinking` |
| `agent_end` (unless `willContinue`) | `waiting` |
| `tool_approval_requested`, `tool_call` of `ask` | `prompting` |
| `tool_result` of `ask` | `thinking` |
| `session_shutdown` | `idle` |

The `ask` rows exist because omp's `ask` tool (the `AskUserQuestion`
equivalent) blocks on the human exactly like a tool approval does — the
prompt queue must not type into it.

**Identity.** omp has no `--session-id`. What it has is `--resume <path>` on
a file that does not exist yet, which it materializes at exactly that path.
clash therefore creates every OMP session at
`<omp_dir>/sessions/<bucket>/<timestamp>_<clash-uuid>.jsonl` (bucket = omp's
own cwd encoding, `omp::encode_session_dir_name`), and the uuid **suffix of
the file name** — not the header's `id`, which omp mints itself — is the
session id everywhere: registry key, status file, wild-scan evidence. The
extension derives it from `sessionManager.getSessionFile()` the same way.
Sessions omp creates on its own (`/new`) follow the same naming with omp's
uuid, so the rule holds for them too.

**Conversation switches.** `/new` and `/resume` inside omp fire
`session_switch` with the previous file. The extension re-keys the registry
entry that answered to the previous id — exactly the shape the Claude
hook's `/clear` branch produces (new key, `claude_session_id` = new id, old
key appended to `previous_ids`) — and bumps the saved name's numeric suffix.
Unlike the Claude hook it does not guess the entry by cwd: the previous file
names it.

**No resume forks.** `omp --resume <file>` appends to that file, so
`chase_resume_forks` / `heal_registry_forks` skip OMP entries (the registry
records `agent: "omp"`; a missing field reads as Claude, which is what every
older entry is).

