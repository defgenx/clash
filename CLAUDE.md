# CLAUDE.md — clash Project Guide

## What is clash?

Terminal UI for managing Claude Code Sessions & Agent Teams. Built in Rust with ratatui.

## Quick Commands

```bash
cargo build --release      # Build
cargo test                 # Run all tests (unit + integration)
cargo run -- --data-dir /path/to/test/data  # Run with test data
cargo run -- --debug       # Run with debug-level logging
```

## Before Pushing

**Always run format and linter before every push:**

```bash
cargo fmt                  # Auto-format code
cargo clippy               # Run linter
```

CI runs `cargo fmt --check` and `cargo clippy` — pushes will fail if either has issues.

## Documentation Rules

Every change **must** keep documentation in sync:

1. **README.md** — Update keybindings, commands, features, or behavior descriptions whenever they change.
2. **Guided tour** (`src/infrastructure/tui/widgets/tour.rs`) — Every new user-facing command or keybinding must be added to the appropriate tour step.
3. **Help overlay** (`src/infrastructure/tui/widgets/help_overlay.rs`) — Every new keybinding or command must appear in the `?` help screen (global, context, or commands section).

If a PR adds a keybinding but doesn't update all three, it is incomplete.

## Architecture: Clean Architecture (strict layers)

```
Infrastructure → Adapters → Application → Domain
(outer)                                   (inner)
```

**Dependencies point inward only.** Inner layers never import from outer layers.

### Layer Map

| Layer | Path | Purpose | May depend on |
|-------|------|---------|---------------|
| **Domain** | `src/domain/` | Entities (`Team`, `Task`, `Member`, `InboxMessage`, `TaskStatus`) and the `DataRepository` port trait | Nothing |
| **Application** | `src/application/` | `AppState`, `Action` enums, `Effect` enum, pure `reducer`, `NavigationStack` | Domain |
| **Adapters** | `src/adapters/` | `input.rs` (KeyEvent→Action), `renderer.rs` (State→Frame), `views/` (TableView/DetailView impls) | Application, Domain |
| **Infrastructure** | `src/infrastructure/` | `app.rs` (event loop), `fs/` (FsBackend, atomic writes, watcher), `tui/` (widgets, theme, layout), `windowing/` (pane/tab/window spawning, standalone attach client), `config.rs`, `error.rs`, `event.rs` | All layers |

### Key Files

