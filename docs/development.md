# Development

## Building

```bash
cargo build --release           # builds BOTH binaries: clash and clash-gui
./target/release/clash-gui
cargo run -- --debug            # run the TUI with debug-level logging
cargo run -- --data-dir /path/to/test/data   # run against test data
```

A recent stable Rust (1.82+) is required. On Linux the GUI build needs the Tauri
system packages: `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libxdo-dev`.
The GUI is self-contained: there is no external daemon and no node build step,
since the frontend in `gui/dist/` is vendored and embedded in the binary.

`make install` (or `make install-tui` / `make install-gui`) installs from a
clone; see [Installation](../README.md#installation).

## Tests and lint

```bash
cargo test --all-targets            # Rust tests
node --test "gui/tests/*.test.js"    # GUI frontend tests
cargo clippy -- -D warnings         # Lint (as CI runs it)
cargo fmt --check                   # Formatting
```

Some tests read the documentation: `tests/docs_consistency_test.rs` and the
skills tests require README.md and [workflows.md](workflows.md) to keep naming
certain features and every embedded skill.

Releases are automatic — push with conventional commits (`feat:`, `fix:`) and
CI handles the rest.

## Architecture

clash follows **The Elm Architecture** (TEA) with clean architecture layers:

```
User Input → Action → reducer() → (State', Effects) → execute_effects() → draw()
                        (pure)                          (infrastructure IO)
```

| Layer | Purpose |
|-------|---------|
| **Domain** | Entities, port traits — no dependencies |
| **Application** | State, actions, effects, pure reducer, pure workflow/session logic |
| **Adapters** | Input mapping, view rendering |
| **Infrastructure** | Event loop, filesystem, in-process PTY daemon, session refresh, config, hooks, windowing, git/`gh`/forge, skills, self-update, TUI widgets |

Two frontends share that core: the TUI (`src/main.rs`) and the GUI — a Tauri 2
app (`gui/src-tauri`) whose frontend is plain JS in `gui/dist` with its pure
modules tested under `gui/tests`. [CLAUDE.md](../CLAUDE.md) maps every
subsystem.
