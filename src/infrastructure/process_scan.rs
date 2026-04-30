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
//! NOTE: many items here are intentionally unused at the bin call-graph
//! today — they are wired up in subsequent tasks of the
//! wild-session-adoption spec (background scan in app.rs, then the
//! TakeoverWildSession effect translator). The lib's tests exercise
//! every path.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::domain::entities::Session;

// ── Types ─────────────────────────────────────────────────────────

/// A running `claude` process discovered outside clash's daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WildProcess {
    pub pid: u32,
    /// Full command line as `ps` reported it.
    pub command: String,
    /// Working directory, if a probe was able to read it. `None` on
    /// permission errors or if the process exited mid-scan.
    pub cwd: Option<String>,
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

/// Read the file descriptors a process holds open, as filesystem paths.
///
/// Used to find the open `.jsonl` that uniquely identifies which Claude
/// session the wild process belongs to. The basename of that `.jsonl`
/// IS the session id, so this gives an authoritative correlation that
/// cwd matching cannot.
///
/// Implementations return an empty `Vec` on errors (permission denied,
/// process exited mid-probe, host platform without a working source) —
/// callers fall back to cwd matching in that case.
pub trait FdProbe {
    fn open_files(&self, pid: u32) -> Vec<PathBuf>;
}

/// Linux: read `/proc/<pid>/fd/` directly via `read_dir` + `read_link`.
/// No subprocess; ~10× faster than shelling to `lsof`.
pub struct LinuxProcFs;

impl FdProbe for LinuxProcFs {
    fn open_files(&self, pid: u32) -> Vec<PathBuf> {
        read_proc_fd_dir(&format!("/proc/{}/fd", pid))
    }
}

/// Pure helper for [`LinuxProcFs`] — exposed for tests that point at a
/// synthetic fd directory in a tempdir.
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

/// macOS: shell out to `lsof -p <pid> -F n` (one path per `n`-line),
/// invoked once per PID. Failures (permission denied, lsof missing,
/// process gone) yield an empty Vec.
pub struct DarwinLsof;

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
}

// ── Correlation ──────────────────────────────────────────────────

