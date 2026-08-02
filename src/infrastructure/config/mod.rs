//! Configuration — one namespaced, layered, schema-described file that both
//! frontends read.
//!
//! ## Shape
//!
//! ```text
//! defaults (from the schema)
//!   ← ~/.config/clash/config.toml        [user]
//!   ← <repo>/.clash/config.toml          [project — paths/actions/hooks only]
//!   ← CLASH_* env overrides              [ephemeral]
//! ```
//!
//! [`schema`] is the single source of truth for every property: its type,
//! range, default, doc string and frontend hints. [`layers`] merges the layers
//! and records provenance. [`migrate`] upgrades old shapes. [`doc`] and
//! [`lock`] are the write path.
//!
//! ## Two bugs this module exists to not have
//!
//! - **A save must not eat unknown keys.** With one file shared by two
//!   frontends, several instances and future versions, re-serializing a struct
//!   would silently delete any key the running binary doesn't model —
//!   including keys a *newer* clash wrote. Writes therefore edit the parsed
//!   document ([`doc`]), touching only the keys that changed.
//! - **A typo must not discard the file.** A parse error keeps the last good
//!   config in memory, surfaces a real error with `line:col`, and *blocks
//!   writes* so the next save cannot overwrite the user's file with defaults.
//!   That block is enforced by the type — [`ConfigHandle::set_values`] returns
//!   [`ConfigWriteError::Blocked`] while [`ConfigState::error`] is set — not by
//!   a convention every call site has to remember.
//!
//! ## One shared handle
//!
//! [`ConfigHandle`] is the only way to read config at runtime. It exists
//! because live reload needs something to update: `Config::load()` used to be
//! called independently at five sites, each holding its own owned copy, so
//! nothing could observe a reload.

/// Format-preserving document edits: the write path, which only the GUI
/// exercises, so the binary's private-`mod` build sees it as dead.
#[allow(dead_code)]
pub mod doc;
pub mod layers;
pub mod lock;
pub mod migrate;
pub mod schema;

use layers::{Layer, Source};
use schema::{Issue, Severity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

// ── Schema-backed defaults ──────────────────────────────────────────
//
// Reading defaults out of the schema rather than repeating them here is what
// keeps `Config::default()` and `--defaults` from drifting apart. A missing or
// mistyped path is a programming error, so these panic — and
// `default_matches_the_schema` below catches it in CI.

fn default_str(path: &str) -> String {
    match schema::prop(path).map(|p| p.default) {
        Some(schema::Val::Str(s)) => s.to_string(),
        other => panic!("{}: expected a string default, got {:?}", path, other),
    }
}

fn default_int(path: &str) -> i64 {
    match schema::prop(path).map(|p| p.default) {
        Some(schema::Val::Int(i)) => i,
        other => panic!("{}: expected an integer default, got {:?}", path, other),
    }
}

fn default_bool(path: &str) -> bool {
    match schema::prop(path).map(|p| p.default) {
        Some(schema::Val::Bool(b)) => b,
        other => panic!("{}: expected a boolean default, got {:?}", path, other),
    }
}

/// Treat `""` as "unset" for path properties, so a config that spells out
/// `scratch_dir = ""` means the same as omitting it.
fn empty_as_none<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from))
}

// ── The config model ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeEntry {
    pub command: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub terminal: bool,
}

/// `[general]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// The `claude` binary sessions are spawned with.
    pub claude_bin: String,
    /// Filesystem-watcher debounce.
    pub debounce_ms: u64,
}

impl Default for General {
    fn default() -> Self {
        Self {
            claude_bin: default_str("general.claude_bin"),
            debounce_ms: default_int("general.debounce_ms") as u64,
        }
    }
}

/// `[paths]` — every override is `None` when unset, meaning "compute it".
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Paths {
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub claude_dir: Option<PathBuf>,
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub scratch_dir: Option<PathBuf>,
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub workflows_dir: Option<PathBuf>,
}

/// `[sessions]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sessions {
    pub default_cwd: String,
    pub confirm_kill: bool,
    pub refresh_secs: u64,
}

impl Default for Sessions {
    fn default() -> Self {
        Self {
            default_cwd: default_str("sessions.default_cwd"),
            confirm_kill: default_bool("sessions.confirm_kill"),
            refresh_secs: default_int("sessions.refresh_secs") as u64,
        }
    }
}

/// `[terminal]` — only the cross-frontend keys. The 20 xterm-rendering keys
/// are GUI-local (plan Decision 2): the TUI can never apply them, so they stay
/// in the GUI's own store and are preserved here as unknown keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terminal {
    /// Shell for in-app terminals; empty means `$SHELL`.
    pub shell: String,
    /// Terminal emulator the TUI launcher uses; empty means auto-detect.
    pub tui_terminal: String,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            shell: default_str("terminal.shell"),
            tui_terminal: default_str("terminal.tui_terminal"),
        }
    }
}

/// `[notifications]`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    pub title_attention: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            enabled: default_bool("notifications.enabled"),
            title_attention: default_bool("notifications.title_attention"),
        }
    }
}

/// The effective configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: i64,
    pub general: General,
    pub paths: Paths,
    pub sessions: Sessions,
    pub terminal: Terminal,
    pub notifications: Notifications,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ides: Vec<IdeEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: schema::SCHEMA_VERSION,
            general: General::default(),
            paths: Paths::default(),
            sessions: Sessions::default(),
            terminal: Terminal::default(),
            notifications: Notifications::default(),
            ides: Vec::new(),
        }
    }
}

