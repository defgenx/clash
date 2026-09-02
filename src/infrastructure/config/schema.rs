//! The config schema — one table that every other consumer reads.
//!
//! This is the single source of truth the plan's Decision 1 calls for. It
//! drives, with no duplication:
//!
//! - the **defaults** every layer falls back to (`Prop::default`),
//! - **validation** of a user's `config.toml` (`validate_table`),
//! - the **JSON Schema** export for editor completion (`json_schema`),
//! - the annotated **`--defaults`** dump (`defaults_toml`),
//! - the **GUI settings migration** (`Prop::gui_key` maps a blob key to a
//!   config path — see [`super::migrate`]),
//! - the **frontend hints** a generated settings UI needs (`term_option`,
//!   `refit`, `restart_required`) — without these, a schema-driven panel
//!   cannot reproduce the live-apply behaviour the hand-wired one has.
//!
//! Everything here is pure: no IO, no globals. Tests at the bottom pin the
//! invariants the rest of the system assumes (every prop has a doc string, a
//! default that satisfies its own constraints, and a unique path).

use serde_json::{json, Map as JsonMap, Value as Json};
use std::collections::BTreeMap;

/// Where a setting actually lives.
///
/// The plan's Decision 2: only 7 of the GUI's 28 settings are meaningful to
/// both frontends. The other 21 configure xterm (or the GUI chrome) and can
/// never be read by the TUI, so they stay in the GUI's own store. They still
/// get schema entries so one validator covers both and a generated settings
/// panel can render every row from one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Lives in `config.toml`; both frontends read it.
    Shared,
    /// Lives in the GUI's `gui-state.json` settings blob; GUI-only.
    GuiLocal,
}

/// A property's type and its constraints. Constraints are part of the schema
/// so the range checks live in exactly one place (they used to be hand-written
/// per key in `applyWorkspacesData`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Float {
        min: f64,
        max: f64,
    },
    Str,
    /// A filesystem path. Serialized as a string; `""` means "unset, use the
    /// computed default" (the `Option<PathBuf>` fields).
    Path,
    Enum(&'static [&'static str]),
}

/// A property's default value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(&'static str),
}

impl Val {
    pub fn to_toml(self) -> toml::Value {
        match self {
            Val::Bool(b) => toml::Value::Boolean(b),
            Val::Int(i) => toml::Value::Integer(i),
            Val::Float(f) => toml::Value::Float(f),
            Val::Str(s) => toml::Value::String(s.to_string()),
        }
    }

    pub fn to_json(self) -> Json {
        match self {
            Val::Bool(b) => json!(b),
            Val::Int(i) => json!(i),
            Val::Float(f) => json!(f),
            Val::Str(s) => json!(s),
        }
    }
}

/// One configurable setting.
#[derive(Debug, Clone, Copy)]
pub struct Prop {
    /// Dotted path into the config document, e.g. `"sessions.refresh_secs"`.
    pub path: &'static str,
    /// The legacy `gui-state.json` settings-blob key this property came from,
    /// when it has one. Drives the one-shot migration and lets the GUI keep
    /// talking camelCase while the file is snake_case.
    pub gui_key: Option<&'static str>,
    pub kind: Kind,
    pub default: Val,
    /// One-line description. Shown in `--defaults`, the JSON Schema
    /// `description`, and (later) the generated settings row.
    pub doc: &'static str,
    pub scope: Scope,
    /// The xterm option this maps to, when the GUI can live-apply it.
    pub term_option: Option<&'static str>,
    /// Changing this alters cell metrics, so open terminals must be refit.
    /// The discriminator for the coalesced reload fan-out (plan Issue 14).
    pub refit: bool,
    /// Cannot be applied to already-running things; reported as "applies to
    /// new sessions" instead of silently doing nothing.
    pub restart_required: bool,
    /// May be overridden by a repo-local `.clash/config.toml`. Deliberately
    /// false for anything that affects how binaries are launched — a cloned
    /// repo must not be able to redirect `claude_bin`.
    pub project_allowed: bool,
}

impl Prop {
    const fn new(path: &'static str, kind: Kind, default: Val, doc: &'static str) -> Self {
        Self {
            path,
            gui_key: None,
            kind,
            default,
            doc,
            scope: Scope::Shared,
            term_option: None,
            refit: false,
            restart_required: false,
            project_allowed: false,
        }
    }
    const fn gui(mut self, key: &'static str) -> Self {
        self.gui_key = Some(key);
        self
    }
    const fn local(mut self) -> Self {
        self.scope = Scope::GuiLocal;
        self
    }
    const fn term(mut self, opt: &'static str) -> Self {
        self.term_option = Some(opt);
        self
    }
    const fn refit(mut self) -> Self {
        self.refit = true;
        self
    }
    const fn restart(mut self) -> Self {
        self.restart_required = true;
        self
    }
    const fn project(mut self) -> Self {
        self.project_allowed = true;
        self
    }

