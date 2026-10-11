//! Inserting and deleting whole rows and columns across the workbook: the edited sheet's cells and
//! ranges, every formula that points at it, defined names and tables.

use open365_opc::{Relationships, TargetMode, rels_part_name, resolve_target};

use super::{CellInput, RawXml, Workbook, parts};
use crate::XlsxError;
use crate::structure::{EditAxis, FormulaShifter, StructEdit};
use crate::tables::{self, Restructured};

impl Workbook {
    /// Inserts or deletes `edit.count` whole rows or columns at `edit.at` on worksheet `sheet`, as Excel does:
    /// cells, row and column properties, merges, conditional formats, validation, hyperlinks, filters, tables
    /// and array ranges move or shrink; every formula in the workbook that points at the sheet is rewritten
    /// by `shifter`, defined names included. Cached values are left as they are: the caller recalculates.
    /// Returns the formula cells, at their new positions, whose result may have changed: formulas whose text
    /// was rewritten and formulas that moved (their `ROW()`/`COLUMN()` follow the cell).
    ///
    /// On an error the workbook may be partly changed; the caller keeps a copy to go back to.
    ///
    /// # Errors
    /// The sheet is not a worksheet, the span is outside the grid, or Excel would refuse the edit (it cuts an
    /// array formula, deletes a table's header row alone, or pushes cells off the grid).
    pub fn edit_structure(
        &mut self,
        sheet: usize,
        edit: StructEdit,
        shifter: &dyn FormulaShifter,
    ) -> Result<Vec<(usize, u32, u32)>, XlsxError> {
        if edit.count == 0 || edit.at == 0 || edit.end() > edit.limit() {
            return Err(XlsxError::UnsupportedEdit(format!(
                "rows or columns {}..={} are outside the grid",
                edit.at,
                edit.end()
            )));
        }
        self.worksheet_mut(sheet)?.check_structure(&edit)?;
        self.edited()?;

        let mut recalc: Vec<(usize, u32, u32)> = Vec::new();
        for (i, entry) in self.sheets.iter_mut().enumerate() {
            let Some(ws) = entry.data.as_mut() else {
                continue;
            };
            ws.materialise_all_shared();
            let mut changed = ws.shift_cell_formulas(shifter, &entry.info.name);
            if i == sheet {
                // Formulas that move: their position-dependent results change.
                let axis_of = |&(r, c): &(u32, u32)| match edit.axis {
                    EditAxis::Rows => r,
                    EditAxis::Cols => c,
                };
                changed.extend(
                    ws.formula_positions()
                        .into_iter()
                        .filter(|p| axis_of(p) >= edit.at),
                );
                changed.sort_unstable();
                changed.dedup();
                changed.retain(|p| edit.map_index(axis_of(p)).is_some());
                for (r, c) in changed {
                    let (r2, c2) = match edit.axis {
                        EditAxis::Rows => (edit.map_index(r).unwrap_or(r), c),
                        EditAxis::Cols => (r, edit.map_index(c).unwrap_or(c)),
                    };
                    recalc.push((i, r2, c2));
                }
            } else {
                recalc.extend(changed.into_iter().map(|(r, c)| (i, r, c)));
            }
            ws.shift_raw_formulas(shifter, &entry.info.name, (i == sheet).then_some(&edit));
            entry.dirty = true;
        }
        self.worksheet_mut(sheet)?.move_structure(&edit);

        self.restructure_tables(sheet, &edit, shifter)?;

        // Defined names: unqualified and relative references never move.
        for el in self
            .root
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .filter(|e| e.head.is_main("definedNames"))
        {
            for child in &mut el.children {
                if let RawXml::Element(name) = child
                    && name.head.is_main("definedName")
                    && let Some(new) = shifter.shift_name(name.own_text().trim_start_matches('='))
                {
                    name.children = vec![RawXml::Text(new)];
                }
            }
        }
        for name in &mut self.defined_names {
            if let Some(new) = shifter.shift_name(&name.formula) {
                name.formula = new;
            }
        }
        self.workbook_dirty = true;
        Ok(recalc)
    }

