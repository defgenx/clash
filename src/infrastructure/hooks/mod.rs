//! Claude Code hooks integration for instant session status detection.
//!
//! Hooks are registered in `~/.claude/settings.local.json` (the only file
//! clash writes inside `~/.claude/`). All clash state files (status, names,
//! hook scripts) live in clash's own data directory (`~/.claude/clash/`).

pub mod registry;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::entities::SessionStatus;
use crate::infrastructure::config::Config;

/// Subdirectories under the clash data dir.
const STATUS_DIR: &str = "status";
const NAMES_DIR: &str = "names";
const PROJECT_NAMES_DIR: &str = "project-names";
const HOOKS_DIR: &str = "hooks";
const HOOK_SCRIPT_NAME: &str = "status-hook.sh";

/// The hook script that Claude Code calls on lifecycle events.
/// It reads JSON from stdin, extracts event + session_id, and writes
/// a status file atomically to the clash data directory.
///
/// NOTE: The DATA_DIR placeholder is replaced at install time with the
/// actual clash data directory path.
const HOOK_SCRIPT_TEMPLATE: &str = r#"#!/bin/sh
# clash status hook — called by Claude Code on lifecycle events.
# Writes session status to {DATA_DIR}/status/{session_id}.
input=$(cat)
event=$(printf '%s' "$input" | grep -o '"hook_event_name":"[^"]*"' | head -1 | cut -d'"' -f4)
sid=$(printf '%s' "$input" | grep -o '"session_id":"[^"]*"' | head -1 | cut -d'"' -f4)
[ -z "$sid" ] && exit 0
case "$event" in
  UserPromptSubmit|PostToolUse|PostToolUseFailure) status="thinking" ;;
  Stop) status="waiting" ;;
  SessionEnd) status="idle" ;;
  PermissionRequest) status="prompting" ;;
  SessionStart) status="starting" ;;
  *) exit 0 ;;
