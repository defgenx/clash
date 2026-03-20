//! Repo-level configuration discovery.
//!
//! Unlike other modules in `infrastructure/fs/`, this reads from **project directories**
//! (the session's working directory), not from `~/.claude/`. It discovers configuration
//! files used by SuperSet and other agent orchestration tools.

use std::collections::HashMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::domain::entities::RepoConfig;

// ── Private intermediate types for typed JSON parsing ───────────────

/// Mirrors `.superset/config.json`.
#[derive(Default, serde::Deserialize)]
struct SupersetFileConfig {
    #[serde(default)]
    setup: Vec<String>,
    #[serde(default)]
    teardown: Vec<String>,
}

/// Mirrors `.mcp.json`.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpFileConfig {
    #[serde(default)]
    mcp_servers: HashMap<String, serde_json::Value>,
}

// ── Public API ──────────────────────────────────────────────────────

/// Discover repo-level configuration from a project directory.
///
/// Reads `.superset/config.json`, `.mcp.json`, `.agents/commands/`, `.claude/agents/`,
/// and `.claude/settings.json`. Each sub-read is independently guarded — partial results
/// are returned on failure, never an error.
pub fn load_repo_config(cwd: &Path) -> RepoConfig {
    let mut config = RepoConfig::default();

    // 1. .superset/config.json → setup/teardown scripts
    let superset_path = cwd.join(".superset/config.json");
    if superset_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&superset_path) {
            match serde_json::from_str::<SupersetFileConfig>(&content) {
                Ok(sc) => {
                    config.setup_scripts = sc.setup;
                    config.teardown_scripts = sc.teardown;
                }
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {}", superset_path.display(), e);
                }
            }
        }
    }

    // 2. .mcp.json → MCP server names + path
    let mcp_path = cwd.join(".mcp.json");
    if mcp_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&mcp_path) {
            match serde_json::from_str::<McpFileConfig>(&content) {
                Ok(mc) => {
                    let mut names: Vec<String> = mc.mcp_servers.keys().cloned().collect();
                    names.sort();
                    config.mcp_servers = names;
                    config.mcp_config_path = Some(
                        mcp_path
                            .canonicalize()
                            .unwrap_or(mcp_path)
                            .to_string_lossy()
                            .to_string(),
                    );
                }
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {}", mcp_path.display(), e);
                }
            }
        }
    }

    // 3. .agents/commands/*.md + .claude/commands/*.md → custom command names
    for dir in &[".agents/commands", ".claude/commands"] {
        let commands_dir = cwd.join(dir);
        if let Ok(entries) = std::fs::read_dir(&commands_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if !config.custom_commands.contains(&stem.to_string()) {
                            config.custom_commands.push(stem.to_string());
                        }
                    }
                }
            }
        }
    }
    config.custom_commands.sort();

    // 4. .claude/agents/*.md → agent definition names
    let agents_dir = cwd.join(".claude/agents");
    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    config.agent_definitions.push(stem.to_string());
                }
            }
        }
    }
    config.agent_definitions.sort();

    // 5. .claude/settings.json → existence check
    config.has_claude_settings = cwd.join(".claude/settings.json").exists();

    config
}

// ── Trust / Consent Mechanism ───────────────────────────────────────

/// Environment variable keys that must never be overridden by repo config.
/// Matches the PTY whitelist in `daemon/session.rs`.
#[allow(dead_code)]
const BLOCKED_ENV_KEYS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "SSH_AUTH_SOCK",
    "ANTHROPIC_API_KEY",
    "DISPLAY",
    "COLORTERM",
    "TERM_PROGRAM",
];

/// Check if a given env var key is blocked from repo config override.
#[allow(dead_code)]
pub fn is_env_blocked(key: &str) -> bool {
    BLOCKED_ENV_KEYS.contains(&key)
}

/// Path to the trust store file.
#[allow(dead_code)]
fn trust_store_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".claude/clash/trusted_repos.json")
}

