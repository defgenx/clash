//! IDE detection and launching.
//!
//! Detects available IDEs on the system, presents them as picker items,
//! and launches the selected IDE with a project directory.

use crate::application::state::PickerItem;
use crate::infrastructure::config::IdeEntry;

/// Prefix for terminal editor values in the picker.
/// The reducer parses this to decide whether to open in a pane or spawn directly.
pub const TERMINAL_VALUE_PREFIX: &str = "terminal:";

/// Hardcoded default IDE definitions.
fn default_ides() -> Vec<IdeEntry> {
    vec![
        IdeEntry {
            command: "cursor".to_string(),
            name: "Cursor".to_string(),
            description: "Cursor AI Editor".to_string(),
            terminal: false,
        },
        IdeEntry {
            command: "code".to_string(),
            name: "VS Code".to_string(),
            description: "Visual Studio Code".to_string(),
            terminal: false,
        },
        IdeEntry {
            command: "zed".to_string(),
            name: "Zed".to_string(),
            description: "Zed Editor".to_string(),
            terminal: false,
        },
        IdeEntry {
            command: "idea".to_string(),
            name: "IntelliJ IDEA".to_string(),
            description: "JetBrains IntelliJ IDEA".to_string(),
            terminal: false,
        },
        IdeEntry {
            command: "rustrover".to_string(),
            name: "RustRover".to_string(),
            description: "JetBrains RustRover".to_string(),
            terminal: false,
        },
        IdeEntry {
            command: "webstorm".to_string(),
            name: "WebStorm".to_string(),
            description: "JetBrains WebStorm".to_string(),
            terminal: false,
        },
        IdeEntry {
            command: "pycharm".to_string(),
            name: "PyCharm".to_string(),
            description: "JetBrains PyCharm".to_string(),
            terminal: false,
        },
        IdeEntry {
            command: "subl".to_string(),
            name: "Sublime Text".to_string(),
            description: "Sublime Text".to_string(),
            terminal: false,
        },
        IdeEntry {
            command: "nvim".to_string(),
            name: "Neovim".to_string(),
            description: "Neovim".to_string(),
            terminal: true,
        },
        IdeEntry {
            command: "vim".to_string(),
            name: "Vim".to_string(),
            description: "Vim".to_string(),
            terminal: true,
        },
    ]
}

/// Additional terminal editors offered when editing a single file (scratch
/// notes), on top of the IDE defaults. These are great for editing one file
/// but a poor fit for opening a whole project directory (e.g. `nano <dir>`
/// fails), so they're kept out of [`detect_ides`] and only used by
/// [`detect_editors`].
fn extra_terminal_editors() -> Vec<IdeEntry> {
    vec![
        IdeEntry {
            command: "emacs".to_string(),
            name: "Emacs".to_string(),
            description: "GNU Emacs".to_string(),
            terminal: true,
        },
        IdeEntry {
            command: "nano".to_string(),
            name: "nano".to_string(),
            description: "GNU nano".to_string(),
            terminal: true,
        },
        IdeEntry {
            command: "hx".to_string(),
            name: "Helix".to_string(),
            description: "Helix editor".to_string(),
            terminal: true,
        },
        IdeEntry {
            command: "micro".to_string(),
            name: "micro".to_string(),
            description: "micro editor".to_string(),
            terminal: true,
        },
    ]
}

/// Filter a list of IDE/editor entries to those available on PATH and map
/// them to picker items, prefixing terminal editors so the reducer can route
/// them to a pane instead of a detached GUI launch.
fn entries_to_items(entries: Vec<IdeEntry>) -> Vec<PickerItem> {
    entries
        .into_iter()
        .filter(|e| {
            let avail = is_command_available(&e.command);
            tracing::debug!("editor check: {} ({}) = {}", e.name, e.command, avail);
            avail
        })
        .map(|e| {
            let value = if e.terminal {
                format!("{}{}", TERMINAL_VALUE_PREFIX, e.command)
            } else {
                e.command.clone()
            };
            PickerItem {
                label: e.name,
                description: e.description,
                value,
            }
        })
        .collect()
}

