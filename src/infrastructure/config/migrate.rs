//! Migrations — pure, idempotent, and the only place that knows about old
//! shapes.
//!
//! Two of them:
//!
//! 1. **v1 → v2**: the original `config.toml` was flat (`claude_bin`,
//!    `scratch_dir`, …). v2 namespaces everything into sections. Applied in
//!    memory on every load and written back the next time we save, so an old
//!    binary reading the same file keeps working until then.
//! 2. **The GUI settings blob → `config.toml`**: [`migrate_gui_blob`] is the
//!    single Rust entry point the plan's Issue 6 asks for. It validates *all*
//!    28 legacy keys against the schema — the 7 shared ones on their way into
//!    `config.toml`, the 21 GUI-local ones on their way back to the frontend —
//!    so range checks and legacy fixups exist once instead of being duplicated
//!    in JS where the TUI could never reuse them.

use super::doc;
use super::layers;
use super::schema::{self, Kind, Prop, Scope, Severity, SCHEMA_VERSION};
use serde_json::{Map as JsonMap, Value as Json};

/// Root-level v1 keys and where they live in v2.
const V1_MOVES: &[(&str, &str)] = &[
    ("claude_bin", "general.claude_bin"),
    ("debounce_ms", "general.debounce_ms"),
    ("claude_dir", "paths.claude_dir"),
    ("scratch_dir", "paths.scratch_dir"),
    ("workflows_dir", "paths.workflows_dir"),
];

/// The document's declared schema version (0 when absent, i.e. a v1 file
/// written before versioning existed).
fn table_version(table: &toml::Table) -> i64 {
    table
        .get("schema_version")
        .and_then(|v| v.as_integer())
        .unwrap_or(0)
}

/// Upgrade a parsed config table in place. Idempotent: running it on a v2
/// document changes nothing.
///
/// A key present in *both* shapes keeps its v2 value — the namespaced form is
/// the one clash writes, so it is the more recent intent.
pub fn migrate_table(table: &mut toml::Table) -> bool {
    if table_version(table) >= SCHEMA_VERSION {
        return false;
    }
    let mut changed = false;
    for (old, new) in V1_MOVES {
        let Some(value) = table.remove(*old) else {
            continue;
        };
        changed = true;
        if layers::get_path(table, new).is_none() {
            layers::set_path(table, new, value);
        }
    }
    table.insert(
        "schema_version".to_string(),
        toml::Value::Integer(SCHEMA_VERSION),
    );
    changed || table_version(table) != SCHEMA_VERSION
}

/// The same upgrade against a format-preserving document, so writing the file
/// back keeps the user's comments and key order.
///
/// Only reached from the write path, which only the GUI exercises.
#[allow(dead_code)]
pub fn migrate_document(document: &mut toml_edit::DocumentMut) -> bool {
    let version = document
        .get("schema_version")
        .and_then(|i| i.as_integer())
        .unwrap_or(0);
    if version >= SCHEMA_VERSION {
        return false;
    }
    for (old, new) in V1_MOVES {
        // `remove_entry` rather than `remove`: a comment written above the key
        // lives on the *key*, not the value, and dropping it would silently eat
        // the user's annotations the first time they saved after an upgrade.
        let Some((key, item)) = document.as_table_mut().remove_entry(old) else {
            continue;
        };
        if doc::get(document, new).is_none() {
            if let Some(value) = item.as_value() {
                doc::set_keeping_decor(document, new, value.clone(), &key);
            }
        }
    }
    doc::set(document, "schema_version", SCHEMA_VERSION.into());
    true
}

// ── GUI settings blob ───────────────────────────────────────────────

/// The outcome of migrating a GUI settings blob.
///
/// This half of the module is consumed only by the GUI's
/// `config_migrate_gui_blob` command, so the binary's private-`mod` build sees
/// it as dead.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct GuiMigration {
    /// Shared settings, as (config path, value), ready to write to
    /// `config.toml`. Only keys the user actually customised appear — we never
    /// write a wall of defaults.
    pub shared: Vec<(String, toml::Value)>,
    /// The GUI-local settings, validated and normalised, keyed by their
    /// camelCase blob key so the frontend can apply them directly.
    pub gui_local: JsonMap<String, Json>,
    /// Per-key problems. A rejected value keeps its default, matching what the
    /// hand-written JS checks did — a corrupt blob must never poison settings.
    pub warnings: Vec<schema::Issue>,
}

