//! Per-session PTY management — each session is an isolated PTY subprocess.
//!
//! The daemon owns PtySessions. Each one:
//! - Spawns claude in a PTY (via openpty + fork)
//! - Runs output-reader thread that feeds a broadcast channel
//! - Maintains a vt100 screen mirror for status detection
//! - Accumulates raw output history (capped at 4 MB) for full replay on attach
//! - Accepts input writes from any attached client

use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use nix::pty::openpty;
use tokio::sync::broadcast;

/// Backpressure: max buffered output frames per client before dropping.
const OUTPUT_CHANNEL_CAPACITY: usize = 4096;

/// Max raw output history kept per session (4 MB). Oldest bytes are discarded
/// when this limit is reached so the session never grows unbounded.
const MAX_HISTORY_BYTES: usize = 4 * 1024 * 1024;

/// Default screen size for status detection parser.
const STATUS_SCREEN_ROWS: u16 = 50;
const STATUS_SCREEN_COLS: u16 = 120;

/// A managed PTY session.
pub struct PtySession {
    pub session_id: String,
    /// Optional human-readable label for the session.
    pub name: Option<String>,
    pub pid: u32,
    pub created_at: u64,
    pub cwd: String,
    master_fd: i32,
    /// Keep the OwnedFd alive — dropping it closes the master PTY fd.
    _master_owned: std::os::fd::OwnedFd,
    alive: Arc<AtomicBool>,
    /// Broadcast channel for output — subscribers get output frames.
    output_tx: broadcast::Sender<Vec<u8>>,
    /// Timestamp of last output received (epoch secs).
    last_output_at: Arc<std::sync::Mutex<u64>>,
    /// vt100 screen mirror — used to detect status by reading screen content.
    screen: Arc<std::sync::Mutex<vt100::Parser>>,
    /// Raw output history — replayed on attach for full session restore.
    history: Arc<std::sync::Mutex<Vec<u8>>>,
    _reader_handle: std::thread::JoinHandle<()>,
    _waiter_handle: std::thread::JoinHandle<()>,
}