impl Config {
    /// Canonical config-file location: `<config_dir>/clash/config.toml`.
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clash")
            .join("config.toml")
    }

    pub fn claude_dir(&self) -> PathBuf {
        self.paths.claude_dir.clone().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude")
        })
    }

    /// Effective scratch-notes directory: the configured override, or the
    /// default `<claude_dir>/clash/scratch`.
    pub fn scratch_dir(&self) -> PathBuf {
        self.paths
            .scratch_dir
            .clone()
            .unwrap_or_else(|| self.claude_dir().join("clash").join("scratch"))
    }

    /// Effective workflow-items directory: the configured override, or the
    /// default `<claude_dir>/clash/workflows`. A dedicated root, independent
    /// of the scratch tree — scratches are free-form notes, workflows are a
    /// structured store.
    pub fn workflows_dir(&self) -> PathBuf {
        self.paths
            .workflows_dir
            .clone()
            .unwrap_or_else(|| self.claude_dir().join("clash").join("workflows"))
    }

    /// Watcher debounce as a `Duration`.
    pub fn debounce(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.general.debounce_ms)
    }

    /// Clash's own data directory for all RW state: `~/.claude/clash/`.
    ///
    /// Everything clash writes (status, names, hooks, tour marker) goes here,
    /// co-located with Claude Code's own data.
    pub fn clash_data_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude")
            .join("clash")
    }
}

// ── Errors ──────────────────────────────────────────────────────────

/// Why the config file could not be understood.
///
/// Carrying `line`/`column` is the point: the old behaviour logged a warning
/// nobody read and silently returned defaults, so a single typo looked exactly
/// like "I never had a config".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}:{line}:{column}: {message}")]
    Parse {
        path: PathBuf,
        message: String,
        line: usize,
        column: usize,
    },
    #[error("cannot read {path}: {message}")]
    Read { path: PathBuf, message: String },
}

impl ConfigError {
    /// A one-line summary for a TUI toast or a GUI banner.
    pub fn summary(&self) -> String {
        self.to_string()
    }
}

/// Why a write did not happen.
///
/// Only the GUI writes config (the TUI is read-only over `config.toml`), so the
/// binary's private-`mod` compilation sees this as dead — same reason the `gh`
/// module and the workflow port carry an `allow`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigWriteError {
    /// The file is unreadable, so writing would destroy it. This is the
    /// type-level form of the plan's "blocks writes until resolved".
    #[error("config not saved: {0} — fix the file first")]
    Blocked(ConfigError),
    /// The caller passed a value the schema rejects.
    #[error("{path}: {message}")]
    Invalid { path: String, message: String },
    #[error("cannot write {path}: {message}")]
    Io { path: PathBuf, message: String },
}

// ── Loaded state ────────────────────────────────────────────────────

/// A loaded config plus everything that went wrong loading it.
///
/// The plan's Issue 8 / D8: pairing the value with the error means no call site
/// can accidentally treat a failed load as an empty config, and `save()` can
/// refuse structurally.
#[derive(Debug, Clone)]
pub struct ConfigState {
    /// The effective config. On a parse error this is the **last good** config
    /// (or the defaults on a first load), never a silently reset one.
    pub config: Config,
    /// Set when the user's file could not be read or parsed. While this is
    /// `Some`, writes are refused.
    pub error: Option<ConfigError>,
    /// Non-fatal problems: unknown keys, out-of-range values, rejected
    /// project-layer overrides, typo'd `CLASH_*` variables.
    pub issues: Vec<Issue>,
    /// Which layer won each key, for `--show-effective`.
    pub provenance: BTreeMap<String, Source>,
    /// The merged table, kept for `--show-effective` and reload diffing.
    merged: toml::Table,
    config_path: PathBuf,
    project_path: Option<PathBuf>,
}

impl ConfigState {
    /// Every effective setting with its provenance, as an annotated TOML
    /// document — `clash config --show-effective`.
    ///
    /// The Ghostty `+show-config` idea: answers "why is this setting not
    /// applying" by naming the layer each value came from.
    pub fn effective_toml(&self) -> String {
        let mut out = String::new();
        out.push_str("# clash effective configuration (merged across all layers).\n");
        out.push_str(&format!("# user:    {}\n", self.config_path.display()));
        match &self.project_path {
            Some(p) => out.push_str(&format!("# project: {}\n", p.display())),
            None => out.push_str("# project: (none found)\n"),
        }
        out.push_str("# Each value is annotated with the layer it came from.\n\n");
        out.push_str(&format!(
            "schema_version = {}\n",
            self.config.schema_version
        ));

        let mut by_section: BTreeMap<&str, Vec<&schema::Prop>> = BTreeMap::new();
        for p in schema::shared_props() {
            by_section.entry(p.section()).or_default().push(p);
        }
        for (section, props) in by_section {
            out.push_str(&format!("\n[{}]\n", section));
            for p in props {
                let value = layers::get_path(&self.merged, p.path)
                    .cloned()
                    .unwrap_or_else(|| p.default.to_toml());
                let source = self
                    .provenance
                    .get(p.path)
                    .copied()
                    .unwrap_or(Source::Default);
                out.push_str(&format!(
                    "{} = {}  # {}\n",
                    p.leaf(),
                    render_scalar(&value),
                    source.label()
                ));
            }
        }
        for ide in &self.config.ides {
            out.push_str("\n[[ides]]\n");
            out.push_str(&format!("name = {:?}\n", ide.name));
            out.push_str(&format!("command = {:?}\n", ide.command));
            if !ide.description.is_empty() {
                out.push_str(&format!("description = {:?}\n", ide.description));
            }
            out.push_str(&format!("terminal = {}\n", ide.terminal));
        }
        out
    }
}

fn render_scalar(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => format!("{:?}", s),
        toml::Value::Float(f) if f.fract() == 0.0 => format!("{:.1}", f),
        other => other.to_string(),
    }
}

// ── The shared handle ───────────────────────────────────────────────

