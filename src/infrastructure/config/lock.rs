//! An advisory lock around the whole config read-modify-write.
//!
//! Multiple clash instances run at once by design (one daemon socket per pid),
//! so two of them can edit settings concurrently. `write_atomic` makes each
//! write *whole* but not *serialized*: both instances read version N, both
//! merge their own key, and the later write's base snapshot predates the
//! earlier one — so a key is silently lost. Merging changed keys only narrows
//! that window; it does not close it (plan Issue 5 / D5).
//!
//! This closes it: `config.toml.lock` created `O_EXCL`, with a short backoff
//! and a stale age-out so a crashed process cannot wedge config editing
//! forever. Released on `Drop`, including on panic.
//!
//! Deliberately *advisory* and best-effort: if the lock cannot be taken within
//! the budget we age it out and proceed rather than refusing to save. A lost
//! setting is bad; an unsaveable config is worse.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long to keep trying before treating the holder as dead.
const ACQUIRE_TIMEOUT: Duration = Duration::from_millis(2_000);
/// Delay between attempts.
const BACKOFF: Duration = Duration::from_millis(20);
/// A lock file older than this belonged to a process that died holding it.
/// Generous relative to a read-modify-write (single-digit milliseconds) so a
/// merely slow machine is never mistaken for a crashed one.
const STALE_AFTER: Duration = Duration::from_secs(30);

/// Held for the duration of a config read-modify-write. Removes the lock file
/// when dropped.
pub struct ConfigLock {
    path: PathBuf,
}

impl ConfigLock {
    /// Take the lock for `config_path`, waiting for a current holder.
    ///
    /// Never fails: on timeout the lock is aged out and taken anyway, which is
    /// reported through the returned flag so the caller can log it.
    pub fn acquire(config_path: &Path) -> (Self, bool) {
        let path = lock_path(config_path);
        let deadline = SystemTime::now() + ACQUIRE_TIMEOUT;
        let mut forced = false;

        loop {
            if try_create(&path) {
                break;
            }
            if is_stale(&path) {
                // The holder died. Clearing it races with any other waiter
                // doing the same, which is harmless: whoever wins the next
                // `try_create` holds the lock.
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if SystemTime::now() >= deadline {
                let _ = std::fs::remove_file(&path);
                forced = try_create(&path);
                break;
            }
            std::thread::sleep(BACKOFF);
        }

        (Self { path }, forced)
    }
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `<config>.lock`, beside the config file so both share a filesystem (and so
/// the `O_EXCL` create is meaningful — it is not across NFS, hence "advisory").
fn lock_path(config_path: &Path) -> PathBuf {
    let mut name = config_path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    config_path.with_file_name(name)
}

/// `O_EXCL` create: succeeds for exactly one caller.
fn try_create(path: &Path) -> bool {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            // The pid makes a wedged lock diagnosable by hand.
            use std::io::Write;
            let _ = write!(file, "{}", std::process::id());
            true
        }
        Err(_) => false,
    }
}

fn is_stale(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        // Vanished between our create attempt and now — not stale, just gone.
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age > STALE_AFTER)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn acquire_creates_and_drop_removes() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config.toml");
        let lock_file = dir.path().join("config.toml.lock");
        {
            let (_lock, forced) = ConfigLock::acquire(&config);
            assert!(!forced);
            assert!(lock_file.exists());
        }
        assert!(!lock_file.exists(), "the lock must be released on drop");
    }

    #[test]
    fn lock_is_reentrant_across_sequential_acquires() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config.toml");
        for _ in 0..3 {
            let (_lock, forced) = ConfigLock::acquire(&config);
            assert!(!forced);
        }
    }

    #[test]
    fn a_stale_lock_is_aged_out() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config.toml");
        let lock_file = lock_path(&config);
        std::fs::write(&lock_file, "99999").unwrap();
        // Backdate past the stale threshold.
        let old = SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
        filetime_set(&lock_file, old);

        let (_lock, forced) = ConfigLock::acquire(&config);
        // Aged out cleanly, so this is a normal acquire rather than a forced one.
        assert!(!forced);
        assert!(lock_file.exists());
    }

    #[test]
    fn a_live_lock_is_forced_after_the_timeout() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config.toml");
        let lock_file = lock_path(&config);
        // A fresh lock nobody will release: not stale, so acquire has to wait
        // out ACQUIRE_TIMEOUT and then break it. This is the "unsaveable config
        // is worse than a lost setting" path.
        std::fs::write(&lock_file, "99999").unwrap();

        let started = std::time::Instant::now();
        let (_lock, forced) = ConfigLock::acquire(&config);
        assert!(forced, "a live lock must be forced, not silently skipped");
        assert!(
            started.elapsed() >= ACQUIRE_TIMEOUT,
            "must wait the full budget before breaking a live lock"
        );
    }

    /// Two threads hammering the same lock must never both hold it. Each
    /// increments a counter while holding, asserting it was zero on entry.
    #[test]
    fn the_lock_actually_excludes() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config.toml");
        let inside = Arc::new(AtomicUsize::new(0));
        let violations = Arc::new(AtomicUsize::new(0));

        let threads: Vec<_> = (0..4)
            .map(|_| {
                let config = config.clone();
                let inside = Arc::clone(&inside);
                let violations = Arc::clone(&violations);
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        let (_lock, _) = ConfigLock::acquire(&config);
                        if inside.fetch_add(1, Ordering::SeqCst) != 0 {
                            violations.fetch_add(1, Ordering::SeqCst);
                        }
                        std::thread::yield_now();
                        inside.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(violations.load(Ordering::SeqCst), 0);
    }

    /// Set a file's mtime without pulling in a new dependency.
    fn filetime_set(path: &Path, when: SystemTime) {
        let secs = when
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let times = [
            libc::timeval {
                tv_sec: secs,
                tv_usec: 0,
            },
            libc::timeval {
                tv_sec: secs,
                tv_usec: 0,
            },
        ];
        let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
        // SAFETY: a valid NUL-terminated path and a two-element timeval array,
        // which is exactly what utimes(2) expects.
        let rc = unsafe { libc::utimes(c_path.as_ptr(), times.as_ptr()) };
        assert_eq!(rc, 0, "utimes failed");
    }
}
