# Configuration — reference

One namespaced, layered, schema-described file that both frontends read.
README.md has the overview and the settings you'll actually reach for; this is
the reference for how the subsystem behaves, and the contract a second frontend
(or a future `[keymap]` phase) has to hold to.

## The schema is the source of truth

`src/infrastructure/config/schema.rs` holds one `PROPS` table. Every consumer
is derived from it, so nothing can drift:

| Consumer | Derived how |
|---|---|
| defaults for every layer | `defaults_table()` — and `Config::default()` reads the same table, pinned by a test |
| validation | `validate_table()` / `check_value()` |
| `clash config --defaults` | `defaults_toml()` |
| `clash config --schema` | `json_schema()` |
| the GUI settings migration | `Prop::gui_key` |
| GUI live-apply hints | `term_option`, `refit`, `restart_required` |

Adding a setting is one `Prop` entry. Nothing else is required for it to be
loadable, validatable, documented in `--defaults`, and present in the JSON
Schema.

### Per-property metadata

| Field | Meaning |
|---|---|
| `path` | dotted path, e.g. `sessions.refresh_secs` |
| `gui_key` | the legacy camelCase `gui-state.json` key, when it has one |
| `kind` | `Bool` / `Int{min,max}` / `Float{min,max}` / `Str` / `Path` / `Enum` |
| `default` | must satisfy its own constraints (asserted by a test) |
| `doc` | one line; shown by `--defaults` and in the JSON Schema `description` |
| `scope` | `Shared` (in `config.toml`) or `GuiLocal` (in `gui-state.json`) |
| `term_option` | the xterm option the GUI live-applies |
| `refit` | changing it alters cell metrics, so terminals must be refit |
| `restart_required` | cannot be applied to already-running things |
| `project_allowed` | a repo-local config may override it |

`Kind::Path` uses `""` for "unset, compute the default" — that is why
`paths.scratch_dir = ""` and omitting the key mean the same thing.

## Shared vs GUI-local

Of the GUI's 28 historical settings, 7 are meaningful to both frontends and live
in `config.toml`:

| `config.toml` | legacy GUI key |
|---|---|
| `sessions.default_cwd` | `defaultCwd` |
| `sessions.confirm_kill` | `confirmKill` |
| `sessions.refresh_secs` | `refreshSecs` |
| `terminal.shell` | `termShell` |
| `terminal.tui_terminal` | `tuiTerminal` |
| `notifications.enabled` | `notifications` |
| `notifications.title_attention` | `titleAttention` |

The other 21 configure xterm rendering or the GUI chrome (font, cursor,
scrollback, scroll, copy/paste, link handling, theme). The TUI can never apply
them, so they stay in `gui-state.json` — but they still get schema entries, so
one validator covers both frontends and a generated settings panel can render
every row from one table.

A test (`all_28_legacy_gui_keys_are_accounted_for`) pins this split: a new GUI
setting without a schema entry fails the build instead of silently reverting to
a default at runtime.

## Layers

```
defaults (from the schema)
  ← ~/.config/clash/config.toml        [user]
  ← <repo>/.clash/config.toml          [project — restricted]
  ← CLASH_* env overrides              [ephemeral]
```

Merging is key by key, so setting one key in `[terminal]` keeps the defaults for
the rest. Arrays (`[[ides]]`) replace wholesale. `clash config --show-effective`
annotates every value with the layer that won it.

### The project layer is restricted

A repo-local `.clash/config.toml` may set `[paths]`, plus the `[[actions]]` and
`[notifications.hooks]` sections reserved for later phases. Everything else is
dropped with a warning.

The limit exists because clash spawns processes: a cloned repo must not be able
to decide which binary runs. `project_layer_cannot_override_claude_bin` asserts
it, and the broader assertion is that *every* project-overridable property lives
under `[paths]`.

Discovery walks up from the current directory, git-style, so it works from a
subdirectory.

### Environment overrides

`CLASH_<SECTION>_<KEY>` maps to `<section>.<key>`, derived from the schema rather
than parsed — so only real settings are reachable and a typo is *reported*
instead of ignored:

```bash
CLASH_SESSIONS_REFRESH_SECS=5 clash
CLASH_PATHS_SCRATCH_DIR=/tmp/notes clash
CLASH_SESSIONS_CONFIRM_KILL=off clash    # 1/true/yes/on, 0/false/no/off
```

Values are parsed to the property's declared type, so `refresh_secs` arrives as
an integer rather than the string `"5"`.

`CLASH_LOG_RETENTION_HOURS` is read directly by the logger and is deliberately
exempt.

## Reading config at runtime

`ConfigHandle` is the only runtime accessor. It is cheap to clone and every
clone observes a reload:

```rust
let config = ConfigHandle::load();       // once, at startup
let settings = config.get();             // a snapshot, whenever you need one
let changed = config.reload();           // returns the dotted paths that moved
```

`get()` returns a clone rather than a guard on purpose: holding a read guard
across a refresh cycle would let a reload deadlock behind it.

This shape is load-bearing. `Config::load()` used to be an associated function
returning an owned value, called independently at five sites — so there was
nothing for a reload to update. If you find yourself wanting an owned `Config`
that outlives a single operation, take a `ConfigHandle` instead.

## Writes

Only the GUI writes config; the TUI is read-only over the file.

```rust
handle.set_json(&[("sessions.refresh_secs", json!(5))])?;  // validated per the schema
handle.reset_values(&["sessions.refresh_secs"])?;          // fall back to the default
```

Four properties hold, and each exists because its absence was a real bug:

