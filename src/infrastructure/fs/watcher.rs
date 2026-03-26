use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use notify_debouncer_full::notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, Debouncer, RecommendedCache};

use crate::infrastructure::error::Result;

pub struct FsWatcher {
    _debouncer: Debouncer<notify_debouncer_full::notify::RecommendedWatcher, RecommendedCache>,
}

impl FsWatcher {
    pub fn new(
        paths: &[PathBuf],
        event_tx: tokio::sync::mpsc::UnboundedSender<Vec<PathBuf>>,
        debounce: Duration,
    ) -> Result<Self> {
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
                            let _ = event_tx.send(paths);
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
