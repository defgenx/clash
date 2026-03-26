use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::centered::centered_rect;
use crate::adapters::views::Keybinding;
use crate::infrastructure::tui::theme;

/// Render a scrollable help overlay in the center of the screen.
pub fn render_help_overlay(
    title: &str,
    global_keys: &[Keybinding],
    context_keys: &[Keybinding],
    scroll: u16,
    frame: &mut Frame,
    area: Rect,
) {
    let popup_area = centered_rect(60, 80, area);
    frame.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "Global",
        Style::default()
            .fg(theme::TITLE_COLOR)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for kb in global_keys {
        lines.push(keybinding_line(kb));
    }

    if !context_keys.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Context",
            Style::default()
                .fg(theme::TITLE_COLOR)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        for kb in context_keys {
            lines.push(keybinding_line(kb));
        }
    }

    // Commands section
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Commands",
        Style::default()
            .fg(theme::TITLE_COLOR)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for &(cmd, desc) in &[
        (":teams", "Navigate to Teams"),
        (":sessions", "Navigate to Sessions"),
        (":agents", "Navigate to Agents"),
        (":tasks", "Navigate to Tasks"),
        (":new <path>", "New session in directory"),
        (":rename <name>", "Rename session (detail view)"),
        (":create team X", "Create team"),
        (":delete team X", "Delete team"),
        (":active", "Show active sessions"),
        (":all", "Show all sessions"),
        (":tour", "Start guided tour"),
        (":update", "Update clash to latest"),
        (":quit", "Quit (stashes running sessions)"),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<16}", cmd), theme::help_key_style()),
            Span::styled(desc, theme::help_desc_style()),
        ]));
    }

    // Indicators section
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Indicators",
        Style::default()
            .fg(theme::TITLE_COLOR)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for &(symbol, desc) in &[
        ("\u{229e}", "Session open in external pane/tab"),
        ("\u{229f}", "Session in a git worktree (shows project/name)"),
        ("\u{25b6} / \u{25bc}", "Collapsed / expanded subagents"),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<16}", symbol), theme::help_key_style()),
            Span::styled(desc, theme::help_desc_style()),
        ]));
    }

    let total_lines = lines.len() as u16;
    let inner_height = popup_area.height.saturating_sub(2);
    let can_scroll = total_lines > inner_height;
    let clamped_scroll = if can_scroll {
        scroll.min(total_lines.saturating_sub(inner_height))
    } else {
        0
    };

    let scroll_hint = if can_scroll {
        format!(" {} — Help  j/k scroll  ?/Esc close ", title)
    } else {
        format!(" {} — Help (?) ", title)
    };

    let block = Block::default()
        .title(scroll_hint)
        .title_style(theme::title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::DIALOG_BORDER));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((clamped_scroll, 0));

    frame.render_widget(paragraph, popup_area);
}

fn keybinding_line(kb: &Keybinding) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<12}", kb.key), theme::help_key_style()),
        Span::styled(kb.description.clone(), theme::help_desc_style()),
    ])
}
