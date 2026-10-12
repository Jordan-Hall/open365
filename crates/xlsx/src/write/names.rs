//! Defined names: create, edit, delete and Create from Selection.

use open365_calc::Ast;
use open365_calc::ast::{AreaKind, AreaRef, CellRef};

use super::LeafShifter;
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW, parse_cell_ref};
use crate::sheet::{CellValue, Rect};
use crate::workbook::{DefinedName, Workbook};
use crate::xml::{Attr, RawXml};

/// A name to create or the new state of one to edit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NameSpec {
    pub name: String,
    /// The sheet (index into [`Workbook::sheets`]) the name is scoped to; `None` is workbook scope.
    pub scope: Option<usize>,
    /// Refers to: a formula without the leading `=`.
    pub formula: String,
    pub comment: Option<String>,
    pub hidden: bool,
}

/// Which edges of the selection hold the labels in Create from Selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NameSource {
    pub top: bool,
    pub left: bool,
    pub bottom: bool,
    pub right: bool,
}

/// Checks a name against Excel's rules.
///
/// # Errors
/// The name is empty, longer than 255 characters, uses a character Excel refuses, or reads as a
/// cell reference (`A1`, `R1C1`) or as `R` or `C`.
pub fn validate_defined_name(name: &str) -> Result<(), XlsxError> {
    let bad = |why: &str| {
        Err(XlsxError::UnsupportedEdit(format!(
            "the name {name:?} {why}"
        )))
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return bad("is empty");
    };
    if name.chars().count() > 255 {
        return bad("is longer than 255 characters");
    }
    if !(first.is_alphabetic() || first == '_' || first == '\\') {
        return bad("must start with a letter, an underscore or a backslash");
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '\\' | '?'))
    {
        return bad("has a character a name cannot contain");
    }
    let upper = name.to_ascii_uppercase();
    if upper == "R" || upper == "C" || looks_like_r1c1(&upper) {
        return bad("is a reference");
    }
    if let Some((r, c)) = parse_cell_ref(name)
        && (1..=MAX_ROW).contains(&r)
        && (1..=MAX_COL).contains(&c)
    {
        return bad("is a cell reference");
    }
    Ok(())
}

fn looks_like_r1c1(upper: &str) -> bool {
    let rest = match upper.strip_prefix('R') {
        Some(r) => r,
        None => return upper.strip_prefix('C').is_some_and(all_digits),
    };
    match rest.split_once('C') {
        Some((r, c)) => (r.is_empty() || all_digits(r)) && (c.is_empty() || all_digits(c)),
        None => all_digits(rest),
    }
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// A label made into a legal name the way Create from Selection does: invalid characters (and spaces)
/// become underscores, and a name that would not be legal gets a leading underscore.
pub(crate) fn name_from_label(label: &str) -> Option<String> {
    let label = label.trim();
    if label.is_empty() {
        return None;
    }
    let mut name: String = label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '.' | '\\' | '?') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if validate_defined_name(&name).is_err() {
        name.insert(0, '_');
    }
    validate_defined_name(&name).ok().map(|()| name)
}

/// The absolute reference text of `rect` on `sheet`, quoted as Excel writes it.
pub(crate) fn absolute_ref(sheet: &str, rect: Rect) -> String {
    let (r1, c1, r2, c2) = rect;
    let ast = if (r1, c1) == (r2, c2) {
        Ast::Ref(CellRef {
            sheet: Some(sheet.to_owned()),
            row: r1,
            col: c1,
            row_abs: true,
            col_abs: true,
        })
    } else {
        Ast::Area(AreaRef {
            sheet: Some(sheet.to_owned()),
            kind: AreaKind::Cells,
            row1: r1,
            col1: c1,
            row2: r2,
            col2: c2,
            row1_abs: true,
            col1_abs: true,
            row2_abs: true,
            col2_abs: true,
        })
    };
    open365_calc::to_formula_string(&ast)
}

impl Workbook {
    fn scope_id(&self, scope: Option<usize>) -> Result<Option<u32>, XlsxError> {
        match scope {
            None => Ok(None),
            Some(i) if i < self.sheets.len() => Ok(Some(u32::try_from(i).unwrap_or(u32::MAX))),
            Some(index) => Err(XlsxError::SheetOutOfRange {
                index,
                count: self.sheets.len(),
            }),
        }
    }

    fn find_name(&self, name: &str, scope: Option<u32>) -> Option<usize> {
        self.defined_names
            .iter()
            .position(|n| n.local_sheet_id == scope && n.name.eq_ignore_ascii_case(name))
    }

