//! Claude Code hooks integration for instant session status detection.
//!
//! Everything clash needs lives in clash's own data directory
//! (`~/.claude/clash/`): the status/name state files, the hook script, and
//! the settings file that registers it. Sessions get the hooks because the
//! daemon passes `--settings <that file>` when it spawns `claude` — clash
//! writes no file it does not own.
//!
//! Registration used to be merged into `~/.claude/settings.local.json`.
//! Claude Code does not load that file: the settings it reads are
//! `~/.claude/settings.json` plus the project's `.claude/settings.json` and
//! `.claude/settings.local.json`. A hook registered there therefore never
//! fired, and clash never learned a session had started thinking, gone idle
//! or blocked on a permission prompt — every row stayed pinned at the
//! `starting` clash itself wrote at spawn. `--settings` is immune to that:
//! the path is explicit, so which files Claude Code searches cannot matter.
//! See `docs/hooks.md`.

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
/// Settings file clash owns outright and hands to `claude --settings`.
const HOOK_SETTINGS_NAME: &str = "settings.json";

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
        # Record the old key in the lineage so a stale id persisted elsewhere
        # (e.g. a GUI workspace pane) resolves forward to this new session.
        prev = list(new_entry.get('previous_ids') or [])
        if k not in prev and k != '$sid':
            prev.append(k)
        new_entry['previous_ids'] = prev
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

/// Install the clash hook script and the settings file that registers it.
/// Safe to call multiple times — idempotent.
///
/// Both go to clash's own data dir (`~/.claude/clash/hooks/`); the daemon
/// points `claude --settings` at the settings file. `claude_dir` is only
/// read from, to clear the registration older versions left in
/// `settings.local.json`.
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

    // 3. Write the settings file the daemon hands to `--settings`. clash owns
    //    it outright, so it is generated rather than merged — but only when
    //    the bytes actually change, because the config dir is watched and a
    //    rewrite per startup would wake the watcher for nothing.
    let settings_path = hook_settings_path();
    let desired = serde_json::to_string_pretty(&build_hook_settings(&script_path))?;
    let current = std::fs::read_to_string(&settings_path).unwrap_or_default();
    if current != desired {
        crate::infrastructure::fs::atomic::write_atomic(&settings_path, desired.as_bytes())?;
    }

    // 4. Withdraw the registration older versions merged into
    //    `~/.claude/settings.local.json`. Claude Code never reads it, so the
    //    entries are dead weight — and leaving them would have a downgraded
    //    clash silently depend on a file that does nothing.
    remove_legacy_hook_settings(claude_dir);

    Ok(())
}

