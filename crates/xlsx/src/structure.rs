//! Inserting and deleting whole rows and columns: the edit description, the index arithmetic
//! Excel applies to ranges, and the hook the formula engine plugs in to rewrite formulas.
//!
//! The arithmetic matches the calc crate's `refshift` (both were measured against Excel with
//! `tools/office/insdel-oracle.ps1`); it is repeated here because the workbook model has no
//! dependency on the formula engine.

use crate::refs::{MAX_COL, MAX_ROW};

/// Which dimension an edit works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAxis {
    /// Whole rows.
    Rows,
    /// Whole columns.
    Cols,
}

/// One insert or delete of whole rows or columns on a worksheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructEdit {
    /// Rows or columns.
    pub axis: EditAxis,
    /// True for a delete, false for an insert.
    pub delete: bool,
    /// First row or column affected (1-based).
    pub at: u32,
    /// How many rows or columns (at least 1).
    pub count: u32,
}

impl StructEdit {
    /// Last row or column of the grid on this axis.
    pub fn limit(&self) -> u32 {
        match self.axis {
            EditAxis::Rows => MAX_ROW,
            EditAxis::Cols => MAX_COL,
        }
    }

    /// Last row or column the edit covers (`at + count - 1`).
    pub fn end(&self) -> u32 {
        self.at.saturating_add(self.count).saturating_sub(1)
    }

    /// Where row or column `index` ends up; `None` when a delete removes it or an insert pushes it off the grid.
    pub fn map_index(&self, index: u32) -> Option<u32> {
        if !self.delete {
            if index < self.at {
                return Some(index);
            }
            let moved = index.checked_add(self.count)?;
            return (moved <= self.limit()).then_some(moved);
        }
        if index < self.at {
            Some(index)
        } else if index <= self.end() {
            None
        } else {
            Some(index - self.count)
        }
    }

    /// Where the range `lo..=hi` (`lo <= hi`) ends up; `None` when a delete removes all of it.
    pub fn map_range(&self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        let limit = self.limit();
        if !self.delete {
            return if lo >= self.at {
                let new_lo = lo.checked_add(self.count)?;
                (new_lo <= limit).then(|| (new_lo, hi.saturating_add(self.count).min(limit)))
            } else if hi >= self.at {
                Some((lo, hi.saturating_add(self.count).min(limit)))
            } else {
                Some((lo, hi))
            };
        }
        let end = self.end();
        if lo >= self.at && hi <= end {
            return None;
        }
        let new_lo = if lo < self.at {
            lo
        } else if lo <= end {
            self.at
        } else {
            lo - self.count
        };
        let new_hi = if hi < self.at {
            hi
        } else if hi <= end {
            self.at - 1
        } else {
            hi - self.count
        };
        Some((new_lo, new_hi))
    }

    /// The rectangle `(r1, c1, r2, c2)` after the edit; `None` when a delete removes all of it.
    pub fn map_rect(&self, rect: (u32, u32, u32, u32)) -> Option<(u32, u32, u32, u32)> {
        let (r1, c1, r2, c2) = rect;
        match self.axis {
            EditAxis::Rows => self.map_range(r1, r2).map(|(lo, hi)| (lo, c1, hi, c2)),
            EditAxis::Cols => self.map_range(c1, c2).map(|(lo, hi)| (r1, lo, r2, hi)),
        }
    }

    /// The edit's interval on its axis, `(at, at + count - 1)`.
    pub fn span(&self) -> (u32, u32) {
        (self.at, self.end())
    }
}

/// Rewrites the formulas of a workbook for a [`StructEdit`] of one named sheet. The formula engine
/// implements it; the workbook model only calls it.
pub trait FormulaShifter {
    /// `text` is a formula without `=` that lives on sheet `home`. Returns the rewritten text, or `None` when
    /// no reference in it changes (or it does not parse).
    fn shift(&self, text: &str, home: &str) -> Option<String>;

    /// As [`FormulaShifter::shift`] for the formula of a defined name: it lives on no sheet (unqualified
    /// references never match) and relative references keep their offset from `A1`.
    fn shift_name(&self, text: &str) -> Option<String>;

