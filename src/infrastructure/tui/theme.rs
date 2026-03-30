use ratatui::style::{Color, Modifier, Style};

// ── Dark base ──────────────────────────────────────────────────
pub const BG: Color = Color::Rgb(12, 12, 18); // deep navy-black
pub const HEADER_BG: Color = Color::Rgb(22, 18, 32); // dark plum
pub const HEADER_FG: Color = Color::Rgb(210, 195, 230); // soft lavender
pub const FOOTER_BG: Color = Color::Rgb(22, 18, 32);
pub const FOOTER_FG: Color = Color::Rgb(210, 195, 230);
pub const SELECTED_BG: Color = Color::Rgb(30, 25, 45); // dark grape
pub const BORDER_COLOR: Color = Color::Rgb(65, 55, 90); // dusty purple
pub const BORDER_DIM: Color = Color::Rgb(45, 38, 65); // dim grape
pub const SEPARATOR: Color = Color::Rgb(50, 42, 72);

// ── Text hierarchy ─────────────────────────────────────────────
pub const TEXT: Color = Color::Rgb(220, 218, 232); // soft white
pub const TEXT_DIM: Color = Color::Rgb(155, 148, 178); // dusty lilac
pub const MUTED: Color = Color::Rgb(95, 88, 115); // muted plum

// ── Accent & titles ────────────────────────────────────────────
pub const ACCENT: Color = Color::Rgb(165, 145, 215); // soft lavender
pub const TITLE_COLOR: Color = Color::Rgb(145, 180, 210); // soft sky

// ── Role colors (conversation) ─────────────────────────────────
pub const USER_COLOR: Color = Color::Rgb(135, 195, 165); // soft mint
pub const CLAUDE_COLOR: Color = Color::Rgb(145, 175, 210); // soft steel blue

// ── Semantic entity colors ─────────────────────────────────────
pub const NAME_COLOR: Color = Color::Rgb(135, 190, 190); // soft teal — entity names
pub const PATH_COLOR: Color = Color::Rgb(140, 185, 150); // soft sage — file paths, CWDs
pub const BRANCH_COLOR: Color = Color::Rgb(200, 185, 125); // muted gold — git branches
pub const COUNT_COLOR: Color = Color::Rgb(170, 158, 205); // soft violet — counts, numbers
pub const DESCRIPTION_COLOR: Color = Color::Rgb(160, 155, 180); // silver lavender — descriptions

// ── Status colors ──────────────────────────────────────────────
pub const STATUS_RUNNING: Color = Color::Rgb(130, 195, 150); // soft sage
pub const STATUS_THINKING: Color = Color::Rgb(145, 175, 210); // soft steel blue
pub const STATUS_WAITING: Color = Color::Rgb(210, 190, 120); // muted gold
pub const STATUS_STARTING: Color = Color::Rgb(165, 145, 215); // soft lavender
pub const STATUS_PROMPTING: Color = Color::Rgb(210, 145, 135); // muted salmon
pub const STATUS_IDLE: Color = Color::Rgb(95, 88, 115); // muted plum

// ── Task status colors ─────────────────────────────────────────
pub const TASK_COMPLETED: Color = Color::Rgb(130, 195, 150); // soft sage
pub const TASK_IN_PROGRESS: Color = Color::Rgb(210, 190, 120); // muted gold
pub const TASK_BLOCKED: Color = Color::Rgb(200, 130, 135); // muted rose
pub const TASK_PENDING: Color = Color::Rgb(95, 88, 115); // muted plum
pub const TASK_UNKNOWN: Color = Color::Rgb(165, 145, 215); // soft lavender

// ── Section header colors ─────────────────────────────────────
pub const SECTION_ACTIVE: Color = Color::Rgb(145, 175, 210); // soft steel blue
pub const SECTION_DONE: Color = Color::Rgb(130, 195, 150); // soft sage
pub const SECTION_FAIL: Color = Color::Rgb(200, 130, 135); // muted rose

// ── Feedback colors ────────────────────────────────────────────
pub const ERROR_COLOR: Color = Color::Rgb(200, 130, 135); // muted rose
pub const UNREAD_COLOR: Color = Color::Rgb(210, 190, 120); // muted gold

// ── Diff colors ───────────────────────────────────────────────
pub const DIFF_ADD: Color = STATUS_RUNNING; // pastel green
pub const DIFF_REMOVE: Color = ERROR_COLOR; // pastel rose
pub const DIFF_HUNK: Color = NAME_COLOR; // pastel teal
pub const DIFF_META: Color = ACCENT; // soft violet

// ── Dialog / overlay colors ────────────────────────────────────
pub const DIALOG_BORDER: Color = Color::Rgb(145, 180, 210); // soft sky
pub const DIALOG_TITLE: Color = Color::Rgb(145, 180, 210); // soft sky
pub const CONFIRM_BORDER: Color = Color::Rgb(210, 190, 120); // muted gold
pub const BUSY_FG: Color = Color::Rgb(55, 48, 72); // dimmed plum
pub const BUSY_BG: Color = Color::Rgb(8, 8, 14); // near-black

// ── Input mode colors ──────────────────────────────────────────
pub const COMMAND_COLOR: Color = Color::Rgb(200, 185, 125); // muted gold for : command mode
pub const FILTER_COLOR: Color = Color::Rgb(135, 195, 165); // soft mint for / filter mode
pub const PROMPT_COLOR: Color = Color::Rgb(135, 190, 190); // soft teal for input prompts

// ── Logo neon colors (intentionally brighter than the pastel UI) ──
pub const LOGO_PRIMARY: Color = Color::Rgb(175, 130, 255); // vibrant purple
pub const LOGO_GLOW: Color = Color::Rgb(110, 80, 180); // purple glow
pub const LOGO_ACCENT: Color = Color::Rgb(255, 140, 200); // neon pink sparkle
pub const LOGO_CORE: Color = Color::Rgb(255, 245, 250); // white-hot explosion center

// ── Composed styles ────────────────────────────────────────────

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
    Style::default().fg(TEXT_DIM).add_modifier(Modifier::BOLD)
}

pub fn title_style() -> Style {
    Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD)
}

pub fn section_title_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
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

pub fn name_style() -> Style {
    Style::default().fg(NAME_COLOR).add_modifier(Modifier::BOLD)
}
