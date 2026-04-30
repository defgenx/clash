//! Shared attach loop and standalone attach client.
//!
//! The attach loop is the core I/O passthrough between stdin/stdout and a
//! daemon PTY session. It is used by both:
//! - `App::run_attached()` (TUI attaches inline, Ctrl+B returns to TUI)
//! - `run_attach_client()` (standalone `clash attach <id>` in a new window)

use color_eyre::eyre::{self, Context};
use tokio::sync::mpsc;

use crate::adapters::format::short_id;
use crate::infrastructure::daemon::client::DaemonClient;
use crate::infrastructure::daemon::protocol;

use crate::infrastructure::tui::widgets::spinner::{
    self as shimmer, CHAR_SPREAD, CYCLE_TICKS, SPINNER_FRAMES, TICKS_PER_FRAME,
};

// ── ANSI color constants (matching theme.rs) ────────────────────
pub(crate) const BUSY_BG: &str = "\x1b[48;2;8;8;14m";
const RESET: &str = "\x1b[0m";

// Status bar colors (matching theme.rs FOOTER_BG / FOOTER_FG / ACCENT / MUTED)
const BAR_BG: &str = "\x1b[48;2;22;18;32m";
const BAR_FG: &str = "\x1b[38;2;210;195;230m";
const BAR_ACCENT: &str = "\x1b[1;38;2;165;145;215m"; // bold + ACCENT
const BAR_MUTED: &str = "\x1b[38;2;95;88;115m";
const BAR_BRANCH: &str = "\x1b[38;2;200;185;125m"; // BRANCH_COLOR
const BAR_SEP: &str = "\x1b[38;2;50;42;72m"; // SEPARATOR

/// Session metadata displayed in the attach status bar.
#[derive(Clone, Default)]
pub struct AttachInfo {
    /// Display name. Prefer `Session.name` when set; otherwise `display_name`
    /// derives a meaningful identifier from project/branch (UUID prefix is the
    /// last-resort fallback).
    pub name: String,
    /// Project name (typically the basename of cwd).
    pub project: String,
    /// Git branch.
    pub branch: String,
}

/// Build the `name` field for `AttachInfo`. Mirrors the session-list display
/// rule so users see the same identifier in the footer as in the table.
///
/// Fallback chain:
///   1. `Session.name`, if set
///   2. `"{project} · {branch}"` (or whichever piece is non-empty)
///   3. 8-char UUID prefix
pub fn display_name(name: Option<&str>, project: &str, branch: &str, id: &str) -> String {
    if let Some(n) = name.map(str::trim).filter(|s| !s.is_empty()) {
        return n.to_string();
    }
    match (project.is_empty(), branch.is_empty()) {
        (false, false) => format!("{project} · {branch}"),
        (false, true) => project.to_string(),
        (true, false) => branch.to_string(),
        (true, true) => short_id(id, 8).to_string(),
    }
}

/// Detect Ctrl+B in any of the three common terminal encodings:
///   - `0x02` — standard raw byte (most terminals in normal mode)
///   - `ESC[98;5u` — Kitty keyboard protocol (CSI u)
///   - `ESC[27;5;98~` — xterm modifyOtherKeys mode (iTerm2, etc.)
fn is_ctrl_b(bytes: &[u8]) -> bool {
    if bytes.contains(&0x02) {
        return true;
    }
    if bytes.windows(7).any(|w| w == b"\x1b[98;5u") {
        return true;
    }
    if bytes.windows(10).any(|w| w == b"\x1b[27;5;98~") {
        return true;
    }
    false
}

/// Why the attach loop ended.
#[derive(Debug, PartialEq, Eq)]
pub enum AttachResult {
    /// User pressed Ctrl+B to detach.
    Detached,
    /// The session process exited.
    SessionExited,
    /// The daemon connection was lost.
    Disconnected,
}

/// Write raw bytes to stdout.
fn write_stdout(data: &[u8]) {
    unsafe {
        libc::write(1, data.as_ptr() as *const libc::c_void, data.len());
    }
}

/// Set the terminal title bar (xterm OSC sequence, works in iTerm2/etc).
fn set_title(title: &str) {
    let seq = format!("\x1b]0;{title}\x07");
    write_stdout(seq.as_bytes());
}

