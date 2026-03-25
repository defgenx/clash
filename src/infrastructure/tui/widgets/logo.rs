use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use tui_big_text::{BigText, PixelSize};

use crate::infrastructure::tui::theme;

const TAGLINE: &str = "your agents aren't gonna manage themselves";

/// Render the splash/landing page centered in the given area.
pub fn render_logo(frame: &mut Frame, area: Rect) {
    // Layout: big_text (4) + glow_line (1) + gap (1) + tagline (1) + gap (1) + hints (4) + gap (1) + version (1) = 14
    let content_height = 14u16;
    let top_pad = area.height.saturating_sub(content_height) / 2;

    let layout = Layout::vertical([
        Constraint::Length(top_pad),
        Constraint::Length(4), // big text
        Constraint::Length(1), // glow underline
        Constraint::Length(1), // gap
        Constraint::Length(1), // tagline
        Constraint::Length(1), // gap
        Constraint::Length(4), // hints
        Constraint::Length(1), // gap
        Constraint::Length(1), // version
        Constraint::Min(0),
    ])
    .split(area);

    // Big "clash" text — uses the brighter logo-specific neon color
    let big = BigText::builder()
        .pixel_size(PixelSize::HalfHeight)
        .style(
            Style::default()
                .fg(theme::LOGO_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
        .lines(vec!["clash".into()])
        .alignment(Alignment::Center)
        .build();

    frame.render_widget(big, layout[1]);

    // Neon glow underline — gradient fade from dim edges to bright center sparkle
    let glow = Paragraph::new(glow_line()).alignment(Alignment::Center);
    frame.render_widget(glow, layout[2]);

    // Sparkle + tagline
    let tagline = Paragraph::new(Line::from(vec![
        Span::styled(
            "✦ ",
            Style::default()
                .fg(theme::LOGO_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(TAGLINE, Style::default().fg(theme::TEXT_DIM)),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(tagline, layout[4]);

    // Hint blocks
    let hints: Vec<Line> = vec![
        hint_line("c", "New session", "a", "Attach to session"),
        hint_line("Tab", "Expand agents", "/", "Filter"),
        hint_line(":", "Command mode", "?", "Help"),
        hint_line("A", "Active / all", "q", "Quit"),
    ];

    let hints_widget = Paragraph::new(hints).alignment(Alignment::Center);
    frame.render_widget(hints_widget, layout[6]);

    // Version
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let version_widget = Paragraph::new(Line::from(Span::styled(
        version,
        Style::default().fg(theme::MUTED),
    )))
    .alignment(Alignment::Center);
    frame.render_widget(version_widget, layout[8]);
}

/// Build the neon glow underline with gradient: dim → bright → sparkle → bright → dim
fn glow_line() -> Line<'static> {
    let dim = Style::default().fg(theme::LOGO_GLOW);
    let bright = Style::default().fg(theme::LOGO_PRIMARY);
    let spark = Style::default()
        .fg(theme::LOGO_ACCENT)
        .add_modifier(Modifier::BOLD);

    Line::from(vec![
        Span::styled("──────", dim),
        Span::styled("──────", bright),
        Span::styled(" ✦ ", spark),
        Span::styled("──────", bright),
        Span::styled("──────", dim),
    ])
}

/// Build a hint line with two key-description pairs.
fn hint_line(key1: &str, desc1: &str, key2: &str, desc2: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {:<6}", key1),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<20}", desc1),
            Style::default().fg(theme::TEXT_DIM),
        ),
        Span::styled(
            format!("  {:<6}", key2),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc2.to_string(), Style::default().fg(theme::TEXT_DIM)),
    ])
}