1. **Unknown keys, comments and key order survive.** Writes edit the parsed
   `toml_edit` document, touching only the keys that changed. Re-serializing a
   struct would delete every key the running binary doesn't model — including
   one a *newer* clash wrote.
2. **A no-op write touches nothing.** Writing a key its current value returns an
   empty change list and does not rewrite the file, so a settings round-trip
   can't wake the FS watcher.
3. **An unreadable file blocks writes, by type.** `set_values` returns
   `ConfigWriteError::Blocked` while `ConfigState::error` is set, so no call site
   can forget to check and overwrite a file it failed to parse.
4. **Concurrent instances can't drop each other's change.** Several clash
   processes run by design (one daemon socket per pid). The whole
   read-modify-write runs under an advisory lock (`config.toml.lock`, `O_EXCL`
   with backoff and a stale age-out), and re-reads the file *inside* the lock.
   `write_atomic` alone makes a write whole, not serialized: without the lock,
   two instances both read version N and the later write silently drops the
   earlier one's key.

The lock is advisory and best-effort. If it can't be taken within ~2s the holder
is treated as dead and the lock is broken, with a warning. A lost setting is
bad; an unsaveable config is worse.

## Errors

`ConfigState` pairs the config with what went wrong loading it:

- `error: Option<ConfigError>` — the file could not be read or parsed. The
  config alongside it is the **last good** one (defaults on a first load), never
  a silent reset, and writes are blocked. `ConfigError::Parse` carries
  `line`/`column` so the TUI toast and the GUI banner can point at the typo.
- `issues: Vec<Issue>` — non-fatal: unknown keys, out-of-range values, rejected
  project overrides, typo'd `CLASH_*` variables. Loading proceeds.

An unknown key is a *warning*, never an error: a config written by a newer clash
must stay loadable by an older one, the same forward-compatibility stance the
domain types take with `#[serde(flatten)]`.

## Live reload

The config directory is a watch root in both frontends.

- **TUI** — a change routes to `App::reload_config`, which applies path changes
  to the backend and toasts what moved, naming any `restart_required` key rather
  than pretending it applied.
- **GUI** — `emit_config_reload` emits `config-changed` with
  `{ changed, refit, settings }`. The frontend applies xterm options only for
  keys that moved, and enters the refit path only for keys the schema marks
  `refit` (font family/size, line height, letter spacing) — coalesced to one
  animation frame. At a 200 ms debounce, an editor saving per keystroke would
  otherwise become a burst of refits across every pane.

Only `config.toml` itself triggers a reload. The advisory lock and
`write_atomic`'s temp file are siblings in the same directory, and reacting to
those would make every save reload itself.

Path comparison resolves the parent directory rather than trusting an exact
match: the watcher reports the path the OS resolved, so a config dir reached
through a symlink (a dotfile-managed or cloud-synced `Application Support`)
would otherwise never match and live reload would silently never fire. The same
applies to watch-root routing, which matches both the configured and the
canonical spelling of each root.

## Migrations

Two, both pure and idempotent, in `config/migrate.rs`.

### v1 → v2 (`schema_version`)

v1 was the flat file: `claude_bin`, `debounce_ms`, `claude_dir`, `scratch_dir`,
`workflows_dir` at the root. v2 namespaces them into `[general]` and `[paths]`.

Applied **in memory on every load**, so an un-upgraded file reads correctly, and
written back the next time something saves — an older binary keeps working
against the same file until then. A key present in both shapes keeps its v2
value. The move carries the key's decor, so a comment written above
`claude_bin` follows it into `[general]` instead of being eaten.

### The GUI settings blob

`migrate_gui_blob` is the single Rust entry point for settings validation. It
takes the `gui-state.json` settings blob and returns:

- `shared` — the 7 cross-frontend keys, coerced and validated, to write into
  `config.toml`;
- `gui_local` — the 21 GUI-local keys, validated and normalised, for the
  frontend to apply;
- `warnings` — per-key problems. A rejected value keeps its default, matching
  what the hand-written JS checks did.

Coercion goes through `coerce_json`, which is *also* what every live GUI write
uses — so a JS number lands as the right TOML type and every range check exists
in exactly one place. Out-of-range values are **rejected, not clamped**: a
nonsense value falls back to the default rather than being silently pulled to a
boundary.

Legacy fixups live here too: `embedLinks: true|false` → `linkOpen:
embedded|external`, applied only when `linkOpen` didn't already land. Losing
that would silently reset every user who set the boolean and never opened the
new dropdown.

### Why the blob can't resurrect a migrated key

Two things, together:

1. The frontend stops persisting shared keys in the blob (`guiLocalSettings()`),
   so a stale `gui-state.json` physically has nothing to resurrect.
2. Only the **disk** blob may seed the migration. WKWebView's `localStorage` is
   not `HOME`-isolated — an instance run under an isolated `HOME` can still read
   the real user's persisted blob — so `loadWorkspaces` passes
   `migratable: false` for the localStorage fallback and it is never allowed to
   write into `config.toml`.

The migration is idempotent regardless: keys already at their stored value are
not rewritten, so a repeat run leaves the file byte-identical.

## Testing

Everything decision-shaped is a pure function with unit tests beside it:
`schema.rs` (constraints, exports, the 28-key split), `layers.rs` (precedence,
provenance, restriction, env parsing), `migrate.rs` (both migrations,
idempotency, every clamp), `doc.rs` (comment and unknown-key preservation),
`lock.rs` (exclusion, stale age-out, forcing), and `mod.rs` (both latent bugs,
concurrent writers, reload diffing).

GUI-side invariants are asserted over the frontend source in
`gui/tests/app_source.test.js`, run by the same `node --test` CI gate as the
diff parser.