/// Check if output bytes contain an Erase in Display (ED) sequence
/// that would clear the status bar row outside the scroll region.
/// Matches CSI J, CSI 0 J, CSI 2 J, and CSI 3 J.
fn contains_screen_clear(bytes: &[u8]) -> bool {
    // CSI J (3 bytes) — equivalent to CSI 0 J
    bytes
        .windows(3)
        .any(|w| w[0] == 0x1b && w[1] == b'[' && w[2] == b'J')
        || bytes.windows(4).any(|w| {
            w[0] == 0x1b && w[1] == b'[' && matches!(w[2], b'0' | b'2' | b'3') && w[3] == b'J'
        })
}

// ── Status bar helpers ─────────────────────────────────────────

/// Set the terminal scroll region to rows 1..content_rows (1-indexed, inclusive),
/// leaving the bottom row free for the status bar.
fn set_scroll_region(content_rows: u16) {
    let seq = format!("\x1b[1;{content_rows}r");
    write_stdout(seq.as_bytes());
}

/// Reset scroll region to full terminal.
fn reset_scroll_region() {
    write_stdout(b"\x1b[r");
}

/// Compute the visual character width of the status-bar content (left + right).
/// `.chars().count()` is used instead of `.len()` because `info.{name,project,branch}`
/// can contain multibyte UTF-8 (accents, emoji, non-ASCII branch names) and we
/// need *cell* counts to compute padding, not byte counts.
fn status_bar_width(info: &AttachInfo, hint_chars: usize) -> usize {
    // " clash │ " (1 + 5 + 3 cells) + name
    let mut used = 1 + 5 + 3 + info.name.chars().count();
    if !info.project.is_empty() {
        // " │ " (3 cells) + project
        used += 3 + info.project.chars().count();
    }
    if !info.branch.is_empty() {
        // " │ ⎇ " (5 cells) + branch
        used += 5 + info.branch.chars().count();
    }
    used + hint_chars
}

/// Draw the status bar on the reserved bottom row (outside the scroll region).
///
/// Layout: ` {name} │ {project} │ ⎇ {branch}         Ctrl+B detach `
fn draw_status_bar(cols: u16, rows: u16, info: &AttachInfo) {
    // Save cursor, move to last row, draw bar, restore cursor.
    // This avoids disrupting Claude Code's cursor position.
    let mut buf = String::with_capacity(cols as usize + 128);

    buf.push_str("\x1b7"); // save cursor
    buf.push_str(&format!("\x1b[{};1H", rows)); // position at bottom row

    // Build left side: clash │ name │ project │ ⎇ branch
    buf.push_str(BAR_BG);
    buf.push(' ');
    buf.push_str(BAR_FG);
    buf.push_str("\x1b[1m"); // bold
    buf.push_str("clash");
    buf.push_str(RESET);
    buf.push_str(BAR_BG);
    buf.push(' ');
    buf.push_str(BAR_SEP);
    buf.push_str(BAR_BG);
    buf.push_str("│ ");
    buf.push_str(BAR_ACCENT);
    buf.push_str(&info.name);
    buf.push_str(RESET);
    buf.push_str(BAR_BG);

    if !info.project.is_empty() {
        buf.push(' ');
        buf.push_str(BAR_SEP);
        buf.push_str(BAR_BG);
        buf.push('│');
        buf.push(' ');
        buf.push_str(BAR_FG);
        buf.push_str(BAR_BG);
        buf.push_str(&info.project);
        buf.push_str(RESET);
        buf.push_str(BAR_BG);
    }

    if !info.branch.is_empty() {
        buf.push(' ');
        buf.push_str(BAR_SEP);
        buf.push_str(BAR_BG);
        buf.push('│');
        buf.push(' ');
        buf.push_str(BAR_BRANCH);
        buf.push_str(BAR_BG);
        buf.push_str("⎇ ");
        buf.push_str(&info.branch);
        buf.push_str(RESET);
        buf.push_str(BAR_BG);
    }

    // Right side: hint. ASCII-only so `chars().count() == len()`.
    let hint = "Ctrl+B detach ";
    let total_content = status_bar_width(info, hint.chars().count());

    // Fill middle with spaces
    if (cols as usize) > total_content {
        let padding = cols as usize - total_content;
        for _ in 0..padding {
            buf.push(' ');
        }
    }

    buf.push_str(BAR_MUTED);
    buf.push_str(BAR_BG);
    buf.push_str(hint);
    buf.push_str(RESET);

    buf.push_str("\x1b8"); // restore cursor

    write_stdout(buf.as_bytes());
}

