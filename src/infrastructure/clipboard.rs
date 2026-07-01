//! System-clipboard copy for the TUI.
//!
//! Two complementary paths, both best-effort:
//!
//! 1. The platform clipboard command (`pbcopy` on macOS, `wl-copy`/`xclip`/
//!    `xsel` on Linux, `clip` on Windows) — reliable for *local* copies
//!    regardless of terminal support.
//! 2. An OSC 52 terminal escape — covers SSH sessions and terminals that own
//!    the clipboard (iTerm2, kitty, WezTerm, Ghostty, tmux with
//!    `set-clipboard on`), where no local command is reachable.
//!
//! We attempt both: the command for local reliability, OSC 52 for everything
//! else. `base64` is already a dependency, so OSC 52 needs no extra crates.

use base64::Engine;
use std::io::Write;
use std::process::{Command, Stdio};

/// Copy `text` to the system clipboard (best-effort, never panics).
///
/// Returns `true` when a platform clipboard command confirmed the copy;
/// `false` when we only emitted the OSC 52 escape (e.g. over SSH, or when no
/// clipboard command is installed) — in which case success depends on the
/// terminal honoring OSC 52.
pub fn copy(text: &str) -> bool {
    let confirmed = copy_via_command(text);
    // Always also emit OSC 52: harmless if the command already worked or the
    // terminal ignores it, and the only option over SSH / in bare terminals.
    emit_osc52(text);
    confirmed
}

/// Try the platform clipboard utility, feeding `text` on stdin. Returns true
/// only if a command was found and exited successfully.
fn copy_via_command(text: &str) -> bool {
    for (cmd, args) in clipboard_commands() {
        let Ok(mut child) = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue; // command not installed — try the next candidate
        };
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(text.as_bytes()).is_err() {
                continue;
            }
            drop(stdin); // close stdin so the tool flushes and exits
        }
        if matches!(child.wait(), Ok(status) if status.success()) {
            return true;
        }
    }
    false
}

/// Ordered clipboard-command candidates for the current platform.
fn clipboard_commands() -> Vec<(&'static str, &'static [&'static str])> {
    if cfg!(target_os = "macos") {
        vec![("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        vec![("clip", &[])]
    } else {
        // Wayland first, then X11 fallbacks.
        vec![
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"][..]),
            ("xsel", &["--clipboard", "--input"][..]),
        ]
    }
}

/// Write the OSC 52 clipboard escape to the terminal (stdout). Best-effort.
fn emit_osc52(text: &str) {
    let seq = osc52_sequence(text, std::env::var_os("TMUX").is_some());
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(seq.as_bytes());
    let _ = stdout.flush();
}

/// Build the OSC 52 escape sequence that sets the system clipboard to `text`.
/// When `tmux` is true, wrap it in tmux's DCS passthrough (doubling inner
/// `ESC`s) so it reaches the outer terminal.
fn osc52_sequence(text: &str, tmux: bool) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    let inner = format!("\x1b]52;c;{}\x07", b64);
    if tmux {
        format!("\x1bPtmux;{}\x1b\\", inner.replace('\x1b', "\x1b\x1b"))
    } else {
        inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_encodes_payload_as_base64() {
        // "hi" → "aGk=" in standard base64.
        let seq = osc52_sequence("hi", false);
        assert_eq!(seq, "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn osc52_tmux_wraps_and_doubles_escapes() {
        let seq = osc52_sequence("hi", true);
        // DCS passthrough opens with ESC P tmux; and closes with ESC \ .
        assert!(seq.starts_with("\x1bPtmux;"));
        assert!(seq.ends_with("\x1b\\"));
        // The inner OSC's ESC is doubled inside the passthrough.
        assert!(seq.contains("\x1b\x1b]52;c;aGk=\x07"));
    }

    #[test]
    fn osc52_empty_text_is_valid() {
        // Empty payload still produces a well-formed (empty-clipboard) escape.
        assert_eq!(osc52_sequence("", false), "\x1b]52;c;\x07");
    }
}