/// Path of the settings file that registers clash's hooks — what the daemon
/// passes to `claude --settings`.
pub fn hook_settings_path() -> PathBuf {
    clash_data_dir().join(HOOKS_DIR).join(HOOK_SETTINGS_NAME)
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

/// Marker file name for sessions stashed during quit.
/// Written atomically before process termination so that startup can trust it
/// even if dying Claude processes overwrite the status files.
const QUIT_STASHED_FILE: &str = "quit_stashed.json";

/// Write a marker file listing session IDs that were stashed during quit.
/// This survives the race where dying Claude processes overwrite "idle" with "waiting".
pub fn write_quit_stashed(session_ids: &[String]) {
    let path = clash_data_dir().join(QUIT_STASHED_FILE);
    if let Ok(json) = serde_json::to_string(session_ids) {
        let _ = crate::infrastructure::fs::atomic::write_atomic(&path, json.as_bytes());
    }
}

/// Read and delete the quit stash marker file. Returns session IDs that were
/// stashed during the previous quit, or an empty vec if no marker exists.
pub fn take_quit_stashed() -> Vec<String> {
    let path = clash_data_dir().join(QUIT_STASHED_FILE);
    let ids = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    // Clean up marker file
    let _ = std::fs::remove_file(&path);
    ids
}

/// The hook registration clash hands to `claude --settings`.
///
/// Pure — the whole file, generated from the script path. `--settings`
/// merges with the settings Claude Code loads on its own rather than
/// replacing them, so naming only clash's own handlers here leaves the
/// user's hooks, permissions and env untouched.
fn build_hook_settings(script_path: &Path) -> serde_json::Value {
    let handler = serde_json::json!({
        "type": "command",
        "command": script_path.to_string_lossy(),
        "async": true
    });

    let mut hooks = serde_json::Map::new();
    // Events that don't use matchers.
    for event in ["UserPromptSubmit", "Stop", "SessionStart", "SessionEnd"] {
        hooks.insert(
            event.to_string(),
            serde_json::json!([{ "hooks": [handler] }]),
        );
    }
    // Events that use matchers (need "*" to match all tools).
    for event in ["PostToolUse", "PostToolUseFailure", "PermissionRequest"] {
        hooks.insert(
            event.to_string(),
            serde_json::json!([{ "matcher": "*", "hooks": [handler] }]),
        );
    }

    serde_json::json!({ "hooks": hooks })
}

/// True when `command` is some clash `status-hook.sh`.
///
/// Matches on the trailing `hooks/status-hook.sh` under a `clash` directory
/// rather than on the current data dir, so an entry left by a run with a
/// different data dir (the GUI's app-support path, an isolated `HOME`) is
/// recognised as clash's own and cleaned up too.
fn is_clash_hook_command(command: &str) -> bool {
    command.contains("clash/hooks/") && command.ends_with(HOOK_SCRIPT_NAME)
}

/// Strip every clash hook handler from a parsed settings document, pruning
/// the groups, events and the `hooks` key itself once they are empty.
/// Returns whether anything changed.
///
/// Pure, and deliberately surgical: this edits a file whose other contents
/// (a user's `permissions`, `env`, their own hooks) are none of clash's
/// business.
fn strip_clash_hooks(settings: &mut serde_json::Value) -> bool {
    let Some(root) = settings.as_object_mut() else {
        return false;
    };
    let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return false;
    };

    let mut changed = false;
    let events: Vec<String> = hooks.keys().cloned().collect();
    for event in events {
        let Some(groups) = hooks.get_mut(&event).and_then(|g| g.as_array_mut()) else {
            continue;
        };
        for group in groups.iter_mut() {
            let Some(handlers) = group.get_mut("hooks").and_then(|h| h.as_array_mut()) else {
                continue;
            };
            let before = handlers.len();
            handlers.retain(|h| {
                !h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(is_clash_hook_command)
            });
            changed |= handlers.len() != before;
        }
        // A group whose handler list we emptied is ours; drop it.
        groups.retain(|g| {
            g.get("hooks")
                .and_then(|h| h.as_array())
                .is_none_or(|h| !h.is_empty())
        });
        if groups.is_empty() {
            hooks.remove(&event);
            changed = true;
        }
    }

    if hooks.is_empty() {
        root.remove("hooks");
        changed = true;
    }
    changed
}