    /// Sets the cached result of an existing formula cell (or other cell of an array formula) without touching
    /// its formula.
    ///
    /// # Errors
    /// `sheet` is not a worksheet.
    pub fn set_cached_value(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        value: super::CellValue,
    ) -> Result<(), XlsxError> {
        if self.worksheet_mut(sheet)?.set_cached(row, col, value) {
            self.sheets[sheet].dirty = true;
        }
        Ok(())
    }

    fn restructure_tables(
        &mut self,
        sheet: usize,
        edit: &StructEdit,
        shifter: &dyn FormulaShifter,
    ) -> Result<(), XlsxError> {
        let mut headers: Vec<(u32, u32, String)> = Vec::new();
        for i in 0..self.sheets.len() {
            let home = self.sheets[i].info.name.clone();
            let sheet_part = self.sheets[i].part.clone();
            let Some(ws) = self.sheets[i].data.as_ref() else {
                continue;
            };
            let parts_list: Vec<(String, usize)> = ws
                .tables()
                .iter()
                .enumerate()
                .map(|(k, t)| (t.part.clone(), k))
                .collect();
            let mut removed: Vec<String> = Vec::new();
            for (part, _) in parts_list {
                let key = part.trim_start_matches('/').to_owned();
                let Some(bytes) = self.package.part(&key).map(|p| p.data().to_vec()) else {
                    continue;
                };
                let applies = (i == sheet).then_some(edit);
                match tables::restructure(&part, &bytes, applies, shifter, &home)? {
                    Restructured::Removed => removed.push(part),
                    Restructured::Updated {
                        bytes: new,
                        new_headers,
                    } => {
                        if new != bytes {
                            self.package
                                .set_part(open365_opc::PartName::new(&key)?, new.clone());
                            if let Some(t) = crate::tables::Table::parse(&part, &new)
                                && let Some(ws) = self.sheets[i].data.as_mut()
                                && let Some(slot) =
                                    ws.tables_mut().iter_mut().find(|x| x.part == part)
                            {
                                *slot = t;
                            }
                        }
                        headers.extend(new_headers);
                    }
                }
            }
            for part in removed {
                self.remove_table(i, &sheet_part, &part)?;
            }
        }
        for (row, col, name) in headers {
            self.set_cell_value(sheet, row, col, CellInput::Text(name))?;
        }
        Ok(())
    }

    /// Drops a table whose cells were all deleted: its part, relationship and `tablePart`.
    fn remove_table(
        &mut self,
        sheet: usize,
        sheet_part: &open365_opc::PartName,
        table_part: &str,
    ) -> Result<(), XlsxError> {
        let wanted = table_part.trim_start_matches('/');
        let rels_name = rels_part_name(Some(sheet_part));
        let rel_id = self
            .package
            .part(rels_name.as_str())
            .and_then(|p| Relationships::parse(p.data()).ok())
            .and_then(|rels| {
                rels.iter()
                    .filter(|r| r.target_mode == TargetMode::Internal)
                    .find(|r| {
                        resolve_target(Some(sheet_part), &r.target)
                            .is_ok_and(|t| t.as_str().trim_start_matches('/') == wanted)
                    })
                    .map(|r| r.id.clone())
            });
        if let Some(id) = rel_id {
            parts::remove_relationships(
                &mut self.package,
                Some(sheet_part),
                std::slice::from_ref(&id),
            )?;
            if let Some(ws) = self.sheets[sheet].data.as_mut() {
                ws.remove_table_part(&id);
            }
        }
        self.package.remove_part_and_override(wanted);
        if let Some(ws) = self.sheets[sheet].data.as_mut() {
            ws.retain_tables(|t| t.part.trim_start_matches('/') != wanted);
        }
        Ok(())
    }
}
