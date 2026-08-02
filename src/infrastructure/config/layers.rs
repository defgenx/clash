//! Layer resolution — pure functions over parsed TOML tables.
//!
//! The layering order from the plan (innermost first):
//!
//! ```text
//! defaults (in code)
//!   ← ~/.config/clash/config.toml        [user]
//!   ← <repo>/.clash/config.toml          [project — restricted subset]
//!   ← CLASH_* env overrides              [ephemeral]
//! ```
//!
//! Nothing here touches the filesystem, so precedence, restriction and
//! provenance are all directly unit-testable — the repo's own convention of
//! keeping decisions in pure functions and the IO in a thin wrapper.

use super::schema::{self, Kind, Severity};
use std::collections::BTreeMap;

/// Where an effective value came from. Drives `--show-effective`'s provenance
/// comments, which is how a user answers "why is this setting not applying"
/// without guessing (the Ghostty `+show-config` idea).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    User,
    Project,
    Env,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::User => "user config",
            Source::Project => "project config",
            Source::Env => "environment",
        }
    }
}

/// One config layer: a parsed table plus where it came from.
#[derive(Debug, Clone)]
pub struct Layer {
    pub source: Source,
    pub table: toml::Table,
}

/// The merged result plus per-key provenance.
#[derive(Debug, Clone, Default)]
pub struct Merged {
    pub table: toml::Table,
    /// Dotted path → the layer that won it.
    pub provenance: BTreeMap<String, Source>,
}

// ── Path helpers ────────────────────────────────────────────────────

/// Read a dotted path out of a table.
pub fn get_path<'a>(table: &'a toml::Table, path: &str) -> Option<&'a toml::Value> {
    let mut current = table;
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        let value = current.get(part)?;
        if parts.peek().is_none() {
            return Some(value);
        }
        current = value.as_table()?;
    }
    None
}

/// Write a dotted path into a table, creating intermediate tables. A
/// non-table value in the middle of the path is replaced — the alternative is
/// silently dropping the write.
pub fn set_path(table: &mut toml::Table, path: &str, value: toml::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let (last, parents) = parts.split_last().expect("path must be non-empty");
    let mut current = table;
    for part in parents {
        let entry = current
            .entry(part.to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::Table::new());
        }
        current = entry.as_table_mut().expect("just ensured a table");
    }
    current.insert(last.to_string(), value);
}

/// Every scalar leaf in a table, as (dotted path, value), depth-first.
///
/// Arrays are treated as leaves: `[[ides]]` is free-form data, not a namespace
/// to walk into, and the schema does not model its innards.
pub fn leaves(table: &toml::Table) -> Vec<(String, &toml::Value)> {
    let mut out = Vec::new();
    walk(table, "", &mut out);
    out
}

fn walk<'a>(table: &'a toml::Table, prefix: &str, out: &mut Vec<(String, &'a toml::Value)>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{}.{}", prefix, key)
        };
        match value {
            toml::Value::Table(inner) => walk(inner, &path, out),
            other => out.push((path, other)),
        }
    }
}

// ── Merging ─────────────────────────────────────────────────────────

/// Merge layers in order (later wins), recording which layer won each leaf.
///
/// Tables merge key-by-key so a user who sets one key in `[terminal]` keeps
/// the defaults for the rest. Non-table values replace wholesale — an array
/// like `[[ides]]` is an all-or-nothing override, which is what a user editing
/// their editor list expects.
pub fn merge(layers: &[Layer]) -> Merged {
    let mut out = Merged::default();
    for layer in layers {
        merge_into(&mut out.table, &layer.table);
        for (path, _) in leaves(&layer.table) {
            out.provenance.insert(path, layer.source);
        }
    }
    out
}

