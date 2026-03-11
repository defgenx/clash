# clash

**K9s-style terminal UI for Claude Code Agent Teams.**

clash provides a real-time dashboard for managing Claude Code teams, agents, and tasks with keyboard-driven navigation, full CRUD management, and the ability to attach/detach to agent sessions — all from a single TUI.

```
┌──────────────────────────────────────────────────────────┐
│ clash      Teams > alpha-team > Tasks              14:32   │
├──────────────────────────────────────────────────────────┤
│ ID     STATUS       OWNER        SUBJECT                 │
│ ▶ 1    in_progress  researcher   Analyze API endpoints   │
│   2    pending      —            Write unit tests        │
│   3    completed    coder        Implement auth module   │
│   4    blocked      —            Deploy to staging       │
│                                                          │
├──────────────────────────────────────────────────────────┤
│ :command  /filter  ?help                                 │
└──────────────────────────────────────────────────────────┘
```

## Features

- **Real-time dashboard** — monitors `~/.claude/teams/` and `~/.claude/tasks/` with filesystem watching (200ms debounce)
- **Keyboard-driven** — vim-style navigation (j/k), command mode (`:`), filter mode (`/`), help (`?`)
- **Full CRUD** — create, update, delete teams and tasks; cycle task status; assign owners
- **Agent attach** — suspend TUI and attach to a running Claude session, return on exit
- **Resilient parsing** — lenient serde handles schema changes; malformed files show as error rows, not crashes
- **Atomic writes** — temp file + rename prevents partial reads from concurrent processes
- **Clean Architecture** — domain, application, adapter, and infrastructure layers with strict dependency direction

## Installation

```bash
# Build from source
cargo build --release

# Run
./target/release/clash

# Or with custom paths
clash --data-dir ~/.claude --claude-bin /usr/local/bin/claude
```

### Requirements

- Rust 1.75+ (2021 edition)
- A terminal with Unicode support
- Claude Code CLI (for team creation and agent attach)

## Usage

### Navigation

| Key | Action |
|-----|--------|
| `j` / `↓` | Select next row |
| `k` / `↑` | Select previous row |
| `g` | Jump to first item |
| `G` | Jump to last item |
| `Enter` | Drill into selected resource |
| `Esc` | Go back |
| `q` | Quit |

### Modes

| Key | Mode | Description |
|-----|------|-------------|
| `:` | Command | Navigate by name: `:teams`, `:tasks`, `:agents`, `:inbox`, `:quit` |
| `/` | Filter | Live-filter table rows as you type |
| `?` | Help | Context-sensitive keybinding reference |

### Actions

| Key | Context | Action |
|-----|---------|--------|
| `c` | Teams/Tasks | Create new resource |
| `d` | Any | Delete selected (with confirmation) |
| `s` | Tasks | Cycle task status (pending → in_progress → completed) |
| `a` | Agents | Attach to agent session |
| `m` | Agents/Inbox | Send message to agent |
| `r` | Any | Force refresh data |

### Views

- **Teams** — all teams with member counts, lead agent, description
- **Team Detail** — team info, member summary, task count
- **Agents** — team members with type, model, status, working directory
- **Agent Detail** — full agent info including prompt preview
- **Tasks** — team tasks with status (color-coded), owner, subject
- **Task Detail** — full task info with dependencies
- **Inbox** — agent inbox messages with read/unread indicators
- **Prompts** — agent system prompt viewer

## Architecture

clash follows **Clean Architecture** (Robert C. Martin) with four concentric layers. Dependencies point strictly inward — inner layers never import from outer layers.

```
┌─────────────────────────────────────────────────┐
│              Infrastructure                      │
│  ┌───────────────────────────────────────────┐  │
│  │             Adapters                       │  │
│  │  ┌─────────────────────────────────────┐  │  │
│  │  │          Application                 │  │  │
│  │  │  ┌───────────────────────────────┐  │  │  │
│  │  │  │           Domain              │  │  │  │
│  │  │  │  entities.rs  ports.rs        │  │  │  │
│  │  │  └───────────────────────────────┘  │  │  │
│  │  │  state.rs  actions/  effects.rs     │  │  │
│  │  │  reducer.rs  nav.rs                 │  │  │
│  │  └─────────────────────────────────────┘  │  │
│  │  input.rs  renderer.rs  views/            │  │
│  └───────────────────────────────────────────┘  │
│  app.rs  fs/  cli/  tui/  config.rs  event.rs   │
└─────────────────────────────────────────────────┘
```

### Layer Responsibilities

| Layer | Purpose | Dependencies |
|-------|---------|-------------|
| **Domain** | Entities (Team, Task, Member) and port traits (DataRepository, CliGateway) | None |
| **Application** | State, actions, effects, pure reducer | Domain |
| **Adapters** | Input → Action mapping, State → Frame rendering, view trait impls | Application, Domain |
| **Infrastructure** | Filesystem, CLI subprocess, TUI widgets, config, event loop | All layers |

### Data Flow (TEA Pattern)

```
User Input → Event → handle_key() → Action
                                       ↓
                              reducer::reduce()  ←── pure function
                                       ↓
                              (State', Vec<Effect>)
                                       ↓
                              execute_effects()  ←── infrastructure IO
                                       ↓
                              renderer::draw()   ←── pure read of state
                                       ↓
                              Terminal Frame
```

