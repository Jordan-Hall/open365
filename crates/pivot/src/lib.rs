//! PivotTables: a lossless typed model of the three pivot parts and an Excel-exact
//! computation of the output grid.
//!
//! * [`PivotTable`], [`CacheDefinition`] and [`CacheRecords`] parse `pivotTableN.xml`,
//!   `pivotCacheDefinitionN.xml` and `pivotCacheRecordsN.xml`; unread markup stays in the
//!   XML tree and `to_bytes` writes it back.
//! * [`sheet_pivots`] lists the pivots of a worksheet with their cache.
//! * [`compute`] builds the [`PivotGrid`] from a [`Dataset`] (the stored cache or the source
//!   range read from the workbook); [`refresh_sheet`] writes it into the cells.
//!
//! See `docs/architecture/pivot-tables.md` for what is measured and what is not yet done.

pub mod calc;
mod calcitems;
mod compute;
mod error;
mod getpivotdata;
mod grouping;
mod location;
mod model;
mod regen;
mod showas;
pub mod style;
mod value;
mod xmlutil;

pub use compute::{
    AxisLine, DataCell, Dataset, GridCell, GridValue, LineInfo, LineKind, OrderMode, PivotGrid,
    PivotStrings, compute,
};
pub use error::PivotError;
pub use getpivotdata::{PivotIndex, get_pivot_data};
pub use location::Location;
pub use model::{
    Axis, CacheDefinition, CacheField, CacheRecords, CacheSource, CalculatedItem, DataField,
    FieldGroup, FieldItem, Func, PageField, PivotField, PivotTable, Record, VALUES_FIELD,
};
pub use value::{ItemKey, Value};

use open365_opc::{PartName, Relationships, rels_part_name, resolve_target};
use open365_xlsx::{CellInput, FormatPatch, NumberFormat, Workbook};

/// A pivot table of a worksheet with its cache.
#[derive(Debug, Clone)]
pub struct SheetPivot {
    pub part: PartName,
    pub table: PivotTable,
    /// The part of [`SheetPivot::cache`].
    pub cache_part: PartName,
    pub cache: CacheDefinition,
    pub records: Option<CacheRecords>,
    /// The part of [`SheetPivot::records`].
    pub records_part: Option<PartName>,
}

fn related(wb: &Workbook, source: &PartName, suffix: &str) -> Option<PartName> {
    let rels = Relationships::parse(
        wb.package()
            .part(rels_part_name(Some(source)).as_str())?
            .data(),
    )
    .ok()?;
    let rel = rels.iter().find(|r| r.rel_type.ends_with(suffix))?;
    resolve_target(Some(source), &rel.target).ok()
}

/// Reads every pivot table anchored on worksheet `sheet`. Parts that do not parse are skipped.
pub fn sheet_pivots(wb: &Workbook, sheet: usize) -> Vec<SheetPivot> {
    let d1904 = wb.date1904();
    compute::sheet_pivot_parts(wb, sheet)
        .into_iter()
        .filter_map(|part| {
            let table = PivotTable::parse(wb.package().part(part.as_str())?.data()).ok()?;
            let cache_part = related(wb, &part, "/pivotCacheDefinition")?;
            let cache =
                CacheDefinition::parse(wb.package().part(cache_part.as_str())?.data(), d1904)
                    .ok()?;
            let records_part = related(wb, &cache_part, "/pivotCacheRecords");
            let records = records_part
                .as_ref()
                .and_then(|p| wb.package().part(p.as_str()))
                .and_then(|p| CacheRecords::parse(p.data(), d1904).ok());
            Some(SheetPivot {
                part,
                table,
                cache_part,
                cache,
                records,
                records_part,
            })
        })
        .collect()
}

impl SheetPivot {
    /// The grid from the stored cache, items in the stored order (no source access).
    pub fn grid_from_cache(&self, strings: &PivotStrings) -> Result<PivotGrid, PivotError> {
        let records = self
            .records
            .as_ref()
            .ok_or(PivotError::MissingSource("pivotCacheRecords".into()))?;
        let data = Dataset::from_cache(&self.cache, records);
        compute(&self.table, &self.cache, &data, strings, OrderMode::Stored)
    }

