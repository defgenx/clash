# Configuration file

One file, shared by the TUI and the GUI. Find it with `clash config --path`
(`~/.config/clash/config.toml` on Linux, `~/Library/Application
Support/clash/config.toml` on macOS, `%APPDATA%\clash\config.toml` on Windows).
The GUI's Settings window (`⌘,`) writes the same file — see
[gui.md → Settings](gui.md#settings).

This page is the user-facing reference. How the subsystem works internally —
the schema table, provenance, the write path and migrations — is in
[configuration.md](configuration.md).

## Inspecting it

```bash
clash config                    # the merged config, annotated with where each value came from
clash config --path             # just the path
clash config --defaults         # the full annotated default file, ready to copy lines out of
clash config --show-effective   # same as bare `clash config`
clash config --validate         # check the file; exits non-zero on an error
clash config --schema           # JSON Schema, for taplo / Even Better TOML completion
```

The flags are mutually exclusive. Bare `clash config` exits 1 on a parse error
(after printing the defaults it fell back to). In the TUI, `:config` shows the
path and `:reload` re-reads the file.

## Layers

Later layers win, key by key:

| Layer | Where | Notes |
|-------|-------|-------|
| defaults | in the binary | `clash config --defaults` prints them |
| user | `clash config --path` | what the GUI Settings window writes |
| project | `<repo>/.clash/config.toml` (found by walking up from the cwd) | **restricted** (see below) |
| environment | `CLASH_<SECTION>_<KEY>` | any key, e.g. `CLASH_SESSIONS_REFRESH_SECS=5` |

A project config may set only `paths.claude_dir`, `paths.scratch_dir`,
`paths.workflows_dir`, `[actions]` and `notifications.hooks`. It deliberately
**cannot** change `claude_bin`, `omp_bin` or any other key — clash spawns
processes, so a cloned repo must not be able to decide which binary runs.
Rejected keys and unknown `CLASH_*` variables are reported by
`clash config --validate`, not silently ignored (`CLASH_LOG_RETENTION_HOURS` is
the one `CLASH_*` variable that is not a config key).

## Settings

```toml
schema_version = 2

[general]
claude_bin = "claude"      # name on PATH, or an absolute path
skills_update = "ask"      # when an upgrade ships skills you edited: ask | all | keep
update_check = true        # look for a new release in the background (GUI: startup + every 6h; TUI: startup)
omp_bin = "omp"            # OMP (oh-my-pi) binary for OMP sessions
default_agent = "claude"   # agent new sessions pre-select: claude | omp
debounce_ms = 200          # filesystem-watcher debounce

[paths]
claude_dir = ""            # empty = ~/.claude
omp_dir = ""               # OMP agent dir; empty = ~/.omp/agent
scratch_dir = ""           # empty = <claude_dir>/clash/scratch
workflows_dir = ""         # empty = <claude_dir>/clash/workflows

[sessions]
default_cwd = ""           # prefill for a new session; empty = home
confirm_kill = true        # ask before killing (stash never asks)
refresh_secs = 2           # GUI session-list poll cadence

[terminal]
shell = ""                 # in-app terminals; empty = $SHELL
tui_terminal = ""          # TUI launcher target; empty = auto-detect

[notifications]
enabled = true
title_attention = true     # "clash (2!)" in the window title

[workflows]
agent = "ask"              # agent CLI workflow sessions run on: ask | claude | omp (per-item override)
omp_model = ""             # --model for OMP workflow sessions; empty = omp's default
lead_model = "claude-opus-5-5"     # --model for every Claude workflow session (the lead)
delegation = "team"        # team: lead fans work out to parallel subagents | solo
assist = "suggest"         # suggest: highlight the next step | autopilot: also start it when it decides nothing | off
explain_detail = "critical"  # how much an explanation writes up front: overview | critical | full
subagent_model = "claude-sonnet-5-5" # model those subagents run on (Claude Code); empty = inherit the lead's
omp_subagent_model = ""    # model those subagents run on (OMP); empty = omp's own task model
review_sizing = "auto"     # size code reviews (verdicts included) by the change: auto | off (Claude Code only)
small_review_model = "claude-sonnet-5-5"          # lead of a small sized review; empty = lead_model
large_review_subagent_model = "claude-opus-5-5"   # subagents of a large sized review; empty = subagent_model
workspace_dir = ""         # folder holding your repositories, where new items live; empty = ask
pr_skill = "hivebrite-engineering:github-pr"  # skill the PR phase opens PRs with; "none" disables
forge = "auto"             # code forge for PR features: auto | github | none
pr_stacking = false        # treat an item's PRs as a GitHub stack (per-item override)
slack_webhook = ""         # Slack incoming webhook for sharing + notifications
discord_webhook = ""       # Discord webhook for sharing + notifications
notify_webhook = "off"     # announce decision states: off | slack | discord
chat_transport = "agent"   # who posts a share to Slack/Discord: agent | clash (the webhooks)
chat_skill = ""            # skill the agent chat route goes through; empty = the session's own tooling

[trackers]                 # ticket systems: where keys link, where a share posts
primary = "jira"           # owner of keys no tracker lists: jira | linear | none (no linking)

[trackers.jira]
site = ""                  # yourorg.atlassian.net — keys link to <site>/browse/<KEY>; empty = no Jira
prefixes = ""              # keys owned by Jira when Linear is set too, e.g. "PS, OPS"
transport = "agent"        # who posts a share: agent (a session) | clash (direct, with email + token)
skill = ""                 # skill the agent route goes through; empty = the session's own tooling
email = ""                 # Jira account email (clash transport)
api_token = ""             # Jira API token (id.atlassian.com → Security)

[trackers.linear]
workspace = ""             # slug or linear.app URL — keys link to linear.app/<slug>/issue/<KEY>
prefixes = ""              # team keys owned by Linear, e.g. "ENG, WEB"
transport = "agent"        # who posts a share: agent (a session) | clash (Linear API, with api_key)
skill = ""                 # skill the agent route goes through; empty = the session's own tooling
api_key = ""               # Linear personal API key (clash transport)

[[ides]]                   # extra editors offered when opening a project or note
name = "VS Code"
command = "code"
terminal = false
description = ""           # optional
```

What the workflow keys do is explained in the
[Workflows guide](workflows-guide.md); the tracker and sharing keys in its
[Ticket trackers and sharing](workflows-guide.md#ticket-trackers-and-sharing)
section.

The GUI's 20 xterm-rendering settings (font, cursor, scrollback, scroll, link
handling) and its theme stay in the GUI's own store (`gui-state.json`) — the TUI
can't apply them. Everything above is read by both.

## Behaviour worth knowing

- **Edits apply live.** The config directory is watched; a change by hand, by
  the GUI, or by another clash instance is picked up without a restart.
  `:reload` forces it. `general.claude_bin`, `general.omp_bin`,
  `general.debounce_ms` and `paths.omp_dir` take effect on restart, and clash
  says so rather than pretending otherwise.
- **A typo never loses your settings.** A parse error keeps the last good values
  in memory, reports the failure with `line:column`, and *blocks writes* until
  you fix it — so the next GUI toggle can't overwrite your file with defaults.
- **Your comments and unknown keys survive a save.** Writes edit the parsed
  document and touch only the keys that changed, so comments, key order, and any
  key this version doesn't know (including one a newer clash wrote) round-trip
  intact.
- **Concurrent instances are safe.** Several clash processes run by design; the
  whole read-modify-write happens under an advisory lock, so two of them editing
  different settings can't drop each other's change.
