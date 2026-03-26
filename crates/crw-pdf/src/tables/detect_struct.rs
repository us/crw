//! Structure-tree-based table detection.
//!
//! When a PDF has a well-formed structure tree with `/Table` > `/TR` > `/TD|TH`
//! elements linked to MCIDs, this module builds `Table` structs directly from
//! the semantic hierarchy — no geometry heuristics needed.

use std::collections::HashMap;

use log::debug;

use crate::structure_tree::StructTable;
use crate::types::TextItem;

use super::Table;

/// Build tables from structure-tree table descriptors by matching MCIDs to TextItems.
///
/// Returns tables for the given page.  Tables where fewer than 50% of cells
/// resolve to TextItems are rejected (stale or broken structure tree).
pub fn detect_tables_from_struct_tree(
    items: &[TextItem],
    struct_tables: &[StructTable],
    page: u32,
) -> Vec<Table> {
    if struct_tables.is_empty() {
        return Vec::new();
    }

    // Build MCID → item indices for this page
    let mut mcid_to_items: HashMap<i64, Vec<usize>> = HashMap::new();
    for (idx, item) in items.iter().enumerate() {
        if item.page == page {
            if let Some(mcid) = item.mcid {
                mcid_to_items.entry(mcid).or_default().push(idx);
            }
        }
    }

    let mut tables = Vec::new();

    for st in struct_tables {
        // Filter rows to this page
        let page_rows: Vec<_> = st
            .rows
            .iter()
            .filter(|row| {
                row.cells
                    .iter()
                    .any(|cell| cell.mcids.iter().any(|&(_, p)| p == page))
            })
            .collect();

        debug!(
            "page {}: struct table has {} rows on this page (from {} total)",
            page,
            page_rows.len(),
            st.rows.len()
        );

        if page_rows.len() < 2 {
            continue;
        }

        // Determine column count from max cells per row
        let num_cols = page_rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
        if num_cols < 2 {
            continue;
        }

        // Build cell text and collect item indices
        let mut cells: Vec<Vec<String>> = Vec::new();
        let mut all_item_indices: Vec<usize> = Vec::new();
        let mut total_cells = 0u32;
        let mut matched_cells = 0u32;

        for row in &page_rows {
            let mut row_cells = Vec::with_capacity(num_cols);
            for (col_idx, cell) in row.cells.iter().enumerate() {
                if col_idx >= num_cols {
                    break;
                }
                total_cells += 1;

                // Collect all items for this cell's MCIDs
                let mut cell_items: Vec<(usize, &TextItem)> = Vec::new();
                for &(mcid, p) in &cell.mcids {
                    if p == page {
                        if let Some(indices) = mcid_to_items.get(&mcid) {
                            for &idx in indices {
                                cell_items.push((idx, &items[idx]));
                            }
                        }
                    }
                }

                if !cell_items.is_empty() {
                    matched_cells += 1;
                }

                // Sort by Y (descending = top-to-bottom) then X
                cell_items.sort_by(|a, b| {
                    b.1.y
                        .partial_cmp(&a.1.y)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(
                            a.1.x
                                .partial_cmp(&b.1.x)
                                .unwrap_or(std::cmp::Ordering::Equal),
                        )
                });

                let text: String = cell_items
                    .iter()
                    .map(|(_, item)| item.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");

                for (idx, _) in &cell_items {
                    all_item_indices.push(*idx);
                }

                row_cells.push(text);
            }

            // Pad to num_cols
            while row_cells.len() < num_cols {
                row_cells.push(String::new());
            }
            cells.push(row_cells);
        }

        // Reject if too few cells matched (stale structure tree)
        let coverage = if total_cells > 0 {
            matched_cells as f32 / total_cells as f32
        } else {
            0.0
        };
        debug!(
            "page {}: struct table {}x{}, {}/{} cells matched ({:.0}%)",
            page,
            page_rows.len(),
            num_cols,
            matched_cells,
            total_cells,
            coverage * 100.0
        );
        if total_cells == 0 || coverage < 0.3 {
            continue;
        }

        // Derive row/column positions from item geometry
        let mut row_positions: Vec<f32> = Vec::new();
        for row in &page_rows {
            let y = row
                .cells
                .iter()
                .flat_map(|c| c.mcids.iter())
                .filter(|(_, p)| *p == page)
                .filter_map(|(mcid, _)| mcid_to_items.get(mcid))
                .flatten()
                .map(|&idx| items[idx].y)
                .reduce(f32::max)
                .unwrap_or(0.0);
            row_positions.push(y);
        }

        // Column positions: use X positions of first non-empty cell in each column
        let mut col_positions: Vec<f32> = vec![0.0; num_cols];
        for (col, col_pos) in col_positions.iter_mut().enumerate() {
            for row in &page_rows {
                if col < row.cells.len() {
                    if let Some(x) = row.cells[col]
                        .mcids
                        .iter()
                        .filter(|(_, p)| *p == page)
                        .filter_map(|(mcid, _)| mcid_to_items.get(mcid))
                        .flatten()
                        .map(|&idx| items[idx].x)
                        .reduce(f32::min)
                    {
                        *col_pos = x;
                        break;
                    }
                }
            }
        }

        all_item_indices.sort_unstable();
        all_item_indices.dedup();

        tables.push(Table {
            columns: col_positions,
            rows: row_positions,
            cells,
            item_indices: all_item_indices,
        });
    }

    tables
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure_tree::{StructTableCell, StructTableRow};
    use crate::types::ItemType;

    fn make_item(text: &str, x: f32, y: f32, page: u32, mcid: Option<i64>) -> TextItem {
        TextItem {
            text: text.to_string(),
            x,
            y,
            width: text.len() as f32 * 5.0,
            height: 10.0,
            font: "Test".to_string(),
            font_size: 10.0,
            page,
            is_bold: false,
            is_italic: false,
            item_type: ItemType::Text,
            mcid,
        }
    }

    #[test]
    fn basic_struct_table() {
        let items = vec![
            make_item("Name", 50.0, 700.0, 1, Some(10)),
            make_item("Age", 200.0, 700.0, 1, Some(11)),
            make_item("Alice", 50.0, 680.0, 1, Some(20)),
            make_item("30", 200.0, 680.0, 1, Some(21)),
            make_item("Bob", 50.0, 660.0, 1, Some(30)),
            make_item("25", 200.0, 660.0, 1, Some(31)),
        ];

        let struct_tables = vec![StructTable {
            rows: vec![
                StructTableRow {
                    cells: vec![
                        StructTableCell {
                            is_header: true,
                            mcids: vec![(10, 1)],
                        },
                        StructTableCell {
                            is_header: true,
                            mcids: vec![(11, 1)],
                        },
                    ],
                },
                StructTableRow {
                    cells: vec![
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(20, 1)],
                        },
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(21, 1)],
                        },
                    ],
                },
                StructTableRow {
                    cells: vec![
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(30, 1)],
                        },
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(31, 1)],
                        },
                    ],
                },
            ],
        }];

        let tables = detect_tables_from_struct_tree(&items, &struct_tables, 1);
        assert_eq!(tables.len(), 1);
        let table = &tables[0];
        assert_eq!(table.cells.len(), 3);
        assert_eq!(table.cells[0], vec!["Name", "Age"]);
        assert_eq!(table.cells[1], vec!["Alice", "30"]);
        assert_eq!(table.cells[2], vec!["Bob", "25"]);
        assert_eq!(table.item_indices.len(), 6);
    }

    #[test]
    fn rejects_low_mcid_coverage() {
        // Items have no MCIDs matching the struct table
        let items = vec![
            make_item("Orphan", 50.0, 700.0, 1, Some(999)),
            make_item("Text", 200.0, 700.0, 1, None),
        ];

        let struct_tables = vec![StructTable {
            rows: vec![
                StructTableRow {
                    cells: vec![
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(10, 1)],
                        },
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(11, 1)],
                        },
                    ],
                },
                StructTableRow {
                    cells: vec![
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(20, 1)],
                        },
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(21, 1)],
                        },
                    ],
                },
            ],
        }];

        let tables = detect_tables_from_struct_tree(&items, &struct_tables, 1);
        assert!(
            tables.is_empty(),
            "should reject table with no MCID matches"
        );
    }

    #[test]
    fn filters_by_page() {
        let items = vec![
            make_item("A", 50.0, 700.0, 2, Some(10)),
            make_item("B", 200.0, 700.0, 2, Some(11)),
            make_item("C", 50.0, 680.0, 2, Some(20)),
            make_item("D", 200.0, 680.0, 2, Some(21)),
        ];

        let struct_tables = vec![StructTable {
            rows: vec![
                StructTableRow {
                    cells: vec![
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(10, 2)],
                        },
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(11, 2)],
                        },
                    ],
                },
                StructTableRow {
                    cells: vec![
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(20, 2)],
                        },
                        StructTableCell {
                            is_header: false,
                            mcids: vec![(21, 2)],
                        },
                    ],
                },
            ],
        }];

        // Page 1 should find nothing
        let tables = detect_tables_from_struct_tree(&items, &struct_tables, 1);
        assert!(tables.is_empty());

        // Page 2 should find the table
        let tables = detect_tables_from_struct_tree(&items, &struct_tables, 2);
        assert_eq!(tables.len(), 1);
    }
}