    /// The section this property lives in (`""` for a root-level key).
    pub fn section(&self) -> &'static str {
        match self.path.split_once('.') {
            Some((section, _)) => section,
            None => "",
        }
    }

    /// The leaf key name.
    pub fn leaf(&self) -> &'static str {
        match self.path.rsplit_once('.') {
            Some((_, leaf)) => leaf,
            None => self.path,
        }
    }
}

/// Font weights xterm accepts — CSS keywords plus the numeric steps. Mirrors
/// the GUI's `FONT_WEIGHTS`, which is now validated against this list.
const FONT_WEIGHTS: &[&str] = &["300", "normal", "500", "600", "bold", "800"];

/// The built-in theme keys (the GUI's `THEMES` table).
///
/// Hardcoded because themes are code today. Phase 5 (user themes) moves
/// `THEMES` to data, at which point this becomes a dynamic list; until then an
/// unknown theme name is a *warning* that falls back to the default, never a
/// hard error, so a config written by a newer clash stays loadable.
const THEMES: &[&str] = &[
    "clash-dark",
    "clash-light",
    "solarized-dark",
    "solarized-light",
    "nord",
    "tokyo-night",
    "catppuccin-mocha",
    "catppuccin-latte",
    "gruvbox-dark",
    "dracula",
    "one-dark",
    "github-light",
];

/// The current config schema version. Bumped whenever a key moves or changes
/// meaning; [`super::migrate::migrate_document`] upgrades older documents in
/// place. v1 was the flat, un-namespaced file (5 keys, no sections).
pub const SCHEMA_VERSION: i64 = 2;

