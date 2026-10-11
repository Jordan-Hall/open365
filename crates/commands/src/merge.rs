//! Excel's Merge & Center menu: Merge & Center, Merge Across, Merge Cells and Unmerge Cells.

use open365_xlsx::{
    AlignmentPatch, FormatPatch, HorizontalAlignment, MergeKind, Workbook, merge_areas,
};
use serde::{Deserialize, Serialize};

use crate::command::{CellEntry, CellRef};
use crate::error::CommandError;
use crate::format::RangeRef;
use crate::xlsx_state::XlsxState;

/// The four items of the Merge & Center drop-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMode {
    /// Merge the range into one cell and centre it horizontally.
    MergeCenter,
    /// Merge each row of the range on its own.
    Across,
    /// Merge the range into one cell, alignment unchanged.
    Cells,
    /// Split every merged cell that touches the range. Values and alignment stay.
    Unmerge,
}

impl MergeMode {
    fn kind(self) -> MergeKind {
        match self {
            Self::MergeCenter | Self::Cells => MergeKind::Merge,
            Self::Across => MergeKind::Across,
            Self::Unmerge => MergeKind::Unmerge,
        }
    }
}

/// The non-blank cells other than the upper-left one of each area that `mode` would empty. Excel
/// shows "Merging cells only keeps the upper-left value and discards other values." when this is
/// not empty; the UI asks before it runs the command.
///
/// # Errors
/// The range is outside the sheet or the sheet is not a worksheet.
pub fn merge_discards(
    workbook: &Workbook,
    sheet: usize,
    range: RangeRef,
    mode: MergeMode,
) -> Result<Vec<(u32, u32)>, CommandError> {
    Ok(workbook.merge_discards(sheet, range.rect(), mode.kind())?)
}

/// The merged areas `mode` would create inside `range`.
#[must_use]
pub fn merge_plan(range: RangeRef, mode: MergeMode) -> Vec<RangeRef> {
    merge_areas(range.rect(), mode.kind())
        .into_iter()
        .map(|(first_row, first_col, last_row, last_col)| RangeRef {
            first_row,
            first_col,
            last_row,
            last_col,
        })
        .collect()
}

/// Runs the merge: empties the discarded cells (recalculating dependents), then edits `mergeCells`
/// and, for Merge & Center, centres the range. The caller keeps a workbook copy for undo; on an
/// error it must put it back.
pub(crate) fn run(
    x: &mut XlsxState,
    sheet: usize,
    range: RangeRef,
    mode: MergeMode,
) -> Result<Vec<CellRef>, CommandError> {
    let rect = range.rect();
    let mut changed = Vec::new();
    // Validates the range before anything is emptied.
    let discarded = x.workbook.merge_discards(sheet, rect, mode.kind())?;
    x.workbook.merge_cells(sheet, rect, mode.kind())?;
    for (row, col) in discarded {
        changed.extend(x.set_value(sheet, row, col, &CellEntry::Empty)?);
    }
    if mode == MergeMode::MergeCenter {
        let patch = FormatPatch {
            alignment: AlignmentPatch {
                horizontal: Some(HorizontalAlignment::Center),
                ..AlignmentPatch::default()
            },
            ..FormatPatch::default()
        };
        x.workbook.format_range(sheet, rect, &patch)?;
    }
    Ok(changed)
}
