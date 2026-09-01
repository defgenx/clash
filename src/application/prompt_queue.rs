//! Queued follow-up prompts: type the next instruction while the agent is
//! still working, and clash delivers it the moment that session is idle.
//!
//! Nothing else can offer this, because nothing else watches session status —
//! which is also why the whole decision of *when* a prompt is due lives here,
//! pure and tested, instead of at the two refresh sites that act on it.
//!
//! Two properties are load-bearing:
//!
//! **A queued prompt must never answer a question.** `Prompting` means Claude
//! is holding a tool-approval dialog where Enter accepts the highlighted
//! option, so a prompt delivered there would approve something nobody read.
//! Only `Waiting` — the free-form input prompt — is a delivery point, and only
//! when the *previous* refresh also said `Waiting`: the daemon's screen
//! detector falls back to `waiting` after eight silent seconds, so a single
//! sample of it can land mid-turn. Requiring two consecutive samples costs one
//! refresh tick (~2s) and separates delivery from every `Prompting` flip.
//!
//! **The text is data, not keystrokes.** It goes to the PTY wrapped in a
//! bracketed paste so its newlines stay newlines instead of submitting each
//! line as its own message, followed by exactly one carriage return. Control
//! bytes are stripped: an `ESC[201~` inside the text would end the paste early
//! and turn the remainder into keypresses.

use std::collections::{HashMap, VecDeque};

use crate::domain::entities::{Session, SessionSource, SessionStatus};

/// Per-session cap. A queue this deep is a script, not a follow-up, and an
/// unbounded one would let a stuck session accumulate forever.
pub const MAX_QUEUED: usize = 20;

/// Why an enqueue was refused, so the caller can say so rather than failing
/// silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueError {
    /// Nothing left after stripping control bytes and trimming.
    Empty,
    /// `MAX_QUEUED` already pending for this session.
    Full,
}

/// Pending follow-ups per session, oldest first. In-memory and per instance:
/// each clash owns the sessions its own daemon holds the PTYs for, so a queue
/// belongs to the same process as the PTY it will be written to.
#[derive(Debug, Default, Clone)]
pub struct PromptQueue {
    by_session: HashMap<String, VecDeque<String>>,
}

impl PromptQueue {
    /// Append a follow-up. The text is sanitized here rather than at send
    /// time, so what the UI lists is exactly what will be delivered.
    pub fn enqueue(&mut self, session_id: &str, text: &str) -> Result<(), EnqueueError> {
        let clean = sanitize(text);
        if clean.is_empty() {
            return Err(EnqueueError::Empty);
        }
        let slot = self.by_session.entry(session_id.to_string()).or_default();
        if slot.len() >= MAX_QUEUED {
            return Err(EnqueueError::Full);
        }
        slot.push_back(clean);
        Ok(())
    }

    /// What is pending for one session, oldest first.
    pub fn pending(&self, session_id: &str) -> &[String] {
        self.by_session
            .get(session_id)
            .map(|q| {
                // A VecDeque built only by push_back is contiguous, so the
                // front slice is the whole queue; the fallback keeps this
                // honest if that ever stops being true.
                let (a, b) = q.as_slices();
                if b.is_empty() {
                    a
                } else {
                    &[]
                }
            })
            .unwrap_or(&[])
    }

    /// How many follow-ups one session has waiting (the row indicator).
    pub fn count(&self, session_id: &str) -> usize {
        self.by_session.get(session_id).map_or(0, VecDeque::len)
    }

    /// Drop one entry by position. `false` when the index is stale — the UI
    /// lists a snapshot, and a delivery can land between listing and clicking.
    pub fn remove_at(&mut self, session_id: &str, index: usize) -> bool {
        let Some(q) = self.by_session.get_mut(session_id) else {
            return false;
        };
        let removed = q.remove(index).is_some();
        if q.is_empty() {
            self.by_session.remove(session_id);
        }
        removed
    }

    /// Drop everything queued for one session; returns how many were dropped.
    pub fn clear(&mut self, session_id: &str) -> usize {
        self.by_session.remove(session_id).map_or(0, |q| q.len())
    }

    /// Pop the next follow-up for delivery.
    pub fn take_next(&mut self, session_id: &str) -> Option<String> {
        let q = self.by_session.get_mut(session_id)?;
        let next = q.pop_front();
        if q.is_empty() {
            self.by_session.remove(session_id);
        }
        next
    }

