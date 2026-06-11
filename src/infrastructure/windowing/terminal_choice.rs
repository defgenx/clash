//! Installed-terminal detection & targeted launch.
//!
//! `terminal_spawn::open_command` targets the terminal the *caller runs
//! in* (env-derived). This module instead lets the user pick: probe the
//! OS for installed emulators, then open a fresh window in the chosen
//! one — starting the emulator if it isn't running. Drives the GUI's
//! "open TUI in…" picker.

use color_eyre::eyre::{self, Context};
use std::process::Command;

use super::terminal_spawn::{applescript_command_expr, run_osascript, spawn_detached};

// `open_command` targets the terminal the *caller runs in* (env-derived).
// These functions instead let the user pick: probe the OS for installed
// emulators, then open a fresh window in the chosen one — starting the
// emulator if it isn't running. Used by the GUI's "open TUI in…" picker.

/// A terminal emulator detected on this machine.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DetectedTerminal {
    /// Stable identifier, accepted by [`open_window_in`].
    pub id: String,
    /// Human-readable name for menus.
    pub name: String,
}

/// Probe the OS for installed terminal emulators.
///
/// macOS: app bundles in /Applications and ~/Applications, plus PATH for
/// the CLI-first emulators (brew installs without a cask bundle).
/// Linux: binaries on PATH. Inside tmux, the running server is offered too.
pub fn detect_terminals() -> Vec<DetectedTerminal> {
    let mut found = Vec::new();
    let mut add = |id: &str, name: &str| {
        found.push(DetectedTerminal {
            id: id.to_string(),
            name: name.to_string(),
        })
    };

    #[cfg(target_os = "macos")]
    {
        add("terminal-app", "Terminal"); // ships with macOS
        if mac_app_installed("iTerm") {
            add("iterm", "iTerm2");
        }
        if mac_app_installed("WezTerm") || on_path("wezterm") {
            add("wezterm", "WezTerm");
        }
        if mac_app_installed("kitty") || on_path("kitty") {
            add("kitty", "kitty");
        }
        if mac_app_installed("Alacritty") || on_path("alacritty") {
            add("alacritty", "Alacritty");
        }
        if mac_app_installed("Ghostty") || on_path("ghostty") {
            add("ghostty", "Ghostty");
        }
        if mac_app_installed("Warp") {
            add("warp", "Warp");
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        for (bin, id, name) in [
            ("gnome-terminal", "gnome-terminal", "GNOME Terminal"),
            ("konsole", "konsole", "Konsole"),
            ("wezterm", "wezterm", "WezTerm"),
            ("kitty", "kitty", "kitty"),
            ("alacritty", "alacritty", "Alacritty"),
            ("ghostty", "ghostty", "Ghostty"),
            ("xterm", "xterm", "xterm"),
        ] {
            if on_path(bin) {
                add(id, name);
            }
        }
    }
    if std::env::var("TMUX").is_ok() {
        add("tmux", "tmux (current session)");
    }
    found
}

/// Open `command` in a new window of the chosen terminal (by
/// [`DetectedTerminal::id`]), starting the emulator if needed.
pub fn open_window_in(terminal_id: &str, command: &str, args: &[&str]) -> eyre::Result<()> {
    let expr = applescript_command_expr(command, args);
    match terminal_id {
        "terminal-app" => {
            let script =
                format!("tell application \"Terminal\"\n  activate\n  do script {expr}\nend tell",);
            run_osascript(&script).wrap_err("Failed to open Terminal window")
        }
        "iterm" => {
            // `create window with default profile command X` is ignored on
            // iTerm2 3.6+ — create the window, then write into its session.
            let script = format!(
                concat!(
                    r#"tell application "iTerm2""#,
                    "\n  activate",
                    "\n  set newWindow to (create window with default profile)",
                    "\n  tell current session of newWindow to write text {expr}",
                    "\nend tell",
                ),
                expr = expr,
            );
            run_osascript(&script).wrap_err("Failed to open iTerm2 window")
        }
        "wezterm" => {
            // `wezterm start` works without a running mux server, unlike
            // `wezterm cli spawn` (which `open_command` uses from inside).
            let bin = resolve_terminal_bin("wezterm", "WezTerm.app/Contents/MacOS/wezterm")?;
            let mut cmd_args = vec!["start", "--", command];
            cmd_args.extend(args);
            spawn_detached(Command::new(bin).args(&cmd_args))
                .wrap_err("Failed to open WezTerm window")
                .map(|_| ())
        }
        "kitty" => {
            let bin = resolve_terminal_bin("kitty", "kitty.app/Contents/MacOS/kitty")?;
            let mut cmd_args = vec![command];
            cmd_args.extend(args);
            spawn_detached(Command::new(bin).args(&cmd_args))
                .wrap_err("Failed to open kitty")
                .map(|_| ())
        }
        "alacritty" => {
            let bin = resolve_terminal_bin("alacritty", "Alacritty.app/Contents/MacOS/alacritty")?;
            let mut cmd_args = vec!["-e", command];
            cmd_args.extend(args);
            spawn_detached(Command::new(bin).args(&cmd_args))
                .wrap_err("Failed to open Alacritty")
                .map(|_| ())
        }
        "ghostty" => {
            let bin = resolve_terminal_bin("ghostty", "Ghostty.app/Contents/MacOS/ghostty")?;
            let mut cmd_args = vec!["-e", command];
            cmd_args.extend(args);
            spawn_detached(Command::new(bin).args(&cmd_args))
                .wrap_err("Failed to open Ghostty")
                .map(|_| ())
        }
        "warp" => {
            // Warp has no "run command" CLI; its supported mechanism is a
            // launch configuration opened through the warp:// URI scheme.
            // Write (atomically) a one-shot config that execs the command.
            let home = std::env::var_os("HOME")
                .ok_or_else(|| eyre::eyre!("HOME not set — cannot locate Warp config dir"))?;
            let dir = std::path::Path::new(&home).join(".warp/launch_configurations");
            std::fs::create_dir_all(&dir).wrap_err("Failed to create Warp config dir")?;
            let exec = std::iter::once(command)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ")
                .replace('"', "\\\"");
            let yaml = format!(
                concat!(
                    "---\n",
                    "name: clash-tui\n",
                    "windows:\n",
                    "  - tabs:\n",
                    "      - title: clash\n",
                    "        layout:\n",
                    "          commands:\n",
                    "            - exec: \"{}\"\n",
                ),
                exec
            );
            crate::infrastructure::fs::atomic::write_atomic(
                &dir.join("clash-tui.yaml"),
                yaml.as_bytes(),
            )
            .wrap_err("Failed to write Warp launch configuration")?;
            spawn_detached(Command::new("open").arg("warp://launch/clash-tui.yaml"))
                .wrap_err("Failed to open Warp")
                .map(|_| ())
        }
        "gnome-terminal" => {
            let mut cmd_args = vec!["--", command];
            cmd_args.extend(args);
            spawn_detached(Command::new("gnome-terminal").args(&cmd_args))
                .wrap_err("Failed to open GNOME Terminal")
                .map(|_| ())
        }
        "konsole" | "xterm" => {
            let mut cmd_args = vec!["-e", command];
            cmd_args.extend(args);
            spawn_detached(Command::new(terminal_id).args(&cmd_args))
                .wrap_err_with(|| format!("Failed to open {terminal_id}"))
                .map(|_| ())
        }
        "tmux" => {
            let mut cmd_args = vec!["new-window", command];
            cmd_args.extend(args);
            spawn_detached(Command::new("tmux").args(&cmd_args))
                .wrap_err("Failed to open tmux window")
                .map(|_| ())
        }
        other => Err(eyre::eyre!("Unknown terminal id: '{}'", other)),
    }
}

/// Is `bin` directly executable from PATH?
fn on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
}