/// The one live view of configuration, cheap to clone and safe to share.
///
/// Finding 2: `Config::load()` was an associated function returning an owned
/// value, called independently at five sites — so there was nothing for a
/// reload to update. Everything now reads through a clone of this handle.
#[derive(Clone)]
pub struct ConfigHandle {
    inner: Arc<RwLock<ConfigState>>,
}

impl ConfigHandle {
    /// Load from the canonical location, discovering a project layer from the
    /// current working directory.
    pub fn load() -> Self {
        let project = std::env::current_dir()
            .ok()
            .and_then(|cwd| discover_project_config(&cwd));
        Self::load_from(Config::config_path(), project)
    }

    /// Load from explicit paths. The seam every test uses.
    pub fn load_from(config_path: PathBuf, project_path: Option<PathBuf>) -> Self {
        let state = read_state(config_path, project_path, None);
        Self {
            inner: Arc::new(RwLock::new(state)),
        }
    }

    /// A snapshot of the effective config.
    ///
    /// Cloned rather than borrowed on purpose: holding a read guard across a
    /// refresh cycle would let a reload deadlock behind it.
    pub fn get(&self) -> Config {
        self.read().config.clone()
    }

    /// The load error, if the file is currently unreadable.
    pub fn error(&self) -> Option<ConfigError> {
        self.read().error.clone()
    }

    /// Non-fatal problems worth surfacing (unknown keys, bad values, rejected
    /// project overrides).
    pub fn issues(&self) -> Vec<Issue> {
        self.read().issues.clone()
    }

    pub fn config_path(&self) -> PathBuf {
        self.read().config_path.clone()
    }

    /// `clash config --show-effective`.
    pub fn effective_toml(&self) -> String {
        self.read().effective_toml()
    }

