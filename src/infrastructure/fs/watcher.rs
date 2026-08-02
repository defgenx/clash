use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify_debouncer_full::notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, Debouncer, RecommendedCache};

use crate::infrastructure::error::Result;

pub struct FsWatcher {
    _debouncer: Debouncer<notify_debouncer_full::notify::RecommendedWatcher, RecommendedCache>,
}

/// Split a batch of changed paths by which root each one belongs to.
///
/// Longest prefix wins, so a root nested inside another (a scratch dir under
/// the Claude data dir, say) is attributed to the more specific one rather than
/// to both. Paths under no root are dropped.
///
/// Pure, so the routing rule is testable without a filesystem — the repo's
/// convention of extracting the decision and keeping the IO wrapper thin.
///
/// Reached through [`RoutedWatcher`], which only the GUI builds.
#[allow(dead_code)]
pub fn route_paths<'a, T: Clone>(
    roots: &'a [(T, PathBuf)],
    paths: &[PathBuf],
) -> Vec<(&'a T, Vec<PathBuf>)> {
    let mut grouped: Vec<(&T, Vec<PathBuf>)> = roots.iter().map(|(t, _)| (t, Vec::new())).collect();
    for path in paths {
        let best = roots
            .iter()
            .enumerate()
            .filter(|(_, (_, root))| path.starts_with(root))
            .max_by_key(|(_, (_, root))| root.components().count());
        if let Some((index, _)) = best {
            grouped[index].1.push(path.clone());
        }
    }
    grouped.retain(|(_, paths)| !paths.is_empty());
    grouped
}

/// Add each root's canonical form alongside the configured one.
///
/// `notify` reports the *resolved* path, so a root reached through a symlink
/// never prefix-matches what the caller configured and every event under it is
/// silently dropped — a scratch dir pointed at a symlink would simply stop
/// auto-refreshing, with nothing in the log to say why. Matching both spellings
/// is cheaper and more forgiving than canonicalizing incoming paths, which fails
/// outright for a file that has just been deleted.
///
/// Both entries carry the same tag, so a duplicate match is harmless.
#[allow(dead_code)] // Reached through RoutedWatcher, which only the GUI builds.
fn with_canonical_roots<T: Clone>(roots: Vec<(T, PathBuf)>) -> Vec<(T, PathBuf)> {
    let mut out = Vec::with_capacity(roots.len() * 2);
    for (tag, path) in roots {
        if let Ok(canonical) = path.canonicalize() {
            if canonical != path {
                out.push((tag.clone(), canonical));
            }
        }
        out.push((tag, path));
    }
    out
}

/// One watcher over several tagged roots, with events routed back per root.
///
/// Replaces the pattern of standing up a separate `FsWatcher` (and a separate
/// debounce) per directory: with four interested subsystems that meant four
/// independent debounce windows to reason about, and adding config as a fifth
/// would have compounded it. One watcher, one debounce, one routing step.
///
/// Consumed by the GUI (the TUI's single watcher already routes inline in its
/// event loop), so the binary's private-`mod` build compiles it as dead.
#[allow(dead_code)]
pub struct RoutedWatcher {
    _watcher: FsWatcher,
}

impl RoutedWatcher {
    /// Watch every root, forwarding `(tag, paths)` batches on `event_tx`.
    ///
    /// Roots that don't exist are skipped by the underlying watcher, so callers
    /// that want a directory watched from first launch must create it first.
    ///
    /// Routing happens on the watcher's own thread rather than in a spawned
    /// task: this is constructed from Tauri's `setup`, which runs on the main
    /// thread with no Tokio runtime entered, so a `tokio::spawn` here aborts the
    /// app at launch ("there is no reactor running").
    #[allow(dead_code)] // GUI-only; see the type-level comment.
    pub fn new<T: Clone + Send + 'static>(
        roots: Vec<(T, PathBuf)>,
        event_tx: tokio::sync::mpsc::UnboundedSender<(T, Vec<PathBuf>)>,
        debounce: Duration,
    ) -> Result<Self> {
        let paths: Vec<PathBuf> = roots.iter().map(|(_, p)| p.clone()).collect();
        let roots = with_canonical_roots(roots);
        let watcher = FsWatcher::spawn(&paths, debounce, move |batch| {
            for (tag, paths) in route_paths(&roots, &batch) {
                // An error means the receiver is gone: the app is shutting down.
                if event_tx.send((tag.clone(), paths)).is_err() {
                    return;
                }
            }
        })?;
        Ok(Self { _watcher: watcher })
    }
}

impl FsWatcher {
    /// Whether a path is a direct child of `dir` — used to tell clash's own
    /// bookkeeping files (an advisory lock, `write_atomic`'s temp file) apart
    /// from real content changes in the same directory.
    pub fn is_child_of(path: &Path, dir: &Path) -> bool {
        path.parent() == Some(dir)
    }

    pub fn new(
        paths: &[PathBuf],
        event_tx: tokio::sync::mpsc::UnboundedSender<Vec<PathBuf>>,
        debounce: Duration,
    ) -> Result<Self> {
        Self::spawn(paths, debounce, move |batch| {
            let _ = event_tx.send(batch);
        })
    }

