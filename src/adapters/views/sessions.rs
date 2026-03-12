use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::adapters::format::{self, or_dash, short_id};
use crate::adapters::views::{ColumnDef, Keybinding, TableView, ViewKind};
use crate::application::actions::{Action, NavAction};
use crate::application::state::AppState;
use crate::domain::entities::{Session, SessionStatus, Subagent};
use crate::infrastructure::tui::theme;

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
fn subagent_row(sa: &Subagent) -> Vec<Cell<'static>> {
    let (status, style) = format::status_cell(sa.status);
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

    vec![
        Cell::from(format!("  {}", status)).style(style),
        Cell::from(format!("  └ {}", display_id)).style(Style::default().fg(theme::MUTED)),
        Cell::from(agent_type).style(Style::default().fg(theme::TEXT_DIM)),
        Cell::from(summary).style(Style::default().fg(theme::TEXT_DIM)),
        Cell::from("".to_string()),
        Cell::from("".to_string()),
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

    let constraints: Vec<Constraint> = columns
        .iter()
        .map(|c| Constraint::Percentage(c.width_pct))
        .collect();

    // Build rows: parent sessions + expanded child subagent rows
    let mut rows: Vec<Row> = Vec::new();
    let mut logical_to_parent: Vec<usize> = Vec::new(); // maps row index → parent session index

    for (i, session) in sessions.iter().enumerate() {
        let is_expanded = state.expanded_sessions.contains(&session.id);
        let expand_indicator = if session.subagent_count > 0 {
            if is_expanded {
                "▼ "
            } else {
                "▶ "
            }
        } else {
            "  "
        };

        // Get subagents for this session
        let subs = state.store.subagents_by_session.get(&session.id);
        let agents_text = if let Some(subs) = subs {
            agents_summary(subs)
        } else if session.subagent_count > 0 {
            format!("{}", session.subagent_count)
        } else {
            "—".to_string()
        };

        let (status, status_style) = format::status_cell(session.status);
        let sid = short_id(&session.id, 8);
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
        let branch = or_dash(&session.git_branch);

        let cells = vec![
            Cell::from(format!("{}{}", expand_indicator, status)).style(status_style),
            Cell::from(sid.to_string()).style(
                Style::default()
                    .fg(theme::CLAUDE_COLOR)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Cell::from(project_display.to_string()).style(Style::default().fg(theme::TEXT)),
            Cell::from(display_name.to_string()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(agents_text).style(Style::default().fg(theme::ACCENT)),
            Cell::from(branch.to_string()).style(Style::default().fg(theme::STATUS_WAITING)),
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
                    let child =
                        Row::new(subagent_row(sa)).style(Style::default().fg(theme::TEXT_DIM));
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
            ColumnDef::new("STATUS", 15),
            ColumnDef::new("SESSION", 12),
            ColumnDef::new("PROJECT", 20),
            ColumnDef::new("SUMMARY", 35),
            ColumnDef::new("AGENTS", 8),
            ColumnDef::new("BRANCH", 10),
        ]
    }

    fn row(item: &Session) -> Vec<Cell<'static>> {
        let (status, status_style) = format::status_cell(item.status);
        let sid = short_id(&item.id, 8).to_string();
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
        let branch = or_dash(&item.git_branch);

        vec![
            Cell::from(status).style(status_style),
            Cell::from(sid).style(
                Style::default()
                    .fg(theme::CLAUDE_COLOR)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Cell::from(project_display).style(Style::default().fg(theme::TEXT)),
            Cell::from(display_name.to_string()).style(Style::default().fg(theme::TEXT_DIM)),
            Cell::from(agents).style(Style::default().fg(theme::ACCENT)),
            Cell::from(branch.to_string()).style(Style::default().fg(theme::STATUS_WAITING)),
        ]
    }

    fn items(state: &AppState) -> Vec<&Session> {
        state.filtered_sessions()
    }

    fn on_select(item: &Session) -> Action {
        Action::Nav(NavAction::DrillIn {
            view: ViewKind::SessionDetail,
            context: item.id.clone(),
        })
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("Enter", "View session details"),
            Keybinding::new("Tab", "Expand/collapse subagents"),
            Keybinding::new("i", "Inspect session details"),
            Keybinding::new("a", "Attach to session"),
            Keybinding::new("c/n", "New session (prompts for dir)"),
            Keybinding::new(":new <path>", "New session in <path>"),
            Keybinding::new("A", "Toggle filter: active / all"),
            Keybinding::new("d", "Close and delete session"),
            Keybinding::new("D", "Close and delete ALL sessions"),
            Keybinding::new(":active", "Show active sessions"),
            Keybinding::new(":all", "Show all sessions"),
        ]
    }

    fn empty_message() -> &'static str {
        "No sessions. Press A to cycle filter, or c to start a new session."
    }
}
