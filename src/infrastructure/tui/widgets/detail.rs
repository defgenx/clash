use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::adapters::views::DetailView;
use crate::application::state::AppState;
use crate::infrastructure::tui::theme;

pub fn render_detail<V: DetailView>(state: &AppState, frame: &mut Frame, area: Rect) {
    let title = V::title(state);
    let sections = V::sections(state);

    let mut lines: Vec<Line> = Vec::new();

    for (i, section) in sections.iter().enumerate() {
        // Section separator (except before first section)
        if i > 0 {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                format!("  {}", "─".repeat(area.width.saturating_sub(4) as usize)),
                theme::separator_style(),
            )]));
            lines.push(Line::from(""));
        }

        let is_conversation = section.title.starts_with("Conversation");

        // Section header with icon
        let section_icon = if is_conversation {
            "  ◆ "
        } else if section.title.starts_with("Subagent") {
            "  ◈ "
        } else if section.title == "Info" {
            "  ● "
        } else if section.title == "Summary" {
            "  ◇ "
        } else {
            "  ◦ "
        };

        lines.push(Line::from(vec![
            Span::styled(section_icon, theme::section_title_style()),
            Span::styled(section.title.to_uppercase(), theme::section_title_style()),
        ]));
        lines.push(Line::from(""));

        if section.loading {
            render_loading_shimmer(&mut lines, state.tick);
        } else if is_conversation {
            render_conversation_section(&section.rows, &mut lines, area.width);
        } else {
            render_info_section(&section.rows, &mut lines);
        }
    }

    let total_lines = lines.len() as u16;
    let visible_height = area.height.saturating_sub(2);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_offset = state.scroll_state.offset.min(max_scroll);

    let block = Block::default()
        .title(format!(" {} ", title))
        .title_style(theme::title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER_COLOR))
        .style(Style::default().bg(theme::BG));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((scroll_offset, 0));

    frame.render_widget(paragraph, area);

    // Scrollbar
    if total_lines > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(max_scroll as usize).position(scroll_offset as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .style(Style::default().fg(theme::BORDER_DIM));
        frame.render_stateful_widget(
            scrollbar,
            area.inner(ratatui::layout::Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }
}

fn render_info_section<'a>(rows: &'a [(String, String)], lines: &mut Vec<Line<'a>>) {
    for (label, value) in rows {
        if label.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(value.as_str(), theme::value_style()),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(format!("{:<16}", label), theme::label_style()),
                Span::styled(value.as_str(), theme::value_style()),
            ]));
        }
    }
}

fn render_conversation_section<'a>(
    rows: &'a [(String, String)],
    lines: &mut Vec<Line<'a>>,
    width: u16,
) {
    let msg_width = width.saturating_sub(10) as usize;

    for (role, text) in rows {
        let is_user = role == "USER";
        let is_claude = role == "CLAUDE";

        let (badge, badge_style) = if is_user {
            (
                " ❯ You ".to_string(),
                Style::default()
                    .fg(theme::USER_COLOR)
                    .add_modifier(Modifier::BOLD),
            )
        } else if is_claude {
            (
                " ✦ Claude ".to_string(),
                Style::default()
                    .fg(theme::CLAUDE_COLOR)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (format!(" {} ", role), theme::muted_style())
        };

        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(badge, badge_style),
        ]));

        // Message text with left border indicator
        let text_style = if is_user {
            theme::user_text_style()
        } else {
            theme::claude_text_style()
        };

        let border_char = "│";
        let border_style = if is_user {
            Style::default().fg(theme::USER_COLOR)
        } else {
            Style::default().fg(theme::CLAUDE_COLOR)
        };

        for msg_line in text.lines() {
            if msg_line.trim().is_empty() {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(border_char, border_style),
                    Span::raw("  "),
                ]));
            } else {
                // Word-wrap long lines
                let wrapped = word_wrap(msg_line, msg_width);
                for wline in wrapped {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(border_char, border_style),
                        Span::raw("  "),
                        Span::styled(wline, text_style),
                    ]));
                }
            }
        }

        // Space between messages
        lines.push(Line::from(""));
    }
}

/// Simple word wrap for long lines.
fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || text.len() <= max_width {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line = word.to_string();
        } else if current_line.len() + 1 + word.len() > max_width {
            result.push(current_line);
            current_line = word.to_string();
        } else {
            current_line.push(' ');
            current_line.push_str(word);
        }
    }
    if !current_line.is_empty() {
        result.push(current_line);
    }

    if result.is_empty() {
        vec![text.to_string()]
    } else {
        result
    }
}

/// Spinner frames matching `spinner.rs`.
const SPINNER_FRAMES: &[&str] = &["○", "◔", "◑", "◕", "●", "◕", "◑", "◔"];
const TICKS_PER_FRAME: usize = 8;

/// Shimmer gradient keyframes (same as `spinner.rs`).
const SHIMMER: &[(u8, u8, u8)] = &[
    (180, 140, 255),
    (220, 150, 215),
    (140, 200, 240),
    (130, 210, 210),
    (180, 140, 255),
];
const CYCLE_TICKS: usize = 120;
const CHAR_SPREAD: usize = 6;

/// Render an inline loading spinner with shimmer text for a section.
fn render_loading_shimmer(lines: &mut Vec<Line<'_>>, tick: usize) {
    let spinner_char = SPINNER_FRAMES[(tick / TICKS_PER_FRAME) % SPINNER_FRAMES.len()];
    let full_text = format!("      {} Loading...", spinner_char);

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

    lines.push(Line::from(spans));
}

/// Interpolate the shimmer gradient at position `t` (0.0 -- 1.0).
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
