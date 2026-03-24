//! Terminal emulator detection and smart pane/tab/window spawning.
//!
//! Opens sessions in split panes when the terminal supports them,
//! falling back to tabs or windows. Calculates how many panes fit
//! based on screen width, overflowing to tabs.

use color_eyre::eyre::{self, Context};
use std::process::Command;

/// Minimum columns for a usable Claude Code pane.
const MIN_PANE_COLS: u16 = 80;
/// Minimum rows for a usable Claude Code pane.
const MIN_PANE_ROWS: u16 = 24;

// ── Public types ─────────────────────────────────────────────────

/// How a session was opened.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenMode {
    Pane,
    Tab,
    Window,
}

/// Split direction for panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

/// Planned layout: how many sessions go in panes vs tabs.
#[derive(Debug, PartialEq, Eq)]
pub struct Layout {
    pub pane_count: usize,
    pub tab_count: usize,
    pub axis: Option<SplitAxis>,
}

/// Result of a batch open operation.
pub struct BatchResult {
    pub panes_opened: usize,
    pub tabs_opened: usize,
}

// ── Public API ───────────────────────────────────────────────────

/// Whether this terminal supports split panes.
#[cfg_attr(not(test), allow(dead_code))]
pub fn supports_panes(term_program: Option<&str>, in_tmux: bool) -> bool {
    let strategy = detect_strategy(term_program, in_tmux);
    strategy_supports_panes(&strategy)
}

/// Max horizontal session panes that fit alongside clash at the given terminal width.
/// Subtracts 1 for clash's own pane.
pub fn max_panes(cols: u16) -> usize {
    (cols / MIN_PANE_COLS).saturating_sub(1) as usize
}

/// Max vertical session panes that fit alongside clash at the given terminal height.
/// Subtracts 1 for clash's own pane.
pub fn max_vertical_panes(rows: u16) -> usize {
    (rows / MIN_PANE_ROWS).saturating_sub(1) as usize
}

/// Plan the layout: how many sessions go in panes vs tabs/windows.
///
/// Picks the axis that fits the most panes (maximize panes before tabs).
/// Horizontal (side-by-side) and vertical (stacked) are both considered.
pub fn plan_layout(
    count: usize,
    term_program: Option<&str>,
    in_tmux: bool,
    cols: u16,
    rows: u16,
) -> Layout {
    if count == 0 {
        return Layout {
            pane_count: 0,
            tab_count: 0,
            axis: None,
        };
    }

    let strategy = detect_strategy(term_program, in_tmux);
    if !strategy_supports_panes(&strategy) {
        return Layout {
            pane_count: 0,
            tab_count: count,
            axis: None,
        };
    }

    let max_h = max_panes(cols);
    let max_v = max_vertical_panes(rows);

    // Pick the axis that fits the most panes
    let (pane_slots, axis) = if max_h >= count {
        // Horizontal fits all — prefer it
        (count, SplitAxis::Horizontal)
    } else if max_v >= count {
        // Vertical fits all
        (count, SplitAxis::Vertical)
    } else if max_h >= max_v && max_h > 0 {
        // Neither fits all — use the larger, overflow to tabs
        (max_h, SplitAxis::Horizontal)
    } else if max_v > 0 {
        (max_v, SplitAxis::Vertical)
    } else {
        // No room for panes at all
        return Layout {
            pane_count: 0,
            tab_count: count,
            axis: None,
        };
    };

    Layout {
        pane_count: pane_slots,
        tab_count: count - pane_slots,
        axis: Some(axis),
    }
}

/// Open a single session — pane if supported and there's room, otherwise tab/window.
pub fn open_session(
    session_id: &str,
    term_program: Option<&str>,
    in_tmux: bool,
    cols: u16,
    rows: u16,
) -> eyre::Result<OpenMode> {
    let binary = resolve_binary()?;
    let strategy = detect_strategy(term_program, in_tmux);

    if strategy_supports_panes(&strategy) {
        let max_h = max_panes(cols);
        let max_v = max_vertical_panes(rows);
        if max_h >= 1 {
            spawn_pane(&binary, session_id, &strategy, SplitAxis::Horizontal)?;
            return Ok(OpenMode::Pane);
        } else if max_v >= 1 {
            spawn_pane(&binary, session_id, &strategy, SplitAxis::Vertical)?;
            return Ok(OpenMode::Pane);
        }
    }

    let mode = spawn_tab(&binary, session_id, &strategy)?;
    Ok(mode)
}

