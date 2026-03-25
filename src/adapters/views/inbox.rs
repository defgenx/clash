use ratatui::style::{Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::InboxMessage;
use crate::infrastructure::tui::theme;

pub struct InboxTable;

fn inbox_texts(item: &InboxMessage) -> Vec<String> {
    let time = item
        .timestamp
        .as_ref()
        .and_then(|t| match t {
            serde_json::Value::Number(n) => n.as_i64().and_then(|ms| {
                chrono::DateTime::from_timestamp_millis(ms)
                    .map(|dt| dt.format("%H:%M:%S").to_string())
            }),
            serde_json::Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.format("%H:%M:%S").to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "—".to_string());
    let read_marker = if item.read { "✓" } else { "●" };
    vec![
        item.from.clone(),
        time,
        item.text.clone(),
        read_marker.to_string(),
    ]
}

impl TableView for InboxTable {
    type Item = InboxMessage;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::flex("FROM", 4, 20),
            ColumnDef::flex("TIME", 8, 12),
            ColumnDef::new("MESSAGE", 60),
            ColumnDef::flex("READ", 3, 5),
        ]
    }

    fn row_texts(item: &InboxMessage, _tick: usize) -> Vec<String> {
        inbox_texts(item)
    }

    fn row(item: &InboxMessage, _tick: usize) -> Vec<Cell<'static>> {
        let texts = inbox_texts(item);
        let style = if item.read {
            Style::default().fg(theme::MUTED)
        } else {
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD)
        };

        vec![
            Cell::from(texts[0].clone()).style(style),
            Cell::from(texts[1].clone()).style(Style::default().fg(theme::MUTED)),
            Cell::from(texts[2].clone()).style(style),
            Cell::from(texts[3].clone()).style(if item.read {
                Style::default().fg(theme::MUTED)
            } else {
                Style::default().fg(theme::UNREAD_COLOR)
            }),
        ]
    }

    fn items(state: &AppState) -> Vec<&InboxMessage> {
        state.inbox_messages.iter().collect()
    }

    fn context_keybindings() -> Vec<Keybinding> {
        vec![Keybinding::new("m", "Send message")]
    }

    fn empty_message() -> &'static str {
        "No messages in inbox."
    }
}