impl PtySession {
    /// Spawn a new PTY session.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        session_id: String,
        name: Option<String>,
        bin: &str,
        args: &[String],
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
        env_vars: &HashMap<String, String>,
    ) -> std::io::Result<Self> {
        let pty = openpty(None, None).map_err(std::io::Error::other)?;

        let master_fd = pty.master.as_raw_fd();
        let slave_fd = pty.slave.as_raw_fd();

        // Set terminal size
        if cols > 0 && rows > 0 {
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            unsafe { libc::ioctl(master_fd, libc::TIOCSWINSZ as libc::c_ulong, &ws) };
        }

        let mut cmd = Command::new(bin);

        // Inherit parent environment (includes Vertex, Bedrock, and other cloud provider vars).
        // Additional env vars from repo config override on top.
        for (key, val) in env_vars {
            cmd.env(key, val);
        }

        if let Some(dir) = cwd {
            let p = PathBuf::from(dir);
            if p.is_dir() {
                cmd.current_dir(p);
            }
        }

        if !args.is_empty() {
            cmd.args(args);
        }

        unsafe {
            cmd.pre_exec(move || {
                libc::setsid();
                libc::ioctl(slave_fd, libc::TIOCSCTTY as libc::c_ulong, 0);
                libc::dup2(slave_fd, 0);
                libc::dup2(slave_fd, 1);
                libc::dup2(slave_fd, 2);
                if slave_fd > 2 {
                    libc::close(slave_fd);
                }
                Ok(())
            });
        }

        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        tracing::info!("Daemon spawning: {} {:?} cwd={:?}", bin, args, cwd);
        let mut child = cmd.spawn()?;
        let pid = child.id();
        tracing::info!("Daemon PTY session {} spawned pid={}", session_id, pid);

        // Drop slave end in parent
        drop(pty.slave);

        let alive = Arc::new(AtomicBool::new(true));
        let (output_tx, _) = broadcast::channel(OUTPUT_CHANNEL_CAPACITY);
        let last_output_at: Arc<std::sync::Mutex<u64>> = Arc::new(std::sync::Mutex::new(0));

        // vt100 parser for screen content analysis
        let screen_rows = if rows > 0 { rows } else { STATUS_SCREEN_ROWS };
        let screen_cols = if cols > 0 { cols } else { STATUS_SCREEN_COLS };
        let screen: Arc<std::sync::Mutex<vt100::Parser>> = Arc::new(std::sync::Mutex::new(
            vt100::Parser::new(screen_rows, screen_cols, 0),
        ));

        let history: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        // Output reader thread: master_fd → broadcast channel + screen mirror + history
        let tx = output_tx.clone();
        let alive2 = alive.clone();
        let sid = session_id.clone();
        let last_out2 = last_output_at.clone();
        let screen2 = screen.clone();
        let history2 = history.clone();
        let reader_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while alive2.load(Ordering::SeqCst) {
                let mut pfd = libc::pollfd {
                    fd: master_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let ret = unsafe { libc::poll(&mut pfd, 1, 200) };
                if ret <= 0 {
                    continue;
                }

                let n = unsafe { libc::read(master_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
                if n <= 0 {
                    tracing::info!("PTY output EOF for session {}", sid);
                    break;
                }

                let data = buf[..n as usize].to_vec();

                // Track output timestamp
                if let Ok(mut ts) = last_out2.lock() {
                    *ts = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                }

                // Feed into vt100 screen mirror
                if let Ok(mut parser) = screen2.lock() {
                    parser.process(&data);
                }

                // Append to history buffer (capped).
                // Detect screen-clear escape sequences — if present, truncate
                // so reattach only replays post-clear content (fixes /clear).
                if let Ok(mut h) = history2.lock() {
                    if let Some(clear_pos) = find_last_screen_clear(&data) {
                        h.clear();
                        h.extend_from_slice(&data[clear_pos..]);
                    } else {
                        h.extend_from_slice(&data);
                    }
                    if h.len() > MAX_HISTORY_BYTES {
                        let excess = h.len() - MAX_HISTORY_BYTES;
                        h.drain(..excess);
                    }
                }

                // Broadcast to live subscribers
                let _ = tx.send(data);
            }
        });

        // Waiter thread: monitors child exit
        let alive3 = alive.clone();
        let sid2 = session_id.clone();
        let waiter_handle = std::thread::spawn(move || {
            match child.wait() {
                Ok(status) => {
                    tracing::info!("Session {} exited: {}", sid2, status);
                }
                Err(e) => {
                    tracing::error!("Session {} wait error: {}", sid2, e);
                }
            }
            alive3.store(false, Ordering::SeqCst);
        });

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(Self {
            session_id,
            name,
            pid,
            created_at: now,
            cwd: cwd.unwrap_or("").to_string(),
            master_fd,
            _master_owned: pty.master,
            alive,
            output_tx,
            last_output_at,
            screen,
            history,
            _reader_handle: reader_handle,
            _waiter_handle: waiter_handle,
        })
    }

    /// Write input bytes to the PTY master.
    pub fn write_input(&self, data: &[u8]) -> std::io::Result<()> {
        if !self.is_alive() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "session dead",
            ));
        }
        let n = unsafe { libc::write(self.master_fd, data.as_ptr() as *const _, data.len()) };
        if n < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Resize the PTY.
    ///
    /// Resets the screen mirror so that subsequent snapshots reflect
    /// the new size. The child process receives SIGWINCH and re-renders.
    pub fn resize(&self, cols: u16, rows: u16) {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe { libc::ioctl(self.master_fd, libc::TIOCSWINSZ as libc::c_ulong, &ws) };
        // Send SIGWINCH to the child process group
        unsafe { libc::kill(-(self.pid as i32), libc::SIGWINCH) };
        // Reset screen mirror — old content was rendered at a different size.
        if let Ok(mut parser) = self.screen.lock() {
            parser.set_size(rows, cols);
        }
    }

    /// Return the full raw output history for replay on attach.
    ///
    /// This is more faithful than `screen_snapshot()` because it replays the
    /// actual PTY byte stream, letting the terminal reconstruct the exact state
    /// including any internal alternate-screen content.
    pub fn output_history(&self) -> Vec<u8> {
        self.history
            .lock()
            .ok()
            .map(|h| h.clone())
            .unwrap_or_default()
    }

    /// Detect session status by analyzing the vt100 screen content.
    ///
    /// Reads the bottom lines of the terminal screen and pattern-matches
    /// against known Claude Code UI elements to determine the real state.
    pub fn status(&self) -> &'static str {
        if !self.is_alive() {
            return "idle";
        }

        // Check if we've ever received output
        let last = self.last_output_at.lock().map(|ts| *ts).unwrap_or(0);
        if last == 0 {
            return "starting";
        }

        // Read the screen content (bottom N lines)
        let bottom_text = self.read_bottom_lines(10);

        // Pattern-match against Claude Code UI states
        detect_claude_status(&bottom_text, last)
    }

    /// Read the bottom N lines of the vt100 screen as plain text.
    fn read_bottom_lines(&self, n: u16) -> String {
        let parser = match self.screen.lock() {
            Ok(p) => p,
            Err(_) => return String::new(),
        };
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let start_row = rows.saturating_sub(n);
        let mut text = String::new();
        for row in start_row..rows {
            for col in 0..cols {
                if let Some(cell) = screen.cell(row, col) {
                    let contents = cell.contents();
                    if contents.is_empty() {
                        text.push(' ');
                    } else {
                        text.push_str(&contents);
                    }
                }
            }
            text.push('\n');
        }
        text
    }

    /// Subscribe to output (each subscriber gets its own bounded receiver).
    pub fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    /// Number of active subscribers (attached clients).
    pub fn subscriber_count(&self) -> usize {
        self.output_tx.receiver_count()
    }

    /// Is the child process still alive?
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Gracefully stop the child process.
    ///
    /// Strategy: /exit → SIGTERM → SIGKILL
    /// 1. Send "/exit\n" to the PTY — Claude's built-in quit command
    /// 2. After 3s, SIGTERM to process group if still alive
    /// 3. After 3 more seconds, SIGKILL if still alive
    pub fn kill(&self) {
        if self.is_alive() {
            // Step 1: Send /exit command to the PTY
            let exit_cmd = b"/exit\n";
            unsafe {
                libc::write(
                    self.master_fd,
                    exit_cmd.as_ptr() as *const _,
                    exit_cmd.len(),
                )
            };

            let pid = self.pid;
            let alive = self.alive.clone();
            std::thread::spawn(move || {
                // Step 2: After 3s, SIGTERM to process group
                std::thread::sleep(std::time::Duration::from_secs(3));
                if alive.load(Ordering::SeqCst) {
                    unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
                }

                // Step 3: After 3 more seconds, SIGKILL
                std::thread::sleep(std::time::Duration::from_secs(3));
                if alive.load(Ordering::SeqCst) {
                    unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
                }
            });
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        // _master_owned (OwnedFd) is dropped automatically, closing the master fd
    }
}

