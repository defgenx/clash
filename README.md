<p align="center">
  <img src="assets/logo.svg" alt="clash logo" width="500">
</p>

<p align="center">
  <strong>Terminal UI for Claude Code Sessions & Agent Teams</strong>
</p>

<p align="center">
  <a href="#installation">Install</a> &bull;
  <a href="#features">Features</a> &bull;
  <a href="#usage">Usage</a> &bull;
  <a href="#keybindings">Keys</a>
</p>

---

## Features

- **Session management** — list, attach, detach, create, stash, and delete Claude Code sessions
- **Inline terminal** — attach to sessions with a full terminal passthrough, status bar showing session name / project / branch
- **Real-time status** — instant status detection via hooks, daemon PTY screen analysis, and JSONL parsing (three-layer system)
- **Animated status icons** — active sessions show animated spinners and pulsing icons for visual feedback
- **Section-based layout** — sessions are grouped into Active (working), Done (idle/stashed), Fail (errored), and External (wild claude processes started outside clash, kept at the bottom so they don't interleave with clash-managed rows) with stable alphabetical ordering; press `A` to cycle section filter
- **In-process daemon** — embedded PTY daemon manages sessions without a separate process
- **Git worktree support** — spawn sessions in isolated worktrees for parallel feature branches (`w` key); worktree column shows `⊟ project/worktree` for project context
- **Repo config discovery** — auto-detects MCP servers, custom commands, agent definitions, and setup scripts from the project directory
- **Teams & tasks** — create, configure, and delete teams (description, members with agent type and model); organize agents, manage tasks, send messages
- **Scratches** — keep free-form text notes inside clash (`:scratch`); create, list, and reopen them anytime. Each note is a plain file under `~/.claude/clash/scratch/` by default — set `scratch_dir` in `config.toml` (or the GUI **Scratch directory** setting) to store them anywhere. Opening a scratch shows an editor picker: terminal editors (vim/emacs/nano…) open in a tab/pane, GUI editors (VS Code/Cursor/Zed…) launch alongside, like opening a project
- **Subagent tracking** — view subagent trees per session, expand/collapse in the sessions table
- **Open in IDE** — press `e` to open a session's project in your editor (auto-detects Cursor, VS Code, Zed, JetBrains, nvim, vim; configurable)
- **Keyboard-driven** — vim-style navigation, command mode (`:`), fuzzy filter (`/`), context help (`?`)
- **UI state persistence** — restores navigation, selection, filters, and expanded sessions on restart
- **Multi-instance** — run several clash apps (TUI and/or GUI) side by side; each owns its own sessions via a per-instance daemon socket
- **Guided tour** — first-launch walkthrough, replay anytime with `:tour`
- **Debug mode** — `clash --debug` enables verbose logging with a header indicator
- **Self-updating** — `:update` in the TUI or `clash update` from the CLI

## Installation

### Quick install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/defgenx/clash/main/install.sh | bash
```

Custom install path:

```bash
CLASH_INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/defgenx/clash/main/install.sh | bash
```

### Build from source

```bash
cargo install --git https://github.com/defgenx/clash.git
```

Or from a clone — installs **both** the TUI and the GUI
(override paths with `INSTALL_DIR=~/.local/bin` / `APP_DIR=~/Applications`):

```bash
make install            # or: make install-tui / make install-gui
```

The TUI installs as the `clash` binary in `INSTALL_DIR`. The GUI installs
as a regular desktop application, discoverable like any other app:

- **macOS** — `Clash.app` in `/Applications` (falls back to
  `~/Applications` when not writable): Spotlight, Launchpad, Dock. A
  `clash-gui` symlink lands in `INSTALL_DIR` for terminal launching.
- **Linux** — `clash-gui` binary plus an XDG `clash.desktop` launcher
  entry and icon (system-wide under `/usr/local/share` as root,
  per-user under `~/.local/share` otherwise).

### Requirements

- Rust 1.75+ (for building from source)
- Claude Code CLI (`claude`)

## Usage

```bash
clash                              # Start (reads from ~/.claude)
clash --data-dir ~/.claude         # Custom data directory
clash --claude-bin /path/to/claude # Custom CLI path
clash --debug                      # Enable debug logging
clash update                       # Update to the latest release
```

On first launch, clash installs lifecycle hooks into `~/.claude/settings.local.json` for instant status detection and shows a guided tour. Replay it anytime with `:tour`.

### Session Status

clash detects session status through three layers (in priority order):

1. **Hooks** — Claude Code lifecycle events (`PermissionRequest`, `Stop`, `SessionStart`, etc.) write instant status updates
2. **Daemon PTY** — screen content analysis pattern-matches the terminal for prompts, approval dialogs, and thinking indicators
3. **JSONL baseline** — conversation log heuristics (last entry type, stop reasons, timing)

| Icon | Status | Meaning |
|------|--------|---------|
| `◆◇` | Prompting | Claude needs tool approval — blinking diamond |
| `◉` | Waiting | Awaiting your next prompt |
| `◌◎◉` | Thinking | Reasoning / generating — pulsing circle |
| `⠋⠙⠹…` | Running | Executing tools — braille spinner |
| `○◔◑◕●` | Starting | Session just spawned — filling circle |
| `✗` | Errored | Session crashed shortly after starting |
| `○` | Stashed | Exited or inactive |

### Session Source Prefixes

Each row in the sessions list may carry a single-character prefix indicating where its underlying Claude process lives:

| Prefix | Source | Meaning |
|--------|--------|---------|
| (none) | Daemon | clash spawned and manages the PTY — attach with `o` or Enter |
| `⊞ `  | External | clash spawned the process in another pane/tab/window via `o`/`O` |
| `🌿 ` | Wild | A `claude` process started outside clash. Press `a` to take over: one confirm, then clash kills the outside process (SIGTERM, SIGKILL after 2s) and attaches to its conversation under the daemon (`--resume <id>`) |

The Wild detection runs in the background every ~2s. clash surfaces every wild claude PID **that started after this clash launched** under the EXTERNAL section — pre-existing claudes from before clash booted are intentionally hidden, the section is for things spawned during this session. Each wild process is **dynamically associated with a conversation**: exact evidence first (`--resume <id>` / `--session-id <id>` in argv, or — rarely — the `.jsonl` held open as an fd), otherwise the **most recently modified conversation in the process's working directory**. The association is re-evaluated on every scan, so it always tracks the latest conversation. Only a bare `claude` in a directory with no conversation on disk at all (typically the few seconds before a brand-new conversation's JSONL appears) shows as a PID-keyed row with takeover disabled. Press `d` to drop a wild row: clash signals the PID directly (SIGTERM, SIGKILL after 5s if still alive and still claude). The row also disappears on the next scan tick once the process exits, so closed/stopped claudes never linger. List the section in isolation with `:external`. The GUI behaves the same way: clicking a wild row (or its ⚡ button) confirms, takes over, and opens the terminal.

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `j` / `k` | Select next / previous |
| `g` / `G` | Jump to first / last |
| `Enter` | Drill in |
| `Esc` | Go back |
| `q` | Quit (with confirmation) |

### Modes

| Key | Description |
|-----|-------------|
| `:` | Command mode — `:teams`, `:sessions`, `:tour`, `:update`, `:quit` |
| `/` | Fuzzy filter |
| `?` | Context help |

### Sessions

| Key | Action |
|-----|--------|
| `a` | Attach (inline terminal); on a 🌿 wild row: take over and attach (one confirm) |
| `p` | View git diff |
| `e` | Open project in IDE (auto-detect + picker) |
| `o` | Open in new pane / tab / window |
| `O` | Open ALL running sessions (smart layout) |
| `c` / `n` | New session (two-step: directory, then name) |
| `s` | Stash / unstash session (stop process, keep in registry) |
| `w` | Spawn session in a git worktree |
| `Tab` | Expand / collapse subagents |
| `A` | Cycle section filter (Active/Done/Fail/External) |
| `S` | Toggle active / all |
| `d` | Drop session |
| `D` | Drop ALL sessions |
| `i` | Inspect (drill into detail) |

### Teams

| Key | Action |
|-----|--------|
| `c` | Create team |
| `d` | Delete team |
| `e` | Edit team description |
| `m` | Add member (name → agent type → model) |
| `x` | Remove member (picker) |
| `Enter` | View team detail |

### Attached Mode

A status bar at the bottom shows session name, project, and git branch. The PTY is resized to fit above the bar.

| Key | Action |
|-----|--------|
| `Ctrl+B` | Detach (works across all terminal encodings) |
| Everything else | Forwarded to Claude |

### Session Detail

| Key | Action |
|-----|--------|
| `s` | Subagents |
| `t` | Linked team |
| `m` | Team members |
| `p` | View git diff |
| `a` | Attach |
| `e` | Open in IDE |
| `d` | Drop |

### Diff View

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll diff content |
| `n` / `p` | Next / previous file |
| `r` | Refresh diff |
| `Esc` | Go back |

### Team Detail

| Key | Action |
|-----|--------|
| `Enter` / `a` | View agents |
| `t` | View tasks |
| `s` | View lead session |
| `e` | Edit team description |
| `m` | Add member (name → agent type → model) |
| `x` | Remove member (picker) |
| `d` | Delete team |

### Scratches

Reach the Scratches view with `:scratch` (also `:notes`).

| Key | Action |
|-----|--------|
| `c` / `n` | New scratch (prompts for a title) |
| `Enter` / `e` | Open the selected scratch in an editor (picker) |
| `d` | Delete the selected scratch |

Scratches are plain files under `~/.claude/clash/scratch/` by default; override
the location with `scratch_dir` in `config.toml` or the GUI **Scratch
directory** setting (which writes the same key, so the TUI honors it too). The
editor picker lists installed IDEs (Cursor, VS Code, Zed, JetBrains, …) and
terminal editors (vim, nvim, emacs, nano, helix, micro); terminal editors open
in a tab/pane, GUI editors launch alongside. In the GUI, scratches live in a
collapsible **Scratches** sidebar section; opening one shows the editor picker —
a terminal editor opens in an in-app terminal tab, a GUI editor launches
alongside.

### Commands

| Command | Action |
|---------|--------|
| `:teams` | Navigate to Teams view |
| `:sessions` | Navigate to Sessions view |
| `:agents` | Navigate to Agents view |
| `:tasks` | Navigate to Tasks view |
| `:subagents` | Navigate to Subagents view |
| `:inbox` | Navigate to Inbox view |
| `:prompts` | Navigate to Prompts view |
| `:scratch` / `:notes` | Navigate to Scratches view |
| `:create team <name>` | Create a new team |
| `:delete team <name>` | Delete a team |
| `:member model <member> [model]` | Set a member's model on the current team (empty = inherit) |
| `:create task <team> <subject>` | Create a task |
| `:new [path]` | Spawn a new session |
| `:new --preset <name>` | Spawn session from a preset |
| `:diff` | View git diff for current session |
| `:rename <name>` | Rename session (from detail view) |
| `:active` / `:all` / `:external` | Filter sessions (active only / all / wild + external only) |
| `:tour` | Replay guided tour |
| `:update` | Update clash |
| `:quit` | Exit |

## Data

clash reads directly from Claude Code's filesystem:

```
~/.claude/
├── projects/{name}/
│   ├── sessions-index.json            # Session index with summaries
│   ├── {session-id}.jsonl             # Conversation log
│   └── {session-id}/subagents/        # Subagent transcripts
├── teams/{name}/config.json           # Team config + members
├── tasks/{team-name}/{id}.json        # Tasks
└── settings.local.json                # Hook registrations (written by clash)
```

clash also maintains its own state in `~/.claude/clash/`:

```
~/.claude/clash/
├── hooks/status-hook.sh               # Lifecycle hook script
├── status/{session-id}                # Instant status from hooks
├── names/{session-id}                 # Session display names
├── project-names/{encoded-cwd}        # Project-to-name mapping
├── sessions.json                      # Session registry
├── ui_state.json                      # Persisted UI state (nav, selection, filters)
├── scratch/{name}.md                  # Scratch notes (free-form text files)
└── trusted_repos.json                 # SHA256 trust store for repo setup scripts
```

Daemon sockets: `~/Library/Application Support/clash/daemon-<pid>.sock` (one
per running instance; `clash attach` auto-discovers the instance that owns a
session).

## Session Presets

Presets are reusable templates for session creation. When presets are available, pressing `n` shows a picker; otherwise the manual 3-step flow is used.

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

### Global presets (`~/.config/clash/presets.json`)

Same format as project presets. Project presets override global presets with the same name.

### Superset compatibility

If `.superset/config.json` exists, it appears as a synthetic "superset" preset with the `setup` and `teardown` fields mapped directly.

### Preset fields

| Field | Type | Description |
|-------|------|-------------|
| `description` | string | Shown in the preset picker |
| `directory` | string | Working directory (relative or absolute) |
| `prompt` | string | Initial prompt for Claude |
| `worktree` | bool? | `true`/`false` = auto, omit = ask |
| `setup` | string[] | Scripts to run after session creation |
| `teardown` | string[] | Scripts to run before session drop |

Setup scripts receive `CLASH_ROOT_PATH` and `CLASH_SESSION_ID` env vars. Each script has a 30s timeout.

## Architecture

clash follows **The Elm Architecture** (TEA) with clean architecture layers:

```
User Input → Action → reducer() → (State', Effects) → execute_effects() → draw()
                        (pure)                          (infrastructure IO)
```

| Layer | Purpose |
|-------|---------|
| **Domain** | Entities, port traits — no dependencies |
| **Application** | State, actions, effects, pure reducer |
| **Adapters** | Input mapping, view rendering |
| **Infrastructure** | Event loop, filesystem, daemon, CLI, TUI widgets |

## Development

```bash
cargo test          # Run all tests
cargo clippy        # Lint
cargo fmt --check   # Check formatting
```

Releases are automatic — push with conventional commits (`feat:`, `fix:`) and CI handles the rest.

## GUI (experimental)

A cmux-style desktop client lives in `gui/` — a Tauri 2 app sharing the same
core as the TUI (session pipeline, in-process PTY daemon, protocol). Sidebar
with session sections and status rings; embedded xterm.js terminals attach to
the same sessions the TUI manages.

GUI features: fuzzy search (`/` or `⌘F`), inline rename (double-click),
new session via the sidebar's `＋ New session` button (`⌘T`) with preset
picker and git-worktree option — the directory prefills from the configured
default directory, falling back to the focused session's project, then home —
rename/details/stash/kill/take-over from a per-session `⋯` menu (also on
right-click of the row), full shell terminals inside the GUI — the
topbar's terminal button picks among the machine's shells (`/etc/shells`
+ `$SHELL`), `⌘⇧T` reopens with the last-used shell, the terminal starts
in the focused session's project (then default directory, then home),
and closing the tab (or `exit`) kills the shell — unlimited split panes
in a balanced grid (`⌘D`
splits, `⌘⇧D` closes the focused pane, zoom `⌘⇧↩` or double-click the
pane title, `⌘⌥←/→` cycles focus), teams browser (members with
live-activity dots and model chips, tasks, agent inboxes, create/delete
via the + button or right-click menu; in the details panel click the
description to edit it, `＋ Add member` to add an agent, and right-click
a member to change its model or remove it), `⌘K` clears the active
terminal,
and quit-stash on close. Closing a Claude tab (the `×`, `⌘W`, or
middle-click) stashes its session — process stopped, conversation kept
resumable — so closing a tab and stashing from the sidebar are the same
linked action whichever way you trigger it; use Detach in the tab's
right-click menu to leave it running in the background instead. On the
next launch the tabs you had open reappear as stashed and resume
(`claude --resume`) the moment you click one. Tabs and panes
follow one rule: the active tab is always the content of the focused
pane — clicking a tab fills the focused pane, focusing a pane activates
its tab, and closing a pane keeps its session reachable as a tab. A
labeled `TUI` badge-button in the sidebar header launches the clash TUI
alongside the GUI — gold when a TUI is running somewhere, grey when not.
Clicking it opens a picker of terminals detected on the OS (Terminal,
iTerm2, WezTerm, kitty, Alacritty, Ghostty, Warp; GNOME Terminal/Konsole/xterm
on Linux; tmux when inside one) plus an Auto entry (split pane when the
GUI was started from a pane-capable terminal, else the default
terminal); the last choice is marked in the menu. The sidebar footer holds a collapsible SETTINGS section
(click the header to expand; the choice persists): default directory
for new sessions, terminal font size and font family (live-applied,
with autocomplete of the monospace fonts installed on the machine),
scrollback lines, cursor style (block/bar/underline) and blink,
copy-on-select, "⌥ sends Esc (Meta)" (off = Option always composes
characters — international layouts), whether terminal links open in the
embedded browser or the system browser, a desktop-notifications toggle,
and an `⟳ Update clash` self-update button — when the update lands, a
modal offers Restart / Cancel (restarting closes running sessions).
Settings persist in `gui-state.json`. The sidebar and details panel are
drag-resizable (widths persist).

Sessions carry the same status vocabulary as the TUI — animated
PROMPTING / THINKING / RUNNING / WAITING / STARTING / STASHED / ERRORED
labels in the sidebar and a colored status dot per tab. External claude
processes (started outside clash) are segregated in their own
`⚡ EXTERNAL` section at the bottom of the sidebar with distinct styling;
clicking one (or its ⚡ button) takes it over after a confirm — the
outside process is killed and its conversation (dynamically associated,
always the latest in that directory) opens attached under clash.
Right-click a tab for the context menu:
rename, close (stash), detach (keep running), stash, kill, details. Every tab — Claude
session, shell terminal, browser, or view — renames via double-click on
its label or the context menu; Claude renames go through the registry
(propagating to the TUI and sidebar), the others are display-only.
`Shift+Enter` inserts a newline in Claude session terminals instead of
submitting (plain `Enter` still submits; shells are untouched).
`⌘C` copies the terminal selection and `⌘V` pastes (use `Ctrl+Shift+C`/
`Ctrl+Shift+V` on Linux); plain `Ctrl+C` still sends an interrupt to the
running program.
The tab strip ends in a `+` ghost tab (same menu as the topbar button):
a terminal per detected shell, a browser tab, or a new Claude session.

The details panel (ⓘ) is a compact overview — live status, branch,
project, CWD, summary. Conversation, Subagents, and Diff open as full
tabs in the main area (closable like terminal tabs); the panel's TOOLS
row has Ports, Open-in-IDE, and Open-in-browser pickers — the latter
opens the diff on GitHub (the PR's files view, or a compare view of the
session branch against the default branch), the session's PR, or the
repository. (The local diff opens as an in-app tab, not in the browser.)

Browser tabs are first-class tabs (`⌘⇧B` opens a blank one with the
address bar focused, also via the `+` new-tab menu): each lives in the
tab strip and panes exactly like a terminal or Claude session — split it
next to a terminal, move it between panes, zoom it, own it per
workspace. Each browser pane has full chrome: back/forward,
reload-or-stop (live loading state), an address bar that takes URLs or
search terms (DuckDuckGo), copy-URL, and open-in-system-browser. While a
browser pane is focused: `⌘L` focuses the address bar, `⌘R` reloads,
`⌘+`/`⌘-`/`⌘0` zoom (also in the tab's right-click menu, next to Open
DevTools). Close with `⌘W`, middle-click, or the tab `×`.
Links inside a browser page that target a new window (`target="_blank"`,
`window.open`) open in a new clash browser tab rather than replacing the
current one. Anything "opened in the browser" opens in a new split pane
beside the current session rather than taking over the focused pane (the
session stays visible side-by-side; if the focused pane is empty it is
used as-is): URLs printed in any terminal are clickable; listening ports
open `localhost:<port>`;
and when a session's output mentions a GitHub pull request, a green
`⇄ PR #n` chip appears on the session (and in the tab's right-click
menu) that opens the PR in-app. Browser tabs persist across restarts
(URL and custom name; the page reloads). Notes: the page itself is a
native overlay — click the chrome strip or the tab to focus a browser
pane, and context menus opened over the page area may be hidden.

Workspaces (cmux-style): each workspace owns its pane layout AND its
sessions — `⌘N` new, `⌘1-9` switch, `⌘⇧R` rename, `⌘⇧W` or the chip's
`×` to close, `⌘B` toggles the sidebar. The sidebar and the tab strip
are scoped to the active workspace: its sessions in status sections,
plus an UNASSIGNED group for sessions no workspace has claimed (opening
one claims it). Tabs owned by another workspace stay hidden until you
switch back; unassigned tabs are always visible.
Searching (`/`) is global across workspaces — results from other
workspaces carry a `⌘n` badge and open in their owning workspace.
Closing a workspace returns its sessions to the unassigned pool.
Right-click a workspace chip for its context menu: rename, close, and
mass-kill all of that workspace's sessions (one confirmation). Every
section header carries a `✕` button that mass-kills the whole group in
one confirmation: the status sections (ACTIVE, FAILED, STASHED, DONE),
UNASSIGNED (sessions no workspace has claimed), and `⚡ EXTERNAL` (all
associated wild claude processes — each row's dynamically-associated PID
is signalled).
Layouts and session ownership are saved to disk (`gui-state.json` in the
clash app-support dir) and survive restarts (running sessions re-attach
automatically).

Notifications: desktop alerts when a session starts waiting for input or
errors (suppressed while the window is focused), unread badges in the
sidebar, plus in-band `OSC 9` / `OSC 777` terminal notification sequences —
`printf '\e]777;notify;Title;Body\a'` from inside any session raises an
alert, so agents and scripts can ping you.

```bash
cargo build --release           # builds BOTH binaries: clash and clash-gui
./target/release/clash-gui      # run — can run alongside the TUI
                                # (each instance owns its own sessions)
```

Release tarballs ship both binaries, and updating from either side
(`clash update`, `:update`, or the GUI's `⟳ Update clash` button)
installs/updates both. Existing installs are replaced through their
symlinks — on macOS the binary inside `Clash.app` is the one updated, the
bundle's `Info.plist` version is bumped, and the bundle is re-signed, so
Finder/Dock launches pick up the new version too.
On Linux, building requires the Tauri system deps (webkit2gtk):
`libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libxdo-dev`.

The GUI is fully self-contained: no external daemon, no node build step
(frontend assets in `gui/dist/` are vendored and embedded in the binary).

## License

MIT
