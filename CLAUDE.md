# CLAUDE.md — clash Project Guide

## What is clash?

Terminal UI for managing Claude Code Sessions & Agent Teams. Built in Rust with ratatui.

## Quick Commands

```bash
cargo build --release      # Build
cargo test                 # Run all 111 tests (unit + integration)
cargo run -- --data-dir /path/to/test/data  # Run with test data
```

## Before Pushing

**Always run format and linter before every push:**

```bash
cargo fmt                  # Auto-format code
cargo clippy               # Run linter
```

CI runs `cargo fmt --check` and `cargo clippy` — pushes will fail if either has issues.

## Architecture: Clean Architecture (strict layers)

```
Infrastructure → Adapters → Application → Domain
(outer)                                   (inner)
```

**Dependencies point inward only.** Inner layers never import from outer layers.

### Layer Map

| Layer | Path | Purpose | May depend on |
|-------|------|---------|---------------|
| **Domain** | `src/domain/` | Entities (`Team`, `Task`, `Member`, `InboxMessage`, `TaskStatus`) and port traits (`DataRepository`, `CliGateway`) | Nothing |
| **Application** | `src/application/` | `AppState`, `Action` enums, `Effect` enum, pure `reducer`, `NavigationStack` | Domain |
| **Adapters** | `src/adapters/` | `input.rs` (KeyEvent→Action), `renderer.rs` (State→Frame), `views/` (TableView/DetailView impls) | Application, Domain |
| **Infrastructure** | `src/infrastructure/` | `app.rs` (event loop), `fs/` (FsBackend, atomic writes, watcher), `cli/` (subprocess), `tui/` (widgets, theme, layout), `config.rs`, `error.rs`, `event.rs` | All layers |

### Key Files

- `src/domain/entities.rs` — All domain types with lenient serde (`#[serde(default)]` + `#[serde(flatten)]`)
- `src/domain/ports.rs` — `DataRepository` and `CliGateway` trait definitions
- `src/application/reducer.rs` — **Pure function** `fn(state, action) → (state, effects)`. No IO. All business logic lives here.
- `src/application/effects.rs` — Abstract effects (`PersistTask`, `RemoveTeam`, `RunCli`). No file paths — infrastructure translates these to real IO.
- `src/application/actions/mod.rs` — Nested action enums: `Nav`, `Table`, `Team`, `Task`, `Agent`, `Ui`
- `src/infrastructure/app.rs` — Event loop coordinator. Executes effects by calling `DataRepository` / `CliGateway` impls.
- `src/infrastructure/fs/backend.rs` — `FsBackend` implements `DataRepository`
- `src/infrastructure/fs/atomic.rs` — `write_atomic()`: write to temp file, then rename (prevents partial reads)

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
└── tasks/{team-name}/{id}.json       # Task with status, owner, dependencies
```

All types use `#[serde(default)]` so missing fields get zero values, and `#[serde(flatten)]` captures unknown fields for forward compatibility.

## Conventions

- **No IO in the reducer.** Effects are the only way the reducer communicates with the outside world.
- **Atomic writes everywhere.** Use `write_atomic()` for any file mutation to prevent corruption during concurrent access.
- **Lenient serde on all domain types.** Never fail on unknown/missing fields — use defaults and capture extras.
- **Each view is a trait impl.** `TableView` for list screens, `DetailView` for detail screens. Generic widgets in `tui/widgets/` render them.
- **Nested action enums.** `Action::Team(TeamAction::Create { .. })`, not flat variants. Each domain has its own sub-reducer.
- **Tests alongside code.** Unit tests in `#[cfg(test)]` modules within each file. Integration tests in `tests/`.

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

## Dependencies

Key crates: `ratatui` (TUI), `crossterm` (terminal), `tokio` (async), `serde`/`serde_json` (data), `notify-debouncer-full` (FS watching), `clap` (CLI args), `color-eyre`/`thiserror` (errors), `lru` (inbox cache), `tui-input` (text input widget), `chrono` (timestamps), `fuzzy-matcher` (filter mode).

## Gotchas

- `main.rs` uses `mod` (private), `lib.rs` uses `pub mod` (public for tests). Both declare the same 4 layers.
- The `notify-debouncer-full` error type lives at `notify_debouncer_full::notify::Error`, not `notify::Error`.
- Raw strings containing `#` in JSON values (like hex colors) need `r##"..."##` delimiters.
- Inboxes are lazy-loaded with LRU(3) eviction — they're only fetched when navigating to the inbox view.
- Agent attach uses suspend-and-resume: save state → restore terminal → spawn `claude --resume` → reclaim terminal on exit.
