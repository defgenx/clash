# clash — K9s-style TUI for Claude Code Agent Teams

## Context

Claude Code's experimental Agent Teams feature lets you spawn, coordinate, and manage multiple AI agents working in parallel. However, there's no dedicated dashboard to visualize team state, jump between agents, or manage tasks — you're limited to the interactive CLI with `Shift+Down` cycling or tmux panes.

**clash** fills this gap: a K9s-inspired terminal UI that provides a real-time dashboard for all your Claude Code teams, agents, and tasks with full CRUD management, keyboard-driven navigation, and the ability to attach/detach to agent sessions.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                    clash TUI (ratatui)                 │
│  ┌──────────┐  ┌───────────┐  ┌──────────────────┐  │
│  │ Event    │  │ App State │  │ View Renderer    │  │
│  │ Loop     │─>│ Machine   │─>│ (table/detail)   │  │
│  └──────────┘  └───────────┘  └──────────────────┘  │
│       ▲              │                               │
│       │              ▼                               │
│  ┌──────────┐  ┌───────────┐                        │
│  │ FS Watch │  │ CLI Exec  │                        │
│  │ (notify) │  │ (tokio)   │                        │
│  └──────────┘  └───────────┘                        │
│       ▲              │                               │
└───────┼──────────────┼───────────────────────────────┘
        │              ▼
   ~/.claude/      claude CLI
   teams/tasks/    (mutations + sessions)
```

**Stack**: Rust + ratatui + crossterm + tokio + serde + notify

**Data strategy**: Direct filesystem reads from `~/.claude/` for speed (teams, tasks, inboxes). Claude CLI (`-p --output-format json`) for mutations and session management.

---

## Data Model (from real filesystem)

**Team** — `~/.claude/teams/{name}/config.json`
```
name, description, createdAt, leadAgentId, leadSessionId
members[]: { agentId, name, agentType, model, prompt, color,
             joinedAt, tmuxPaneId, cwd, backendType, isActive, mode }
```

**Task** — `~/.claude/tasks/{team-name}/{id}.json`
```
id, subject, description, activeForm, status (pending|in_progress|completed),
blocks[], blockedBy[], owner
```

**Inbox** — `~/.claude/teams/{name}/inboxes/{agent-name}.json`
```
[{ from, text, timestamp, color, read }]
```

---

## Project Structure

```
clash/
├── Cargo.toml
└── src/
    ├── main.rs              # Entry point, tokio runtime, clap args
    ├── app.rs               # App struct, state machine, update/draw loop
    ├── event.rs             # Event enum, crossterm reader, tick timer
    ├── action.rs            # Action enum (all user-initiated mutations)
    ├── nav.rs               # NavigationStack, breadcrumb management
    ├── input.rs             # InputMode enum, key dispatch
    ├── config.rs            # clash config (~/.config/clash/config.toml)
    ├── error.rs             # AppError with thiserror
    ├── cli/
    │   ├── mod.rs
    │   ├── executor.rs      # Async subprocess runner (tokio::process)
    │   ├── commands.rs      # Typed command builders
    │   └── parser.rs        # Parse CLI JSON output into domain types
    ├── data/
    │   ├── mod.rs
    │   ├── types.rs         # Team, Member, Task, InboxMessage (serde)
    │   ├── store.rs         # In-memory cache with freshness tracking
    │   ├── loader.rs        # Direct filesystem JSON reader
    │   └── watcher.rs       # notify-based FS watcher + polling fallback
    ├── ui/
    │   ├── mod.rs
    │   ├── layout.rs        # Frame layout: header, body, footer
    │   ├── theme.rs         # Colors, status-to-style mapping
    │   └── widgets/
    │       ├── mod.rs
    │       ├── table.rs     # Generic sortable/filterable table
    │       ├── detail.rs    # Key-value detail pane
    │       ├── command_bar.rs  # ":" command input
    │       ├── filter_bar.rs   # "/" filter input
    │       ├── help_overlay.rs # "?" context-sensitive help
    │       ├── confirm_dialog.rs  # y/n for destructive actions
    │       └── toast.rs     # Transient notification bar
    └── views/
        ├── mod.rs           # ViewKind enum, view dispatch
        ├── teams.rs         # :teams list
        ├── team_detail.rs   # Single team (members, info)
        ├── agents.rs        # :agents list
        ├── agent_detail.rs  # Single agent (prompt, status, model)
        ├── tasks.rs         # :tasks list
        ├── task_detail.rs   # Single task (description, deps, owner)
        ├── inbox.rs         # :inbox message list
        └── prompts.rs       # :prompts viewer/editor
