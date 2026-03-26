//! Preset loader — reads .clash/presets.json, global config, and .superset/config.json.

use std::collections::HashMap;
use std::path::Path;

use crate::domain::entities::{Preset, PresetFile, PresetSource, SupersetConfig};

/// Load and merge presets from all sources.
///
/// Precedence: project overrides global for same name; superset always included
/// unless a project preset is also named "superset".
pub fn load_presets(project_dir: &Path, global_config_dir: &Path) -> Vec<Preset> {
    let mut presets: HashMap<String, Preset> = HashMap::new();

    // 1. Global presets
    let global_path = global_config_dir.join("presets.json");
    if let Some(file) = read_preset_file(&global_path) {
        for (name, mut preset) in file.presets {
            preset.name = name.clone();
            preset.source = PresetSource::Global;
            presets.insert(name, preset);
        }
    }

    // 2. Superset compat (.superset/config.json)
    let superset_path = project_dir.join(".superset/config.json");
    if let Some(config) = read_superset_config(&superset_path) {
        let preset = superset_to_preset(&config);
        // Only insert if not already overridden by a project preset named "superset"
        presets.entry("superset".to_string()).or_insert(preset);
    }

    // 3. Project presets (.clash/presets.json) — override everything
    let project_path = project_dir.join(".clash/presets.json");
    if let Some(file) = read_preset_file(&project_path) {
        for (name, mut preset) in file.presets {
            preset.name = name.clone();
            preset.source = PresetSource::Project;
            presets.insert(name, preset);
        }
    }

    let mut result: Vec<Preset> = presets.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

fn read_preset_file(path: &Path) -> Option<PresetFile> {
    let content = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&content) {
        Ok(file) => Some(file),
        Err(e) => {
            tracing::warn!("Malformed preset file {}: {}", path.display(), e);
            None
        }
    }
}

fn read_superset_config(path: &Path) -> Option<SupersetConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&content) {
        Ok(config) => Some(config),
        Err(e) => {
            tracing::warn!("Malformed superset config {}: {}", path.display(), e);
            None
        }
    }
}

/// Translate a Superset config into a synthetic preset.
fn superset_to_preset(config: &SupersetConfig) -> Preset {
    Preset {
        name: "superset".to_string(),
        description: "From .superset/config.json".to_string(),
        directory: ".".to_string(),
        setup: config.setup.clone(),
        teardown: config.teardown.clone(),
        source: PresetSource::Superset,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn test_valid_presets_json() {
        let dir = setup_dir();
        let clash_dir = dir.path().join(".clash");
        fs::create_dir_all(&clash_dir).unwrap();
        fs::write(
            clash_dir.join("presets.json"),
            r#"{"presets": {"backend": {"description": "Backend fix", "directory": "./", "worktree": true, "setup": ["./setup.sh"]}}}"#,
        )
        .unwrap();

        let global = setup_dir();
        let presets = load_presets(dir.path(), global.path());
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "backend");
        assert_eq!(presets[0].description, "Backend fix");
        assert_eq!(presets[0].worktree, Some(true));
        assert_eq!(presets[0].setup, vec!["./setup.sh"]);
        assert!(matches!(presets[0].source, PresetSource::Project));
    }

    #[test]
    fn test_empty_file_returns_empty() {
        let dir = setup_dir();
        let global = setup_dir();
        let presets = load_presets(dir.path(), global.path());
        assert!(presets.is_empty());
    }

    #[test]
    fn test_malformed_json_returns_empty() {
        let dir = setup_dir();
        let clash_dir = dir.path().join(".clash");
        fs::create_dir_all(&clash_dir).unwrap();
        fs::write(clash_dir.join("presets.json"), "not valid json!!!").unwrap();

        let global = setup_dir();
        let presets = load_presets(dir.path(), global.path());
        assert!(presets.is_empty());
    }

    #[test]
    fn test_extra_fields_preserved() {
        let dir = setup_dir();
        let clash_dir = dir.path().join(".clash");
        fs::create_dir_all(&clash_dir).unwrap();
        fs::write(
            clash_dir.join("presets.json"),
            r#"{"presets": {"test": {"custom_field": 42}}}"#,
        )
        .unwrap();

        let global = setup_dir();
        let presets = load_presets(dir.path(), global.path());
        assert_eq!(presets.len(), 1);
        assert!(presets[0].extra.contains_key("custom_field"));
    }

    #[test]
    fn test_missing_fields_get_defaults() {
        let dir = setup_dir();
        let clash_dir = dir.path().join(".clash");
        fs::create_dir_all(&clash_dir).unwrap();
        fs::write(
            clash_dir.join("presets.json"),
            r#"{"presets": {"minimal": {}}}"#,
        )
        .unwrap();

        let global = setup_dir();
        let presets = load_presets(dir.path(), global.path());
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "minimal");
        assert!(presets[0].description.is_empty());
        assert!(presets[0].setup.is_empty());
        assert!(presets[0].worktree.is_none());
    }

    #[test]
    fn test_superset_translation() {
        let dir = setup_dir();
        let superset_dir = dir.path().join(".superset");
        fs::create_dir_all(&superset_dir).unwrap();
        fs::write(
            superset_dir.join("config.json"),
            r#"{"setup": ["./setup.sh"], "teardown": ["./teardown.sh"], "run": ["bun dev"]}"#,
        )
        .unwrap();

        let global = setup_dir();
        let presets = load_presets(dir.path(), global.path());
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "superset");
        assert_eq!(presets[0].setup, vec!["./setup.sh"]);
        assert_eq!(presets[0].teardown, vec!["./teardown.sh"]);
        assert!(matches!(presets[0].source, PresetSource::Superset));
    }

    #[test]
    fn test_merge_project_overrides_global() {
        let dir = setup_dir();
        let global = setup_dir();

        // Global preset "foo"
        fs::write(
            global.path().join("presets.json"),
            r#"{"presets": {"foo": {"description": "global foo"}}}"#,
        )
        .unwrap();

        // Project preset "foo" (should override)
        let clash_dir = dir.path().join(".clash");
        fs::create_dir_all(&clash_dir).unwrap();
        fs::write(
            clash_dir.join("presets.json"),
            r#"{"presets": {"foo": {"description": "project foo"}}}"#,
        )
        .unwrap();

        let presets = load_presets(dir.path(), global.path());
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].description, "project foo");
        assert!(matches!(presets[0].source, PresetSource::Project));
    }

    #[test]
    fn test_superset_alongside_project_presets() {
        let dir = setup_dir();
        let global = setup_dir();

        // Project preset
        let clash_dir = dir.path().join(".clash");
        fs::create_dir_all(&clash_dir).unwrap();
        fs::write(
            clash_dir.join("presets.json"),
            r#"{"presets": {"backend": {"description": "Backend fix"}}}"#,
        )
        .unwrap();

        // Superset config
        let superset_dir = dir.path().join(".superset");
        fs::create_dir_all(&superset_dir).unwrap();
        fs::write(
            superset_dir.join("config.json"),
            r#"{"setup": ["./setup.sh"]}"#,
        )
        .unwrap();

        let presets = load_presets(dir.path(), global.path());
        assert_eq!(presets.len(), 2);
        let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"backend"));
        assert!(names.contains(&"superset"));
    }
}