/// Validate and split a `gui-state.json` settings blob.
///
/// Pure: no IO, no globals. The caller decides what to do with the halves —
/// the Tauri command writes `shared` into `config.toml` and hands `gui_local`
/// plus `warnings` back to the frontend.
#[allow(dead_code)] // GUI-only; see GuiMigration.
pub fn migrate_gui_blob(blob: &Json) -> GuiMigration {
    let mut out = GuiMigration::default();
    let Some(map) = blob.as_object() else {
        return out;
    };

    for p in schema::PROPS {
        let Some(key) = p.gui_key else { continue };
        let Some(raw) = map.get(key) else { continue };
        // JSON null is how a JS `undefined` round-trips; treat it as absent.
        if raw.is_null() {
            continue;
        }
        match coerce_json(p, raw) {
            Ok(value) => match p.scope {
                Scope::Shared => out.shared.push((p.path.to_string(), value)),
                Scope::GuiLocal => {
                    out.gui_local.insert(key.to_string(), toml_to_json(&value));
                }
            },
            Err(message) => out.warnings.push(schema::Issue {
                path: format!("{} ({})", key, p.path),
                message,
                severity: Severity::Warning,
            }),
        }
    }

    apply_legacy_fixups(map, &mut out);
    // Stable order so a migration's diff (and its tests) don't depend on
    // schema declaration order changing.
    out.shared.sort_by(|a, b| a.0.cmp(&b.0));
    out.warnings.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Legacy blob keys that no longer exist under their old name.
///
/// `embedLinks: true|false` became the three-way `linkOpen`. The fixup only
/// fires when `linkOpen` did not already land — otherwise a blob carrying both
/// would regress to the older key. Losing this would silently reset every
/// user who set the boolean and never touched the new dropdown (plan Finding 5).
#[allow(dead_code)] // GUI-only; see GuiMigration.
fn apply_legacy_fixups(map: &JsonMap<String, Json>, out: &mut GuiMigration) {
    let link_prop = schema::prop("browser.link_open").expect("link_open is in the schema");
    let link_key = link_prop.gui_key.expect("link_open has a gui key");
    if out.gui_local.contains_key(link_key) {
        return;
    }
    if let Some(Json::Bool(embed)) = map.get("embedLinks") {
        out.gui_local.insert(
            link_key.to_string(),
            Json::String(if *embed { "embedded" } else { "external" }.to_string()),
        );
    }
}

/// Coerce one JSON value into the property's declared type, applying the
/// schema's own constraints.
///
/// The single validation path for anything arriving as JSON: the one-shot blob
/// migration *and* every live write from the GUI Settings panel. Keeping both on
/// this function is what makes "one validator, derived from the schema" true
/// rather than aspirational.
///
/// Deliberately *rejects* out-of-range values rather than clamping them, which
/// is what the hand-written JS checks did: a nonsense value falls back to the
/// default instead of being silently pulled to a boundary.
pub fn coerce_json(p: &Prop, raw: &Json) -> Result<toml::Value, String> {
    match p.kind {
        Kind::Bool => raw
            .as_bool()
            .map(toml::Value::Boolean)
            .ok_or_else(|| format!("expected a boolean, got {}", json_type(raw))),
        Kind::Int { min, max } => {
            let n = finite_number(raw)?;
            if n < min as f64 || n > max as f64 {
                return Err(format!("must be between {} and {} (got {})", min, max, n));
            }
            // JS stores every number as a double; the hand-written checks
            // rounded before use, so 13.6 must land on 14, not fail.
            Ok(toml::Value::Integer(n.round() as i64))
        }
        Kind::Float { min, max } => {
            let n = finite_number(raw)?;
            if n < min || n > max {
                return Err(format!("must be between {} and {} (got {})", min, max, n));
            }
            Ok(toml::Value::Float(n))
        }
        Kind::Str => {
            let s = raw
                .as_str()
                .ok_or_else(|| format!("expected a string, got {}", json_type(raw)))?
                .trim();
            // An empty value is meaningful only where the default is empty
            // (`termShell` = "$SHELL", `tuiTerminal` = auto-detect). Where the
            // default is a real value, empty is a corrupt blob.
            if s.is_empty() && !matches!(p.default, schema::Val::Str("")) {
                return Err("must not be empty".to_string());
            }
            Ok(toml::Value::String(s.to_string()))
        }
        Kind::Path => raw
            .as_str()
            .map(|s| toml::Value::String(s.to_string()))
            .ok_or_else(|| format!("expected a path string, got {}", json_type(raw))),
        Kind::Enum(allowed) => {
            // The JS did `String(value)` before the whitelist check, so a
            // numeric font weight (500) is a legitimate spelling of "500".
            let s = match raw {
                Json::String(s) => s.clone(),
                Json::Number(n) => n.to_string(),
                Json::Bool(b) => b.to_string(),
                other => return Err(format!("expected a string, got {}", json_type(other))),
            };
            if allowed.contains(&s.as_str()) {
                Ok(toml::Value::String(s))
            } else {
                Err(format!(
                    "must be one of {} (got {:?})",
                    allowed.join(", "),
                    s
                ))
            }
        }
    }
}

fn finite_number(raw: &Json) -> Result<f64, String> {
    match raw.as_f64() {
        Some(n) if n.is_finite() => Ok(n),
        _ => Err(format!("expected a number, got {}", json_type(raw))),
    }
}

fn json_type(v: &Json) -> &'static str {
    match v {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::String(_) => "a string",
        Json::Array(_) => "an array",
        Json::Object(_) => "an object",
    }
}