```

---

## Core Architecture: TEA (The Elm Architecture)

Unidirectional data flow: **Events → Actions → State → Render**

### Key Types

```
AppState {
    teams: Vec<Team>
    current_team: Option<String>
    tasks: HashMap<String, Vec<Task>>   // team → tasks
    inboxes: HashMap<key, Vec<Message>>
    filter: String
    sort_column: usize
    selected_index: usize
    pending_confirm: Option<PendingAction>
    toast: Option<(String, Instant)>
}

InputMode = Normal | Command(String) | Filter(String) | Confirm | Help

Event = Key(KeyEvent) | Tick | DataRefresh(payload) | CliResult(id, Result) | Resize

Action = NavigateTo(ViewKind) | DrillIn | GoBack
       | SelectNext | SelectPrev | Sort(col) | ApplyFilter | ClearFilter
       | CreateTeam{..} | DeleteTeam{..}
       | CreateTask{..} | UpdateTaskStatus{..} | AssignTask{..}
       | AttachAgent{..} | DetachAgent{..} | SendMessage{..}
       | ShowHelp | ShowConfirm(Action) | Quit | RefreshAll

ViewKind = Teams | TeamDetail(name) | Agents(team?) | AgentDetail(team, id)
         | Tasks(team) | TaskDetail(team, id) | Inbox(team, agent) | Prompts(team?)
