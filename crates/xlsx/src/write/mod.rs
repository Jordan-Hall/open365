//! Writers that add to a workbook what the reader keeps raw: new parts and relationships, tables,
//! defined names, hyperlinks, notes, charts, pivot tables, calculation settings, conditional
//! formats, sheet operations and cell shifts. They live in a module under `workbook` so they can
//! reach the package and the sheet list directly.
//!
//! Every writer keeps `[Content_Types].xml` and the relationship parts in step with the parts it
//! adds and names them the way Excel does (`xl/tables/table1.xml`, `rId1` ...).

use open365_calc::{Ast, parse_formula, to_formula_string};
use open365_opc::{PartName, Relationship, Relationships, TargetMode, rels_part_name};

use super::Workbook;
use crate::XlsxError;
use crate::parts;
use crate::structure::FormulaShifter;
use crate::xml::{self, XmlElement};

mod calc;
mod cf;
mod chart;
mod links;
mod names;
mod notes;
mod page_setup;
mod pivot;
mod sheets;
mod shift;
mod table;

pub use calc::{CalcMode, CalcSettings};
pub use cf::{CfRuleSpec, CfSpec};
pub use chart::{ChartAnchor, ChartInsert};
pub use links::{HyperlinkSpec, HyperlinkTarget};
pub use names::{NameSource, NameSpec, validate_defined_name};
pub use notes::NoteSpec;
pub use page_setup::{PageMargins, PageSetup};
pub use pivot::{PivotInsert, PivotSpec};
pub use sheets::TabColor;
pub use shift::ShiftDirection;
pub use table::{HeaderChoice, TableInfo, TableSpec, TableStyleChoice};

/// Relationship types Excel writes (transitional).
pub mod rel {
    pub const WORKSHEET: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet";
    pub const TABLE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
    pub const HYPERLINK: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
    pub const DRAWING: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
    pub const CHART: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart";
    pub const COMMENTS: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
    pub const VML_DRAWING: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing";
    pub const PIVOT_TABLE: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotTable";
    pub const PIVOT_CACHE_DEFINITION: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition";
    pub const PIVOT_CACHE_RECORDS: &str =
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheRecords";
}

/// Content types Excel writes.
pub mod content_type {
    pub const WORKSHEET: &str =
        "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
    pub const TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";
    pub const DRAWING: &str = "application/vnd.openxmlformats-officedocument.drawing+xml";
    pub const CHART: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
    pub const COMMENTS: &str =
        "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml";
    pub const VML_DRAWING: &str = "application/vnd.openxmlformats-officedocument.vmlDrawing";
    pub const PIVOT_TABLE: &str =
        "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml";
    pub const PIVOT_CACHE_DEFINITION: &str =
        "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml";
    pub const PIVOT_CACHE_RECORDS: &str =
        "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml";
}

pub(crate) const MAIN: &str = xml::MAIN_NS;
pub(crate) const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub(crate) const XML_DECL: &str =
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";

