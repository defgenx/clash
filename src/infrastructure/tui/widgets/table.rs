use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::adapters::views::{ColumnSizing, TableView};
use crate::application::state::AppState;
use crate::infrastructure::tui::theme;

/// Compute constraints for columns, measuring content for Flex columns.
pub fn compute_constraints(
    columns: &[crate::adapters::views::ColumnDef],
    content_rows: &[Vec<String>],
    available_width: u16,
) -> Vec<Constraint> {
    // Subtract border (2) + column spacing between columns
    let spacing = if columns.len() > 1 {
        columns.len() as u16 - 1
    } else {
        0
    };
    let usable = available_width.saturating_sub(2).saturating_sub(spacing);

    let has_flex = columns
        .iter()
        .any(|c| matches!(c.sizing, ColumnSizing::Flex { .. }));

    if !has_flex {
        // All percentage — legacy path
        return columns
            .iter()
            .map(|c| Constraint::Percentage(c.width_pct))
            .collect();
    }

    // Measure max content width per column (including header)
    let mut measured: Vec<u16> = columns.iter().map(|c| c.name.len() as u16).collect();
    for row in content_rows {
        for (i, cell_text) in row.iter().enumerate() {
            if i < measured.len() {
                measured[i] = measured[i].max(cell_text.len() as u16);
            }
        }
    }

    // First pass: allocate fixed-pct columns and clamp flex columns
    let mut widths: Vec<u16> = Vec::with_capacity(columns.len());
    let mut pct_total: u16 = 0;
    let mut flex_indices: Vec<usize> = Vec::new();

    for (i, col) in columns.iter().enumerate() {
        match col.sizing {
            ColumnSizing::Pct(p) => {
                pct_total += p;
                widths.push(0); // resolved later
            }
            ColumnSizing::Flex { min, max } => {
                let clamped = measured[i].clamp(min, max);
                widths.push(clamped);
                flex_indices.push(i);
            }
        }
    }

    // Calculate space taken by flex columns
    let flex_total: u16 = flex_indices.iter().map(|&i| widths[i]).sum();

    // Remaining space goes to pct columns (proportionally)
    let remaining = usable.saturating_sub(flex_total);

    for (i, col) in columns.iter().enumerate() {
        if let ColumnSizing::Pct(p) = col.sizing {
            if pct_total > 0 {
                widths[i] = (remaining as u32 * p as u32 / pct_total as u32) as u16;
            }
        }
    }

    widths.iter().map(|&w| Constraint::Length(w)).collect()
}

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

    // Measure content for dynamic sizing
    let content_rows: Vec<Vec<String>> = items
        .iter()
        .map(|item| V::row_texts(item, state.tick))
        .collect();

    let constraints = compute_constraints(&columns, &content_rows, area.width);

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
