//! Insert and delete cells on a partial range, shifting the neighbours right, down, left or up,
//! with every formula reference rewritten.
//!
//! The reference rules are those of whole-row insertion restricted to the band of columns (or rows)
//! the shift covers: a reference moves when it lies wholly inside the band; an area inside the band
//! that spans the edit point grows or shrinks; a reference that leaves the band is not touched.

use open365_calc::Ast;
use open365_calc::ast::{AreaKind, AreaRef, CellRef};

use super::LeafShifter;

// Drawing objects follow the cells they are anchored to (insert, delete, resize, hide). The
// modules live beside this file and are declared here so `write/mod.rs` stays untouched.
#[path = "drawing_edit.rs"]
mod drawing_edit;
#[path = "drawing.rs"]
mod drawing_ops;
#[path = "drawing_shift.rs"]
mod drawing_shift;
#[path = "drawing_xml.rs"]
mod drawing_xml;
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW};
use crate::sheet::{CellShift, Rect, RectFate};
use crate::structure::range_a1;
use crate::tables::Table;
use crate::workbook::Workbook;
use crate::xml::{self, RawXml};

/// Which way the cells beside the range move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftDirection {
    /// Insert cells; the cells of the range and to its right move right.
    Right,
    /// Insert cells; the cells of the range and below move down.
    Down,
    /// Delete cells; the cells to the right move left.
    Left,
    /// Delete cells; the cells below move up.
    Up,
}

impl ShiftDirection {
    fn shift(self, rect: Rect) -> CellShift {
        let (r1, c1, r2, c2) = rect;
        match self {
            Self::Down | Self::Up => CellShift {
                rows_axis: true,
                band: (c1, c2),
                at: r1,
                count: r2 - r1 + 1,
                delete: self == Self::Up,
            },
            Self::Right | Self::Left => CellShift {
                rows_axis: false,
                band: (r1, r2),
                at: c1,
                count: c2 - c1 + 1,
                delete: self == Self::Left,
            },
        }
    }

    /// True for the deleting directions.
    pub fn deletes(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }
}

fn dead(sheet: Option<&String>) -> Ast {
    match sheet {
        Some(s) => Ast::DeadRef { sheet: s.clone() },
        None => Ast::Error(open365_calc::ErrorKind::Ref),
    }
}

fn leaf(node: &Ast, home: Option<&str>, target: &str, s: &CellShift) -> Option<Ast> {
    let points_here = |q: Option<&String>| {
        q.map(String::as_str)
            .or(home)
            .is_some_and(|n| n.eq_ignore_ascii_case(target))
    };
    match node {
        Ast::Ref(r) if points_here(r.sheet.as_ref()) => {
            let (along, perp) = if s.rows_axis {
                (r.row, r.col)
            } else {
                (r.col, r.row)
            };
            if !s.in_band(perp) {
                return None;
            }
            let moved = s.map_index(along).filter(|n| *n <= s.limit());
            match moved {
                Some(n) if n == along => None,
                Some(n) => {
                    let mut out: CellRef = r.clone();
                    if s.rows_axis {
                        out.row = n;
                    } else {
                        out.col = n;
                    }
                    Some(Ast::Ref(out))
                }
                None => Some(dead(r.sheet.as_ref())),
            }
        }
        Ast::Area(a) if a.kind == AreaKind::Cells && points_here(a.sheet.as_ref()) => {
            let mut out: AreaRef = a.clone();
            if out.row1 > out.row2 {
                std::mem::swap(&mut out.row1, &mut out.row2);
                std::mem::swap(&mut out.row1_abs, &mut out.row2_abs);
            }
            if out.col1 > out.col2 {
                std::mem::swap(&mut out.col1, &mut out.col2);
                std::mem::swap(&mut out.col1_abs, &mut out.col2_abs);
            }
            let (along, perp) = if s.rows_axis {
                ((out.row1, out.row2), (out.col1, out.col2))
            } else {
                ((out.col1, out.col2), (out.row1, out.row2))
            };
            if perp.0 < s.band.0 || perp.1 > s.band.1 {
                return None;
            }
            let Some((lo, hi)) = s.map_range(along.0, along.1) else {
                return Some(dead(a.sheet.as_ref()));
            };
            if hi > s.limit() {
                return Some(dead(a.sheet.as_ref()));
            }
            if (lo, hi) == along && out == *a {
                return None;
            }
            if s.rows_axis {
                out.row1 = lo;
                out.row2 = hi;
            } else {
                out.col1 = lo;
                out.col2 = hi;
            }
            Some(Ast::Area(out))
        }
        _ => None,
    }
}

