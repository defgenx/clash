use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::Frame;

use crate::infrastructure::tui::theme;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Render a spinner with a message.
pub fn render_spinner(message: &str, tick: usize, frame: &mut Frame, area: Rect) {
    let spinner_char = SPINNER_FRAMES[tick % SPINNER_FRAMES.len()];
    let text = format!("{} {}", spinner_char, message);
    let span = Span::styled(text, theme::spinner_style());
    let paragraph = ratatui::widgets::Paragraph::new(span);
    frame.render_widget(paragraph, area);
}
