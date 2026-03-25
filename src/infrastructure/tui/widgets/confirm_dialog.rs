use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::centered::centered_rect;
use crate::infrastructure::tui::theme;

/// Render a confirmation dialog (simple y/n).
pub fn render_confirm_dialog(message: &str, frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(50, 20, area);

    frame.render_widget(Clear, popup_area);

    let lines = vec![
        Line::from(""),
        Line::from(Span::raw(message)),
        Line::from(""),
        Line::from(vec![
            Span::styled("  y", theme::help_key_style()),
            Span::raw(" Confirm   "),
            Span::styled("n/Esc", theme::help_key_style()),
            Span::raw(" Cancel"),
        ]),
    ];

    let block = Block::default()
        .title(" Confirm ")
        .title_style(Style::default().fg(theme::CONFIRM_BORDER))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::CONFIRM_BORDER));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center);

    frame.render_widget(paragraph, popup_area);
}
