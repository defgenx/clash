use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::Frame;

use crate::infrastructure::tui::theme;

/// Render a toast notification in the footer area.
pub fn render_toast(message: &str, frame: &mut Frame, area: Rect) {
    let span = Span::styled(message, theme::toast_style());
    let paragraph = ratatui::widgets::Paragraph::new(span);
    frame.render_widget(paragraph, area);
}
