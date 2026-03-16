//! Terminal emulator widget — renders a vt100::Screen into a ratatui frame.
//!
//! This bridges the PTY output (parsed by vt100) into ratatui's cell grid,
//! preserving colors, bold, italic, underline, and inverse attributes.
//!
//! Used for inline session rendering when attached to a daemon PTY session.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;

/// A widget that renders a `vt100::Screen` snapshot into a ratatui buffer.
pub struct TerminalWidget<'a> {
    screen: &'a vt100::Screen,
}

impl<'a> TerminalWidget<'a> {
    pub fn new(screen: &'a vt100::Screen) -> Self {
        Self { screen }
    }
}

impl Widget for TerminalWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (screen_rows, screen_cols) = self.screen.size();
        let rows = area.height.min(screen_rows);
        let cols = area.width.min(screen_cols);

        for row in 0..rows {
            for col in 0..cols {
                let cell = self.screen.cell(row, col);
                let Some(cell) = cell else { continue };

                // Skip wide-char continuation cells (second half of CJK chars)
                if cell.is_wide_continuation() {
                    continue;
                }

                let contents = cell.contents();
                let ch = if contents.is_empty() { " " } else { &contents };

                let fg = convert_color(cell.fgcolor());
                let bg = convert_color(cell.bgcolor());

                let mut modifier = Modifier::empty();
                if cell.bold() {
                    modifier |= Modifier::BOLD;
                }
                if cell.italic() {
                    modifier |= Modifier::ITALIC;
                }
                if cell.underline() {
                    modifier |= Modifier::UNDERLINED;
                }

                // Respect the inverse attribute from the vt100 screen
                let (fg, bg) = if cell.inverse() { (bg, fg) } else { (fg, bg) };

                let style = Style::default().fg(fg).bg(bg).add_modifier(modifier);

                let buf_x = area.x + col;
                let buf_y = area.y + row;

                if buf_x < area.x + area.width && buf_y < area.y + area.height {
                    let buf_cell = &mut buf[(buf_x, buf_y)];
                    buf_cell.set_symbol(ch);
                    buf_cell.set_style(style);
                }
            }
        }
    }
}

/// Convert vt100::Color to ratatui::Color.
fn convert_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(idx) => Color::Indexed(idx),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}