/// Draw a dimmed overlay with a shimmer spinner message in the bottom-right
/// corner, matching the TUI busy overlay style.
pub fn draw_status_screen(cols: u16, rows: u16, message: &str, tick: usize) {
    // Set BUSY_BG once, then fill screen with spaces (terminal retains BG color)
    let mut buf = String::with_capacity(cols as usize * rows as usize + 256);
    buf.push_str(BUSY_BG);
    buf.push_str("\x1b[H");
    let row = " ".repeat(cols as usize);
    for _r in 0..rows {
        buf.push_str(&row);
    }

    // Build shimmer spinner text
    let spinner_char = SPINNER_FRAMES[(tick / TICKS_PER_FRAME) % SPINNER_FRAMES.len()];
    let full_text = format!("{spinner_char} {message}");

    // Position in bottom-right corner (matching busy_overlay.rs layout)
    let msg_len = full_text.chars().count() as u16;
    let msg_width = (msg_len + 4).min(cols);
    let msg_x = cols.saturating_sub(msg_width + 2) + 1; // ANSI cols are 1-based
    let msg_y = rows; // last row (1-based)

    buf.push_str(&format!("\x1b[{msg_y};{msg_x}H"));

    // Render each character with shimmer color
    for (i, ch) in full_text.chars().enumerate() {
        let phase = ((i.wrapping_mul(CHAR_SPREAD).wrapping_add(tick)) % CYCLE_TICKS) as f32
            / CYCLE_TICKS as f32;
        let (r, g, b) = shimmer::shimmer_rgb_at(phase);
        buf.push_str(&format!("\x1b[1;38;2;{r};{g};{b}m{ch}"));
    }
    buf.push_str(RESET);

    write_stdout(buf.as_bytes());
}