    /// Every pending follow-up, keyed by session id. One call for the whole
    /// picture: the GUI paints a per-row count on every refresh tick and would
    /// otherwise ask once per session.
    ///
    /// GUI-only — the TUI reads counts straight off `AppState.prompt_queue` —
    /// and the binary compiles this module through a private `mod`, so it
    /// carries the same allowance as `application::workflow`.
    #[allow(dead_code)]
    pub fn snapshot(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.by_session
            .iter()
            .map(|(id, q)| (id.clone(), q.iter().cloned().collect()))
            .collect()
    }

    /// Sessions with a follow-up that is due now, sorted for determinism.
    /// `prev_status` answers "what did the previous refresh say about this
    /// id?" — the TUI holds the old session list, the GUI a status map, and
    /// neither shape needs to leak in here.
    pub fn due<F>(&self, prev_status: F, current: &[Session]) -> Vec<String>
    where
        F: Fn(&str) -> Option<SessionStatus>,
    {
        let mut ids: Vec<String> = current
            .iter()
            .filter(|s| self.count(&s.id) > 0 && is_ready(prev_status(&s.id), s))
            .map(|s| s.id.clone())
            .collect();
        ids.sort();
        ids
    }
}

/// Whether this session can take a queued prompt right now.
///
/// Deliberately narrow: a live PTY this clash owns, sitting at the free-form
/// input prompt, and already sitting there at the previous refresh.
pub fn is_ready(prev: Option<SessionStatus>, session: &Session) -> bool {
    session.is_running
        && matches!(session.source, SessionSource::Daemon)
        && session.status == SessionStatus::Waiting
        && prev == Some(SessionStatus::Waiting)
}

/// Strip what must never reach a PTY as-is and normalize line endings.
/// Newlines and tabs survive (a follow-up is prose, sometimes with a list);
/// every other control byte — `ESC` above all — does not.
fn sanitize(text: &str) -> String {
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    let stripped: String = unified
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect();
    stripped.trim().to_string()
}