fn merge_into(base: &mut toml::Table, overlay: &toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(key), value) {
            (Some(toml::Value::Table(base_inner)), toml::Value::Table(overlay_inner)) => {
                merge_into(base_inner, overlay_inner);
            }
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

// ── Project-layer restriction ───────────────────────────────────────

/// Strip everything a repo-local `.clash/config.toml` is not allowed to set.
///
/// A cloned repo can redirect *paths* (and, in later phases, declare actions
/// and notification hooks) but never `claude_bin` or anything else affecting
/// how clash launches processes — the same blast-radius limit cmux draws, with
/// a sharper edge here because clash spawns binaries.
///
/// Returns the filtered table plus a warning per rejected key, so the
/// restriction is visible rather than a silent no-op.
pub fn restrict_project_layer(table: &toml::Table) -> (toml::Table, Vec<schema::Issue>) {
    let mut kept = toml::Table::new();
    let mut issues = Vec::new();
    for (path, value) in leaves(table) {
        if path == "schema_version" {
            continue;
        }
        // Sections later phases own, and which carry no process-launch risk.
        if path.starts_with("actions") || path.starts_with("notifications.hooks") {
            set_path(&mut kept, &path, value.clone());
            continue;
        }
        match schema::prop(&path) {
            Some(p) if p.project_allowed => set_path(&mut kept, &path, value.clone()),
            Some(_) => issues.push(schema::Issue {
                path: path.clone(),
                message: "ignored: a project config may only set paths, actions and \
                          notification hooks"
                    .to_string(),
                severity: Severity::Warning,
            }),
            None => issues.push(schema::Issue {
                path: path.clone(),
                message: "ignored: unknown setting in a project config".to_string(),
                severity: Severity::Warning,
            }),
        }
    }
    (kept, issues)
}

// ── Environment overrides ───────────────────────────────────────────

/// Build the ephemeral env layer from `CLASH_*` variables.
///
/// `CLASH_<SECTION>_<KEY>` maps to `<section>.<key>`, e.g.
/// `CLASH_SESSIONS_REFRESH_SECS=5` → `sessions.refresh_secs = 5`. The mapping
/// is derived from the schema rather than parsed, so only real settings are
/// reachable and a typo'd variable is reported instead of silently ignored.
///
/// Values are parsed according to the property's declared kind — an env var is
/// always a string, and `refresh_secs = "5"` would fail validation.
pub fn env_layer<I, K, V>(vars: I) -> (toml::Table, Vec<schema::Issue>)
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let mut table = toml::Table::new();
    let mut issues = Vec::new();
    for (key, value) in vars {
        let key = key.as_ref();
        let Some(suffix) = key.strip_prefix("CLASH_") else {
            continue;
        };
        // `CLASH_LOG_RETENTION_HOURS` is an existing, unrelated knob read
        // straight from the environment by the logger; it is not a config
        // property and must not be reported as a typo.
        if suffix == "LOG_RETENTION_HOURS" {
            continue;
        }
        let wanted = suffix.to_ascii_lowercase();
        match schema::PROPS
            .iter()
            .find(|p| p.path.replace('.', "_") == wanted)
        {
            Some(p) => match parse_scalar(p.kind, value.as_ref()) {
                Ok(v) => set_path(&mut table, p.path, v),
                Err(message) => issues.push(schema::Issue {
                    path: key.to_string(),
                    message,
                    severity: Severity::Error,
                }),
            },
            None => issues.push(schema::Issue {
                path: key.to_string(),
                message: "no such setting; see `clash config --defaults`".to_string(),
                severity: Severity::Warning,
            }),
        }
    }
    (table, issues)
}