/// Find the last screen-clear escape sequence in a byte slice.
///
/// Detects `\x1b[2J` (ED 2 — erase entire display) and `\x1b[3J` (ED 3 —
/// erase scrollback). Returns the offset of the `\x1b` that starts the
/// last matching sequence, so the history buffer can be truncated there.
fn find_last_screen_clear(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    (0..=data.len() - 4).rev().find(|&i| {
        data[i] == 0x1b
            && data[i + 1] == b'['
            && (data[i + 2] == b'2' || data[i + 2] == b'3')
            && data[i + 3] == b'J'
    })
}

/// Analyze screen text to detect Claude Code's current state.
///
/// Claude Code renders specific UI elements at different states:
/// - Input prompt: `>`, `❯`, cursor at empty line after prompt
/// - Tool approval: `Yes`, `No`, `Allow`, permission-related text, `(Y/n)`, `(y/N)`
/// - Thinking: `Thinking…`, spinner characters, `...`
/// - Working: active tool output, file operations, code generation
fn detect_claude_status(screen_text: &str, last_output_at: u64) -> &'static str {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed = now.saturating_sub(last_output_at);

    // Normalize: trim each line, collect non-empty bottom lines
    let lines: Vec<&str> = screen_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    // Check the last few non-empty lines for patterns
    let last_lines: Vec<&str> = lines.iter().rev().take(5).copied().collect();
    let bottom = last_lines.join(" ").to_lowercase();

    // ── Tool approval / permission prompts ──────────────────────
    // Claude Code shows approval prompts with Yes/No options
    if has_approval_prompt(&bottom, &last_lines) {
        return "prompting";
    }

    // ── User input prompt ───────────────────────────────────────
    // Claude Code shows a prompt when waiting for user input
    if has_input_prompt(&bottom, &last_lines) {
        return "waiting";
    }

    // ── Thinking indicator ──────────────────────────────────────
    if has_thinking_indicator(&bottom, &last_lines) {
        return "thinking";
    }

    // ── Timing-based fallback ───────────────────────────────────
    // If screen parsing doesn't match a clear pattern, use timing
    if elapsed <= 2 {
        "running"
    } else if elapsed <= 8 {
        "thinking"
    } else {
        // Long silence + no prompt detected → assume waiting for user input.
        // Only hooks (SessionEnd) or manual drop should set idle.
        "waiting"
    }
}

