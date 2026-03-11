use ratatui::style::{Color, Modifier, Style};

// ── Claude Code inspired palette ────────────────────────────────
pub const BG: Color = Color::Rgb(13, 13, 20);        // near-black background
pub const HEADER_BG: Color = Color::Rgb(20, 20, 35);
pub const HEADER_FG: Color = Color::Rgb(180, 180, 200);
pub const FOOTER_BG: Color = Color::Rgb(20, 20, 35);
pub const FOOTER_FG: Color = Color::Rgb(180, 180, 200);
pub const SELECTED_BG: Color = Color::Rgb(35, 35, 60);
pub const BORDER_COLOR: Color = Color::Rgb(55, 55, 85);
pub const BORDER_DIM: Color = Color::Rgb(40, 40, 60);
pub const TITLE_COLOR: Color = Color::Rgb(130, 170, 255); // soft blue
pub const MUTED: Color = Color::Rgb(90, 90, 110);
pub const TEXT: Color = Color::Rgb(210, 210, 225);
pub const TEXT_DIM: Color = Color::Rgb(140, 140, 165);

// ── Role colors (Claude Code style) ─────────────────────────────
pub const USER_COLOR: Color = Color::Rgb(100, 200, 130);   // green for user
pub const CLAUDE_COLOR: Color = Color::Rgb(130, 170, 255);  // blue for claude
// ── Status colors ───────────────────────────────────────────────
pub const STATUS_RUNNING: Color = Color::Rgb(80, 200, 120);
pub const STATUS_THINKING: Color = Color::Rgb(130, 170, 255);
pub const STATUS_WAITING: Color = Color::Rgb(240, 190, 60);
pub const STATUS_STARTING: Color = Color::Rgb(180, 140, 255);
pub const STATUS_PROMPTING: Color = Color::Rgb(255, 120, 80);  // orange-red for approval needed
pub const STATUS_IDLE: Color = Color::Rgb(90, 90, 110);

// ── Accent ──────────────────────────────────────────────────────
pub const ACCENT: Color = Color::Rgb(180, 140, 255);        // purple accent
pub const SEPARATOR: Color = Color::Rgb(45, 45, 70);

pub fn header_style() -> Style {
    Style::default().bg(HEADER_BG).fg(HEADER_FG)
}

pub fn footer_style() -> Style {
    Style::default().bg(FOOTER_BG).fg(FOOTER_FG)
}

pub fn selected_style() -> Style {
    Style::default()
        .bg(SELECTED_BG)
        .add_modifier(Modifier::BOLD)
}

pub fn table_header_style() -> Style {
    Style::default()
        .fg(TEXT_DIM)
        .add_modifier(Modifier::BOLD)
}

pub fn title_style() -> Style {
    Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD)
}

pub fn section_title_style() -> Style {
    Style::default()
        .fg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

pub fn label_style() -> Style {
    Style::default().fg(TEXT_DIM)
}

pub fn value_style() -> Style {
    Style::default().fg(TEXT)
}

pub fn muted_style() -> Style {
    Style::default().fg(MUTED)
}

pub fn spinner_style() -> Style {
    Style::default().fg(STATUS_WAITING)
}

pub fn toast_style() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(Color::Rgb(50, 50, 80))
}

pub fn help_key_style() -> Style {
    Style::default()
        .fg(CLAUDE_COLOR)
        .add_modifier(Modifier::BOLD)
}

pub fn help_desc_style() -> Style {
    Style::default().fg(TEXT)
}

pub fn user_text_style() -> Style {
    Style::default().fg(TEXT)
}

pub fn claude_text_style() -> Style {
    Style::default().fg(TEXT_DIM)
}

pub fn separator_style() -> Style {
    Style::default().fg(SEPARATOR)
}