The **reducer** is a pure function: `fn(state, action) → (state, effects)`. It contains all business logic but performs no IO. Effects are domain-level descriptions (`PersistTask`, `RemoveTeam`, `RunCli`) that the infrastructure layer translates into real filesystem and process operations.

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Abstract Effects (no file paths in reducer) | Keeps application logic infrastructure-agnostic; testable without filesystem |
| Atomic writes (temp + rename) | Prevents partial-read corruption during concurrent access |
| Lenient serde (`#[serde(default)]` + `#[serde(flatten)]`) | Resilient to Claude Code schema changes |
| Debounced FS watching (200ms) | Coalesces burst events; targeted reload by path |
| LRU(3) inbox cache | Inboxes loaded on navigation, evicted automatically |
| `TableView` / `DetailView` traits | Each view is ~30-50 lines of declarations; generic widgets handle rendering |
| Suspend-and-resume for agent attach | Save state → restore terminal → spawn claude → reclaim terminal |

## Data Model

clash reads from Claude Code's filesystem layout:

```
~/.claude/
├── teams/{name}/
│   ├── config.json          # Team: name, description, members[]
│   └── inboxes/{agent}.json # InboxMessage[]: from, text, timestamp, read
└── tasks/{team-name}/
    └── {id}.json            # Task: id, subject, status, owner, blocks[], blockedBy[]
```

All types use `#[serde(default)]` so missing fields get zero values, and `#[serde(flatten)]` to capture unknown fields — ensuring forward compatibility when Claude Code adds new fields.

## Configuration

Optional config file at `~/.config/clash/config.toml`:

```toml
claude_bin = "claude"           # Path to Claude CLI
claude_dir = "/home/user/.claude"  # Override data directory
tick_rate_ms = 250              # Animation tick rate
debounce_ms = 200               # FS watcher debounce
```

## Development

```bash
# Run tests (111 tests: unit + integration)
cargo test

# Run with custom data dir (for testing)
cargo run -- --data-dir /path/to/test/data

# Check for warnings
cargo build 2>&1 | grep warning
```

### Test Strategy

- **Unit tests** (inline `#[cfg(test)]`): reducer actions, serde parsing, navigation, input handling, atomic writes, CLI commands
- **Integration tests** (`tests/`): full data layer with fixture files, end-to-end action→state→effect cycles
- **Test fixtures** (`tests/fixtures/`): 5 teams (valid, empty, malformed, extra fields), 4 tasks, inbox messages

### Project Structure

```
src/
├── domain/                  # Inner layer: entities + ports
│   ├── entities.rs          # Team, Member, Task, InboxMessage, TaskStatus
│   └── ports.rs             # DataRepository, CliGateway traits
├── application/             # Application layer: pure logic
│   ├── state.rs             # AppState, InputMode, TableState
│   ├── nav.rs               # NavigationStack with breadcrumbs
│   ├── actions/             # Nested action enums (Nav, Table, Team, Task, Agent, UI)
│   ├── effects.rs           # Effect + CliCommand enums (domain-level, no file paths)
│   └── reducer.rs           # Pure fn(state, action) → (state, effects)
├── adapters/                # Translation layer
│   ├── input.rs             # KeyEvent → Action mapping
│   ├── renderer.rs          # AppState → Frame rendering
│   └── views/               # TableView + DetailView trait implementations
│       ├── teams.rs         # Teams table (NAME, MEMBERS, LEAD, DESCRIPTION)
│       ├── tasks.rs         # Tasks table (ID, STATUS, OWNER, SUBJECT)
│       ├── agents.rs        # Agents table (NAME, TYPE, MODEL, STATUS, MODE, CWD)
│       ├── inbox.rs         # Inbox table (FROM, TIME, MESSAGE, READ)
│       ├── team_detail.rs   # Team info sections
│       ├── task_detail.rs   # Task info + dependencies
│       ├── agent_detail.rs  # Agent info + runtime + prompt
│       └── prompts.rs       # Prompt viewer
└── infrastructure/          # Outer layer: real IO
    ├── app.rs               # Event loop + effect executor
    ├── config.rs            # Config file loading
    ├── error.rs             # AppError (thiserror)
    ├── event.rs             # Crossterm event reader + tick timer
    ├── fs/                  # Filesystem backend
    │   ├── backend.rs       # FsBackend (impl DataRepository)
    │   ├── atomic.rs        # write_atomic(path, data)
    │   ├── store.rs         # In-memory cache with LRU inbox
    │   └── watcher.rs       # Debounced FS watcher (notify, 200ms)
    ├── cli/                 # Claude CLI integration
    │   ├── runner.rs        # RealCliRunner (impl CliGateway)
    │   ├── commands.rs      # CliCommand → raw args translation
    │   └── parser.rs        # CLI JSON output parsing
    └── tui/                 # Terminal UI framework
        ├── layout.rs        # Header/body/footer frame layout
        ├── theme.rs         # Colors, styles, status indicators
        └── widgets/         # Reusable UI components
            ├── table.rs     # Generic table renderer
            ├── detail.rs    # Generic detail renderer
            ├── input_bar.rs # Command/filter input
            ├── help_overlay.rs
            ├── confirm_dialog.rs
            ├── spinner.rs
            └── toast.rs
```

## License

MIT - see [LICENSE](LICENSE)
