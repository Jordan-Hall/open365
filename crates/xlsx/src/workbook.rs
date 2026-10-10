use std::sync::Arc;

use open365_opc::{
    Limits, Package, PartName, Relationships, TargetMode, rels_part_name, resolve_target,
};

use crate::XlsxError;
use crate::parts::{self, HygieneAction};
use crate::refs::{MAX_COL, MAX_ROW};
use crate::sheet::{Cell, CellValue, Formula, Worksheet};
use crate::sst::SharedStrings;
use crate::styles::Styles;
use crate::xml::{self, Attr, RawXml, XmlElement};

const OFFICE_DOCUMENT_TYPES: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
];
const SST_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings";
const SST_REL_TYPE_STRICT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/sharedStrings";
const SST_CONTENT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";

/// Content the parser could not represent. A non-empty log blocks [`Workbook::save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedItem {
    /// Where it was found (part name and a short path).
    pub location: String,
    pub description: String,
}

/// Visibility of a sheet (`state` attribute).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetState {
    Visible,
    Hidden,
    VeryHidden,
}

/// What kind of part a sheet entry points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetKind {
    Worksheet,
    /// Chart sheet, dialog sheet or macro sheet: kept untouched, no cells.
    Other,
}

/// One `sheet` entry of the workbook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetInfo {
    pub name: String,
    pub sheet_id: u32,
    pub state: SheetState,
    pub kind: SheetKind,
    /// Relationship id (`r:id`) in the workbook relationships.
    pub rel_id: String,
}

/// A `definedName`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinedName {
    pub name: String,
    /// Index into [`Workbook::sheets`] when the name is sheet-scoped.
    pub local_sheet_id: Option<u32>,
    pub hidden: bool,
    /// The formula text, without `=`.
    pub formula: String,
}

/// What a caller can put in a cell.
#[derive(Debug, Clone, PartialEq)]
pub enum CellInput {
    Number(f64),
    /// Stored as a shared string.
    Text(String),
    Bool(bool),
    /// Clears the value and formula; the style stays.
    Empty,
}

#[derive(Debug)]
struct SheetEntry {
    info: SheetInfo,
    part: PartName,
    data: Option<Worksheet>,
    dirty: bool,
}

/// A SpreadsheetML workbook: the OPC package plus a typed model of the workbook part,
/// every worksheet, the shared strings and the number-format tables. Every other part
/// is untouched and written back byte-identical.
#[derive(Debug)]
pub struct Workbook {
    package: Package,
    workbook_part: PartName,
    prolog: String,
    root: XmlElement,
    epilogue: String,
    sheets: Vec<SheetEntry>,
    infos: Vec<SheetInfo>,
    defined_names: Vec<DefinedName>,
    date1904: bool,
    sst: Option<(PartName, SharedStrings)>,
    styles: Styles,
    workbook_dirty: bool,
    sst_dirty: bool,
    hygiene_done: bool,
    dropped: Vec<DroppedItem>,
    hygiene: Vec<HygieneAction>,
}

fn is_true(v: &str) -> bool {
    matches!(v.trim(), "1" | "true")
}

