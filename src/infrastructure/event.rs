use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};

/// Application events.
#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Tick,
    Resize(u16, u16),
}

/// Read crossterm events with a tick interval.
pub async fn read_event(tick_rate: Duration) -> Option<Event> {
    if event::poll(tick_rate).ok()? {
        match event::read().ok()? {
            CrosstermEvent::Key(key) => Some(Event::Key(key)),
            CrosstermEvent::Resize(w, h) => Some(Event::Resize(w, h)),
            _ => None,
        }
    } else {
        Some(Event::Tick)
    }
}
