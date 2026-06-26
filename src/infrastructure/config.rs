use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeEntry {
    pub command: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_claude_bin")]
    pub claude_bin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_dir: Option<PathBuf>,
    /// Override for where scratch notes are stored. When unset, defaults to
    /// `<claude_dir>/clash/scratch`. Editable from the GUI Settings panel
    /// (which writes it back here) or by hand. Declared before `ides` so the
    /// TOML serializer emits all scalar fields before the `[[ides]]` tables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratch_dir: Option<PathBuf>,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ides: Vec<IdeEntry>,
}

fn default_claude_bin() -> String {
    "claude".to_string()
}
fn default_debounce_ms() -> u64 {
    200
}

impl Default for Config {
    fn default() -> Self {
        Self {
            claude_bin: default_claude_bin(),
            claude_dir: None,
            scratch_dir: None,
            debounce_ms: default_debounce_ms(),
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

    pub fn load() -> Self {
        let config_path = Self::config_path();

        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(config) => return config,
                    Err(e) => tracing::warn!("Failed to parse config: {}", e),
                },
                Err(e) => tracing::warn!("Failed to read config: {}", e),
            }
        }

        Self::default()
    }

    pub fn claude_dir(&self) -> PathBuf {
        self.claude_dir.clone().unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".claude")
        })
    }

    /// Effective scratch-notes directory: the configured override, or the
    /// default `<claude_dir>/clash/scratch`.
    ///
    /// `dead_code` is allowed because the only non-test caller is the sibling
    /// `clash-gui` crate (Settings panel); the TUI reads the field directly
    /// when constructing the backend.
    #[allow(dead_code)]
    pub fn scratch_dir(&self) -> PathBuf {
        self.scratch_dir
            .clone()
            .unwrap_or_else(|| self.claude_dir().join("clash").join("scratch"))
    }

    /// Persist the config back to `config.toml` (atomic write). Used by the
    /// GUI Settings panel when the user changes a shared setting such as the
    /// scratch directory. Unknown fields not modeled by `Config` are not
    /// preserved across a save.
    ///
    /// `dead_code` is allowed because only the sibling `clash-gui` crate
    /// writes config; the TUI is read-only over `config.toml`.
    #[allow(dead_code)]
    pub fn save(&self) -> std::io::Result<()> {
        let toml = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::other(format!("serialize config: {}", e)))?;
        crate::infrastructure::fs::atomic::write_atomic(&Self::config_path(), toml.as_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_dir_defaults_under_claude_dir() {
        let config = Config {
            claude_dir: Some(PathBuf::from("/tmp/fake-claude")),
            scratch_dir: None,
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
            claude_dir: Some(PathBuf::from("/tmp/fake-claude")),
            scratch_dir: Some(PathBuf::from("/tmp/elsewhere/notes")),
            ..Config::default()
        };
        assert_eq!(config.scratch_dir(), PathBuf::from("/tmp/elsewhere/notes"));
    }

    #[test]
    fn serializes_scalars_before_ide_tables() {
        // TOML requires scalar fields before array-of-tables; a regression in
        // field order would make this fail to serialize.
        let config = Config {
            scratch_dir: Some(PathBuf::from("/tmp/notes")),
            ides: vec![IdeEntry {
                command: "code".into(),
                name: "VS Code".into(),
                description: String::new(),
                terminal: false,
            }],
            ..Config::default()
        };
        let toml = toml::to_string_pretty(&config).expect("serialize");
        let scratch_at = toml.find("scratch_dir").expect("scratch_dir present");
        let ides_at = toml.find("[[ides]]").expect("ides table present");
        assert!(
            scratch_at < ides_at,
            "scalars must precede tables:\n{}",
            toml
        );
    }
}