/// Open multiple sessions with smart layout: panes first, overflow to tabs.
pub fn open_batch(
    session_ids: &[String],
    term_program: Option<&str>,
    in_tmux: bool,
    cols: u16,
    rows: u16,
) -> eyre::Result<BatchResult> {
    let binary = resolve_binary()?;
    let strategy = detect_strategy(term_program, in_tmux);
    let layout = plan_layout(session_ids.len(), term_program, in_tmux, cols, rows);

    let (pane_sessions, tab_sessions) = session_ids.split_at(layout.pane_count);

    let mut result = BatchResult {
        panes_opened: 0,
        tabs_opened: 0,
    };

    let axis = layout.axis.unwrap_or(SplitAxis::Horizontal);
    for id in pane_sessions {
        if spawn_pane(&binary, id, &strategy, axis).is_ok() {
            result.panes_opened += 1;
        }
    }
    for id in tab_sessions {
        if spawn_tab(&binary, id, &strategy).is_ok() {
            result.tabs_opened += 1;
        }
    }

    Ok(result)
}

// ── Internal ─────────────────────────────────────────────────────

/// Terminal spawning strategy.
#[derive(Debug, PartialEq, Eq, Clone)]
pub(crate) enum SpawnStrategy {
    AppleTerminal,
    ITerm,
    Ghostty,
    WezTerm,
    Alacritty,
    Kitty,
    Tmux,
    /// Cross-platform fallback: macOS Terminal.app, Linux x-terminal-emulator/xterm, or direct spawn.
    Fallback,
}

/// Detect the best spawning strategy from environment context.
pub(crate) fn detect_strategy(term_program: Option<&str>, in_tmux: bool) -> SpawnStrategy {
    if in_tmux {
        return SpawnStrategy::Tmux;
    }
    match term_program {
        Some("Apple_Terminal") => SpawnStrategy::AppleTerminal,
        Some("iTerm.app" | "iTerm2") => SpawnStrategy::ITerm,
        Some("ghostty") => SpawnStrategy::Ghostty,
        Some("WezTerm") => SpawnStrategy::WezTerm,
        Some("Alacritty") => SpawnStrategy::Alacritty,
        Some("kitty") => SpawnStrategy::Kitty,
        _ => SpawnStrategy::Fallback,
    }
}

fn strategy_supports_panes(strategy: &SpawnStrategy) -> bool {
    matches!(
        strategy,
        SpawnStrategy::Tmux | SpawnStrategy::ITerm | SpawnStrategy::WezTerm | SpawnStrategy::Kitty
    )
}

fn resolve_binary() -> eyre::Result<String> {
    Ok(std::env::current_exe()
        .wrap_err("Could not determine clash binary path")?
        .to_string_lossy()
        .to_string())
}

/// Spawn a session in a split pane with the given axis.
fn spawn_pane(
    binary: &str,
    session_id: &str,
    strategy: &SpawnStrategy,
    axis: SplitAxis,
) -> eyre::Result<()> {
    match strategy {
        SpawnStrategy::Tmux => {
            let flag = match axis {
                SplitAxis::Horizontal => "-h",
                SplitAxis::Vertical => "-v",
            };
            Command::new("tmux")
                .args(["split-window", flag, binary, "attach", session_id])
                .spawn()
                .wrap_err("Failed to open tmux pane")?;
        }
        SpawnStrategy::ITerm => {
            let direction = match axis {
                SplitAxis::Horizontal => "horizontally",
                SplitAxis::Vertical => "vertically",
            };
            let script = format!(
                concat!(
                    r#"tell application "iTerm2""#,
                    "\n  tell current session of current window",
                    "\n    split {} with default profile command (quoted form of \"{}\") & \" attach \" & (quoted form of \"{}\")",
                    "\n  end tell",
                    "\nend tell",
                ),
                direction, binary, session_id,
            );
            Command::new("osascript")
                .args(["-e", &script])
                .spawn()
                .wrap_err("Failed to open iTerm2 pane")?;
        }
        SpawnStrategy::WezTerm => {
            let side = match axis {
                SplitAxis::Horizontal => "--right",
                SplitAxis::Vertical => "--bottom",
            };
            Command::new("wezterm")
                .args([
                    "cli",
                    "split-pane",
                    side,
                    "--",
                    binary,
                    "attach",
                    session_id,
                ])
                .spawn()
                .wrap_err("Failed to open WezTerm pane")?;
        }
        SpawnStrategy::Kitty => {
            // Kitty doesn't distinguish split axis — uses OS-level window split
            Command::new("kitty")
                .args(["@", "launch", "--type=window", binary, "attach", session_id])
                .spawn()
                .wrap_err("Failed to open Kitty pane")?;
        }
        _ => {
            return Err(eyre::eyre!(
                "Terminal does not support panes: {:?}",
                strategy
            ));
        }
    }
    Ok(())
}

