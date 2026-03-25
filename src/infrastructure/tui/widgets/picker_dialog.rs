use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::centered::centered_rect;
use crate::application::state::PickerDialog;
use crate::infrastructure::tui::theme;

/// Render a picker dialog overlay.
pub fn render_picker_dialog(picker: &PickerDialog, frame: &mut Frame, area: Rect) {
    // Scale height with item count: min 20%, max 60%
    let item_lines = picker.items.len() as u16;
    // +4 for border top/bottom, title, footer
    let needed = item_lines + 4;
    let percent_y = ((needed * 100) / area.height.max(1)).clamp(20, 60);
    let popup_area = centered_rect(50, percent_y, area);

    frame.render_widget(Clear, popup_area);

    let mut lines = Vec::new();
    for (i, item) in picker.items.iter().enumerate() {
        let is_selected = i == picker.selected;
        let prefix = if is_selected { "> " } else { "  " };

        let label_style = if is_selected {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT_DIM)
        };

        let mut spans = vec![
            Span::styled(prefix, label_style),
            Span::styled(&item.label, label_style),
        ];
        if !item.description.is_empty() {
            spans.push(Span::styled(
                format!("  {}", item.description),
                Style::default().fg(theme::MUTED),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Footer
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" j/k", theme::help_key_style()),
        Span::raw(" Navigate  "),
        Span::styled("Enter", theme::help_key_style()),
        Span::raw(" Select  "),
        Span::styled("Esc", theme::help_key_style()),
        Span::raw(" Cancel"),
    ]));

    let block = Block::default()
        .title(format!(" {} ", picker.title))
        .title_style(Style::default().fg(theme::ACCENT))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup_area);
}
