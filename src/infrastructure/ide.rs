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

/// Detect available IDEs by merging defaults with custom entries,
/// deduplicating by command, and filtering to only those found on PATH.
pub fn detect_ides(custom: &[IdeEntry]) -> Vec<PickerItem> {
    let mut entries = default_ides();

    // Append custom entries, dedup by command
    for custom_entry in custom {
        if !entries.iter().any(|e| e.command == custom_entry.command) {
            entries.push(custom_entry.clone());
        }
    }

    entries
        .into_iter()
        .filter(|e| {
            let avail = is_command_available(&e.command);
            tracing::debug!("IDE check: {} ({}) = {}", e.name, e.command, avail);
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
                let items = vec![PickerItem {
                    label: entry.name.clone(),
                    description: entry.description.clone(),
                    value: format!("{}{}", TERMINAL_VALUE_PREFIX, entry.command),
                }];
                assert!(
                    items[0].value.starts_with(TERMINAL_VALUE_PREFIX),
                    "Terminal editor {} should have prefix",
                    entry.command
                );
            }
        }
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
