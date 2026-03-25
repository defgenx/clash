use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::application::state::UpdatePhase;
use crate::infrastructure::tui::theme;

/// All phases in display order.
const PHASE_LABELS: &[&str] = &["Checking", "Downloading", "Extracting", "Installing"];

/// Render the self-update progress overlay.
pub fn render_update_overlay(phase: &UpdatePhase, tick: usize, frame: &mut Frame, area: Rect) {
    let popup = centered_fixed(42, 12, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Updating clash ")
        .title_alignment(Alignment::Center)
        .title_style(
            Style::default()
                .fg(theme::LOGO_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top padding
            Constraint::Length(4), // phase list
            Constraint::Length(1), // gap
            Constraint::Length(1), // progress bar
            Constraint::Length(1), // gap
            Constraint::Length(1), // status message
            Constraint::Min(0),
        ])
        .split(inner);

    // Phase list with checkmarks
    let current_idx = phase_index(phase);
    let is_terminal = matches!(phase, UpdatePhase::Done { .. } | UpdatePhase::Failed { .. });

    let phase_lines: Vec<Line> = PHASE_LABELS
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let (icon, style) = if is_terminal && matches!(phase, UpdatePhase::Done { .. }) {
                // All done — every step gets a checkmark
                (
                    "  \u{2714} ", // ✔
                    Style::default().fg(theme::TASK_COMPLETED),
                )
            } else if i < current_idx {
                // Completed phase
                (
                    "  \u{2714} ", // ✔
                    Style::default().fg(theme::TASK_COMPLETED),
                )
            } else if i == current_idx && !is_terminal {
                // Current phase — animated
                let frames = ["  \u{25CB} ", "  \u{25D4} ", "  \u{25D1} ", "  \u{25D5} "];
                let frame_idx = (tick / 10) % frames.len();
                (
                    frames[frame_idx],
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                // Pending phase
                (
                    "  \u{25CB} ", // ○
                    Style::default().fg(theme::MUTED),
                )
            };

            let version_suffix = if i == 1 {
                // "Downloading" — append version if available
                match phase {
                    UpdatePhase::Downloading { version } | UpdatePhase::Done { version } => {
                        format!(" v{}", version)
                    }
                    _ => String::new(),
                }
            } else {
                String::new()
            };

            Line::from(vec![
                Span::styled(icon, style),
                Span::styled(format!("{}{}", label, version_suffix), style),
            ])
        })
        .collect();

    let phases_widget = Paragraph::new(phase_lines);
    frame.render_widget(phases_widget, layout[1]);

    // Animated progress bar
    render_progress_bar(phase, tick, frame, layout[3]);

    // Status message
    let status_msg = match phase {
        UpdatePhase::Done { version } => Line::from(Span::styled(
            format!("Updated to v{}! Restart to apply.", version),
            Style::default()
                .fg(theme::TASK_COMPLETED)
                .add_modifier(Modifier::BOLD),
        )),
        UpdatePhase::Failed { message } => Line::from(Span::styled(
            truncate(message, inner.width as usize - 4),
            Style::default().fg(theme::ERROR_COLOR),
        )),
        _ => Line::from(""),
    };

    let status_widget = Paragraph::new(status_msg).alignment(Alignment::Center);
    frame.render_widget(status_widget, layout[5]);
}

/// Render an animated progress bar.
fn render_progress_bar(phase: &UpdatePhase, tick: usize, frame: &mut Frame, area: Rect) {
    if area.width < 6 {
        return;
    }

    let bar_width = (area.width as usize).saturating_sub(4); // 2 padding each side
    let bar_area = Rect {
        x: area.x + 2,
        y: area.y,
        width: bar_width as u16,
        height: 1,
    };

    match phase {
        UpdatePhase::Done { .. } => {
            // Full bar in green
            let bar: String = "\u{2588}".repeat(bar_width); // █
            let span = Span::styled(
                bar,
                Style::default()
                    .fg(theme::TASK_COMPLETED)
                    .add_modifier(Modifier::BOLD),
            );
            frame.render_widget(Paragraph::new(Line::from(span)), bar_area);
        }
        UpdatePhase::Failed { .. } => {
            // Empty dim bar
            let bar: String = "\u{2591}".repeat(bar_width); // ░
            let span = Span::styled(bar, Style::default().fg(theme::MUTED));
            frame.render_widget(Paragraph::new(Line::from(span)), bar_area);
        }
        _ => {
            // Indeterminate: pulse that travels across the bar
            let pulse_width = bar_width / 4;
            let cycle = bar_width + pulse_width;
            let pos = (tick / 2) % (cycle * 2); // bounce back and forth

            // Convert to position: 0..cycle forward, then cycle..0 backward
            let pos = if pos < cycle { pos } else { cycle * 2 - pos };

            let spans: Vec<Span> = (0..bar_width)
                .map(|i| {
                    let dist = if i >= pos && i < pos + pulse_width {
                        0 // inside pulse
                    } else {
                        let start = pos;
                        let end = pos + pulse_width;
                        if i < start {
                            start - i
                        } else {
                            i - end + 1
                        }
                    };

                    if dist == 0 {
                        // Inside pulse — shimmer color
                        let t = (i.wrapping_sub(pos)) as f32 / pulse_width.max(1) as f32;
                        let color = lerp_color(theme::ACCENT, theme::LOGO_PRIMARY, t);
                        Span::styled(
                            "\u{2588}",
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        )
                    } else if dist <= 2 {
                        // Near pulse — dim glow
                        Span::styled("\u{2593}", Style::default().fg(theme::BORDER_COLOR))
                    // ▓
                    } else {
                        // Background
                        Span::styled("\u{2591}", Style::default().fg(theme::BORDER_DIM))
                        // ░
                    }
                })
                .collect();

            frame.render_widget(Paragraph::new(Line::from(spans)), bar_area);
        }
    }
}

/// Map phase to its index in the phase list.
fn phase_index(phase: &UpdatePhase) -> usize {
    match phase {
        UpdatePhase::Checking => 0,
        UpdatePhase::Downloading { .. } => 1,
        UpdatePhase::Extracting => 2,
        UpdatePhase::Installing => 3,
        UpdatePhase::Done { .. } => 4,
        UpdatePhase::Failed { .. } => 4,
    }
}

/// Linearly interpolate between two colors.
fn lerp_color(a: ratatui::style::Color, b: ratatui::style::Color, t: f32) -> ratatui::style::Color {
    use ratatui::style::Color;
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let lerp = |a: u8, b: u8| -> u8 { (a as f32 + (b as f32 - a as f32) * t) as u8 };
            Color::Rgb(lerp(r1, r2), lerp(g1, g2), lerp(b1, b2))
        }
        _ => a,
    }
}

/// Create a fixed-size centered rectangle.
fn centered_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

/// Truncate a string to fit within `max_len` characters.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}
