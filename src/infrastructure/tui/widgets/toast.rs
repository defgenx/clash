use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::Frame;

use crate::infrastructure::tui::theme;

/// Toast text color — muted gold, matching STATUS_WAITING / COMMAND_COLOR.
const TOAST_FG: ratatui::style::Color = ratatui::style::Color::Rgb(210, 190, 120);

/// Render a toast notification with static gold text in the footer area.
pub fn render_toast(message: &str, frame: &mut Frame, area: Rect) {
    let span = Span::styled(
        message,
        Style::default()
            .fg(TOAST_FG)
            .bg(theme::FOOTER_BG)
            .add_modifier(Modifier::BOLD),
    );
    let paragraph =
        ratatui::widgets::Paragraph::new(span).alignment(ratatui::layout::Alignment::Right);
    frame.render_widget(paragraph, area);
}
