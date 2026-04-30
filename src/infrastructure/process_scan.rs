//! Discover Claude Code processes running outside clash's daemon and
//! correlate them to sessions on disk.
//!
//! Pure helpers — `parse_ps_line`, `parse_lsof_n_output`,
//! `correlate_wild_to_sessions`, `should_signal` — are exhaustively
//! unit-tested. IO wrappers (`gather_wild_processes`, [`LinuxProcFs`],
//! [`DarwinLsof`], [`LiveProcessProbe`]) are thin shells over the pure
//! pieces, mirroring the `parse_gitdir_content` / `detect_worktree`
//! precedent in CLAUDE.md.
//!
//! Correlation is authoritative when the running process holds the
//! session's `.jsonl` open as a file descriptor — the basename of that
//! `.jsonl` IS the session id. cwd matching is a fallback that yields a
//! correlation only when exactly one session in the list shares the
//! process's cwd; ambiguous cwd matches are treated as Unknown and the
//! key is left absent from the result map, never picked arbitrarily.
//!
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::domain::entities::Session;

// ── Types ─────────────────────────────────────────────────────────

/// A running `claude` process discovered outside clash's daemon.
///
/// All correlation-relevant data is captured at scan time so the pure
/// [`correlate_wild_to_sessions`] never needs to do IO.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WildProcess {
    pub pid: u32,
    /// Full command line as `ps` reported it.
    pub command: String,
    /// Working directory, if a probe was able to read it. `None` on
    /// permission errors or if the process exited mid-scan.
    pub cwd: Option<String>,
    /// Session ids derived from `.jsonl` files the process holds open
    /// (basename without the `.jsonl` extension). Populated by the probe
    /// during gather; empty if the probe yielded nothing or the process
    /// holds no `.jsonl` file (e.g. transient state).
    pub open_jsonl_session_ids: Vec<String>,
}

/// Outcome of [`should_signal`] — whether SIGTERM is safe right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignalDecision {
    /// PID is alive and its cmdline still starts with the `claude`
    /// basename. Proceed with SIGTERM.
    Allow,
    /// PID is no longer alive. The caller should refresh the wild list.
    ProcessExited,
    /// PID is alive but its cmdline no longer starts with `claude` —
    /// the kernel reused the PID. Do not signal.
    CmdlineChanged,
}

// ── Pure parsers ──────────────────────────────────────────────────

/// Parse one row of `ps -p <pids> -o pid=,state=,command=` output.
///
/// Returns `None` when:
/// - The row is empty / unparseable.
/// - The process state is `Z` (zombie) — its fds are gone, no point
///   correlating.
/// - The command's executable basename is not exactly `claude` — paths
///   like `/Users/foo/claude-experiments/runner` must be rejected.
///
/// The entire remaining text after the state column becomes
/// [`WildProcess::command`]; embedded whitespace, long arg lists, and
/// non-ASCII paths are preserved verbatim.
pub fn parse_ps_line(line: &str) -> Option<WildProcess> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let mut tokens = trimmed.splitn(3, char::is_whitespace);
    let pid_str = tokens.next()?.trim();
    let state = tokens.next()?.trim();
    let command = tokens.next()?.trim();

    let pid: u32 = pid_str.parse().ok()?;
    if pid == 0 {
        return None; // PID 0 / scheduler — never a real claude
    }

    // Zombie state column on Linux is `Z`/`Z+`; macOS uses `Z` too.
    if state.starts_with('Z') {
        return None;
    }

    // Match basename only — `/path/to/claude-experiments/bin/runner`
    // must not look like a wild claude.
    let executable = command.split_whitespace().next()?;
    let basename = executable.rsplit('/').next().unwrap_or(executable);
    if basename != "claude" {
        return None;
    }

    Some(WildProcess {
        pid,
        command: command.to_string(),
        cwd: None,
        open_jsonl_session_ids: Vec::new(),
    })
}