    /// The directory to watch for live reload — the config file's parent.
    pub fn watch_dir(&self) -> PathBuf {
        self.read()
            .config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Whether a path reported by the watcher is the config file itself.
    ///
    /// The lock file and `write_atomic`'s temp file live in the same directory;
    /// reacting to those would make every save reload itself.
    ///
    /// The comparison resolves the *parent* directory rather than trusting an
    /// exact match: the watcher reports the path FSEvents/inotify resolved, so a
    /// config dir reached through a symlink (a dotfile-managed or cloud-synced
    /// `Application Support`, say) never matches the configured spelling and live
    /// reload silently never fires. The parent always exists — `write_atomic`
    /// renames into place — so canonicalizing it is safe, unlike canonicalizing
    /// the file itself.
    pub fn is_config_path(&self, path: &Path) -> bool {
        let own = self.read().config_path.clone();
        if path == own {
            return true;
        }
        if path.file_name() != own.file_name() {
            return false;
        }
        match (path.parent(), own.parent()) {
            (Some(reported), Some(configured)) => {
                reported == configured
                    || reported
                        .canonicalize()
                        .ok()
                        .zip(configured.canonicalize().ok())
                        .map(|(a, b)| a == b)
                        .unwrap_or(false)
            }
            _ => false,
        }
    }

    /// Re-read every layer and swap the result in.
    ///
    /// Returns the dotted paths whose effective value changed, which is what
    /// drives the coalesced reload fan-out: the GUI applies xterm options only
    /// for changed keys and refits only when one of them is marked `x-refit`
    /// (plan Issue 14 / D12).
    ///
    /// A parse error keeps the previous good config and records the error, so a
    /// half-saved file never blanks a running instance's settings.
    pub fn reload(&self) -> Vec<String> {
        let mut guard = self.write();
        match read_document(&guard.config_path) {
            Ok(document) => {
                let next = build_state(
                    guard.config_path.clone(),
                    guard.project_path.clone(),
                    document,
                    None,
                    Some(&guard.config),
                );
                let changed = changed_paths(&guard.merged, &next.merged);
                *guard = next;
                changed
            }
            Err(error) => {
                // Keep every value already in memory and only record the error.
                // Rebuilding from an empty document would merge down to the
                // defaults — i.e. exactly the silent reset this module exists to
                // prevent, just deferred to the first save after a typo.
                tracing::error!("{}", error);
                guard.error = Some(error);
                Vec::new()
            }
        }
    }

    /// Write settings, touching only the keys that actually change.
    ///
    /// The whole read-modify-write runs under an advisory lock and re-reads the
    /// file first, so a concurrent instance's edit to a *different* key
    /// survives (Finding 6 + Issue 5). Returns the paths that changed; an empty
    /// result means the file was not touched at all, which keeps a no-op save
    /// from waking the FS watcher.
    #[allow(dead_code)] // GUI-only write path; see ConfigWriteError.
    pub fn set_values(
        &self,
        edits: &[(&str, toml::Value)],
    ) -> Result<Vec<String>, ConfigWriteError> {
        self.edit(edits.iter().map(|(p, v)| (*p, Some(v.clone()))).collect())
    }

    /// Write settings given as JSON, the frontend's native representation.
    ///
    /// Coercion goes through [`migrate::coerce_json`] — the same function the
    /// one-shot blob migration uses — so a JS number lands as the right TOML
    /// type and every range check is applied in exactly one place. This is what
    /// keeps the GUI from needing its own validator (and from needing to speak
    /// TOML at all).
    #[allow(dead_code)] // GUI-only write path; see ConfigWriteError.
    pub fn set_json(
        &self,
        edits: &[(&str, serde_json::Value)],
    ) -> Result<Vec<String>, ConfigWriteError> {
        let mut coerced = Vec::with_capacity(edits.len());
        for (path, value) in edits {
            let prop = schema::prop(path).ok_or_else(|| ConfigWriteError::Invalid {
                path: path.to_string(),
                message: "no such setting".to_string(),
            })?;
            let value =
                migrate::coerce_json(prop, value).map_err(|message| ConfigWriteError::Invalid {
                    path: path.to_string(),
                    message,
                })?;
            coerced.push((*path, value));
        }
        self.set_values(&coerced)
    }

    /// Write the shared half of a one-shot GUI-blob migration.
    ///
    /// Values are already coerced and validated by
    /// [`migrate::migrate_gui_blob`], so this is the only write path that skips
    /// re-coercion — and it exists so the GUI never has to construct a
    /// `toml::Value` itself.
    #[allow(dead_code)] // GUI-only write path; see ConfigWriteError.
    pub fn apply_gui_migration(
        &self,
        migration: &migrate::GuiMigration,
    ) -> Result<Vec<String>, ConfigWriteError> {
        let edits: Vec<(&str, toml::Value)> = migration
            .shared
            .iter()
            .map(|(path, value)| (path.as_str(), value.clone()))
            .collect();
        if edits.is_empty() {
            return Ok(Vec::new());
        }
        self.set_values(&edits)
    }

    /// Reset settings to their defaults by removing them from the user layer,
    /// so the value falls back through the remaining layers.
    #[allow(dead_code)] // GUI-only write path; see ConfigWriteError.
    pub fn reset_values(&self, paths: &[&str]) -> Result<Vec<String>, ConfigWriteError> {
        self.edit(paths.iter().map(|p| (*p, None)).collect())
    }

    /// Shared write path. `None` means "remove this key".
    #[allow(dead_code)] // GUI-only write path; see ConfigWriteError.
    fn edit(
        &self,
        edits: Vec<(&str, Option<toml::Value>)>,
    ) -> Result<Vec<String>, ConfigWriteError> {
        // Validate before taking any lock — a bad value is a caller bug and
        // should not disturb another instance's write.
        for (path, value) in &edits {
            let Some(prop) = schema::prop(path) else {
                return Err(ConfigWriteError::Invalid {
                    path: path.to_string(),
                    message: "no such setting".to_string(),
                });
            };
            if prop.scope != schema::Scope::Shared {
                return Err(ConfigWriteError::Invalid {
                    path: path.to_string(),
                    message: "GUI-local setting; it does not live in config.toml".to_string(),
                });
            }
            if let Some(value) = value {
                if let Some(message) = schema::check_value(prop, value) {
                    return Err(ConfigWriteError::Invalid {
                        path: path.to_string(),
                        message,
                    });
                }
            }
        }

        let (config_path, project_path, blocked) = {
            let state = self.read();
            (
                state.config_path.clone(),
                state.project_path.clone(),
                state.error.clone(),
            )
        };
        if let Some(error) = blocked {
            return Err(ConfigWriteError::Blocked(error));
        }

        let (_lock, forced) = lock::ConfigLock::acquire(&config_path);
        if forced {
            tracing::warn!(
                "config lock was held by a dead or wedged process; broke it to save {}",
                config_path.display()
            );
        }

        // Re-read *under the lock*: this is what makes a concurrent edit to a
        // different key survive instead of being clobbered by our snapshot.
        let mut document = match read_document(&config_path) {
            Ok(document) => document,
            Err(error) => return Err(ConfigWriteError::Blocked(error)),
        };
        migrate::migrate_document(&mut document);

        let mut changed = Vec::new();
        for (path, value) in &edits {
            match value {
                Some(value) => {
                    let already = doc::get(&document, path)
                        .and_then(|i| i.as_value())
                        .map(|v| value_eq(v, value))
                        .unwrap_or(false);
                    if already {
                        continue;
                    }
                    let Some(edit) = doc::scalar(value) else {
                        return Err(ConfigWriteError::Invalid {
                            path: path.to_string(),
                            message: "only scalar settings can be written".to_string(),
                        });
                    };
                    doc::set(&mut document, path, edit);
                    changed.push(path.to_string());
                }
                None => {
                    if doc::remove(&mut document, path) {
                        changed.push(path.to_string());
                    }
                }
            }
        }

        if changed.is_empty() {
            return Ok(changed);
        }

        let text = document.to_string();
        crate::infrastructure::fs::atomic::write_atomic(&config_path, text.as_bytes()).map_err(
            |e| ConfigWriteError::Io {
                path: config_path.clone(),
                message: e.to_string(),
            },
        )?;

        // Rebuild from the document we just wrote rather than re-reading the
        // file: the value is already in hand, and a watcher-driven reload will
        // land shortly anyway.
        let mut guard = self.write();
        let previous = guard.config.clone();
        *guard = build_state(config_path, project_path, document, None, Some(&previous));
        Ok(changed)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, ConfigState> {
        // A poisoned lock means another thread panicked mid-update. The state
        // is still structurally valid (we only ever swap whole states), so
        // recovering beats taking the process down over a settings read.
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, ConfigState> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }
}

impl std::fmt::Debug for ConfigHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigHandle")
            .field("path", &self.config_path())
            .finish()
    }
}

/// Compare a `toml_edit` value with a `toml` one, so an unchanged setting can
/// be skipped without rewriting the file.
#[allow(dead_code)] // Reached only from the GUI-only write path.
fn value_eq(left: &toml_edit::Value, right: &toml::Value) -> bool {
    match (left, right) {
        (toml_edit::Value::String(a), toml::Value::String(b)) => a.value() == b,
        (toml_edit::Value::Integer(a), toml::Value::Integer(b)) => a.value() == b,
        (toml_edit::Value::Float(a), toml::Value::Float(b)) => a.value() == b,
        // `line_height = 1` and `1.0` are the same setting.
        (toml_edit::Value::Integer(a), toml::Value::Float(b)) => (*a.value() as f64) == *b,
        (toml_edit::Value::Float(a), toml::Value::Integer(b)) => *a.value() == (*b as f64),
        (toml_edit::Value::Boolean(a), toml::Value::Boolean(b)) => a.value() == b,
        _ => false,
    }
}

