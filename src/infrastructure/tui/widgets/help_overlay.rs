use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::adapters::views::Keybinding;
use crate::infrastructure::tui::theme;

/// Render a help overlay in the center of the screen.
pub fn render_help_overlay(
    title: &str,
    global_keys: &[Keybinding],
    context_keys: &[Keybinding],
    frame: &mut Frame,
    area: Rect,
) {
    // Center a box at ~60% width, ~70% height
    let popup_area = centered_rect(60, 70, area);

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled("Global", theme::title_style())));
    lines.push(Line::from(""));

    for kb in global_keys {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<12}", kb.key), theme::help_key_style()),
            Span::styled(&kb.description, theme::help_desc_style()),
        ]));
    }

    if !context_keys.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Context", theme::title_style())));
        lines.push(Line::from(""));

        for kb in context_keys {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<12}", kb.key), theme::help_key_style()),
                Span::styled(&kb.description, theme::help_desc_style()),
            ]));
        }
    }

    let block = Block::default()
        .title(format!(" {} — Help (?) ", title))
        .title_style(theme::title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
