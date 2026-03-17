use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::adapters::views::TableView;
use crate::application::state::AppState;
use crate::infrastructure::tui::theme;

pub fn render_table<V: TableView>(state: &AppState, frame: &mut Frame, area: Rect) {
    let columns = V::columns();
    let items = V::items(state);

    if items.is_empty() {
        let empty = ratatui::widgets::Paragraph::new(V::empty_message())
            .style(theme::muted_style())
            .alignment(ratatui::layout::Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::BORDER_DIM))
                    .style(Style::default().bg(theme::BG)),
            );
        frame.render_widget(empty, area);
        return;
    }

    let header_cells: Vec<Cell> = columns
        .iter()
        .map(|c| Cell::from(c.name.clone()).style(theme::table_header_style()))
        .collect();
    let header = Row::new(header_cells).height(1);

    let constraints: Vec<Constraint> = columns
        .iter()
        .map(|c| Constraint::Percentage(c.width_pct))
        .collect();

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let cells = V::row(item, state.tick);
            let row = Row::new(cells);
            if i == state.table_state.selected {
                row.style(theme::selected_style())
            } else {
                row
            }
        })
        .collect();

    let highlight_style = theme::selected_style();

    let table = Table::new(rows, &constraints)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER_DIM))
                .style(Style::default().bg(theme::BG)),
        )
        .column_spacing(1)
        .row_highlight_style(highlight_style);

    let mut ratatui_table_state =
        ratatui::widgets::TableState::default().with_selected(state.table_state.selected);
    frame.render_stateful_widget(table, area, &mut ratatui_table_state);
}