/// Map wild processes to session ids — `session_id → pid`.
///
/// Strategy:
/// 1. Ask the [`FdProbe`] for the process's open files. Any `.jsonl`
///    whose basename matches a session in `sessions` gives an
///    authoritative correlation; first match wins.
/// 2. If the probe yielded no match, fall back to cwd: if **exactly
///    one** session has the same cwd, correlate. Multiple matches are
///    ambiguous and produce no entry (caller treats absence as Unknown
///    rather than guessing).
/// 3. Process with no probe match and no cwd: no entry.
pub fn correlate_wild_to_sessions(
    wild: &[WildProcess],
    probe: &impl FdProbe,
    sessions: &[Session],
) -> HashMap<String, u32> {
    let mut result = HashMap::new();
    let session_ids: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();

    for w in wild {
        // 1. FdProbe — open .jsonl basename = session id.
        let mut matched: Option<String> = None;
        for path in probe.open_files(w.pid) {
            let ext_jsonl = path.extension().and_then(|e| e.to_str()) == Some("jsonl");
            if !ext_jsonl {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s,
                None => continue,
            };
            if session_ids.contains(stem) {
                matched = Some(stem.to_string());
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
        // kill(pid, 0) returns 0 iff the PID exists and we have permission
        // to signal it. EPERM (-1, errno=EPERM) also implies the PID
        // exists; we treat that as alive too.
        let rc = unsafe { libc::kill(pid as i32, 0) };
        if rc == 0 {
            return true;
        }
        let errno = unsafe { *libc::__error() };
        errno == libc::EPERM
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

/// Two-stage discovery:
///   1. `pgrep -fl '^claude($|\\s)'` to narrow the process table down to
///      processes whose cmdline begins with the `claude` basename.
///   2. `ps -p <pids> -o pid=,state=,command=` to enrich with state
///      (so [`parse_ps_line`] can drop zombies) and the full command.
///
/// Returns an empty Vec on any IO failure — callers treat that as "no
/// wild processes detected this cycle" rather than an error.
pub fn gather_wild_processes() -> Vec<WildProcess> {
    let pgrep = Command::new("pgrep")
        .args(["-fl", r"^claude($|\s)"])
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
        .collect()
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

    // ── correlate_wild_to_sessions ────────────────────────────────

    /// Test fake — returns canned open-file lists keyed by pid.
    struct FakeFdProbe(HashMap<u32, Vec<PathBuf>>);
    impl FdProbe for FakeFdProbe {
        fn open_files(&self, pid: u32) -> Vec<PathBuf> {
            self.0.get(&pid).cloned().unwrap_or_default()
        }
    }

    fn session_with(id: &str, cwd: Option<&str>) -> Session {
        Session {
            id: id.to_string(),
            cwd: cwd.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn correlate_basename_match_wins_over_cwd() {
        let wild = vec![WildProcess {
            pid: 123,
            command: "claude --resume sess-A".into(),
            cwd: Some("/repo".into()),
        }];
        let probe = FakeFdProbe(HashMap::from([(
            123,
            vec![PathBuf::from("/Users/me/.claude/projects/p/sess-A.jsonl")],
        )]));
        let sessions = vec![
            session_with("sess-A", Some("/repo")),
            session_with("sess-B", Some("/repo")),
        ];
        let map = correlate_wild_to_sessions(&wild, &probe, &sessions);
        assert_eq!(map.get("sess-A"), Some(&123));
        assert!(!map.contains_key("sess-B"));
    }

    #[test]
    fn correlate_cwd_unique_match_used_when_no_fd_match() {
        let wild = vec![WildProcess {
            pid: 200,
            command: "claude".into(),
            cwd: Some("/repo".into()),
        }];
        let probe = FakeFdProbe(HashMap::new());
        let sessions = vec![session_with("only-one", Some("/repo"))];
        let map = correlate_wild_to_sessions(&wild, &probe, &sessions);
        assert_eq!(map.get("only-one"), Some(&200));
    }

    #[test]
    fn correlate_cwd_ambiguous_yields_no_entry() {
        // Two sessions in same repo, no FdProbe info → ambiguous → Unknown.
        let wild = vec![WildProcess {
            pid: 300,
            command: "claude".into(),
            cwd: Some("/repo".into()),
        }];
        let probe = FakeFdProbe(HashMap::new());
        let sessions = vec![
            session_with("a", Some("/repo")),
            session_with("b", Some("/repo")),
        ];
        let map = correlate_wild_to_sessions(&wild, &probe, &sessions);
        assert!(map.is_empty(), "ambiguous cwd must NOT pick a session");
    }

    #[test]
    fn correlate_no_cwd_no_fd_yields_no_entry() {
        let wild = vec![WildProcess {
            pid: 400,
            command: "claude".into(),
            cwd: None,
        }];
        let probe = FakeFdProbe(HashMap::new());
        let sessions = vec![session_with("a", Some("/repo"))];
        let map = correlate_wild_to_sessions(&wild, &probe, &sessions);
        assert!(map.is_empty());
    }

    #[test]
    fn correlate_ignores_non_jsonl_open_files() {
        // Open files include stuff that is NOT a session jsonl — must not
        // accidentally correlate via a non-jsonl path.
        let wild = vec![WildProcess {
            pid: 500,
            command: "claude".into(),
            cwd: None,
        }];
        let probe = FakeFdProbe(HashMap::from([(
            500,
            vec![
                PathBuf::from("/dev/tty"),
                PathBuf::from("/Users/me/repo/log.txt"),
                PathBuf::from("/Users/me/.claude/projects/p/abc.txt"), // wrong ext
            ],
        )]));
        let sessions = vec![session_with("abc", None)];
        let map = correlate_wild_to_sessions(&wild, &probe, &sessions);
        assert!(map.is_empty());
    }

    #[test]
    fn correlate_multiple_processes_independent() {
        let wild = vec![
            WildProcess {
                pid: 11,
                command: "claude".into(),
                cwd: Some("/r1".into()),
            },
            WildProcess {
                pid: 22,
                command: "claude".into(),
                cwd: Some("/r2".into()),
            },
        ];
        let probe = FakeFdProbe(HashMap::new());
        let sessions = vec![
            session_with("s1", Some("/r1")),
            session_with("s2", Some("/r2")),
        ];
        let map = correlate_wild_to_sessions(&wild, &probe, &sessions);
        assert_eq!(map.get("s1"), Some(&11));
        assert_eq!(map.get("s2"), Some(&22));
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