impl Workbook {
    /// Insert Cells or Delete Cells on `rect` (`(row1, col1, row2, col2)`), shifting the cells beside it.
    /// Merges, conditional formats, validations, hyperlinks, array ranges, tables and every formula
    /// that points at the cells move with them. Cached values are left; the caller recalculates the
    /// returned formula cells (at their new positions).
    ///
    /// Excel's refusals are kept: the shift may not cut a merged area, an array formula or a table,
    /// or push cells off the grid.
    ///
    /// # Errors
    /// The sheet is not a worksheet, `rect` is outside the grid, or Excel would refuse the shift.
    pub fn shift_cells(
        &mut self,
        sheet: usize,
        rect: Rect,
        direction: ShiftDirection,
    ) -> Result<Vec<(usize, u32, u32)>, XlsxError> {
        let (r1, c1, r2, c2) = rect;
        if r1 == 0 || c1 == 0 || r1 > r2 || c1 > c2 || r2 > MAX_ROW || c2 > MAX_COL {
            return Err(XlsxError::UnsupportedEdit(
                "the cells are outside the worksheet".to_owned(),
            ));
        }
        let shift = direction.shift(rect);
        self.worksheet_mut(sheet)?.check_cell_shift(&shift)?;
        self.edited()?;
        let drawings = self.drawing_snapshot(sheet);
        let target = self.sheets[sheet].info.name.clone();

        let shifter = LeafShifter {
            leaf: move |n: &Ast, home: Option<&str>| leaf(n, home, &target, &shift),
        };
        let mut recalc = self.rewrite_all_formulas(&shifter);

        // Formula cells that move: their ROW()/COLUMN() results follow them.
        let movers: Vec<(u32, u32)> = self
            .worksheet_mut(sheet)?
            .formula_positions()
            .into_iter()
            .filter(|&(r, c)| {
                let (along, perp) = if shift.rows_axis { (r, c) } else { (c, r) };
                shift.in_band(perp) && along >= shift.at
            })
            .collect();
        recalc.extend(movers.into_iter().map(|(r, c)| (sheet, r, c)));
        let map_pos = |(i, r, c): (usize, u32, u32)| -> Option<(usize, u32, u32)> {
            if i != sheet {
                return Some((i, r, c));
            }
            let (along, perp) = if shift.rows_axis { (r, c) } else { (c, r) };
            if !shift.in_band(perp) {
                return Some((i, r, c));
            }
            let n = shift.map_index(along)?;
            Some(if shift.rows_axis {
                (i, n, c)
            } else {
                (i, r, n)
            })
        };
        let mut recalc: Vec<(usize, u32, u32)> = recalc.into_iter().filter_map(map_pos).collect();
        recalc.sort_unstable();
        recalc.dedup();

        let removed = self.worksheet_mut(sheet)?.apply_cell_shift(&shift);
        self.sheets[sheet].dirty = true;
        if removed > 0
            && let Some((_, sst)) = self.sst.as_mut()
        {
            sst.adjust_count(-i64::try_from(removed).unwrap_or(0));
            self.sst_dirty = true;
        }
        self.move_tables(sheet, &shift)?;
        if let Some(snap) = drawings {
            self.drawing_reflow(
                sheet,
                snap,
                Some(drawing_shift::AxisShift {
                    rows: shift.rows_axis,
                    band: Some(shift.band),
                    at: shift.at,
                    count: shift.count,
                    delete: shift.delete,
                }),
            )?;
        }
        Ok(recalc)
    }

    /// Updates the table parts whose range moved wholly with a shift.
    fn move_tables(&mut self, sheet: usize, s: &CellShift) -> Result<(), XlsxError> {
        let tables: Vec<Table> = self.sheets[sheet]
            .data
            .as_ref()
            .map(|ws| ws.tables().to_vec())
            .unwrap_or_default();
        for t in tables {
            let Some(old) = t.rect else { continue };
            // The cells have moved; the table text still has the old range.
            let RectFate::Moved(new) = s.fate(old) else {
                continue;
            };
            let key = t.part.trim_start_matches('/').to_owned();
            let Some(bytes) = self.part_bytes(&key).map(<[u8]>::to_vec) else {
                continue;
            };
            let mut parsed = xml::parse(&bytes)?;
            parsed.root.head.set_attr("ref", &range_a1(new));
            let (dr, dc) = (
                i64::from(new.0) - i64::from(old.0),
                i64::from(new.1) - i64::from(old.1),
            );
            for child in &mut parsed.root.children {
                if let RawXml::Element(e) = child
                    && e.head.is_main("autoFilter")
                    && let Some(af) = e.head.attr("ref").and_then(crate::refs::parse_range)
                {
                    let sh = |v: u32, d: i64| u32::try_from(i64::from(v) + d).unwrap_or(v);
                    let moved = (sh(af.0, dr), sh(af.1, dc), sh(af.2, dr), sh(af.3, dc));
                    e.head.set_attr("ref", &range_a1(moved));
                }
            }
            let out = parsed.to_bytes();
            self.put_part_bytes(&key, out.clone())?;
            if let Some(model) = Table::parse(&t.part, &out)
                && let Some(ws) = self.sheets[sheet].data.as_mut()
                && let Some(slot) = ws.tables_mut().iter_mut().find(|x| x.part == t.part)
            {
                *slot = model;
            }
        }
        Ok(())
    }
}
