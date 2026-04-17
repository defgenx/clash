//! Shared escape-byte constants for restoring terminal state.
//!
//! Used on TUI exit, detach-to-TUI cleanup, and standalone attach exit so
//! that all three call sites agree on exactly which modes to disable. Adding
//! a new mode here propagates to every cleanup path.

/// Disable terminal modes that clash or an attached Claude session may have
/// enabled. Safe to write while the alternate screen is still active —
/// these are mode toggles, not visual resets.
pub(crate) const MODES_RESET: &[u8] = concat!(
    "\x1b[?1000l", // mouse button tracking
    "\x1b[?1002l", // cell-motion mouse
    "\x1b[?1003l", // all-motion mouse
    "\x1b[?1006l", // SGR mouse
    "\x1b[?1015l", // urxvt mouse
    "\x1b[?2004l", // bracketed paste
    "\x1b[?1004l", // focus reporting
    "\x1b[<u",     // pop Kitty keyboard protocol (if active)
)
.as_bytes();

/// Final visual reset — meant for the main screen, after leaving the alt
/// screen (TUI exit) or on standalone attach exit.
pub(crate) const FINAL_RESET: &[u8] = concat!(
    "\x1b[?6l",      // origin mode off
    "\x1b[r",        // scroll region reset
    "\x1b[2J\x1b[H", // clear + cursor home
    "\x1b[0 q",      // DECSCUSR — default cursor shape (Claude sets a bar)
    "\x1b[?25h",     // show cursor
    "\x1b[0m",       // reset SGR attributes
)
.as_bytes();

#[cfg(test)]
mod tests {
    use super::*;

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn modes_reset_has_all_required_subsequences() {
        for seq in [
            b"\x1b[?1000l".as_slice(),
            b"\x1b[?1002l".as_slice(),
            b"\x1b[?1003l".as_slice(),
            b"\x1b[?1006l".as_slice(),
            b"\x1b[?1015l".as_slice(),
            b"\x1b[?2004l".as_slice(),
            b"\x1b[?1004l".as_slice(),
            b"\x1b[<u".as_slice(),
        ] {
            assert!(
                contains(MODES_RESET, seq),
                "MODES_RESET missing subsequence {:?}",
                std::str::from_utf8(seq).unwrap_or("<bin>")
            );
        }
    }

    #[test]
    fn final_reset_has_all_required_subsequences() {
        for seq in [
            b"\x1b[?6l".as_slice(),
            b"\x1b[r".as_slice(),
            b"\x1b[2J".as_slice(),
            b"\x1b[H".as_slice(),
            b"\x1b[0 q".as_slice(),
            b"\x1b[?25h".as_slice(),
            b"\x1b[0m".as_slice(),
        ] {
            assert!(
                contains(FINAL_RESET, seq),
                "FINAL_RESET missing subsequence {:?}",
                std::str::from_utf8(seq).unwrap_or("<bin>")
            );
        }
    }
}