impl Workbook {
    /// Opens an XLSX. The workbook part is found through the package-level
    /// `officeDocument` relationship, worksheets through the workbook relationships.
    pub fn open(bytes: &[u8], limits: &Limits) -> Result<Self, XlsxError> {
        let package = Package::read(bytes, limits)?;
        let root_rels = package
            .part(rels_part_name(None).as_str())
            .ok_or(XlsxError::NoWorkbook)?;
        let root_rels = Relationships::parse(root_rels.data())?;
        let rel = root_rels
            .iter()
            .find(|r| {
                r.target_mode == TargetMode::Internal
                    && OFFICE_DOCUMENT_TYPES.contains(&r.rel_type.as_str())
            })
            .ok_or(XlsxError::NoWorkbook)?;
        let target = resolve_target(None, &rel.target)?;
        let part = package
            .part(target.as_str())
            .ok_or_else(|| XlsxError::MissingPart(target.to_string()))?;
        let workbook_part = part.name().clone();
        let parsed = xml::parse(part.data())?;
        if !parsed.root.head.is_main("workbook") {
            return Err(XlsxError::NotSpreadsheet(
                "main part is not a workbook".to_owned(),
            ));
        }

        let wb_rels = match package.part(rels_part_name(Some(&workbook_part)).as_str()) {
            Some(p) => Relationships::parse(p.data())?,
            None => Relationships::default(),
        };
        let internal_part = |suffix: &str| -> Result<Option<PartName>, XlsxError> {
            let Some(rel) = wb_rels
                .iter()
                .find(|r| r.target_mode == TargetMode::Internal && r.rel_type.ends_with(suffix))
            else {
                return Ok(None);
            };
            let name = resolve_target(Some(&workbook_part), &rel.target)?;
            match package.part(name.as_str()) {
                Some(p) => Ok(Some(p.name().clone())),
                None => Err(XlsxError::MissingPart(name.to_string())),
            }
        };

        let sst = match internal_part("/sharedStrings")? {
            Some(name) => {
                let data = package.part(name.as_str()).map(|p| p.data());
                let sst = SharedStrings::parse(data.unwrap_or_default())?;
                Some((name, sst))
            }
            None => None,
        };
        let mut styles = match internal_part("/styles")? {
            Some(name) => {
                let data = package.part(name.as_str()).map(|p| p.data());
                Styles::parse(data.unwrap_or_default())?
            }
            None => Styles::default(),
        };
        if let Some(name) = internal_part("/theme")?
            && let Some(part) = package.part(name.as_str())
        {
            styles.set_theme(crate::format::Theme::parse(part.data()));
        }
        let empty: [Arc<str>; 0] = [];
        let sst_texts: &[Arc<str>] = sst.as_ref().map_or(&empty[..], |(_, s)| s.texts());

        let mut sheets = Vec::new();
        let mut dropped = Vec::new();
        if let Some(list) = parsed.root.child_main("sheets") {
            for (i, el) in list
                .elements()
                .filter(|e| e.head.is_main("sheet"))
                .enumerate()
            {
                let name = el
                    .head
                    .attr("name")
                    .ok_or_else(|| malformed("sheet without a name"))?
                    .to_owned();
                let rel_id = el
                    .head
                    .attr_r("id")
                    .ok_or_else(|| malformed("sheet without r:id"))?
                    .to_owned();
                let sheet_id = el
                    .head
                    .attr("sheetId")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or_else(|| u32::try_from(i + 1).unwrap_or(u32::MAX));
                let state = match el.head.attr("state") {
                    Some("hidden") => SheetState::Hidden,
                    Some("veryHidden") => SheetState::VeryHidden,
                    _ => SheetState::Visible,
                };
                let rel = wb_rels
                    .get(&rel_id)
                    .filter(|r| r.target_mode == TargetMode::Internal)
                    .ok_or_else(|| malformed(&format!("sheet {name:?} has no relationship")))?;
                let target = resolve_target(Some(&workbook_part), &rel.target)?;
                let sheet_part = package
                    .part(target.as_str())
                    .ok_or_else(|| XlsxError::MissingPart(target.to_string()))?;
                let is_worksheet = rel.rel_type.ends_with("/worksheet");
                let data = if is_worksheet {
                    let ws = Worksheet::parse(sheet_part.data(), sst_texts)
                        .map_err(|e| in_part(sheet_part.name(), e))?;
                    for d in ws.dropped() {
                        dropped.push(DroppedItem {
                            location: sheet_part.name().to_string(),
                            description: d.clone(),
                        });
                    }
                    Some(ws)
                } else {
                    None
                };
                sheets.push(SheetEntry {
                    info: SheetInfo {
                        name,
                        sheet_id,
                        state,
                        kind: if is_worksheet {
                            SheetKind::Worksheet
                        } else {
                            SheetKind::Other
                        },
                        rel_id,
                    },
                    part: sheet_part.name().clone(),
                    data,
                    dirty: false,
                });
            }
        }

        let defined_names = parsed
            .root
            .child_main("definedNames")
            .map(|names| {
                names
                    .elements()
                    .filter(|e| e.head.is_main("definedName"))
                    .filter_map(|e| {
                        Some(DefinedName {
                            name: e.head.attr("name")?.to_owned(),
                            local_sheet_id: e
                                .head
                                .attr("localSheetId")
                                .and_then(|v| v.trim().parse().ok()),
                            hidden: e.head.attr("hidden").is_some_and(is_true),
                            formula: e.own_text(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let date1904 = parsed
            .root
            .child_main("workbookPr")
            .and_then(|e| e.head.attr("date1904"))
            .is_some_and(is_true);
        let infos = sheets.iter().map(|s| s.info.clone()).collect();

        Ok(Self {
            package,
            workbook_part,
            prolog: parsed.prolog,
            root: parsed.root,
            epilogue: parsed.epilogue,
            sheets,
            infos,
            defined_names,
            date1904,
            sst,
            styles,
            workbook_dirty: false,
            sst_dirty: false,
            hygiene_done: false,
            dropped,
            hygiene: Vec::new(),
        })
    }

    pub fn package(&self) -> &Package {
        &self.package
    }

    /// Name of the workbook part.
    pub fn workbook_part(&self) -> &PartName {
        &self.workbook_part
    }

    /// Sheets in workbook order (index 0 is the first tab).
    pub fn sheets(&self) -> &[SheetInfo] {
        &self.infos
    }

    /// Index of the sheet called `name` (case-insensitive, as Excel compares them).
    pub fn sheet_index(&self, name: &str) -> Option<usize> {
        self.infos
            .iter()
            .position(|s| s.name.to_lowercase() == name.to_lowercase())
    }

    /// Part name of sheet `sheet`.
    pub fn sheet_part(&self, sheet: usize) -> Option<&PartName> {
        self.sheets.get(sheet).map(|s| &s.part)
    }

    /// The worksheet model of sheet `sheet` (`None` for chart sheets and bad indices).
    pub fn sheet(&self, sheet: usize) -> Option<&Worksheet> {
        self.sheets.get(sheet)?.data.as_ref()
    }

    /// The cell at 1-based `(row, col)` of sheet `sheet`.
    pub fn cell(&self, sheet: usize, row: u32, col: u32) -> Option<&Cell> {
        self.sheet(sheet)?.cell(row, col)
    }

    /// All cells of a sheet in row-major order as `(row, col, cell)`.
    pub fn iter_cells(&self, sheet: usize) -> impl Iterator<Item = (u32, u32, &Cell)> {
        self.sheet(sheet)
            .into_iter()
            .flat_map(Worksheet::iter_cells)
    }

    /// Formula text of a cell without `=`; shared-formula dependents are expanded.
    pub fn formula_text(&self, sheet: usize, row: u32, col: u32) -> Option<String> {
        self.sheet(sheet)?.formula_text(row, col)
    }

    pub fn defined_names(&self) -> &[DefinedName] {
        &self.defined_names
    }

    /// True when dates count from 1904 (`workbookPr date1904`).
    pub fn date1904(&self) -> bool {
        self.date1904
    }

    pub fn styles(&self) -> &Styles {
        &self.styles
    }

    pub fn shared_strings(&self) -> Option<&SharedStrings> {
        self.sst.as_ref().map(|(_, s)| s)
    }

    /// Number format code of a cell (`General` when the cell or its style is missing).
    pub fn number_format(&self, sheet: usize, row: u32, col: u32) -> &str {
        let xf = self
            .cell(sheet, row, col)
            .and_then(|c| c.style)
            .unwrap_or(0);
        self.styles.number_format(xf)
    }

    /// Content that could not be represented. Empty means nothing was lost.
    pub fn dropped(&self) -> &[DroppedItem] {
        &self.dropped
    }

    /// Parts, relationships and settings changed by the first edit.
    pub fn hygiene_log(&self) -> &[HygieneAction] {
        &self.hygiene
    }

    /// True when anything changed since opening or the last save.
    pub fn is_dirty(&self) -> bool {
        self.workbook_dirty || self.sst_dirty || self.sheets.iter().any(|s| s.dirty)
    }

    // ---- editing -------------------------------------------------------------------

    fn check_target(&self, sheet: usize, row: u32, col: u32) -> Result<&Worksheet, XlsxError> {
        let entry = self.sheets.get(sheet).ok_or(XlsxError::SheetOutOfRange {
            index: sheet,
            count: self.sheets.len(),
        })?;
        let ws = entry.data.as_ref().ok_or(XlsxError::NotAWorksheet(sheet))?;
        if !(1..=MAX_ROW).contains(&row) || !(1..=MAX_COL).contains(&col) {
            return Err(XlsxError::CellOutOfRange { row, col });
        }
        ws.check_editable(row, col)?;
        Ok(ws)
    }

    /// Sets a cell to a constant. Text becomes a shared string (reusing an identical
    /// plain item, else appending one and updating `uniqueCount`/`count`). Replaces any
    /// formula; when the cell was a shared-formula master its dependents become normal
    /// formulas first.
    pub fn set_cell_value(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        input: CellInput,
    ) -> Result<(), XlsxError> {
        let ws = self.check_target(sheet, row, col)?;
        if let CellInput::Number(n) = &input
            && !n.is_finite()
        {
            return Err(XlsxError::InvalidNumber(*n));
        }
        if matches!(input, CellInput::Empty) && ws.cell(row, col).is_none() {
            return Ok(());
        }
        self.edited()?;
        let value = match input {
            CellInput::Number(n) => CellValue::Number(n),
            CellInput::Bool(b) => CellValue::Bool(b),
            CellInput::Empty => CellValue::Empty,
            CellInput::Text(text) => {
                let (index, text) = self.intern(&text)?;
                CellValue::SharedString { index, text }
            }
        };
        self.store(sheet, row, col, value, None);
        Ok(())
    }

    /// Sets a normal formula. `text` may start with `=`. `cached` is the result to write
    /// next to it (strings become `t="str"`); without one the cell has no cached value
    /// and the application computes it on load.
    pub fn set_cell_formula(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        text: &str,
        cached: Option<CellValue>,
    ) -> Result<(), XlsxError> {
        self.check_target(sheet, row, col)?;
        let text = text.strip_prefix('=').unwrap_or(text);
        if text.trim().is_empty() {
            return Err(XlsxError::UnsupportedEdit("empty formula".to_owned()));
        }
        if let Some(c) = text
            .chars()
            .find(|c| matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}'))
        {
            return Err(XlsxError::InvalidText(c));
        }
        let cached = match cached {
            None => CellValue::Empty,
            Some(CellValue::Number(n)) if !n.is_finite() => {
                return Err(XlsxError::InvalidNumber(n));
            }
            // A formula cannot reference the shared string table or hold an inline string.
            Some(CellValue::SharedString { text, .. } | CellValue::InlineStr(text)) => {
                CellValue::Str(text)
            }
            Some(other) => other,
        };
        self.edited()?;
        self.store(sheet, row, col, cached, Some(Formula::normal(text)));
        Ok(())
    }

    fn intern(&mut self, text: &str) -> Result<(u32, Arc<str>), XlsxError> {
        self.ensure_shared_strings()?;
        let Some((_, sst)) = self.sst.as_mut() else {
            return Err(XlsxError::MissingPart("shared strings".to_owned()));
        };
        self.sst_dirty = true;
        Ok(sst.intern(text))
    }

    /// Writes the new content, keeping the shared-formula and shared-string bookkeeping.
    fn store(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        value: CellValue,
        formula: Option<Formula>,
    ) {
        let Some(entry) = self.sheets.get_mut(sheet) else {
            return;
        };
        let Some(ws) = entry.data.as_mut() else {
            return;
        };
        ws.materialise_if_master(row, col);
        let was_shared = matches!(
            ws.cell(row, col).map(|c| &c.value),
            Some(CellValue::SharedString { .. })
        );
        let is_shared = matches!(value, CellValue::SharedString { .. });
        ws.set_content(row, col, value, formula);
        entry.dirty = true;
        if let Some((_, sst)) = self.sst.as_mut() {
            let delta = i64::from(is_shared) - i64::from(was_shared);
            if delta != 0 {
                sst.adjust_count(delta);
                self.sst_dirty = true;
            }
        }
    }

    /// First edit: drop what it makes stale and ask the application to recalculate.
    fn edited(&mut self) -> Result<(), XlsxError> {
        if self.hygiene_done {
            return Ok(());
        }
        self.hygiene_done = true;
        parts::invalidate_stale_parts(&mut self.package, &self.workbook_part, &mut self.hygiene)?;
        self.set_full_calc_on_load();
        Ok(())
    }

    fn set_full_calc_on_load(&mut self) {
        const AFTER_CALC_PR: [&str; 9] = [
            "oleSize",
            "customWorkbookViews",
            "pivotCaches",
            "smartTagPr",
            "smartTagTypes",
            "webPublishing",
            "fileRecoveryPr",
            "webPublishObjects",
            "extLst",
        ];
        if let Some(calc) = self
            .root
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("calcPr"))
        {
            calc.head.set_attr("fullCalcOnLoad", "1");
        } else {
            let mut calc = self.root.sibling("calcPr");
            calc.head.attrs.push(Attr::plain("fullCalcOnLoad", "1"));
            let at = self
                .root
                .children
                .iter()
                .position(|c| {
                    c.as_element()
                        .is_some_and(|e| AFTER_CALC_PR.contains(&e.head.local_name()))
                })
                .unwrap_or(self.root.children.len());
            self.root.children.insert(at, RawXml::Element(calc));
        }
        self.workbook_dirty = true;
        self.hygiene.push(HygieneAction::SetFullCalcOnLoad);
    }

    /// Creates `sharedStrings.xml` (with relationship and content-type override) when the
    /// workbook has none.
    fn ensure_shared_strings(&mut self) -> Result<(), XlsxError> {
        if self.sst.is_some() {
            return Ok(());
        }
        let dir = self
            .workbook_part
            .as_str()
            .rsplit_once('/')
            .map_or("", |(d, _)| d);
        let file = "sharedStrings.xml";
        let full = if dir.is_empty() {
            file.to_owned()
        } else {
            format!("{dir}/{file}")
        };
        if self.package.part(&full).is_some() {
            return Err(XlsxError::UnsupportedEdit(format!(
                "{full} exists but is not related to the workbook"
            )));
        }
        let name = PartName::new(&full)?;
        let strict = self
            .root
            .head
            .ns
            .as_deref()
            .is_some_and(|ns| ns == xml::MAIN_NS_STRICT);
        let main_ns = self
            .root
            .head
            .ns
            .as_deref()
            .unwrap_or(xml::MAIN_NS)
            .to_owned();
        let sst = SharedStrings::new_empty(&main_ns);
        self.package.set_part(name.clone(), sst.serialize());
        parts::add_relationship(
            &mut self.package,
            &self.workbook_part,
            if strict {
                SST_REL_TYPE_STRICT
            } else {
                SST_REL_TYPE
            },
            file,
        )?;
        parts::add_override(&mut self.package, name.as_str(), SST_CONTENT_TYPE)?;
        self.hygiene.push(HygieneAction::AddedPart {
            name: name.to_string(),
            reason: "the workbook had no shared string table",
        });
        self.sst = Some((name, sst));
        self.sst_dirty = true;
        Ok(())
    }

    // ---- writing -------------------------------------------------------------------

    /// The workbook part serialised from the model. Does not touch the package.
    pub fn serialize_workbook(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push_str(&self.prolog);
        self.root.write(&mut out);
        out.push_str(&self.epilogue);
        out.into_bytes()
    }

    /// Worksheet `sheet` serialised from the model. Does not touch the package.
    pub fn serialize_sheet(&self, sheet: usize) -> Option<Vec<u8>> {
        self.sheet(sheet).map(Worksheet::to_bytes)
    }

    /// The shared strings part serialised from the model, when there is one.
    pub fn serialize_shared_strings(&self) -> Option<Vec<u8>> {
        self.sst.as_ref().map(|(_, s)| s.serialize())
    }

    /// The styles part serialised from its tree, when there is one.
    pub fn serialize_styles(&self) -> Option<Vec<u8>> {
        self.styles.serialize()
    }

    /// Writes the package. Only changed parts are re-serialised; every other part is
    /// written back byte-identical. Refuses while the dropped-content log is non-empty.
    pub fn save(&mut self) -> Result<Vec<u8>, XlsxError> {
        if !self.dropped.is_empty() {
            return Err(XlsxError::DroppedContent(self.dropped.len()));
        }
        if self.workbook_dirty {
            let bytes = self.serialize_workbook();
            self.package.set_part(self.workbook_part.clone(), bytes);
            self.workbook_dirty = false;
        }
        for entry in &mut self.sheets {
            if entry.dirty
                && let Some(ws) = &entry.data
            {
                self.package.set_part(entry.part.clone(), ws.to_bytes());
                entry.dirty = false;
            }
        }
        if self.sst_dirty
            && let Some((name, sst)) = &self.sst
        {
            self.package.set_part(name.clone(), sst.serialize());
            self.sst_dirty = false;
        }
        Ok(self.package.write()?)
    }
}

fn malformed(reason: &str) -> XlsxError {
    XlsxError::Malformed {
        part: "workbook".to_owned(),
        reason: reason.to_owned(),
    }
}

fn in_part(part: &PartName, e: XlsxError) -> XlsxError {
    match e {
        XlsxError::Malformed { reason, .. } => XlsxError::Malformed {
            part: part.to_string(),
            reason,
        },
        other => other,
    }
}