/// Dotted paths whose value differs between two merged tables (in either
/// direction, so a removed key counts as changed).
fn changed_paths(before: &toml::Table, after: &toml::Table) -> Vec<String> {
    let mut changed = Vec::new();
    let old: BTreeMap<String, &toml::Value> = layers::leaves(before).into_iter().collect();
    let new: BTreeMap<String, &toml::Value> = layers::leaves(after).into_iter().collect();
    for (path, value) in &new {
        if old.get(path) != Some(value) {
            changed.push(path.clone());
        }
    }
    for path in old.keys() {
        if !new.contains_key(path) {
            changed.push(path.clone());
        }
    }
    changed.sort();
    changed.dedup();
    changed
}

/// Walk up from `start` looking for `.clash/config.toml`, git-style.
///
/// The project layer belongs to the repo clash was launched in; walking up
/// means it works from a subdirectory too.
pub fn discover_project_config(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        let candidate = current.join(".clash").join("config.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = current.parent();
    }
    None
}

/// Read a user-layer document from disk. A missing file is an empty document,
/// not an error — that is the normal state for most installs.
fn read_document(path: &Path) -> Result<toml_edit::DocumentMut, ConfigError> {
    if !path.exists() {
        return Ok(toml_edit::DocumentMut::new());
    }
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Read {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|e| parse_error(path, &text, e.span(), e.message()))
}

fn parse_error(
    path: &Path,
    text: &str,
    span: Option<std::ops::Range<usize>>,
    message: &str,
) -> ConfigError {
    let (line, column) = span.map(|s| line_col(text, s.start)).unwrap_or((1, 1));
    ConfigError::Parse {
        path: path.to_path_buf(),
        message: message.trim().to_string(),
        line,
        column,
    }
}

/// 1-based line and column for a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let clamped = offset.min(text.len());
    let before = &text[..clamped];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rfind('\n')
        .map(|i| clamped - i)
        .unwrap_or(clamped + 1);
    (line, column)
}

/// Read every layer and assemble a state.
fn read_state(
    config_path: PathBuf,
    project_path: Option<PathBuf>,
    previous: Option<&Config>,
) -> ConfigState {
    match read_document(&config_path) {
        Ok(document) => build_state(config_path, project_path, document, None, previous),
        Err(error) => {
            // The bug this replaces: warn, return defaults, and let the next
            // save overwrite the user's file. Keep the last good config, keep
            // the file, and refuse to write until it parses.
            tracing::error!("{}", error);
            build_state(
                config_path,
                project_path,
                toml_edit::DocumentMut::new(),
                Some(error),
                previous,
            )
        }
    }
}

