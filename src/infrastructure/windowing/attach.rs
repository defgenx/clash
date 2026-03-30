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
const BUSY_BG: &str = "\x1b[48;2;8;8;14m";
const RESET: &str = "\x1b[0m";

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

/// Buffer daemon history bytes while showing a loading spinner.
///
/// Used by the standalone attach client (`clash attach <id>`) which doesn't
/// have a TUI to show a busy overlay. The TUI path uses its own buffering
/// loop in `App::buffer_attach_history()` instead.
pub async fn buffer_history(
    name: &str,
    daemon_rx: &mut Option<mpsc::UnboundedReceiver<protocol::Event>>,
) -> Result<Vec<u8>, AttachResult> {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));

    const IDLE_MS: u64 = 80;
    const DEADLINE_MS: u64 = 1500;
    const TICK_MS: u64 = 50;

    let mut history: Vec<u8> = Vec::new();
    let mut tick = 0usize;
    let mut got_output = false;
    let mut last_output = tokio::time::Instant::now();
    let deadline = last_output + std::time::Duration::from_millis(DEADLINE_MS);

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
                    protocol::Event::Output { data, .. } => {
                        if let Ok(bytes) = protocol::decode_data(&data) {
                            history.extend_from_slice(&bytes);
                        }
                        got_output = true;
                        last_output = tokio::time::Instant::now();
                    }
                    protocol::Event::Exited { .. } => return Err(AttachResult::SessionExited),
                    _ => {}
                }
            }

            _ = tokio::time::sleep(std::time::Duration::from_millis(TICK_MS)) => {
                let now = tokio::time::Instant::now();
                let idle = now.duration_since(last_output).as_millis() as u64 >= IDLE_MS;
                if (got_output && idle) || now >= deadline {
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
/// stdout with no chrome overlay (Claude Code manages its own full-screen UI).
///
/// Session info is shown in the terminal title bar instead.
///
/// - `name` is the session display name (shown in title bar and loading screen).
/// - `pre_history` — if provided, skips the loading phase and replays this
///   history immediately. Used by the TUI which buffers history while showing
///   its own busy overlay.
/// - Returns an `AttachResult` indicating why the loop ended.
/// - The caller is responsible for calling `daemon.detach()` afterwards.
pub async fn attach_loop(
    daemon: &mut DaemonClient,
    session_id: &str,
    name: &str,
    daemon_rx: &mut Option<mpsc::UnboundedReceiver<protocol::Event>>,
    pre_history: Option<Vec<u8>>,
) -> AttachResult {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));

    // PTY gets full terminal size — no chrome to reserve rows for
    let _ = daemon.resize(session_id, cols, rows).await;

    // Set terminal title bar
    set_title(&format!("clash │ {name}"));

    // ── Loading phase ───────────────────────────────────────────
    // If pre-buffered history is provided, the caller already replayed it
    // to stdout — drop the buffer and skip loading. Otherwise buffer here
    // with a spinner (standalone client path), then replay + drop.
    let needs_loading = pre_history.is_none();
    drop(pre_history);
    if needs_loading {
        let raw_history = match buffer_history(name, daemon_rx).await {
            Ok(h) => h,
            Err(result) => return result,
        };
        write_stdout(b"\x1b[2J\x1b[H"); // clear loading screen
        write_stdout(b"\x1b[?25l"); // hide cursor during replay
        write_stdout(&raw_history); // full history → terminal scrollback
        write_stdout(b"\x1b[?25h"); // show cursor
                                    // raw_history dropped here — buffer released right after replay
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

            // Terminal resized → just resize PTY (no chrome to redraw)
            Some(_) = async {
                match sigwinch.as_mut() {
                    Some(sig) => sig.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok((w, h)) = crossterm::terminal::size() {
                    let _ = daemon.resize(session_id, w, h).await;
                }
            }

            Some(daemon_event) = async {
                match daemon_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match daemon_event {
                    protocol::Event::Output { data, .. } => {
                        if let Ok(bytes) = protocol::decode_data(&data) {
                            write_stdout(&bytes);
                        }
                    }
                    protocol::Event::Exited { .. } => {
                        result = AttachResult::SessionExited;
                        break;
                    }
                    _ => {}
                }
            }

            else => break,
        }
    }

    // Show detaching feedback (same busy overlay style)
    let (cols, rows) = crossterm::terminal::size().unwrap_or((cols, rows));
    draw_status_screen(cols, rows, &format!("Detaching {name}…"), 0);

    // Reset terminal title
    set_title("");

    drop(input_rx);
    reader.abort();

    result
}

// ── Standalone attach client ───────────────────────────────────────

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
    // internal reader thread, which would compete with attach_loop's /dev/tty reader.
    let tty = std::fs::File::open("/dev/tty").wrap_err("Could not open /dev/tty")?;
    let orig_termios =
        nix::sys::termios::tcgetattr(&tty).map_err(|e| eyre::eyre!("tcgetattr failed: {}", e))?;
    let mut raw = orig_termios.clone();
    nix::sys::termios::cfmakeraw(&mut raw);
    nix::sys::termios::tcsetattr(&tty, nix::sys::termios::SetArg::TCSANOW, &raw)
        .map_err(|e| eyre::eyre!("tcsetattr failed: {}", e))?;

    let name = short_id(&session_id, 8);
    let _result = attach_loop(&mut daemon, &session_id, name, &mut daemon_rx, None).await;

    // Restore terminal
    nix::sys::termios::tcsetattr(&tty, nix::sys::termios::SetArg::TCSANOW, &orig_termios).ok();
    write_stdout(b"\x1b[2J\x1b[H");

    let _ = daemon.detach(&session_id).await;
    std::process::exit(0);
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
}
