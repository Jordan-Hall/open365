//! The `location` element of a pivot table definition: the table's range and the offsets
//! Excel writes next to it.

use open365_xlsx::cell_ref_string;

/// `location/@ref` and the offsets of the header and data areas, as Excel writes them.
///
/// `reference` is the table body (page fields excluded). The offsets are relative to its
/// top-left cell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Location {
    pub reference: String,
    pub first_header_row: u32,
    pub first_data_row: u32,
    pub first_data_col: u32,
    /// Number of page fields stacked in one column.
    pub row_page_count: u32,
    /// Number of page-field columns (1 when there are page fields).
    pub col_page_count: u32,
}

impl Location {
    /// Range string (`B4:C10`) of a bounding box (1-based first row, col, last row, col).
    pub fn range_string(body: (u32, u32, u32, u32)) -> String {
        let (r1, c1, r2, c2) = body;
        if (r1, c1) == (r2, c2) {
            cell_ref_string(r1, c1)
        } else {
            format!("{}:{}", cell_ref_string(r1, c1), cell_ref_string(r2, c2))
        }
    }
}