/// Merge the layers around an already-parsed user document.
fn build_state(
    config_path: PathBuf,
    project_path: Option<PathBuf>,
    document: toml_edit::DocumentMut,
    error: Option<ConfigError>,
    previous: Option<&Config>,
) -> ConfigState {
    let mut issues = Vec::new();

    // User layer, migrated in memory so an un-upgraded file still reads.
    let mut user: toml::Table = toml::from_str(&document.to_string()).unwrap_or_default();
    migrate::migrate_table(&mut user);
    issues.extend(schema::validate_table(&user));

    // Project layer, restricted.
    let mut project = toml::Table::new();
    if let Some(path) = &project_path {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<toml::Table>(&text) {
                Ok(mut table) => {
                    migrate::migrate_table(&mut table);
                    let (kept, rejected) = layers::restrict_project_layer(&table);
                    project = kept;
                    issues.extend(rejected);
                }
                Err(e) => issues.push(Issue {
                    path: path.display().to_string(),
                    message: format!("project config ignored: {}", e.message().trim()),
                    severity: Severity::Error,
                }),
            },
            Err(e) => issues.push(Issue {
                path: path.display().to_string(),
                message: format!("project config unreadable: {}", e),
                severity: Severity::Warning,
            }),
        }
    }

    // Env layer.
    let (env, env_issues) = layers::env_layer(std::env::vars());
    issues.extend(env_issues);

    let merged = layers::merge(&[
        Layer {
            source: Source::Default,
            table: schema::defaults_table(),
        },
        Layer {
            source: Source::User,
            table: user,
        },
        Layer {
            source: Source::Project,
            table: project,
        },
        Layer {
            source: Source::Env,
            table: env,
        },
    ]);

    // A merged table that fails to deserialize means a value survived
    // validation but not the model (e.g. a negative debounce). Keep the
    // previous good config rather than resetting to defaults.
    let config = match toml::Value::Table(merged.table.clone()).try_into::<Config>() {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(
                "config did not fit the model, keeping the previous values: {}",
                e
            );
            previous.cloned().unwrap_or_default()
        }
    };

    issues.sort_by(|a, b| a.path.cmp(&b.path));
    ConfigState {
        config,
        error,
        issues,
        provenance: merged.provenance,
        merged: merged.table,
        config_path,
        project_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A handle over a throwaway directory, with no project layer.
    fn handle_in(dir: &TempDir, contents: Option<&str>) -> (ConfigHandle, PathBuf) {
        let path = dir.path().join("config.toml");
        if let Some(text) = contents {
            std::fs::write(&path, text).unwrap();
        }
        (ConfigHandle::load_from(path.clone(), None), path)
    }

    // ── Defaults ────────────────────────────────────────────────────

    /// `Config::default()` and the schema must agree, or `--defaults` documents
    /// values the binary doesn't actually use.
    #[test]
    fn default_matches_the_schema() {
        let from_schema: Config = toml::Value::Table(schema::defaults_table())
            .try_into()
            .expect("defaults table must fit Config");
        assert_eq!(Config::default(), from_schema);
    }

    #[test]
    fn missing_file_yields_defaults_without_an_error() {
        let dir = TempDir::new().unwrap();
        let (handle, _) = handle_in(&dir, None);
        assert_eq!(handle.get(), Config::default());
        assert!(handle.error().is_none());
    }

    // ── Path resolution (behaviour preserved from the flat config) ───

    #[test]
    fn scratch_dir_defaults_under_claude_dir() {
        let config = Config {
            paths: Paths {
                claude_dir: Some(PathBuf::from("/tmp/fake-claude")),
                ..Paths::default()
            },
            ..Config::default()
        };
        assert_eq!(
            config.scratch_dir(),
            PathBuf::from("/tmp/fake-claude/clash/scratch")
        );
    }

    #[test]
    fn scratch_dir_honors_override() {
        let config = Config {
            paths: Paths {
                claude_dir: Some(PathBuf::from("/tmp/fake-claude")),
                scratch_dir: Some(PathBuf::from("/tmp/elsewhere/notes")),
                ..Paths::default()
            },
            ..Config::default()
        };
        assert_eq!(config.scratch_dir(), PathBuf::from("/tmp/elsewhere/notes"));
    }

    /// Scratches and workflows are separate stores: overriding the scratch dir
    /// must not move the workflows root.
    #[test]
    fn workflows_dir_independent_of_scratch_override() {
        let config = Config {
            paths: Paths {
                claude_dir: Some(PathBuf::from("/tmp/fake-claude")),
                scratch_dir: Some(PathBuf::from("/tmp/elsewhere/notes")),
                ..Paths::default()
            },
            ..Config::default()
        };
        assert_eq!(
            config.workflows_dir(),
            PathBuf::from("/tmp/fake-claude/clash/workflows")
        );
    }

    #[test]
    fn an_empty_path_string_means_unset() {
        let dir = TempDir::new().unwrap();
        let (handle, _) = handle_in(
            &dir,
            Some("[paths]\nscratch_dir = \"\"\nclaude_dir = \"/tmp/c\"\n"),
        );
        let config = handle.get();
        assert_eq!(config.paths.scratch_dir, None);
        assert_eq!(config.scratch_dir(), PathBuf::from("/tmp/c/clash/scratch"));
    }

    // ── The two latent bugs ─────────────────────────────────────────

    /// The silent-reset bug: a typo used to log a warning, return defaults, and
    /// let the next save overwrite the file.
    #[test]
    fn a_parse_error_keeps_defaults_reports_line_col_and_blocks_writes() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(&dir, Some("[sessions]\nrefresh_secs = \n"));

        let error = handle.error().expect("a parse error must surface");
        match &error {
            ConfigError::Parse { line, .. } => assert_eq!(*line, 2),
            other => panic!("expected a parse error, got {:?}", other),
        }
        assert!(error.summary().contains("config.toml:2:"), "{}", error);

        // Writes are refused by type, not by convention.
        let result = handle.set_values(&[("sessions.refresh_secs", toml::Value::Integer(5))]);
        assert!(
            matches!(result, Err(ConfigWriteError::Blocked(_))),
            "{:?}",
            result
        );

        // And the user's file is untouched — the whole point of blocking.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[sessions]\nrefresh_secs = \n"
        );
    }

    /// A reload over a now-broken file must keep the values already in memory,
    /// not blank a running instance's settings.
    #[test]
    fn a_reload_onto_a_broken_file_keeps_the_last_good_config() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(&dir, Some("[sessions]\nrefresh_secs = 9\n"));
        assert_eq!(handle.get().sessions.refresh_secs, 9);

        std::fs::write(&path, "[sessions]\nrefresh_secs = ").unwrap();
        handle.reload();

        assert!(handle.error().is_some());
        assert_eq!(
            handle.get().sessions.refresh_secs,
            9,
            "the last good value must survive a broken reload"
        );

        // Fixing the file clears the error and unblocks writes.
        std::fs::write(&path, "[sessions]\nrefresh_secs = 4\n").unwrap();
        handle.reload();
        assert!(handle.error().is_none());
        assert_eq!(handle.get().sessions.refresh_secs, 4);
        assert!(handle
            .set_values(&[("sessions.refresh_secs", toml::Value::Integer(6))])
            .is_ok());
    }

    /// The clobber bug: `save()` used to re-serialize a struct, deleting every
    /// key it didn't model — including keys a newer clash wrote.
    #[test]
    fn a_save_preserves_unknown_keys_comments_and_future_sections() {
        let dir = TempDir::new().unwrap();
        let original = r#"# my config
schema_version = 2

[sessions]
refresh_secs = 2
# a key this build has never heard of
from_the_future = "keep me"

[keymap]
"session.reload" = "cmd+r"

[[ides]]
name = "VS Code"
command = "code"
"#;
        let (handle, path) = handle_in(&dir, Some(original));
        handle
            .set_values(&[("sessions.refresh_secs", toml::Value::Integer(7))])
            .unwrap();

        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("refresh_secs = 7"), "{}", saved);
        assert!(saved.contains("# my config"), "{}", saved);
        assert!(saved.contains("from_the_future = \"keep me\""), "{}", saved);
        assert!(saved.contains("[keymap]"), "{}", saved);
        assert!(saved.contains("\"session.reload\""), "{}", saved);
        assert!(saved.contains("[[ides]]"), "{}", saved);
    }

    /// Downgrade safety, stated as its own requirement in the plan's testing
    /// section: a config carrying future keys survives a save by this build.
    #[test]
    fn downgrade_safety_a_newer_schema_version_survives_a_save() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(
            &dir,
            Some("schema_version = 99\n[sessions]\nrefresh_secs = 3\nunknown_thing = true\n"),
        );
        assert!(handle.error().is_none());
        handle
            .set_values(&[("sessions.confirm_kill", toml::Value::Boolean(false))])
            .unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("schema_version = 99"), "{}", saved);
        assert!(saved.contains("unknown_thing = true"), "{}", saved);
        assert!(saved.contains("confirm_kill = false"), "{}", saved);
    }

    // ── Writes ──────────────────────────────────────────────────────

    #[test]
    fn set_values_writes_only_what_changed() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(&dir, Some("[sessions]\nrefresh_secs = 5\n"));

        // Writing the value it already has touches nothing.
        let changed = handle
            .set_values(&[("sessions.refresh_secs", toml::Value::Integer(5))])
            .unwrap();
        assert!(changed.is_empty(), "a no-op save must not rewrite the file");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[sessions]\nrefresh_secs = 5\n"
        );

        let changed = handle
            .set_values(&[("sessions.refresh_secs", toml::Value::Integer(6))])
            .unwrap();
        assert_eq!(changed, vec!["sessions.refresh_secs"]);
        assert_eq!(handle.get().sessions.refresh_secs, 6);
    }

    #[test]
    fn set_values_rejects_values_the_schema_refuses() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(&dir, Some(""));

        let out_of_range = handle.set_values(&[("sessions.refresh_secs", toml::Value::Integer(0))]);
        assert!(matches!(
            out_of_range,
            Err(ConfigWriteError::Invalid { .. })
        ));

        let unknown = handle.set_values(&[("sessions.nope", toml::Value::Integer(1))]);
        assert!(matches!(unknown, Err(ConfigWriteError::Invalid { .. })));

        // GUI-local settings do not belong in config.toml at all.
        let local = handle.set_values(&[("terminal.font_size", toml::Value::Integer(14))]);
        assert!(
            matches!(local, Err(ConfigWriteError::Invalid { .. })),
            "{:?}",
            local
        );

        // None of the rejects touched the file.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn reset_values_removes_the_key_so_the_default_comes_back() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(&dir, Some("[sessions]\nrefresh_secs = 9\n"));
        assert_eq!(handle.get().sessions.refresh_secs, 9);

        let changed = handle.reset_values(&["sessions.refresh_secs"]).unwrap();
        assert_eq!(changed, vec!["sessions.refresh_secs"]);
        assert_eq!(handle.get().sessions.refresh_secs, 2);
        // The emptied section is pruned rather than left as a `[sessions]` stub.
        // What remains is the version stamp the save added.
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("refresh_secs"), "{}", saved);
        assert!(!saved.contains("[sessions]"), "{}", saved);

        // Resetting an already-default key is a no-op.
        assert!(handle
            .reset_values(&["sessions.refresh_secs"])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_v1_flat_file_is_migrated_on_load_and_on_save() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(
            &dir,
            Some("# hand written\nclaude_bin = \"/opt/claude\"\ndebounce_ms = 350\n"),
        );
        // Read through the new shape without touching the file.
        assert_eq!(handle.get().general.claude_bin, "/opt/claude");
        assert_eq!(handle.get().general.debounce_ms, 350);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("claude_bin = \"/opt/claude\""));

        handle
            .set_values(&[("sessions.confirm_kill", toml::Value::Boolean(false))])
            .unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("[general]"), "{}", saved);
        assert!(saved.contains("# hand written"), "{}", saved);
        assert!(saved.contains("schema_version = 2"), "{}", saved);
        assert_eq!(handle.get().general.claude_bin, "/opt/claude");
    }

    // ── Concurrency (plan Issue 5 / D5) ─────────────────────────────

    /// Two writers each flipping a *different* key must both survive. Without
    /// the lock + re-read, the later write's stale snapshot drops the earlier
    /// one's key.
    #[test]
    fn concurrent_writers_on_different_keys_both_survive() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();

        let rounds = 40;
        let a = std::thread::spawn({
            let path = path.clone();
            move || {
                let handle = ConfigHandle::load_from(path, None);
                for i in 0..rounds {
                    handle
                        .set_values(&[(
                            "sessions.refresh_secs",
                            toml::Value::Integer(1 + (i % 30)),
                        )])
                        .expect("writer A");
                }
            }
        });
        let b = std::thread::spawn({
            let path = path.clone();
            move || {
                let handle = ConfigHandle::load_from(path, None);
                for i in 0..rounds {
                    handle
                        .set_values(&[("notifications.enabled", toml::Value::Boolean(i % 2 == 0))])
                        .expect("writer B");
                }
            }
        });
        a.join().unwrap();
        b.join().unwrap();

        // Both keys are present in the final file — neither writer erased the
        // other's setting.
        let reloaded = ConfigHandle::load_from(path.clone(), None);
        assert!(reloaded.error().is_none());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("refresh_secs"),
            "writer A's key is gone:\n{}",
            text
        );
        assert!(
            text.contains("enabled"),
            "writer B's key is gone:\n{}",
            text
        );
    }

    // ── Layers ──────────────────────────────────────────────────────

    #[test]
    fn a_project_layer_overrides_paths_and_nothing_else() {
        let dir = TempDir::new().unwrap();
        let user = dir.path().join("config.toml");
        std::fs::write(
            &user,
            "[paths]\nscratch_dir = \"/user/notes\"\n[general]\nclaude_bin = \"user-claude\"\n",
        )
        .unwrap();

        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".clash")).unwrap();
        let project = repo.join(".clash").join("config.toml");
        std::fs::write(
            &project,
            "[paths]\nscratch_dir = \"/repo/notes\"\n[general]\nclaude_bin = \"/tmp/evil\"\n",
        )
        .unwrap();

        let handle = ConfigHandle::load_from(user, Some(project));
        let config = handle.get();
        assert_eq!(config.paths.scratch_dir, Some(PathBuf::from("/repo/notes")));
        // The blast-radius limit: a cloned repo cannot change the binary.
        assert_eq!(config.general.claude_bin, "user-claude");
        assert!(handle
            .issues()
            .iter()
            .any(|i| i.path == "general.claude_bin" && i.severity == Severity::Warning));
    }

    #[test]
    fn discover_project_config_walks_up_from_a_subdirectory() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        let deep = repo.join("src").join("nested");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(repo.join(".clash")).unwrap();
        let expected = repo.join(".clash").join("config.toml");
        std::fs::write(&expected, "").unwrap();

        assert_eq!(discover_project_config(&deep), Some(expected));
    }

    // ── Reload diffing ──────────────────────────────────────────────

    #[test]
    fn reload_reports_exactly_the_changed_paths() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(&dir, Some("[sessions]\nrefresh_secs = 2\n"));

        assert!(
            handle.reload().is_empty(),
            "an unchanged file changes nothing"
        );

        std::fs::write(
            &path,
            "[sessions]\nrefresh_secs = 8\n[notifications]\nenabled = false\n",
        )
        .unwrap();
        let changed = handle.reload();
        assert_eq!(
            changed,
            vec!["notifications.enabled", "sessions.refresh_secs"]
        );
        assert_eq!(handle.get().sessions.refresh_secs, 8);
        assert!(!handle.get().notifications.enabled);

        // Removing a key is a change too — it falls back to the default.
        std::fs::write(&path, "[sessions]\nrefresh_secs = 8\n").unwrap();
        assert_eq!(handle.reload(), vec!["notifications.enabled"]);
        assert!(handle.get().notifications.enabled);
    }

    /// The reload fan-out only refits when a changed key is marked `x-refit`
    /// (plan Issue 14 / D12) — this pins the classification the GUI relies on.
    #[test]
    fn changed_paths_can_be_classified_for_the_refit_fan_out() {
        let needs_refit = |paths: &[&str]| {
            paths
                .iter()
                .filter_map(|p| schema::prop(p))
                .any(|p| p.refit)
        };
        assert!(needs_refit(&["terminal.font_size"]));
        assert!(!needs_refit(&[
            "sessions.refresh_secs",
            "notifications.enabled"
        ]));
    }

    #[test]
    fn a_shared_handle_is_observed_by_every_clone() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(&dir, Some("[sessions]\nrefresh_secs = 2\n"));
        let other = handle.clone();

        std::fs::write(&path, "[sessions]\nrefresh_secs = 11\n").unwrap();
        handle.reload();

        // This is what an owned `Config` per call site could never do.
        assert_eq!(other.get().sessions.refresh_secs, 11);
    }

    #[test]
    fn is_config_path_ignores_siblings_like_the_lock_and_temp_files() {
        let dir = TempDir::new().unwrap();
        let (handle, path) = handle_in(&dir, Some(""));
        assert!(handle.is_config_path(&path));
        assert!(!handle.is_config_path(&dir.path().join("config.toml.lock")));
        assert!(!handle.is_config_path(&dir.path().join(".clash-tmp-1-2")));
        assert!(!handle.is_config_path(&dir.path().join("sub").join("config.toml")));
    }

    /// The watcher reports the path the OS resolved. A config dir reached through
    /// a symlink must still be recognised, or live reload silently never fires —
    /// which is exactly how it failed on a symlinked `Application Support`.
    #[test]
    fn is_config_path_matches_through_a_symlinked_directory() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("real");
        let link = dir.path().join("link");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        std::fs::write(real.join("config.toml"), "").unwrap();

        // Configured through the symlink…
        let handle = ConfigHandle::load_from(link.join("config.toml"), None);
        assert!(handle.is_config_path(&link.join("config.toml")));
        // …and reported under the resolved path.
        let resolved = real.canonicalize().unwrap();
        assert!(handle.is_config_path(&resolved.join("config.toml")));
        // A sibling under the resolved path is still not the config file.
        assert!(!handle.is_config_path(&resolved.join("config.toml.lock")));
    }

    // ── Reporting ───────────────────────────────────────────────────

    #[test]
    fn effective_toml_annotates_provenance_and_parses_back() {
        let dir = TempDir::new().unwrap();
        let (handle, _) = handle_in(&dir, Some("[sessions]\nrefresh_secs = 7\n"));
        let text = handle.effective_toml();

        assert!(text.contains("refresh_secs = 7  # user config"), "{}", text);
        assert!(text.contains("confirm_kill = true  # default"), "{}", text);
        // It is a real TOML document, so it can be copied into a config file.
        let table: toml::Table = toml::from_str(&text).expect("effective output must parse");
        assert!(schema::validate_table(&table).is_empty());
    }

    #[test]
    fn issues_report_unknown_keys_without_failing_the_load() {
        let dir = TempDir::new().unwrap();
        let (handle, _) = handle_in(&dir, Some("[sessions]\nrefresh_secs = 3\nnonsense = 1\n"));
        assert!(handle.error().is_none());
        assert_eq!(handle.get().sessions.refresh_secs, 3);
        let issues = handle.issues();
        assert!(issues
            .iter()
            .any(|i| i.path == "sessions.nonsense" && i.severity == Severity::Warning));
    }

    #[test]
    fn line_col_is_one_based() {
        assert_eq!(line_col("abc", 0), (1, 1));
        assert_eq!(line_col("abc", 2), (1, 3));
        assert_eq!(line_col("ab\ncd", 3), (2, 1));
        assert_eq!(line_col("ab\ncd", 4), (2, 2));
        // Past the end clamps rather than panicking.
        assert_eq!(line_col("ab", 99), (1, 3));
    }

    #[test]
    fn ides_still_load_from_the_root_table() {
        let dir = TempDir::new().unwrap();
        let (handle, _) = handle_in(
            &dir,
            Some("[[ides]]\nname = \"VS Code\"\ncommand = \"code\"\nterminal = false\n"),
        );
        let config = handle.get();
        assert_eq!(config.ides.len(), 1);
        assert_eq!(config.ides[0].name, "VS Code");
    }
}
