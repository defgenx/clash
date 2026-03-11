use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Standard frame layout: header (1 line), body (fills), footer (1 line).
pub struct FrameLayout {
    pub header: Rect,
    pub body: Rect,
    pub footer: Rect,
}

impl FrameLayout {
    pub fn new(area: Rect) -> Self {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // header
                Constraint::Min(3),   // body
                Constraint::Length(1), // footer
            ])
            .split(area);

        Self {
            header: chunks[0],
            body: chunks[1],
            footer: chunks[2],
        }
    }
}
