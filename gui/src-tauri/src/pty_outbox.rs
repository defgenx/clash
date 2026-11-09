//! Terminal output on its way to the webview, pulled by the frontend.
//!
//! PTY output used to reach the webview as one tauri event per chunk. Tauri
//! delivers an event by evaluating `(function () { … payload … })()` in the
//! page, and WebKit keeps every such script's source alive — payload included
//! — so the web view grew by roughly the size of all terminal output it ever
//! received (measured: 20k 6KB events, 12 MB → 107 MB; a long-lived window
//! held 60k of them). An `invoke` response comes back through the IPC custom
//! protocol and evaluates nothing, so the frontend long-polls `pty_drain`
//! instead. Full rationale: `docs/gui.md` → *Terminal output*.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;

/// Bytes kept for a frontend that stopped draining (a suspended or reloading
/// webview). Past it the oldest output goes; exits are always kept.
const MAX_QUEUED_BYTES: usize = 32 * 1024 * 1024;

/// One item for the frontend, in the order the daemon produced them.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PtyItem {
    /// Base64-encoded raw terminal bytes.
    Output { session_id: String, data: String },
    Exited {
        session_id: String,
        exit_code: Option<i32>,
    },
}

#[derive(Debug)]
enum Queued {
    Output {
        session_id: String,
        bytes: Vec<u8>,
    },
    Exited {
        session_id: String,
        exit_code: Option<i32>,
    },
}

/// The ordered queue between the attach forwarders and `pty_drain`.
#[derive(Default)]
pub struct PtyOutbox {
    /// The items, and the output bytes among them (for the cap).
    queue: Mutex<(VecDeque<Queued>, usize)>,
    ready: tokio::sync::Notify,
}

impl PtyOutbox {
    pub fn push_output(&self, session_id: &str, bytes: &[u8]) {
        let dropped = {
            let mut guard = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            let (q, total) = &mut *guard;
            match q.back_mut() {
                // Consecutive chunks of one session become one write.
                Some(Queued::Output {
                    session_id: s,
                    bytes: b,
                }) if s == session_id => b.extend_from_slice(bytes),
                _ => q.push_back(Queued::Output {
                    session_id: session_id.to_string(),
                    bytes: bytes.to_vec(),
                }),
            }
            *total += bytes.len();
            trim_oldest_output(q, total, MAX_QUEUED_BYTES)
        };
        if dropped > 0 {
            tracing::warn!(
                "pty outbox: frontend not draining, dropped {} bytes of old output",
                dropped
            );
        }
        self.ready.notify_one();
    }

    pub fn push_exited(&self, session_id: &str, exit_code: Option<i32>) {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .0
            .push_back(Queued::Exited {
                session_id: session_id.to_string(),
                exit_code,
            });
        self.ready.notify_one();
    }

    /// Everything queued, now; empty when nothing is.
    pub fn take(&self) -> Vec<PtyItem> {
        let items: Vec<Queued> = {
            let mut guard = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            guard.1 = 0;
            guard.0.drain(..).collect()
        };
        items
            .into_iter()
            .map(|q| match q {
                Queued::Output { session_id, bytes } => PtyItem::Output {
                    session_id,
                    data: base64::engine::general_purpose::STANDARD.encode(bytes),
                },
                Queued::Exited {
                    session_id,
                    exit_code,
                } => PtyItem::Exited {
                    session_id,
                    exit_code,
                },
            })
            .collect()
    }

    /// Wait until something is queued (at most `wait`), then take it all.
    /// `notify_one` stores a permit when nobody waits, so a push between
    /// `take` and `notified` is not lost.
    pub async fn drain(&self, wait: Duration) -> Vec<PtyItem> {
        let items = self.take();
        if !items.is_empty() {
            return items;
        }
        let _ = tokio::time::timeout(wait, self.ready.notified()).await;
        self.take()
    }
}

/// Drop whole output items from the front until `total` fits `max`; returns
/// the bytes dropped. Exits stay: a lost exit leaves a tab that never closes.
fn trim_oldest_output(q: &mut VecDeque<Queued>, total: &mut usize, max: usize) -> usize {
    let mut dropped = 0;
    while *total > max {
        let Some(i) = q.iter().position(|e| matches!(e, Queued::Output { .. })) else {
            break;
        };
        // The newest output is what the frontend needs most; never drop it.
        if i + 1 == q.len() {
            break;
        }
        if let Some(Queued::Output { bytes, .. }) = q.remove(i) {
            *total -= bytes.len();
            dropped += bytes.len();
        }
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(session_id: &str, s: &str) -> PtyItem {
        PtyItem::Output {
            session_id: session_id.into(),
            data: base64::engine::general_purpose::STANDARD.encode(s),
        }
    }

    #[test]
    fn chunks_of_one_session_merge_and_order_is_kept_across_sessions() {
        let o = PtyOutbox::default();
        o.push_output("a", b"he");
        o.push_output("a", b"llo");
        o.push_output("b", b"x");
        o.push_output("a", b"!");
        o.push_exited("a", Some(0));
        assert_eq!(
            o.take(),
            vec![
                out("a", "hello"),
                out("b", "x"),
                out("a", "!"),
                PtyItem::Exited {
                    session_id: "a".into(),
                    exit_code: Some(0)
                },
            ]
        );
        assert!(o.take().is_empty());
    }

    #[test]
    fn an_exit_never_merges_with_output_after_it() {
        let o = PtyOutbox::default();
        o.push_exited("a", None);
        o.push_output("a", b"late");
        assert_eq!(o.take().len(), 2);
    }

    #[test]
    fn a_stalled_frontend_loses_the_oldest_output_but_no_exit() {
        let mut q = VecDeque::from([
            Queued::Output {
                session_id: "a".into(),
                bytes: vec![0; 10],
            },
            Queued::Exited {
                session_id: "a".into(),
                exit_code: None,
            },
            Queued::Output {
                session_id: "b".into(),
                bytes: vec![0; 10],
            },
            Queued::Output {
                session_id: "c".into(),
                bytes: vec![0; 10],
            },
        ]);
        let mut total = 30;
        assert_eq!(trim_oldest_output(&mut q, &mut total, 15), 20);
        assert_eq!(total, 10);
        assert!(matches!(q[0], Queued::Exited { .. }));
        assert!(matches!(&q[1], Queued::Output { session_id, .. } if session_id == "c"));
        // The newest output stays even when it alone exceeds the cap.
        assert_eq!(trim_oldest_output(&mut q, &mut total, 1), 0);
    }

    #[test]
    fn items_serialize_with_a_kind_tag() {
        let v = serde_json::to_value(out("a", "x")).unwrap();
        assert_eq!(v["kind"], "output");
        assert_eq!(v["session_id"], "a");
        let e = serde_json::to_value(PtyItem::Exited {
            session_id: "a".into(),
            exit_code: Some(1),
        })
        .unwrap();
        assert_eq!(e["kind"], "exited");
        assert_eq!(e["exit_code"], 1);
    }

    #[tokio::test]
    async fn drain_waits_for_a_push_and_returns_at_the_deadline_otherwise() {
        let o = std::sync::Arc::new(PtyOutbox::default());
        assert!(o.drain(Duration::from_millis(20)).await.is_empty());
        let o2 = o.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            o2.push_output("a", b"hi");
        });
        let started = std::time::Instant::now();
        let got = o.drain(Duration::from_secs(5)).await;
        assert_eq!(got, vec![out("a", "hi")]);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
