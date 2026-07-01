//! Scratch-notes tree view — an IntelliJ-style "Scratches and Consoles" tree
//! of the user's free-form text files and folders.
//!
//! Rendering goes through [`render_scratch_table`] rather than the generic
//! `table::render_table`, because the view is a tree: rows are indented by
//! depth, folders carry an expand/collapse caret, and entries under a
//! collapsed folder are hidden. The `TableView` impl below still defines the
//! columns, the help-overlay keybindings, and the empty message; its
//! `row`/`row_texts`/`items` methods exist only to satisfy the trait and are
//! bypassed by the custom renderer (same pattern as `SessionsTable::row()`,
//! per CLAUDE.md).

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::{visible_scratch_indices, AppState};
use crate::domain::entities::ScratchNote;
use crate::infrastructure::tui::theme;
use crate::infrastructure::tui::widgets::table::compute_constraints;

pub struct ScratchTable;

/// Render the `NAME` column text for a tree row: depth indentation, a caret
/// for folders (`▾` expanded / `▸` collapsed), and a trailing `/` on folders.
/// Files align their name under the folder name (two leading spaces where the
/// caret would be).
fn tree_name(item: &ScratchNote, expanded: bool) -> String {
    let indent = "  ".repeat(item.depth);
    if item.is_dir {
        let caret = if expanded { "▾" } else { "▸" };
        format!("{indent}{caret} {}/", item.title)
    } else {
        format!("{indent}  {}", item.title)
    }
}

/// Tree-aware renderer for the Scratch view (replaces the generic table path).
/// Walks the visible rows (collapsed subtrees hidden), indents by depth, and
/// highlights the selection — the selection index is into the *visible* list.
pub fn render_scratch_table(state: &AppState, frame: &mut Frame, area: Rect) {
    let visible = visible_scratch_indices(&state.store.scratch_notes, &state.expanded_scratch_dirs);

    if visible.is_empty() {
        let empty = Paragraph::new(ScratchTable::empty_message())
            .style(theme::muted_style())
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::BORDER_DIM))
                    .style(Style::default().bg(theme::BG)),
            );
        frame.render_widget(empty, area);
        return;
    }

    let columns = ScratchTable::columns();
    let header = Row::new(
        columns
            .iter()
            .map(|c| Cell::from(c.name.clone()).style(theme::table_header_style()))
            .collect::<Vec<_>>(),
    )
    .height(1);

    let mut content_rows: Vec<Vec<String>> = Vec::with_capacity(visible.len());
    let mut rows: Vec<Row> = Vec::with_capacity(visible.len());
    for (vis_i, &note_i) in visible.iter().enumerate() {
        let n = &state.store.scratch_notes[note_i];
        let expanded = state.expanded_scratch_dirs.contains(&n.id);
        let name = tree_name(n, expanded);
        let modified = relative_time(n.updated_at);
        // Size is meaningless for folders.
        let size = if n.is_dir {
            String::new()
        } else {
            human_size(n.size)
        };
        content_rows.push(vec![name.clone(), modified.clone(), size.clone()]);

        let name_style = if n.is_dir {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };
        let row = Row::new(vec![
            Cell::from(name).style(name_style),
            Cell::from(modified).style(Style::default().fg(theme::MUTED)),
            Cell::from(size).style(Style::default().fg(theme::MUTED)),
        ]);
        rows.push(if vis_i == state.table_state.selected {
            row.style(theme::selected_style())
        } else {
            row
        });
    }

    let constraints = compute_constraints(&columns, &content_rows, area.width);
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

    let mut ts = ratatui::widgets::TableState::default().with_selected(state.table_state.selected);
    frame.render_stateful_widget(table, area, &mut ts);
}

/// Format a byte count compactly (e.g. `12 B`, `3.4 KB`).
fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Format an epoch-millis timestamp as a short relative time (e.g. `5m ago`).
/// Returns `—` when the timestamp is missing (0).
fn relative_time(updated_at_ms: i64) -> String {
    if updated_at_ms <= 0 {
        return "—".to_string();
    }
    let now = chrono::Utc::now().timestamp_millis();
    let secs = (now - updated_at_ms).max(0) / 1000;
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 86_400 * 30 {
        format!("{}d ago", secs / 86_400)
    } else {
        chrono::DateTime::from_timestamp_millis(updated_at_ms)
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_else(|| "—".to_string())
    }
}

fn note_texts(item: &ScratchNote) -> Vec<String> {
    vec![
        item.title.clone(),
        relative_time(item.updated_at),
        human_size(item.size),
    ]
}

impl TableView for ScratchTable {
    type Item = ScratchNote;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("NAME", 60),
            ColumnDef::flex("MODIFIED", 8, 14),
            ColumnDef::flex("SIZE", 6, 10),
        ]
    }

    // `row_texts`/`row`/`items` satisfy the `TableView` contract but are
    // bypassed by `render_scratch_table` (the tree renderer). Kept for the
    // trait obligation, mirroring `SessionsTable::row()` (see CLAUDE.md).
    fn row_texts(item: &ScratchNote, _tick: usize) -> Vec<String> {
        note_texts(item)
    }

    fn row(item: &ScratchNote, _tick: usize) -> Vec<Cell<'static>> {
        let texts = note_texts(item);
        vec![
            Cell::from(texts[0].clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(texts[1].clone()).style(Style::default().fg(theme::MUTED)),
            Cell::from(texts[2].clone()).style(Style::default().fg(theme::MUTED)),
        ]
    }

    fn items(state: &AppState) -> Vec<&ScratchNote> {
        state.store.scratch_notes.iter().collect()
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![
            Keybinding::new("a / c / n", "New scratch"),
            Keybinding::new("A", "New folder"),
            Keybinding::new("Enter", "Open file / toggle folder"),
            Keybinding::new("e", "Open in editor"),
            Keybinding::new("r", "Rename"),
            Keybinding::new("m", "Move to another folder"),
            Keybinding::new("y", "Copy path (absolute / relative / name)"),
            Keybinding::new("d", "Delete"),
        ]
    }

    fn empty_message() -> &'static str {
        "No scratches. Press 'a' for a new note, 'A' for a folder."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_human_size() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
    }

    #[test]
    fn test_relative_time_zero_is_dash() {
        assert_eq!(relative_time(0), "—");
    }

    #[test]
    fn test_relative_time_recent() {
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(relative_time(now), "just now");
        assert_eq!(relative_time(now - 120_000), "2m ago");
    }
}