esac
dir="{DATA_DIR}/status"
mkdir -p "$dir"
tmp=$(mktemp "$dir/.tmp.XXXXXX")
printf '{"status":"%s","session_id":"%s"}' "$status" "$sid" > "$tmp"
mv "$tmp" "$dir/$sid"
# On SessionStart from /clear, inherit the previous session name with suffix
# and update the clash session registry to link the new session ID.
if [ "$event" = "SessionStart" ]; then
  source=$(printf '%s' "$input" | grep -o '"source":"[^"]*"' | head -1 | cut -d'"' -f4)
  if [ "$source" = "clear" ]; then
    cwd=$(printf '%s' "$input" | grep -o '"cwd":"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -n "$cwd" ]; then
      encoded_cwd=$(printf '%s' "$cwd" | tr '/' '-')
      pdir="{DATA_DIR}/project-names"
      old_name=""
      [ -f "$pdir/$encoded_cwd" ] && old_name=$(cat "$pdir/$encoded_cwd")
      if [ -n "$old_name" ]; then
        base=$(printf '%s' "$old_name" | sed 's/-[0-9][0-9]*$//')
        suffix=$(printf '%s' "$old_name" | grep -o -- '-[0-9][0-9]*$' | tr -d '-')
        if [ -n "$suffix" ]; then
          new_suffix=$((suffix + 1))
        else
          new_suffix=2
        fi
        new_name="${base}-${new_suffix}"
        ndir="{DATA_DIR}/names"
        mkdir -p "$ndir"
        ntmp=$(mktemp "$ndir/.tmp.XXXXXX")
        printf '%s' "$new_name" > "$ntmp"
        mv "$ntmp" "$ndir/$sid"
        ptmp=$(mktemp "$pdir/.tmp.XXXXXX")
        printf '%s' "$new_name" > "$ptmp"
        mv "$ptmp" "$pdir/$encoded_cwd"
      fi
      # Update clash session registry: find the entry matching this cwd
      # and replace its claude_session_id with the new session ID.
      reg="{DATA_DIR}/sessions.json"
      if [ -f "$reg" ]; then
        # Use a temp file for atomic update
        rtmp=$(mktemp "{DATA_DIR}/.tmp.XXXXXX")
        if command -v python3 >/dev/null 2>&1; then
          python3 -c "
import json, sys
with open('$reg') as f: reg = json.load(f)
for k, v in list(reg.items()):
    if v.get('cwd','').rstrip('/') == '$cwd'.rstrip('/'):
        v['claude_session_id'] = '$sid'
        new_entry = dict(v)
        del reg[k]
        reg['$sid'] = new_entry
        new_entry['session_id'] = '$sid'
        break
with open('$rtmp', 'w') as f: json.dump(reg, f, indent=2)
" && mv "$rtmp" "$reg" || rm -f "$rtmp"
        else
          rm -f "$rtmp"
        fi
      fi
    fi
  fi
fi
"#;

/// Get the clash data directory path.
fn clash_data_dir() -> PathBuf {
    Config::clash_data_dir()
}

/// Install the clash hook script and merge hook config into Claude Code settings.
/// Safe to call multiple times — idempotent.
///
/// - Hook script + state files go to `~/.claude/clash/` (clash's RW dir)
/// - Hook registration goes to `~/.claude/settings.local.json` (only RW in .claude)
pub fn install_hooks(claude_dir: &Path) -> std::io::Result<()> {
    let data_dir = clash_data_dir();

    // 1. Write the hook script (to clash data dir)
    let hooks_dir = data_dir.join(HOOKS_DIR);
    std::fs::create_dir_all(&hooks_dir)?;
    let script_path = hooks_dir.join(HOOK_SCRIPT_NAME);
    let script_content = HOOK_SCRIPT_TEMPLATE.replace("{DATA_DIR}", &data_dir.to_string_lossy());
    std::fs::write(&script_path, script_content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // 2. Ensure state directories exist (in clash data dir)
    std::fs::create_dir_all(data_dir.join(STATUS_DIR))?;
    std::fs::create_dir_all(data_dir.join(NAMES_DIR))?;
    std::fs::create_dir_all(data_dir.join(PROJECT_NAMES_DIR))?;

    // 3. Merge hooks into Claude Code's settings.local.json
    // (the only file clash writes inside ~/.claude/)
    let settings_path = claude_dir.join("settings.local.json");
    merge_hook_settings(&settings_path, &script_path)?;

    Ok(())
}

/// Get the path to the status directory (for FS watcher).
pub fn status_dir(_claude_dir: &Path) -> PathBuf {
    clash_data_dir().join(STATUS_DIR)
}

/// Save a session name to disk so it survives daemon restarts.
/// Also persists a project->name mapping so the hook script can inherit
/// the name when `/clear` creates a new session in the same project.
pub fn save_session_name(_claude_dir: &Path, session_id: &str, name: &str, cwd: Option<&str>) {
    let data_dir = clash_data_dir();

    let dir = data_dir.join(NAMES_DIR);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(session_id), name);

    if let Some(cwd) = cwd {
        let pdir = data_dir.join(PROJECT_NAMES_DIR);
        let _ = std::fs::create_dir_all(&pdir);
        let encoded = encode_cwd(cwd);
        let _ = std::fs::write(pdir.join(encoded), name);
    }
}

/// Encode a CWD path to a safe filename (matching the hook script's `tr '/' '-'`).
fn encode_cwd(cwd: &str) -> String {
    cwd.replace('/', "-")
}

/// Read all saved session names from disk.
pub fn read_all_session_names(_claude_dir: &Path) -> HashMap<String, String> {
    let dir = clash_data_dir().join(NAMES_DIR);
    let mut names = HashMap::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return names,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(session_id) = path.file_name().and_then(|n| n.to_str()) {
            if session_id.starts_with('.') {
                continue;
            }
            if let Ok(name) = std::fs::read_to_string(&path) {
                if !name.is_empty() {
                    names.insert(session_id.to_string(), name);
                }
            }
        }
    }
    names
}

/// Write a specific status for a session (e.g. "idle").
pub fn write_session_status(_claude_dir: &Path, session_id: &str, status: &str) {
    let dir = clash_data_dir().join(STATUS_DIR);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(session_id);
    let json = format!(r#"{{"status":"{}","session_id":"{}"}}"#, status, session_id);
    let _ = crate::infrastructure::fs::atomic::write_atomic(&path, json.as_bytes());
}

/// Read all session statuses from the status directory.
/// Returns (status, mtime) so callers can compare freshness against JSONL files.
pub fn read_all_statuses(
    _claude_dir: &Path,
) -> HashMap<String, (SessionStatus, Option<std::time::SystemTime>)> {
    let dir = clash_data_dir().join(STATUS_DIR);
    let mut statuses = HashMap::new();

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return statuses,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        let mtime = path.metadata().ok().and_then(|m| m.modified().ok());

        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                let status_str = val.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let session_id = val.get("session_id").and_then(|s| s.as_str()).unwrap_or("");
                if !session_id.is_empty() {
                    if let Ok(status) = status_str.parse::<SessionStatus>() {
                        statuses.insert(session_id.to_string(), (status, mtime));
                    }
                }
            }
        }
    }

    statuses
}

