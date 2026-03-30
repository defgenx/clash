//! Renderer adapter — translates application state into terminal frames.
//!
//! This is a pure read of AppState. No mutation happens here.

use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::adapters::views::*;
use crate::application::state::{AppState, InputMode};
use crate::infrastructure::tui::layout::FrameLayout;
use crate::infrastructure::tui::theme;
use crate::infrastructure::tui::widgets::{
    busy_overlay, confirm_dialog, detail, diff_widget, help_overlay, input_bar, logo,
    picker_dialog, table, terminal, toast, update_overlay,
};

/// Draw the clash UI.
pub fn draw(
    state: &AppState,
    sessions_visual_state: &mut ratatui::widgets::TableState,
    frame: &mut Frame,
) {
    let layout = FrameLayout::new(frame.area());

    // When attached: render inline terminal
    if state.input_mode == InputMode::Attached {
        draw_header(state, frame, layout.header);
        if let Some(ref parser) = state.terminal_screen {
            let screen = parser.screen();
            let widget = terminal::TerminalWidget::new(screen);
            frame.render_widget(widget, layout.body);

            // Cursor positioning strategy:
            // - If vt100 says cursor is on the prompt line → trust cursor_position()
            //   (tracks arrow keys, Option+Left, etc.)
            // - Otherwise → find the prompt line and place cursor at end of text
            //   (Claude Code's ink parks cursor elsewhere during re-renders)
            let (cursor_row, cursor_col) = screen.cursor_position();
            let prompt_line = find_prompt_line(screen);

            let (cy, cx) = if let Some(prompt_row) = prompt_line {
                if cursor_row == prompt_row {
                    // Cursor is on prompt line — use real column position
                    (prompt_row, cursor_col)
                } else {
                    // Cursor is elsewhere (ink re-render) — snap to end of prompt text
                    (prompt_row, find_text_end(screen, prompt_row))
                }
            } else {
                // No prompt found — use vt100 cursor as-is
                (cursor_row, cursor_col)
            };

            let px = layout.body.x + cx;
            let py = layout.body.y + cy;
            if px < layout.body.x + layout.body.width && py < layout.body.y + layout.body.height {
                frame.set_cursor_position(ratatui::layout::Position { x: px, y: py });
            }
        }
        let session_label = state
            .attached_session
            .as_deref()
            .map(|s| crate::adapters::format::short_id(s, 8))
            .unwrap_or("?");
        let hint = Line::from(vec![
            Span::styled(
                format!(" {} ", session_label),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled("  Ctrl+B detach", Style::default().fg(theme::MUTED)),
        ]);
        frame.render_widget(
            Paragraph::new(hint).style(theme::footer_style()),
            layout.footer,
        );

        // Show busy overlay while waiting for terminal to load
        if state.terminal_screen.is_none() {
            if let Some(ref msg) = state.spinner {
                busy_overlay::render_busy_overlay(msg, state.tick, frame, frame.area());
            }
        }
        return;
    }

    draw_header(state, frame, layout.header);
    draw_body(state, sessions_visual_state, frame, layout.body);
    draw_footer(state, frame, layout.footer);

    // Overlays (drawn on top)
    if let Some(step) = state.tour_step {
        crate::infrastructure::tui::widgets::tour::render_tour(step, frame, frame.area());
    } else if state.show_help {
        draw_help(state, frame, frame.area());
    }
    if let Some(ref dialog) = state.confirm_dialog {
        confirm_dialog::render_confirm_dialog(&dialog.message, frame, frame.area());
    }
    if let Some(ref picker) = state.picker_dialog {
        picker_dialog::render_picker_dialog(picker, frame, frame.area());
    }

    // Update progress overlay — on top of everything except busy
    if let Some(ref phase) = state.update_progress {
        update_overlay::render_update_overlay(phase, state.tick, frame, frame.area());
    }

    // Busy overlay — drawn last, on top of everything, on any view
    if let Some(ref msg) = state.spinner {
        busy_overlay::render_busy_overlay(msg, state.tick, frame, frame.area());
    }
}

fn draw_header(state: &AppState, frame: &mut Frame, area: ratatui::layout::Rect) {
    use crate::domain::entities::SessionStatus;

    let breadcrumbs = state.nav.breadcrumbs().join(" > ");
    let now = chrono::Local::now().format("%H:%M").to_string();

    let filter_indicator: String =
        if state.current_view() == crate::adapters::views::ViewKind::Sessions {
            let session_label = match state.session_filter {
                crate::application::state::SessionFilter::Active => "active",
                crate::application::state::SessionFilter::All => "all",
            };
            use crate::application::state::SectionFilter;
            let section_label = match state.section_filter {
                SectionFilter::All => None,
                SectionFilter::Active => Some("running"),
                SectionFilter::Done => Some("stashed"),
                SectionFilter::Fail => Some("errored"),
            };
            if let Some(section) = section_label {
                let count = state.filtered_sessions().len();
                format!(" [{}:{} ({})]", session_label, section, count)
            } else {
                format!(" [{}]", session_label)
            }
        } else {
            String::new()
        };

    // Count sessions needing approval (Prompting) vs waiting for input (Waiting)
    let approval_count = state
        .store
        .sessions
        .iter()
        .filter(|s| s.status == SessionStatus::Prompting)
        .count();
    let waiting_count = state
        .store
        .sessions
        .iter()
        .filter(|s| s.status == SessionStatus::Waiting)
        .count();

    let mut spans = vec![
        Span::styled(
            " ✦ clash",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(theme::BORDER_DIM)),
        Span::styled(breadcrumbs, Style::default().fg(theme::TEXT_DIM)),
        Span::styled(filter_indicator, Style::default().fg(theme::STATUS_RUNNING)),
    ];

    if approval_count > 0 {
        let icon = crate::adapters::format::status_icon(SessionStatus::Prompting, state.tick);
        spans.push(Span::styled(
            format!("  {} {} approval needed", icon, approval_count),
            Style::default()
                .fg(theme::STATUS_PROMPTING)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }
    if waiting_count > 0 {
        spans.push(Span::styled(
            format!("  ◉ {} waiting", waiting_count),
            Style::default()
                .fg(theme::STATUS_WAITING)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }

    spans.push(Span::raw("  "));

    let header = Line::from(spans);

    let time_span = if state.debug_mode {
        Line::from(vec![
            Span::styled("DEBUG ", Style::default().fg(theme::STATUS_PROMPTING)),
            Span::styled(format!("{}  ", now), Style::default().fg(theme::MUTED)),
        ])
    } else {
        Line::from(Span::styled(
            format!("{}  ", now),
            Style::default().fg(theme::MUTED),
        ))
    };

    let header_paragraph = Paragraph::new(header).style(theme::header_style());
    frame.render_widget(header_paragraph, area);

    let time_paragraph = Paragraph::new(time_span)
        .alignment(Alignment::Right)
        .style(theme::header_style());
    frame.render_widget(time_paragraph, area);
}

fn draw_body(
    state: &AppState,
    sessions_visual_state: &mut ratatui::widgets::TableState,
    frame: &mut Frame,
    area: ratatui::layout::Rect,
) {
    match state.current_view() {
        ViewKind::Teams => table::render_table::<teams::TeamsTable>(state, frame, area),
        ViewKind::TeamDetail => {
            detail::render_detail::<team_detail::TeamDetailView>(state, frame, area)
        }
        ViewKind::Agents => table::render_table::<agents::AgentsTable>(state, frame, area),
        ViewKind::AgentDetail => {
            detail::render_detail::<agent_detail::AgentDetailView>(state, frame, area)
        }
        ViewKind::Tasks => table::render_table::<tasks::TasksTable>(state, frame, area),
        ViewKind::TaskDetail => {
            detail::render_detail::<task_detail::TaskDetailView>(state, frame, area)
        }
        ViewKind::Inbox => table::render_table::<inbox::InboxTable>(state, frame, area),
        ViewKind::Prompts => detail::render_detail::<prompts::PromptsView>(state, frame, area),
        ViewKind::Sessions => {
            if sessions::SessionsTable::has_items(state) {
                sessions::render_sessions_table(state, sessions_visual_state, frame, area);
            } else {
                logo::render_logo(frame, area);
            }
        }
        ViewKind::SessionDetail => {
            detail::render_detail::<session_detail::SessionDetailView>(state, frame, area)
        }
        ViewKind::Diff => diff_widget::render_diff(state, frame, area),
        ViewKind::Subagents => table::render_table::<subagents::SubagentsTable>(state, frame, area),
        ViewKind::SubagentDetail => {
            detail::render_detail::<subagent_detail::SubagentDetailView>(state, frame, area)
        }
    }
}

fn draw_footer(state: &AppState, frame: &mut Frame, area: ratatui::layout::Rect) {
    match &state.input_mode {
        crate::application::state::InputMode::Command
        | crate::application::state::InputMode::Filter
        | crate::application::state::InputMode::NewSession
        | crate::application::state::InputMode::NewSessionName
        | crate::application::state::InputMode::NewSessionWorktree => {
            input_bar::render_input_bar(
                &state.input_mode,
                &state.input_buffer,
                state.input_cursor,
                frame,
                area,
            );
            return;
        }
        _ => {}
    }

    let left = if !state.filter.is_empty() {
        format!(" /{}", state.filter)
    } else {
        " :command  /filter  ?help".to_string()
    };

    let left_span = Span::styled(left, theme::footer_style());
    let left_paragraph = Paragraph::new(left_span).style(theme::footer_style());
    frame.render_widget(left_paragraph, area);

    if let Some(ref toast_msg) = state.toast {
        // Size the toast area to the message length (+ padding) and right-align it
        let msg_width = (toast_msg.len() as u16 + 2).min(area.width);
        let right_area = ratatui::layout::Rect {
            x: area.x + area.width.saturating_sub(msg_width),
            width: msg_width,
            ..area
        };
        toast::render_toast(toast_msg, state.tick, frame, right_area);
    } else {
        // Show version on the right side of the footer
        let version = format!("v{}  ", env!("CARGO_PKG_VERSION"));
        let version_span = Span::styled(version, Style::default().fg(theme::MUTED));
        let version_paragraph = Paragraph::new(version_span)
            .alignment(Alignment::Right)
            .style(theme::footer_style());
        frame.render_widget(version_paragraph, area);
    }
}

/// Find the row containing the prompt (`❯` or `>`), scanning bottom-to-top.
fn find_prompt_line(screen: &vt100::Screen) -> Option<u16> {
    let (rows, cols) = screen.size();
    for row in (0..rows).rev() {
        let mut line = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                let c = cell.contents();
                line.push_str(if c.is_empty() { " " } else { &c });
            }
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('❯') || trimmed.starts_with('>') {
            return Some(row);
        }
    }
    None
}

/// Find the column after the last non-space character on a given row.
fn find_text_end(screen: &vt100::Screen, row: u16) -> u16 {
    let (_, cols) = screen.size();
    let mut last_non_space: u16 = 0;
    for col in 0..cols {
        if let Some(cell) = screen.cell(row, col) {
            let c = cell.contents();
            if !c.is_empty() && c != " " {
                last_non_space = col + 1;
            }
        }
    }
    last_non_space.min(cols.saturating_sub(1))
}

fn draw_help(state: &AppState, frame: &mut Frame, area: ratatui::layout::Rect) {
    let global_keys = vec![
        Keybinding::new("j/\u{2193}", "Select next"),
        Keybinding::new("k/\u{2191}", "Select previous"),
        Keybinding::new("g", "First item"),
        Keybinding::new("G", "Last item"),
        Keybinding::new("Enter", "Drill in"),
        Keybinding::new("Esc", "Go back"),
        Keybinding::new("a", "Attach to session (inline)"),
        Keybinding::new("o", "Open in new pane/tab"),
        Keybinding::new("e", "Open in IDE"),
        Keybinding::new("O", "Open ALL running sessions"),
        Keybinding::new("p", "View diff"),
        Keybinding::new("w", "Open in git worktree"),
        Keybinding::new(":", "Command mode"),
        Keybinding::new("/", "Filter mode"),
        Keybinding::new("r", "Refresh"),
        Keybinding::new("q", "Quit"),
    ];

    let context_keys = match state.current_view() {
        ViewKind::Teams => teams::TeamsTable::context_keybindings(),
        ViewKind::Tasks => tasks::TasksTable::context_keybindings(),
        ViewKind::Agents => agents::AgentsTable::context_keybindings(),
        ViewKind::Inbox => inbox::InboxTable::context_keybindings(),
        ViewKind::TeamDetail => team_detail::TeamDetailView::context_keybindings(),
        ViewKind::TaskDetail => task_detail::TaskDetailView::context_keybindings(),
        ViewKind::AgentDetail => agent_detail::AgentDetailView::context_keybindings(),
        ViewKind::Prompts => prompts::PromptsView::context_keybindings(),
        ViewKind::Sessions => sessions::SessionsTable::context_keybindings(),
        ViewKind::SessionDetail => session_detail::SessionDetailView::context_keybindings(),
        ViewKind::Diff => diff::DiffView::context_keybindings(),
        ViewKind::Subagents => subagents::SubagentsTable::context_keybindings(),
        ViewKind::SubagentDetail => subagent_detail::SubagentDetailView::context_keybindings(),
    };

    help_overlay::render_help_overlay(
        state.current_view().label(),
        &global_keys,
        &context_keys,
        state.help_scroll,
        frame,
        area,
    );
}
