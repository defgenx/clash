//! Claude Code hooks integration for instant session status detection.
//!
//! Instead of parsing JSONL tails and guessing status from file ages, we
//! configure Claude Code's native hook system to write status updates to
//! `~/.claude/clash/status/{session_id}`. This gives us instant, accurate
//! status transitions driven by Claude's own lifecycle events.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::entities::SessionStatus;

/// The hook script that Claude Code calls on lifecycle events.
/// It reads JSON from stdin, extracts event + session_id, and writes
/// a status file atomically.
const HOOK_SCRIPT: &str = r#"#!/bin/sh
# clash status hook — called by Claude Code on lifecycle events.
# Writes session status to ~/.claude/clash/status/{session_id}.
# On /clear, inherits the previous session name with an incremented suffix.
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
dir="$HOME/.claude/clash/status"
mkdir -p "$dir"
tmp=$(mktemp "$dir/.tmp.XXXXXX")
printf '{"status":"%s","session_id":"%s"}' "$status" "$sid" > "$tmp"
mv "$tmp" "$dir/$sid"
# On SessionStart from /clear, inherit the previous session name with suffix
if [ "$event" = "SessionStart" ]; then
  source=$(printf '%s' "$input" | grep -o '"source":"[^"]*"' | head -1 | cut -d'"' -f4)
  if [ "$source" = "clear" ]; then
    cwd=$(printf '%s' "$input" | grep -o '"cwd":"[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -n "$cwd" ]; then
      encoded_cwd=$(printf '%s' "$cwd" | tr '/' '-')
      pdir="$HOME/.claude/clash/project-names"
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
        ndir="$HOME/.claude/clash/names"
        mkdir -p "$ndir"
        ntmp=$(mktemp "$ndir/.tmp.XXXXXX")
        printf '%s' "$new_name" > "$ntmp"
        mv "$ntmp" "$ndir/$sid"
        ptmp=$(mktemp "$pdir/.tmp.XXXXXX")
        printf '%s' "$new_name" > "$ptmp"
        mv "$ptmp" "$pdir/$encoded_cwd"
      fi
    fi
  fi
fi
"#;

/// Directory under ~/.claude/clash/ where status files are written.
const STATUS_DIR: &str = "clash/status";
/// Directory under ~/.claude/clash/ where session names are persisted.
const NAMES_DIR: &str = "clash/names";
/// Directory under ~/.claude/clash/ where project→name mappings live.
const PROJECT_NAMES_DIR: &str = "clash/project-names";
/// Directory under ~/.claude/clash/ where the hook script lives.
const HOOKS_DIR: &str = "clash/hooks";
/// Name of the hook script file.
const HOOK_SCRIPT_NAME: &str = "status-hook.sh";

/// Install the clash hook script and merge hook config into Claude Code settings.
/// Safe to call multiple times — idempotent.
pub fn install_hooks(claude_dir: &Path) -> std::io::Result<()> {
    // 1. Write the hook script
    let hooks_dir = claude_dir.join(HOOKS_DIR);
    std::fs::create_dir_all(&hooks_dir)?;
    let script_path = hooks_dir.join(HOOK_SCRIPT_NAME);
    std::fs::write(&script_path, HOOK_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // 2. Ensure status, names, and project-names directories exist
    let status_dir = claude_dir.join(STATUS_DIR);
    std::fs::create_dir_all(&status_dir)?;
    let names_dir = claude_dir.join(NAMES_DIR);
    std::fs::create_dir_all(&names_dir)?;
    let project_names_dir = claude_dir.join(PROJECT_NAMES_DIR);
    std::fs::create_dir_all(&project_names_dir)?;

    // 3. Merge hooks into settings.local.json
    let settings_path = claude_dir.join("settings.local.json");
    merge_hook_settings(&settings_path, &script_path)?;

    Ok(())
}

/// Get the path to the status directory.
pub fn status_dir(claude_dir: &Path) -> PathBuf {
    claude_dir.join(STATUS_DIR)
}

/// Save a session name to disk so it survives daemon restarts.
/// Also persists a project→name mapping so the hook script can inherit
/// the name when `/clear` creates a new session in the same project.
pub fn save_session_name(claude_dir: &Path, session_id: &str, name: &str, cwd: Option<&str>) {
    let dir = claude_dir.join(NAMES_DIR);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(session_id);
    let _ = std::fs::write(path, name);

    // Also write project→name mapping for hook-based name inheritance
    if let Some(cwd) = cwd {
        let pdir = claude_dir.join(PROJECT_NAMES_DIR);
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
pub fn read_all_session_names(claude_dir: &Path) -> HashMap<String, String> {
    let dir = claude_dir.join(NAMES_DIR);
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
pub fn write_session_status(claude_dir: &Path, session_id: &str, status: &str) {
    let dir = claude_dir.join(STATUS_DIR);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(session_id);
    let json = format!(r#"{{"status":"{}","session_id":"{}"}}"#, status, session_id);
    let _ = crate::infrastructure::fs::atomic::write_atomic(&path, json.as_bytes());
}

/// Read all session statuses from the status directory.
pub fn read_all_statuses(claude_dir: &Path) -> HashMap<String, SessionStatus> {
    let dir = claude_dir.join(STATUS_DIR);
    let mut statuses = HashMap::new();

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return statuses,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        // Skip temp files and non-files
        if !path.is_file() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                let status_str = val.get("status").and_then(|s| s.as_str()).unwrap_or("");
                let session_id = val.get("session_id").and_then(|s| s.as_str()).unwrap_or("");
                if !session_id.is_empty() {
                    if let Ok(status) = status_str.parse::<SessionStatus>() {
                        statuses.insert(session_id.to_string(), status);
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
    // Use atomic write to avoid partial reads
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
    fn test_install_hooks_creates_files() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path();

        install_hooks(claude_dir).unwrap();

        // Hook script exists and is executable
        let script = claude_dir.join(HOOKS_DIR).join(HOOK_SCRIPT_NAME);
        assert!(script.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&script).unwrap().permissions();
            assert_eq!(perms.mode() & 0o111, 0o111);
        }

        // Status directory exists
        assert!(claude_dir.join(STATUS_DIR).exists());

        // Settings file has hooks
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
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path();
        let status_dir = claude_dir.join(STATUS_DIR);
        std::fs::create_dir_all(&status_dir).unwrap();

        std::fs::write(
            status_dir.join("session-1"),
            r#"{"status":"thinking","session_id":"session-1"}"#,
        )
        .unwrap();
        std::fs::write(
            status_dir.join("session-2"),
            r#"{"status":"prompting","session_id":"session-2"}"#,
        )
        .unwrap();
        std::fs::write(
            status_dir.join("session-3"),
            r#"{"status":"idle","session_id":"session-3"}"#,
        )
        .unwrap();

        let statuses = read_all_statuses(claude_dir);
        assert_eq!(statuses.len(), 3);
        assert_eq!(statuses["session-1"], SessionStatus::Thinking);
        assert_eq!(statuses["session-2"], SessionStatus::Prompting);
        assert_eq!(statuses["session-3"], SessionStatus::Idle);
    }

    #[test]
    fn test_save_session_name_writes_project_mapping() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path();

        save_session_name(
            claude_dir,
            "sess-1",
            "my-feature",
            Some("/Users/me/project"),
        );

        // Session name file exists
        let name = std::fs::read_to_string(claude_dir.join(NAMES_DIR).join("sess-1")).unwrap();
        assert_eq!(name, "my-feature");

        // Project name mapping exists
        let project_name =
            std::fs::read_to_string(claude_dir.join(PROJECT_NAMES_DIR).join("-Users-me-project"))
                .unwrap();
        assert_eq!(project_name, "my-feature");
    }

    #[test]
    fn test_save_session_name_without_cwd() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path();

        save_session_name(claude_dir, "sess-1", "my-feature", None);

        // Session name file exists
        let name = std::fs::read_to_string(claude_dir.join(NAMES_DIR).join("sess-1")).unwrap();
        assert_eq!(name, "my-feature");

        // No project-names directory created
        assert!(!claude_dir.join(PROJECT_NAMES_DIR).exists());
    }

    #[test]
    fn test_encode_cwd() {
        assert_eq!(encode_cwd("/Users/me/project"), "-Users-me-project");
        assert_eq!(encode_cwd("/"), "-");
        assert_eq!(encode_cwd("no-slash"), "no-slash");
    }

    #[test]
    fn test_install_hooks_creates_project_names_dir() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path();

        install_hooks(claude_dir).unwrap();

        assert!(claude_dir.join(PROJECT_NAMES_DIR).exists());
    }

    #[test]
    fn test_hook_script_contains_name_inheritance() {
        // Verify the hook script has the /clear name inheritance logic
        assert!(HOOK_SCRIPT.contains("source"));
        assert!(HOOK_SCRIPT.contains("clear"));
        assert!(HOOK_SCRIPT.contains("project-names"));
        assert!(HOOK_SCRIPT.contains("new_suffix"));
    }
}
