//! Adopt confirm dialog for wild / external sessions.
//!
//! Two-option overlay (View-only / Takeover) that gates which buttons
//! render against the `AdoptionOptions` snapshot taken at dialog-open
//! time — so a session whose status flipped between Wild→Stashed
//! mid-dialog still shows whatever was valid then.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::centered::centered_rect;
use crate::application::state::AdoptDialog;
use crate::infrastructure::tui::theme;

pub fn render_adopt_dialog(dialog: &AdoptDialog, frame: &mut Frame, area: Rect) {
    let popup_area = centered_rect(60, 30, area);
    frame.render_widget(Clear, popup_area);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("Adopt session "),
            Span::styled(&dialog.display_name, theme::help_key_style()),
        ]),
        Line::from(""),
    ];

    if dialog.options.view_only {
        lines.push(Line::from(vec![
            Span::styled("  v", theme::help_key_style()),
            Span::raw("  View-only — tail the conversation; do not steal the PTY"),
        ]));
    }
    if dialog.options.takeover {
        lines.push(Line::from(vec![
            Span::styled("  t", theme::help_key_style()),
            Span::raw("  Takeover — terminate the wild process and re-spawn under daemon"),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ⚠ Takeover SIGTERMs the running process. In-flight tool calls may not complete cleanly.",
            Style::default().fg(theme::TEXT_DIM),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Esc", theme::help_key_style()),
        Span::raw("  Cancel"),
    ]));

    let block = Block::default()
        .title(" Adopt wild session ")
        .title_style(Style::default().fg(theme::CONFIRM_BORDER))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::CONFIRM_BORDER));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, popup_area);
}