/// Spawn a session in a new tab (or window for terminals without tab support).
/// Returns the mode used (Tab or Window).
fn spawn_tab(binary: &str, session_id: &str, strategy: &SpawnStrategy) -> eyre::Result<OpenMode> {
    match strategy {
        SpawnStrategy::AppleTerminal => {
            let script = format!(
                concat!(
                    r#"tell application "Terminal""#,
                    "\n  activate",
                    "\n  do script (quoted form of \"{}\") & \" attach \" & (quoted form of \"{}\") in front window",
                    "\nend tell",
                ),
                binary, session_id,
            );
            Command::new("osascript")
                .args(["-e", &script])
                .spawn()
                .wrap_err("Failed to open Apple Terminal tab")?;
            Ok(OpenMode::Tab)
        }
        SpawnStrategy::ITerm => {
            let script = format!(
                concat!(
                    r#"tell application "iTerm2""#,
                    "\n  tell current window",
                    "\n    create tab with default profile command (quoted form of \"{}\") & \" attach \" & (quoted form of \"{}\")",
                    "\n  end tell",
                    "\nend tell",
                ),
                binary, session_id,
            );
            Command::new("osascript")
                .args(["-e", &script])
                .spawn()
                .wrap_err("Failed to open iTerm2 tab")?;
            Ok(OpenMode::Tab)
        }
        SpawnStrategy::Ghostty => {
            Command::new("ghostty")
                .args(["-e", binary, "attach", session_id])
                .spawn()
                .wrap_err("Failed to open Ghostty window")?;
            Ok(OpenMode::Window)
        }
        SpawnStrategy::WezTerm => {
            Command::new("wezterm")
                .args(["cli", "spawn", "--", binary, "attach", session_id])
                .spawn()
                .wrap_err("Failed to open WezTerm tab")?;
            Ok(OpenMode::Tab)
        }
        SpawnStrategy::Alacritty => {
            Command::new("alacritty")
                .args(["-e", binary, "attach", session_id])
                .spawn()
                .wrap_err("Failed to open Alacritty window")?;
            Ok(OpenMode::Window)
        }
        SpawnStrategy::Kitty => {
            Command::new("kitty")
                .args(["@", "launch", "--type=tab", binary, "attach", session_id])
                .spawn()
                .wrap_err("Failed to open Kitty tab")?;
            Ok(OpenMode::Tab)
        }
        SpawnStrategy::Tmux => {
            Command::new("tmux")
                .args(["new-window", binary, "attach", session_id])
                .spawn()
                .wrap_err("Failed to open tmux window")?;
            Ok(OpenMode::Tab)
        }
        SpawnStrategy::Fallback => {
            #[cfg(target_os = "macos")]
            {
                Command::new("open")
                    .args(["-a", "Terminal", binary, "--args", "attach", session_id])
                    .spawn()
                    .wrap_err("Failed to open terminal window")?;
            }
            #[cfg(target_os = "linux")]
            {
                let result = Command::new("x-terminal-emulator")
                    .args(["-e", binary, "attach", session_id])
                    .spawn();
                if result.is_err() {
                    Command::new("xterm")
                        .args(["-e", binary, "attach", session_id])
                        .spawn()
                        .wrap_err("Failed to open terminal (tried x-terminal-emulator, xterm)")?;
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                Command::new(binary)
                    .args(["attach", session_id])
                    .spawn()
                    .wrap_err("Failed to spawn attach process")?;
            }
            Ok(OpenMode::Window)
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_strategy ──

    #[test]
    fn detect_apple_terminal() {
        assert_eq!(
            detect_strategy(Some("Apple_Terminal"), false),
            SpawnStrategy::AppleTerminal
        );
    }

    #[test]
    fn detect_iterm_app() {
        assert_eq!(
            detect_strategy(Some("iTerm.app"), false),
            SpawnStrategy::ITerm
        );
    }

    #[test]
    fn detect_iterm2() {
        assert_eq!(detect_strategy(Some("iTerm2"), false), SpawnStrategy::ITerm);
    }

    #[test]
    fn detect_ghostty() {
        assert_eq!(
            detect_strategy(Some("ghostty"), false),
            SpawnStrategy::Ghostty
        );
    }

    #[test]
    fn detect_wezterm() {
        assert_eq!(
            detect_strategy(Some("WezTerm"), false),
            SpawnStrategy::WezTerm
        );
    }

    #[test]
    fn detect_alacritty() {
        assert_eq!(
            detect_strategy(Some("Alacritty"), false),
            SpawnStrategy::Alacritty
        );
    }

    #[test]
    fn detect_kitty() {
        assert_eq!(detect_strategy(Some("kitty"), false), SpawnStrategy::Kitty);
    }

    #[test]
    fn detect_tmux_takes_priority() {
        assert_eq!(
            detect_strategy(Some("Apple_Terminal"), true),
            SpawnStrategy::Tmux
        );
    }

    #[test]
    fn detect_tmux_no_term_program() {
        assert_eq!(detect_strategy(None, true), SpawnStrategy::Tmux);
    }

    #[test]
    fn detect_unknown_falls_back() {
        assert_eq!(
            detect_strategy(Some("SomeUnknownTerminal"), false),
            SpawnStrategy::Fallback
        );
    }

    #[test]
    fn detect_none_falls_back() {
        assert_eq!(detect_strategy(None, false), SpawnStrategy::Fallback);
    }

    // ── supports_panes ──

    #[test]
    fn panes_supported_tmux() {
        assert!(supports_panes(None, true));
    }

    #[test]
    fn panes_supported_iterm() {
        assert!(supports_panes(Some("iTerm.app"), false));
    }

    #[test]
    fn panes_supported_wezterm() {
        assert!(supports_panes(Some("WezTerm"), false));
    }

    #[test]
    fn panes_supported_kitty() {
        assert!(supports_panes(Some("kitty"), false));
    }

    #[test]
    fn panes_not_supported_apple_terminal() {
        assert!(!supports_panes(Some("Apple_Terminal"), false));
    }

    #[test]
    fn panes_not_supported_ghostty() {
        assert!(!supports_panes(Some("ghostty"), false));
    }

    #[test]
    fn panes_not_supported_alacritty() {
        assert!(!supports_panes(Some("Alacritty"), false));
    }

    #[test]
    fn panes_not_supported_fallback() {
        assert!(!supports_panes(None, false));
    }

    // ── max_panes (horizontal) ──

    #[test]
    fn max_panes_160_cols() {
        assert_eq!(max_panes(160), 1);
    }

    #[test]
    fn max_panes_240_cols() {
        assert_eq!(max_panes(240), 2);
    }

    #[test]
    fn max_panes_320_cols() {
        assert_eq!(max_panes(320), 3);
    }

    #[test]
    fn max_panes_80_cols_no_room() {
        assert_eq!(max_panes(80), 0);
    }

    #[test]
    fn max_panes_0_cols() {
        assert_eq!(max_panes(0), 0);
    }

    // ── max_vertical_panes ──

    #[test]
    fn max_vertical_panes_48_rows() {
        assert_eq!(max_vertical_panes(48), 1); // 48/24 - 1 = 1
    }

    #[test]
    fn max_vertical_panes_72_rows() {
        assert_eq!(max_vertical_panes(72), 2); // 72/24 - 1 = 2
    }

    #[test]
    fn max_vertical_panes_24_rows_no_room() {
        assert_eq!(max_vertical_panes(24), 0); // 24/24 - 1 = 0
    }

    #[test]
    fn max_vertical_panes_0_rows() {
        assert_eq!(max_vertical_panes(0), 0);
    }

    // ── plan_layout ──

    #[test]
    fn layout_single_pane_horizontal() {
        let l = plan_layout(1, None, true, 160, 48); // tmux, wide enough
        assert_eq!(
            l,
            Layout {
                pane_count: 1,
                tab_count: 0,
                axis: Some(SplitAxis::Horizontal)
            }
        );
    }

    #[test]
    fn layout_overflow_to_tabs() {
        let l = plan_layout(3, None, true, 160, 48); // h=1, v=1 → h wins (tie), 2 tabs
        assert_eq!(
            l,
            Layout {
                pane_count: 1,
                tab_count: 2,
                axis: Some(SplitAxis::Horizontal)
            }
        );
    }

    #[test]
    fn layout_wider_terminal() {
        let l = plan_layout(3, None, true, 240, 48); // h=2, v=1 → h wins, 1 tab
        assert_eq!(
            l,
            Layout {
                pane_count: 2,
                tab_count: 1,
                axis: Some(SplitAxis::Horizontal)
            }
        );
    }

    #[test]
    fn layout_no_room_for_panes() {
        let l = plan_layout(1, None, true, 80, 24); // h=0, v=0 → all tabs
        assert_eq!(
            l,
            Layout {
                pane_count: 0,
                tab_count: 1,
                axis: None
            }
        );
    }

    #[test]
    fn layout_no_pane_support() {
        let l = plan_layout(2, Some("Apple_Terminal"), false, 240, 72);
        assert_eq!(
            l,
            Layout {
                pane_count: 0,
                tab_count: 2,
                axis: None
            }
        );
    }

    #[test]
    fn layout_zero_sessions() {
        let l = plan_layout(0, None, true, 240, 48);
        assert_eq!(
            l,
            Layout {
                pane_count: 0,
                tab_count: 0,
                axis: None
            }
        );
    }

    #[test]
    fn layout_many_sessions_wide_terminal() {
        let l = plan_layout(5, None, true, 320, 48); // h=3, v=1 → h wins, 2 tabs
        assert_eq!(
            l,
            Layout {
                pane_count: 3,
                tab_count: 2,
                axis: Some(SplitAxis::Horizontal)
            }
        );
    }

    #[test]
    fn layout_exact_fit_horizontal() {
        let l = plan_layout(2, None, true, 240, 48); // h=2 fits all
        assert_eq!(
            l,
            Layout {
                pane_count: 2,
                tab_count: 0,
                axis: Some(SplitAxis::Horizontal)
            }
        );
    }

    #[test]
    fn layout_fewer_sessions_than_slots() {
        let l = plan_layout(1, None, true, 320, 48); // h=3, only need 1
        assert_eq!(
            l,
            Layout {
                pane_count: 1,
                tab_count: 0,
                axis: Some(SplitAxis::Horizontal)
            }
        );
    }

    // ── axis priority: vertical wins when it fits more ──

    #[test]
    fn layout_vertical_fits_all_horizontal_doesnt() {
        // 160 cols (h=1), 72 rows (v=2), 2 sessions → vertical fits all
        let l = plan_layout(2, None, true, 160, 72);
        assert_eq!(
            l,
            Layout {
                pane_count: 2,
                tab_count: 0,
                axis: Some(SplitAxis::Vertical)
            }
        );
    }

    #[test]
    fn layout_vertical_wins_overflow() {
        // 160 cols (h=1), 72 rows (v=2), 4 sessions → v has more slots, overflow to tabs
        let l = plan_layout(4, None, true, 160, 72);
        assert_eq!(
            l,
            Layout {
                pane_count: 2,
                tab_count: 2,
                axis: Some(SplitAxis::Vertical)
            }
        );
    }

    #[test]
    fn layout_horizontal_preferred_when_both_fit() {
        // 160 cols (h=1), 48 rows (v=1), 1 session → both fit, h preferred
        let l = plan_layout(1, None, true, 160, 48);
        assert_eq!(
            l,
            Layout {
                pane_count: 1,
                tab_count: 0,
                axis: Some(SplitAxis::Horizontal)
            }
        );
    }

    #[test]
    fn layout_narrow_but_tall() {
        // 80 cols (h=0), 72 rows (v=2), 1 session → only vertical works
        let l = plan_layout(1, None, true, 80, 72);
        assert_eq!(
            l,
            Layout {
                pane_count: 1,
                tab_count: 0,
                axis: Some(SplitAxis::Vertical)
            }
        );
    }
}
