use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::application::state::{AppState, DiffLineKind};
use crate::infrastructure::tui::theme;

fn style_for_kind(kind: &DiffLineKind) -> Style {
    match kind {
        DiffLineKind::Add => Style::default().fg(theme::DIFF_ADD),
        DiffLineKind::Remove => Style::default().fg(theme::DIFF_REMOVE),
        DiffLineKind::Hunk => Style::default().fg(theme::DIFF_HUNK),
        DiffLineKind::Meta => Style::default()
            .fg(theme::DIFF_META)
            .add_modifier(Modifier::BOLD),
        DiffLineKind::FilePath => Style::default().fg(theme::TEXT_DIM),
        DiffLineKind::Context => Style::default().fg(theme::TEXT),
    }
}

pub fn render_diff(state: &AppState, frame: &mut Frame, area: Rect) {
    let session_name = state
        .diff
        .session_id
        .as_deref()
        .and_then(|id| state.store.find_session(id))
        .and_then(|s| s.name.clone())
        .unwrap_or_else(|| "?".to_string());

    let auto_refresh = state
        .diff
        .session_id
        .as_deref()
        .and_then(|id| state.store.find_session(id))
        .map(|s| s.is_running)
        .unwrap_or(false);

    let title_suffix = if auto_refresh { " [auto-refresh]" } else { "" };

    // Loading / empty states — render as full-width single panel
    if !state.diff.loaded {
        let title = format!(" Diff: {} {}", session_name, title_suffix);
        let block = Block::default()
            .title(title)
            .title_style(theme::title_style())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER_COLOR))
            .style(Style::default().bg(theme::BG));
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "Loading...",
            Style::default().fg(theme::MUTED),
        )))
        .block(block)
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(paragraph, area);
        return;
    }

    if state.diff.lines.is_empty() || state.diff.files.is_empty() {
        let title = format!(" Diff: {} {}", session_name, title_suffix);
        let block = Block::default()
            .title(title)
            .title_style(theme::title_style())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER_COLOR))
            .style(Style::default().bg(theme::BG));
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "No changes (working tree clean)",
            Style::default().fg(theme::MUTED),
        )))
        .block(block)
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(paragraph, area);
        return;
    }

    // Two-panel layout: 25% file list, 75% diff content
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
        .split(area);

    // ── Left panel: file list ──
    let file_items: Vec<ListItem> = state
        .diff
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let label = format!(" {} [+{}/-{}]", f.path, f.additions, f.deletions);
            let style = if i == state.diff.selected_file {
                theme::selected_style()
            } else {
                Style::default().fg(theme::TEXT)
            };
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect();

    let files_block = Block::default()
        .title(" Files ")
        .title_style(theme::title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER_COLOR))
        .style(Style::default().bg(theme::BG));

    let file_list = List::new(file_items).block(files_block);
    frame.render_widget(file_list, chunks[0]);

    // ── Right panel: selected file's diff ──
    let selected_file = state.diff.files.get(state.diff.selected_file);

    let (diff_lines, file_path): (Vec<Line>, String) = if let Some(file) = selected_file {
        let slice = &state.diff.lines[file.start_line..file.end_line];
        let lines: Vec<Line> = slice
            .iter()
            .map(|dl| {
                Line::from(Span::styled(
                    format!("  {}", dl.content),
                    style_for_kind(&dl.kind),
                ))
            })
            .collect();
        (lines, file.path.clone())
    } else {
        (vec![], String::new())
    };

    let diff_title = format!(" {} {}", file_path, title_suffix);
    let total_lines = diff_lines.len() as u16;
    let visible_height = chunks[1].height.saturating_sub(2);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_offset = state.diff.file_scroll.min(max_scroll);

    let diff_block = Block::default()
        .title(diff_title)
        .title_style(theme::title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER_COLOR))
        .style(Style::default().bg(theme::BG));

    let paragraph = Paragraph::new(diff_lines)
        .block(diff_block)
        .scroll((scroll_offset, 0));

    frame.render_widget(paragraph, chunks[1]);

    if total_lines > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(max_scroll as usize).position(scroll_offset as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(Style::default().fg(theme::BORDER_DIM)),
            chunks[1],
            &mut scrollbar_state,
        );
    }
}