/// Every configurable property, in the order `--defaults` emits them.
pub const PROPS: &[Prop] = &[
    // ── [general] ───────────────────────────────────────────────────
    Prop::new(
        "general.claude_bin",
        Kind::Str,
        Val::Str("claude"),
        "The `claude` binary to spawn sessions with — a name resolved on PATH, or an absolute path.",
    )
    .restart(),
    Prop::new(
        "general.debounce_ms",
        Kind::Int { min: 10, max: 5000 },
        Val::Int(200),
        "Filesystem-watcher debounce in milliseconds. Higher means fewer, larger refresh batches.",
    )
    .restart(),
    Prop::new(
        "general.skills_update",
        Kind::Enum(&["ask", "all", "untouched", "keep"]),
        Val::Str("ask"),
        "What to do when a clash upgrade ships agent skills over ones you edited by hand: \
         `ask` shows a popup at startup; `all` overwrites your edits; `keep` (and its older \
         synonym `untouched`) keeps them. Skills you never edited are always refreshed, \
         missing ones always install, and retired ones are removed unless you edited them — \
         none of that asks.",
    ),
    // ── [paths] ─────────────────────────────────────────────────────
    Prop::new(
        "paths.claude_dir",
        Kind::Path,
        Val::Str(""),
        "Claude Code data directory. Empty means `~/.claude`.",
    )
    .project(),
    Prop::new(
        "paths.scratch_dir",
        Kind::Path,
        Val::Str(""),
        "Where scratch notes live. Empty means `<claude_dir>/clash/scratch`.",
    )
    .project(),
    Prop::new(
        "paths.workflows_dir",
        Kind::Path,
        Val::Str(""),
        "Where workflow items live. Empty means `<claude_dir>/clash/workflows`.",
    )
    .project(),
    // ── [sessions] ──────────────────────────────────────────────────
    Prop::new(
        "sessions.default_cwd",
        Kind::Path,
        Val::Str(""),
        "Directory pre-filled when starting a new session. Empty means your home directory.",
    )
    .gui("defaultCwd"),
    Prop::new(
        "sessions.confirm_kill",
        Kind::Bool,
        Val::Bool(true),
        "Ask for confirmation before killing a session. Stashing never asks.",
    )
    .gui("confirmKill"),
    Prop::new(
        "sessions.refresh_secs",
        Kind::Int { min: 1, max: 30 },
        Val::Int(2),
        "How often the GUI polls the session list, in seconds. The TUI refreshes on filesystem \
         events and keeps its own faster backstop poll.",
    )
    .gui("refreshSecs"),
    // ── [terminal] — shared ─────────────────────────────────────────
    Prop::new(
        "terminal.shell",
        Kind::Str,
        Val::Str(""),
        "Shell for in-app terminals. Empty means `$SHELL`.",
    )
    .gui("termShell"),
    Prop::new(
        "terminal.tui_terminal",
        Kind::Str,
        Val::Str(""),
        "Terminal emulator the TUI launcher uses. Empty means auto-detect.",
    )
    .gui("tuiTerminal"),
    // ── [notifications] ─────────────────────────────────────────────
    Prop::new(
        "notifications.enabled",
        Kind::Bool,
        Val::Bool(true),
        "Show desktop notifications when a session needs attention.",
    )
    .gui("notifications"),
    Prop::new(
        "notifications.title_attention",
        Kind::Bool,
        Val::Bool(true),
        "Show a count of sessions needing input in the window title, e.g. `clash (2!)`.",
    )
    .gui("titleAttention"),
    // ── [workflows] ─────────────────────────────────────────────────
    Prop::new(
        "workflows.pr_skill",
        Kind::Str,
        Val::Str("hivebrite-engineering:github-pr"),
        "Skill the workflow PR phase opens pull requests with. `none` disables it (the agent \
         follows the repo's own PR conventions with `gh`); the skill itself falls back to \
         conventions when it isn't installed in the session.",
    ),
    Prop::new(
        "workflows.forge",
        Kind::Enum(&["auto", "github", "none"]),
        Val::Str("auto"),
        "Code forge for workflow PR features. `auto` detects from the repo's origin remote \
         (unknown hosts count as GitHub, so GitHub Enterprise keeps working); `none` disables \
         change-request features for repos without a supported forge.",
    ),
    Prop::new(
        "workflows.slack_webhook",
        Kind::Str,
        Val::Str(""),
        "Slack incoming-webhook URL used by workflow sharing and notifications. Empty disables \
         the Slack destination. Nothing is ever sent without an explicit action or the \
         notify_webhook opt-in.",
    ),
    Prop::new(
        "workflows.discord_webhook",
        Kind::Str,
        Val::Str(""),
        "Discord webhook URL used by workflow sharing and notifications. Empty disables the \
         Discord destination. Nothing is ever sent without an explicit action or the \
         notify_webhook opt-in.",
    ),
    Prop::new(
        "workflows.notify_webhook",
        Kind::Enum(&["off", "slack", "discord"]),
        Val::Str("off"),
        "Announce workflow items that park at a decision state (plan review, diff review, PR \
         draft) on the configured webhook. Only agent-driven transitions post — never your own \
         clicks.",
    ),
    Prop::new(
        "workflows.jira_skill",
        Kind::Str,
        Val::Str(""),
        "Skill that posts the share document to Jira in a Claude Code session, instead of \
         clash's own API-token transport. Empty does not disable the session route — it only \
         means no skill is named, and the session is told to use whatever tooling it has \
         connected (an MCP server for Jira, say). clash's own transport is used when the \
         credentials are set and no skill is named. A named skill that is not installed in the \
         session falls back to that same tooling.",
    ),
    Prop::new(
        "workflows.chat_skill",
        Kind::Str,
        Val::Str(""),
        "Skill that posts the share document to Slack or Discord in a Claude Code session, \
         instead of the webhook transport. Same shape as `jira_skill`, including the fallback: \
         empty means no skill is named, not that the destination is unavailable — with no \
         webhook either, a session posts it with whatever it has connected. Decision \
         notifications (`notify_webhook`) always use the webhook: they fire without a human \
         present, and a notification you have to go read in a session is not a notification.",
    ),
    Prop::new(
        "workflows.jira_base_url",
        Kind::Str,
        Val::Str(""),
        "Jira site URL (e.g. https://yourorg.atlassian.net) used by the workflow share \
         dialog's \"Post to Jira\" destination. Empty disables it. Nothing is ever posted \
         without an explicit share action.",
    ),
    Prop::new(
        "workflows.jira_email",
        Kind::Str,
        Val::Str(""),
        "Jira account email paired with the API token for the \"Post to Jira\" destination.",
    ),
    Prop::new(
        "workflows.jira_api_token",
        Kind::Str,
        Val::Str(""),
        "Jira API token (id.atlassian.com → Security → API tokens) for the \"Post to Jira\" \
         destination. Stored in config.toml like the webhook URLs.",
    ),
    // ── [appearance] — GUI-local ────────────────────────────────────
    Prop::new(
        "appearance.theme",
        Kind::Enum(THEMES),
        Val::Str("clash-dark"),
        "Colour theme for the window chrome and the terminals.",
    )
    .gui("theme")
    .local(),
    // ── [terminal] — GUI-local (xterm rendering) ────────────────────
    Prop::new(
        "terminal.font_size",
        Kind::Int { min: 9, max: 24 },
        Val::Int(13),
        "Terminal font size in pixels.",
    )
    .gui("fontSize")
    .local()
    .term("fontSize")
    .refit(),
    Prop::new(
        "terminal.font_family",
        Kind::Str,
        Val::Str("SF Mono, Menlo, monospace"),
        "Terminal font stack (CSS font-family syntax).",
    )
    .gui("fontFamily")
    .local()
    .term("fontFamily")
    .refit(),
    Prop::new(
        "terminal.font_weight",
        Kind::Enum(FONT_WEIGHTS),
        Val::Str("normal"),
        "Weight for normal terminal text.",
    )
    .gui("fontWeight")
    .local()
    .term("fontWeight"),
    Prop::new(
        "terminal.font_weight_bold",
        Kind::Enum(FONT_WEIGHTS),
        Val::Str("bold"),
        "Weight for bold terminal text.",
    )
    .gui("fontWeightBold")
    .local()
    .term("fontWeightBold"),
    Prop::new(
        "terminal.line_height",
        Kind::Float { min: 1.0, max: 2.0 },
        Val::Float(1.0),
        "Terminal line height as a multiple of the font size.",
    )
    .gui("lineHeight")
    .local()
    .term("lineHeight")
    .refit(),
    Prop::new(
        "terminal.letter_spacing",
        Kind::Float {
            min: -2.0,
            max: 5.0,
        },
        Val::Float(0.0),
        "Extra space between terminal characters, in pixels.",
    )
    .gui("letterSpacing")
    .local()
    .term("letterSpacing")
    .refit(),
    Prop::new(
        "terminal.scrollback",
        Kind::Int {
            min: 0,
            max: 200_000,
        },
        Val::Int(10_000),
        "Lines of terminal scrollback to keep.",
    )
    .gui("scrollback")
    .local()
    .term("scrollback"),
    Prop::new(
        "terminal.cursor_style",
        Kind::Enum(&["block", "bar", "underline"]),
        Val::Str("block"),
        "Cursor shape in a focused terminal.",
    )
    .gui("cursorStyle")
    .local()
    .term("cursorStyle"),
    Prop::new(
        "terminal.cursor_inactive_style",
        Kind::Enum(&["outline", "block", "bar", "underline", "none"]),
        Val::Str("outline"),
        "Cursor shape in an unfocused terminal.",
    )
    .gui("cursorInactiveStyle")
    .local()
    .term("cursorInactiveStyle"),
    Prop::new(
        "terminal.cursor_width",
        Kind::Int { min: 1, max: 5 },
        Val::Int(1),
        "Thickness of a bar cursor, in pixels.",
    )
    .gui("cursorWidth")
    .local()
    .term("cursorWidth"),
    Prop::new(
        "terminal.cursor_blink",
        Kind::Bool,
        Val::Bool(false),
        "Blink the terminal cursor.",
    )
    .gui("cursorBlink")
    .local()
    .term("cursorBlink"),
    Prop::new(
        "terminal.minimum_contrast",
        Kind::Float { min: 1.0, max: 21.0 },
        Val::Float(1.0),
        "Minimum contrast ratio between terminal text and background. 1 = off.",
    )
    .gui("minimumContrast")
    .local()
    .term("minimumContrastRatio"),
    Prop::new(
        "terminal.bright_bold",
        Kind::Bool,
        Val::Bool(false),
        "Draw bold terminal text in the bright ANSI colours.",
    )
    .gui("brightBold")
    .local()
    .term("drawBoldTextInBrightColors"),
    Prop::new(
        "terminal.scroll_speed",
        Kind::Int { min: 1, max: 10 },
        Val::Int(1),
        "Lines scrolled per wheel notch.",
    )
    .gui("scrollSpeed")
    .local()
    .term("scrollSensitivity"),
    Prop::new(
        "terminal.smooth_scroll",
        Kind::Int { min: 0, max: 500 },
        Val::Int(0),
        "Smooth-scroll animation in milliseconds. 0 = instant.",
    )
    .gui("smoothScroll")
    .local()
    .term("smoothScrollDuration"),
    Prop::new(
        "terminal.copy_on_select",
        Kind::Bool,
        Val::Bool(false),
        "Copy to the clipboard as soon as text is selected.",
    )
    .gui("copyOnSelect")
    .local(),
    Prop::new(
        "terminal.right_click_word",
        Kind::Bool,
        Val::Bool(true),
        "Right-click selects the word under the pointer.",
    )
    .gui("rightClickWord")
    .local()
    .term("rightClickSelectsWord"),
    Prop::new(
        "terminal.option_meta",
        Kind::Bool,
        Val::Bool(true),
        "⌥ sends Esc (Meta) in terminals. Off means ⌥ always composes characters.",
    )
    .gui("optionMeta")
    .local()
    .term("macOptionIsMeta"),
    Prop::new(
        "terminal.bell_toast",
        Kind::Bool,
        Val::Bool(false),
        "Surface a terminal bell as an in-app toast.",
    )
    .gui("bellToast")
    .local(),
    // ── [browser] — GUI-local ───────────────────────────────────────
    Prop::new(
        "browser.link_open",
        Kind::Enum(&["ask", "embedded", "external"]),
        Val::Str("ask"),
        "How terminal links open: ask each time, in clash's browser panel, or in the system browser.",
    )
    .gui("linkOpen")
    .local(),
];

