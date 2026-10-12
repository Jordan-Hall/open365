//! Sheet operations: insert, delete, rename, move and copy a sheet, tab colour, hide and unhide.

use std::collections::BTreeSet;

use open365_calc::Ast;
use open365_opc::{PartName, TargetMode, resolve_target};

use super::{LeafShifter, R_NS, XML_DECL, content_type, rel, relative_target};
use crate::XlsxError;
use crate::format::{ColorSource, Rgb};
use crate::sheet::Worksheet;
use crate::tables::Table;
use crate::workbook::{SheetEntry, SheetInfo, SheetKind, SheetState, Workbook};
use crate::xml::{self, Attr, RawXml, XmlElement};

/// The colour of a sheet tab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabColor {
    /// No colour (the default tab).
    None,
    Rgb(Rgb),
    Theme {
        index: u32,
        tint: f64,
    },
    Indexed(u32),
}

const BLANK_SHEET: &str = "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><dimension ref=\"A1\"/><sheetViews><sheetView workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><sheetData/><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>";

/// Sheet name text as a formula writes it: `Sheet1!` or `'My sheet'!`.
fn quoted_prefix(name: &str) -> String {
    let probe = open365_calc::to_formula_string(&Ast::Ref(open365_calc::ast::CellRef {
        sheet: Some(name.to_owned()),
        row: 1,
        col: 1,
        row_abs: false,
        col_abs: false,
    }));
    probe.strip_suffix("A1").unwrap_or(&probe).to_owned()
}

fn rels_source_of(rels: &str) -> Option<Option<String>> {
    if rels.eq_ignore_ascii_case("_rels/.rels") {
        return Some(None);
    }
    let (dir, file) = rels.rsplit_once('/')?;
    let stem = file.strip_suffix(".rels")?;
    let parent = dir.strip_suffix("_rels")?.trim_end_matches('/');
    Some(Some(if parent.is_empty() {
        stem.to_owned()
    } else {
        format!("{parent}/{stem}")
    }))
}

