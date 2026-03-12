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

- **Session management** — list, attach, detach, create, and delete Claude Code sessions
- **Inline terminal** — attach to sessions with a full vt100 terminal emulator, no window switching
- **Real-time status** — instant status detection via file watcher + JSONL parsing
- **Teams & tasks** — organize agents into teams, manage tasks, send messages
- **Keyboard-driven** — vim-style navigation, command mode (`:`), fuzzy filter (`/`), context help (`?`)
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

### Requirements

- Rust 1.75+ (for building from source)
- Claude Code CLI (`claude`)

## Usage

```bash
clash                              # Start (reads from ~/.claude)
clash --data-dir ~/.claude         # Custom data directory
clash --claude-bin /path/to/claude # Custom CLI path
clash daemon                       # Start daemon separately
clash update                       # Update to the latest release
```

On first launch, clash shows a guided tour. Replay it anytime with `:tour`.

### Session Status

| Icon | Status | Meaning |
|------|--------|---------|
| `◉` | Prompting | Claude needs tool approval |
| `◉` | Waiting | Awaiting your next prompt |
| `◎` | Thinking | Reasoning / generating |
| `●` | Running | Executing tools |
| `⦿` | Starting | Session just spawned |
| `○` | Idle | Exited or inactive |

## Keybindings

### Navigation

| Key | Action |
|-----|--------|
| `j` / `k` | Select next / previous |
| `g` / `G` | Jump to first / last |
| `Enter` | Drill in |
| `Esc` | Go back |
| `q` | Quit |

### Modes

| Key | Description |
|-----|-------------|
| `:` | Command mode — `:teams`, `:sessions`, `:tour`, `:update`, `:quit` |
| `/` | Fuzzy filter |
| `?` | Context help |

### Sessions

| Key | Action |
|-----|--------|
| `a` | Attach (inline terminal) |
| `c` / `n` | New session |
| `Tab` | Expand / collapse subagents |
| `A` | Toggle active / all |
| `d` | Delete session |
| `D` | Delete ALL sessions |

### Attached Mode

| Key | Action |
|-----|--------|
| `Esc` / `Ctrl+B` | Detach |
| Everything else | Forwarded to Claude |

### Session Detail

| Key | Action |
|-----|--------|
| `s` | Subagents |
| `t` | Linked team |
| `m` | Team members |
| `a` | Attach |
| `d` | Delete |

## Data

clash reads directly from Claude Code's filesystem:

```
~/.claude/
├── projects/{name}/
│   ├── {session-id}.jsonl          # Conversation
│   └── {session-id}/subagents/     # Subagent transcripts
├── teams/{name}/config.json        # Team config + members
└── tasks/{team-name}/{id}.json     # Tasks
```

## Development

```bash
cargo test          # Run all tests
cargo clippy        # Lint
cargo fmt --check   # Check formatting
```

Releases are automatic — push with conventional commits (`feat:`, `fix:`) and CI handles the rest.

## License

MIT