/// Look up a property by its dotted path.
pub fn prop(path: &str) -> Option<&'static Prop> {
    PROPS.iter().find(|p| p.path == path)
}

/// Look up a property by its legacy GUI settings-blob key.
///
/// Consumed by the GUI's settings commands, so the binary's private-`mod`
/// build compiles it as dead.
#[allow(dead_code)]
pub fn prop_by_gui_key(key: &str) -> Option<&'static Prop> {
    PROPS.iter().find(|p| p.gui_key == Some(key))
}

/// Every property that lives in `config.toml`.
pub fn shared_props() -> impl Iterator<Item = &'static Prop> {
    PROPS.iter().filter(|p| p.scope == Scope::Shared)
}

/// Every property that stays in the GUI's own store.
///
/// Consumed by the GUI's settings commands (see `prop_by_gui_key`).
#[allow(dead_code)]
pub fn gui_local_props() -> impl Iterator<Item = &'static Prop> {
    PROPS.iter().filter(|p| p.scope == Scope::GuiLocal)
}

// ── Validation ──────────────────────────────────────────────────────

/// A problem found in a config document.
///
/// Distinguishes *errors* (a value the schema rejects) from *warnings* (a key
/// the schema doesn't know). An unknown key is deliberately not an error: a
/// config written by a newer clash must stay loadable by an older one, which
/// is the same forward-compatibility stance the domain types take with
/// `#[serde(flatten)]`.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub path: String,
    pub message: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tag = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{}: {}: {}", tag, self.path, self.message)
    }
}