/// Escapes text for an attribute value or element content.
pub(crate) fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// The relative target that leads from `from` (a part, or `None` for the package) to `to`.
pub(crate) fn relative_target(from: Option<&PartName>, to: &str) -> String {
    let Some(from) = from else {
        return to.to_owned();
    };
    let from_dir: Vec<&str> = from
        .as_str()
        .rsplit_once('/')
        .map_or(Vec::new(), |(d, _)| d.split('/').collect());
    let to_parts: Vec<&str> = to.split('/').collect();
    let (to_dir, file) = to_parts.split_at(to_parts.len() - 1);
    let common = from_dir
        .iter()
        .zip(to_dir.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out: Vec<&str> = vec![".."; from_dir.len() - common];
    out.extend(&to_dir[common..]);
    out.extend(file);
    out.join("/")
}

/// Applies `leaf` to every reference-like node of `ast`; a `Some` replaces the node.
pub(crate) fn map_leaves(ast: &Ast, leaf: &dyn Fn(&Ast) -> Option<Ast>) -> Ast {
    let go = |a: &Ast| map_leaves(a, leaf);
    match ast {
        Ast::Ref(_)
        | Ast::Area(_)
        | Ast::DeadRef { .. }
        | Ast::Name(_)
        | Ast::Structured { .. } => leaf(ast).unwrap_or_else(|| ast.clone()),
        Ast::Func { name, xlfn, args } => Ast::Func {
            name: name.clone(),
            xlfn: *xlfn,
            args: args.iter().map(go).collect(),
        },
        Ast::Unary { op, operand } => Ast::Unary {
            op: *op,
            operand: Box::new(go(operand)),
        },
        Ast::Percent(inner) => Ast::Percent(Box::new(go(inner))),
        Ast::Binary { op, lhs, rhs } => Ast::Binary {
            op: *op,
            lhs: Box::new(go(lhs)),
            rhs: Box::new(go(rhs)),
        },
        Ast::Array(rows) => Ast::Array(rows.iter().map(|r| r.iter().map(go).collect()).collect()),
        Ast::Invoke { callee, args } => Ast::Invoke {
            callee: Box::new(go(callee)),
            args: args.iter().map(go).collect(),
        },
        other => other.clone(),
    }
}

/// Rewrites formulas with a function over the parsed tree. `leaf(node, home)` sees each
/// reference, name and structured reference, `home` being the sheet the formula lives on.
pub(crate) struct LeafShifter<F: Fn(&Ast, Option<&str>) -> Option<Ast>> {
    pub leaf: F,
}

impl<F: Fn(&Ast, Option<&str>) -> Option<Ast>> LeafShifter<F> {
    fn run(&self, text: &str, home: Option<&str>) -> Option<String> {
        let ast = parse_formula(text).ok()?;
        let new = map_leaves(&ast, &|n| (self.leaf)(n, home));
        (new != ast).then(|| to_formula_string(&new))
    }
}

impl<F: Fn(&Ast, Option<&str>) -> Option<Ast>> FormulaShifter for LeafShifter<F> {
    fn shift(&self, text: &str, home: &str) -> Option<String> {
        self.run(text, Some(home))
    }

    fn shift_name(&self, text: &str) -> Option<String> {
        self.run(text, None)
    }

    fn shift_anchored(&self, text: &str, home: &str, _delta: i64) -> Option<String> {
        self.run(text, Some(home))
    }
}

impl Workbook {
    fn require_transitional(&self) -> Result<(), XlsxError> {
        if self.root.head.ns.as_deref() == Some(xml::MAIN_NS_STRICT) {
            return Err(XlsxError::UnsupportedEdit(
                "adding parts to a Strict Open XML workbook is not supported".to_owned(),
            ));
        }
        Ok(())
    }

    /// The bytes of a package part.
    pub fn part_bytes(&self, name: &str) -> Option<&[u8]> {
        self.package
            .part(name.trim_start_matches('/'))
            .map(open365_opc::Part::data)
    }

    /// Names of every package part, in package order.
    pub fn part_names(&self) -> Vec<String> {
        self.package.parts().map(|p| p.name().to_string()).collect()
    }

    /// The relationships whose source is `source` (`None` is the package itself).
    pub fn relationships(&self, source: Option<&PartName>) -> Vec<Relationship> {
        self.package
            .part(rels_part_name(source).as_str())
            .and_then(|p| Relationships::parse(p.data()).ok())
            .map(|r| r.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// A part name `dir/stemN.ext` with the lowest free `N` (1 upwards), as Excel numbers them.
    pub fn next_part_name(&self, dir: &str, stem: &str, ext: &str) -> String {
        (1u32..)
            .map(|n| format!("{dir}/{stem}{n}.{ext}"))
            .find(|name| self.package.part(name).is_none())
            .unwrap_or_default()
    }

    /// Adds a new part and its content type. The type is an extension `Default` for `vml` (as Excel
    /// writes it) and an `Override` otherwise; nothing is written when the type already resolves to
    /// `content_type`.
    ///
    /// # Errors
    /// The part exists, the name is not a valid part name, or the workbook is Strict Open XML.
    pub fn add_part(
        &mut self,
        name: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<PartName, XlsxError> {
        self.require_transitional()?;
        let name = name.trim_start_matches('/');
        let part = PartName::new(name)?;
        if self.package.part(name).is_some() {
            return Err(XlsxError::UnsupportedEdit(format!(
                "part {name} already exists"
            )));
        }
        self.edited()?;
        self.package.set_part(part.clone(), bytes);
        self.sync_content_type(name, content_type)?;
        self.parts_dirty = true;
        Ok(part)
    }

    fn sync_content_type(&mut self, name: &str, content_type: &str) -> Result<(), XlsxError> {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        if ext.as_deref() == Some("vml") && content_type == content_type::VML_DRAWING {
            parts::add_default(&mut self.package, "vml", content_type)?;
        }
        if parts::content_type_of(&self.package, name).as_deref() == Some(content_type) {
            return Ok(());
        }
        if parts::set_override_type(&mut self.package, name, content_type)? {
            return Ok(());
        }
        parts::add_override(&mut self.package, name, content_type)
    }

    /// Replaces the bytes of a part this module added or any part the model does not own; unlike
    /// [`Workbook::replace_part`] it can also write a part that does not exist yet (no content type).
    pub(crate) fn put_part_bytes(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), XlsxError> {
        self.package.set_part(PartName::new(name)?, bytes);
        self.parts_dirty = true;
        Ok(())
    }

    /// Adds a relationship from `source` (`None` is the package) and returns its id. `target` is
    /// written as given: relative to the source part for an internal one, a URI for an external one.
    ///
    /// # Errors
    /// The source part does not exist or the relationship part cannot be written.
    pub fn add_relationship(
        &mut self,
        source: Option<&PartName>,
        rel_type: &str,
        target: &str,
        mode: TargetMode,
    ) -> Result<String, XlsxError> {
        if let Some(src) = source
            && self.package.part(src.as_str()).is_none()
        {
            return Err(XlsxError::MissingPart(src.to_string()));
        }
        self.edited()?;
        let id = parts::add_relationship_mode(
            &mut self.package,
            source,
            rel_type,
            target,
            mode == TargetMode::External,
        )?;
        self.parts_dirty = true;
        Ok(id)
    }

    /// Removes one relationship (the target part is left alone).
    pub fn remove_relationship(
        &mut self,
        source: Option<&PartName>,
        id: &str,
    ) -> Result<(), XlsxError> {
        self.edited()?;
        parts::remove_relationships(&mut self.package, source, &[id.to_owned()])?;
        self.parts_dirty = true;
        Ok(())
    }

    /// Removes a part, its content-type override and its own relationship part.
    pub(crate) fn drop_part(&mut self, name: &str) {
        self.package.remove_part_and_override(name);
        let rels = rels_part_name(PartName::new(name).ok().as_ref());
        self.package.remove_part(rels.as_str());
        self.parts_dirty = true;
    }

    /// Replaces, inserts (in schema order) or removes a child of the `workbook` element.
    pub(crate) fn put_workbook_child(&mut self, local: &str, element: Option<XmlElement>) {
        const ORDER: [&str; 19] = [
            "fileVersion",
            "fileSharing",
            "workbookPr",
            "workbookProtection",
            "bookViews",
            "sheets",
            "functionGroups",
            "externalReferences",
            "definedNames",
            "calcPr",
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
        let at = self
            .root
            .children
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.head.is_main(local)));
        match (at, element) {
            (Some(i), Some(el)) => self.root.children[i] = xml::RawXml::Element(el),
            (Some(i), None) => {
                self.root.children.remove(i);
            }
            (None, Some(el)) => {
                let rank = |name: &str| ORDER.iter().position(|n| *n == name);
                let mine = rank(local).unwrap_or(ORDER.len());
                let pos = self
                    .root
                    .children
                    .iter()
                    .position(|n| {
                        n.as_element()
                            .filter(|e| e.head.ns.as_deref() == self.root.head.ns.as_deref())
                            .and_then(|e| rank(e.head.local_name()))
                            .is_some_and(|r| r > mine)
                    })
                    .unwrap_or(self.root.children.len());
                self.root.children.insert(pos, xml::RawXml::Element(el));
            }
            (None, None) => {}
        }
        self.workbook_dirty = true;
    }

    pub(crate) fn workbook_fragment(&self, text: &str) -> Result<XmlElement, XlsxError> {
        crate::sheet::parse_fragment(text, &self.root.head)
    }

    /// Rewrites every formula of the workbook (cells, conditional formats, validations, names, table
    /// formulas) with `shifter`. Returns the formula cells whose text changed.
    pub(crate) fn rewrite_all_formulas(
        &mut self,
        shifter: &dyn FormulaShifter,
    ) -> Vec<(usize, u32, u32)> {
        let mut changed = Vec::new();
        for (i, entry) in self.sheets.iter_mut().enumerate() {
            let Some(ws) = entry.data.as_mut() else {
                continue;
            };
            ws.materialise_all_shared();
            let hits = ws.shift_cell_formulas(shifter, &entry.info.name);
            ws.shift_raw_formulas(shifter, &entry.info.name, None);
            if !hits.is_empty() {
                entry.dirty = true;
            }
            // Raw formulas may have changed too; the sheet is always rewritten.
            entry.dirty = true;
            changed.extend(hits.into_iter().map(|(r, c)| (i, r, c)));
        }
        for el in self
            .root
            .children
            .iter_mut()
            .filter_map(xml::RawXml::as_element_mut)
            .filter(|e| e.head.is_main("definedNames"))
        {
            for child in &mut el.children {
                if let xml::RawXml::Element(name) = child
                    && name.head.is_main("definedName")
                    && let Some(new) = shifter.shift_name(name.own_text().trim_start_matches('='))
                {
                    name.children = vec![xml::RawXml::Text(new)];
                }
            }
        }
        for name in &mut self.defined_names {
            if let Some(new) = shifter.shift_name(&name.formula) {
                name.formula = new;
            }
        }
        self.workbook_dirty = true;
        changed
    }
}
