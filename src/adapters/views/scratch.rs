//! Scratch-notes table view — lists the user's free-form text notes.

use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::ScratchNote;
use crate::infrastructure::tui::theme;

pub struct ScratchTable;

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
            Keybinding::new("c / n", "New scratch"),
            Keybinding::new("Enter / e", "Open in editor"),
            Keybinding::new("d", "Delete scratch"),
        ]
    }

    fn empty_message() -> &'static str {
        "No scratches. Press 'c' to create one."
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