/// Check one value against a property's declared type and constraints.
/// Returns `None` when the value is acceptable.
pub fn check_value(p: &Prop, value: &toml::Value) -> Option<String> {
    match (&p.kind, value) {
        (Kind::Bool, toml::Value::Boolean(_)) => None,
        (Kind::Int { min, max }, toml::Value::Integer(i)) => {
            if i < min || i > max {
                Some(format!("must be between {} and {} (got {})", min, max, i))
            } else {
                None
            }
        }
        // An integer is a valid float in TOML's eyes for our purposes — writing
        // `line_height = 1` should not be an error just because it has no `.0`.
        (Kind::Float { min, max }, toml::Value::Integer(i)) => {
            let f = *i as f64;
            if f < *min || f > *max {
                Some(format!("must be between {} and {} (got {})", min, max, i))
            } else {
                None
            }
        }
        (Kind::Float { min, max }, toml::Value::Float(f)) => {
            if !f.is_finite() || f < min || f > max {
                Some(format!("must be between {} and {} (got {})", min, max, f))
            } else {
                None
            }
        }
        (Kind::Str, toml::Value::String(_)) | (Kind::Path, toml::Value::String(_)) => None,
        (Kind::Enum(allowed), toml::Value::String(s)) => {
            if allowed.contains(&s.as_str()) {
                None
            } else {
                Some(format!(
                    "must be one of {} (got {:?})",
                    allowed.join(", "),
                    s
                ))
            }
        }
        _ => Some(format!(
            "expected {}, got {}",
            kind_name(&p.kind),
            value.type_str()
        )),
    }
}

fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::Bool => "a boolean",
        Kind::Int { .. } => "an integer",
        Kind::Float { .. } => "a number",
        Kind::Str => "a string",
        Kind::Path => "a path string",
        Kind::Enum(_) => "one of a fixed set of strings",
    }
}

/// Validate a whole config document against the schema.
///
/// Walks every leaf in the table (so a typo two sections deep is still found)
/// and reports unknown paths as warnings, bad values as errors. `[[ides]]` and
/// the reserved `[keymap]`/`[actions]` sections are skipped — they are
/// free-form or belong to later phases, and reporting them would train users
/// to ignore the output.
pub fn validate_table(table: &toml::Table) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (path, value) in super::layers::leaves(table) {
        if is_reserved(&path) {
            continue;
        }
        match prop(&path) {
            Some(p) => {
                if let Some(message) = check_value(p, value) {
                    issues.push(Issue {
                        path,
                        message,
                        severity: Severity::Error,
                    });
                }
            }
            None => issues.push(Issue {
                path,
                message: "unknown setting (kept as-is, but clash does not read it)".to_string(),
                severity: Severity::Warning,
            }),
        }
    }
    issues.sort_by(|a, b| a.path.cmp(&b.path));
    issues
}

