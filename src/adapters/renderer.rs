//! Renderer adapter — translates application state into terminal frames.
//!
//! This is a pure read of AppState. No mutation happens here.

use ratatui::layout::Alignment;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::adapters::views::*;
use crate::application::state::AppState;
use crate::infrastructure::tui::layout::FrameLayout;
use crate::infrastructure::tui::theme;
use crate::infrastructure::tui::widgets::{
    confirm_dialog, detail, help_overlay, input_bar, logo, spinner, table, toast,
};

/// Draw the clash UI (not called when attached — terminal is handed off).
pub fn draw(state: &AppState, frame: &mut Frame) {
    let layout = FrameLayout::new(frame.area());

    draw_header(state, frame, layout.header);
    draw_body(state, frame, layout.body);
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
}

fn draw_header(state: &AppState, frame: &mut Frame, area: ratatui::layout::Rect) {
    use crate::domain::entities::SessionStatus;

    let breadcrumbs = state.nav.breadcrumbs().join(" > ");
    let now = chrono::Local::now().format("%H:%M").to_string();

    let filter_indicator = if state.current_view() == crate::adapters::views::ViewKind::Sessions {
        match state.session_filter {
            crate::application::state::SessionFilter::Active => " [active]",
            crate::application::state::SessionFilter::All => " [all]",
        }
    } else {
        ""
    };

    // Count sessions needing attention (prompting for user input)
    let prompting_count = state
        .store
        .sessions
        .iter()
        .filter(|s| s.status == SessionStatus::Waiting || s.status == SessionStatus::Prompting)
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

    if prompting_count > 0 {
        spans.push(Span::styled(
            format!("  ▸ {} prompting", prompting_count),
            Style::default()
                .fg(theme::STATUS_PROMPTING)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }

    spans.push(Span::raw("  "));

    let header = Line::from(spans);

    let time_span = Span::styled(format!("{}  ", now), Style::default().fg(theme::MUTED));

    let header_paragraph = Paragraph::new(header).style(theme::header_style());
    frame.render_widget(header_paragraph, area);

    let time_paragraph = Paragraph::new(time_span)
        .alignment(Alignment::Right)
        .style(theme::header_style());
    frame.render_widget(time_paragraph, area);
}

fn draw_body(state: &AppState, frame: &mut Frame, area: ratatui::layout::Rect) {
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
                sessions::render_sessions_table(state, frame, area);
            } else {
                logo::render_logo(frame, area);
            }
        }
        ViewKind::SessionDetail => {
            detail::render_detail::<session_detail::SessionDetailView>(state, frame, area)
        }
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
        | crate::application::state::InputMode::NewSessionName => {
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

    if let Some(ref spinner_msg) = state.spinner {
        let right_area = ratatui::layout::Rect {
            x: area.x + area.width.saturating_sub(40),
            width: 40.min(area.width),
            ..area
        };
        spinner::render_spinner(spinner_msg, state.tick, frame, right_area);
    } else if let Some(ref toast_msg) = state.toast {
        let right_area = ratatui::layout::Rect {
            x: area.x + area.width.saturating_sub(40),
            width: 40.min(area.width),
            ..area
        };
        toast::render_toast(toast_msg, frame, right_area);
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

fn draw_help(state: &AppState, frame: &mut Frame, area: ratatui::layout::Rect) {
    let global_keys = vec![
        Keybinding::new("j/↓", "Select next"),
        Keybinding::new("k/↑", "Select previous"),
        Keybinding::new("g", "First item"),
        Keybinding::new("G", "Last item"),
        Keybinding::new("Enter", "Drill in"),
        Keybinding::new("Esc", "Go back"),
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