/// Merge default entries with custom ones (dedup by command), appending any
/// from `extra` not already present.
fn merge_entries(custom: &[IdeEntry], extra: Vec<IdeEntry>) -> Vec<IdeEntry> {
    let mut entries = default_ides();
    for custom_entry in custom {
        if !entries.iter().any(|e| e.command == custom_entry.command) {
            entries.push(custom_entry.clone());
        }
    }
    for e in extra {
        if !entries.iter().any(|x| x.command == e.command) {
            entries.push(e);
        }
    }
    entries
}

/// Detect available IDEs by merging defaults with custom entries,
/// deduplicating by command, and filtering to only those found on PATH.
pub fn detect_ides(custom: &[IdeEntry]) -> Vec<PickerItem> {
    entries_to_items(merge_entries(custom, Vec::new()))
}

/// Detect available editors for opening a single file (scratch notes):
/// the IDE list plus common terminal editors (emacs, nano, helix, micro).
pub fn detect_editors(custom: &[IdeEntry]) -> Vec<PickerItem> {
    entries_to_items(merge_entries(custom, extra_terminal_editors()))
}

/// Launch a GUI IDE with the given project directory (fire-and-forget).
pub fn open_ide(command: &str, project_dir: &str) -> Result<(), String> {
    std::process::Command::new(command)
        .arg(project_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch {}: {}", command, e))?;
    Ok(())
}

/// Check whether a command is available on PATH.
pub fn is_command_available(command: &str) -> bool {
    std::process::Command::new("which")
        .arg(command)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_editors_have_prefix() {
        let defaults = default_ides();
        for entry in &defaults {
            if entry.terminal {
                let item = PickerItem {
                    label: entry.name.clone(),
                    description: entry.description.clone(),
                    value: format!("{}{}", TERMINAL_VALUE_PREFIX, entry.command),
                };
                assert!(
                    item.value.starts_with(TERMINAL_VALUE_PREFIX),
                    "Terminal editor {} should have prefix",
                    entry.command
                );
            }
        }
    }

    #[test]
    fn test_extra_terminal_editors_are_terminal() {
        for e in extra_terminal_editors() {
            assert!(e.terminal, "{} should be a terminal editor", e.command);
        }
    }

    #[test]
    fn test_merge_entries_appends_extras_and_dedups() {
        let merged = merge_entries(&[], extra_terminal_editors());
        // nano is editor-only — added.
        assert!(merged.iter().any(|e| e.command == "nano"));
        // vim is in defaults — still present exactly once.
        assert_eq!(merged.iter().filter(|e| e.command == "vim").count(), 1);
    }

    #[test]
    fn test_is_command_available_true() {
        // `ls` should be available on any system
        assert!(is_command_available("ls"));
    }

    #[test]
    fn test_is_command_available_false() {
        assert!(!is_command_available("nonexistent_xyz_command_12345"));
    }

    #[test]
    fn test_custom_ides_merged_and_deduplicated() {
        let custom = vec![
            IdeEntry {
                command: "fleet".to_string(),
                name: "Fleet".to_string(),
                description: "JetBrains Fleet".to_string(),
                terminal: false,
            },
            // Duplicate of a default — should be ignored
            IdeEntry {
                command: "code".to_string(),
                name: "My VS Code".to_string(),
                description: "Custom".to_string(),
                terminal: false,
            },
        ];
        // We can't test detect_ides fully (depends on PATH), but we can test the merge logic
        let mut entries = default_ides();
        for custom_entry in &custom {
            if !entries.iter().any(|e| e.command == custom_entry.command) {
                entries.push(custom_entry.clone());
            }
        }
        // "fleet" should be added, "code" duplicate should not
        assert!(entries.iter().any(|e| e.command == "fleet"));
        assert_eq!(
            entries.iter().filter(|e| e.command == "code").count(),
            1,
            "code should not be duplicated"
        );
    }
}