    fn check_new_name(&self, spec: &NameSpec, scope: Option<u32>) -> Result<(), XlsxError> {
        validate_defined_name(&spec.name)?;
        if spec.formula.trim().is_empty() {
            return Err(XlsxError::UnsupportedEdit(
                "a name needs a formula to refer to".to_owned(),
            ));
        }
        if open365_calc::parse_formula(spec.formula.trim_start_matches('=')).is_err() {
            return Err(XlsxError::UnsupportedEdit(format!(
                "the formula {:?} does not parse",
                spec.formula
            )));
        }
        if self.tables().any(|(_, t)| {
            t.name.eq_ignore_ascii_case(&spec.name)
                || t.display_name.eq_ignore_ascii_case(&spec.name)
        }) && scope.is_none()
        {
            return Err(XlsxError::UnsupportedEdit(format!(
                "a table is already called {:?}",
                spec.name
            )));
        }
        Ok(())
    }

    fn name_element(&self, n: &DefinedName) -> crate::xml::XmlElement {
        let mut el = self.root.sibling("definedName");
        el.head.attrs.push(Attr::plain("name", &n.name));
        if let Some(c) = &n.comment {
            el.head.attrs.push(Attr::plain("comment", c));
        }
        if let Some(id) = n.local_sheet_id {
            el.head
                .attrs
                .push(Attr::plain("localSheetId", &id.to_string()));
        }
        if n.hidden {
            el.head.attrs.push(Attr::plain("hidden", "1"));
        }
        el.children = vec![RawXml::Text(n.formula.clone())];
        el
    }

