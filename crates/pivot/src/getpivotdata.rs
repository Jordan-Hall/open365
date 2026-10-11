//! GETPIVOTDATA evaluation hook.
//!
//! `open365-calc` does not depend on this crate yet. When it does, the worksheet function
//! `GETPIVOTDATA(data_field, pivot_table, [field1, item1], ...)` is evaluated like this:
//!
//! 1. Collect a [`PivotIndex`] per pivot of the workbook (`PivotIndex::build` over
//!    `sheet_pivots`), keyed by sheet; the index is a snapshot of the displayed grid and must be
//!    rebuilt after the pivot or its source changes.
//! 2. Find the index whose [`PivotIndex::contains`] holds for the sheet and cell of the
//!    `pivot_table` argument (any cell of the table range, page fields included). No index is
//!    `#REF!`.
//! 3. Call [`get_pivot_data`] with the data field text and the `(field, item)` pairs, items
//!    converted to [`Value`] (text, number, bool). The result is a number, text or error to
//!    return from the function; every miss is `#REF!`.

use crate::compute::{DataCell, GridValue, PivotGrid};
use crate::model::{Axis, CacheDefinition, PivotTable};
use crate::value::{ItemKey, Value};
use crate::{PivotStrings, SheetPivot};

/// A snapshot of one pivot table's displayed values.
#[derive(Debug, Clone)]
pub struct PivotIndex {
    table: PivotTable,
    cache: CacheDefinition,
    grid: PivotGrid,
}

impl PivotIndex {
    /// Snapshot from the stored cache (items in the stored order).
    pub fn build(pivot: &SheetPivot, strings: &PivotStrings) -> Option<Self> {
        Some(Self {
            table: pivot.table.clone(),
            cache: pivot.cache.clone(),
            grid: pivot.grid_from_cache(strings).ok()?,
        })
    }

    /// Whether the sheet cell lies in the table (page fields included).
    pub fn contains(&self, row: u32, col: u32) -> bool {
        let (r1, c1, r2, c2) = self.grid.range;
        (r1..=r2).contains(&row) && (c1..=c2).contains(&col)
    }

    pub fn grid(&self) -> &PivotGrid {
        &self.grid
    }

    fn data_index(&self, name: &str) -> Option<usize> {
        let want = name.to_lowercase();
        let fields = &self.table.data_fields;
        fields
            .iter()
            .position(|d| d.name.to_lowercase() == want)
            .or_else(|| {
                fields.iter().position(|d| {
                    self.cache
                        .fields
                        .get(d.fld as usize)
                        .is_some_and(|f| f.name.to_lowercase() == want)
                })
            })
    }

    /// The field is on the row or column axis and has an item with this key.
    fn item_exists(&self, field: usize, key: &ItemKey) -> bool {
        let on_axis = self
            .table
            .fields
            .get(field)
            .is_some_and(|f| matches!(f.axis, Some(Axis::Row | Axis::Col)));
        on_axis
            && self
                .cache
                .fields
                .get(field)
                .is_some_and(|f| f.item_values().iter().any(|v| ItemKey::of(v) == *key))
    }

    fn field_index(&self, name: &str) -> Option<usize> {
        let want = name.to_lowercase();
        self.cache
            .fields
            .iter()
            .position(|f| f.name.to_lowercase() == want)
    }
}

/// `GETPIVOTDATA` over one table. `Err` carries the error code to return (always `#REF!`).
pub fn get_pivot_data(
    index: &PivotIndex,
    data_field: &str,
    pairs: &[(String, Value)],
) -> Result<GridValue, &'static str> {
    let di = index.data_index(data_field).ok_or("#REF!")?;
    let mut want: Vec<(usize, ItemKey)> = Vec::new();
    for (field, item) in pairs {
        let fi = index.field_index(field).ok_or("#REF!")?;
        let key = ItemKey::of(item);
        match want.iter().find(|(f, _)| *f == fi) {
            Some((_, k)) if *k != key => return Err("#REF!"),
            Some(_) => {}
            None => want.push((fi, key)),
        }
    }
    let matches = |c: &DataCell| -> bool {
        c.data == di
            && c.items.len() == want.len()
            && want.iter().all(|(f, k)| {
                c.items
                    .iter()
                    .any(|(cf, cv)| cf == f && ItemKey::of(cv) == *k)
            })
    };
    let Some(cell) = index.grid.data_cells.iter().find(|c| matches(c)) else {
        // The named fields form a displayed combination and every item exists, but the
        // intersection has no value: Excel returns 0.
        let shown = index.grid.data_cells.iter().any(|c| {
            c.data == di
                && c.items.len() == want.len()
                && want
                    .iter()
                    .all(|(f, _)| c.items.iter().any(|(cf, _)| cf == f))
        });
        return if !want.is_empty() && shown && want.iter().all(|(f, k)| index.item_exists(*f, k)) {
            Ok(GridValue::Number(0.0))
        } else {
            Err("#REF!")
        };
    };
    index
        .grid
        .cells
        .iter()
        .find(|g| g.row == cell.row && g.col == cell.col)
        .map(|g| g.value.clone())
        .ok_or("#REF!")
}