/// Compute a SHA256 hash of the concatenated contents of scripts relative to cwd.
#[allow(dead_code)]
fn compute_scripts_hash(cwd: &Path, scripts: &[String]) -> String {
    let mut hasher = Sha256::new();
    for script in scripts {
        let path = cwd.join(script);
        if let Ok(content) = std::fs::read(&path) {
            hasher.update(&content);
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Check if a repo's setup scripts are trusted.
///
/// Trusted = repo path + SHA256 hash of setup script contents both match
/// a previously stored entry. If scripts change, trust is revoked.
#[allow(dead_code)]
pub fn is_repo_trusted(cwd: &Path, scripts: &[String]) -> bool {
    is_repo_trusted_with_store(cwd, scripts, &trust_store_path())
}

#[allow(dead_code)]
fn is_repo_trusted_with_store(
    cwd: &Path,
    scripts: &[String],
    store_path: &std::path::Path,
) -> bool {
    if scripts.is_empty() {
        return true;
    }
    let store: HashMap<String, String> = std::fs::read_to_string(store_path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();

    let key = cwd
        .canonicalize()
        .unwrap_or_else(|_| cwd.to_path_buf())
        .to_string_lossy()
        .to_string();
    let current_hash = compute_scripts_hash(cwd, scripts);

    store.get(&key) == Some(&current_hash)
}

/// Mark a repo's current setup scripts as trusted.
#[allow(dead_code)]
pub fn mark_repo_trusted(cwd: &Path, scripts: &[String]) {
    mark_repo_trusted_with_store(cwd, scripts, &trust_store_path());
}

#[allow(dead_code)]
fn mark_repo_trusted_with_store(cwd: &Path, scripts: &[String], store_path: &std::path::Path) {
    let mut store: HashMap<String, String> = std::fs::read_to_string(store_path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();

    let key = cwd
        .canonicalize()
        .unwrap_or_else(|_| cwd.to_path_buf())
        .to_string_lossy()
        .to_string();
    let hash = compute_scripts_hash(cwd, scripts);
    store.insert(key, hash);

    if let Ok(json) = serde_json::to_string_pretty(&store) {
        if let Some(parent) = store_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(store_path, json);
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    // ── load_repo_config happy paths ──

    #[test]
    fn test_empty_dir() {
        let dir = setup_dir();
        let config = load_repo_config(dir.path());
        assert!(config.setup_scripts.is_empty());
        assert!(config.teardown_scripts.is_empty());
        assert!(config.mcp_servers.is_empty());
        assert!(config.mcp_config_path.is_none());
        assert!(config.custom_commands.is_empty());
        assert!(config.agent_definitions.is_empty());
        assert!(!config.has_claude_settings);
    }

    #[test]
    fn test_superset_config() {
        let dir = setup_dir();
        std::fs::create_dir_all(dir.path().join(".superset")).unwrap();
        std::fs::write(
            dir.path().join(".superset/config.json"),
            r#"{"setup": ["./setup.sh"], "teardown": ["./teardown.sh"]}"#,
        )
        .unwrap();
        let config = load_repo_config(dir.path());
        assert_eq!(config.setup_scripts, vec!["./setup.sh"]);
        assert_eq!(config.teardown_scripts, vec!["./teardown.sh"]);
    }

    #[test]
    fn test_mcp_config() {
        let dir = setup_dir();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers": {"superset": {"type": "http"}, "neon": {"type": "http"}}}"#,
        )
        .unwrap();
        let config = load_repo_config(dir.path());
        assert_eq!(config.mcp_servers, vec!["neon", "superset"]); // sorted
        assert!(config.mcp_config_path.is_some());
    }

    #[test]
    fn test_commands_discovery() {
        let dir = setup_dir();
        std::fs::create_dir_all(dir.path().join(".agents/commands")).unwrap();
        std::fs::create_dir_all(dir.path().join(".claude/commands")).unwrap();
        std::fs::write(dir.path().join(".agents/commands/foo.md"), "# Foo").unwrap();
        std::fs::write(dir.path().join(".claude/commands/bar.md"), "# Bar").unwrap();
        let config = load_repo_config(dir.path());
        assert!(config.custom_commands.contains(&"foo".to_string()));
        assert!(config.custom_commands.contains(&"bar".to_string()));
    }

    #[test]
    fn test_agent_definitions() {
        let dir = setup_dir();
        std::fs::create_dir_all(dir.path().join(".claude/agents")).unwrap();
        std::fs::write(dir.path().join(".claude/agents/my-agent.md"), "# Agent").unwrap();
        let config = load_repo_config(dir.path());
        assert_eq!(config.agent_definitions, vec!["my-agent"]);
    }

    #[test]
    fn test_claude_settings_exists() {
        let dir = setup_dir();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(dir.path().join(".claude/settings.json"), "{}").unwrap();
        let config = load_repo_config(dir.path());
        assert!(config.has_claude_settings);
    }

    // ── load_repo_config edge cases ──

    #[test]
    fn test_superset_dir_without_config() {
        let dir = setup_dir();
        std::fs::create_dir_all(dir.path().join(".superset")).unwrap();
        // no config.json inside
        let config = load_repo_config(dir.path());
        assert!(config.setup_scripts.is_empty());
    }

    #[test]
    fn test_mcp_servers_null() {
        let dir = setup_dir();
        std::fs::write(dir.path().join(".mcp.json"), r#"{"mcpServers": null}"#).unwrap();
        let config = load_repo_config(dir.path());
        assert!(config.mcp_servers.is_empty());
    }

    #[test]
    fn test_non_md_files_skipped() {
        let dir = setup_dir();
        std::fs::create_dir_all(dir.path().join(".agents/commands")).unwrap();
        std::fs::write(dir.path().join(".agents/commands/foo.md"), "# Foo").unwrap();
        std::fs::write(dir.path().join(".agents/commands/bar.txt"), "# Bar").unwrap();
        std::fs::write(dir.path().join(".agents/commands/baz.json"), "{}").unwrap();
        let config = load_repo_config(dir.path());
        assert_eq!(config.custom_commands, vec!["foo"]);
    }

    #[test]
    fn test_partial_failure() {
        let dir = setup_dir();
        // valid superset config
        std::fs::create_dir_all(dir.path().join(".superset")).unwrap();
        std::fs::write(
            dir.path().join(".superset/config.json"),
            r#"{"setup": ["./run.sh"]}"#,
        )
        .unwrap();
        // malformed mcp.json
        std::fs::write(dir.path().join(".mcp.json"), "not valid json!!!").unwrap();
        let config = load_repo_config(dir.path());
        assert_eq!(config.setup_scripts, vec!["./run.sh"]); // partial success
        assert!(config.mcp_servers.is_empty()); // graceful failure
    }

    #[test]
    fn test_malformed_superset_json() {
        let dir = setup_dir();
        std::fs::create_dir_all(dir.path().join(".superset")).unwrap();
        std::fs::write(dir.path().join(".superset/config.json"), "{{{bad").unwrap();
        let config = load_repo_config(dir.path());
        assert!(config.setup_scripts.is_empty()); // no panic
    }

    // ── Trust mechanism tests ──

    #[test]
    fn test_empty_scripts_always_trusted() {
        let dir = setup_dir();
        assert!(is_repo_trusted(dir.path(), &[]));
    }

    #[test]
    fn test_untrusted_repo() {
        let dir = setup_dir();
        let store = dir.path().join("trust.json");
        std::fs::write(dir.path().join("setup.sh"), "#!/bin/sh\necho hello").unwrap();
        assert!(!is_repo_trusted_with_store(
            dir.path(),
            &["setup.sh".to_string()],
            &store
        ));
    }

    #[test]
    fn test_trusted_repo_roundtrip() {
        let dir = setup_dir();
        let store = dir.path().join("trust.json");
        let scripts = vec!["setup.sh".to_string()];
        std::fs::write(dir.path().join("setup.sh"), "#!/bin/sh\necho test").unwrap();

        mark_repo_trusted_with_store(dir.path(), &scripts, &store);
        assert!(is_repo_trusted_with_store(dir.path(), &scripts, &store));
    }

    #[test]
    fn test_trust_revoked_on_change() {
        let dir = setup_dir();
        let store = dir.path().join("trust.json");
        let scripts = vec!["setup.sh".to_string()];
        std::fs::write(dir.path().join("setup.sh"), "#!/bin/sh\necho v1").unwrap();

        mark_repo_trusted_with_store(dir.path(), &scripts, &store);
        assert!(is_repo_trusted_with_store(dir.path(), &scripts, &store));

        // Modify the script
        std::fs::write(dir.path().join("setup.sh"), "#!/bin/sh\necho v2").unwrap();
        assert!(!is_repo_trusted_with_store(dir.path(), &scripts, &store));
    }

    // ── Env var blocklist ──

    #[test]
    fn test_env_blocked() {
        assert!(is_env_blocked("PATH"));
        assert!(is_env_blocked("HOME"));
        assert!(is_env_blocked("ANTHROPIC_API_KEY"));
        assert!(!is_env_blocked("CUSTOM_VAR"));
        assert!(!is_env_blocked("MY_CONFIG"));
    }
}