    /// As [`FormulaShifter::shift`] for a conditional format or validation formula on the edited sheet: its
    /// relative references are offsets from the top-left cell of the range it applies to, which moves by
    /// `delta` rows or columns along the edited axis.
    fn shift_anchored(&self, text: &str, home: &str, delta: i64) -> Option<String>;
}

/// Applies `edit` to the range written as A1 text (`C3`, `A1:B2`, `$A$1:$B$2`, `A:C`, `3:5`); `None` when the
/// range is deleted or the text is not a plain range. Anchors (`$`) are kept.
pub(crate) fn shift_range_text(text: &str, edit: &StructEdit) -> Option<String> {
    let (a, b) = match text.split_once(':') {
        Some((a, b)) => (a, Some(b)),
        None => (text, None),
    };
    let first = Corner::parse(a)?;
    let second = match b {
        Some(b) => Corner::parse(b)?,
        None => first.clone(),
    };
    // Whole-column and whole-row ranges are unaffected by the other axis.
    let whole_cols = first.row.is_none() && second.row.is_none();
    let whole_rows = first.col.is_none() && second.col.is_none();
    if (whole_cols && edit.axis == EditAxis::Rows) || (whole_rows && edit.axis == EditAxis::Cols) {
        return Some(text.to_owned());
    }
    let pick = |c: &Corner| match edit.axis {
        EditAxis::Rows => c.row.map(|r| r.0),
        EditAxis::Cols => c.col.map(|r| r.0),
    };
    let (lo, hi) = (pick(&first)?, pick(&second)?);
    let (flip, lo, hi) = if lo <= hi {
        (false, lo, hi)
    } else {
        (true, hi, lo)
    };
    let (new_lo, new_hi) = edit.map_range(lo, hi)?;
    let (mut c1, mut c2) = (first, second);
    if flip {
        std::mem::swap(&mut c1, &mut c2);
    }
    match edit.axis {
        EditAxis::Rows => {
            if let Some(r) = c1.row.as_mut() {
                r.0 = new_lo;
            }
            if let Some(r) = c2.row.as_mut() {
                r.0 = new_hi;
            }
        }
        EditAxis::Cols => {
            if let Some(c) = c1.col.as_mut() {
                c.0 = new_lo;
            }
            if let Some(c) = c2.col.as_mut() {
                c.0 = new_hi;
            }
        }
    }
    Some(if c1 == c2 {
        c1.write()
    } else {
        format!("{}:{}", c1.write(), c2.write())
    })
}

/// A parsed A1 corner: optional column and row, each with its `$` flag.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Corner {
    col: Option<(u32, bool)>,
    row: Option<(u32, bool)>,
}

impl Corner {
    fn parse(s: &str) -> Option<Self> {
        let first_dollar = s.starts_with('$');
        let rest = s.strip_prefix('$').unwrap_or(s);
        let letters = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let (col, row_abs, digits) = if letters > 0 {
            let col = crate::refs::col_number(&rest[..letters])?;
            let tail = &rest[letters..];
            let row_dollar = tail.starts_with('$');
            (
                Some((col, first_dollar)),
                row_dollar,
                tail.strip_prefix('$').unwrap_or(tail),
            )
        } else {
            (None, first_dollar, rest)
        };
        let row = if digits.is_empty() {
            None
        } else {
            if digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let r: u32 = digits.parse().ok()?;
            if !(1..=MAX_ROW).contains(&r) {
                return None;
            }
            Some((r, row_abs))
        };
        if col.is_none() && row.is_none() {
            return None;
        }
        Some(Self { col, row })
    }

    fn write(&self) -> String {
        let mut out = String::new();
        if let Some((c, abs)) = self.col {
            if abs {
                out.push('$');
            }
            out.push_str(&crate::refs::col_letters(c));
        }
        if let Some((r, abs)) = self.row {
            if abs {
                out.push('$');
            }
            out.push_str(&r.to_string());
        }
        out
    }
}

/// `(r1, c1, r2, c2)` as `A1` or `A1:C3`.
pub(crate) fn range_a1(rect: (u32, u32, u32, u32)) -> String {
    let (r1, c1, r2, c2) = rect;
    if (r1, c1) == (r2, c2) {
        crate::refs::cell_ref_string(r1, c1)
    } else {
        format!(
            "{}:{}",
            crate::refs::cell_ref_string(r1, c1),
            crate::refs::cell_ref_string(r2, c2)
        )
    }
}

