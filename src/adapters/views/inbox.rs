use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Cell;

use crate::adapters::views::{ColumnDef, Keybinding, TableView};
use crate::application::state::AppState;
use crate::domain::entities::InboxMessage;

pub struct InboxTable;

impl TableView for InboxTable {
    type Item = InboxMessage;

    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef::new("FROM", 20),
            ColumnDef::new("TIME", 20),
            ColumnDef::new("MESSAGE", 55),
            ColumnDef::new("READ", 5),
        ]
    }

    fn row(item: &InboxMessage) -> Vec<Cell<'static>> {
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
        let style = if item.read {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        };

        vec![
            Cell::from(item.from.clone()).style(style),
            Cell::from(time).style(Style::default().fg(Color::DarkGray)),
            Cell::from(item.text.clone()).style(style),
            Cell::from(read_marker.to_string()).style(if item.read {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Yellow)
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
