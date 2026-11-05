# The desktop app (GUI)

The GUI is the primary way to use clash. It is a Tauri 2 desktop app (`gui/`)
sharing the TUI's core: the session pipeline, the in-process PTY daemon and the
protocol. Both frontends can run side by side, each instance owning its own
sessions.

Sessions, teams, scratches, queued follow-ups and presets exist in both
frontends. Workspaces, split panes, the embedded browser, the Files panel, the
attention inbox and [Workflows](workflows-guide.md) are GUI-only.

The first launch opens a **guided tour**, a spotlight walkthrough of
workspaces, starting a session, sessions, the inbox, workflows, scratches,
teams, files, tabs and panes, the terminal and settings. Replay it from
*Settings → General → ▶ Show the tour*. It follows the layout: its text describes
what that layout shows (workspace chips or rail buttons, sections or one view
at a time, docked panels or drawers), a step about a view opens that view
first, and the sidebar is left as the tour found it.

The first workflow item you open gets a second, shorter tour of the item tab
(its header, the stage stepper, its documents, the next-step card, the last
review, the content and its settings), in whichever item view it is showing.
Replay it from the **?** in any item's header or *Settings → General → ▶ Show
the workflow item tour*.

- [Layouts](#layouts)
- [Workspaces, panes and tabs](#workspaces-panes-and-tabs)
- [Sessions](#sessions)
- [Terminals](#terminals)
- [Browser and links](#browser-and-links)
- [Files](#files)
- [Scratches](#scratches)
- [Teams](#teams)
- [Attention inbox](#attention-inbox)
- [Queued follow-ups](#queued-follow-ups)
- [Settings](#settings)
- [Updates](#updates)
- [Keyboard shortcuts](#keyboard-shortcuts)

## Layouts

How the sidebar is arranged is a setting (*Settings → Appearance → Layout*, also
in the sidebar's quick settings). The features are the same in every layout;
only their arrangement changes, live, without a restart:

| Layout | Arrangement |
|---|---|
| **Classic** (default) | One sidebar: workspace chips, sessions, then collapsible Workflows, Scratches and Teams, each with its own height. |
| **Rail** | An icon rail on the left holds the workspaces (their initials; right-click for the workspace menu), the views (Sessions, Workflows, Scratches, Teams, with badges), the inbox, an update button when a release is found, and Settings. The panel beside it shows one view at a time, full height. |
| **Flyout** | The rail alone stays on screen, so the terminals get the window's whole width. Opening a view slides its panel over the terminals; Esc, a click outside, or picking a session or an item puts it away. |

The right-hand panels — **Files** (`⌘E`) and the **Inspector** (`ⓘ`) — follow
the same logic. Classic docks them side by side and lets both stay open; Rail
docks **one at a time**, with a *Files | Inspector* switch in the panel's header;
Flyout opens either as a **drawer** over the terminals, closing on Esc or a
press outside. The top-right actions are grouped the same way in every layout:
new tab · stash all · panes · panels (lit while open).

The **Inspector** shows the focused session (or a team) as cards: status, agent,
branch and Wild badge; quick actions (conversation, subagents, diff, open in IDE,
browser, listening ports); the summary; where it runs, with the worktree and
directory copied by a click; activity; and the session ID, also copyable.

`⌘B` hides or brings back the sidebar; in Flyout it slides the panel in and out.
A second click on the open view's rail icon does the same. Switching back to
Classic restores the sections you had open there. The tour follows the layout:
a step about a view opens that view first.

How it works: the app speaks in layout-neutral actions (show a view, toggle a
section, fold the sidebar, repaint the rail), and each layout implements them.
The catalogue of views and layouts and every decision about what shows live
in the pure `gui/dist/layout.js` (tested by `gui/tests/layout.test.js`); the
adapter in `app.js` (*Sidebar layout adapter*) applies them to the DOM. Adding
a layout is one entry in `LAYOUTS` plus its CSS; adding a view is one entry in
`SIDEBAR_VIEWS`. The setting is `appearance.layout` (GUI-local, in
`gui-state.json`).

## Workspaces, panes and tabs

### Workspaces

Workspaces (cmux-style) each own a pane layout *and* their sessions.

- `⌘N` creates one, `⌘1–9` switches, and `⌘⇧R` or a double-click on the chip
  renames it.
- `⌘⇧W` or the chip's `×` closes it; the last one can't be closed. Closing a
  workspace **kills its sessions** (after one confirm) and closes its other
  tabs — nothing is moved to another workspace. Sessions started outside clash
  are left running.
- The sidebar is scoped to the active workspace, plus an **UNASSIGNED** group
  for sessions no workspace has claimed (opening one claims it).
- Every tab — agent session, shell, browser, workflow item, inbox, board —
  belongs to exactly one workspace: the one it was opened in. Other workspaces
  never show it in their strip or panes; opening it again from elsewhere
  switches to its workspace.
- Search (`/`, `⌘F`) is global; results from other workspaces carry a `⌘n`
  badge.
- **Move to new workspace** / **Move to workspace…** in the menu of a tab, a
  pane title or a sidebar session moves that session, terminal, browser or view
  tab to a fresh or existing workspace, and switches there. A session's own
  conversation/diff tabs go with it, and a workflow item's tab takes every agent
  session of that item (any round, running or stashed).
- A workflow agent joins the workspace its launch was clicked in, even if you
  switched away while it set up — it opens there on your next visit. Agents
  nobody clicked for (check runs, autopilot, auto-applied rounds) join the
  workspace holding the item's tab or its earlier agents.

### Split panes

Panes nest freely, like iTerm.

- `⌘D` splits the focused pane (beside it when it is wide, below it when it is
  tall) and `⌘⇧D` closes it.
- Drag a tab — from the strip or a pane's title bar — onto any pane: like
  iTerm, the half under the cursor (picked by the pane's diagonals) is where the
  split opens, and the highlight shows it. Dragging a pane's title moves the
  pane; its neighbours take the space it leaves. Dropping on an empty pane fills
  it, and on a pane's title bar swaps the two.
- Drop it on the outer edge of the whole pane area instead to get a pane that
  spans that side — e.g. one full-width pane under several columns.
- Drop a pane's title on the tab strip to unsplit it back into a plain tab, at
  the spot the accent bar marks. `Esc` cancels a drag.
- Closing a tab leaves its pane in place, empty, ready for the next tab.
- `⌘⇧↩` (or a double-click on the pane title) zooms.
- `⌘⌥←/→` cycles focus.
- Drag a divider to resize; the layout persists per workspace.

### Tabs

- The active tab is always the content of the focused pane. The `+` ghost tab
  opens a shell terminal, a browser tab or a new session; right-click an empty
  pane for the same menu.
- Drag a tab along the strip to reorder it; the order survives a relaunch.
- Double-click a label to rename; middle-click or `⌘W` closes.
- Closing an agent tab **stashes** the session (stopped, conversation kept
  resumable); choose *Detach* in the tab menu to leave it running instead.

On relaunch clash restores **where you were**: workspace, tabs, layout, focused
pane and browser tabs. Stashed sessions resume the moment you click them.

The sidebar and details panel are drag-resizable. The WORKFLOWS / SCRATCHES /
TEAMS sections have a draggable divider and their own scrollbars.

## Sessions

How clash finds sessions, detects their status and handles processes started
elsewhere is described in [sessions.md](sessions.md).

- **Status**: the sidebar shows status sections with animated status labels
  (PROMPTING / THINKING / RUNNING / WAITING / STARTING / STASHED / ERRORED /
  DONE), and each tab gets a colored dot. Every row carries a **CC** / **OMP**
  agent badge.
  - STASHED means *resumable*: sessions are stashed when clash starts, because a
    transcript can't say whether its process still lives.
  - Sessions whose conversation Claude Code has deleted (after ~30 days) stop
    being listed.
- **New session** (`＋ New session`, `⌘T`):
  - pick a directory (prefilled from the default directory or the focused
    project, with a 📁 picker);
  - pick a [preset](sessions.md#session-presets), optionally a git worktree, and
    the agent (Claude Code or OMP; one whose binary doesn't resolve is greyed
    out).
- **Session actions**: each row's `⋯` menu (or right-click) offers rename,
  ⟳ reload, details, stash, kill, open PR, queue or cancel a follow-up, and take
  over (for wild rows).
- **External claudes** started outside clash are listed under `⚡ EXTERNAL`.
  Clicking one (or its ⚡ button) takes it over after a confirm: the outside
  process is killed and its conversation opens under clash. See
  [Wild sessions](sessions.md#wild-sessions).
- **Section buttons**: every section header has `✕` (kill the whole group, one
  confirm). The topbar's **⏸ all** stashes every running session.
- **⟳ Reload** hot-restarts a session on the newest `claude` binary, resuming
  its latest conversation. It is on rows, tabs and section headers, and `⌘R`
  reloads the focused session. Busy sessions are skipped by group reloads, and
  an individual reload asks first. Wild rows get no reload (take-over is their
  action).
- **Details panel** (ⓘ): status, branch, project, CWD and summary, plus
  *Ports*, *Open in IDE* and *Open in browser* (the PR, the diff on GitHub, or
  the repo). Conversation, Subagents and Diff open as full tabs.
- A session whose output mentions a GitHub PR gets a green `⇄ PR #n` chip.
- The **TUI** badge in the sidebar header launches the clash TUI in a detected
  terminal (Terminal, iTerm2, WezTerm, kitty, Alacritty, Ghostty, Warp, GNOME
  Terminal, Konsole, xterm, tmux). It turns gold while a TUI is running.

## Terminals

- Terminals are xterm.js with GPU (WebGL) rendering; a lost GL context is
  reacquired automatically.
- **Shell terminals**: the topbar button picks among the machine's shells, and
  `⌘⇧T` reopens the last one. Closing the tab kills the shell.
- **Keyboard**:
  - `Shift+Enter` inserts a newline in agent sessions.
  - `⌘C` / `⌘V` copy and paste (`Ctrl+Shift+C/V` on Linux); plain `Ctrl+C`
    interrupts.
  - `⌘K` clears the terminal.
- **Selecting text**: Claude Code captures the mouse, so **⌥-drag** (Shift on
  Linux) to select text. Right-click selects a word.
- **Dropping files**: drop a file or a screenshot onto a terminal and its path
  is typed into the session, escaped like iTerm2 does — Claude Code and OMP
  attach a dropped image. The window has native file drops off (in-app
  drag-and-drop needs it), so the page gets the file's bytes without its path:
  clash stores a copy under `~/.claude/clash/drops/` (kept 7 days, 64 MB max)
  and types that path instead.
- **International layouts**: ⌥ composes characters (`{`, `[` on AZERTY).
  *⌥ sends Esc (Meta)* keeps ⌥+letter as Meta.
- **Notifications**: a desktop alert fires when a session starts waiting or
  errors (not while the window is focused). Any process can raise one with
  `printf '\e]777;notify;Title;Body\a'` (OSC 9 / 777).

## Browser and links

- Browser tabs are first-class tabs. `⌘⇧B` opens a blank one in its own split.
- Full chrome: back/forward, reload/stop, an address bar taking URLs or
  searches, copy URL, open in the system browser and DevTools.
- While focused: `⌘L` focuses the address bar, `⌘R` reloads, and `⌘+` / `⌘-` /
  `⌘0` zoom.
- `target="_blank"` links open a new clash tab.

**How links open** is one setting, *ask* (default), *in clash* or *system
browser*. It applies to every link clash opens: terminal URLs, PR buttons and
chips, listening ports, and links in rendered plans and reviews. A link opened
in clash goes to a new split beside the current session. Non-web links
(`mailto:`, `tel:`, `file:`) always go to the system.

## Files

The **Files** panel (`⌘E`, or the folder button in the topbar) is a file
explorer docked to the right of the panes.

- **Root**: it shows the focused session's folder and follows you as you switch
  sessions. 📌 pins the current folder; the folder button shows any other one
  (pinned). Click the folder name to reveal it in Finder.
- **Tree**: folders load as you expand them and remember what was open per
  folder. Git-ignored files are hidden (◌ shows them, dimmed), `.git` never
  appears, and the panel refreshes every few seconds while it is open.
- **Git status**: the header shows the branch and ahead/behind counts. Files are
  marked **M**odified, **A**dded, **D**eleted, **R**enamed, **U**ntracked or
  **!** (conflict), and a collapsed folder holding changes shows `•`.
- **Find** (`⌘P`, or type in the filter box): fuzzy search over every file git
  knows about (tracked and untracked, not ignored); outside a repository it
  walks the folder, skipping dot-folders and `node_modules`/`target`-style
  trees. Matched letters are highlighted.
- **Keyboard**: `↑`/`↓` move, `→`/`←` open and close a folder, `↵` previews,
  `⌥↵` mentions the file in the session, `Esc` clears the search.
- **Preview**: clicking a file opens it in a preview pane beside the session.
  Code is syntax-highlighted with line numbers, Markdown is rendered (mermaid
  included) with a *Source* toggle, and images are shown. Binary files and text
  over 2 MB are not previewed.
  - Clicking another file replaces the preview (its tab is in *italics*).
    Double-click, `↵` or 📌 *Keep* keeps a tab open.
- **Actions** (right-click a row, or the preview's toolbar): *Mention in
  session* types `@path` into the session (relative to its folder), *Open in
  editor…* (the scratch editor picker), copy the absolute or relative path or
  the name, reveal in Finder, and for folders *New terminal here* and *Show as
  root*. Dragging a row into a terminal types its path.

## Scratches

Scratches are an IntelliJ-style **"Scratches and Consoles"** tree of free-form
notes and folders, shared with the TUI (its keys are in
[tui.md](tui.md#scratches)). They are plain files and folders under
`~/.claude/clash/scratch/` by default; *Settings → Paths → Scratch directory*
writes `scratch_dir` in `config.toml`, so the TUI honours it too.

- The collapsible **SCRATCHES** sidebar section renders the tree: folders sort
  first; click a folder to expand/collapse it, click a note to open it.
- The section's **+** button (or a folder's right-click menu) creates notes and
  folders.
- **Drag and drop** any note or folder onto another folder — or onto empty space
  to move it back to the root — to reorganize.
- Right-click any entry to copy its path (absolute path, path relative to the
  scratch root, or file name — handy for pasting into a Claude session), rename,
  or delete it.
- Notes open through the editor picker: installed IDEs (Cursor, VS Code, Zed,
  JetBrains, …) launch alongside, terminal editors (vim, nvim, emacs, nano,
  helix, micro) open in a tab.
- The tree **auto-refreshes** when the scratch directory changes on disk (a note
  saved from an editor, the TUI, a `git pull`…) via a filesystem watcher; the
  section's **⟳** button forces a manual re-list.

## Teams

The TEAMS section lists real, user-managed Claude Code agent teams, each with a
live `n/m` running count; Claude Code's per-session `session-<id>` teams are
hidden.

- A team's panel lists its members with run indicators and model chips.
  Left-click a running member to jump to its session, or a stopped one to open
  its inbox.
- Right-click a member to edit its model, agent type, prompt or name, remove it,
  or open its inbox. **＋ Add member** adds one.
- Tasks can be created, have their status cycled (click the badge), get an
  owner, or be deleted.
- The name and description are click-to-edit, and the panel live-refreshes.

## Attention inbox

The **inbox** (sidebar tray icon, `⌘I`) answers "what needs me?" as one list
across every workspace and project, ordered by urgency, then by how long each
has waited:

1. sessions holding a tool-approval prompt;
2. workflow agents that died mid-round;
3. errored sessions;
4. workflow items parked on a decision;
5. PRs with open review threads;
6. sessions waiting for your next message.

The count turns red only when something is blocked. Clicking a row jumps to it.

## Queued follow-ups

**Queue follow-up…** (a running session's `⋯` menu, the inbox, or `f` in the
TUI) delivers a multi-line prompt to that session the moment it is idle at its
input prompt. `⧖n` on the row shows pending prompts; click it (or `F` in the
TUI) to review or cancel one.

It is safe to leave running:

- delivery waits for two consecutive idle samples and never goes to a
  tool-approval question;
- the text arrives as one bracketed paste plus one Enter;
- each session holds at most 20 queued prompts;
- the queue is in memory for that clash instance;
- every delivery is announced by a toast.

## Settings

Every setting lives in the **Settings** window: `⌘,`, or **⚙ All settings…** in
the sidebar's SETTINGS panel. A search box at the top searches every tab (type
"cursor" or "font"), and every terminal setting applies live.

| Tab | Settings |
|---|---|
| **General** | how links open · refresh interval · default shell · terminal for the TUI launcher · default agent (Claude Code / OMP) · desktop notifications · attention count in the window title · confirm before killing a session · check for updates in the background · ▶ Show the tour |
| **Appearance** | theme (12 palettes, below) · layout (Classic / Rail / Flyout, [above](#layouts)) · workflow item view (Workbench / Tabs, see [Reading an item](workflows-guide.md#reading-an-item)) · terminal text: font family (searchable picker), size, weight, bold weight, line height, letter spacing · cursor: style, unfocused style, bar width, blink · colors: minimum contrast ratio, bold text in bright colors |
| **Terminal** | scrollback · scroll speed · smooth scroll · copy on select · right-click selects word · ⌥ sends Esc (Meta) · toast on terminal bell |
| **Paths** | default session directory · scratch directory · workflows directory · `claude` binary · OMP binary |
| **Workflows** | workflow agent (ask / Claude Code / OMP) · next-step assist (suggest / autopilot / off) · explanation detail · delegation (team / solo) · PR skill · forge · PR stacking · skill updates |
| **Models** | Claude Code: lead model, subagent model, review sizing, small review lead, large review subagents · OMP: lead model, subagent model |
| **Trackers & sharing** | primary tracker · Jira (site, key prefixes, who posts: agent / clash, skill, email, API token) · Linear (workspace, key prefixes, who posts, skill, API key) · who posts to Slack/Discord (agent / clash), chat skill, webhooks · decision notifications |

The sidebar's **SETTINGS** panel keeps only the quick settings — theme, layout,
font size, how links open, default agent — plus **⚙ All settings…** and the
**⟳ Update clash** button.

Theme and terminal settings are the GUI's own (`gui-state.json`), because the
TUI can't apply them. Everything else lives in the shared
[`config.toml`](config-file.md), so the TUI agrees.

**Themes** recolor the chrome and the terminals together:

| Dark | Light |
|---|---|
| clash dark *(default)* · Tokyo Night · Catppuccin Mocha · Nord · Dracula · One Dark · Gruvbox Dark · Solarized Dark | clash light · Catppuccin Latte · Solarized Light · GitHub Light |

The **font picker** lists the installed families, each previewed in its own
face and tagged *mono* or *proportional*. *Monospace only* is on by default, and
*Custom…* takes a full CSS font stack.

## Updates

- **Background check**: the GUI looks for a new clash release at startup and
  every 6 hours (`general.update_check`, on by default; *Settings → General →
  Check for updates in the background*). A release found is announced with a
  toast, and the version label in the sidebar footer becomes a clickable update
  button. Nothing installs until you click.
- **⟳ Update clash** (in the sidebar's SETTINGS panel), like `clash update` and
  the TUI's `:update`, updates **both** binaries. Progress shows in the footer
  version label.
- A Restart dialog stashes every session, relaunches the new binary, and logs
  the relaunch in `clash.log`.
- Symlinked installs are followed. On macOS the binary inside `Clash.app` is
  replaced, its version bumped and the bundle re-signed.

## Keyboard shortcuts

| Key | Action |
|---|---|
| `⌘T` / `⌘⇧T` | new session / new shell terminal (last-used shell) |
| `⌘N` · `⌘1–9` · `⌘⇧R` · `⌘⇧W` | new / switch / rename / close workspace |
| `⌘D` / `⌘⇧D` | split / close the focused pane |
| `⌘⇧↩` · `⌘⌥←/→` | zoom pane · cycle pane focus |
| `⌘W` | close the active tab (agent: stash, shell: kill) |
| `⌘B` | toggle the sidebar (Flyout: slide its panel in and out) |
| `⌘E` / `⌘P` | toggle the Files panel / find a file |
| `⌘⇧B` | new browser tab |
| `⌘F` or `/` | search |
| `⌘I` | inbox |
| `⌘,` | settings |
| `⌘K` | clear the terminal |
| `⌘R` | reload the focused session (in a browser pane: reload the page) |
| `⌘L` · `⌘+`/`⌘-`/`⌘0` | browser: address bar · zoom |
| `Esc` | close the new-session dialog / clear the search |

Text fields support `⌘A` / `⌘C` / `⌘X` / `⌘V` and forward delete (fn+⌫; ⌥ =
word, ⌘ = to end of line). Dialogs take Enter / Esc, and composers send with
`⌘↵`.
