use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::application::state::InputMode;

pub fn render_input_bar(
    mode: &InputMode,
    input: &str,
    cursor_pos: usize,
    frame: &mut Frame,
    area: Rect,
) {
    let (prefix, style) = match mode {
        InputMode::Command => (":", Style::default().fg(Color::Yellow)),
        InputMode::Filter => ("/", Style::default().fg(Color::Green)),
        _ => return,
    };

    let line = Line::from(vec![Span::styled(prefix, style), Span::raw(input)]);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);

    frame.set_cursor_position((area.x + prefix.len() as u16 + cursor_pos as u16, area.y));
}
