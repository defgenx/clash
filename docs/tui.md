# The terminal UI (TUI)

The TUI (`clash`) is the terminal-native mode: keyboard-driven, runnable over
SSH, and backed by the same core and in-process PTY daemon as the
[desktop app](gui.md). It can run side by side with the GUI and with other
clash instances, each owning its own sessions. Workflows, workspaces, the
browser, the Files panel and the attention inbox are GUI-only.

```bash
clash                              # start the TUI (reads from ~/.claude)
clash --data-dir ~/.claude         # custom data directory
clash --claude-bin /path/to/claude # custom CLI path
clash --debug                      # debug logging (the header shows DEBUG)
```

The first launch shows a guided tour; replay it anytime with `:tour`. `?` opens
context help.

- [Navigation](#navigation) · [Modes](#modes) · [Sessions](#sessions) ·
  [Attached mode](#attached-mode) · [Session detail](#session-detail) ·
  [Subagents](#subagents) · [Diff view](#diff-view)
- [Teams](#teams) · [Team detail](#team-detail) · [Tasks](#tasks)
- [Scratches](#scratches) · [Commands](#commands)
- [Opening sessions externally](#opening-sessions-externally) ·
  [Updates](#updates)

## Navigation

| Key | Action |
|-----|--------|
| `j` / `k` | Select next / previous |
| `g` / `G` | Jump to first / last |
| `Enter` | Drill in |
| `Esc` | Go back |
| `r` | Refresh |
| `q` / `Ctrl+C` | Quit (with confirmation; `Ctrl+C` again while shutting down force-quits) |

In dialogs: `y` / `n` (or `Esc`) answer a confirmation; pickers take `j`/`k`,
`Enter`, `Esc`; text fields support the usual readline keys (`Ctrl+A/E/U/K/W`,
`Alt+B/F/D`). The tour advances with `Enter`/`Space`/`→` and is skipped with
`Esc`/`q`; the help overlay scrolls with `j`/`k` and closes with `?`/`Esc`/`q`.

## Modes

| Key | Description |
|-----|-------------|
| `:` | Command mode — `:teams`, `:sessions`, `:tour`, `:update`, `:quit` (see [Commands](#commands)) |
| `/` | Fuzzy filter |
| `?` | Context help |

## Sessions

Status icons and the `⊞` / 🌿 source prefixes are explained in
[sessions.md](sessions.md).

| Key | Action |
|-----|--------|
| `Enter` / `i` | Open the session detail |
| `a` | Attach (inline terminal); on a 🌿 wild row: take over and attach (one confirm) |
| `p` | View git diff |
| `e` | Open project in IDE (auto-detect + picker) |
| `f` | Queue a follow-up prompt — delivered when the session is next idle |
| `F` | Cancel a queued follow-up (picker when several are pending) |
| `o` | Open in new pane / tab / window |
| `O` | Open ALL running sessions (smart layout) |
| `c` / `n` | New session (directory, name, worktree?, then agent: `claude`/`omp`); with [presets](sessions.md#session-presets), a picker first |
| `s` | Stash / unstash session (stop process, keep in registry) |
| `w` | Spawn session in a git worktree |
| `Tab` | Expand / collapse subagents |
| `A` | Cycle section filter (All/Active/Done/Fail/External; Fail is skipped under `:active`) |
| `S` | Stash / unstash ALL sessions (with confirmation) |
| `d` | Drop session |
| `D` | Drop ALL sessions |

Queued follow-ups behave as described in
[gui.md → Queued follow-ups](gui.md#queued-follow-ups).

## Attached mode

A status bar at the bottom shows session name, project and git branch. The PTY
is resized to fit above the bar.

| Key | Action |
|-----|--------|
| `Ctrl+B` | Detach (works across all terminal encodings) |
| Everything else | Forwarded to the session's agent (`claude` or `omp`), mouse scroll included |

## Session detail

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll |
| `s` / `Enter` | Subagents |
| `t` | Linked team (the Teams list when none is linked) |
| `m` | Team members (all agents when no team is linked) |
| `p` | View git diff |
| `a` | Attach |
| `o` | Open in new pane / tab / window |
| `w` | Spawn a session in a git worktree |
| `e` | Open in IDE |
| `d` | Drop |

## Subagents

| Key | Action |
|-----|--------|
| `Enter` | Subagent detail |
| `a` | Attach to the parent session |
| `o` | Open the parent session in a new pane / tab / window |
| `e` | Open in IDE |
| `p` | View git diff |
| `f` / `F` | Queue / cancel a follow-up for the parent session |

## Diff view

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll diff content |
| `n` / `p` | Next / previous file |
| `r` | Refresh diff |
| `Esc` | Go back |

## Teams

| Key | Action |
|-----|--------|
| `Enter` / `i` | Open the team's detail (its members and tasks from there) |
| `c` | Create team |
| `R` | Rename team (moves its config + tasks) |
| `d` | Delete team |
| `e` | Edit team description |
| `m` | Add member (name → agent type → model) |
| `x` | Remove member (picker) |

## Team detail

Opening a team scopes the Agents and Tasks views to that team. Its member list
marks members whose session is currently running with `●` (`○` otherwise); the
Agents view says `active` / `idle`. Per-member edits are commands run from a
team view — `:member model <name> [model]`, `:member type <name> <type>`,
`:member prompt <name> <text>`, `:member rename <old> <new>`.

| Key | Action |
|-----|--------|
| `Enter` / `a` | View agents (team-scoped) |
| `t` | View tasks (team-scoped) |
| `s` | View lead session |
| `e` | Edit team description |
| `m` | Add member (name → agent type → model) |
| `x` | Remove member (picker) |
| `R` | Rename team |
| `d` | Delete team |

## Tasks

The Tasks view is scoped to the current team.

| Key | Action |
|-----|--------|
| `Enter` | View task detail |
| `c` | Create task |
| `s` | Cycle status (pending → in-progress → completed → …) |
| `a` | Assign owner (picker of the team's members) |
| `d` | Delete task |

In a task's detail view, `s` cycles its status and `d` deletes it (with
confirmation).

## Scratches

Reach the Scratches view with `:scratch` (also `:notes`). Scratches are an
IntelliJ-style **"Scratches and Consoles"** tree: notes and folders you can
nest, rename and reorganize. Folders sort first; the tree is shown indented with
an expand/collapse caret. The GUI renders the same tree
([gui.md → Scratches](gui.md#scratches)).

| Key | Action |
|-----|--------|
| `a` / `c` / `n` | New scratch — created inside the selected folder (or alongside the selected note, else at the root) |
| `A` | New folder (same placement rule) |
| `Enter` | Open a file in an editor (picker), or expand/collapse a folder |
| `e` | Open the selected note in an editor (picker) |
| `r` | Rename the selected file or folder |
| `m` | Move the selected file or folder into another folder (picker; choose **/ (root)** to move it back to the top level) |
| `y` | Copy the entry's path to the clipboard (picker: absolute path, path relative to the scratch root, or file name) — IntelliJ-style "Copy Path/Reference…" |
| `d` | Delete the selected entry (folders are removed recursively, with confirmation) |

- Scratches are plain files and folders under `~/.claude/clash/scratch/` by
  default; override the location with `scratch_dir` in `config.toml` or the GUI
  **Scratch directory** setting (which writes the same key).
- The tree **auto-refreshes** when the scratch directory changes on disk (a note
  saved from an editor, the GUI, a `git pull`…) via a filesystem watcher that
  follows the configured directory.
- The editor picker lists installed IDEs (Cursor, VS Code, Zed, JetBrains, …)
  and terminal editors (vim, nvim, emacs, nano, helix, micro); terminal editors
  open in a tab/pane, GUI editors launch alongside.
- `y` uses the platform clipboard tool (`pbcopy`/`wl-copy`/`xclip`/`xsel`/`clip`)
  for local copies and also emits an OSC 52 escape, so it works over SSH and in
  clipboard-capable terminals (iTerm2, kitty, WezTerm, Ghostty, tmux with
  `set-clipboard on`).

## Commands

| Command | Action |
|---------|--------|
| `:teams` | Navigate to Teams view (each view name also works singular: `:team`, `:session`, …) |
| `:sessions` | Navigate to Sessions view |
| `:agents` | Navigate to Agents view |
| `:tasks` | Navigate to Tasks view |
| `:subagents` | Navigate to Subagents view |
| `:inbox` | Show the selected/drilled-in agent's inbox |
| `:prompts` | Navigate to Prompts view |
| `:scratch` / `:notes` | Navigate to Scratches view |
| `:create team <name>` | Create a new team |
| `:rename team <old> <new>` | Rename a team |
| `:delete team <name>` | Delete a team (also `:remove team`) |
| `:member model <member> [model]` | Set a member's model (current team; empty = inherit) |
| `:member type <member> [type]` | Set a member's agent type (empty = general-purpose) |
| `:member prompt <member> <text>` | Set a member's system prompt |
| `:member rename <old> <new>` | Rename a member |
| `:create task <team> <subject>` | Create a task |
| `:new [path]` | Spawn a new session (default agent) |
| `:new --agent <claude\|omp> <path>` | Spawn a new Claude Code or OMP session (the path is required for the agent to stick) |
| `:new --preset <name>` | Spawn session from a preset |
| `:diff` | View git diff for current session |
| `:rename <name>` | Rename session (from detail view) |
| `:active` / `:all` / `:external` (`:wild`) | Filter sessions (active only / all / wild + external only) |
| `:tour` (`:guide`) | Replay guided tour |
| `:config` | Show the `config.toml` path |
| `:reload` (`:reload-config`) | Re-read `config.toml` now (it is watched, so this is only ever a nudge) |
| `:update` (`:upgrade`) | Update clash |
| `:quit` (`:q`) | Exit |

## Opening sessions externally

`o` sends the selected session to a pane (when the terminal supports it) or a
tab/window of your terminal; `O` opens every running session with a planned
layout, after a confirm. Pane-capable terminals (tmux, iTerm2, WezTerm, kitty)
get split panes: panes fill first, horizontally or vertically depending on the
screen size, and overflow goes to tabs. Other terminals get tabs or windows.

Each external pane runs `clash attach <session-id>`, which connects to the
running clash that owns the session (each instance has its own
`daemon-<pid>.sock`). Sessions opened this way show the `⊞` prefix and cannot be
reopened until closed; the indicator clears on the next refresh after the
external attach exits.

## Updates

The TUI checks once at startup for a newer clash release and announces it with
a toast; it honours the same `general.update_check` setting as the GUI.
`:update` (or `clash update`) installs it, updating both binaries.
