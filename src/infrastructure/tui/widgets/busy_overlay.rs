use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::Frame;

use super::spinner;

/// Render a dimmed overlay with a spinner message in the bottom-right corner.
///
/// Directly modifies buffer cells to dim existing content, then draws the
/// spinner message on top. Should be called as the final render layer.
pub fn render_busy_overlay(message: &str, tick: usize, frame: &mut Frame, area: Rect) {
    // Dim every cell in the area by overwriting fg/bg to muted colors.
    // This creates the "grayed-out" effect over whatever was already rendered.
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(
                    Style::default()
                        .fg(crate::infrastructure::tui::theme::BUSY_FG)
                        .bg(crate::infrastructure::tui::theme::BUSY_BG)
                        .remove_modifier(Modifier::BOLD | Modifier::ITALIC),
                );
            }
        }
    }

    // Render spinner + message in bottom-right corner
    let msg_width = (message.len() as u16 + 4).min(area.width);
    let msg_area = Rect {
        x: area.x + area.width.saturating_sub(msg_width + 2),
        y: area.y + area.height.saturating_sub(1),
        width: msg_width,
        height: 1,
    };
    spinner::render_spinner(message, tick, frame, msg_area);
}