impl Workbook {
    fn sheet_elements(&self) -> Vec<XmlElement> {
        self.root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("sheets"))
            .map(|s| {
                s.elements()
                    .filter(|e| e.head.is_main("sheet"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn write_sheet_elements(&mut self, els: Vec<XmlElement>) {
        let mut container = self
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("sheets"))
            .cloned()
            .unwrap_or_else(|| self.root.sibling("sheets"));
        container.children = els.into_iter().map(RawXml::Element).collect();
        self.put_workbook_child("sheets", Some(container));
        self.infos = self.sheets.iter().map(|s| s.info.clone()).collect();
    }

    fn workbook_view_mut(&mut self) -> Option<&mut XmlElement> {
        self.root
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("bookViews"))?
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("workbookView"))
    }

    /// The active tab (`activeTab`, 0 when absent).
    pub fn active_sheet(&self) -> usize {
        self.root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("bookViews"))
            .and_then(|b| b.child_main("workbookView"))
            .and_then(|v| v.head.attr("activeTab"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    }

    fn set_active_tab_index(&mut self, index: usize) {
        if let Some(v) = self.workbook_view_mut() {
            if index == 0 {
                v.head.remove_attr("activeTab");
            } else {
                v.head.set_attr("activeTab", &index.to_string());
            }
        }
        self.workbook_dirty = true;
    }

    fn first_visible_index(&self) -> Option<usize> {
        self.sheets
            .iter()
            .position(|s| s.info.state == SheetState::Visible)
    }

    /// Checks a sheet name against Excel's rules; `ignore` is the sheet being renamed.
    ///
    /// # Errors
    /// The name is empty, longer than 31 characters, uses `\ / ? * [ ] :`, begins or ends with an
    /// apostrophe, is `History`, or is taken by another sheet (case-insensitively).
    pub fn validate_sheet_name(&self, name: &str, ignore: Option<usize>) -> Result<(), XlsxError> {
        let bad = |why: &str| {
            Err(XlsxError::UnsupportedEdit(format!(
                "the sheet name {name:?} {why}"
            )))
        };
        if name.trim().is_empty() {
            return bad("is blank");
        }
        if name.chars().count() > 31 {
            return bad("is longer than 31 characters");
        }
        if name
            .chars()
            .any(|c| matches!(c, '\\' | '/' | '?' | '*' | '[' | ']' | ':'))
        {
            return bad("contains one of \\ / ? * [ ] :");
        }
        if name.starts_with('\'') || name.ends_with('\'') {
            return bad("begins or ends with an apostrophe");
        }
        if name.eq_ignore_ascii_case("History") {
            return bad("is reserved");
        }
        if self
            .sheets
            .iter()
            .enumerate()
            .any(|(i, s)| Some(i) != ignore && s.info.name.eq_ignore_ascii_case(name))
        {
            return bad("is already used by another sheet");
        }
        Ok(())
    }

    fn default_sheet_name(&self) -> String {
        (self.sheets.len() + 1..)
            .map(|n| format!("Sheet{n}"))
            .find(|n| self.validate_sheet_name(n, None).is_ok())
            .unwrap_or_default()
    }

    fn remap_sheet_scopes(&mut self, f: &dyn Fn(u32) -> Option<u32>) {
        self.defined_names.retain_mut(|n| match n.local_sheet_id {
            Some(id) => match f(id) {
                Some(new) => {
                    n.local_sheet_id = Some(new);
                    true
                }
                None => false,
            },
            None => true,
        });
        self.rebuild_names_element();
    }

    fn rebuild_names_element(&mut self) {
        // `sync_names_element` lives with the names writer.
        let names = self.defined_names.clone();
        self.defined_names = names;
        self.sync_names_element();
    }

    /// Inserts an empty worksheet at position `at` (0 is first; the end when `at` is past it) and
    /// returns its index. The name defaults to `SheetN` as Excel numbers it.
    ///
    /// # Errors
    /// The name is illegal or taken.
    pub fn add_sheet(&mut self, at: usize, name: Option<&str>) -> Result<usize, XlsxError> {
        self.require_transitional()?;
        let name = name.map_or_else(|| self.default_sheet_name(), str::to_owned);
        self.validate_sheet_name(&name, None)?;
        let at = at.min(self.sheets.len());
        self.edited()?;
        let part_name = self.next_part_name("xl/worksheets", "sheet", "xml");
        let bytes = format!("{XML_DECL}{BLANK_SHEET}").into_bytes();
        let part = self.add_part(&part_name, content_type::WORKSHEET, bytes.clone())?;
        let rid = self.add_relationship(
            Some(&self.workbook_part.clone()),
            rel::WORKSHEET,
            &relative_target(Some(&self.workbook_part.clone()), &part_name),
            TargetMode::Internal,
        )?;
        let ws = Worksheet::parse(&bytes, &[])?;
        let sheet_id = self
            .sheets
            .iter()
            .map(|s| s.info.sheet_id)
            .max()
            .unwrap_or(0)
            + 1;
        self.insert_sheet_entry(at, &name, sheet_id, &rid, part, ws);
        Ok(at)
    }

    fn insert_sheet_entry(
        &mut self,
        at: usize,
        name: &str,
        sheet_id: u32,
        rid: &str,
        part: PartName,
        ws: Worksheet,
    ) {
        let info = SheetInfo {
            name: name.to_owned(),
            sheet_id,
            state: SheetState::Visible,
            kind: SheetKind::Worksheet,
            rel_id: rid.to_owned(),
        };
        let mut els = self.sheet_elements();
        let mut el = self.root.sibling("sheet");
        el.head.attrs.push(Attr::plain("name", name));
        el.head
            .attrs
            .push(Attr::plain("sheetId", &sheet_id.to_string()));
        el.head.attrs.push(Attr {
            name: "r:id".to_owned(),
            ns: Some(R_NS.into()),
            value: rid.to_owned(),
        });
        els.insert(at.min(els.len()), el);
        self.sheets.insert(
            at,
            SheetEntry {
                info,
                part,
                data: Some(ws),
                dirty: true,
            },
        );
        let at32 = u32::try_from(at).unwrap_or(u32::MAX);
        self.remap_sheet_scopes(&|id| Some(if id >= at32 { id + 1 } else { id }));
        let active = self.active_sheet();
        if active >= at && self.sheets.len() > 1 {
            self.set_active_tab_index(active + 1);
        }
        self.write_sheet_elements(els);
    }

    /// Parts reachable from `root` through internal relationships that nothing outside that set uses.
    fn owned_parts(&self, root: &PartName) -> BTreeSet<String> {
        let mut set: BTreeSet<String> = BTreeSet::new();
        let mut stack = vec![root.clone()];
        while let Some(p) = stack.pop() {
            if !set.insert(p.as_str().to_owned()) {
                continue;
            }
            for r in self.relationships(Some(&p)) {
                if r.target_mode == TargetMode::Internal
                    && let Ok(t) = resolve_target(Some(&p), &r.target)
                    && self.package.part(t.as_str()).is_some()
                {
                    stack.push(t);
                }
            }
        }
        loop {
            let mut shared: Vec<String> = Vec::new();
            for part in self.package.parts() {
                let name = part.name().as_str();
                let Some(source) = rels_source_of(name) else {
                    continue;
                };
                let inside = source.as_ref().is_some_and(|s| set.contains(s));
                if inside {
                    continue;
                }
                let src_part = source.as_deref().and_then(|s| PartName::new(s).ok());
                for r in self.relationships(src_part.as_ref()) {
                    if r.target_mode == TargetMode::Internal
                        && let Ok(t) = resolve_target(src_part.as_ref(), &r.target)
                        && set.contains(t.as_str())
                        && t.as_str() != root.as_str()
                    {
                        shared.push(t.as_str().to_owned());
                    }
                }
            }
            if shared.is_empty() {
                break;
            }
            for s in shared {
                set.remove(&s);
            }
        }
        set
    }

    /// Deletes a sheet with the parts only it uses. References to it become `#REF!`, names scoped to it
    /// go, and later sheets move up. Returns the formula cells (indexes after the delete) to recalculate.
    ///
    /// # Errors
    /// `sheet` is out of range or is the last visible sheet.
    pub fn delete_sheet(&mut self, sheet: usize) -> Result<Vec<(usize, u32, u32)>, XlsxError> {
        let count = self.sheets.len();
        let (info, part) = {
            let e = self.sheets.get(sheet).ok_or(XlsxError::SheetOutOfRange {
                index: sheet,
                count,
            })?;
            (e.info.clone(), e.part.clone())
        };
        let visible = self
            .sheets
            .iter()
            .filter(|s| s.info.state == SheetState::Visible)
            .count();
        if count <= 1 || (info.state == SheetState::Visible && visible <= 1) {
            return Err(XlsxError::UnsupportedEdit(
                "a workbook must contain at least one visible sheet".to_owned(),
            ));
        }
        self.edited()?;
        let name = info.name.clone();
        let rid = info.rel_id.clone();
        let wb_part = self.workbook_part.clone();

        // References to the sheet die.
        let dead_name = name.clone();
        let shifter = LeafShifter {
            leaf: move |n: &Ast, _home: Option<&str>| match n {
                Ast::Ref(r)
                    if r.sheet
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&dead_name)) =>
                {
                    Some(Ast::DeadRef {
                        sheet: dead_name.clone(),
                    })
                }
                Ast::Area(a)
                    if a.sheet
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&dead_name)) =>
                {
                    Some(Ast::DeadRef {
                        sheet: dead_name.clone(),
                    })
                }
                _ => None,
            },
        };
        let recalc = self.rewrite_all_formulas(&shifter);

        let doomed = self.owned_parts(&part);
        self.remove_relationship(Some(&wb_part), &rid)?;
        for p in &doomed {
            self.drop_part(p);
        }
        let mut els = self.sheet_elements();
        els.remove(sheet);
        self.sheets.remove(sheet);
        let at32 = u32::try_from(sheet).unwrap_or(u32::MAX);
        self.remap_sheet_scopes(&|id| match id.cmp(&at32) {
            std::cmp::Ordering::Less => Some(id),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(id - 1),
        });
        let active = self.active_sheet();
        let new_active = match active.cmp(&sheet) {
            std::cmp::Ordering::Less => active,
            std::cmp::Ordering::Equal => sheet.min(self.sheets.len() - 1),
            std::cmp::Ordering::Greater => active - 1,
        };
        self.write_sheet_elements(els);
        self.set_active_tab_index(new_active);
        self.make_selected(new_active);
        Ok(recalc
            .into_iter()
            .filter(|&(i, _, _)| i != sheet)
            .map(|(i, r, c)| (if i > sheet { i - 1 } else { i }, r, c))
            .collect())
    }

    /// Marks `sheet` as the selected tab and clears the mark from every other sheet.
    fn make_selected(&mut self, sheet: usize) {
        for (i, e) in self.sheets.iter_mut().enumerate() {
            if let Some(ws) = e.data.as_mut()
                && ws.set_tab_selected(i == sheet)
            {
                e.dirty = true;
            }
        }
    }

    /// Renames a sheet and rewrites every reference to it: formulas, names, conditional formats,
    /// validations, hyperlink locations, chart series and pivot cache sources.
    ///
    /// # Errors
    /// The name is illegal or taken.
    pub fn rename_sheet(
        &mut self,
        sheet: usize,
        new_name: &str,
    ) -> Result<Vec<(usize, u32, u32)>, XlsxError> {
        let count = self.sheets.len();
        let old = self
            .sheets
            .get(sheet)
            .ok_or(XlsxError::SheetOutOfRange {
                index: sheet,
                count,
            })?
            .info
            .name
            .clone();
        self.validate_sheet_name(new_name, Some(sheet))?;
        if old == new_name {
            return Ok(Vec::new());
        }
        self.edited()?;
        let (o, n) = (old.clone(), new_name.to_owned());
        let shifter = LeafShifter {
            leaf: move |node: &Ast, _home: Option<&str>| match node {
                Ast::Ref(r)
                    if r.sheet
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&o)) =>
                {
                    let mut r = r.clone();
                    r.sheet = Some(n.clone());
                    Some(Ast::Ref(r))
                }
                Ast::Area(a)
                    if a.sheet
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&o)) =>
                {
                    let mut a = a.clone();
                    a.sheet = Some(n.clone());
                    Some(Ast::Area(a))
                }
                Ast::DeadRef { sheet } if sheet.eq_ignore_ascii_case(&o) => {
                    Some(Ast::DeadRef { sheet: n.clone() })
                }
                _ => None,
            },
        };
        let recalc = self.rewrite_all_formulas(&shifter);
        self.sheets[sheet].info.name = new_name.to_owned();
        let mut els = self.sheet_elements();
        if let Some(e) = els.get_mut(sheet) {
            e.head.set_attr("name", new_name);
        }
        self.write_sheet_elements(els);

        let (old_prefix, new_prefix) = (quoted_prefix(&old), quoted_prefix(new_name));
        // Hyperlink locations.
        for entry in &mut self.sheets {
            let Some(ws) = entry.data.as_mut() else {
                continue;
            };
            let mut touched = false;
            for e in ws.root_elements_mut() {
                if e.head.is_main("hyperlinks") {
                    for h in e.children.iter_mut().filter_map(RawXml::as_element_mut) {
                        if let Some(loc) = h.head.attr("location").map(str::to_owned)
                            && let Some(rest) = strip_sheet_prefix(&loc, &old)
                        {
                            h.head.set_attr("location", &format!("{new_prefix}{rest}"));
                            touched = true;
                        }
                    }
                }
            }
            if touched {
                entry.dirty = true;
            }
        }
        let _ = old_prefix;
        // Chart series and pivot cache sources.
        let names: Vec<String> = self.part_names();
        for part in names {
            let Some(ct) = crate::parts::content_type_of(&self.package, &part) else {
                continue;
            };
            let is_chart = ct == content_type::CHART;
            let is_cache = ct == content_type::PIVOT_CACHE_DEFINITION;
            if !is_chart && !is_cache {
                continue;
            }
            let Some(bytes) = self.part_bytes(&part).map(<[u8]>::to_vec) else {
                continue;
            };
            let Ok(mut doc) = xml::parse(&bytes) else {
                continue;
            };
            let mut changed = false;
            retarget_sheet_refs(&mut doc.root, &old, new_name, &mut changed);
            if changed {
                self.put_part_bytes(&part, doc.to_bytes())?;
            }
        }
        Ok(recalc)
    }

    /// Moves a sheet so it ends up at index `to`.
    ///
    /// # Errors
    /// An index is out of range.
    pub fn move_sheet(&mut self, from: usize, to: usize) -> Result<(), XlsxError> {
        let count = self.sheets.len();
        for index in [from, to] {
            if index >= count {
                return Err(XlsxError::SheetOutOfRange { index, count });
            }
        }
        if from == to {
            return Ok(());
        }
        self.edited()?;
        let mut els = self.sheet_elements();
        let e = els.remove(from);
        els.insert(to, e);
        let s = self.sheets.remove(from);
        self.sheets.insert(to, s);
        let map = |i: usize| -> usize {
            if i == from {
                to
            } else if from < to && i > from && i <= to {
                i - 1
            } else if from > to && i >= to && i < from {
                i + 1
            } else {
                i
            }
        };
        self.remap_sheet_scopes(&|id| Some(u32::try_from(map(id as usize)).unwrap_or(id)));
        let active = self.active_sheet();
        self.set_active_tab_index(map(active));
        self.write_sheet_elements(els);
        Ok(())
    }

    /// Copies a sheet to position `at`, with its tables, drawings, charts, notes and sheet-scoped
    /// names. A table is renamed `Name_2`; references to the sheet by name inside the copy point at the
    /// copy. Returns the index of the new sheet.
    ///
    /// # Errors
    /// The source is not a worksheet or the name is illegal or taken.
    pub fn copy_sheet(
        &mut self,
        sheet: usize,
        at: usize,
        name: Option<&str>,
    ) -> Result<usize, XlsxError> {
        self.require_transitional()?;
        let count = self.sheets.len();
        let src = self.sheets.get(sheet).ok_or(XlsxError::SheetOutOfRange {
            index: sheet,
            count,
        })?;
        let src_name = src.info.name.clone();
        let src_part = src.part.clone();
        let mut ws = src.data.clone().ok_or(XlsxError::NotAWorksheet(sheet))?;
        let new_name = match name {
            Some(n) => n.to_owned(),
            None => (2u32..)
                .map(|k| {
                    let suffix = format!(" ({k})");
                    let keep = 31usize.saturating_sub(suffix.chars().count());
                    format!(
                        "{}{suffix}",
                        src_name.chars().take(keep).collect::<String>()
                    )
                })
                .find(|n| self.validate_sheet_name(n, None).is_ok())
                .unwrap_or_default(),
        };
        self.validate_sheet_name(&new_name, None)?;
        let at = at.min(count);
        self.edited()?;

        let new_part_name = self.next_part_name("xl/worksheets", "sheet", "xml");
        let new_part = PartName::new(&new_part_name)?;
        // Copy the sheet's relationships and the parts behind them.
        let mut copied_charts: Vec<String> = Vec::new();
        let mut table_models: Vec<Table> = Vec::new();
        for r in self.relationships(Some(&src_part)) {
            if r.target_mode == TargetMode::External {
                self.add_relationship_with_id(&new_part, &r.id, &r.rel_type, &r.target, true)?;
                continue;
            }
            if r.rel_type == rel::PIVOT_TABLE {
                continue;
            }
            let Ok(target) = resolve_target(Some(&src_part), &r.target) else {
                continue;
            };
            if self.package.part(target.as_str()).is_none() {
                continue;
            }
            let copy = self.copy_part_tree(&target, &mut copied_charts)?;
            let mut copy_name = copy.clone();
            if r.rel_type == rel::TABLE {
                copy_name = self.rename_copied_table(&copy, &mut table_models)?;
            }
            self.add_relationship_with_id(
                &new_part,
                &r.id,
                &r.rel_type,
                &relative_target(Some(&new_part), &copy_name),
                false,
            )?;
        }
        // Charts of the copy follow the copy.
        let (o, n) = (src_name.clone(), new_name.clone());
        for part in &copied_charts {
            if let Some(bytes) = self.part_bytes(part).map(<[u8]>::to_vec)
                && let Ok(mut doc) = xml::parse(&bytes)
            {
                let mut changed = false;
                retarget_sheet_refs(&mut doc.root, &o, &n, &mut changed);
                if changed {
                    self.put_part_bytes(part, doc.to_bytes())?;
                }
            }
        }
        let self_name = src_name.clone();
        let target_name = new_name.clone();
        let shifter = LeafShifter {
            leaf: move |node: &Ast, _home: Option<&str>| match node {
                Ast::Ref(r)
                    if r.sheet
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&self_name)) =>
                {
                    let mut r = r.clone();
                    r.sheet = Some(target_name.clone());
                    Some(Ast::Ref(r))
                }
                Ast::Area(a)
                    if a.sheet
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case(&self_name)) =>
                {
                    let mut a = a.clone();
                    a.sheet = Some(target_name.clone());
                    Some(Ast::Area(a))
                }
                _ => None,
            },
        };
        {
            use crate::structure::FormulaShifter as _;
            ws.materialise_all_shared();
            ws.shift_cell_formulas(&shifter, &src_name);
            ws.shift_raw_formulas(&shifter, &src_name, None);
            ws.set_tab_selected(false);
            ws.strip_revision_uid();
            ws.set_tables(table_models);
            let _ = shifter.shift_name("");
        }
        let bytes = ws.to_bytes();
        self.package.set_part(new_part.clone(), bytes);
        self.sync_content_type(&new_part_name, content_type::WORKSHEET)?;
        let rid = self.add_relationship(
            Some(&self.workbook_part.clone()),
            rel::WORKSHEET,
            &relative_target(Some(&self.workbook_part.clone()), &new_part_name),
            TargetMode::Internal,
        )?;
        let sheet_id = self
            .sheets
            .iter()
            .map(|s| s.info.sheet_id)
            .max()
            .unwrap_or(0)
            + 1;
        // Names scoped to the source sheet are copied to the new one.
        let src_id = u32::try_from(sheet).unwrap_or(u32::MAX);
        let copies: Vec<crate::workbook::DefinedName> = self
            .defined_names
            .iter()
            .filter(|d| d.local_sheet_id == Some(src_id))
            .cloned()
            .collect();
        self.insert_sheet_entry(at, &new_name, sheet_id, &rid, new_part, ws);
        let new_id = u32::try_from(at).unwrap_or(u32::MAX);
        for mut d in copies {
            d.local_sheet_id = Some(new_id);
            let rewrite = LeafShifter {
                leaf: {
                    let (o, n) = (src_name.clone(), new_name.clone());
                    move |node: &Ast, _h: Option<&str>| match node {
                        Ast::Ref(r)
                            if r.sheet
                                .as_deref()
                                .is_some_and(|s| s.eq_ignore_ascii_case(&o)) =>
                        {
                            let mut r = r.clone();
                            r.sheet = Some(n.clone());
                            Some(Ast::Ref(r))
                        }
                        Ast::Area(a)
                            if a.sheet
                                .as_deref()
                                .is_some_and(|s| s.eq_ignore_ascii_case(&o)) =>
                        {
                            let mut a = a.clone();
                            a.sheet = Some(n.clone());
                            Some(Ast::Area(a))
                        }
                        _ => None,
                    }
                },
            };
            use crate::structure::FormulaShifter as _;
            if let Some(f) = rewrite.shift_name(&d.formula) {
                d.formula = f;
            }
            self.defined_names.push(d);
        }
        self.sync_names_element();
        self.sheets[at].dirty = true;
        Ok(at)
    }

    pub(super) fn add_relationship_with_id(
        &mut self,
        source: &PartName,
        id: &str,
        rel_type: &str,
        target: &str,
        external: bool,
    ) -> Result<(), XlsxError> {
        let name = open365_opc::rels_part_name(Some(source));
        let mut tree = match self.package.part(name.as_str()) {
            Some(p) => xml::parse(p.data())?,
            None => xml::parse(
                br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
            )?,
        };
        let mut r = tree.root.sibling("Relationship");
        r.head.attrs = vec![
            Attr::plain("Id", id),
            Attr::plain("Type", rel_type),
            Attr::plain("Target", target),
        ];
        if external {
            r.head.attrs.push(Attr::plain("TargetMode", "External"));
        }
        tree.root.children.push(RawXml::Element(r));
        self.package.set_part(name, tree.to_bytes());
        self.parts_dirty = true;
        Ok(())
    }

    /// Copies `part` and everything it relates to under fresh names; returns the copy's name.
    fn copy_part_tree(
        &mut self,
        part: &PartName,
        charts: &mut Vec<String>,
    ) -> Result<String, XlsxError> {
        let src = part.as_str().to_owned();
        let (dir, file) = src
            .rsplit_once('/')
            .map_or(("", src.as_str()), |(d, f)| (d, f));
        let (stem, ext) = file.rsplit_once('.').unwrap_or((file, ""));
        let base = stem.trim_end_matches(|c: char| c.is_ascii_digit());
        let name = (1u32..)
            .map(|n| {
                let f = if ext.is_empty() {
                    format!("{base}{n}")
                } else {
                    format!("{base}{n}.{ext}")
                };
                if dir.is_empty() {
                    f
                } else {
                    format!("{dir}/{f}")
                }
            })
            .find(|n| self.package.part(n).is_none())
            .unwrap_or_default();
        let bytes = self
            .part_bytes(&src)
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        let ct = crate::parts::content_type_of(&self.package, &src);
        self.package.set_part(PartName::new(&name)?, bytes);
        self.parts_dirty = true;
        if let Some(ct) = &ct {
            self.sync_content_type(&name, ct)?;
            if ct == content_type::CHART {
                charts.push(name.clone());
            }
        }
        let new_part = PartName::new(&name)?;
        for r in self.relationships(Some(part)) {
            if r.target_mode == TargetMode::External {
                self.add_relationship_with_id(&new_part, &r.id, &r.rel_type, &r.target, true)?;
                continue;
            }
            let Ok(t) = resolve_target(Some(part), &r.target) else {
                continue;
            };
            if self.package.part(t.as_str()).is_none() {
                continue;
            }
            let child = self.copy_part_tree(&t, charts)?;
            self.add_relationship_with_id(
                &new_part,
                &r.id,
                &r.rel_type,
                &relative_target(Some(&new_part), &child),
                false,
            )?;
        }
        Ok(name)
    }

    /// Gives a copied table part a new name and id (`Table1` becomes `Table1_2`).
    fn rename_copied_table(
        &mut self,
        part: &str,
        models: &mut Vec<Table>,
    ) -> Result<String, XlsxError> {
        let bytes = self
            .part_bytes(part)
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        let mut doc = xml::parse(&bytes)?;
        let old = doc.root.head.attr("name").unwrap_or("Table").to_owned();
        let unique = (2u32..)
            .map(|k| format!("{old}_{k}"))
            .find(|n| {
                !self.tables().any(|(_, t)| t.name.eq_ignore_ascii_case(n))
                    && !models.iter().any(|t| t.name.eq_ignore_ascii_case(n))
                    && !self
                        .defined_names
                        .iter()
                        .any(|d| d.name.eq_ignore_ascii_case(n))
            })
            .unwrap_or(old);
        let id = self
            .tables()
            .map(|(_, t)| t.id)
            .chain(models.iter().map(|t| t.id))
            .max()
            .unwrap_or(0)
            + 1;
        doc.root.head.attrs.retain(|a| a.name != "xr:uid");
        doc.root.head.set_attr("name", &unique);
        doc.root.head.set_attr("displayName", &unique);
        doc.root.head.set_attr("id", &id.to_string());
        let out = doc.to_bytes();
        self.put_part_bytes(part, out.clone())?;
        if let Some(m) = Table::parse(part, &out) {
            models.push(m);
        }
        Ok(part.to_owned())
    }

    /// Sets or clears the tab colour.
    ///
    /// # Errors
    /// The sheet is not a worksheet.
    pub fn set_tab_color(&mut self, sheet: usize, color: TabColor) -> Result<(), XlsxError> {
        self.worksheet_mut(sheet)?;
        self.edited()?;
        let attrs = match color {
            TabColor::None => None,
            TabColor::Rgb(c) => Some(format!("rgb=\"FF{:02X}{:02X}{:02X}\"", c.r, c.g, c.b)),
            TabColor::Theme { index, tint } => Some(if tint == 0.0 {
                format!("theme=\"{index}\"")
            } else {
                format!("theme=\"{index}\" tint=\"{tint}\"")
            }),
            TabColor::Indexed(i) => Some(format!("indexed=\"{i}\"")),
        };
        let ws = self.worksheet_mut(sheet)?;
        ws.set_tab_color_element(attrs.as_deref())?;
        self.sheets[sheet].dirty = true;
        Ok(())
    }

    /// The tab colour, when the sheet has one.
    pub fn tab_color(&self, sheet: usize) -> Option<TabColor> {
        let ws = self.sheet(sheet)?;
        let c = ws.tab_color_element()?;
        let col = crate::format::Color::parse(c);
        Some(match col.source {
            ColorSource::Rgb(rgb) => TabColor::Rgb(rgb),
            ColorSource::Theme(index) => TabColor::Theme {
                index,
                tint: col.tint,
            },
            ColorSource::Indexed(i) => TabColor::Indexed(i),
            ColorSource::Auto => TabColor::None,
        })
    }

    /// Hides, unhides or very-hides a sheet. A workbook keeps one visible sheet; hiding the active tab
    /// activates the next visible one.
    ///
    /// # Errors
    /// `sheet` is out of range or hiding it would leave no visible sheet.
    pub fn set_sheet_state(&mut self, sheet: usize, state: SheetState) -> Result<(), XlsxError> {
        let count = self.sheets.len();
        let cur = self
            .sheets
            .get(sheet)
            .ok_or(XlsxError::SheetOutOfRange {
                index: sheet,
                count,
            })?
            .info
            .state;
        if cur == state {
            return Ok(());
        }
        if state != SheetState::Visible
            && !self
                .sheets
                .iter()
                .enumerate()
                .any(|(i, s)| i != sheet && s.info.state == SheetState::Visible)
        {
            return Err(XlsxError::UnsupportedEdit(
                "a workbook must contain at least one visible sheet".to_owned(),
            ));
        }
        self.edited()?;
        self.sheets[sheet].info.state = state;
        let mut els = self.sheet_elements();
        if let Some(e) = els.get_mut(sheet) {
            match state {
                SheetState::Visible => set_state_attr(e, None),
                SheetState::Hidden => set_state_attr(e, Some("hidden")),
                SheetState::VeryHidden => set_state_attr(e, Some("veryHidden")),
            }
        }
        self.write_sheet_elements(els);
        if state != SheetState::Visible && self.active_sheet() == sheet {
            let next = (sheet + 1..count)
                .chain((0..sheet).rev())
                .find(|&i| self.sheets[i].info.state == SheetState::Visible)
                .or_else(|| self.first_visible_index())
                .unwrap_or(0);
            self.set_active_tab_index(next);
            self.make_selected(next);
        }
        Ok(())
    }
}