/// Sections and keys the schema deliberately does not model.
///
/// `ides` is a free-form array of tables; `keymap` and `actions` are declared
/// by later phases of the config plan and must round-trip untouched until
/// then; `schema_version` is metadata, not a setting.
fn is_reserved(path: &str) -> bool {
    // `[[ides]]` is an array, and `leaves()` treats arrays as leaves, so the
    // path is the bare section name — not `ides.<something>`.
    const SECTIONS: &[&str] = &["ides", "keymap", "actions"];
    path == "schema_version"
        || SECTIONS
            .iter()
            .any(|s| path == *s || path.starts_with(&format!("{}.", s)))
}

// ── Exports ─────────────────────────────────────────────────────────

/// The defaults as a `toml::Table` — the innermost config layer.
pub fn defaults_table() -> toml::Table {
    let mut table = toml::Table::new();
    table.insert(
        "schema_version".to_string(),
        toml::Value::Integer(SCHEMA_VERSION),
    );
    for p in shared_props() {
        super::layers::set_path(&mut table, p.path, p.default.to_toml());
    }
    table
}

/// The annotated default config file — `clash config --defaults`.
///
/// Zed's "open default settings" idea: the full file, every key present with
/// its default and its doc comment, ready to copy lines out of. Only shared
/// properties appear; GUI-local ones are not read from this file.
pub fn defaults_toml() -> String {
    let mut out = String::new();
    out.push_str("# clash configuration — annotated defaults.\n");
    out.push_str("# Copy the lines you want into your own config.toml.\n");
    out.push_str("# Path: see `clash config --path`\n\n");
    out.push_str(&format!("schema_version = {}\n", SCHEMA_VERSION));

    let mut by_section: BTreeMap<&str, Vec<&Prop>> = BTreeMap::new();
    for p in shared_props() {
        by_section.entry(p.section()).or_default().push(p);
    }
    for (section, props) in by_section {
        out.push_str(&format!("\n[{}]\n", section));
        for p in props {
            out.push_str(&format!("# {}\n", p.doc));
            if p.restart_required {
                out.push_str(
                    "# Takes effect on restart; already-running things keep their value.\n",
                );
            }
            out.push_str(&format!(
                "{} = {}\n",
                p.leaf(),
                toml_scalar(&p.default.to_toml())
            ));
        }
    }
    out.push_str("\n# Editors offered when opening a project or a scratch note.\n");
    out.push_str("# [[ides]]\n# name = \"VS Code\"\n# command = \"code\"\n# terminal = false\n");
    out
}

