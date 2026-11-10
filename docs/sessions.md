# Sessions

How clash finds agent sessions, tells you what each one is doing, and starts
new ones. Everything here applies to both frontends; the GUI's session UI is in
[gui.md](gui.md#sessions) and the TUI's keys in [tui.md](tui.md#sessions).

- [Status detection](#status-detection)
- [Session sources](#session-sources)
- [Wild sessions](#wild-sessions)
- [OMP sessions](#omp-sessions)
- [Session presets](#session-presets)

## Status detection

At every launch, clash (re)writes its lifecycle hooks to `~/.claude/clash/hooks/`
(plus the OMP status extension) for instant status detection. clash passes that
hook file to every session it spawns (`claude --settings …`), so it registers
nothing in your own settings files — see [hooks.md](hooks.md).

clash detects session status through three layers (in priority order):

1. **Hooks** — Claude Code lifecycle events (`PermissionRequest`, `Stop`,
   `SessionStart`, etc.) write instant status updates.
2. **Daemon PTY** — screen content analysis pattern-matches the terminal for
   prompts, approval dialogs and thinking indicators.
3. **JSONL baseline** — conversation log heuristics (last entry type, stop
   reasons, timing).

| Icon | Status | Meaning |
|------|--------|---------|
| `◆◇` | Prompting | Claude needs tool approval — blinking diamond |
| `◉` | Waiting | Awaiting your next prompt |
| `◌◎◉` | Thinking | Reasoning / generating — pulsing circle |
| `⠋⠙⠹…` | Running | Executing tools — braille spinner |
| `○◔◑◕●` | Starting | Session just spawned — filling circle |
| `✗` | Errored | Session crashed shortly after starting |
| `○` | Stashed | Exited or inactive |
| `✓` | Done | A subagent that finished |

Sessions are grouped by status into Active / Done / Fail / External sections.

## Session sources

Each row in the sessions list may carry a single-character prefix indicating
where its underlying Claude process lives:

| Prefix | Source | Meaning |
|--------|--------|---------|
| (none) | Daemon | clash spawned and manages the PTY — attach inline with `a` (TUI) or a click (GUI) |
| `⊞ ` | External | clash spawned the process in another pane/tab/window via `o`/`O` (TUI) |
| `🌿 ` | Wild | A `claude` process started outside clash; see below |

## Wild sessions

A **Wild** session is a `claude` process started outside clash. Press `a` (TUI)
or click the row or its ⚡ button (GUI) to take it over: one confirm, then clash
kills the outside process (SIGTERM, SIGKILL after 2s) and attaches to its
conversation under the daemon (`--resume <id>`).

- Detection runs in the background every ~2s. clash surfaces every wild claude
  PID **that started after this clash launched** under the EXTERNAL section —
  pre-existing claudes from before clash booted are intentionally hidden, since
  the section is for things spawned during this session.
- Each wild process is **dynamically associated with a conversation**: exact
  evidence first (`--resume <id>` / `--session-id <id>` in argv, or — rarely —
  the `.jsonl` held open as an fd), otherwise the **most recently modified
  conversation in the process's working directory**. The association is
  re-evaluated on every scan, so it always tracks the latest conversation.
- Only a bare `claude` in a directory with no conversation on disk at all
  (typically the few seconds before a brand-new conversation's JSONL appears)
  shows as a PID-keyed row with takeover disabled.
- Press `d` (TUI) to drop a wild row: clash signals the PID directly (SIGTERM,
  SIGKILL after 5s if still alive and still claude). The row also disappears on
  the next scan tick once the process exits, so closed/stopped claudes never
  linger.
- List the section in isolation with `:external` (TUI).

## OMP sessions

clash runs [OMP](https://omp.sh) (oh-my-pi, `omp`) sessions next to Claude Code
ones; the choice is made per session (the GUI's new-session dialog, the TUI's
last new-session prompt, or `:new --agent omp <path>`).

Every workflow step — plan, implement, change rounds, reviews, answering PR
comments, explanations, drift, shares — follows one setting: the item's ⚙
Settings tab, else `workflows.agent` (`ask` by default, or `claude` / `omp`). A
fixed agent runs every step unasked; `ask` asks at every start — a picker for
one-click starts, a *Which agent runs it?* group in the review composer (code
review, drift) and a *Run on* row in the change-request composer —
pre-selecting the agent the item last ran on, which is also what a relaunch or
an auto-applied round uses without asking. An agent whose binary does not
resolve is greyed out with the reason, in these pickers and in the new-session
dialog, and the backend refuses it before any set-up.

An OMP session is the same row as any other — the GUI badges every row with its
agent (`CC` for Claude Code, `OMP`) — and everything built around sessions
applies:

- **Identity and resume.** clash **seeds** the transcript itself — omp's own
  new-session shape, title slot plus session header — at
  `~/.omp/agent/sessions/<bucket>/<timestamp>_<id>.jsonl` and resumes it with
  `omp --resume <that path>` (omp 18 refuses a `--resume` path that does not
  exist), so the file's uuid is the session id exactly like
  `claude --session-id`. A resume reopens that file (omp appends in place — no
  fork to chase).
- **Status.** A clash-owned extension (`~/.claude/clash/hooks/omp-status.js`,
  loaded with `omp -e`) writes the same status files as the Claude hook, from
  omp's own events — a tool-approval prompt or an `ask` question reads as
  *prompting*. `/new` and `/resume` inside omp re-key the row like `/clear`.
- **Skills.** The embedded `clash-*` skills are also installed into
  `~/.omp/agent/skills/` at startup (when that directory exists) and again
  before every OMP workflow launch, so an omp installed since startup still has
  the skills its kickoff names.
- **Workflows.** OMP workflow sessions launch with `workflows.omp_model` (empty
  = omp's own default model) instead of `workflows.lead_model`, and their
  subagents run on `workflows.omp_subagent_model` (empty = omp's own `task`
  model) instead of `workflows.subagent_model`.

Settings: `general.omp_bin`, `general.default_agent`, `paths.omp_dir`,
`workflows.agent`, `workflows.omp_model`, `workflows.omp_subagent_model`.
Details: [hooks.md](hooks.md#omp-sessions).

## Session presets

Presets are reusable templates for session creation. When presets are
available, pressing `n` (TUI) shows a picker; otherwise the manual flow is used
(directory → name → worktree → agent). The GUI's new-session dialog offers them
too.

### Project presets (`.clash/presets.json`)

```json
{
  "presets": {
    "backend-fix": {
      "description": "Backend bugfix workflow",
      "directory": "./",
      "worktree": true,
      "setup": ["./.clash/setup-backend.sh"],
      "teardown": ["./.clash/teardown.sh"]
    },
    "frontend-feature": {
      "description": "New frontend feature",
      "directory": "./frontend",
      "worktree": false
    }
  }
}
```

### Global presets (`presets.json` in the config directory)

Same format as project presets. Project presets override global presets with
the same name.

### Superset compatibility

If `.superset/config.json` exists, it appears as a synthetic "superset" preset
with the `setup` and `teardown` fields mapped directly.

### Preset fields

| Field | Type | Description |
|-------|------|-------------|
| `description` | string | Shown in the preset picker |
| `directory` | string | Working directory (relative or absolute) |
| `prompt` | string | Initial prompt sent to the session (GUI) |
| `worktree` | bool? | `true`/`false` = auto, omit = ask |
| `setup` | string[] | Scripts to run after session creation |
| `teardown` | string[] | Scripts to run before session drop |

Setup scripts receive `CLASH_ROOT_PATH` and `CLASH_SESSION_ID` env vars. Each
script has a 30s timeout. `setup`/`teardown` run from the TUI; the GUI applies a
preset's directory, worktree and prompt.
