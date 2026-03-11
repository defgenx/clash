# clash

**K9s-style terminal UI for Claude Code Sessions.**

Attach to running Claude sessions, monitor their status in real-time, manage teams, agents, and tasks — all from a keyboard-driven TUI.

```
┌─────────────────────────────────────────────────────────────────┐
│ ✦ clash  │  Sessions                        ▸ 2 waiting  15:42 │
├─────────────────────────────────────────────────────────────────┤
│ STATUS       SESSION   PROJECT       SUMMARY          BRANCH   │
│ ◉ PROMPTING  a1b2c3d4  my-api       Fix auth module   main    │
│ ◉ WAITING    e5f6g7h8  web-app      Add dark mode     feat/ui │
│ ● RUNNING    i9j0k1l2  cli-tool     Refactor parser   main    │
│ ◎ THINKING   m3n4o5p6  docs         Update README     docs    │
│ ○ IDLE       q7r8s9t0  backend      Deploy setup      main    │
├─────────────────────────────────────────────────────────────────┤
│ :command  /filter  ?help                                        │
└─────────────────────────────────────────────────────────────────┘
```

## Features

- **Session management** — list, attach, detach, create, and delete Claude Code sessions
- **Inline terminal** — attach to sessions with a full vt100 terminal emulator, no window switching
- **Real-time status** — 6 granular states (Idle, Starting, Running, Thinking, Waiting, Prompting) detected by parsing terminal screen content
- **Daemon architecture** — persistent PTY sessions survive TUI restarts, multi-client attach support
- **Teams & tasks** — CRUD for teams, agents, and tasks backed by Claude Code's filesystem
- **Keyboard-driven** — vim-style navigation, command mode (`:`), fuzzy filter (`/`), context help (`?`)
- **Resilient parsing** — lenient serde handles schema changes; malformed files show as error rows
- **Atomic writes** — temp file + rename prevents corruption from concurrent access

## Installation

### Quick install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/defgenx/clash/main/install.sh | bash
```

This downloads the latest release binary for your platform and installs it to `/usr/local/bin`. To install elsewhere:

```bash
CLASH_INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/defgenx/clash/main/install.sh | bash
```

### Build from source

```bash
cargo install --git https://github.com/defgenx/clash.git
```

Or clone and build:

```bash
git clone https://github.com/defgenx/clash.git
cd clash
cargo build --release
./target/release/clash
```

### Requirements

- Rust 1.75+ (for building from source)
- A terminal with Unicode support
- Claude Code CLI (`claude`) for session management

## Usage

```bash
# Start clash (reads from ~/.claude by default)
clash

# Custom data directory
clash --data-dir ~/.claude

# Custom Claude CLI binary path
clash --claude-bin /usr/local/bin/claude

# Start the daemon separately (auto-started on first use)
clash daemon
```

### Session Status

| Icon | Status | Meaning |
|------|--------|---------|
| `◉ PROMPTING` | Tool approval needed | Claude is asking for permission (Yes/No) |
| `◉ WAITING` | Waiting for input | Claude finished and awaits your next prompt |
| `◎ THINKING` | Thinking | Claude is reasoning / generating |
| `● RUNNING` | Running | Claude is executing tools, writing code |
| `⦿ STARTING` | Starting | Session just spawned |
| `○ IDLE` | Idle | Session is inactive |

Status is detected by parsing the terminal screen content via a vt100 emulator — not timing heuristics.

### Keybindings

#### Navigation

| Key | Action |
|-----|--------|
| `j` / `↓` | Select next |
| `k` / `↑` | Select previous |
| `g` | Jump to first |
| `G` | Jump to last |
| `Enter` | Drill in / attach |
| `Esc` | Go back |
| `q` | Quit |

#### Modes

| Key | Mode | Description |
|-----|------|-------------|
| `:` | Command | Navigate: `:teams`, `:tasks`, `:sessions`, `:quit` |
| `/` | Filter | Fuzzy filter table rows |
| `?` | Help | Context-sensitive keybinding reference |

#### Sessions

| Key | Action |
|-----|--------|
| `Enter` | Attach to session (inline terminal) |
| `c` | Create new Claude session |
| `d` | Delete session (with confirmation) |
| `A` | Toggle all / active sessions |
| `i` | Inspect session details |

#### Attached mode

| Key | Action |
|-----|--------|
| `Esc` / `Ctrl+B` | Detach from session |
| All other keys | Forwarded to Claude |

#### Teams & Tasks

| Key | Context | Action |
|-----|---------|--------|
| `c` | Teams/Tasks | Create new |
| `d` | Any | Delete (with confirmation) |
| `s` | Tasks | Cycle status |
| `r` | Any | Force refresh |

### Views

- **Sessions** — all Claude sessions with status, project, summary, branch
- **Session Detail** — session info, subagent list, conversation transcript
- **Subagents** — subagents spawned by a session
- **Teams** — all teams with member counts and description
- **Team Detail** — team info, members, task count
- **Agents** — team members with type, model, status
- **Tasks** — team tasks with status, owner, subject
- **Inbox** — agent inbox messages

## Architecture

Clean Architecture with four layers. Dependencies point strictly inward.

```
┌─────────────────────────────────────────────────┐
│              Infrastructure                      │
│  ┌───────────────────────────────────────────┐  │
│  │             Adapters                       │  │
│  │  ┌─────────────────────────────────────┐  │  │
│  │  │          Application                 │  │  │
│  │  │  ┌───────────────────────────────┐  │  │  │
│  │  │  │           Domain              │  │  │  │
│  │  │  └───────────────────────────────┘  │  │  │
│  │  └─────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

**TEA pattern** (The Elm Architecture):

```
Input → Action → reducer(state, action) → (state', effects) → execute → draw
                 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
                         pure function, no IO
```

The reducer contains all business logic but performs no IO. Effects are domain-level descriptions (`PersistTask`, `DaemonAttach`, `RefreshSessions`) that infrastructure translates into real operations.

## Data Model

clash reads from Claude Code's filesystem:

```
~/.claude/
├── projects/{name}/
│   ├── sessions-index.json     # Session metadata
│   ├── {session-id}.jsonl      # Conversation transcript
│   └── {session-id}/subagents/ # Subagent transcripts
├── teams/{name}/
│   ├── config.json             # Team config + members[]
│   └── inboxes/{agent}.json    # Inbox messages
└── tasks/{team-name}/
    └── {id}.json               # Task with status, owner, deps
```

## Configuration

Optional config at `~/.config/clash/config.toml`:

```toml
claude_bin = "claude"
claude_dir = "/home/user/.claude"
tick_rate_ms = 250
debounce_ms = 200
```

## Development

```bash
cargo test          # 111 tests (unit + integration)
cargo clippy        # Zero warnings
cargo fmt --check   # Formatting
```

## License

MIT — see [LICENSE](LICENSE)