```

### Main Loop (tokio select)

```
loop {
    terminal.draw(|f| app.draw(f));
    select! {
        event = event_rx.recv() => app.handle_event(event) → action_tx
        action = action_rx.recv() => app.update(action).await
    }
    if app.should_quit { break }
}
```

---

## UI Layout

```
┌──────────────────────────────────────────────────────────┐
│ clash v0.1.0      Teams > my-team > Tasks          14:32  │ ← Header
├──────────────────────────────────────────────────────────┤
│ NAME          STATUS      OWNER        SUBJECT           │ ← Table
│ ▶ 1           in_progress researcher   Analyze API       │   (body)
│   2           pending     —            Write tests       │
│   3           completed   coder        Implement auth    │
│   4           pending     —            Deploy staging    │
│                                                          │
├──────────────────────────────────────────────────────────┤
│ :tasks                                          ? help   │ ← Footer
└──────────────────────────────────────────────────────────┘
```

**Status colors**: green=completed, yellow=in_progress, red=blocked, dim=pending

---

## Navigation & Keybindings

| Key | Mode | Action |
|-----|------|--------|
| `j`/`↓` | Normal | Select next row |
| `k`/`↑` | Normal | Select previous row |
| `Enter` | Normal | Drill into selected resource |
| `Esc` | Normal | Go back (pop navigation stack) |
| `:` | Normal | Enter command mode |
| `/` | Normal | Enter filter mode |
| `?` | Normal | Toggle help overlay |
| `c` | Normal | Create new resource (context-dependent) |
| `d` | Normal | Delete selected (with confirmation) |
| `a` | Normal | Attach to agent / Assign task |
| `e` | Normal | Edit selected resource |
| `r` | Normal | Force refresh data |
| `s` | Tasks | Cycle task status |
| `m` | Agent | Send message to agent |
| `q` | Normal | Quit |
| `Enter` | Cmd/Filter | Confirm input |
| `Esc` | Cmd/Filter | Cancel input |

**Commands**: `:teams`, `:agents`, `:tasks`, `:prompts`, `:inbox`, `:quit`

**Navigation stack**: Enter pushes, Esc pops, `:command` replaces stack root. Breadcrumbs rendered in header.

---

## Data Layer

### Reads — Direct Filesystem (fast, real-time)
- `FsLoader` reads `~/.claude/teams/*/config.json` and `~/.claude/tasks/*/*.json`
- `notify::RecommendedWatcher` triggers targeted reloads on file changes
- Fallback: polling every 2s if notify is unavailable
- `DataStore` caches parsed data in memory with freshness timestamps

### Writes — CLI + Direct File Manipulation

| Operation | Method |
|-----------|--------|
| Create team | `claude -p "Create team {name}: {desc}" --output-format json` |
| Delete team | Remove `~/.claude/teams/{name}/` directory |
| Create task | Write JSON to `~/.claude/tasks/{team}/{next_id}.json` |
| Update task | Edit JSON fields in task file directly |
| Assign task | Set `owner` field in task JSON |
| Attach agent | `claude --resume {session_id}` in embedded terminal |
| Send message | Write to `~/.claude/teams/{name}/inboxes/{agent}.json` |

### Session Management
- `claude -p --output-format json` returns `session_id` in responses
- `claude --resume {session_id}` to reattach
- `claude --continue` for most recent session
- Stream output via `--output-format stream-json --verbose`

---

## CRUD Action Mapping

| View | `c` Create | `d` Delete | `e` Edit | Extra |
|------|-----------|-----------|---------|-------|
| :teams | New team (name+desc form) | Delete team (confirm) | Edit description | — |
| :agents | Spawn agent (name+model) | Remove from team (confirm) | Edit prompt | `a` attach, `m` message |
| :tasks | New task (subject+desc) | Delete task (confirm) | Edit subject/desc | `s` cycle status, `a` assign |
| :prompts | — | — | Edit prompt text | — |
| :inbox | — | Clear inbox (confirm) | — | — |

---

## Dependencies (Cargo.toml)

```toml
ratatui = "0.29"          # TUI framework
crossterm = "0.28"        # Terminal backend
tokio = { version = "1", features = ["full"] }  # Async runtime
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }  # CLI args
thiserror = "2"           # Error types
notify = "7"              # Filesystem watching
dirs = "6"                # Home directory resolution
chrono = "0.4"            # Timestamp formatting
unicode-width = "0.2"    # Column width handling
toml = "0.8"             # Config file parsing
```

---

## Implementation Phases

### Phase 1 — Skeleton
- `main.rs`: tokio runtime, crossterm init/restore, clap args
- `event.rs`: crossterm event reader + 250ms tick timer
- `app.rs`: minimal App with empty state, draw loop, quit on `q`
- `ui/layout.rs`: header/body/footer frame layout
- `nav.rs`: NavigationStack with push/pop/breadcrumbs

### Phase 2 — Data Layer
- `data/types.rs`: all serde structs matching real JSON schemas
- `data/loader.rs`: filesystem reader for teams, tasks, inboxes
- `data/store.rs`: DataStore with load_all and caching
- `data/watcher.rs`: notify-based FS watcher + polling fallback

### Phase 3 — Table Views
- `ui/widgets/table.rs`: generic sortable/filterable table widget
- `views/teams.rs`: Teams list (name, agents count, tasks count, status)
- `views/tasks.rs`: Tasks list (id, status, owner, subject)
- `views/agents.rs`: Agents list (name, type, model, status, color)
- `input.rs`: Normal mode key handling (j/k/Enter/Esc)

### Phase 4 — Detail Views & Navigation
- `views/team_detail.rs`, `views/task_detail.rs`, `views/agent_detail.rs`
- `ui/widgets/detail.rs`: key-value detail pane
- `views/inbox.rs`: inbox message list
- Command mode (`:`) and filter mode (`/`) input handling

### Phase 5 — Actions & Mutations
- `cli/executor.rs`: async subprocess runner with timeout
- `action.rs`: full Action enum and dispatch in app.update()
- Direct file mutations for task/team CRUD
- `ui/widgets/confirm_dialog.rs` + `ui/widgets/toast.rs`
- `views/prompts.rs`: prompt viewer/editor

### Phase 6 — Polish
- `ui/theme.rs`: consistent color scheme, status indicators
- `ui/widgets/help_overlay.rs`: context-sensitive `?` help
- `config.rs`: clash config file (`~/.config/clash/config.toml`)
- Error handling: toast for non-fatal, full-screen for fatal
- Edge cases: empty states, concurrent file writes, malformed JSON

---

## Verification Plan

1. **Build**: `cargo build` compiles without errors
2. **Smoke test**: `cargo run` launches TUI, renders header/footer, quits on `q`
3. **Data loading**: Create a test team via Claude Code, verify clash displays it
4. **Navigation**: Test `:teams` → Enter → `:tasks` → Esc flow, verify breadcrumbs
5. **Filter**: Type `/` + search term, verify table rows filter live
6. **CRUD**: Create a task from clash, verify it appears in `~/.claude/tasks/`
7. **Real-time**: Modify a task file externally, verify clash updates within 1s
8. **Attach**: Press `a` on an agent, verify claude session opens
9. **Error handling**: Remove a team directory, verify toast not crash