    /// The grid after a refresh: source range re-read from the workbook, items re-sorted.
    pub fn grid_refreshed(
        &self,
        wb: &Workbook,
        strings: &PivotStrings,
    ) -> Result<PivotGrid, PivotError> {
        let data = Dataset::from_workbook(wb, &self.cache)?;
        compute(&self.table, &self.cache, &data, strings, OrderMode::Refresh)
    }
}

/// Recomputes every pivot of worksheet `sheet` from its source range, writes the values and
/// number formats into the cells (clearing the previous output area first) and writes the pivot
/// parts back into the workbook package: the pivot table part gets the new `location`, and the
/// cache definition gets `refreshOnLoad="1"` so Excel regenerates the cache records, shared
/// items and row/column item lists itself when it opens the saved file (this crate does not
/// regenerate those yet). Every other part stays byte-identical on [`Workbook::save`]. Returns
/// the number of pivots refreshed.
pub fn refresh_sheet(
    wb: &mut Workbook,
    sheet: usize,
    strings: &PivotStrings,
) -> Result<usize, PivotError> {
    let pivots = sheet_pivots(wb, sheet);
    let mut done = 0;
    for p in &pivots {
        let grid = p.grid_refreshed(wb, strings)?;
        if let Some((r1, c1, r2, c2)) = parse_ref(&p.table.reference) {
            let top = grid.range.0.min(r1);
            for r in top..=r2.max(grid.range.2) {
                for c in c1..=c2.max(grid.range.3) {
                    wb.set_cell_value(sheet, r, c, CellInput::Empty)
                        .map_err(|e| PivotError::Write(e.to_string()))?;
                }
            }
        }
        for c in &grid.cells {
            let input = match &c.value {
                GridValue::Text(t) => CellInput::Text(t.clone()),
                GridValue::Number(n) => CellInput::Number(*n),
                GridValue::Error(e) => CellInput::Text(e.clone()),
            };
            wb.set_cell_value(sheet, c.row, c.col, input)
                .map_err(|e| PivotError::Write(e.to_string()))?;
            if c.num_fmt_id != 0 {
                let patch = FormatPatch {
                    number_format: Some(NumberFormat::Builtin(c.num_fmt_id)),
                    ..FormatPatch::default()
                };
                wb.format_range(sheet, (c.row, c.col, c.row, c.col), &patch)
                    .map_err(|e| PivotError::Write(e.to_string()))?;
            }
        }
        let data = Dataset::from_workbook(wb, &p.cache)?;
        let shared_cache = pivots_on_cache(wb, &p.cache_part) > 1;
        let regenerated = if shared_cache {
            None
        } else {
            regen::regenerate(p, &data, &grid, now_serial())
        };
        match (regenerated, &p.records_part) {
            (Some(r), Some(records_part)) => {
                for (name, bytes) in [
                    (&p.part, r.table),
                    (&p.cache_part, r.cache),
                    (records_part, r.records),
                ] {
                    wb.replace_part(name, bytes)
                        .map_err(|e| PivotError::Write(e.to_string()))?;
                }
            }
            _ => {
                // Not regenerated: Excel rebuilds the cache and the item lists on load.
                let mut table = p.table.clone();
                table.set_location(&grid.location);
                let mut cache = p.cache.clone();
                cache.set_refresh_on_load(true);
                wb.replace_part(&p.part, table.to_bytes())
                    .map_err(|e| PivotError::Write(e.to_string()))?;
                wb.replace_part(&p.cache_part, cache.to_bytes())
                    .map_err(|e| PivotError::Write(e.to_string()))?;
            }
        }
        done += 1;
    }
    Ok(done)
}

/// Number of pivot tables of the workbook that use the cache part.
fn pivots_on_cache(wb: &Workbook, cache_part: &PartName) -> usize {
    (0..wb.sheets().len())
        .map(|i| {
            compute::sheet_pivot_parts(wb, i)
                .iter()
                .filter(|part| {
                    related(wb, part, "/pivotCacheDefinition").as_ref() == Some(cache_part)
                })
                .count()
        })
        .sum()
}

/// Now as a spreadsheet serial number (1900 date system).
fn now_serial() -> f64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    secs / 86_400.0 + 25_569.0
}

fn parse_ref(reference: &str) -> Option<(u32, u32, u32, u32)> {
    let plain: String = reference.chars().filter(|c| *c != '$').collect();
    open365_xlsx::parse_range(&plain)
}
