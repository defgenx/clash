use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::adapters::format::{self, or_dash};
use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::{Session, SessionStatus, Subagent};
use crate::infrastructure::tui::{theme, widgets::table::compute_constraints};

pub struct SessionsTable;

impl SessionsTable {
    pub fn has_items(state: &AppState) -> bool {
        !state.filtered_sessions().is_empty()
    }
}

/// Build the AGENTS column text from subagents: shows compact status summary.
fn agents_summary(subagents: &[Subagent]) -> String {
    if subagents.is_empty() {
        return "—".to_string();
    }
    let (mut thinking, mut running, mut prompting) = (0u16, 0u16, 0u16);
    for sa in subagents {
        match sa.status {
            SessionStatus::Thinking => thinking += 1,
            SessionStatus::Running => running += 1,
            SessionStatus::Prompting | SessionStatus::Waiting => prompting += 1,
            _ => {}
        }
    }
    let total = subagents.len();
    let mut parts = Vec::new();
    if prompting > 0 {
        parts.push(format!("{}!", prompting));
    }
    if thinking > 0 {
        parts.push(format!("{}◎", thinking));
    }
    if running > 0 {
        parts.push(format!("{}●", running));
    }
    if parts.is_empty() {
        format!("{}", total)
    } else {
        format!("{} ({})", total, parts.join(" "))
    }
}

/// Build a subagent child row (indented) for expanded sessions.
fn subagent_row(sa: &Subagent, tick: usize) -> Vec<Cell<'static>> {
    let (status, style) = format::status_cell(sa.status, tick);
    let display_id = format::truncate(&sa.id, 15, "…");
    let summary = if sa.summary.is_empty() {
        "—".to_string()
    } else {
        format::truncate(&sa.summary, 40, "…")
    };
    let agent_type = if sa.agent_type.is_empty() {
        "agent".to_string()
    } else {
        sa.agent_type.clone()
    };

    let worktree_display = match &sa.worktree {
        Some(name) => format!("⊟ {}", name),
        None => "".to_string(),
    };

    vec![
        Cell::from(format!("  {}", status)).style(style),
        Cell::from(format!("  └ {}", display_id)).style(Style::default().fg(theme::MUTED)),
        Cell::from(agent_type).style(Style::default().fg(theme::TEXT_DIM)),
        Cell::from(summary).style(Style::default().fg(theme::TEXT_DIM)),
        Cell::from("".to_string()),
        Cell::from("".to_string()),
        Cell::from(worktree_display).style(Style::default().fg(theme::ACCENT)),
    ]
}

/// Extract plain text values from a session row (shared by row() and row_texts()).
fn session_texts(item: &Session, tick: usize) -> Vec<String> {
    let (status, _) = format::status_cell(item.status, tick);
    let name_display = item.name.clone().unwrap_or_else(|| "—".to_string());
    let display_name = or_dash(if item.summary.is_empty() {
        ""
    } else {
        &item.summary
    });
    let project_display = item
        .project_path
        .rsplit('/')
        .next()
        .unwrap_or(&item.project_path)
        .to_string();
    let agents = if item.subagent_count > 0 {
        format!("{}", item.subagent_count)
    } else {
        "—".to_string()
    };
    let branch = item
        .source_branch
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&item.git_branch);
    let branch = or_dash(branch).to_string();
    let worktree_display = match &item.worktree {
        Some(name) => format!("⊟ {}", name),
        None => "—".to_string(),
    };

    vec![
        status,
        name_display,
        project_display,
        display_name.to_string(),
        agents,
        branch,
        worktree_display,
    ]
}

