//! Sheets Home tab operations that rewrite worksheet structures: Clear All, Sort, and the
//! AutoFilter switch. Each runs as one undo step (the caller keeps a workbook copy).

use open365_xlsx::{
    AutoFilter, HeaderMode, SortKey, SortOrientation, SortSpec, Workbook, XlsxError,
};
use serde::{Deserialize, Serialize};

use crate::command::{CellEntry, CellRef};
use crate::error::CommandError;
use crate::format::RangeRef;
use crate::xlsx_state::XlsxState;

/// One level of a sort: the absolute 1-based column that holds the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortLevel {
    pub column: u32,
    #[serde(default)]
    pub descending: bool,
}

/// What a [`crate::Command::HomeOp`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HomeOp {
    /// Clear > Clear All: contents, formats and merges of the range.
    ClearAll { range: RangeRef },
    /// Sort & Filter > Sort A to Z / Z to A / Custom Sort. `header` `None` lets Excel guess.
    /// `locale` is a BCP-47 tag (en-US when absent).
    Sort {
        range: RangeRef,
        levels: Vec<SortLevel>,
        #[serde(default)]
        header: Option<bool>,
        #[serde(default)]
        locale: Option<String>,
    },
    /// Sort & Filter > Filter: switches the AutoFilter of the sheet on over `range` (header row
    /// first) or, with `None`, off and shows every row again.
    SetAutoFilter { range: Option<RangeRef> },
    /// Sort & Filter > Reapply. `today` is the date serial the date filters use.
    ReapplyFilter {
        #[serde(default)]
        locale: Option<String>,
        today: f64,
    },
    /// Sort & Filter > Clear: drops the criteria of every column and shows all rows.
    ClearFilter {
        #[serde(default)]
        locale: Option<String>,
        today: f64,
    },
}

impl HomeOp {
    /// Whether the operation discards content that exists.
    pub(crate) fn destroys(&self) -> bool {
        matches!(self, Self::ClearAll { .. })
    }
}

fn locale_of(tag: Option<&str>) -> open365_locale::Locale {
    tag.and_then(|t| open365_locale::Locale::from_bcp47(t).ok())
        .unwrap_or_else(open365_locale::Locale::en_us)
}

fn bad(e: XlsxError) -> CommandError {
    CommandError::Xlsx(e)
}

/// Runs `op`; on an error the caller puts its workbook copy back.
pub(crate) fn run(
    x: &mut XlsxState,
    sheet: usize,
    op: &HomeOp,
) -> Result<Vec<CellRef>, CommandError> {
    let mut changed = Vec::new();
    match op {
        HomeOp::ClearAll { range } => {
            let rect = range.rect();
            let (r1, c1, r2, c2) = rect;
            let cells: Vec<(u32, u32)> = x
                .workbook
                .iter_cells(sheet)
                .filter(|(r, c, cell)| {
                    (r1..=r2).contains(r)
                        && (c1..=c2).contains(c)
                        && (!matches!(cell.value, open365_xlsx::CellValue::Empty)
                            || cell.formula.is_some())
                })
                .map(|(r, c, _)| (r, c))
                .collect();
            x.workbook
                .merge_cells(sheet, rect, open365_xlsx::MergeKind::Unmerge)
                .map_err(bad)?;
            for (r, c) in cells {
                changed.extend(x.set_value(sheet, r, c, &CellEntry::Empty)?);
            }
            x.workbook.clear_formats(sheet, rect).map_err(bad)?;
        }
        HomeOp::Sort {
            range,
            levels,
            header,
            locale,
        } => {
            let spec = SortSpec {
                range: range.rect(),
                header: match header {
                    Some(true) => HeaderMode::Yes,
                    Some(false) => HeaderMode::No,
                    None => HeaderMode::Guess,
                },
                orientation: SortOrientation::TopToBottom,
                case_sensitive: false,
                method: open365_xlsx::SortMethod::Default,
                keys: levels
                    .iter()
                    .map(|l| SortKey::value(l.column, l.descending))
                    .collect(),
            };
            x.workbook
                .sort_range(sheet, &spec, &locale_of(locale.as_deref()))
                .map_err(bad)?;
            reset_engine(x);
        }
        HomeOp::SetAutoFilter { range } => match range {
            Some(r) => {
                x.workbook
                    .set_auto_filter(sheet, Some(&AutoFilter::new(r.rect())))
                    .map_err(bad)?;
            }
            None => {
                if let Some(f) = x.workbook.sheet(sheet).and_then(|w| w.auto_filter()) {
                    let (r1, _, r2, _) = f.range;
                    x.workbook.set_auto_filter(sheet, None).map_err(bad)?;
                    x.workbook.unhide_rows(sheet, r1, r2).map_err(bad)?;
                }
            }
        },
        HomeOp::ReapplyFilter { locale, today } => {
            x.workbook
                .apply_auto_filter(sheet, *today, &locale_of(locale.as_deref()))
                .map_err(bad)?;
        }
        HomeOp::ClearFilter { locale, today } => {
            if let Some(mut f) = x.workbook.sheet(sheet).and_then(|w| w.auto_filter()) {
                f.columns.clear();
                x.workbook.set_auto_filter(sheet, Some(&f)).map_err(bad)?;
                x.workbook
                    .apply_auto_filter(sheet, *today, &locale_of(locale.as_deref()))
                    .map_err(bad)?;
            }
        }
    }
    Ok(changed)
}

/// The calculation engine is rebuilt from the workbook on the next edit.
fn reset_engine(x: &mut XlsxState) {
    let wb: Workbook = x.workbook.clone();
    x.restore_workbook(wb);
}