/// Parse `lsof -F n` output. Each path appears on its own line prefixed
/// with `n`; lines with other prefixes (`p`, `f`, etc.) are ignored.
///
/// Pure so it can be unit-tested without spawning lsof.
pub fn parse_lsof_n_output(bytes: &[u8]) -> Vec<PathBuf> {
    let s = std::str::from_utf8(bytes).unwrap_or("");
    s.lines()
        .filter_map(|l| l.strip_prefix('n'))
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}

// ── FdProbe ──────────────────────────────────────────────────────

/// Read filesystem state about a single process.
///
/// `open_files` is used to find the `.jsonl` that uniquely identifies
/// the Claude session — its basename IS the session id, giving an
/// authoritative correlation that cwd matching cannot. `cwd` is the
/// fallback used when the probe yields no matching `.jsonl`.
///
/// Implementations return empty / `None` on any error (permission
/// denied, process exited mid-probe, host platform without a working
/// source). Callers tolerate partial probe failures by design.
pub trait FdProbe {
    fn open_files(&self, pid: u32) -> Vec<PathBuf>;
    fn cwd(&self, pid: u32) -> Option<String>;
}

/// Linux: read `/proc/<pid>/fd/` directly via `read_dir` + `read_link`.
/// No subprocess; ~10× faster than shelling to `lsof`.
#[cfg(target_os = "linux")]
pub struct LinuxProcFs;

#[cfg(target_os = "linux")]
impl FdProbe for LinuxProcFs {
    fn open_files(&self, pid: u32) -> Vec<PathBuf> {
        read_proc_fd_dir(&format!("/proc/{}/fd", pid))
    }
    fn cwd(&self, pid: u32) -> Option<String> {
        let path = format!("/proc/{}/cwd", pid);
        std::fs::read_link(path)
            .ok()
            .and_then(|p| p.to_str().map(str::to_string))
    }
}

/// Pure helper for [`LinuxProcFs`] — exposed for tests that point at a
/// synthetic fd directory in a tempdir. Only compiled on Linux (where
/// LinuxProcFs uses it) and during `cargo test` (so the cross-platform
/// fixture test still runs on macOS CI).
#[cfg(any(target_os = "linux", test))]
pub fn read_proc_fd_dir(dir: &str) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if let Ok(target) = std::fs::read_link(entry.path()) {
            out.push(target);
        }
    }
    out
}

/// macOS / non-Linux Unix: shell out to `lsof` once per PID for files,
/// and again with `-d cwd` for the working directory. Failures
/// (permission denied, lsof missing, process gone) yield empty / `None`.
#[cfg(not(target_os = "linux"))]
pub struct DarwinLsof;