/// Merge clash hook configuration into the Claude Code settings file.
/// Preserves any existing hooks the user has configured.
fn merge_hook_settings(settings_path: &Path, script_path: &Path) -> std::io::Result<()> {
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(settings_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let script = script_path.to_string_lossy().to_string();
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert(serde_json::json!({}));

    let hook_handler = serde_json::json!({
        "type": "command",
        "command": script,
        "async": true
    });

    // Events that don't use matchers
    for event in &["UserPromptSubmit", "Stop", "SessionStart", "SessionEnd"] {
        ensure_hook(hooks, event, None, &hook_handler);
    }

    // Events that use matchers (need "*" to match all tools)
    for event in &["PostToolUse", "PostToolUseFailure", "PermissionRequest"] {
        ensure_hook(hooks, event, Some("*"), &hook_handler);
    }

    let output = serde_json::to_string_pretty(&settings)?;
    crate::infrastructure::fs::atomic::write_atomic(settings_path, output.as_bytes())
}

/// Ensure a hook handler exists in the settings for the given event.
/// Does not duplicate if the clash hook is already present.
fn ensure_hook(
    hooks: &mut serde_json::Value,
    event: &str,
    matcher: Option<&str>,
    handler: &serde_json::Value,
) {
    let command = handler
        .get("command")
        .and_then(|c| c.as_str())
        .unwrap_or("");

    let event_hooks = hooks
        .as_object_mut()
        .unwrap()
        .entry(event)
        .or_insert(serde_json::json!([]));

    let groups = match event_hooks.as_array_mut() {
        Some(a) => a,
        None => return,
    };

    // Check if our hook is already registered in any matcher group
    for group in groups.iter() {
        if let Some(handlers) = group.get("hooks").and_then(|h| h.as_array()) {
            for h in handlers {
                if h.get("command").and_then(|c| c.as_str()) == Some(command) {
                    return; // Already installed
                }
            }
        }
    }

    // Add a new matcher group with our hook
    let mut group = serde_json::json!({
        "hooks": [handler]
    });
    if let Some(m) = matcher {
        group
            .as_object_mut()
            .unwrap()
            .insert("matcher".to_string(), serde_json::json!(m));
    }
    groups.push(group);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_install_hooks_creates_settings() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path();

        install_hooks(claude_dir).unwrap();

        // Settings file has hooks (written to claude_dir)
        let settings_path = claude_dir.join("settings.local.json");
        assert!(settings_path.exists());
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(val.get("hooks").is_some());
        assert!(val["hooks"].get("Stop").is_some());
        assert!(val["hooks"].get("PostToolUse").is_some());
        assert!(val["hooks"].get("PermissionRequest").is_some());
    }

    #[test]
    fn test_install_hooks_idempotent() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path();

        install_hooks(claude_dir).unwrap();
        install_hooks(claude_dir).unwrap();

        let settings_path = claude_dir.join("settings.local.json");
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Should have exactly 1 matcher group per event, not duplicates
        let stop = val["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
    }

    #[test]
    fn test_install_preserves_existing_hooks() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path();

        // Write existing settings with a user hook
        let settings_path = claude_dir.join("settings.local.json");
        std::fs::write(
            &settings_path,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"my-hook.sh"}]}]}}"#,
        )
        .unwrap();

        install_hooks(claude_dir).unwrap();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Should have 2 matcher groups for Stop: user's + clash's
        let stop = val["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
    }

    #[test]
    fn test_read_all_statuses() {
        let data_dir = clash_data_dir();
        let status_dir = data_dir.join(STATUS_DIR);
        let _ = std::fs::create_dir_all(&status_dir);

        let test_id = format!("test-status-{}", std::process::id());
        std::fs::write(
            status_dir.join(&test_id),
            format!(r#"{{"status":"thinking","session_id":"{}"}}"#, test_id),
        )
        .unwrap();

        let statuses = read_all_statuses(Path::new(""));
        assert_eq!(statuses[&test_id].0, SessionStatus::Thinking);
        assert!(statuses[&test_id].1.is_some()); // mtime should be present

        // Cleanup
        let _ = std::fs::remove_file(status_dir.join(&test_id));
    }

    #[test]
    fn test_encode_cwd() {
        assert_eq!(encode_cwd("/Users/me/project"), "-Users-me-project");
        assert_eq!(encode_cwd("/"), "-");
        assert_eq!(encode_cwd("no-slash"), "no-slash");
    }

    #[test]
    fn test_hook_script_template_has_placeholder() {
        assert!(HOOK_SCRIPT_TEMPLATE.contains("{DATA_DIR}"));
    }

    #[test]
    fn test_merge_hook_settings_unit() {
        // Test the merge logic directly without filesystem side effects
        let dir = TempDir::new().unwrap();
        let settings_path = dir.path().join("settings.json");
        let script_path = dir.path().join("hook.sh");

        // First install
        merge_hook_settings(&settings_path, &script_path).unwrap();
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        let stop = val["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);

        // Second install — should not duplicate
        merge_hook_settings(&settings_path, &script_path).unwrap();
        let content = std::fs::read_to_string(&settings_path).unwrap();
        let val: serde_json::Value = serde_json::from_str(&content).unwrap();
        let stop = val["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
    }
}
