//! Starting a process without waiting for it.
//!
//! A child that exits stays in the process table as a zombie until its parent
//! waits for it, and dropping a `std::process::Child` does not wait. clash runs
//! for days and fires `osascript`/`open`/IDE launchers constantly, so every
//! fire-and-forget spawn used to leave one entry behind until the per-user
//! process limit was full and the whole Mac refused to fork.

use std::process::Command;

/// Spawn `cmd` and reap it on a background thread once it exits. Returns the
/// child's pid. Use this for every spawn whose caller does not `wait()` itself.
pub fn spawn_reaped(cmd: &mut Command) -> std::io::Result<u32> {
    let mut child = cmd.spawn()?;
    let pid = child.id();
    let reaper = std::thread::Builder::new()
        .name(format!("reap-{pid}"))
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(e) = reaper {
        // No thread to wait with (the system is out of resources): the child
        // is still running, so report the spawn and accept one zombie.
        tracing::warn!("cannot reap pid {}: {}", pid, e);
    }
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A reaped child disappears from the process table: `kill(pid, 0)` still
    /// succeeds on a zombie and fails with ESRCH only once it has been waited.
    #[test]
    fn a_spawned_child_is_reaped_after_it_exits() {
        let pid = spawn_reaped(&mut Command::new("true")).unwrap() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let gone = unsafe { libc::kill(pid, 0) } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            if gone {
                break;
            }
            assert!(Instant::now() < deadline, "pid {pid} was never reaped");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Every `.spawn()` must either be waited by its caller (the files listed
    /// here) or go through `spawn_reaped`. A new fire-and-forget spawn
    /// elsewhere fails this test instead of leaking a zombie per call.
    #[test]
    fn every_spawn_is_waited_or_reaped() {
        const WAITED: &[&str] = &[
            "src/infrastructure/detached.rs",
            "src/infrastructure/daemon/session.rs",
            "src/infrastructure/git.rs",
            "src/infrastructure/gh.rs",
            "src/infrastructure/secure_post.rs",
            "src/infrastructure/webhook.rs",
            "src/infrastructure/clipboard.rs",
            "src/infrastructure/explorer.rs",
            "src/infrastructure/hermes.rs",
        ];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut offenders = Vec::new();
        for dir in ["src", "gui/src-tauri/src"] {
            let mut stack = vec![root.join(dir)];
            while let Some(d) = stack.pop() {
                for entry in std::fs::read_dir(&d).unwrap().flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    if path.extension().is_none_or(|e| e != "rs") {
                        continue;
                    }
                    let rel = path.strip_prefix(root).unwrap().to_string_lossy();
                    if WAITED.contains(&rel.as_ref()) {
                        continue;
                    }
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (n, line) in text.lines().enumerate() {
                        if line.contains(".spawn()") {
                            offenders.push(format!("{rel}:{}", n + 1));
                        }
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "unwaited spawns (use detached::spawn_reaped): {offenders:?}"
        );
    }
}