#[cfg(not(target_os = "linux"))]
impl FdProbe for DarwinLsof {
    fn open_files(&self, pid: u32) -> Vec<PathBuf> {
        let output = Command::new("lsof")
            .args(["-p", &pid.to_string(), "-F", "n"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(o) => parse_lsof_n_output(&o.stdout),
            Err(_) => Vec::new(),
        }
    }
    fn cwd(&self, pid: u32) -> Option<String> {
        let output = Command::new("lsof")
            .args(["-p", &pid.to_string(), "-d", "cwd", "-F", "n"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        parse_lsof_n_output(&output.stdout)
            .into_iter()
            .next()
            .and_then(|p| p.to_str().map(str::to_string))
    }
}

// ── Correlation ──────────────────────────────────────────────────

/// Run a full probe pass on a single PID and fill the IO-derived fields
/// of `WildProcess` (cwd + open `.jsonl` session-id basenames).
///
/// This is the boundary between the IO trait ([`FdProbe`]) and the pure
/// correlation function below — once the result is in `WildProcess`,
/// no probe is needed downstream.
pub fn fill_wild_process_io(probe: &impl FdProbe, mut w: WildProcess) -> WildProcess {
    if w.cwd.is_none() {
        w.cwd = probe.cwd(w.pid);
    }
    w.open_jsonl_session_ids = extract_jsonl_session_ids(&probe.open_files(w.pid));
    w
}

/// Pure: pull `.jsonl` basenames out of a list of open file paths.
pub fn extract_jsonl_session_ids(open_files: &[PathBuf]) -> Vec<String> {
    open_files
        .iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect()
}

/// Map wild processes to session ids — `session_id → pid`. **Pure** —
/// uses only fields already populated on [`WildProcess`].
///
/// Strategy:
/// 1. If the process has any `open_jsonl_session_ids` and at least one
///    of them matches a session in `sessions`, that's an authoritative
///    correlation; first match wins.
/// 2. If no `.jsonl` match, fall back to cwd: when **exactly one**
///    session shares the process's cwd, correlate. Multiple matches are
///    ambiguous and produce no entry (caller treats absence as Unknown
///    rather than guessing).
/// 3. Process with no `.jsonl` match and no cwd: no entry.
pub fn correlate_wild_to_sessions(
    wild: &[WildProcess],
    sessions: &[Session],
) -> HashMap<String, u32> {
    let mut result = HashMap::new();
    let session_ids: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();

    for w in wild {
        // 1. Open .jsonl → session id (authoritative).
        let mut matched: Option<String> = None;
        for sid in &w.open_jsonl_session_ids {
            if session_ids.contains(sid.as_str()) {
                matched = Some(sid.clone());
                break;
            }
        }
        if let Some(id) = matched {
            result.insert(id, w.pid);
            continue;
        }

        // 2. cwd fallback — only when exactly one session matches.
        if let Some(cwd) = &w.cwd {
            let matching_ids: Vec<&str> = sessions
                .iter()
                .filter(|s| s.cwd.as_deref() == Some(cwd.as_str()))
                .map(|s| s.id.as_str())
                .collect();
            if matching_ids.len() == 1 {
                result.insert(matching_ids[0].to_string(), w.pid);
            }
            // Zero or multiple: leave the row Unknown.
        }
    }

    result
}

// ── ProcessProbe + should_signal ─────────────────────────────────

/// Live-state probe for a single PID. Used by [`should_signal`] to
/// verify a wild PID is still the same claude process immediately
/// before SIGTERM is sent — defense against PID reuse races.
pub trait ProcessProbe {
    /// `true` if the PID currently exists (kill(pid, 0) on POSIX).
    fn is_alive(&self, pid: u32) -> bool;
    /// Current cmdline as a single string, or `None` if the PID is gone.
    fn cmdline(&self, pid: u32) -> Option<String>;
}

/// Decide whether SIGTERM is safe to send to `pid` right now.
///
/// Pure — accepts any [`ProcessProbe`] so tests can inject a fake.
pub fn should_signal(pid: u32, probe: &impl ProcessProbe) -> SignalDecision {
    if !probe.is_alive(pid) {
        return SignalDecision::ProcessExited;
    }
    let cmdline = match probe.cmdline(pid) {
        Some(c) => c,
        None => return SignalDecision::ProcessExited,
    };
    let executable = cmdline.split_whitespace().next().unwrap_or("");
    let basename = executable.rsplit('/').next().unwrap_or(executable);
    if basename != "claude" {
        return SignalDecision::CmdlineChanged;
    }
    SignalDecision::Allow
}

/// Real [`ProcessProbe`] — `kill(pid, 0)` for liveness, `ps -p` for cmdline.
pub struct LiveProcessProbe;

impl ProcessProbe for LiveProcessProbe {
    fn is_alive(&self, pid: u32) -> bool {
        // kill(pid, 0) returns Ok iff the PID exists and we have permission
        // to signal it. EPERM also implies the PID exists; treat that as
        // alive too. Use the `nix` errno wrapper for cross-platform errno.
        match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None) {
            Ok(()) => true,
            Err(nix::errno::Errno::EPERM) => true,
            Err(_) => false,
        }
    }

    fn cmdline(&self, pid: u32) -> Option<String> {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let s = std::str::from_utf8(&output.stdout).ok()?.trim();
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    }
}

// ── IO: gather wild processes ────────────────────────────────────

/// Three-stage discovery:
///   1. `pgrep -fl '^claude($|[[:space:]])'` narrows the process table
///      down to processes whose cmdline begins with the `claude`
///      basename. `[[:space:]]` is POSIX-portable; `\s` is a Perl-ism
///      that macOS pgrep treats as the literal letter `s`, which would
///      silently exclude every `claude --resume …` / `claude
///      --session-id …` process — exactly the wild sessions we want
///      to see.
///   2. `ps -p <pids> -o pid=,state=,command=` enriches with state and
///      the full command (so [`parse_ps_line`] can drop zombies).
///   3. Per surviving PID, the [`FdProbe`] populates `cwd` and the
///      `.jsonl` basenames the process holds open.
///
/// Returns an empty Vec on any IO failure — callers treat that as "no
/// wild processes detected this cycle" rather than an error.
pub fn gather_wild_processes(probe: &impl FdProbe) -> Vec<WildProcess> {
    let pgrep = Command::new("pgrep")
        .args(["-fl", r"^claude($|[[:space:]])"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let pgrep_out = match pgrep {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };

    let pids: Vec<u32> = std::str::from_utf8(&pgrep_out)
        .unwrap_or("")
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|s| s.parse().ok())
        .collect();
    if pids.is_empty() {
        return Vec::new();
    }

    let pid_arg = pids
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let ps = Command::new("ps")
        .args(["-p", &pid_arg, "-o", "pid=,state=,command="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let ps_out = match ps {
        Ok(o) => o.stdout,
        Err(_) => return Vec::new(),
    };

    std::str::from_utf8(&ps_out)
        .unwrap_or("")
        .lines()
        .filter_map(parse_ps_line)
        .map(|w| fill_wild_process_io(probe, w))
        .collect()
}

/// The [`FdProbe`] used in production. Linux reads `/proc` directly;
/// every other Unix shells out to `lsof` (most BSD derivatives ship a
/// compatible one).
#[cfg(target_os = "linux")]
pub type DefaultFdProbe = LinuxProcFs;
#[cfg(not(target_os = "linux"))]
pub type DefaultFdProbe = DarwinLsof;

/// Construct the host-appropriate probe.
pub fn default_fd_probe() -> DefaultFdProbe {
    #[cfg(target_os = "linux")]
    {
        LinuxProcFs
    }
    #[cfg(not(target_os = "linux"))]
    {
        DarwinLsof
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_ps_line ─────────────────────────────────────────────

    #[test]
    fn parse_ps_line_basic() {
        let line = "12345 S claude --resume abc";
        let p = parse_ps_line(line).expect("should parse");
        assert_eq!(p.pid, 12345);
        assert_eq!(p.command, "claude --resume abc");
        assert_eq!(p.cwd, None);
    }

    #[test]
    fn parse_ps_line_path_prefixed_basename_matches() {
        let line = "777 S /opt/homebrew/bin/claude --foo bar";
        let p = parse_ps_line(line).expect("should match basename");
        assert_eq!(p.pid, 777);
    }

    #[test]
    fn parse_ps_line_substring_does_not_match() {
        // /Users/foo/claude-experiments/bin/runner must NOT be wild claude
        let line = "555 S /Users/foo/claude-experiments/bin/runner --x";
        assert!(parse_ps_line(line).is_none());
    }

    #[test]
    fn parse_ps_line_command_named_claude_anywhere_in_path_is_safe() {
        // Path *contains* claude, basename is `clauded` — must not match
        let line = "601 S /opt/claude/clauded --watch";
        assert!(parse_ps_line(line).is_none());
    }

    #[test]
    fn parse_ps_line_zombie_filtered() {
        let line = "404 Z claude --resume xyz";
        assert!(parse_ps_line(line).is_none());
        let line2 = "405 Z+ claude --resume xyz";
        assert!(parse_ps_line(line2).is_none());
    }

    #[test]
    fn parse_ps_line_kernel_thread_or_short_command_skipped() {
        // ps may report kernel threads with bracketed names — never claude
        let line = "2 S [kthreadd]";
        assert!(parse_ps_line(line).is_none());
    }

    #[test]
    fn parse_ps_line_long_args_with_whitespace_preserved() {
        let line =
            "8888 R claude --resume abc-def-ghi --arg \"with spaces and a path /tmp/very long/x\"";
        let p = parse_ps_line(line).unwrap();
        assert_eq!(p.pid, 8888);
        assert!(p.command.contains("\"with spaces"));
        assert!(p.command.contains("/tmp/very long/x"));
    }

    #[test]
    fn parse_ps_line_leading_whitespace_tolerated() {
        // ps right-aligns PIDs by default; we strip leading whitespace.
        let line = "    99 S claude";
        let p = parse_ps_line(line).unwrap();
        assert_eq!(p.pid, 99);
    }

    #[test]
    fn parse_ps_line_empty_returns_none() {
        assert!(parse_ps_line("").is_none());
        assert!(parse_ps_line("   ").is_none());
    }

    #[test]
    fn parse_ps_line_pid_zero_rejected() {
        // PID 0 is the scheduler — never a real claude.
        let line = "0 S claude";
        assert!(parse_ps_line(line).is_none());
    }

    // ── parse_lsof_n_output ───────────────────────────────────────

    #[test]
    fn parse_lsof_n_output_extracts_paths() {
        let bytes = b"p12345\nfcwd\nn/Users/me/repo\nf3\nn/Users/me/.claude/projects/p/abc.jsonl\n";
        let paths = parse_lsof_n_output(bytes);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/me/repo"),
                PathBuf::from("/Users/me/.claude/projects/p/abc.jsonl"),
            ]
        );
    }

    #[test]
    fn parse_lsof_n_output_handles_empty() {
        assert!(parse_lsof_n_output(b"").is_empty());
    }

    #[test]
    fn parse_lsof_n_output_skips_non_n_lines() {
        let bytes = b"p123\nf0\nf1\nf2\n";
        assert!(parse_lsof_n_output(bytes).is_empty());
    }

    // ── extract_jsonl_session_ids ─────────────────────────────────

    #[test]
    fn extract_jsonl_keeps_basenames_strips_extension() {
        let paths = vec![
            PathBuf::from("/dev/tty"),
            PathBuf::from("/Users/me/.claude/projects/p/abc-def.jsonl"),
            PathBuf::from("/tmp/ignore.txt"),
            PathBuf::from("/Users/me/.claude/projects/p/xyz.jsonl"),
        ];
        let ids = extract_jsonl_session_ids(&paths);
        assert_eq!(ids, vec!["abc-def".to_string(), "xyz".to_string()]);
    }

    #[test]
    fn extract_jsonl_empty_input_empty_output() {
        assert!(extract_jsonl_session_ids(&[]).is_empty());
    }

    // ── correlate_wild_to_sessions ────────────────────────────────

    fn session_with(id: &str, cwd: Option<&str>) -> Session {
        Session {
            id: id.to_string(),
            cwd: cwd.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    fn wild(pid: u32, cwd: Option<&str>, open_jsonl: &[&str]) -> WildProcess {
        WildProcess {
            pid,
            command: "claude".into(),
            cwd: cwd.map(|s| s.to_string()),
            open_jsonl_session_ids: open_jsonl.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn correlate_basename_match_wins_over_cwd() {
        // Two sessions share /repo. Open .jsonl points at sess-A — must
        // pick sess-A, not pick arbitrarily from the cwd-matching pair.
        let wild = vec![wild(123, Some("/repo"), &["sess-A"])];
        let sessions = vec![
            session_with("sess-A", Some("/repo")),
            session_with("sess-B", Some("/repo")),
        ];
        let map = correlate_wild_to_sessions(&wild, &sessions);
        assert_eq!(map.get("sess-A"), Some(&123));
        assert!(!map.contains_key("sess-B"));
    }

    #[test]
    fn correlate_cwd_unique_match_used_when_no_fd_match() {
        let wild = vec![wild(200, Some("/repo"), &[])];
        let sessions = vec![session_with("only-one", Some("/repo"))];
        let map = correlate_wild_to_sessions(&wild, &sessions);
        assert_eq!(map.get("only-one"), Some(&200));
    }

    #[test]
    fn correlate_cwd_ambiguous_yields_no_entry() {
        // Two sessions in same repo, no .jsonl info → ambiguous → Unknown.
        let wild = vec![wild(300, Some("/repo"), &[])];
        let sessions = vec![
            session_with("a", Some("/repo")),
            session_with("b", Some("/repo")),
        ];
        let map = correlate_wild_to_sessions(&wild, &sessions);
        assert!(map.is_empty(), "ambiguous cwd must NOT pick a session");
    }

    #[test]
    fn correlate_no_cwd_no_fd_yields_no_entry() {
        let wild = vec![wild(400, None, &[])];
        let sessions = vec![session_with("a", Some("/repo"))];
        let map = correlate_wild_to_sessions(&wild, &sessions);
        assert!(map.is_empty());
    }

    #[test]
    fn correlate_ignores_open_jsonl_for_unknown_session() {
        // Process holds a .jsonl open whose basename matches NO session
        // in the list — that's not the session we're looking for.
        let wild = vec![wild(500, None, &["stale-id"])];
        let sessions = vec![session_with("abc", None)];
        let map = correlate_wild_to_sessions(&wild, &sessions);
        assert!(map.is_empty());
    }

    #[test]
    fn correlate_multiple_processes_independent() {
        let wild = vec![wild(11, Some("/r1"), &[]), wild(22, Some("/r2"), &[])];
        let sessions = vec![
            session_with("s1", Some("/r1")),
            session_with("s2", Some("/r2")),
        ];
        let map = correlate_wild_to_sessions(&wild, &sessions);
        assert_eq!(map.get("s1"), Some(&11));
        assert_eq!(map.get("s2"), Some(&22));
    }

    // ── fill_wild_process_io ──────────────────────────────────────

    /// Test probe — returns canned cwd + open files keyed by pid.
    struct FakeProbe {
        cwds: HashMap<u32, String>,
        files: HashMap<u32, Vec<PathBuf>>,
    }
    impl FdProbe for FakeProbe {
        fn open_files(&self, pid: u32) -> Vec<PathBuf> {
            self.files.get(&pid).cloned().unwrap_or_default()
        }
        fn cwd(&self, pid: u32) -> Option<String> {
            self.cwds.get(&pid).cloned()
        }
    }

    #[test]
    fn fill_wild_process_io_populates_cwd_and_open_jsonl() {
        let probe = FakeProbe {
            cwds: HashMap::from([(7, "/repo".to_string())]),
            files: HashMap::from([(
                7,
                vec![
                    PathBuf::from("/dev/tty"),
                    PathBuf::from("/Users/me/.claude/projects/p/sess.jsonl"),
                ],
            )]),
        };
        let w = WildProcess {
            pid: 7,
            command: "claude".into(),
            ..Default::default()
        };
        let filled = fill_wild_process_io(&probe, w);
        assert_eq!(filled.cwd.as_deref(), Some("/repo"));
        assert_eq!(filled.open_jsonl_session_ids, vec!["sess".to_string()]);
    }

    #[test]
    fn fill_wild_process_io_does_not_clobber_existing_cwd() {
        // If cwd was already set (e.g. by a prior probe), keep it.
        let probe = FakeProbe {
            cwds: HashMap::from([(7, "/probed".to_string())]),
            files: HashMap::new(),
        };
        let w = WildProcess {
            pid: 7,
            command: "claude".into(),
            cwd: Some("/preexisting".to_string()),
            ..Default::default()
        };
        let filled = fill_wild_process_io(&probe, w);
        assert_eq!(filled.cwd.as_deref(), Some("/preexisting"));
    }

    // ── should_signal ─────────────────────────────────────────────

    struct FakeProcessProbe {
        alive: HashSet<u32>,
        cmdlines: HashMap<u32, String>,
    }
    impl ProcessProbe for FakeProcessProbe {
        fn is_alive(&self, pid: u32) -> bool {
            self.alive.contains(&pid)
        }
        fn cmdline(&self, pid: u32) -> Option<String> {
            self.cmdlines.get(&pid).cloned()
        }
    }

    #[test]
    fn should_signal_allow_when_alive_and_cmdline_starts_with_claude() {
        let probe = FakeProcessProbe {
            alive: HashSet::from([42]),
            cmdlines: HashMap::from([(42, "claude --resume abc".to_string())]),
        };
        assert_eq!(should_signal(42, &probe), SignalDecision::Allow);
    }

    #[test]
    fn should_signal_allow_with_path_prefixed_basename() {
        let probe = FakeProcessProbe {
            alive: HashSet::from([42]),
            cmdlines: HashMap::from([(42, "/opt/homebrew/bin/claude --foo".to_string())]),
        };
        assert_eq!(should_signal(42, &probe), SignalDecision::Allow);
    }

    #[test]
    fn should_signal_process_exited_when_dead() {
        let probe = FakeProcessProbe {
            alive: HashSet::new(),
            cmdlines: HashMap::new(),
        };
        assert_eq!(should_signal(99, &probe), SignalDecision::ProcessExited);
    }

    #[test]
    fn should_signal_process_exited_when_alive_but_no_cmdline() {
        // Edge case: kill(pid, 0) said alive but ps didn't return cmdline
        // (process exited mid-call). Treat as ProcessExited.
        let probe = FakeProcessProbe {
            alive: HashSet::from([42]),
            cmdlines: HashMap::new(),
        };
        assert_eq!(should_signal(42, &probe), SignalDecision::ProcessExited);
    }

    #[test]
    fn should_signal_cmdline_changed_when_basename_differs() {
        // PID was reused after the original claude exited.
        let probe = FakeProcessProbe {
            alive: HashSet::from([42]),
            cmdlines: HashMap::from([(42, "/usr/bin/zsh".to_string())]),
        };
        assert_eq!(should_signal(42, &probe), SignalDecision::CmdlineChanged);
    }

    #[test]
    fn should_signal_cmdline_changed_for_substring_match() {
        // Substring `claude` in path but basename is different — must NOT
        // be Allow. Mirrors the parse_ps_line basename rule.
        let probe = FakeProcessProbe {
            alive: HashSet::from([42]),
            cmdlines: HashMap::from([(
                42,
                "/Users/foo/claude-experiments/bin/runner --x".to_string(),
            )]),
        };
        assert_eq!(should_signal(42, &probe), SignalDecision::CmdlineChanged);
    }

    // ── read_proc_fd_dir ──────────────────────────────────────────

    #[test]
    fn read_proc_fd_dir_missing_returns_empty() {
        let paths = read_proc_fd_dir("/nonexistent/path/we/do/not/expect/12345");
        assert!(paths.is_empty());
    }

    #[test]
    fn read_proc_fd_dir_reads_symlink_targets() {
        // Build a synthetic /proc/<pid>/fd-shaped directory with symlinks.
        // Works on any UNIX; the LinuxProcFs impl just delegates here.
        let tmp = tempfile::tempdir().unwrap();
        let target_a = tmp.path().join("a.jsonl");
        let target_b = tmp.path().join("b.txt");
        std::fs::write(&target_a, "x").unwrap();
        std::fs::write(&target_b, "x").unwrap();
        let fd_dir = tmp.path().join("fd");
        std::fs::create_dir_all(&fd_dir).unwrap();
        std::os::unix::fs::symlink(&target_a, fd_dir.join("3")).unwrap();
        std::os::unix::fs::symlink(&target_b, fd_dir.join("4")).unwrap();

        let mut got = read_proc_fd_dir(fd_dir.to_str().unwrap());
        got.sort();
        let mut want = vec![target_a, target_b];
        want.sort();
        assert_eq!(got, want);
    }
}