    /// Watch `paths` and hand each debounced, non-empty batch to `on_batch` on
    /// the watcher's own thread.
    ///
    /// The shared core of [`FsWatcher::new`] and [`RoutedWatcher::new`] — the
    /// only difference between them is what the callback does with a batch, so
    /// neither needs its own thread or its own debouncer.
    fn spawn<F>(paths: &[PathBuf], debounce: Duration, mut on_batch: F) -> Result<Self>
    where
        F: FnMut(Vec<PathBuf>) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let mut debouncer = new_debouncer(debounce, None, tx)?;

        for path in paths {
            if path.exists() {
                debouncer.watch(path, RecursiveMode::Recursive)?;
            }
        }

        std::thread::spawn(move || {
            while let Ok(result) = rx.recv() {
                match result {
                    Ok(events) => {
                        let paths: Vec<PathBuf> =
                            events.iter().flat_map(|e| e.paths.clone()).collect();
                        if !paths.is_empty() {
                            on_batch(paths);
                        }
                    }
                    Err(errors) => {
                        for e in errors {
                            tracing::warn!("FS watch error: {}", e);
                        }
                    }
                }
            }
        });

        Ok(Self {
            _debouncer: debouncer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Root {
        Projects,
        Scratch,
        Config,
    }

    fn roots() -> Vec<(Root, PathBuf)> {
        vec![
            (Root::Projects, PathBuf::from("/home/u/.claude/projects")),
            (
                Root::Scratch,
                PathBuf::from("/home/u/.claude/clash/scratch"),
            ),
            (Root::Config, PathBuf::from("/home/u/.config/clash")),
        ]
    }

    #[test]
    fn routes_each_path_to_its_root() {
        let roots = roots();
        let grouped = route_paths(
            &roots,
            &[
                PathBuf::from("/home/u/.claude/projects/a/s.jsonl"),
                PathBuf::from("/home/u/.config/clash/config.toml"),
                PathBuf::from("/home/u/.claude/projects/b/s.jsonl"),
            ],
        );
        assert_eq!(grouped.len(), 2);
        assert_eq!(*grouped[0].0, Root::Projects);
        assert_eq!(grouped[0].1.len(), 2);
        assert_eq!(*grouped[1].0, Root::Config);
    }

    /// A scratch dir living under the Claude data dir must be attributed to the
    /// scratch root only — otherwise every note edit would also trigger a full
    /// session rescan.
    #[test]
    fn the_longest_matching_root_wins() {
        let roots = vec![
            (Root::Projects, PathBuf::from("/home/u/.claude")),
            (
                Root::Scratch,
                PathBuf::from("/home/u/.claude/clash/scratch"),
            ),
        ];
        let grouped = route_paths(
            &roots,
            &[PathBuf::from("/home/u/.claude/clash/scratch/note.md")],
        );
        assert_eq!(grouped.len(), 1);
        assert_eq!(*grouped[0].0, Root::Scratch);
    }

    #[test]
    fn paths_under_no_root_are_dropped() {
        let roots = roots();
        assert!(route_paths(&roots, &[PathBuf::from("/tmp/elsewhere")]).is_empty());
        assert!(route_paths(&roots, &[]).is_empty());
    }

    /// A root reached through a symlink is reported by `notify` under its
    /// resolved path, so both spellings must route to the same tag — otherwise
    /// every event under it is dropped and the directory just quietly stops
    /// auto-refreshing.
    #[test]
    fn a_symlinked_root_routes_under_its_resolved_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let real = dir.path().join("real");
        let link = dir.path().join("link");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let roots = with_canonical_roots(vec![(Root::Scratch, link.clone())]);
        // Configured spelling…
        assert_eq!(
            route_paths(&roots, &[link.join("note.md")])
                .first()
                .map(|(t, _)| (*t).clone()),
            Some(Root::Scratch)
        );
        // …and the resolved one the watcher actually reports.
        let resolved = real.canonicalize().unwrap();
        assert_eq!(
            route_paths(&roots, &[resolved.join("note.md")])
                .first()
                .map(|(t, _)| (*t).clone()),
            Some(Root::Scratch)
        );
    }

    #[test]
    fn with_canonical_roots_keeps_a_single_entry_when_nothing_resolves() {
        // A path that doesn't exist can't be canonicalized; it must still be
        // watchable (the dir may be created later).
        let roots = with_canonical_roots(vec![(Root::Config, PathBuf::from("/nope/missing"))]);
        assert_eq!(roots.len(), 1);
    }

    #[test]
    fn is_child_of_only_matches_direct_children() {
        let dir = Path::new("/home/u/.config/clash");
        assert!(FsWatcher::is_child_of(&dir.join("config.toml.lock"), dir));
        assert!(!FsWatcher::is_child_of(
            &dir.join("nested").join("f.toml"),
            dir
        ));
        assert!(!FsWatcher::is_child_of(Path::new("/elsewhere/f"), dir));
    }
}