/// Check if the screen shows a tool/action approval prompt.
fn has_approval_prompt(bottom: &str, last_lines: &[&str]) -> bool {
    // Common approval patterns in Claude Code
    let approval_patterns = [
        "(y/n)",
        "(y)",
        "(n)",
        "yes/no",
        "allow",
        "approve",
        "permit",
        "deny",
        "reject",
        "do you want",
        "would you like",
        "press y",
        "press enter to",
        // Claude Code specific tool approval UI
        "run this",
        "execute",
        "write to",
        "create file",
        "delete file",
        "modify",
        "allow once",
        "allow always",
    ];

    for pattern in &approval_patterns {
        if bottom.contains(pattern) {
            return true;
        }
    }

    // Check for highlighted Yes/No buttons (common in Claude Code TUI)
    for line in last_lines {
        let lower = line.to_lowercase();
        // Pattern: "Yes" and "No" on the same line or adjacent lines
        if (lower.contains("yes") && lower.contains("no"))
            || lower.contains("[yes]")
            || lower.contains("[no]")
        {
            return true;
        }
    }

    false
}

/// Check if the screen shows a user input prompt.
fn has_input_prompt(bottom: &str, last_lines: &[&str]) -> bool {
    // Check the very last non-empty line for standalone prompt characters.
    // Only match when the line IS the prompt (short, just the marker),
    // not when ">" appears at the end of arbitrary output.
    if let Some(last) = last_lines.first() {
        let trimmed = last.trim();
        if trimmed == ">" || trimmed == "❯" || trimmed == "$" || trimmed == "%" || trimmed == ">>>"
        {
            return true;
        }
    }

    // "what would you like" / "how can i help" patterns (Claude greeting)
    let prompt_phrases = [
        "what would you like",
        "how can i help",
        "enter a prompt",
        "type a message",
        "your message",
    ];
    for phrase in &prompt_phrases {
        if bottom.contains(phrase) {
            return true;
        }
    }

    false
}

/// Check if the screen shows a thinking/processing indicator.
fn has_thinking_indicator(bottom: &str, _last_lines: &[&str]) -> bool {
    let thinking_patterns = [
        "thinking",
        "generating",
        "processing",
        "loading",
        "analyzing",
        "searching",
        "reading",
        "writing",
        "⠋",
        "⠙",
        "⠹",
        "⠸",
        "⠼",
        "⠴",
        "⠦",
        "⠧",
        "⠇",
        "⠏", // braille spinner
        "◐",
        "◓",
        "◑",
        "◒", // circle spinner
        "⣾",
        "⣽",
        "⣻",
        "⢿",
        "⡿",
        "⣟",
        "⣯",
        "⣷", // dots spinner
    ];

    for pattern in &thinking_patterns {
        if bottom.contains(pattern) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_idle_prompt() {
        let screen = "Some output\n\n>\n";
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(detect_claude_status(screen, now - 10), "waiting");
    }

    #[test]
    fn test_detect_approval_prompt() {
        let screen = "Claude wants to write to file.rs\n  Yes    No\n";
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(detect_claude_status(screen, now - 10), "prompting");
    }

    #[test]
    fn test_detect_thinking() {
        let screen = "Thinking...\n⠋ Processing request\n";
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(detect_claude_status(screen, now), "thinking");
    }

    #[test]
    fn test_detect_running_recent_output() {
        let screen = "Building project...\nCompiling src/main.rs\n";
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(detect_claude_status(screen, now), "running");
    }

    #[test]
    fn test_detect_yn_prompt() {
        let screen = "Allow tool use? (y/n)\n";
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(detect_claude_status(screen, now - 10), "prompting");
    }

    #[test]
    fn test_find_last_screen_clear_ed2() {
        // \x1b[2J = Erase entire display
        let data = b"hello\x1b[2Jworld";
        assert_eq!(find_last_screen_clear(data), Some(5));
    }

    #[test]
    fn test_find_last_screen_clear_ed3() {
        // \x1b[3J = Erase scrollback
        let data = b"old stuff\x1b[3Jnew stuff";
        assert_eq!(find_last_screen_clear(data), Some(9));
    }

    #[test]
    fn test_find_last_screen_clear_multiple() {
        // Multiple clears — returns the last one
        let data = b"a\x1b[2Jb\x1b[2Jc";
        assert_eq!(find_last_screen_clear(data), Some(6));
    }

    #[test]
    fn test_find_last_screen_clear_none() {
        let data = b"no clear sequences here";
        assert_eq!(find_last_screen_clear(data), None);
    }

    #[test]
    fn test_find_last_screen_clear_too_short() {
        let data = b"\x1b[2";
        assert_eq!(find_last_screen_clear(data), None);
    }

    #[test]
    fn test_find_last_screen_clear_with_cursor_home() {
        // Common pattern: cursor home + clear = \x1b[H\x1b[2J
        let data = b"old\x1b[H\x1b[2Jnew";
        assert_eq!(find_last_screen_clear(data), Some(6));
    }
}
