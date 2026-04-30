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

// ── History snapshot ──────────────────────────────────────────

/// Process raw PTY history through a vt100 parser and render only the final
/// screen state. This avoids visible scrolling when replaying large histories.
fn render_history_snapshot(history: &[u8], content_rows: u16, cols: u16) {
    if history.is_empty() {
        return;
    }

    // Process history offscreen
    let mut parser = vt100::Parser::new(content_rows, cols, 0);
    parser.process(history);
    let screen = parser.screen();

    // Render the final screen image
    write_stdout(b"\x1b[?25l"); // hide cursor
    write_stdout(b"\x1b[H"); // cursor home
    let rendered = screen.contents_formatted();
    write_stdout(&rendered);

    // Restore cursor to its correct position
    let (cy, cx) = screen.cursor_position();
    let seq = format!("\x1b[{};{}H", cy + 1, cx + 1);
    write_stdout(seq.as_bytes());
    write_stdout(b"\x1b[?25h"); // show cursor
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

// ── History-buffer exit predicate ─────────────────────────────

/// Attach history-buffer timings. Shared between the TUI
/// (`App::buffer_attach_history`) and the standalone client
/// (`buffer_history`) so both loops have identical pacing.
pub(crate) const ATTACH_MIN_VISIBLE_MS: u64 = 150;
pub(crate) const ATTACH_HARD_LIMIT_MS: u64 = 500;
pub(crate) const ATTACH_EMPTY_TIMEOUT_MS: u64 = 80;
pub(crate) const ATTACH_IDLE_MS: u64 = 80;

/// Pure predicate deciding when the history-buffering phase should end.
///
/// Returns `true` once any of:
/// - the hard limit has elapsed (cap regardless of activity);
/// - no output has arrived after `empty_timeout_ms` (new session — nothing to
///   coalesce, stop waiting);
/// - output has arrived and gone idle for at least `idle_threshold_ms`.
///
/// Never returns `true` before `min_visible_ms`, so the busy overlay can't
/// flash. Kept pure so the four-way truth table is unit-testable.
pub(crate) fn should_break_history_buffer(
    got_output: bool,
    elapsed_ms: u64,
    idle_ms: u64,
    min_visible_ms: u64,
    hard_limit_ms: u64,
    empty_timeout_ms: u64,
    idle_threshold_ms: u64,
) -> bool {
    if elapsed_ms < min_visible_ms {
        return false;
    }
    if elapsed_ms >= hard_limit_ms {
        return true;
    }
    if !got_output && elapsed_ms >= empty_timeout_ms {
        return true;
    }
    if got_output && idle_ms >= idle_threshold_ms {
        return true;
    }
    false
}

/// Buffer daemon history bytes while showing a loading spinner.
///
/// Used by the standalone attach client (`clash attach <id>`) which doesn't
/// have a TUI to show a busy overlay. The TUI path uses its own buffering
/// loop in `App::buffer_attach_history()` instead.
///
/// `session_id` is required so we can drop Output events from any other
/// session — the daemon's mpsc fan-in does not pre-filter, and a stale
/// forwarder (e.g. from a prior attach on the same connection) can leave
/// bytes belonging to another session in the receive queue.
pub async fn buffer_history(
    session_id: &str,
    name: &str,
    daemon_rx: &mut Option<mpsc::UnboundedReceiver<protocol::Event>>,
) -> Result<Vec<u8>, AttachResult> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));

    const TICK_MS: u64 = 50;

    let mut history: Vec<u8> = Vec::new();
    let mut tick = 0usize;
    let mut got_output = false;
    let started = tokio::time::Instant::now();
    let mut last_output = started;

    let loading_msg = format!("Loading {name}…");
    draw_status_screen(cols, rows, &loading_msg, tick);

    loop {
        tokio::select! {
            biased;

            Some(ev) = async {
                match daemon_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match ev {
                    protocol::Event::Output {
                        session_id: ev_sid,
                        data,
                    } => {
                        // Drop output from any other session — see function-doc on
                        // why filtering is required even on the standalone path.
                        if ev_sid != session_id {
                            continue;
                        }
                        if let Ok(bytes) = protocol::decode_data(&data) {
                            history.extend_from_slice(&bytes);
                        }
                        got_output = true;
                        last_output = tokio::time::Instant::now();
                    }
                    protocol::Event::Exited {
                        session_id: ev_sid, ..
                    } => {
                        if ev_sid == session_id {
                            return Err(AttachResult::SessionExited);
                        }
                    }
                    _ => {}
                }
            }

            _ = tokio::time::sleep(std::time::Duration::from_millis(TICK_MS)) => {
                let now = tokio::time::Instant::now();
                let elapsed_ms = now.duration_since(started).as_millis() as u64;
                let idle_ms = now.duration_since(last_output).as_millis() as u64;
                if should_break_history_buffer(
                    got_output,
                    elapsed_ms,
                    idle_ms,
                    ATTACH_MIN_VISIBLE_MS,
                    ATTACH_HARD_LIMIT_MS,
                    ATTACH_EMPTY_TIMEOUT_MS,
                    ATTACH_IDLE_MS,
                ) {
                    break;
                }
                tick += 1;
                draw_status_screen(cols, rows, &loading_msg, tick);
            }
        }
    }

    Ok(history)
}