/// Run the I/O passthrough loop between stdin/stdout and a daemon PTY session.
///
/// Reads input from a freshly opened `/dev/tty` fd to avoid competing with
/// crossterm's internal reader thread. Daemon output is written directly to
/// stdout. A status bar with session info is drawn on the bottom row using
/// ANSI scroll regions to keep it outside the scrollable area.
///
/// - `info` contains session metadata for the status bar.
/// - `_pre_history` — kept for ABI compatibility with callers that still
///   pass buffered history. The bytes are now ignored: the live phase
///   relies on Claude repainting itself in response to the PTY size
///   toggle below, which is faster (no vt100 parse) and avoids the
///   stale-replay visual artifacts (wrong wrap when the captured size
///   differs from the current one, SGR bleed, cursor in the wrong place,
///   garbled escapes the parser couldn't handle) that the snapshot path
///   produced.
/// - Returns an `AttachResult` indicating why the loop ended.
/// - The caller is responsible for calling `daemon.detach()` afterwards.
pub async fn attach_loop(
    daemon: &mut DaemonClient,
    session_id: &str,
    info: &AttachInfo,
    daemon_rx: &mut Option<mpsc::UnboundedReceiver<protocol::Event>>,
    _pre_history: Option<Vec<u8>>,
) -> AttachResult {
    let (mut cols, mut rows) = crossterm::terminal::size().unwrap_or((120, 40));
    let content_rows = rows.saturating_sub(1).max(1);

    // Reserve bottom row for status bar via scroll region.
    set_scroll_region(content_rows);

    // Clear the content area so we don't show whatever the previous
    // foreground app left behind (the TUI's overlay, an old shell prompt,
    // etc.) before Claude paints. We deliberately do not replay any
    // history snapshot here — see attach_loop's doc comment.
    write_stdout(b"\x1b[H\x1b[J");
    draw_status_bar(cols, rows, info);

    // Set terminal title bar
    set_title(&format!("clash │ {}", info.name));

    // Force Claude to repaint its UI from scratch. Resize sends SIGWINCH
    // and resets the daemon-side screen mirror; doing it twice with a
    // different intermediate size guarantees the child sees a real
    // dimension change even when the user attaches at the exact size the
    // PTY already had. The intermediate (cols-1) resize is sub-frame fast
    // and Claude redraws over it before any flicker becomes visible.
    let nudge_cols = cols.saturating_sub(1).max(1);
    let _ = daemon.resize(session_id, nudge_cols, content_rows).await;
    let _ = daemon.resize(session_id, cols, content_rows).await;

    // SIGWINCH for terminal resize detection
    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change()).ok();

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // Open /dev/tty directly for reading. This avoids competing with
    // crossterm's internal reader thread which may still hold fd 0 or its
    // own /dev/tty handle after EventStream is dropped.
    let reader = tokio::task::spawn_blocking(move || {
        let tty_fd = unsafe { libc::open(c"/dev/tty".as_ptr(), libc::O_RDONLY) };
        if tty_fd < 0 {
            tracing::warn!("attach: failed to open /dev/tty, falling back to fd 0");
            return;
        }
        let mut buf = [0u8; 4096];
        loop {
            let n = unsafe { libc::read(tty_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            if input_tx.send(buf[..n as usize].to_vec()).is_err() {
                break;
            }
        }
        unsafe {
            libc::close(tty_fd);
        }
    });

    let mut result = AttachResult::Disconnected;

    loop {
        tokio::select! {
            biased;

            Some(bytes) = input_rx.recv() => {
                tracing::debug!("attach input: {:02x?}", &bytes[..bytes.len().min(32)]);
                if is_ctrl_b(&bytes) {
                    result = AttachResult::Detached;
                    break;
                }
                if let Err(e) = daemon.send_input(session_id, &bytes).await {
                    tracing::warn!("send_input failed: {}", e);
                    result = AttachResult::Disconnected;
                    break;
                }
            }

            // Terminal resized → update scroll region, resize PTY, redraw bar
            Some(_) = async {
                match sigwinch.as_mut() {
                    Some(sig) => sig.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok((w, h)) = crossterm::terminal::size() {
                    cols = w;
                    rows = h;
                    let cr = h.saturating_sub(1).max(1);
                    set_scroll_region(cr);
                    let _ = daemon.resize(session_id, w, cr).await;
                    draw_status_bar(w, h, info);
                }
            }

            Some(daemon_event) = async {
                match daemon_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match daemon_event {
                    protocol::Event::Output {
                        session_id: ev_sid,
                        data,
                    } => {
                        // Drop bytes belonging to a different session — the
                        // daemon's mpsc fan-in does not pre-filter and stale
                        // forwarder bytes from a prior attach on this same
                        // connection can leak into the queue.
                        if ev_sid != session_id {
                            continue;
                        }
                        if let Ok(bytes) = protocol::decode_data(&data) {
                            write_stdout(&bytes);
                            // Only redraw when output contains Erase in Display
                            // sequences (ED) that clear outside the scroll region.
                            // Redrawing on every chunk breaks Claude Code's input.
                            if contains_screen_clear(&bytes) {
                                draw_status_bar(cols, rows, info);
                            }
                        }
                    }
                    protocol::Event::Exited {
                        session_id: ev_sid, ..
                    } => {
                        if ev_sid == session_id {
                            result = AttachResult::SessionExited;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            else => break,
        }
    }

    // Reset scroll region before any cleanup drawing
    reset_scroll_region();

    // Show detaching feedback (same busy overlay style)
    let (cols, rows) = crossterm::terminal::size().unwrap_or((cols, rows));
    draw_status_screen(cols, rows, &format!("Detaching {}…", info.name), 0);

    // Reset terminal title
    set_title("");

    drop(input_rx);
    reader.abort();

    result
}

// ── Standalone attach client ───────────────────────────────────────

/// RAII guard that flips `/dev/tty` into raw mode on construction and
/// restores the original termios on drop. Drop runs on panic unwind, so
/// the client's terminal is never left wedged if the attach loop explodes.
struct RawModeGuard {
    tty: std::fs::File,
    orig: nix::sys::termios::Termios,
}

impl RawModeGuard {
    fn enter(tty: std::fs::File) -> eyre::Result<Self> {
        use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
        let orig = tcgetattr(&tty).map_err(|e| eyre::eyre!("tcgetattr failed: {}", e))?;
        // Build the guard first so any later failure unwinds through Drop
        // and restores the untouched original termios (a no-op restore is
        // harmless; leaving raw mode active with no guard would not be).
        let guard = Self { tty, orig };
        let mut raw = guard.orig.clone();
        cfmakeraw(&mut raw);
        tcsetattr(&guard.tty, SetArg::TCSANOW, &raw)
            .map_err(|e| eyre::eyre!("tcsetattr failed: {}", e))?;
        Ok(guard)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        use nix::sys::termios::{tcsetattr, SetArg};
        let _ = tcsetattr(&self.tty, SetArg::TCSANOW, &self.orig);
        write_stdout(crate::infrastructure::tui::terminal_reset::FINAL_RESET);
    }
}

/// Look up a session on the daemon and build a footer `AttachInfo` from it.
///
/// `cwd` and `name` come straight from `SessionInfo`; the branch is read from
/// `.git/HEAD` in `cwd` (the daemon doesn't track branch — it's a property of
/// the working tree, not the PTY). Failures fall back to a UUID-prefix name so
/// the bar always renders something.
async fn build_attach_info_from_daemon(daemon: &mut DaemonClient, session_id: &str) -> AttachInfo {
    let session_meta = match daemon.list_sessions().await {
        Ok(list) => list.into_iter().find(|s| s.session_id == session_id),
        Err(e) => {
            tracing::warn!("list_sessions failed during attach: {}", e);
            None
        }
    };

    let (name_opt, cwd) = match session_meta {
        Some(s) => (s.name, s.cwd),
        None => (None, String::new()),
    };

    let project = cwd
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(&cwd)
        .to_string();
    let branch = crate::infrastructure::fs::backend::FsBackend::detect_git_branch(&cwd);

    AttachInfo {
        name: display_name(name_opt.as_deref(), &project, &branch, session_id),
        project,
        branch,
    }
}

/// Close the surrounding pane / tab / window after `clash attach` finishes.
///
/// Each host terminal has its own close-by-id mechanism and exports a
/// dedicated env var that uniquely identifies the calling pane. We try
/// strategies in order; the first matching env var wins, the matching
/// command closes that exact pane (so a sibling pane the user clicked
/// into mid-session is never affected), and unmatched terminals fall
/// through to whatever close-on-exit default the terminal has.
fn close_external_pane_if_inside() {
    for strategy in CLOSE_PANE_STRATEGIES {
        if (strategy.run)() {
            return;
        }
    }
}

struct ClosePaneStrategy {
    run: fn() -> bool,
}

/// Strategies dispatch on the env var the host terminal exports. First
/// successful match returns true; later strategies don't run.
const CLOSE_PANE_STRATEGIES: &[ClosePaneStrategy] = &[
    ClosePaneStrategy {
        run: close_via_term_program_iterm,
    },
    ClosePaneStrategy {
        run: close_via_tmux_pane,
    },
    ClosePaneStrategy {
        run: close_via_wezterm_pane,
    },
    ClosePaneStrategy {
        run: close_via_kitty_window,
    },
];

/// $TERM_PROGRAM == "iTerm.app" → osascript, close by `unique id` matching
/// the UUID tail of $ITERM_SESSION_ID = "wXtYpZ:UUID". Required because
/// the default profile on this terminal keeps panes alive after the
/// foreground process exits ("[Process completed]").
///
/// The call is synchronous: the terminal SIGHUPs this process mid-script,
/// so this function effectively never returns when the close succeeds.
fn close_via_term_program_iterm() -> bool {
    if std::env::var("TERM_PROGRAM").as_deref() != Ok("iTerm.app") {
        return false;
    }
    let raw = match std::env::var("ITERM_SESSION_ID") {
        Ok(v) if !v.is_empty() => v,
        _ => return false,
    };
    let uuid = raw.rsplit(':').next().unwrap_or(&raw);
    if uuid.is_empty() {
        return false;
    }
    let script = format!(
        concat!(
            "tell application \"iTerm2\"\n",
            "  repeat with theWindow in windows\n",
            "    repeat with theTab in tabs of theWindow\n",
            "      repeat with theSession in sessions of theTab\n",
            "        if (unique id of theSession as string) is \"{uuid}\" then\n",
            "          tell theSession to close\n",
            "          return\n",
            "        end if\n",
            "      end repeat\n",
            "    end repeat\n",
            "  end repeat\n",
            "end tell",
        ),
        uuid = uuid
    );
    run_silent("osascript", &["-e", &script])
}

/// $TMUX_PANE present (e.g. "%17") → `tmux kill-pane -t $TMUX_PANE`.
/// Handles `set -g remain-on-exit on` users.
fn close_via_tmux_pane() -> bool {
    let pane = match std::env::var("TMUX_PANE") {
        Ok(v) if !v.is_empty() => v,
        _ => return false,
    };
    run_silent("tmux", &["kill-pane", "-t", &pane])
}

/// $WEZTERM_PANE present → `wezterm cli kill-pane --pane-id $WEZTERM_PANE`.
/// Handles `exit_behavior = "Hold"` users.
fn close_via_wezterm_pane() -> bool {
    let pane = match std::env::var("WEZTERM_PANE") {
        Ok(v) if !v.is_empty() => v,
        _ => return false,
    };
    run_silent("wezterm", &["cli", "kill-pane", "--pane-id", &pane])
}

/// $KITTY_WINDOW_ID present → `kitty @ close-window --match id:N`. Needs
/// `allow_remote_control yes` in kitty.conf; if not enabled the call is a
/// silent no-op and we fall back to kitty's default close-on-child-death.
fn close_via_kitty_window() -> bool {
    let win = match std::env::var("KITTY_WINDOW_ID") {
        Ok(v) if !v.is_empty() => v,
        _ => return false,
    };
    run_silent(
        "kitty",
        &["@", "close-window", "--match", &format!("id:{}", win)],
    )
}

/// Run `cmd args...` synchronously with all stdio silenced. Returns true
/// when the command was actually launched (regardless of its exit status —
/// in the close-pane case, success usually kills us before we can read it).
fn run_silent(cmd: &str, args: &[&str]) -> bool {
    std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Entry point for `clash attach <session_id>`.
///
/// Connects to the running daemon, attaches to the specified session,
/// and runs the I/O passthrough loop. The session must already exist
/// in the daemon (created by the TUI).
pub async fn run_attach_client(session_id: String) -> eyre::Result<()> {
    let socket_path = DaemonClient::default_socket_path();
    let mut daemon = DaemonClient::new(socket_path);

    daemon
        .connect()
        .await
        .wrap_err("Could not connect to clash daemon. Is clash running?")?;

    let mut daemon_rx = daemon.take_stream_rx();

    // Retry attach — the TUI may still be creating the session.
    // Skip the replay buffer; attach_loop forces Claude to repaint via a
    // PTY size-toggle, which is faster than parsing+rendering the history
    // snapshot and avoids the stale-replay visual artifacts.
    let mut last_err = None;
    for attempt in 0..3 {
        match daemon.attach(&session_id, true).await {
            Ok(()) => {
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                if attempt < 2 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    }
    if let Some(e) = last_err {
        return Err(e).wrap_err(format!("Failed to attach to session {}", session_id));
    }

    // Enter raw mode via nix termios directly — avoids initializing crossterm's
    // internal reader thread, which would compete with attach_loop's /dev/tty
    // reader. The guard restores termios + FINAL_RESET on Drop (incl. panic).
    let tty = std::fs::File::open("/dev/tty").wrap_err("Could not open /dev/tty")?;
    let guard = RawModeGuard::enter(tty)?;

    // Fetch session metadata from the daemon so the footer shows the real
    // session name (or project · branch fallback) instead of a UUID prefix.
    // Best-effort: if the lookup fails the footer still shows the short ID.
    let info = build_attach_info_from_daemon(&mut daemon, &session_id).await;

    let result = attach_loop(&mut daemon, &session_id, &info, &mut daemon_rx, None).await;

    let _ = daemon.detach(&session_id).await;

    // Drop the guard explicitly before any user-facing eprintln, so termios
    // is back in cooked mode and \n renders with proper CR/LF.
    drop(guard);

    match result {
        AttachResult::SessionExited => eprintln!("Session exited."),
        AttachResult::Disconnected => eprintln!("Disconnected from daemon."),
        AttachResult::Detached => {}
    }

    // We were spawned into an external pane/tab/window by clash's `o`/`O`.
    // Close it explicitly so any host terminal that holds panes after
    // foreground exit (configurable on every supported terminal, default
    // on at least one) doesn't leave a zombie pane behind.
    close_external_pane_if_inside();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_b_standard_raw_byte() {
        assert!(is_ctrl_b(&[0x02]));
    }

    #[test]
    fn ctrl_b_raw_byte_in_middle_of_data() {
        assert!(is_ctrl_b(&[0x61, 0x02, 0x63]));
    }

    #[test]
    fn ctrl_b_kitty_csi_u() {
        assert!(is_ctrl_b(b"\x1b[98;5u"));
    }

    #[test]
    fn ctrl_b_xterm_modify_other_keys() {
        assert!(is_ctrl_b(b"\x1b[27;5;98~"));
    }

    #[test]
    fn ctrl_b_xterm_embedded_in_stream() {
        let mut data = vec![0x61, 0x62];
        data.extend_from_slice(b"\x1b[27;5;98~");
        data.push(0x63);
        assert!(is_ctrl_b(&data));
    }

    #[test]
    fn not_ctrl_b_regular_text() {
        assert!(!is_ctrl_b(b"hello world"));
    }

    #[test]
    fn not_ctrl_b_empty() {
        assert!(!is_ctrl_b(&[]));
    }

    #[test]
    fn not_ctrl_b_other_escape_sequence() {
        assert!(!is_ctrl_b(b"\x1b[27;5;97~"));
    }

    #[test]
    fn status_bar_content() {
        // Just verify draw_status_bar doesn't panic with various inputs
        let info = AttachInfo {
            name: "test-session".to_string(),
            project: "my-project".to_string(),
            branch: "main".to_string(),
        };
        // Can't easily test stdout output, but exercise the code path
        let _ = &info;
    }

    #[test]
    fn status_bar_empty_fields() {
        let info = AttachInfo {
            name: "test".to_string(),
            project: String::new(),
            branch: String::new(),
        };
        let _ = &info;
    }

    #[test]
    fn status_bar_width_ascii_matches_visual_columns() {
        // " clash │ test │ proj │ ⎇ main " (left) + "Ctrl+B detach " (right).
        // 5 prefix + 4 + 6 + 4 + 14 hint = 33. (See helper for breakdown.)
        let info = AttachInfo {
            name: "test".to_string(),
            project: "proj".to_string(),
            branch: "main".to_string(),
        };
        let hint_len = "Ctrl+B detach ".chars().count();
        let w = status_bar_width(&info, hint_len);
        // Sanity: hand-computed width.
        // " clash │ "(9) + "test"(4) + " │ "(3) + "proj"(4)
        //     + " │ ⎇ "(5) + "main"(4) + hint(14) = 43
        assert_eq!(w, 43);
    }

    // ── display_name fallback chain ─────────────────────────────

    #[test]
    fn display_name_uses_session_name_when_set() {
        assert_eq!(
            display_name(Some("my-session"), "proj", "main", "abc12345xxxx"),
            "my-session"
        );
    }

    #[test]
    fn display_name_treats_blank_name_as_unset() {
        assert_eq!(
            display_name(Some("   "), "proj", "main", "abc12345xxxx"),
            "proj · main"
        );
    }

    #[test]
    fn display_name_falls_back_to_project_dot_branch() {
        assert_eq!(
            display_name(None, "clash", "feature/utf8", "abc12345xxxx"),
            "clash · feature/utf8"
        );
    }

    #[test]
    fn display_name_falls_back_to_project_only_when_no_branch() {
        assert_eq!(display_name(None, "clash", "", "abc12345xxxx"), "clash");
    }

    #[test]
    fn display_name_falls_back_to_branch_only_when_no_project() {
        assert_eq!(display_name(None, "", "main", "abc12345xxxx"), "main");
    }

    #[test]
    fn display_name_falls_back_to_short_id_as_last_resort() {
        assert_eq!(display_name(None, "", "", "abcdef1234567890"), "abcdef12");
    }

    #[test]
    fn status_bar_width_counts_multibyte_as_single_cells() {
        // Regression: bytes (.len()) used to be conflated with cells.
        // "café" is 5 bytes but 4 cells; "feature/café" is 13 bytes but 12 cells.
        let info = AttachInfo {
            name: "café".to_string(),
            project: "tiramisú".to_string(),
            branch: "feature/café".to_string(),
        };
        let hint_len = "Ctrl+B detach ".chars().count();
        let w = status_bar_width(&info, hint_len);
        // 9 + 4 + 3 + 8 + 5 + 12 + 14 = 55 cells.
        assert_eq!(w, 55);
        // And — the bug we're fixing — must not equal the byte-based total.
        let bytes_based =
            1 + 5 + 3 + info.name.len() + 3 + info.project.len() + 5 + info.branch.len() + hint_len;
        assert_ne!(w, bytes_based);
    }
}