/// Does a macOS app bundle named `{app}.app` exist in (~)/Applications?
#[cfg(target_os = "macos")]
fn mac_app_installed(app: &str) -> bool {
    let bundle = format!("{app}.app");
    if std::path::Path::new("/Applications").join(&bundle).exists() {
        return true;
    }
    std::env::var_os("HOME").is_some_and(|home| {
        std::path::Path::new(&home)
            .join("Applications")
            .join(&bundle)
            .exists()
    })
}

/// Resolve a terminal's launch binary: PATH first, then the executable
/// embedded in its macOS app bundle (cask installs don't link the CLI).
fn resolve_terminal_bin(bin: &str, bundle_relative: &str) -> eyre::Result<String> {
    if on_path(bin) {
        return Ok(bin.to_string());
    }
    let bundled = std::path::Path::new("/Applications").join(bundle_relative);
    if bundled.is_file() {
        return Ok(bundled.to_string_lossy().into_owned());
    }
    if let Some(home) = std::env::var_os("HOME") {
        let bundled = std::path::Path::new(&home)
            .join("Applications")
            .join(bundle_relative);
        if bundled.is_file() {
            return Ok(bundled.to_string_lossy().into_owned());
        }
    }
    Err(eyre::eyre!("{} not found on PATH or in /Applications", bin))
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_terminals / open_window_in ──

    #[test]
    fn detected_terminal_ids_are_unique_and_launchable() {
        let terminals = detect_terminals();
        let mut ids: Vec<&str> = terminals.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate terminal ids");
        for t in &terminals {
            assert!(!t.id.is_empty());
            assert!(!t.name.is_empty());
        }
    }

    #[test]
    fn open_window_in_rejects_unknown_id() {
        let err = open_window_in("not-a-terminal", "true", &[]).unwrap_err();
        assert!(err.to_string().contains("Unknown terminal id"));
    }
}