    /// Writes `defined_names` back into the `definedNames` element, sorted as Excel writes them.
    pub(super) fn sync_names_element(&mut self) {
        let mut names = self.defined_names.clone();
        names.sort_by_key(|n| (n.name.to_lowercase(), n.local_sheet_id));
        if names.is_empty() {
            self.put_workbook_child("definedNames", None);
            return;
        }
        let mut container = self
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("definedNames"))
            .map_or_else(
                || self.root.sibling("definedNames"),
                |e| {
                    let mut c = e.clone();
                    c.children.clear();
                    c
                },
            );
        // Keep names the model did not parse (no `name` attribute) out of the way: there are none.
        container.children = names
            .iter()
            .map(|n| RawXml::Element(self.name_element(n)))
            .collect();
        self.put_workbook_child("definedNames", Some(container));
        self.defined_names = names;
    }

    /// Creates a name.
    ///
    /// # Errors
    /// The name is illegal, already exists in that scope, clashes with a table, or the sheet is out of range.
    pub fn define_name(&mut self, spec: &NameSpec) -> Result<(), XlsxError> {
        let scope = self.scope_id(spec.scope)?;
        self.check_new_name(spec, scope)?;
        if self.find_name(&spec.name, scope).is_some() {
            return Err(XlsxError::UnsupportedEdit(format!(
                "the name {:?} already exists",
                spec.name
            )));
        }
        self.edited()?;
        self.defined_names.push(DefinedName {
            name: spec.name.clone(),
            local_sheet_id: scope,
            hidden: spec.hidden,
            formula: spec.formula.trim_start_matches('=').to_owned(),
            comment: spec.comment.clone(),
        });
        self.sync_names_element();
        Ok(())
    }

    /// Changes a name: its refers-to formula, comment, visibility and (as Excel's Name Manager does) its
    /// text. Formulas that use the old text are rewritten; the returned cells need a recalculation.
    ///
    /// # Errors
    /// The name does not exist, the new text is illegal or taken, or the scope differs (Excel cannot move a name).
    pub fn edit_defined_name(
        &mut self,
        old_name: &str,
        old_scope: Option<usize>,
        spec: &NameSpec,
    ) -> Result<Vec<(usize, u32, u32)>, XlsxError> {
        let scope = self.scope_id(old_scope)?;
        if spec.scope != old_scope {
            return Err(XlsxError::UnsupportedEdit(
                "the scope of a name cannot be changed".to_owned(),
            ));
        }
        let Some(at) = self.find_name(old_name, scope) else {
            return Err(XlsxError::UnsupportedEdit(format!("no name {old_name:?}")));
        };
        self.check_new_name(spec, scope)?;
        if !old_name.eq_ignore_ascii_case(&spec.name) && self.find_name(&spec.name, scope).is_some()
        {
            return Err(XlsxError::UnsupportedEdit(format!(
                "the name {:?} already exists",
                spec.name
            )));
        }
        self.edited()?;
        let renamed = old_name != spec.name;
        {
            let n = &mut self.defined_names[at];
            n.name.clone_from(&spec.name);
            n.formula = spec.formula.trim_start_matches('=').to_owned();
            n.comment.clone_from(&spec.comment);
            n.hidden = spec.hidden;
        }
        self.sync_names_element();
        if !renamed {
            return Ok(Vec::new());
        }
        let old = old_name.to_owned();
        let new = spec.name.clone();
        let scope_sheet = old_scope
            .and_then(|i| self.sheets.get(i))
            .map(|s| s.info.name.clone());
        let shifter = LeafShifter {
            leaf: move |node: &Ast, home: Option<&str>| match node {
                Ast::Name(n) if n.eq_ignore_ascii_case(&old) => {
                    let in_scope = match (&scope_sheet, home) {
                        (None, _) => true,
                        (Some(s), Some(h)) => s.eq_ignore_ascii_case(h),
                        (Some(_), None) => false,
                    };
                    in_scope.then(|| Ast::Name(new.clone()))
                }
                _ => None,
            },
        };
        Ok(self.rewrite_all_formulas(&shifter))
    }

    /// Deletes a name. Formulas that use it keep its text and show `#NAME?`, as in Excel.
    ///
    /// # Errors
    /// The name does not exist.
    pub fn delete_defined_name(
        &mut self,
        name: &str,
        scope: Option<usize>,
    ) -> Result<DefinedName, XlsxError> {
        let scope = self.scope_id(scope)?;
        let Some(at) = self.find_name(name, scope) else {
            return Err(XlsxError::UnsupportedEdit(format!("no name {name:?}")));
        };
        self.edited()?;
        let gone = self.defined_names.remove(at);
        self.sync_names_element();
        Ok(gone)
    }

    /// Formulas > Create from Selection: names the rows and columns of `rect` (`(row1, col1, row2, col2)`)
    /// after the labels on the chosen edges, at workbook scope. A name that exists at workbook scope is
    /// replaced when `replace` is set and refused otherwise.
    ///
    /// # Errors
    /// No edge is chosen, a name exists and `replace` is not set, or the sheet is not a worksheet.
    pub fn create_names_from_selection(
        &mut self,
        sheet: usize,
        rect: Rect,
        source: NameSource,
        replace: bool,
    ) -> Result<Vec<DefinedName>, XlsxError> {
        if !(source.top || source.left || source.bottom || source.right) {
            return Err(XlsxError::UnsupportedEdit(
                "choose at least one edge that holds the labels".to_owned(),
            ));
        }
        let sheet_name = self
            .sheets
            .get(sheet)
            .ok_or(XlsxError::SheetOutOfRange {
                index: sheet,
                count: self.sheets.len(),
            })?
            .info
            .name
            .clone();
        let (r1, c1, r2, c2) = rect;
        let mut wanted: Vec<(String, Rect)> = Vec::new();
        let label = |wb: &Workbook, r: u32, c: u32| -> Option<String> {
            match wb.cell(sheet, r, c).map(|x| &x.value)? {
                CellValue::Str(t) | CellValue::InlineStr(t) => Some(t.to_string()),
                CellValue::SharedString { text, .. } => Some(text.to_string()),
                CellValue::Number(n) => Some(format!("{n}")),
                _ => None,
            }
        };
        let row_lo = r1 + u32::from(source.top);
        let row_hi = r2.saturating_sub(u32::from(source.bottom));
        let col_lo = c1 + u32::from(source.left);
        let col_hi = c2.saturating_sub(u32::from(source.right));
        if source.top && row_lo <= row_hi {
            for c in c1..=c2 {
                if let Some(n) = label(self, r1, c).and_then(|l| name_from_label(&l)) {
                    wanted.push((n, (row_lo, c, row_hi, c)));
                }
            }
        }
        if source.bottom && row_lo <= row_hi {
            for c in c1..=c2 {
                if let Some(n) = label(self, r2, c).and_then(|l| name_from_label(&l)) {
                    wanted.push((n, (row_lo, c, row_hi, c)));
                }
            }
        }
        if source.left && col_lo <= col_hi {
            for r in r1..=r2 {
                if let Some(n) = label(self, r, c1).and_then(|l| name_from_label(&l)) {
                    wanted.push((n, (r, col_lo, r, col_hi)));
                }
            }
        }
        if source.right && col_lo <= col_hi {
            for r in r1..=r2 {
                if let Some(n) = label(self, r, c2).and_then(|l| name_from_label(&l)) {
                    wanted.push((n, (r, col_lo, r, col_hi)));
                }
            }
        }
        if !replace
            && let Some((n, _)) = wanted
                .iter()
                .find(|(n, _)| self.find_name(n, None).is_some())
        {
            return Err(XlsxError::UnsupportedEdit(format!(
                "the name {n:?} already exists"
            )));
        }
        self.edited()?;
        let mut made = Vec::new();
        for (name, area) in wanted {
            if let Some(at) = self.find_name(&name, None) {
                self.defined_names.remove(at);
            }
            let d = DefinedName {
                name,
                local_sheet_id: None,
                hidden: false,
                formula: absolute_ref(&sheet_name, area),
                comment: None,
            };
            self.defined_names.push(d.clone());
            made.push(d);
        }
        self.sync_names_element();
        Ok(made)
    }
}