/// The bytes that deliver one follow-up: a bracketed paste, then Enter.
///
/// Without the paste wrapper every newline in a multi-line prompt would submit
/// the line before it, turning one instruction into several half-instructions.
pub fn pty_paste(text: &str) -> Vec<u8> {
    let body = sanitize(text);
    let mut out = Vec::with_capacity(body.len() + 14);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out.push(b'\r');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, status: SessionStatus) -> Session {
        Session {
            id: id.to_string(),
            is_running: true,
            status,
            source: SessionSource::Daemon,
            ..Default::default()
        }
    }

    #[test]
    fn enqueue_sanitizes_and_caps() {
        let mut q = PromptQueue::default();
        assert_eq!(q.enqueue("s", "  keep going  "), Ok(()));
        assert_eq!(q.pending("s"), ["keep going"]);
        assert_eq!(q.enqueue("s", "   "), Err(EnqueueError::Empty));
        assert_eq!(q.enqueue("s", "\u{1b}"), Err(EnqueueError::Empty));
        // The escape byte is gone before the text is ever stored, so what the
        // UI lists is already safe to write to a PTY.
        q.enqueue("s", "\x1b[201~ done").unwrap();
        assert_eq!(q.pending("s")[1], "[201~ done");
        for i in 0..MAX_QUEUED - 2 {
            assert_eq!(q.enqueue("s", &format!("p{i}")), Ok(()));
        }
        assert_eq!(q.count("s"), MAX_QUEUED);
        assert_eq!(q.enqueue("s", "one too many"), Err(EnqueueError::Full));
    }

    #[test]
    fn fifo_order_and_removal() {
        let mut q = PromptQueue::default();
        q.enqueue("s", "first").unwrap();
        q.enqueue("s", "second").unwrap();
        q.enqueue("s", "third").unwrap();
        assert_eq!(q.pending("s"), ["first", "second", "third"]);
        assert!(q.remove_at("s", 1));
        assert_eq!(q.pending("s"), ["first", "third"]);
        assert!(!q.remove_at("s", 9), "a stale index must not panic");
        assert_eq!(q.take_next("s").as_deref(), Some("first"));
        assert_eq!(q.clear("s"), 1);
        assert_eq!(q.count("s"), 0);
        assert_eq!(q.take_next("s"), None);
        assert!(!q.remove_at("nobody", 0));
        assert_eq!(q.clear("nobody"), 0);
    }

    #[test]
    fn snapshot_holds_every_queue_in_order() {
        let mut q = PromptQueue::default();
        q.enqueue("s2", "b").unwrap();
        q.enqueue("s1", "a1").unwrap();
        q.enqueue("s1", "a2").unwrap();
        let snap = q.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap["s1"], ["a1", "a2"]);
        assert_eq!(snap["s2"], ["b"]);
        // Emptying a session drops its key rather than leaving an empty list
        // the UI would have to special-case.
        q.clear("s2");
        assert!(!q.snapshot().contains_key("s2"));
    }

    #[test]
    fn a_queued_prompt_never_answers_a_permission_dialog() {
        // The whole safety case in one assertion: Prompting is Enter-sensitive,
        // so it is never a delivery point no matter what came before it.
        for prev in [
            None,
            Some(SessionStatus::Waiting),
            Some(SessionStatus::Running),
            Some(SessionStatus::Prompting),
        ] {
            assert!(!is_ready(prev, &session("s", SessionStatus::Prompting)));
        }
    }

    #[test]
    fn delivery_needs_two_consecutive_waiting_samples() {
        // The daemon's screen detector reports `waiting` after eight silent
        // seconds even mid-turn, so one sample is not evidence of an idle
        // agent. The turn-end transition therefore delivers on the tick after
        // the one that first saw it.
        let s = session("s", SessionStatus::Waiting);
        assert!(!is_ready(Some(SessionStatus::Thinking), &s));
        assert!(!is_ready(None, &s));
        assert!(is_ready(Some(SessionStatus::Waiting), &s));
    }

    #[test]
    fn delivery_needs_a_live_pty_this_instance_owns() {
        let mut dead = session("s", SessionStatus::Waiting);
        dead.is_running = false;
        assert!(!is_ready(Some(SessionStatus::Waiting), &dead));
        for source in [
            SessionSource::Wild,
            SessionSource::External,
            SessionSource::Unknown,
        ] {
            let mut foreign = session("s", SessionStatus::Waiting);
            foreign.source = source;
            assert!(
                !is_ready(Some(SessionStatus::Waiting), &foreign),
                "{source:?} has no daemon PTY to write to"
            );
        }
    }

    #[test]
    fn due_lists_only_queued_ready_sessions_sorted() {
        let mut q = PromptQueue::default();
        q.enqueue("b-ready", "x").unwrap();
        q.enqueue("a-ready", "x").unwrap();
        q.enqueue("busy", "x").unwrap();
        let current = vec![
            session("b-ready", SessionStatus::Waiting),
            session("a-ready", SessionStatus::Waiting),
            session("busy", SessionStatus::Thinking),
            // Ready, but nothing queued for it.
            session("empty", SessionStatus::Waiting),
        ];
        let prev = |_: &str| Some(SessionStatus::Waiting);
        assert_eq!(q.due(prev, &current), ["a-ready", "b-ready"]);
    }

    #[test]
    fn paste_keeps_newlines_out_of_the_submit_path() {
        let bytes = pty_paste("line one\nline two");
        let s = String::from_utf8(bytes).unwrap();
        assert_eq!(s, "\x1b[200~line one\nline two\x1b[201~\r");
        // Exactly one submit, at the very end.
        assert_eq!(s.matches('\r').count(), 1);
        assert!(s.ends_with('\r'));
    }

    #[test]
    fn paste_cannot_be_escaped_out_of() {
        // A text carrying the paste terminator would end the paste early and
        // have the rest read as keystrokes.
        let s = String::from_utf8(pty_paste("evil\x1b[201~\rrm -rf /")).unwrap();
        assert_eq!(s.matches("\x1b[201~").count(), 1);
        assert_eq!(s.matches('\r').count(), 1);
        assert!(s.starts_with("\x1b[200~evil[201~"));
    }

    #[test]
    fn paste_normalizes_carriage_returns_in_the_body() {
        let s = String::from_utf8(pty_paste("a\r\nb\rc")).unwrap();
        assert_eq!(s, "\x1b[200~a\nb\nc\x1b[201~\r");
    }
}