/// `Old!A1` or `'Old'!A1` (case-insensitive sheet name) gives `A1`.
fn strip_sheet_prefix<'a>(text: &'a str, sheet: &str) -> Option<&'a str> {
    let plain = format!("{sheet}!");
    let quoted = format!("'{}'!", sheet.replace('\'', "''"));
    for p in [plain, quoted] {
        if text.len() >= p.len()
            && text.is_char_boundary(p.len())
            && text[..p.len()].eq_ignore_ascii_case(&p)
        {
            return Some(&text[p.len()..]);
        }
    }
    None
}

/// Rewrites `Old!` references inside `c:f` / `f` formulas and `worksheetSource sheet=` attributes.
fn retarget_sheet_refs(el: &mut XmlElement, old: &str, new: &str, changed: &mut bool) {
    let new_prefix = quoted_prefix(new);
    if el.head.local_name() == "worksheetSource"
        && el
            .head
            .attr("sheet")
            .is_some_and(|s| s.eq_ignore_ascii_case(old))
    {
        el.head.set_attr("sheet", new);
        *changed = true;
    }
    if el.head.local_name() == "f" {
        let text = el.own_text();
        let mut out = String::new();
        let mut any = false;
        for part in text.split(',') {
            // Series references are comma-separated areas inside parentheses; handle the plain forms.
            let trimmed = part.trim_start_matches('(');
            let lead = &part[..part.len() - trimmed.len()];
            if let Some(rest) = strip_sheet_prefix(trimmed, old) {
                out.push_str(&format!("{lead}{new_prefix}{rest}"));
                any = true;
            } else {
                out.push_str(part);
            }
            out.push(',');
        }
        if any {
            out.pop();
            el.children = vec![RawXml::Text(out)];
            *changed = true;
        }
    }
    for c in &mut el.children {
        if let RawXml::Element(x) = c {
            retarget_sheet_refs(x, old, new, changed);
        }
    }
}

/// Sets `state` on a `sheet` element, before `r:id` as Excel writes it.
fn set_state_attr(e: &mut XmlElement, value: Option<&str>) {
    e.head.remove_attr("state");
    if let Some(v) = value {
        let at = e
            .head
            .attrs
            .iter()
            .position(|a| a.ns.is_some() && a.local_name() == "id")
            .unwrap_or(e.head.attrs.len());
        e.head.attrs.insert(at, Attr::plain("state", v));
    }
}