/// Parse an env-var string into the property's declared type.
fn parse_scalar(kind: Kind, raw: &str) -> Result<toml::Value, String> {
    match kind {
        Kind::Bool => match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(toml::Value::Boolean(true)),
            "0" | "false" | "no" | "off" => Ok(toml::Value::Boolean(false)),
            other => Err(format!("expected a boolean, got {:?}", other)),
        },
        Kind::Int { .. } => raw
            .trim()
            .parse::<i64>()
            .map(toml::Value::Integer)
            .map_err(|_| format!("expected an integer, got {:?}", raw)),
        Kind::Float { .. } => raw
            .trim()
            .parse::<f64>()
            .map(toml::Value::Float)
            .map_err(|_| format!("expected a number, got {:?}", raw)),
        Kind::Str | Kind::Path | Kind::Enum(_) => Ok(toml::Value::String(raw.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).expect("valid toml")
    }

    #[test]
    fn get_set_remove_round_trip() {
        let mut t = toml::Table::new();
        set_path(&mut t, "a.b.c", toml::Value::Integer(1));
        assert_eq!(get_path(&t, "a.b.c"), Some(&toml::Value::Integer(1)));
        assert!(get_path(&t, "a.b").unwrap().is_table());
        assert!(get_path(&t, "a.b.d").is_none());
        assert!(get_path(&t, "a.b.c.d").is_none());
    }

    #[test]
    fn set_path_replaces_a_scalar_standing_where_a_table_belongs() {
        let mut t = table("terminal = 5");
        set_path(&mut t, "terminal.font_size", toml::Value::Integer(14));
        assert_eq!(
            get_path(&t, "terminal.font_size"),
            Some(&toml::Value::Integer(14))
        );
    }

    #[test]
    fn leaves_walks_tables_but_treats_arrays_as_values() {
        let t = table(
            r#"
            top = 1
            [a]
            b = 2
            [a.c]
            d = 3
            [[ides]]
            name = "code"
        "#,
        );
        let paths: Vec<String> = leaves(&t).into_iter().map(|(p, _)| p).collect();
        assert_eq!(paths, vec!["a.b", "a.c.d", "ides", "top"]);
    }

    #[test]
    fn later_layers_win_key_by_key() {
        let merged = merge(&[
            Layer {
                source: Source::Default,
                table: table("[terminal]\nfont_size = 13\nshell = \"\"\n"),
            },
            Layer {
                source: Source::User,
                table: table("[terminal]\nfont_size = 16\n"),
            },
        ]);
        assert_eq!(
            get_path(&merged.table, "terminal.font_size"),
            Some(&toml::Value::Integer(16))
        );
        // The sibling key survived — a partial section is not a replacement.
        assert_eq!(
            get_path(&merged.table, "terminal.shell"),
            Some(&toml::Value::String(String::new()))
        );
        assert_eq!(
            merged.provenance.get("terminal.font_size"),
            Some(&Source::User)
        );
        assert_eq!(
            merged.provenance.get("terminal.shell"),
            Some(&Source::Default)
        );
    }

    #[test]
    fn full_precedence_chain_default_user_project_env() {
        let (env, _) = env_layer([("CLASH_PATHS_SCRATCH_DIR", "/from/env")]);
        let (project, _) =
            restrict_project_layer(&table("[paths]\nscratch_dir = \"/from/project\""));
        let merged = merge(&[
            Layer {
                source: Source::Default,
                table: schema::defaults_table(),
            },
            Layer {
                source: Source::User,
                table: table("[paths]\nscratch_dir = \"/from/user\"\n[sessions]\nrefresh_secs = 7"),
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
        assert_eq!(
            get_path(&merged.table, "paths.scratch_dir"),
            Some(&toml::Value::String("/from/env".into()))
        );
        assert_eq!(merged.provenance["paths.scratch_dir"], Source::Env);
        // Untouched by later layers → still the user's.
        assert_eq!(merged.provenance["sessions.refresh_secs"], Source::User);
        // Never set anywhere → default.
        assert_eq!(merged.provenance["sessions.confirm_kill"], Source::Default);
    }

    #[test]
    fn project_layer_keeps_paths_and_rejects_the_rest() {
        let (kept, issues) = restrict_project_layer(&table(
            r#"
            [paths]
            scratch_dir = "/repo/notes"
            [general]
            claude_bin = "/tmp/evil"
            [sessions]
            refresh_secs = 1
        "#,
        ));
        assert_eq!(
            get_path(&kept, "paths.scratch_dir"),
            Some(&toml::Value::String("/repo/notes".into()))
        );
        // The whole point: a cloned repo cannot redirect the binary clash spawns.
        assert!(get_path(&kept, "general.claude_bin").is_none());
        assert!(get_path(&kept, "sessions.refresh_secs").is_none());
        let rejected: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            rejected,
            vec!["general.claude_bin", "sessions.refresh_secs"]
        );
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
    }

    #[test]
    fn project_layer_allows_sections_reserved_for_later_phases() {
        let (kept, issues) = restrict_project_layer(&table(
            "[[actions]]\nid = \"test\"\ncommand = \"make test\"\n",
        ));
        assert!(get_path(&kept, "actions").is_some());
        assert!(issues.is_empty());
    }

    #[test]
    fn env_layer_parses_by_declared_kind() {
        let (t, issues) = env_layer([
            ("CLASH_SESSIONS_REFRESH_SECS", "5"),
            ("CLASH_SESSIONS_CONFIRM_KILL", "off"),
            ("CLASH_TERMINAL_LINE_HEIGHT", "1.25"),
            ("CLASH_GENERAL_CLAUDE_BIN", "/opt/claude"),
            ("PATH", "/usr/bin"),
        ]);
        assert!(issues.is_empty(), "{:?}", issues);
        assert_eq!(
            get_path(&t, "sessions.refresh_secs"),
            Some(&toml::Value::Integer(5))
        );
        assert_eq!(
            get_path(&t, "sessions.confirm_kill"),
            Some(&toml::Value::Boolean(false))
        );
        assert_eq!(
            get_path(&t, "terminal.line_height"),
            Some(&toml::Value::Float(1.25))
        );
        assert_eq!(
            get_path(&t, "general.claude_bin"),
            Some(&toml::Value::String("/opt/claude".into()))
        );
        // A non-CLASH_ variable is not our business.
        assert!(get_path(&t, "path").is_none());
    }

    #[test]
    fn env_layer_reports_typos_and_bad_values() {
        let (t, issues) = env_layer([
            ("CLASH_SESSIONS_REFRESH_SECONDS", "5"),
            ("CLASH_SESSIONS_REFRESH_SECS", "soon"),
        ]);
        assert!(t.is_empty());
        assert_eq!(issues.len(), 2);
        assert!(
            issues
                .iter()
                .any(|i| i.severity == Severity::Warning
                    && i.path == "CLASH_SESSIONS_REFRESH_SECONDS")
        );
        assert!(issues
            .iter()
            .any(|i| i.severity == Severity::Error && i.path == "CLASH_SESSIONS_REFRESH_SECS"));
    }

    /// The logger reads this one straight from the environment; flagging it as
    /// a typo'd setting would be a false positive on a documented variable.
    #[test]
    fn env_layer_ignores_the_log_retention_variable() {
        let (t, issues) = env_layer([("CLASH_LOG_RETENTION_HOURS", "48")]);
        assert!(t.is_empty());
        assert!(issues.is_empty());
    }
}