/// Render a scalar the way TOML wants it. Only used for the defaults dump, so
/// it only needs to handle the scalar kinds the schema can express.
fn toml_scalar(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => format!("{:?}", s),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => {
            // TOML floats need a decimal point; `1` would parse as an integer.
            if f.fract() == 0.0 {
                format!("{:.1}", f)
            } else {
                f.to_string()
            }
        }
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// The JSON Schema for `config.toml` — `clash config --schema`.
///
/// Point taplo / Even Better TOML at it for completion and inline docs, which
/// is what Decision 1 trades JSONC's `$schema` for. `x-`-prefixed keys carry
/// the frontend hints a generated settings UI needs; JSON Schema validators
/// ignore unknown keywords, so they cost nothing.
pub fn json_schema() -> Json {
    let mut sections: BTreeMap<&str, JsonMap<String, Json>> = BTreeMap::new();

    for p in PROPS {
        let mut node = JsonMap::new();
        match p.kind {
            Kind::Bool => {
                node.insert("type".into(), json!("boolean"));
            }
            Kind::Int { min, max } => {
                node.insert("type".into(), json!("integer"));
                node.insert("minimum".into(), json!(min));
                node.insert("maximum".into(), json!(max));
            }
            Kind::Float { min, max } => {
                node.insert("type".into(), json!("number"));
                node.insert("minimum".into(), json!(min));
                node.insert("maximum".into(), json!(max));
            }
            Kind::Str => {
                node.insert("type".into(), json!("string"));
            }
            Kind::Path => {
                node.insert("type".into(), json!("string"));
                node.insert("x-path".into(), json!(true));
            }
            Kind::Enum(allowed) => {
                node.insert("type".into(), json!("string"));
                node.insert("enum".into(), json!(allowed));
            }
        }
        node.insert("description".into(), json!(p.doc));
        node.insert("default".into(), p.default.to_json());
        node.insert(
            "x-scope".into(),
            json!(match p.scope {
                Scope::Shared => "shared",
                Scope::GuiLocal => "gui-local",
            }),
        );
        if let Some(key) = p.gui_key {
            node.insert("x-gui-key".into(), json!(key));
        }
        if let Some(opt) = p.term_option {
            node.insert("x-term-option".into(), json!(opt));
        }
        if p.refit {
            node.insert("x-refit".into(), json!(true));
        }
        if p.restart_required {
            node.insert("x-restart-required".into(), json!(true));
        }
        if p.project_allowed {
            node.insert("x-project-allowed".into(), json!(true));
        }
        sections
            .entry(p.section())
            .or_default()
            .insert(p.leaf().to_string(), Json::Object(node));
    }

    let mut properties = JsonMap::new();
    properties.insert(
        "schema_version".into(),
        json!({
            "type": "integer",
            "description": "Config schema version. clash migrates older documents on load.",
            "default": SCHEMA_VERSION,
        }),
    );
    for (section, props) in sections {
        properties.insert(
            section.to_string(),
            json!({
                "type": "object",
                "properties": Json::Object(props),
                // Unknown keys are preserved on save, so the schema must not
                // call them invalid.
                "additionalProperties": true,
            }),
        );
    }
    properties.insert(
        "ides".into(),
        json!({
            "type": "array",
            "description": "Editors offered when opening a project or a scratch note.",
            "items": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "command": { "type": "string" },
                    "description": { "type": "string" },
                    "terminal": { "type": "boolean", "default": false },
                },
                "required": ["name", "command"],
            },
        }),
    );

    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "clash configuration",
        "type": "object",
        "properties": Json::Object(properties),
        "additionalProperties": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The `--defaults` dump and a generated settings row both print the doc
    /// string, so an empty one ships a blank row.
    #[test]
    fn every_prop_has_a_doc_string() {
        for p in PROPS {
            assert!(!p.doc.trim().is_empty(), "{} has no doc string", p.path);
            assert!(
                p.doc.ends_with('.') || p.doc.ends_with(')'),
                "{}: doc should read as a sentence: {:?}",
                p.path,
                p.doc
            );
        }
    }

    /// A default that its own schema rejects would make a fresh install fail
    /// `--validate`.
    #[test]
    fn every_default_satisfies_its_own_constraints() {
        for p in PROPS {
            let value = p.default.to_toml();
            assert!(
                check_value(p, &value).is_none(),
                "{}: default {:?} fails its own check: {:?}",
                p.path,
                value,
                check_value(p, &value)
            );
        }
    }

    #[test]
    fn paths_and_gui_keys_are_unique() {
        let mut paths = HashSet::new();
        let mut gui_keys = HashSet::new();
        for p in PROPS {
            assert!(paths.insert(p.path), "duplicate path {}", p.path);
            if let Some(k) = p.gui_key {
                assert!(gui_keys.insert(k), "duplicate gui key {}", k);
            }
            assert!(
                p.path.contains('.'),
                "{} must live in a section, not at the root",
                p.path
            );
        }
    }

    /// Decision 2, pinned. The GUI settings blob had exactly these 28 keys;
    /// every one must be accounted for — 7 migrated to `config.toml`, 21 kept
    /// GUI-local. A key added to the GUI without a schema entry fails here
    /// instead of silently reverting to a default at runtime.
    #[test]
    fn all_28_legacy_gui_keys_are_accounted_for() {
        const SHARED: &[&str] = &[
            "defaultCwd",
            "confirmKill",
            "refreshSecs",
            "termShell",
            "tuiTerminal",
            "notifications",
            "titleAttention",
        ];
        const GUI_LOCAL: &[&str] = &[
            "theme",
            "fontSize",
            "fontFamily",
            "fontWeight",
            "fontWeightBold",
            "lineHeight",
            "letterSpacing",
            "scrollback",
            "cursorStyle",
            "cursorInactiveStyle",
            "cursorWidth",
            "cursorBlink",
            "minimumContrast",
            "brightBold",
            "scrollSpeed",
            "smoothScroll",
            "copyOnSelect",
            "rightClickWord",
            "optionMeta",
            "bellToast",
            "linkOpen",
        ];
        assert_eq!(SHARED.len() + GUI_LOCAL.len(), 28);

        for key in SHARED {
            let p = prop_by_gui_key(key).unwrap_or_else(|| panic!("{} has no schema entry", key));
            assert_eq!(p.scope, Scope::Shared, "{} should be shared", key);
        }
        for key in GUI_LOCAL {
            let p = prop_by_gui_key(key).unwrap_or_else(|| panic!("{} has no schema entry", key));
            assert_eq!(p.scope, Scope::GuiLocal, "{} should be GUI-local", key);
        }
        // And nothing else claims a GUI key — a new blob key must be added to
        // one of the lists above deliberately.
        let known: HashSet<&str> = SHARED.iter().chain(GUI_LOCAL).copied().collect();
        for p in PROPS {
            if let Some(k) = p.gui_key {
                assert!(
                    known.contains(k),
                    "{} maps to unlisted gui key {}",
                    p.path,
                    k
                );
            }
        }
    }

    /// The blast-radius limit from the plan: a cloned repo must never be able
    /// to change how binaries are launched.
    #[test]
    fn project_layer_cannot_override_claude_bin() {
        assert!(!prop("general.claude_bin").unwrap().project_allowed);
        for p in PROPS {
            if p.project_allowed {
                assert_eq!(
                    p.section(),
                    "paths",
                    "{} is project-overridable but not a path",
                    p.path
                );
            }
        }
    }

    /// Only keys that change cell metrics may ask for a refit — that flag is
    /// what keeps a config-reload burst from refitting every pane (Issue 14).
    #[test]
    fn refit_is_limited_to_cell_metric_keys() {
        let refit: Vec<&str> = PROPS.iter().filter(|p| p.refit).map(|p| p.path).collect();
        assert_eq!(
            refit,
            vec![
                "terminal.font_size",
                "terminal.font_family",
                "terminal.line_height",
                "terminal.letter_spacing",
            ]
        );
    }

    #[test]
    fn defaults_toml_round_trips_and_validates_clean() {
        let text = defaults_toml();
        let table: toml::Table = toml::from_str(&text).expect("defaults must parse");
        let issues = validate_table(&table);
        assert!(
            issues.is_empty(),
            "defaults must validate clean: {:?}",
            issues
        );
        // Every shared prop is present, so the dump really is complete.
        for p in shared_props() {
            assert!(
                super::super::layers::get_path(&table, p.path).is_some(),
                "{} missing from --defaults",
                p.path
            );
        }
    }

    #[test]
    fn defaults_table_matches_the_schema() {
        let table = defaults_table();
        for p in shared_props() {
            let got = super::super::layers::get_path(&table, p.path).expect(p.path);
            assert_eq!(*got, p.default.to_toml(), "{}", p.path);
        }
        // GUI-local props never land in config.toml.
        for p in gui_local_props() {
            assert!(
                super::super::layers::get_path(&table, p.path).is_none(),
                "{} is GUI-local and must not be written to config.toml",
                p.path
            );
        }
    }

    #[test]
    fn json_schema_carries_frontend_hints() {
        let schema = json_schema();
        let font = &schema["properties"]["terminal"]["properties"]["font_size"];
        assert_eq!(font["type"], "integer");
        assert_eq!(font["minimum"], 9);
        assert_eq!(font["x-term-option"], "fontSize");
        assert_eq!(font["x-refit"], true);
        assert_eq!(font["x-scope"], "gui-local");
        let bin = &schema["properties"]["general"]["properties"]["claude_bin"];
        assert_eq!(bin["x-restart-required"], true);
        assert_eq!(bin["default"], "claude");
        // Unknown keys survive a save, so the schema must not reject them.
        assert_eq!(schema["additionalProperties"], true);
    }

    #[test]
    fn check_value_rejects_out_of_range_and_wrong_types() {
        let p = prop("sessions.refresh_secs").unwrap();
        assert!(check_value(p, &toml::Value::Integer(2)).is_none());
        assert!(check_value(p, &toml::Value::Integer(0)).is_some());
        assert!(check_value(p, &toml::Value::Integer(31)).is_some());
        assert!(check_value(p, &toml::Value::String("2".into())).is_some());

        let theme = prop("appearance.theme").unwrap();
        assert!(check_value(theme, &toml::Value::String("nord".into())).is_none());
        assert!(check_value(theme, &toml::Value::String("neon".into())).is_some());

        // An integer literal is accepted where a float is expected.
        let lh = prop("terminal.line_height").unwrap();
        assert!(check_value(lh, &toml::Value::Integer(1)).is_none());
        assert!(check_value(lh, &toml::Value::Float(1.4)).is_none());
        assert!(check_value(lh, &toml::Value::Float(2.5)).is_some());
    }

    #[test]
    fn validate_flags_unknown_keys_as_warnings_not_errors() {
        let table: toml::Table = toml::from_str(
            r#"
            [sessions]
            refresh_secs = 3
            nonsense = true
            [future_section]
            whatever = 1
        "#,
        )
        .unwrap();
        let issues = validate_table(&table);
        assert_eq!(issues.len(), 2, "{:?}", issues);
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
        assert_eq!(issues[0].path, "future_section.whatever");
        assert_eq!(issues[1].path, "sessions.nonsense");
    }

    #[test]
    fn validate_reports_bad_values_as_errors() {
        let table: toml::Table = toml::from_str("[sessions]\nrefresh_secs = 99\n").unwrap();
        let issues = validate_table(&table);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("between 1 and 30"));
    }

    /// Later phases own `[keymap]` and `[actions]`; flagging them now would
    /// teach users that `--validate` output is noise.
    #[test]
    fn reserved_sections_are_not_flagged() {
        let table: toml::Table = toml::from_str(
            r#"
            schema_version = 2
            [keymap]
            "session.reload" = "cmd+r"
            [[ides]]
            name = "VS Code"
            command = "code"
        "#,
        )
        .unwrap();
        assert!(validate_table(&table).is_empty());
    }
}
