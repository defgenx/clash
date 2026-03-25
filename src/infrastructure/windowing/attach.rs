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

/// Detect Ctrl+B in any of the three common terminal encodings:
///   - `0x02` — standard raw byte (most terminals in normal mode)
///   - `ESC[98;5u` — Kitty keyboard protocol (CSI u)
///   - `ESC[27;5;98~` — xterm modifyOtherKeys mode (iTerm2, etc.)
fn is_ctrl_b(bytes: &[u8]) -> bool {
    // Standard raw byte
    if bytes.contains(&0x02) {
        return true;
    }
    // Kitty CSI u: \x1b[98;5u (7 bytes)
    if bytes.windows(7).any(|w| w == b"\x1b[98;5u") {
        return true;
    }
    // xterm modifyOtherKeys: \x1b[27;5;98~ (10 bytes)
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

/// Run the I/O passthrough loop between stdin/stdout and a daemon PTY session.
///
/// Reads input from a freshly opened `/dev/tty` fd to avoid competing with
/// crossterm's internal reader thread (which may linger on fd 0 after
/// EventStream is dropped). Daemon output is written directly to stdout.
///
/// - `hint` is the text shown in the footer bar (e.g., "Ctrl+B detach").
/// - Returns an `AttachResult` indicating why the loop ended.
/// - The caller is responsible for calling `daemon.detach()` afterwards.
pub async fn attach_loop(
    daemon: &mut DaemonClient,
    session_id: &str,
    daemon_rx: &mut Option<mpsc::UnboundedReceiver<protocol::Event>>,
    hint: &str,
) -> AttachResult {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((120, 40));

    // PTY = rows-1 (footer takes the last row)
    let body_rows = rows.saturating_sub(1).max(1);
    let _ = daemon.resize(session_id, cols, body_rows).await;

    // Set scroll region + draw footer + position cursor
    draw_attach_chrome(cols, rows, session_id, hint);

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
                // Ctrl+B = detach. Supported encodings:
                //   0x02              — standard raw byte
                //   ESC[98;5u         — Kitty CSI u protocol
                //   ESC[27;5;98~      — xterm modifyOtherKeys
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

            // Terminal resized → resize PTY + redraw footer
            Some(_) = async {
                match sigwinch.as_mut() {
                    Some(sig) => sig.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok((w, h)) = crossterm::terminal::size() {
                    let body = h.saturating_sub(1).max(1);
                    let _ = daemon.resize(session_id, w, body).await;
                    draw_attach_chrome(w, h, session_id, hint);
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
                            unsafe {
                                libc::write(1, bytes.as_ptr() as *const libc::c_void, bytes.len());
                            }
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

    // Reset scroll region to full terminal before leaving
    if let Ok((_, h)) = crossterm::terminal::size() {
        let reset = format!("\x1b[1;{}r", h);
        unsafe {
            libc::write(1, reset.as_ptr() as *const libc::c_void, reset.len());
        }
    }

    drop(input_rx);
    reader.abort();

    result
}

/// Draw the attach chrome: scroll region + footer bar.
/// Leaves cursor at row 2 so Claude has a blank line at the top.
pub fn draw_attach_chrome(cols: u16, rows: u16, session_id: &str, hint: &str) {
    let short = short_id(session_id, 8);
    let footer = format!(" clash | {}  {}", short, hint);
    let pad = cols as usize - footer.len().min(cols as usize);

    // Scroll region: rows 1 to rows-1 (footer on row `rows`)
    // Footer: dark bar with session info
    // Cursor: row 2 (blank line at top for visual breathing room)
    let chrome = format!(
        "\x1b[1;{}r\x1b[{};1H\x1b[48;5;236m\x1b[38;2;90;90;110m{}{}\x1b[0m\x1b[2;1H",
        rows - 1,
        rows,
        footer,
        " ".repeat(pad),
    );
    unsafe {
        libc::write(1, chrome.as_ptr() as *const libc::c_void, chrome.len());
    }
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

    let _result = attach_loop(&mut daemon, &session_id, &mut daemon_rx, "Ctrl+B exit").await;

    // Restore terminal
    nix::sys::termios::tcsetattr(&tty, nix::sys::termios::SetArg::TCSANOW, &orig_termios).ok();
    // Clear screen and reset cursor
    unsafe {
        libc::write(1, b"\x1b[2J\x1b[H".as_ptr() as *const libc::c_void, 10);
    }

    let _ = daemon.detach(&session_id).await;

    // Exit immediately so the terminal window closes on detach.
    // No message needed — the user pressed Ctrl+B intentionally.
    std::process::exit(0);
}
