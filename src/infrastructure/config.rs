use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_claude_bin")]
    pub claude_bin: String,
    #[serde(default)]
    pub claude_dir: Option<PathBuf>,
    #[serde(default = "default_tick_rate")]
    pub tick_rate_ms: u64,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
}

fn default_claude_bin() -> String {
    "claude".to_string()
}
fn default_tick_rate() -> u64 {
    250
}
fn default_debounce_ms() -> u64 {
    200
}

impl Default for Config {
    fn default() -> Self {
        Self {
            claude_bin: default_claude_bin(),
            claude_dir: None,
            tick_rate_ms: default_tick_rate(),
            debounce_ms: default_debounce_ms(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let config_path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clash")
            .join("config.toml");

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
}