/// How far the top-left cell of the first range of `sqref` moves along the edited axis.
pub(crate) fn anchor_delta(sqref: &str, edit: &StructEdit) -> i64 {
    let first = sqref.split_whitespace().next().unwrap_or_default();
    let (a, b) = first.split_once(':').unwrap_or((first, first));
    let (Some(c1), Some(c2)) = (Corner::parse(a), Corner::parse(b)) else {
        return 0;
    };
    let pick = |c: &Corner| match edit.axis {
        EditAxis::Rows => c.row.map(|r| r.0),
        EditAxis::Cols => c.col.map(|r| r.0),
    };
    let (Some(x), Some(y)) = (pick(&c1), pick(&c2)) else {
        return 0;
    };
    let (lo, hi) = (x.min(y), x.max(y));
    edit.map_range(lo, hi)
        .map_or(0, |(new_lo, _)| i64::from(new_lo) - i64::from(lo))
}

/// As [`shift_sqref`] for conditional formats and validation: an insert directly below (right of) a range
/// extends it, because Excel formats the inserted cells like the ones above (left of) them.
pub(crate) fn shift_sqref_formats(text: &str, edit: &StructEdit) -> Option<String> {
    if edit.delete || edit.at < 2 {
        return shift_sqref(text, edit);
    }
    let grown = StructEdit {
        at: edit.at - 1,
        ..*edit
    };
    let parts: Vec<String> = text
        .split_whitespace()
        .filter_map(|r| {
            let moved = shift_range_text(r, edit)?;
            // A range ending just above the insert point: shifting it as if inserted one row earlier
            // would grow it; detect by comparing with the grown version.
            let g = shift_range_text(r, &grown)?;
            let (a, b) = (parse_a1(r)?, parse_a1(&g)?);
            let ends_above = match edit.axis {
                EditAxis::Rows => a.2 == edit.at - 1,
                EditAxis::Cols => a.3 == edit.at - 1,
            };
            let _ = b;
            if ends_above {
                let rect = match edit.axis {
                    EditAxis::Rows => (a.0, a.1, a.2 + edit.count, a.3),
                    EditAxis::Cols => (a.0, a.1, a.2, a.3 + edit.count),
                };
                Some(range_a1(rect))
            } else {
                Some(moved)
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn parse_a1(text: &str) -> Option<(u32, u32, u32, u32)> {
    crate::refs::parse_range(&text.replace('$', ""))
}

/// Applies `edit` to a space-separated list of ranges (an `sqref`); `None` when nothing is left.
pub(crate) fn shift_sqref(text: &str, edit: &StructEdit) -> Option<String> {
    let parts: Vec<String> = text
        .split_whitespace()
        .filter_map(|r| shift_range_text(r, edit))
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(delete: bool, at: u32, count: u32) -> StructEdit {
        StructEdit {
            axis: EditAxis::Rows,
            delete,
            at,
            count,
        }
    }

    #[test]
    fn ranges_follow_excel() {
        let ins = rows(false, 5, 2);
        assert_eq!(ins.map_range(3, 9), Some((3, 11)));
        assert_eq!(ins.map_range(5, 9), Some((7, 11)));
        assert_eq!(ins.map_range(3, 4), Some((3, 4)));
        let del = rows(true, 4, 3);
        assert_eq!(del.map_range(4, 6), None);
        assert_eq!(del.map_range(3, 9), Some((3, 6)));
        assert_eq!(del.map_range(5, 9), Some((4, 6)));
        assert_eq!(del.map_range(2, 5), Some((2, 3)));
    }

    #[test]
    fn range_text_keeps_anchors() {
        let ins = rows(false, 2, 1);
        assert_eq!(
            shift_range_text("$C$3:$F$9", &ins).as_deref(),
            Some("$C$4:$F$10")
        );
        assert_eq!(shift_range_text("A:C", &ins).as_deref(), Some("A:C"));
        assert_eq!(shift_range_text("3:5", &ins).as_deref(), Some("4:6"));
        assert_eq!(shift_range_text("B2", &ins).as_deref(), Some("B3"));
        let del = rows(true, 2, 1);
        assert_eq!(shift_range_text("B2", &del), None);
        assert_eq!(shift_sqref("B2 D4:D6", &del).as_deref(), Some("D3:D5"));
    }
}