/// Custom renderer for the Sessions table with expandable subagent rows.
pub fn render_sessions_table(state: &AppState, frame: &mut Frame, area: Rect) {
    let sessions = state.filtered_sessions();
    if sessions.is_empty() {
        return;
    }

    let columns = SessionsTable::columns();
    let header_cells: Vec<Cell> = columns
        .iter()
        .map(|c| Cell::from(c.name.clone()).style(theme::table_header_style()))
        .collect();
    let header = Row::new(header_cells).height(1);

    // Measure content for dynamic column sizing
    let content_rows: Vec<Vec<String>> = sessions
        .iter()
        .map(|s| session_texts(s, state.tick))
        .collect();
    let constraints = compute_constraints(&columns, &content_rows, area.width);

    // Build rows: parent sessions + expanded child subagent rows
    let mut rows: Vec<Row> = Vec::new();
    let mut logical_to_parent: Vec<usize> = Vec::new(); // maps row index → parent session index

    for (i, session) in sessions.iter().enumerate() {
        let is_expanded = state.expanded_sessions.contains(&session.id);

        // Get subagents for this session
        let subs = state.store.subagents_by_session.get(&session.id);

        // Only show expand arrow if there are active (non-idle) subagents
        let has_active_subs = subs
            .map(|s| s.iter().any(|sa| !matches!(sa.status, SessionStatus::Idle)))
            .unwrap_or(false);
        let expand_indicator = if has_active_subs {
            if is_expanded {
                "▼ "
            } else {
                "▶ "
            }
        } else {
            "  "
        };
        let agents_text = if let Some(subs) = subs {
            agents_summary(subs)
        } else if session.subagent_count > 0 {
            format!("{}", session.subagent_count)
        } else {
            "—".to_string()
        };

        let (status, status_style) = format::status_cell(session.status, state.tick);
        let name_display = session.name.as_deref().unwrap_or("—");
        let display_name = or_dash(if session.summary.is_empty() {
            ""
        } else {
            &session.summary
        });
        let project_display = session
            .project_path
            .rsplit('/')
            .next()
            .unwrap_or(&session.project_path);
        let branch_str = session
            .source_branch
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&session.git_branch);
        let branch = or_dash(branch_str);
        let worktree_display = match &session.worktree {
            Some(name) => format!("⊟ {}", name),
            None => "—".to_string(),
        };

        let is_ext_open = state.externally_opened.contains(&session.id);
        let name_text = if is_ext_open {
            format!("⊞ {}", name_display)
        } else {
            name_display.to_string()
        };
        let name_style = if is_ext_open {
            Style::default()
                .fg(theme::TEXT_DIM)
                .add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme::CLAUDE_COLOR)
                .add_modifier(ratatui::style::Modifier::BOLD)
        };

        let cells = vec![
            Cell::from(format!("{}{}", expand_indicator, status)).style(status_style),
            Cell::from(name_text).style(name_style),
            Cell::from(project_display.to_string()).style(Style::default().fg(theme::PATH_COLOR)),
            Cell::from(display_name.to_string()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(agents_text).style(Style::default().fg(theme::COUNT_COLOR)),
            Cell::from(branch.to_string()).style(Style::default().fg(theme::BRANCH_COLOR)),
            Cell::from(worktree_display).style(Style::default().fg(theme::ACCENT)),
        ];

        let row = Row::new(cells);
        let row = if i == state.table_state.selected {
            row.style(theme::selected_style())
        } else {
            row
        };
        rows.push(row);
        logical_to_parent.push(i);

        // Add child rows if expanded (only active subagents)
        if is_expanded {
            if let Some(subs) = subs {
                for sa in subs
                    .iter()
                    .filter(|sa| !matches!(sa.status, SessionStatus::Idle))
                {
                    let child = Row::new(subagent_row(sa, state.tick))
                        .style(Style::default().fg(theme::TEXT_DIM));
                    rows.push(child);
                    logical_to_parent.push(i);
                }
            }
        }
    }

    // Find the visual row index that corresponds to the selected session
    let visual_selected = {
        let mut vis = 0;
        for (idx, &parent) in logical_to_parent.iter().enumerate() {
            if parent == state.table_state.selected
                && (idx == 0 || logical_to_parent[idx - 1] != parent)
            {
                vis = idx;
                break;
            }
        }
        vis
    };

    let table = Table::new(rows, &constraints)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER_DIM))
                .style(Style::default().bg(theme::BG)),
        )
        .column_spacing(1)
        .row_highlight_style(theme::selected_style());

    let mut ratatui_table_state =
        ratatui::widgets::TableState::default().with_selected(visual_selected);
    frame.render_stateful_widget(table, area, &mut ratatui_table_state);
}

impl TableView for SessionsTable {
    type Item = Session;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::flex("STATUS", 8, 16),
            ColumnDef::flex("NAME", 4, 30),
            ColumnDef::flex("PROJECT", 7, 25),
            ColumnDef::new("SUMMARY", 35),
            ColumnDef::flex("AGENTS", 4, 12),
            ColumnDef::flex("BRANCH", 6, 25),
            ColumnDef::flex("WORKTREE", 4, 20),
        ]
    }

    fn row_texts(item: &Session, tick: usize) -> Vec<String> {
        session_texts(item, tick)
    }

    fn row(item: &Session, tick: usize) -> Vec<Cell<'static>> {
        let texts = session_texts(item, tick);
        let (_, status_style) = format::status_cell(item.status, tick);

        vec![
            Cell::from(texts[0].clone()).style(status_style),
            Cell::from(texts[1].clone()).style(
                Style::default()
                    .fg(theme::CLAUDE_COLOR)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Cell::from(texts[2].clone()).style(Style::default().fg(theme::PATH_COLOR)),
            Cell::from(texts[3].clone()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(texts[4].clone()).style(Style::default().fg(theme::COUNT_COLOR)),
            Cell::from(texts[5].clone()).style(Style::default().fg(theme::BRANCH_COLOR)),
            Cell::from(texts[6].clone()).style(Style::default().fg(theme::ACCENT)),
        ]
    }

    fn items(state: &AppState) -> Vec<&Session> {
        state.filtered_sessions()
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("Enter", "View session details"),
            Keybinding::new("Tab", "Expand/collapse subagents"),
            Keybinding::new("i", "Inspect session details"),
            Keybinding::new("a", "Attach to session"),
            Keybinding::new("e", "Open in IDE"),
            Keybinding::new("o", "Open in new tab"),
            Keybinding::new("O", "Open ALL in new tabs"),
            Keybinding::new("c/n", "New session (prompts for dir, then name)"),
            Keybinding::new(":new <path>", "New session in <path>"),
            Keybinding::new("s", "Stash/unstash session"),
            Keybinding::new("A", "Toggle filter: active / all"),
            Keybinding::new("w", "Open in git worktree"),
            Keybinding::new("d", "Drop session (kill + unregister)"),
            Keybinding::new("D", "Drop ALL sessions"),
            Keybinding::new(":active", "Show active sessions"),
            Keybinding::new(":all", "Show all sessions"),
        ]
    }

    fn empty_message() -> &'static str {
        "No sessions. Press A to cycle filter, or c to start a new session."
    }
}
