use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::adapters::format::{self, or_dash};
use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::application::store::DataStore;
use crate::domain::entities::{Session, SessionSection, SessionStatus, Subagent};
use crate::infrastructure::tui::{theme, widgets::table::compute_constraints};
use ratatui::style::Modifier;

pub struct SessionsTable;

impl SessionsTable {
    pub fn has_items(state: &AppState) -> bool {
        !state.filtered_sessions().is_empty()
    }
}

/// Build the AGENTS column text from subagents: shows compact status summary.
fn agents_summary_refs(subagents: &[&Subagent]) -> String {
    if subagents.is_empty() {
        return "—".to_string();
    }
    let (mut thinking, mut running, mut prompting) = (0u16, 0u16, 0u16);
    for sa in subagents {
        match sa.status {
            SessionStatus::Thinking => thinking += 1,
            SessionStatus::Running => running += 1,
            SessionStatus::Prompting => prompting += 1,
            _ => {}
        }
    }
    let active = (thinking + running + prompting) as usize;
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
    if active == 0 {
        if total > 0 {
            format!("{}", total)
        } else {
            "—".to_string()
        }
    } else {
        format!("{}/{} ({})", active, total, parts.join(" "))
    }
}

/// Compute the AGENTS column text for a session using the subagent store.
///
/// This is the single source of truth for agents text — used by both
/// column measurement and cell rendering to prevent width mismatches.
fn compute_agents_text(session: &Session, store: &DataStore) -> String {
    let subs = store.subagents_by_session.get(&session.id);
    let active_subs: Option<Vec<_>> = subs.map(|s| {
        s.iter()
            .filter(|sa| sa.status != SessionStatus::Done)
            .collect()
    });
    if let Some(ref active) = active_subs {
        if active.is_empty() {
            "—".to_string()
        } else {
            agents_summary_refs(active)
        }
    } else if session.subagent_count > 0 {
        format!("{}", session.subagent_count)
    } else {
        "—".to_string()
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
        Some(name) => format::worktree_display(name, sa.worktree_project.as_deref()),
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
///
/// When `agents_override` is `Some`, that text is used for the AGENTS column
/// (the custom renderer pre-computes it via `compute_agents_text`). When `None`,
/// falls back to the simple subagent count (used by the generic `TableView` path).
fn session_texts(item: &Session, tick: usize, agents_override: Option<&str>) -> Vec<String> {
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
    let agents = match agents_override {
        Some(text) => text.to_string(),
        None => {
            if item.subagent_count > 0 {
                format!("{}", item.subagent_count)
            } else {
                "—".to_string()
            }
        }
    };
    let branch = item
        .source_branch
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&item.git_branch);
    let branch = or_dash(branch).to_string();
    let worktree_display = match &item.worktree {
        Some(name) => format::worktree_display(name, item.worktree_project.as_deref()),
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
pub fn render_sessions_table(
    state: &AppState,
    visual_state: &mut ratatui::widgets::TableState,
    frame: &mut Frame,
    area: Rect,
) {
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

    // Pre-compute agents text per session for measurement + rendering
    let agents_texts: Vec<String> = sessions
        .iter()
        .map(|s| compute_agents_text(s, &state.store))
        .collect();

    // Measure content for dynamic column sizing (with correct agents text)
    let content_rows: Vec<Vec<String>> = sessions
        .iter()
        .enumerate()
        .map(|(i, s)| session_texts(s, state.tick, Some(&agents_texts[i])))
        .collect();
    let constraints = compute_constraints(&columns, &content_rows, area.width);

    // Count sessions per section for header labels
    let mut section_counts: std::collections::HashMap<SessionSection, usize> =
        std::collections::HashMap::new();
    for s in &sessions {
        *section_counts.entry(s.status.section()).or_insert(0) += 1;
    }

    // Section header overlay info: (row_index, label, style)
    let mut section_overlays: Vec<(usize, String, Style)> = Vec::new();

    // Build rows: parent sessions + expanded child subagent rows + section headers
    let mut rows: Vec<Row> = Vec::new();
    let mut logical_to_parent: Vec<usize> = Vec::new(); // maps row index → parent session index
    let mut current_section: Option<SessionSection> = None;

    for (i, session) in sessions.iter().enumerate() {
        let section = session.status.section();
        // Insert section header when entering a new section
        if current_section != Some(section) {
            let count = section_counts.get(&section).copied().unwrap_or(0);
            let section_color = match section {
                SessionSection::Active => theme::SECTION_ACTIVE,
                SessionSection::Done => theme::SECTION_DONE,
                SessionSection::Fail => theme::SECTION_FAIL,
            };
            let icon = match section {
                SessionSection::Active => "◆",
                SessionSection::Done => "◇",
                SessionSection::Fail => "✦",
            };
            let label = format!(" {} {} ({}) ", icon, section.label(), count);
            let style = Style::default()
                .fg(section_color)
                .add_modifier(Modifier::BOLD);

            // Track for overlay rendering after the table is drawn
            section_overlays.push((rows.len(), label, style));

            // Empty placeholder row in the table (will be overwritten by overlay)
            let mut empty_cells = Vec::with_capacity(columns.len());
            for _ in 0..columns.len() {
                empty_cells.push(Cell::from(""));
            }
            rows.push(Row::new(empty_cells));
            logical_to_parent.push(usize::MAX); // sentinel — not selectable
            current_section = Some(section);
        }
        let is_expanded = state.expanded_sessions.contains(&session.id);

        // Get subagents for this session (excluding Done)
        let subs = state.store.subagents_by_session.get(&session.id);
        let active_subs: Option<Vec<_>> = subs.map(|s| {
            s.iter()
                .filter(|sa| sa.status != crate::domain::entities::SessionStatus::Done)
                .collect()
        });

        let has_subs = active_subs.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
        let expand_indicator = if has_subs {
            if is_expanded {
                "▼ "
            } else {
                "▶ "
            }
        } else {
            "  "
        };
        let agents_text = agents_texts[i].clone();

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
            Some(name) => format::worktree_display(name, session.worktree_project.as_deref()),
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

        // Add child rows if expanded (Done already filtered out)
        if is_expanded {
            if let Some(ref active) = active_subs {
                for sa in active {
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

    // Update selection on the persisted visual state — preserves scroll offset across frames.
    *visual_state = visual_state.clone().with_selected(visual_selected);
    frame.render_stateful_widget(table, area, visual_state);

    // Overlay full-width section headers on top of the empty placeholder rows.
    // The table has: 1 border top + 1 header row = offset 2 from area.y.
    // TableState::offset() tells us the scroll position.
    let scroll_offset = visual_state.offset();
    let body_y = area.y + 2; // border + column header
    let body_height = area.height.saturating_sub(3) as usize; // border top + header + border bottom
    let inner_width = area.width.saturating_sub(2); // left + right borders

    for &(row_idx, ref label, style) in &section_overlays {
        if row_idx < scroll_offset {
            continue;
        }
        let visible_row = row_idx - scroll_offset;
        if visible_row >= body_height {
            continue;
        }
        let y = body_y + visible_row as u16;

        // Build the line: label + rule filling the rest of the width
        let label_len = label.chars().count() as u16;
        let rule_len = inner_width.saturating_sub(label_len);
        let rule: String = "─".repeat(rule_len as usize);

        let line = ratatui::text::Line::from(vec![
            ratatui::text::Span::styled(label.clone(), style),
            ratatui::text::Span::styled(rule, Style::default().fg(theme::SEPARATOR)),
        ]);

        let overlay_area = Rect {
            x: area.x + 1, // inside left border
            y,
            width: inner_width,
            height: 1,
        };
        frame.render_widget(ratatui::widgets::Paragraph::new(line), overlay_area);
    }
}

impl TableView for SessionsTable {
    type Item = Session;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::flex("STATUS", 15, 18),
            ColumnDef::flex("NAME", 4, 30),
            ColumnDef::flex("PROJECT", 7, 25),
            ColumnDef::new("SUMMARY", 35),
            ColumnDef::flex("AGENTS", 4, 22),
            ColumnDef::flex("BRANCH", 6, 25),
            ColumnDef::flex("WORKTREE", 4, 20),
        ]
    }

    fn row_texts(item: &Session, tick: usize) -> Vec<String> {
        session_texts(item, tick, None)
    }

    fn row(item: &Session, tick: usize) -> Vec<Cell<'static>> {
        let texts = session_texts(item, tick, None);
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
            Keybinding::new("S", "Stash/unstash ALL sessions"),
            Keybinding::new("A", "Cycle section filter"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{SessionStatus, Subagent};

    fn make_subagent(status: SessionStatus) -> Subagent {
        Subagent {
            status,
            ..Default::default()
        }
    }

    // ── agents_summary_refs tests ────────────────────────────────────

    #[test]
    fn agents_summary_empty() {
        let subs: Vec<&Subagent> = vec![];
        assert_eq!(agents_summary_refs(&subs), "—");
    }

    #[test]
    fn agents_summary_all_idle() {
        let s1 = make_subagent(SessionStatus::Stashed);
        let s2 = make_subagent(SessionStatus::Stashed);
        let s3 = make_subagent(SessionStatus::Stashed);
        let subs: Vec<&Subagent> = vec![&s1, &s2, &s3];
        assert_eq!(agents_summary_refs(&subs), "3");
    }

    #[test]
    fn agents_summary_single_prompting() {
        let s1 = make_subagent(SessionStatus::Prompting);
        let subs: Vec<&Subagent> = vec![&s1];
        assert_eq!(agents_summary_refs(&subs), "1/1 (1!)");
    }

    #[test]
    fn agents_summary_running_only() {
        let s1 = make_subagent(SessionStatus::Running);
        let s2 = make_subagent(SessionStatus::Stashed);
        let s3 = make_subagent(SessionStatus::Stashed);
        let subs: Vec<&Subagent> = vec![&s1, &s2, &s3];
        assert_eq!(agents_summary_refs(&subs), "1/3 (1●)");
    }

    #[test]
    fn agents_summary_mixed_active() {
        let s1 = make_subagent(SessionStatus::Prompting);
        let s2 = make_subagent(SessionStatus::Thinking);
        let s3 = make_subagent(SessionStatus::Stashed);
        let s4 = make_subagent(SessionStatus::Stashed);
        let s5 = make_subagent(SessionStatus::Stashed);
        let subs: Vec<&Subagent> = vec![&s1, &s2, &s3, &s4, &s5];
        assert_eq!(agents_summary_refs(&subs), "2/5 (1! 1◎)");
    }

    #[test]
    fn agents_summary_all_active_types() {
        let s1 = make_subagent(SessionStatus::Prompting);
        let s2 = make_subagent(SessionStatus::Thinking);
        let s3 = make_subagent(SessionStatus::Running);
        let subs: Vec<&Subagent> = vec![&s1, &s2, &s3];
        assert_eq!(agents_summary_refs(&subs), "3/3 (1! 1◎ 1●)");
    }

    // ── measurement agreement test ───────────────────────────────────

    #[test]
    fn measurement_matches_rendering() {
        let session = Session {
            id: "test-session".to_string(),
            subagent_count: 3,
            ..Default::default()
        };

        let s1 = make_subagent(SessionStatus::Thinking);
        let s2 = make_subagent(SessionStatus::Running);
        let s3 = make_subagent(SessionStatus::Stashed);

        let mut store = DataStore::default();
        store
            .subagents_by_session
            .insert("test-session".to_string(), vec![s1, s2, s3]);

        let agents_text = compute_agents_text(&session, &store);
        let texts = session_texts(&session, 0, Some(&agents_text));

        // Index 4 is the AGENTS column
        assert_eq!(texts[4], agents_text);
        // The actual text should be the full summary, not just the count
        assert_eq!(texts[4], "2/3 (1◎ 1●)");
    }

    #[test]
    fn measurement_fallback_without_override() {
        let session = Session {
            subagent_count: 5,
            ..Default::default()
        };

        let texts = session_texts(&session, 0, None);
        // Without override, falls back to simple count
        assert_eq!(texts[4], "5");
    }

    #[test]
    fn compute_agents_text_no_subagents() {
        let session = Session::default();
        let store = DataStore::default();
        assert_eq!(compute_agents_text(&session, &store), "—");
    }

    #[test]
    fn compute_agents_text_with_count_no_store_data() {
        let session = Session {
            subagent_count: 7,
            ..Default::default()
        };
        let store = DataStore::default();
        // No subagents in store → falls back to count
        assert_eq!(compute_agents_text(&session, &store), "7");
    }
}
