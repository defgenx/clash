use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::infrastructure::tui::theme;

const LOGO: &[&str] = &[
    r"        .__                .__     ",
    r"   ____ |  | _____    _____|  |__  ",
    r" _/ ___\|  | \__  \  /  ___|  |  \ ",
    r" \  \___|  |__/ __ \_\___ \|   Y  \",
    r"  \___  |____(____  /____  |___|  /",
    r"      \/          \/     \/     \/ ",
];

const TAGLINE: &str = "Claude Stash - TUI for Claude Code Sessions";

const HINTS: &[&str] = &[
    "c  New session     :teams  View teams     ?  Help",
    "A  Show all sessions      /  Filter        q  Quit",
];

/// Render the splash logo centered in the given area.
pub fn render_logo(frame: &mut Frame, area: Rect) {
    // Vertically center: logo (6) + blank (1) + tagline (1) + blank (1) + hints (2) = 11 lines
    let content_height = 11u16;
    let top_pad = area.height.saturating_sub(content_height) / 2;

    let layout = Layout::vertical([
        Constraint::Length(top_pad),
        Constraint::Length(6), // logo
        Constraint::Length(1), // gap
        Constraint::Length(1), // tagline
        Constraint::Length(1), // gap
        Constraint::Length(2), // hints
        Constraint::Min(0),
    ])
    .split(area);

    // Logo
    let logo_lines: Vec<Line> = LOGO
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                *line,
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ))
        })
        .collect();

    let logo_widget = Paragraph::new(logo_lines).alignment(Alignment::Center);
    frame.render_widget(logo_widget, layout[1]);

    // Tagline
    let tagline = Paragraph::new(Line::from(Span::styled(
        TAGLINE,
        Style::default().fg(theme::TEXT_DIM),
    )))
    .alignment(Alignment::Center);
    frame.render_widget(tagline, layout[3]);

    // Hints
    let hint_lines: Vec<Line> = HINTS
        .iter()
        .map(|line| Line::from(Span::styled(*line, Style::default().fg(theme::MUTED))))
        .collect();

    let hints_widget = Paragraph::new(hint_lines).alignment(Alignment::Center);
    frame.render_widget(hints_widget, layout[5]);
}
