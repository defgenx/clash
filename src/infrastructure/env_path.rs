//! Login-shell PATH adoption for GUI launches.
//!
//! Apps launched from Finder/Dock inherit launchd's minimal
//! `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, which lacks `~/.local/bin`,
//! Homebrew, npm globals, etc. Sessions then fail to spawn (`claude` not
//! found) and any session that did spawn would run with a crippled PATH.
//! The fix is to ask the user's login shell for its PATH once at startup
//! and adopt it for this process — every child (daemon PTY sessions
//! included) inherits the corrected value.

use std::process::Command;

/// Directories probed as a fallback when the login shell can't be queried.
fn fallback_dirs() -> Vec<String> {
    let home = dirs::home_dir()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    vec![
        format!("{home}/.local/bin"),
        format!("{home}/.claude/local"),
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
    ]
}

/// Query the user's login shell for its PATH. Returns `None` if the shell
/// can't be run, hangs, or produces nothing usable.
///
/// Runs interactive + login (`-i -l`) because PATH exports commonly live in
/// `.zshrc`, which non-interactive login shells never source. An interactive
/// shell can block on a misbehaving rc file, so the query is bounded by a
/// timeout and runs on a throwaway thread.
fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    // fish stores PATH as a list; everything else expands "$PATH" directly.
    // fish sources config.fish for non-interactive shells too, so plain
    // login mode suffices there.
    let (flags, cmd): (&[&str], &str) = if shell.ends_with("fish") {
        (&["-l", "-c"], "string join : $PATH")
    } else {
        (&["-i", "-l", "-c"], "printf %s \"$PATH\"")
    };
    let args: Vec<String> = flags
        .iter()
        .map(|s| s.to_string())
        .chain(std::iter::once(cmd.to_string()))
        .collect();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = Command::new(&shell)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output();
        let _ = tx.send(result);
    });
    let out = rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_shell_path_output(&String::from_utf8_lossy(&out.stdout))
}

/// Extract the PATH value from login-shell output. Shell startup files may
/// print banners, so take the last non-empty line that looks like a PATH.
fn parse_shell_path_output(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && l.contains('/'))
        .map(str::to_string)
}

/// Merge `extra` PATH entries into `current`, preserving order and skipping
/// duplicates. Entries from `extra` take precedence (prepended).
fn merge_paths(current: &str, extra: &str) -> String {
    let mut seen = Vec::new();
    for entry in extra.split(':').chain(current.split(':')) {
        if !entry.is_empty() && !seen.iter().any(|e| e == entry) {
            seen.push(entry.to_string());
        }
    }
    seen.join(":")
}

/// Adopt the login shell's PATH for this process. Call once at startup,
/// before anything resolves binaries or spawns children. Falls back to
/// appending well-known bin directories if the shell can't be queried.
///
/// No-op when PATH already contains a home-relative entry — that means we
/// were launched from a real shell and querying the login shell would only
/// add startup latency.
pub fn adopt_login_shell_path() {
    let current = std::env::var("PATH").unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if current.split(':').any(|e| e.starts_with(home.as_ref())) {
            return;
        }
    }
    let merged = match login_shell_path() {
        Some(login) => merge_paths(&current, &login),
        None => merge_paths(&current, &fallback_dirs().join(":")),
    };
    if merged != current {
        tracing::info!("PATH adopted from login shell: {}", merged);
        std::env::set_var("PATH", merged);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_path() {
        assert_eq!(
            parse_shell_path_output("/usr/bin:/bin"),
            Some("/usr/bin:/bin".to_string())
        );
    }

    #[test]
    fn parse_skips_banner_lines() {
        let out = "Welcome!\n\n/Users/x/.local/bin:/usr/bin:/bin\n";
        assert_eq!(
            parse_shell_path_output(out),
            Some("/Users/x/.local/bin:/usr/bin:/bin".to_string())
        );
    }

    #[test]
    fn parse_empty_output() {
        assert_eq!(parse_shell_path_output(""), None);
        assert_eq!(parse_shell_path_output("no path here"), None);
    }

    #[test]
    fn merge_prepends_and_dedupes() {
        assert_eq!(
            merge_paths("/usr/bin:/bin", "/Users/x/.local/bin:/usr/bin"),
            "/Users/x/.local/bin:/usr/bin:/bin"
        );
    }

    #[test]
    fn merge_handles_empty_current() {
        assert_eq!(merge_paths("", "/a:/b"), "/a:/b");
    }
}