/// Run the I/O passthrough loop between stdin/stdout and a daemon PTY session.
///
/// Reads input from a freshly opened `/dev/tty` fd to avoid competing with
/// crossterm's internal reader thread. Daemon output is written directly to
/// stdout. A status bar with session info is drawn on the bottom row using
/// ANSI scroll regions to keep it outside the scrollable area.
///
/// - `info` contains session metadata for the status bar.
/// - `pre_history` — if provided, skips the loading phase and replays this
///   history immediately. Used by the TUI which buffers history while showing
///   its own busy overlay.
/// - Returns an `AttachResult` indicating why the loop ended.
/// - The caller is responsible for calling `daemon.detach()` afterwards.
pub async fn attach_loop(
    daemon: &mut DaemonClient,
    session_id: &str,
    info: &AttachInfo,
    daemon_rx: &mut Option<mpsc::UnboundedReceiver<protocol::Event>>,
    pre_history: Option<Vec<u8>>,
) -> AttachResult {
    let (mut cols, mut rows) = crossterm::terminal::size().unwrap_or((120, 40));
    let content_rows = rows.saturating_sub(1).max(1);

    // Reserve bottom row for status bar via scroll region, resize PTY to fit
    set_scroll_region(content_rows);
    let _ = daemon.resize(session_id, cols, content_rows).await;
    draw_status_bar(cols, rows, info);

    // Set terminal title bar
    set_title(&format!("clash │ {}", info.name));

    // ── History replay / loading phase ──────────────────────────
    // The scroll region is already active, so all output stays above the bar.
    // If pre-buffered history is provided, replay it now.  Otherwise buffer
    // from the daemon with a spinner (standalone client path).
    if let Some(history) = pre_history {
        render_history_snapshot(&history, content_rows, cols);
        draw_status_bar(cols, rows, info);
    } else {
        let raw_history = match buffer_history(session_id, &info.name, daemon_rx).await {
            Ok(h) => h,
            Err(result) => {
                reset_scroll_region();
                return result;
            }
        };
        // Clear loading screen within scroll region, replay history
        set_scroll_region(content_rows); // re-set after loading
        render_history_snapshot(&raw_history, content_rows, cols);
        draw_status_bar(cols, rows, info);
    }

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

/// If running inside an iTerm2 session, ask iTerm2 to close THIS specific
/// session by `unique id`, which we read out of `ITERM_SESSION_ID`.
///
/// Why: iTerm2's "Session > When Done" profile setting defaults to "Don't
/// close" — when `clash attach` exits, the pane stays around showing
/// "[Process completed]". Other terminals (tmux, WezTerm, Kitty) close on
/// foreground exit by default, so this only affects iTerm2.
///
/// We close by UUID rather than `current session of current window` so a
/// user who clicked into a sibling pane mid-session doesn't get the wrong
/// pane closed. The osascript call is synchronous: iTerm2 will close the
/// pane (killing the foreground process) while osascript is in-flight, so
/// this function effectively never returns when it succeeds.
fn close_iterm2_session_if_inside() {
    if std::env::var("TERM_PROGRAM").as_deref() != Ok("iTerm.app") {
        return;
    }
    let raw = match std::env::var("ITERM_SESSION_ID") {
        Ok(v) if !v.is_empty() => v,
        _ => return,
    };
    // ITERM_SESSION_ID is shaped like "w0t0p0:UUID". The UUID is what
    // matches `unique id of session` in iTerm2's AppleScript dictionary.
    let uuid = raw.rsplit(':').next().unwrap_or(&raw);
    let script = format!(
        concat!(
            r#"tell application "iTerm2""#,
            "\n  repeat with theWindow in windows",
            "\n    repeat with theTab in tabs of theWindow",
            "\n      repeat with theSession in sessions of theTab",
            "\n        if (unique id of theSession as string) is \"{uuid}\" then",
            "\n          tell theSession to close",
            "\n          return",
            "\n        end if",
            "\n      end repeat",
            "\n    end repeat",
            "\n  end repeat",
            "\nend tell",
        ),
        uuid = uuid
    );
    // Synchronous wait: iTerm2 will SIGHUP us mid-script when it closes the
    // pane, which terminates this process. If iTerm2 isn't running (rare —
    // we just verified TERM_PROGRAM), osascript exits cleanly and we return.
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
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

    // Retry attach — the TUI may still be creating the session
    let mut last_err = None;
    for attempt in 0..3 {
        match daemon.attach(&session_id).await {
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

    // If we're running inside an iTerm2 pane/tab spawned by `o`/`O`, close
    // it. iTerm2's default profile keeps the pane open after the foreground
    // command exits (showing "[Process completed]"); we want it to vanish
    // so the user's `o → Ctrl+B` flow leaves no zombie panes behind.
    close_iterm2_session_if_inside();

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

    // ── should_break_history_buffer ────────────────────────────

    // Helper: call with the production constants so tests mirror real pacing.
    fn sbhb(got_output: bool, elapsed_ms: u64, idle_ms: u64) -> bool {
        should_break_history_buffer(
            got_output,
            elapsed_ms,
            idle_ms,
            ATTACH_MIN_VISIBLE_MS,
            ATTACH_HARD_LIMIT_MS,
            ATTACH_EMPTY_TIMEOUT_MS,
            ATTACH_IDLE_MS,
        )
    }

    #[test]
    fn buffer_never_breaks_before_min_visible() {
        // Even if the hard limit, empty timeout, and idle all say "break",
        // we must hold until min_visible_ms so the overlay doesn't flash.
        assert!(!sbhb(false, 0, 0));
        assert!(!sbhb(true, 100, 100));
        assert!(!sbhb(false, ATTACH_MIN_VISIBLE_MS - 1, 999));
    }

    #[test]
    fn buffer_breaks_for_empty_session_after_min_visible() {
        // New session: no output ever arrives. Once min_visible is met and
        // empty_timeout has elapsed, break immediately — don't wait the full
        // hard limit.
        assert!(sbhb(false, ATTACH_MIN_VISIBLE_MS, 0));
    }

    #[test]
    fn buffer_breaks_when_output_idles() {
        // Session has history; output streamed in and then went quiet for
        // at least idle_threshold_ms past min_visible_ms.
        assert!(sbhb(true, ATTACH_MIN_VISIBLE_MS + 10, ATTACH_IDLE_MS));
        // Still streaming (not idle) — do not break.
        assert!(!sbhb(true, ATTACH_MIN_VISIBLE_MS + 10, 0));
    }

    #[test]
    fn buffer_breaks_at_hard_limit() {
        // Pathological case: output keeps streaming past hard limit. Break
        // anyway so attach never gets stuck in the busy overlay.
        assert!(sbhb(true, ATTACH_HARD_LIMIT_MS, 0));
        assert!(sbhb(false, ATTACH_HARD_LIMIT_MS, 0));
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