- `src/domain/entities.rs` — All domain types with lenient serde (`#[serde(default)]` + `#[serde(flatten)]`)
- `src/domain/ports.rs` — `DataRepository` trait definition
- `src/application/reducer.rs` — **Pure function** `fn(state, action) → (state, effects)`. No IO. All business logic lives here.
- `src/application/effects.rs` — Abstract effects (`PersistTask`, `CreateTeam`, `RemoveTeam`). No file paths — infrastructure translates these to real IO.
- `src/application/actions/mod.rs` — Nested action enums: `Nav`, `Table`, `Team`, `Task`, `Agent`, `Ui`
- `src/infrastructure/app.rs` — Event loop coordinator. Executes effects by calling the `DataRepository` impl. Session refresh is a thin call to `session_refresh`.
- `src/infrastructure/session_refresh.rs` — Pure session refresh pipeline: gathers input (disk, hooks, daemon), builds a complete sorted session list via `build_session_list()` (no IO), then swaps atomically into `DataStore`.
- `src/infrastructure/fs/backend.rs` — `FsBackend` implements `DataRepository`
- `src/infrastructure/fs/atomic.rs` — `write_atomic()`: write to temp file, then rename (prevents partial reads)
- `src/infrastructure/windowing/terminal_spawn.rs` — Terminal detection, smart pane/tab/window spawning with layout planning
- `src/infrastructure/windowing/attach.rs` — Shared attach loop (`attach_loop`) and standalone `clash attach <id>` client. Uses `/dev/tty` for input and `nix` termios for raw mode (avoids crossterm's reader thread)

## Core Pattern: TEA (The Elm Architecture)

```
User Input → Event → handle_key() → Action
                                      ↓
                            reducer::reduce()  ← pure, no IO
                                      ↓
                            (State', Vec<Effect>)
                                      ↓
                            execute_effects()  ← infrastructure IO
                                      ↓
                            renderer::draw()   ← reads state, draws frame
```

The reducer **never** touches the filesystem, network, or terminal. It returns `Effect` values that the infrastructure layer executes.

## Data Model

clash reads from Claude Code's filesystem:

```
~/.claude/
├── teams/{name}/config.json          # Team config + members[]
│   └── inboxes/{agent}.json          # InboxMessage[]
├── tasks/{team-name}/{id}.json       # Task with status, owner, dependencies
└── clash/scratch/{…tree}            # Scratches: a nested tree of free-form text files and folders (dir configurable)
```

All types use `#[serde(default)]` so missing fields get zero values, and `#[serde(flatten)]` captures unknown fields for forward compatibility.

Scratches are an exception: the file on disk *is* the note (no structured JSON), so `ScratchNote` is a runtime view DTO built from the filesystem listing. They form an IntelliJ-style **tree** of files and folders: `load_scratch_notes` walks the scratch dir recursively and returns a depth-first **pre-order** flattening (folders first, alphabetical) where each node carries `id` (POSIX path relative to the scratch root — the stable identifier), `parent`, `depth`, and `is_dir`. `DataRepository` exposes `load_scratch_notes` plus the tree ops `create_scratch_note(parent, title)`, `create_scratch_dir(parent, name)`, `rename_scratch(id, new_name)`, `move_scratch(id, new_parent)` (cycle-guarded), and `delete_scratch_note(id)` (recursive for folders). Path safety is enforced by `sanitize_component`/`sanitize_rel_path` (reject separators/`..`, so nothing escapes the root). The pure `visible_scratch_indices(notes, expanded)` (in `application::state`) computes which rows are visible given the expanded-folder set, and is shared by the reducer (selection clamping) and both renderers. The TUI renders the tree via the custom `render_scratch_table` (not the generic table path) and supports create/rename/move/delete + expand-collapse. Moving goes through the core like every other op: `m` opens a folder picker (`UiAction::EnterMoveScratchMode` builds the candidate list via `scratch_move_targets` — root + every folder minus the entry, its descendants, and its current parent), the pick dispatches `ScratchAction::Move { id, new_parent }` → `Effect::MoveScratch` → `move_scratch`. The GUI instead reorganizes by drag-and-drop, calling the Tauri scratch command directly (bypassing the reducer/effects) — so the GUI path does not emit `ScratchAction::Move`, but the variant and effect now exist for the TUI. `y` copies an entry's path (IntelliJ-style "Copy Path/Reference…"): `UiAction::EnterCopyScratchPathMode` opens a picker (absolute `note.path` / relative `note.id` / file name), and the pick emits `Effect::CopyToClipboard { text }`, executed by `infrastructure::clipboard::copy` — best-effort platform command (`pbcopy`/`wl-copy`/`xclip`/`xsel`/`clip`) plus an OSC 52 escape (so it works over SSH and in clipboard-capable terminals; `base64` is already a dep, no new crate). The GUI mirrors the same three formats in the scratch-tree right-click menu (`noteContextMenu` → `copyScratchPath`), but writes directly via the `clipboard_write_text` Tauri command (like its other scratch ops) and confirms with a `flashToast` — it does not go through the reducer/effects. Both frontends open a scratch via the IDE/editor picker (`ide::detect_editors`): terminal editors run in a pane/tab, GUI editors launch alongside — so the backend never reads/writes note *contents*, only lists/creates/renames/moves/deletes entries. The scratch directory defaults to `<claude_dir>/clash/scratch` but is overridable via `config.toml`'s `scratch_dir` (resolved by `Config::scratch_dir`, held on `FsBackend` with interior mutability so the GUI Settings panel can change it live via `set_scratch_dir` and persist with `Config::save`).

## Conventions

- **No IO in the reducer.** Effects are the only way the reducer communicates with the outside world.
- **Atomic writes everywhere.** Use `write_atomic()` for any file mutation to prevent corruption during concurrent access.
- **Lenient serde on all domain types.** Never fail on unknown/missing fields — use defaults and capture extras.
- **Each view is a trait impl.** `TableView` for list screens, `DetailView` for detail screens. Generic widgets in `tui/widgets/` render them.
- **Nested action enums.** `Action::Team(TeamAction::Create { .. })`, not flat variants. Each domain has its own sub-reducer.
- **Tests alongside code.** Unit tests in `#[cfg(test)]` modules within each file. Integration tests in `tests/`.

## Clean Architecture Principles

These rules must be followed on every change:

- **One core, two frontends.** Any feature must be implementable by both the TUI and the GUI (modulo genuine platform limits): business logic and persistence go through the shared core (`DataRepository`, effects, `session_refresh`), never duplicated in `gui/src-tauri/main.rs` or the TUI adapters. If a frontend needs a capability, add it to the port/effect layer first.
- **Infrastructure does not own business logic.** `backend.rs` loads and returns data — it does not sort, filter, or transform for presentation. Sorting, filtering, and display formatting belong in the Application or Adapter layers.
- **Computed display fields use `#[serde(skip)]`.** Fields derived at runtime (e.g., `worktree_project`) that are never in the on-disk JSON must use `#[serde(skip)]`, not `#[serde(default)]`. Follow the `repo_config` precedent.
- **Pure functions for testability.** When logic parses strings, formats output, or makes decisions, extract it into a pure function (no IO) and test it directly. Filesystem-touching wrappers should be thin. Example: `parse_gitdir_content()` (pure) wraps into `detect_worktree()` (reads file).
- **Domain port traits stay minimal.** Never leak infrastructure concerns (filesystem paths, watcher events, cache hints) into `DataRepository`. If the infrastructure layer needs an optimization API (e.g., cache invalidation), put it on the concrete struct (`FsBackend`), not the trait.
- **No dead code.** Do not leave unused functions, imports, or fields. If trait obligations force methods that are never called through the generic path (e.g., `SessionsTable::row()` — needed by `TableView` but bypassed by `render_sessions_table()`), document why with a comment.
- **DRY display helpers.** When the same formatting logic is needed in multiple views, create a single helper function (e.g., `worktree_display_from_cwd()`) rather than repeating the pattern at each call site.
- **Stable session ordering.** Sessions are sorted by section (Active/Pending/Done/Fail) then alphabetically by name inside `session_refresh::build_session_list()`. The backend returns unsorted data; sorting happens in the pure pipeline before atomic swap. Selection is stabilized by ID across refreshes.
- **Shadow-swap session refresh.** The session refresh pipeline builds a complete new session list in a staging `Vec<Session>` without touching `store.sessions`, then swaps atomically. If daemon IPC fails, running daemon-only sessions are preserved from the previous cycle. This prevents flickering.
- **Agent liveness cross-reference.** `DataStore::rebuild_all_members()` checks each agent's `is_active` against running sessions (CWD-based matching). Agents with no matching running session are marked inactive.
- **Delta subagent reloading.** `refresh_changed_subagents()` only reloads subagents for sessions whose status or subagent_count changed since the last refresh, avoiding N disk reads per refresh cycle.
- **Cache transparency.** `FsBackend`'s internal `SessionCache` is invisible to the `DataRepository` trait. Invalidation is driven by the FS watcher in `app.rs` via `invalidate_session_cache()`. On first load, everything is scanned; on subsequent loads, only dirty projects are re-parsed.

## Testing

- **Unit tests** (inline): reducer actions, serde parsing, navigation, input handling, atomic writes, CLI commands
- **Integration tests** (`tests/`): `data_layer_test.rs` (fixtures → DataRepository), `full_cycle_test.rs` (action→state→effect cycles)
- **Test fixtures** (`tests/fixtures/`): 5 teams (valid, empty, malformed, extra fields), 4 tasks, inbox messages
- **TestDataDir helper** (`tests/helpers/test_data_dir.rs`): copies fixtures to a temp dir for isolated tests

## Common Tasks

### Adding a new view
1. Create `src/adapters/views/my_view.rs` — impl `TableView` or `DetailView`
2. Add variant to `ViewKind` enum in `src/adapters/views/mod.rs`
3. Add navigation action in `src/application/actions/navigation.rs`
4. Handle in `renderer.rs` draw dispatch
5. Add keybinding in `input.rs`

### Adding a new action
1. Add variant to the appropriate action enum in `src/application/actions/`
2. Handle in `src/application/reducer.rs` — return appropriate `Effect`s
3. Map keyboard input in `src/adapters/input.rs`

### Adding a new effect
1. Add variant to `Effect` enum in `src/application/effects.rs`
2. Handle execution in `src/infrastructure/app.rs` `execute_effects()`

### Opening sessions externally (windowing)
- `o` opens the selected session in a pane (if terminal supports it) or tab/window
- `O` (Shift+O) opens ALL running sessions with smart layout (confirm dialog first)
- Layout: panes fill first (horizontal or vertical based on screen size), overflow to tabs
- Sessions opened externally show `⊞` prefix and cannot be reopened until closed
- Cleanup: when the external `clash attach` process exits, the indicator clears on next refresh
- Implementation: `src/infrastructure/windowing/terminal_spawn.rs` (spawn logic), `src/infrastructure/windowing/attach.rs` (attach client)

## Dependencies

Key crates: `ratatui` (TUI), `crossterm` (terminal events, TUI raw mode), `nix` (termios for attach raw mode, PTY), `tokio` (async), `serde`/`serde_json` (data), `notify-debouncer-full` (FS watching), `clap` (CLI args), `color-eyre`/`thiserror` (errors), `lru` (inbox cache), `tui-input` (text input widget), `chrono` (timestamps), `fuzzy-matcher` (filter mode).

## Gotchas

- `main.rs` uses `mod` (private), `lib.rs` uses `pub mod` (public for tests). Both declare the same 4 layers.
- The `notify-debouncer-full` error type lives at `notify_debouncer_full::notify::Error`, not `notify::Error`.
- Raw strings containing `#` in JSON values (like hex colors) need `r##"..."##` delimiters.
- Inboxes are lazy-loaded with LRU(3) eviction — they're only fetched when navigating to the inbox view.
- Agent attach uses suspend-and-resume: save state → restore terminal → spawn `claude --resume` → reclaim terminal on exit.
- External session opening (`o`/`O`) uses the `windowing` module: pane-capable terminals (tmux, iTerm, WezTerm, Kitty) get split panes; others get tabs/windows. Sessions opened externally are tracked in-memory via `externally_opened` and shown with `⊞` prefix.
- `clash attach <id>` is a lightweight subcommand for external panes — it connects to the in-process daemon of a running clash app (TUI or GUI), not a standalone daemon. Multiple instances may run simultaneously, each with its own `daemon-<pid>.sock`; attach probes live sockets and picks the instance that owns the session.
- The attach loop reads from `/dev/tty` (not fd 0) to avoid racing with crossterm's internal reader thread. The standalone client uses `nix::sys::termios` for raw mode instead of crossterm to prevent crossterm's reader from being initialized.
- Ctrl+B detach supports three terminal encodings: raw `0x02`, Kitty CSI u (`ESC[98;5u`), and xterm modifyOtherKeys (`ESC[27;5;98~`). iTerm2 uses the xterm format.
- `--debug` flag enables debug-level logging; the header shows a `DEBUG` indicator when active.
- Tauri 2 frontend APIs are permission-gated: `gui/src-tauri/capabilities/default.json` must grant `core:default` (includes `core:event:allow-listen`) or every `listen()` in the GUI frontend fails as a silent unhandled rejection — no `pty-output` ever reaches xterm and terminals render blank.
- wry's WKWebView does not implement native `alert`/`confirm`/`prompt` (silent no-ops) — use the in-app `uiConfirm`/`uiPrompt`/`uiAlert` dialogs in `gui/dist/app.js`.
- In-page **HTML5 drag-and-drop** (the scratch tree's move-by-drag) only fires if the window sets `"dragDropEnabled": false` in `tauri.conf.json`. Tauri v2 defaults it to `true`, which registers the OS/WKWebView file-drop handler that swallows `dragstart`/`dragover`/`drop` inside the webview — so the drag code runs but no drop ever lands (scratches silently won't move). Safe to disable because clash consumes no OS file-drop events. The scratch DnD itself lives in `buildNoteRow`/`wireNoteDropTarget`/`moveNote` (folders are drop targets; the `#notes-list` container is the root drop target), calling the `move_scratch` Tauri command directly.
- The bare-binary WKWebView's localStorage is not reliably persisted — durable GUI state (workspaces) goes through `save_gui_state`/`load_gui_state` to `gui-state.json` (atomic write) in the clash app-support dir.
- The embedded browser panel is one native child webview per tab (`Window::add_child`, tauri `unstable` feature, labels `embedded-browser-<n>`), stacked over `#browser-slot`; switching tabs hides/shows webviews, and the frontend reports the slot rect to ALL tabs on every layout change (`syncBrowserBounds` in `fitAll`). In-app DOM (dialogs, menus) cannot render above it.
- Child-webview coordinates ≠ DOM coordinates on macOS: the window uses a full-size content view, so in windowed mode WKWebView insets the main webview's page content by the title-bar height (~32px) while the native view spans the full window. Browser-tab webviews positioned with raw `getBoundingClientRect` values land a title-bar height too high and cover the pane's chrome strip (invisible in fullscreen, where the inset is 0). `browser_coord_offset()` measures the main webview's frame + `safeAreaInsets` live and every `browser_open`/`browser_set_bounds` adds it.
- In-app context menus must call `hideBrowserWebviews()` when opening (`showContextMenu` does) and `fitAll()` on close — native browser webviews paint over all DOM, so an unhidden webview covers the menu.
- Never call `Webview::url()` on an embedded browser tab — wry's macOS impl unwraps WKWebView's `URL` property, which is nil until the first navigation commits; the panic hits the tao event loop thread and aborts the app (and replays on every launch via the persisted tab in `gui-state.json`). `browser_get_url` reads the property via `with_webview` + objc2 with a nil check instead.
- Don't feed `data:` URLs to the embedded browser — tauri rejects them at webview creation without the `webview-data-url` cargo feature (not enabled), and WKWebView silently drops multi-MB ones. Serve generated pages via `register_uri_scheme_protocol` instead (the local diff intentionally lives in an in-app tab, not the browser — the browser picker opens the GitHub diff).
- PTY children get `TERM=xterm-256color` + `COLORTERM=truecolor` defaults **only when TERM is missing or `dumb`** (Finder/Dock launches inherit launchd's bare env). A real terminal's values are never overridden, so Claude Code keeps its native colors in both the GUI and the TUI. Same "fill the blanks" pattern for locale: `session.rs` sets `LC_CTYPE` (macOS `UTF-8`, else `C.UTF-8`) **only when `LC_ALL`/`LC_CTYPE`/`LANG` are all unset** — otherwise a Finder/Dock launch runs the child under the `C`/POSIX locale and it misdecodes multibyte UTF-8 *on input* (a pasted/typed em dash `—` → `‚Äî` mojibake), even though output looks fine (frontends always decode UTF-8). Decision is the pure `default_lc_ctype()` (unit-tested); set before the `env_vars` loop so a repo-config locale still wins.
- Wild claude processes are dynamically associated with a conversation: exact argv/fd evidence first, otherwise the latest-modified conversation in the process cwd (`latest_session_for_cwd`), re-evaluated each scan. `a` (TUI) / row click (GUI) = one confirm → kill outside process → `--resume` attach under the daemon.
- Log file (`~/Library/Application Support/clash/clash.log`) appends across restarts and auto-rotates after 24h (configurable via `CLASH_LOG_RETENTION_HOURS`).
- The GUI terminals set `macOptionIsMeta: true`, which alone would make brackets/braces untypeable on international Mac layouts (AZERTY types `{`/`[` with Option). Merely bypassing xterm (returning false from the custom key handler) does NOT fix it: WKWebView fires no keypress for Option combos and xterm's input-event fallback drops insertText preceded by a keydown (`!e.composed || !this._keyDownSeen`). The handler in `createTerminal` (gui/dist/app.js) must send the composed character to the PTY directly via `send_input`; Alt+letter still goes through xterm as Meta while the "⌥ sends Esc (Meta)" setting is on (when off, letters are direct-sent too).
- GUI terminal selection/copy requires `macOptionClickForcesSelection: true` on the xterm config. Claude Code enables mouse tracking (`?1000h`/`?1006h`), so xterm reports plain drags to it as mouse events and never selects text — making ⌘C copy nothing (or a garbled partial selection); this surfaced as "copy/paste doesn't work". With the flag, ⌥-drag forces a real selection (the native iTerm2 convention; xterm's `_shouldForceSelection` is `isMac ? altKey && macOptionClickForcesSelection : shiftKey`). It is mouse-only, so it does not interact with `macOptionIsMeta` / typing ⌥-composed glyphs. PTY I/O is byte-faithful end to end in *transport* (input: `send_input` → base64 `Request::Input` → raw PTY write; output: base64 → `Uint8Array` → `term.write`, which decodes UTF-8 and buffers split sequences), so mojibake is never introduced in transit. Two separate real bugs produced garbled chars: (1) the broken selection copying a partial cell run, fixed by the flag above; and (2) the PTY child running under the `C` locale and misdecoding pasted/typed UTF-8, fixed by the `LC_CTYPE` default in `session.rs` (see the PTY-env gotcha). The TUI attach needs no equivalent: it is raw passthrough, so the host terminal owns selection/copy/paste (⌥-drag in iTerm2, bracketed paste forwarded through `/dev/tty`).
- GUI session **reload** (⟳, next to the `✕` on section headers, on every session row/tab, and in the context menus) hot-restarts a session on the newest `claude` binary: `reloadSession(sid)` = `stash_session` (kill, keep resumable) → `dropTerminal` if open → `openSession` (which resumes the *latest* conversation id via `resolve_resume_id`, so no id is passed around by the frontend). It composes existing core ops — no new Tauri command. The section-level `reloadAll` and the per-row/tab buttons skip **actively-working** sessions (`isActivelyWorking`: status ∈ Thinking/Prompting/Waiting/Starting) because a turn in flight has no persisted id yet; reloading one of those individually first asks via `uiConfirm`. Wild rows get no reload (take-over is their action). No TUI equivalent yet — the crosses/icons are GUI-only.
- GUI split panes are resizable: `#terminal-host` is a CSS grid and `renderPanes` drives `gridTemplateColumns/Rows` from per-workspace `w.colFracs`/`w.rowFracs` (in `fr`), reset to equal whenever the grid shape changes (pane add/remove) or a single cell shows (zoom/one pane). Draggable `.pane-gutter` overlays sit at the internal track boundaries — positioned by `repositionGutters` from the cumulative fractions (not by reading pane layout), so `fitAll` (also the window-resize handler) repositions them. Dragging redistributes the fraction between the two adjacent tracks (min ~0.15fr each). Fractions persist in `gui-state.json`; `renderPanes` validates length + positivity and resets on mismatch, so a stale layout never yields a collapsed track.
- GUI panes clip their terminal (`.pane { overflow: hidden }` in `gui/dist/style.css`). FitAddon sizes rows from the **CSS** cell height (`rows = floor(availableHeight / cssCellHeight)`), but xterm renders each row rounded to **device** pixels, so `rows × renderedRowHeight` can exceed the container by a sub-row sliver. Neither `.pane`, `.term-wrap`, nor `.xterm` clips by default, so that excess `.xterm-screen` painted *past the pane's bottom edge* — and the excess scales with row count, so it was worst in the tallest terminal: a **single title-less pane** (no `.pane-title`, so it fills the whole grid cell). On a Retina Mac (fractional cell height, dpr 2) that pushed Claude's bottom input line out of the window — reported as "when only one tab is open, the bottom goes out of the window." `overflow: hidden` on `.pane` clips at the grid-cell (= window) boundary; xterm's own `.xterm-viewport` (`overflow-y: scroll`, absolute-positioned inside `.xterm`) still handles scrollback, so nothing is lost — only the sub-pixel sliver of the last row is trimmed.
- Self-update (`update.rs`) is symlink-aware and name-aware: it canonicalizes targets before replacing (so `/usr/local/bin/clash-gui` → `Clash.app/Contents/MacOS/clash-gui` updates the bundle binary, not the link), installs the running binary first and the sibling at its existing canonical location, and on macOS bumps the bundle's Info.plist version + re-signs. Replacing the symlink itself would leave Finder/Dock launches permanently stale.
- Apps launched from Finder/Dock inherit launchd's minimal `PATH` (no `~/.local/bin`), so spawning `claude` fails with ENOENT. Both binaries call `env_path::adopt_login_shell_path()` at startup: it queries the user's interactive login shell (rc-file PATH exports included) with a 3s timeout, falls back to well-known bin dirs, and no-ops when PATH already has home-relative entries (i.e. launched from a real shell).
