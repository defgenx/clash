use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::application::state::InputMode;
use crate::infrastructure::tui::theme;

pub fn render_input_bar(
    mode: &InputMode,
    input: &str,
    cursor_pos: usize,
    frame: &mut Frame,
    area: Rect,
) {
    let (prefix, style) = match mode {
        InputMode::Command => (":", Style::default().fg(theme::COMMAND_COLOR)),
        InputMode::Filter => ("/", Style::default().fg(theme::FILTER_COLOR)),
        InputMode::NewSession => ("New session in: ", Style::default().fg(theme::PROMPT_COLOR)),
        InputMode::NewSessionName => ("Session name: ", Style::default().fg(theme::PROMPT_COLOR)),
        InputMode::NewSessionWorktree => (
            "Start in worktree? (y/n): ",
            Style::default().fg(theme::PROMPT_COLOR),
        ),
        _ => return,
    };

    let line = Line::from(vec![Span::styled(prefix, style), Span::raw(input)]);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, area);

    frame.set_cursor_position((area.x + prefix.len() as u16 + cursor_pos as u16, area.y));
}