/// Remove clash's hook registration from `~/.claude/settings.local.json`.
///
/// Best-effort: a missing or unparseable file is left exactly as it is —
/// rewriting one clash cannot read would destroy settings it does not
/// understand, and the entries being stale is harmless in itself.
fn remove_legacy_hook_settings(claude_dir: &Path) {
    let path = claude_dir.join("settings.local.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    if !strip_clash_hooks(&mut settings) {
        return;
    }
    if let Ok(output) = serde_json::to_string_pretty(&settings) {
        let _ = crate::infrastructure::fs::atomic::write_atomic(&path, output.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn install_writes_the_settings_file_the_spawn_points_at() {
        let dir = TempDir::new().unwrap();
        install_hooks(dir.path()).unwrap();

        // The file the daemon hands to `--settings` — not anything under
        // the Claude dir, which clash no longer writes.
        let settings_path = hook_settings_path();
        assert!(settings_path.is_file());
        assert!(!dir.path().join("settings.local.json").exists());

        let val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        for event in [
            "UserPromptSubmit",
            "Stop",
            "SessionStart",
            "SessionEnd",
            "PostToolUse",
            "PostToolUseFailure",
            "PermissionRequest",
        ] {
            assert!(
                val["hooks"].get(event).is_some(),
                "{event} missing from the hook settings"
            );
        }
    }

    #[test]
    fn install_is_idempotent() {
        let dir = TempDir::new().unwrap();
        install_hooks(dir.path()).unwrap();
        let first = std::fs::read_to_string(hook_settings_path()).unwrap();
        install_hooks(dir.path()).unwrap();
        let second = std::fs::read_to_string(hook_settings_path()).unwrap();
        assert_eq!(first, second);

        let val: serde_json::Value = serde_json::from_str(&second).unwrap();
        assert_eq!(val["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }

    /// `--settings` merges with the settings Claude Code loads on its own,
    /// so clash's file names only clash's handlers.
    #[test]
    fn hook_settings_name_only_clash_handlers() {
        let val = build_hook_settings(Path::new("/data/clash/hooks/status-hook.sh"));
        let stop = val["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        let handlers = stop[0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 1);
        assert_eq!(
            handlers[0]["command"].as_str().unwrap(),
            "/data/clash/hooks/status-hook.sh"
        );
        // Matcher events carry "*", matcher-less ones carry none.
        assert_eq!(val["hooks"]["PostToolUse"][0]["matcher"], "*");
        assert!(val["hooks"]["Stop"][0].get("matcher").is_none());
    }

    #[test]
    fn legacy_registration_is_withdrawn_and_nothing_else_touched() {
        let dir = TempDir::new().unwrap();
        let legacy = dir.path().join("settings.local.json");
        std::fs::write(
            &legacy,
            r#"{
              "env": {"FOO": "bar"},
              "permissions": {"allow": ["Bash(ls:*)"]},
              "hooks": {
                "Stop": [
                  {"hooks": [{"type": "command", "command": "/home/me/.claude/clash/hooks/status-hook.sh", "async": true}]},
                  {"hooks": [{"type": "command", "command": "my-own-hook.sh"}]}
                ],
                "PostToolUse": [
                  {"matcher": "*", "hooks": [{"type": "command", "command": "/somewhere/else/clash/hooks/status-hook.sh"}]}
                ]
              }
            }"#,
        )
        .unwrap();

        install_hooks(dir.path()).unwrap();

        let val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&legacy).unwrap()).unwrap();
        // The user's own hook survives; both clash entries are gone —
        // including the one written under a different data dir.
        let stop = val["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], "my-own-hook.sh");
        // An event left with nothing but clash's handler is removed outright.
        assert!(val["hooks"].get("PostToolUse").is_none());
        // Everything that was never clash's business is untouched.
        assert_eq!(val["env"]["FOO"], "bar");
        assert_eq!(val["permissions"]["allow"][0], "Bash(ls:*)");
    }

    #[test]
    fn stripping_drops_the_hooks_key_once_it_is_empty() {
        let mut val: serde_json::Value = serde_json::json!({
            "includeCoAuthoredBy": false,
            "hooks": {
                "Stop": [{"hooks": [{"command": "/x/clash/hooks/status-hook.sh"}]}]
            }
        });
        assert!(strip_clash_hooks(&mut val));
        assert!(val.get("hooks").is_none());
        assert_eq!(val["includeCoAuthoredBy"], false);
        // Nothing left to strip — a second pass reports no change, so the
        // caller never rewrites the file for nothing.
        assert!(!strip_clash_hooks(&mut val));
    }

    #[test]
    fn a_foreign_hook_named_status_hook_is_not_clash() {
        assert!(is_clash_hook_command("/a/clash/hooks/status-hook.sh"));
        assert!(is_clash_hook_command(
            "/Users/me/Library/Application Support/clash/hooks/status-hook.sh"
        ));
        assert!(!is_clash_hook_command("/a/other/hooks/status-hook.sh"));
        assert!(!is_clash_hook_command("/a/clash/hooks/something-else.sh"));
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
}