/// Convert a validated scalar back to JSON for the frontend.
#[allow(dead_code)] // GUI-only; see GuiMigration.
fn toml_to_json(v: &toml::Value) -> Json {
    match v {
        toml::Value::Boolean(b) => Json::Bool(*b),
        toml::Value::Integer(i) => Json::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        toml::Value::String(s) => Json::String(s.clone()),
        other => Json::String(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shared_of(m: &GuiMigration, path: &str) -> Option<toml::Value> {
        m.shared
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, v)| v.clone())
    }

    // ── v1 → v2 ─────────────────────────────────────────────────────

    #[test]
    fn v1_flat_config_migrates_into_sections() {
        let mut table: toml::Table = toml::from_str(
            r#"
            claude_bin = "/opt/claude"
            debounce_ms = 350
            scratch_dir = "/tmp/notes"
            [[ides]]
            name = "VS Code"
            command = "code"
        "#,
        )
        .unwrap();
        assert!(migrate_table(&mut table));

        assert_eq!(
            layers::get_path(&table, "general.claude_bin"),
            Some(&toml::Value::String("/opt/claude".into()))
        );
        assert_eq!(
            layers::get_path(&table, "general.debounce_ms"),
            Some(&toml::Value::Integer(350))
        );
        assert_eq!(
            layers::get_path(&table, "paths.scratch_dir"),
            Some(&toml::Value::String("/tmp/notes".into()))
        );
        // Flat keys are gone, `[[ides]]` untouched, version stamped.
        assert!(table.get("claude_bin").is_none());
        assert!(table.get("ides").is_some());
        assert_eq!(table_version(&table), SCHEMA_VERSION);
        // A v1 file that has been migrated validates clean.
        assert!(schema::validate_table(&table).is_empty());
    }

    #[test]
    fn migrate_table_is_idempotent() {
        let mut table: toml::Table = toml::from_str("claude_bin = \"x\"").unwrap();
        assert!(migrate_table(&mut table));
        let once = table.clone();
        assert!(!migrate_table(&mut table), "second run must be a no-op");
        assert_eq!(table, once);
    }

    #[test]
    fn a_key_present_in_both_shapes_keeps_the_namespaced_value() {
        let mut table: toml::Table =
            toml::from_str("claude_bin = \"old\"\n[general]\nclaude_bin = \"new\"").unwrap();
        migrate_table(&mut table);
        assert_eq!(
            layers::get_path(&table, "general.claude_bin"),
            Some(&toml::Value::String("new".into()))
        );
    }

    #[test]
    fn document_migration_preserves_comments() {
        let mut document: toml_edit::DocumentMut = r#"# my notes
claude_bin = "/opt/claude"  # the good one
debounce_ms = 350
"#
        .parse()
        .unwrap();
        assert!(migrate_document(&mut document));
        let text = document.to_string();
        assert!(text.contains("# my notes"), "{}", text);
        assert!(text.contains("# the good one"), "{}", text);
        assert!(text.contains("[general]"), "{}", text);
        // And it parses back to the migrated shape.
        let table: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(
            layers::get_path(&table, "general.claude_bin"),
            Some(&toml::Value::String("/opt/claude".into()))
        );
        assert_eq!(table_version(&table), SCHEMA_VERSION);
        // Idempotent here too.
        assert!(!migrate_document(&mut document));
    }

    // ── GUI blob ────────────────────────────────────────────────────

    #[test]
    fn splits_shared_settings_from_gui_local_ones() {
        let blob = json!({
            "refreshSecs": 5,
            "confirmKill": false,
            "termShell": "/bin/zsh",
            "fontSize": 15,
            "theme": "nord",
        });
        let m = migrate_gui_blob(&blob);
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);

        assert_eq!(
            shared_of(&m, "sessions.refresh_secs"),
            Some(toml::Value::Integer(5))
        );
        assert_eq!(
            shared_of(&m, "sessions.confirm_kill"),
            Some(toml::Value::Boolean(false))
        );
        assert_eq!(
            shared_of(&m, "terminal.shell"),
            Some(toml::Value::String("/bin/zsh".into()))
        );
        // xterm-only keys never reach config.toml…
        assert!(shared_of(&m, "terminal.font_size").is_none());
        // …they come back for the GUI's own store instead.
        assert_eq!(m.gui_local["fontSize"], json!(15));
        assert_eq!(m.gui_local["theme"], json!("nord"));
    }

    #[test]
    fn only_keys_the_user_actually_set_are_migrated() {
        let m = migrate_gui_blob(&json!({ "refreshSecs": 4 }));
        assert_eq!(m.shared.len(), 1, "{:?}", m.shared);
        assert!(m.gui_local.is_empty());
        // A null (a JS `undefined` that round-tripped) is absent, not invalid.
        let m = migrate_gui_blob(&json!({ "refreshSecs": null }));
        assert!(m.shared.is_empty());
        assert!(m.warnings.is_empty());
    }

    #[test]
    fn out_of_range_and_wrong_typed_values_warn_and_keep_the_default() {
        let m = migrate_gui_blob(&json!({
            "refreshSecs": 99,
            "confirmKill": "yes",
            "fontSize": 4,
            "theme": "neon",
        }));
        assert!(m.shared.is_empty(), "{:?}", m.shared);
        assert!(m.gui_local.is_empty(), "{:?}", m.gui_local);
        assert_eq!(m.warnings.len(), 4);
        assert!(m.warnings.iter().all(|w| w.severity == Severity::Warning));
        assert!(m
            .warnings
            .iter()
            .any(|w| w.message.contains("between 1 and 30")));
        assert!(m
            .warnings
            .iter()
            .any(|w| w.message.contains("expected a boolean")));
    }

    /// JS numbers are all doubles, and the old checks rounded before use.
    #[test]
    fn fractional_numbers_round_for_integer_properties() {
        let m = migrate_gui_blob(&json!({ "refreshSecs": 4.6, "fontSize": 13.4 }));
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        assert_eq!(
            shared_of(&m, "sessions.refresh_secs"),
            Some(toml::Value::Integer(5))
        );
        assert_eq!(m.gui_local["fontSize"], json!(13));
    }

    /// Plan Finding 5: dropping this fixup silently resets every user who set
    /// the boolean and never opened the new dropdown.
    #[test]
    fn legacy_embed_links_boolean_maps_to_link_open() {
        let m = migrate_gui_blob(&json!({ "embedLinks": true }));
        assert_eq!(m.gui_local["linkOpen"], json!("embedded"));
        let m = migrate_gui_blob(&json!({ "embedLinks": false }));
        assert_eq!(m.gui_local["linkOpen"], json!("external"));
    }

    #[test]
    fn an_explicit_link_open_wins_over_the_legacy_boolean() {
        let m = migrate_gui_blob(&json!({ "embedLinks": true, "linkOpen": "ask" }));
        assert_eq!(m.gui_local["linkOpen"], json!("ask"));
        // …and an *invalid* linkOpen still falls back to the legacy boolean
        // rather than to the default, which is what the JS chain did.
        let m = migrate_gui_blob(&json!({ "embedLinks": true, "linkOpen": "nope" }));
        assert_eq!(m.gui_local["linkOpen"], json!("embedded"));
        assert_eq!(m.warnings.len(), 1);
    }

    #[test]
    fn numeric_font_weights_are_accepted_as_their_string_spelling() {
        let m = migrate_gui_blob(&json!({ "fontWeight": 500, "fontWeightBold": "800" }));
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        assert_eq!(m.gui_local["fontWeight"], json!("500"));
        assert_eq!(m.gui_local["fontWeightBold"], json!("800"));
    }

    #[test]
    fn empty_strings_are_rejected_only_where_a_default_exists() {
        // fontFamily has a real default — empty is a corrupt blob.
        let m = migrate_gui_blob(&json!({ "fontFamily": "   " }));
        assert!(m.gui_local.is_empty());
        assert_eq!(m.warnings.len(), 1);
        // termShell defaults to "" (meaning $SHELL) — empty is the real value.
        let m = migrate_gui_blob(&json!({ "termShell": "" }));
        assert!(m.warnings.is_empty());
        assert_eq!(
            shared_of(&m, "terminal.shell"),
            Some(toml::Value::String(String::new()))
        );
    }

    #[test]
    fn font_family_is_trimmed_like_the_old_check() {
        let m = migrate_gui_blob(&json!({ "fontFamily": "  Menlo, monospace  " }));
        assert_eq!(m.gui_local["fontFamily"], json!("Menlo, monospace"));
    }

    #[test]
    fn migrating_twice_produces_the_same_result() {
        let blob = json!({ "refreshSecs": 5, "fontSize": 15, "embedLinks": false });
        let first = migrate_gui_blob(&blob);
        let second = migrate_gui_blob(&blob);
        assert_eq!(first.shared, second.shared);
        assert_eq!(first.gui_local, second.gui_local);
    }

    #[test]
    fn a_junk_blob_yields_nothing_rather_than_failing() {
        assert!(migrate_gui_blob(&json!(null)).shared.is_empty());
        assert!(migrate_gui_blob(&json!("nonsense")).shared.is_empty());
        assert!(migrate_gui_blob(&json!([1, 2])).gui_local.is_empty());
    }

    /// Everything the migration emits must pass the schema it was derived
    /// from — otherwise a migration could write a `config.toml` that
    /// `--validate` then rejects.
    #[test]
    fn migrated_shared_values_validate_clean() {
        let blob = json!({
            "defaultCwd": "/work",
            "confirmKill": false,
            "refreshSecs": 9,
            "termShell": "/bin/fish",
            "tuiTerminal": "iTerm",
            "notifications": false,
            "titleAttention": false,
        });
        let m = migrate_gui_blob(&blob);
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        // All 7 shared keys made it.
        assert_eq!(m.shared.len(), 7, "{:?}", m.shared);

        let mut table = schema::defaults_table();
        for (path, value) in &m.shared {
            layers::set_path(&mut table, path, value.clone());
        }
        assert!(schema::validate_table(&table).is_empty());
    }
}
