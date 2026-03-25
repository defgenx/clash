use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::Frame;

use ratatui::style::Color;

const SPINNER_FRAMES: &[&str] = &["○", "◔", "◑", "◕", "●", "◕", "◑", "◔"];
/// Ticks per spinner frame. At 10ms/tick this gives ~80ms per frame.
const TICKS_PER_FRAME: usize = 8;

/// Shimmer gradient keyframes — loops back to the first color.
const SHIMMER: &[(u8, u8, u8)] = &[
    (180, 140, 255), // soft violet
    (220, 150, 215), // pastel pink
    (140, 200, 240), // pastel sky
    (130, 210, 210), // pastel teal
    (180, 140, 255), // soft violet (loop)
];

/// Ticks for one full gradient cycle across the text.
const CYCLE_TICKS: usize = 120;
/// Phase offset between adjacent characters (higher = tighter wave).
const CHAR_SPREAD: usize = 6;

/// Render a spinner with a shimmer-animated message.
pub fn render_spinner(message: &str, tick: usize, frame: &mut Frame, area: Rect) {
    let spinner_char = SPINNER_FRAMES[(tick / TICKS_PER_FRAME) % SPINNER_FRAMES.len()];

    let full_text = format!("{} {}", spinner_char, message);

    let spans: Vec<Span> = full_text
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let phase = ((i.wrapping_mul(CHAR_SPREAD).wrapping_add(tick)) % CYCLE_TICKS) as f32
                / CYCLE_TICKS as f32;
            let color = shimmer_at(phase);
            Span::styled(
                ch.to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )
        })
        .collect();

    let paragraph = ratatui::widgets::Paragraph::new(Line::from(spans));
    frame.render_widget(paragraph, area);
}

/// Interpolate the shimmer gradient at position `t` (0.0 – 1.0).
fn shimmer_at(t: f32) -> Color {
    let n = SHIMMER.len() - 1;
    let scaled = t * n as f32;
    let idx = (scaled as usize).min(n - 1);
    let frac = scaled - idx as f32;

    let (r1, g1, b1) = SHIMMER[idx];
    let (r2, g2, b2) = SHIMMER[idx + 1];

    Color::Rgb(
        lerp_u8(r1, r2, frac),
        lerp_u8(g1, g2, frac),
        lerp_u8(b1, b2, frac),
    )
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t) as u8
}
