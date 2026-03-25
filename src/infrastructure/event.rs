//! Async event multiplexer — combines terminal, daemon, and tick events.
//!
//! Uses crossterm's `EventStream` instead of blocking `poll()`/`read()`,
//! allowing `tokio::select!` to process terminal input and daemon output
//! concurrently without blocking or starvation.
//!
//! Tick only fires after `tick_rate` of inactivity (no real events pending),
//! matching the original "tick = idle" semantics from the blocking poll() API.

use std::time::Duration;

use crossterm::event::{
    Event as CrosstermEvent, EventStream, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind,
};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::time::Sleep;

/// Application events from all sources.
#[derive(Debug)]
pub enum Event {
    /// Terminal key press or repeat.
    Key(KeyEvent),
    /// Periodic tick (fires after `tick_rate` of inactivity).
    Tick,
    /// Terminal window resized.
    Resize(u16, u16),
    /// Daemon sent output bytes (currently discarded in TUI mode;
    /// attach mode reads daemon output via its own loop).
    DaemonOutput,
    /// Daemon reports a session exited.
    DaemonExited { session_id: String },
    /// Mouse event (scroll, click, etc.).
    Mouse(MouseEvent),
    /// Self-update progress changed.
    UpdateProgress(crate::application::state::UpdatePhase),
}

/// Multiplexes crossterm terminal events, daemon events, and a tick timer.
pub struct EventLoop {
    crossterm: EventStream,
    tick_rate: Duration,
    daemon_rx: Option<mpsc::UnboundedReceiver<crate::infrastructure::daemon::protocol::Event>>,
    update_rx: Option<mpsc::UnboundedReceiver<crate::application::state::UpdatePhase>>,
    tick_sleep: std::pin::Pin<Box<Sleep>>,
}

impl EventLoop {
    pub fn new(tick_rate: Duration) -> Self {
        Self {
            crossterm: EventStream::new(),
            tick_rate,
            daemon_rx: None,
            update_rx: None,
            tick_sleep: Box::pin(tokio::time::sleep(tick_rate)),
        }
    }

    /// Attach the daemon event receiver. Call when daemon connects.
    pub fn set_daemon_rx(
        &mut self,
        rx: mpsc::UnboundedReceiver<crate::infrastructure::daemon::protocol::Event>,
    ) {
        self.daemon_rx = Some(rx);
    }

    /// Attach the self-update progress receiver.
    pub fn set_update_rx(
        &mut self,
        rx: mpsc::UnboundedReceiver<crate::application::state::UpdatePhase>,
    ) {
        self.update_rx = Some(rx);
    }

    /// Take the daemon event receiver back (used when suspending the event loop for attach).
    pub fn take_daemon_rx(
        &mut self,
    ) -> Option<mpsc::UnboundedReceiver<crate::infrastructure::daemon::protocol::Event>> {
        self.daemon_rx.take()
    }

    fn reset_tick(&mut self) {
        self.tick_sleep
            .as_mut()
            .reset(tokio::time::Instant::now() + self.tick_rate);
    }

    /// Wait for the next event from any source.
    pub async fn next(&mut self) -> Option<Event> {
        loop {
            tokio::select! {
                biased;

                // Terminal events
                maybe_event = self.crossterm.next() => {
                    match maybe_event {
                        Some(Ok(CrosstermEvent::Key(key)))
                            if key.kind == KeyEventKind::Press
                                || key.kind == KeyEventKind::Repeat =>
                        {
                            self.reset_tick();
                            return Some(Event::Key(key));
                        }
                        Some(Ok(CrosstermEvent::Key(_))) => continue,
                        Some(Ok(CrosstermEvent::Mouse(mouse))) => {
                            match mouse.kind {
                                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                                    self.reset_tick();
                                    return Some(Event::Mouse(mouse));
                                }
                                _ => continue,
                            }
                        }
                        Some(Ok(CrosstermEvent::Resize(w, h))) => {
                            self.reset_tick();
                            return Some(Event::Resize(w, h));
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(_)) => return None,
                        None => return None,
                    }
                }

                // Daemon events
                Some(daemon_event) = async {
                    match self.daemon_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.reset_tick();
                    match daemon_event {
                        crate::infrastructure::daemon::protocol::Event::Output { .. } => {
                            // Drain any queued output events to avoid backpressure
                            if let Some(ref mut rx) = self.daemon_rx {
                                while let Ok(extra) = rx.try_recv() {
                                    if matches!(extra, crate::infrastructure::daemon::protocol::Event::Output { .. }) {
                                        // discard — TUI mode doesn't render daemon output
                                    }
                                }
                            }
                            return Some(Event::DaemonOutput);
                        }
                        crate::infrastructure::daemon::protocol::Event::Exited { session_id, .. } => {
                            return Some(Event::DaemonExited { session_id });
                        }
                        _ => continue,
                    }
                }

                // Self-update progress
                Some(phase) = async {
                    match self.update_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.reset_tick();
                    return Some(Event::UpdateProgress(phase));
                }

                // Tick
                _ = self.tick_sleep.as_mut() => {
                    self.reset_tick();
                    return Some(Event::Tick);
                }
            }
        }
    }
}
