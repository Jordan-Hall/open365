//! More Writer Insert tab operations: cover pages, fields (the Field dialog, cross-references,
//! page and date fields), simple text boxes and quick tables. Each is one undo step (the caller
//! keeps the previous document). The designs are Open365's own.

use open365_docx::{
    CivilDateTime, DocPos, Document, DocxError, FieldContext, PageInfo, PageProvider, RawXml,
    XmlElement, escape_xml,
};
use serde::{Deserialize, Serialize};

use super::writer_xml::{self as x, fail, rewrite};
use crate::error::CommandError;

/// What a cross-reference shows (the dialog's "Insert reference to").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XRefKind {
    /// The text of the bookmark or heading (`REF name \h`).
    Text,
    /// The page number (`PAGEREF name \h`).
    Page,
    /// The paragraph number (`REF name \r \h`).
    Number,
    /// "above" or "below" (`REF name \p \h`).
    Position,
}

/// The cover page designs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverDesign {
    /// Centred title block.
    Classic,
    /// A dark band behind the title.
    Banner,
    /// Left aligned with an accent rule.
    Stripe,
}

impl CoverDesign {
    /// `classic`, `banner`, `stripe`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "classic" => Self::Classic,
            "banner" => Self::Banner,
            "stripe" => Self::Stripe,
            _ => return None,
        })
    }
}

/// What a [`crate::ext::writer_insert::WriterInsertOp::More`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum InsertMoreOp {
    /// A field with the code `instruction` at `at`, evaluated at once. `now` is the clock
    /// (year, month, day, hour, minute, second) and `pages[paragraph]` the 1-based page of each
    /// paragraph, both read by the caller so replay is deterministic.
    Field {
        at: DocPos,
        instruction: String,
        pages: Vec<u32>,
        now: (i32, u8, u8, u8, u8, u8),
    },
    /// A reference to a bookmark, or to the heading whose text is `target` (the heading gets a
    /// hidden `_Ref` bookmark).
    CrossReference {
        at: DocPos,
        target: String,
        kind: XRefKind,
        pages: Vec<u32>,
        now: (i32, u8, u8, u8, u8, u8),
    },
    /// An anchored text box (`wps`) with `text`, centred between the margins.
    TextBox {
        at: DocPos,
        text: String,
        width_pt: f64,
        height_pt: f64,
    },
    /// A cover page before the first paragraph, ending in a page break.
    CoverPage { design: CoverDesign },
    /// A table filled with `cells` (row by row; every row has `cols` cells).
    QuickTable {
        pos: DocPos,
        cells: Vec<Vec<String>>,
        text_width_pt: f64,
    },
}

struct ParaPages<'a>(&'a [u32]);

impl PageProvider for ParaPages<'_> {
    fn page_at(&self, pos: DocPos) -> Option<PageInfo> {
        let page = *self.0.get(pos.para)?;
        Some(PageInfo {
            page,
            number: page,
            label: None,
            section: 0,
        })
    }

    fn total_pages(&self) -> Option<u32> {
        self.0.iter().copied().max()
    }

    fn section_pages(&self, _section: usize) -> Option<u32> {
        None
    }
}

fn civil(now: (i32, u8, u8, u8, u8, u8)) -> CivilDateTime {
    CivilDateTime::new(now.0, now.1, now.2, now.3, now.4, now.5).unwrap_or(CivilDateTime {
        year: 2000,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    })
}

fn docx(e: DocxError) -> CommandError {
    CommandError::Docx(e)
}

/// Runs `op` on `doc`; `doc` is unchanged when it fails.
pub(crate) fn run(doc: &mut Document, op: &InsertMoreOp) -> Result<(), CommandError> {
    let before = doc.save()?;
    let result = apply(doc, op);
    if result.is_err()
        && let Ok(old) = Document::open(&before, &open365_opc::Limits::default())
    {
        *doc = old;
    }
    result
}

fn apply(doc: &mut Document, op: &InsertMoreOp) -> Result<(), CommandError> {
    match op {
        InsertMoreOp::Field {
            at,
            instruction,
            pages,
            now,
        } => insert_field(doc, *at, instruction, pages, *now),
        InsertMoreOp::CrossReference {
            at,
            target,
            kind,
            pages,
            now,
        } => cross_reference(doc, *at, target, *kind, pages, *now),
        InsertMoreOp::TextBox {
            at,
            text,
            width_pt,
            height_pt,
        } => text_box(doc, *at, text, *width_pt, *height_pt),
        InsertMoreOp::CoverPage { design } => {
            doc.insert_blocks_xml(0, &cover_page(*design))
                .map_err(docx)?;
            Ok(())
        }
        InsertMoreOp::QuickTable {
            pos,
            cells,
            text_width_pt,
        } => quick_table(doc, *pos, cells, *text_width_pt),
    }
}

// ---- fields ---------------------------------------------------------------------------------

fn field_runs(instruction: &str) -> String {
    format!(
        "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>\
<w:r><w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r>\
<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>\
<w:r><w:t xml:space=\"preserve\"></w:t></w:r>\
<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
        escape_xml(instruction)
    )
}

fn insert_field(
    doc: &mut Document,
    at: DocPos,
    instruction: &str,
    pages: &[u32],
    now: (i32, u8, u8, u8, u8, u8),
) -> Result<(), CommandError> {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err(fail("a field needs a code"));
    }
    doc.insert_inline_xml(at, &field_runs(instruction))
        .map_err(docx)?;
    let name = instruction.split_whitespace().next().unwrap_or_default();
    let target = doc
        .fields()
        .iter()
        .filter(|f| {
            f.paragraph == at.para
                && f.depth == 0
                && f.result_text.is_empty()
                && f.instruction.name.eq_ignore_ascii_case(name)
        })
        .map(|f| f.index)
        .next_back();
    if let Some(index) = target {
        let provider = ParaPages(pages);
        let ctx = FieldContext::new(civil(now));
        let ctx = ctx.with_pages(&provider);
        doc.update_field(index, &ctx).map_err(docx)?;
    }
    Ok(())
}

fn walk(e: &XmlElement, f: &mut dyn FnMut(&XmlElement)) {
    f(e);
    for c in &e.children {
        if let RawXml::Element(c) = c {
            walk(c, f);
        }
    }
}

fn cross_reference(
    doc: &mut Document,
    at: DocPos,
    target: &str,
    kind: XRefKind,
    pages: &[u32],
    now: (i32, u8, u8, u8, u8, u8),
) -> Result<(), CommandError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(fail("a cross-reference needs a bookmark or heading"));
    }
    let mut name = None::<String>;
    rewrite(doc, |m| {
        let mut names: Vec<String> = Vec::new();
        let mut max_id = 0i64;
        walk(m.root(), &mut |el| {
            if el.head.is_w("bookmarkStart") {
                if let Some(id) = el.head.attr_w("id").and_then(|v| v.parse::<i64>().ok()) {
                    max_id = max_id.max(id);
                }
                if let Some(n) = el.head.attr_w("name") {
                    names.push(n.to_owned());
                }
            }
        });
        if names.iter().any(|n| n == target) {
            name = Some(target.to_owned());
            return Ok(());
        }
        // A heading with this text gets a hidden bookmark (an existing one is reused).
        let count = x::paragraph_count(m.root());
        let mut found = None;
        for i in 0..count {
            let p = x::para_mut(m.root(), i)?;
            let is_heading = x::ppr(p)
                .and_then(|pp| x::child(pp, "pStyle"))
                .and_then(|s| s.head.attr_w("val"))
                .is_some_and(|v| v.to_ascii_lowercase().starts_with("heading"));
            if is_heading && x::para_text(p).trim() == target {
                found = Some(i);
                break;
            }
        }
        let Some(i) = found else {
            return Err(fail(format!("{target} is not a bookmark or heading")));
        };
        let p = x::para_mut(m.root(), i)?;
        let existing = p.children.iter().find_map(|c| match c {
            RawXml::Element(e) if e.head.is_w("bookmarkStart") => e
                .head
                .attr_w("name")
                .filter(|n| n.starts_with("_Ref"))
                .map(str::to_owned),
            _ => None,
        });
        if let Some(n) = existing {
            name = Some(n);
            return Ok(());
        }
        let id = max_id + 1;
        let bm = format!("_Ref{}", 100_000_000 + id);
        let mut start = x::w(&p.head, "bookmarkStart");
        start.head.set_attr_w("id", &id.to_string());
        start.head.set_attr_w("name", &bm);
        let mut end = x::w(&p.head, "bookmarkEnd");
        end.head.set_attr_w("id", &id.to_string());
        let len = x::char_len(p);
        x::insert_nodes_at(p, len, vec![RawXml::Element(end)]);
        x::insert_nodes_at(p, 0, vec![RawXml::Element(start)]);
        name = Some(bm);
        Ok(())
    })?;
    let name = name.ok_or_else(|| fail("no bookmark"))?;
    let instruction = match kind {
        XRefKind::Text => format!("REF {name} \\h"),
        XRefKind::Page => format!("PAGEREF {name} \\h"),
        XRefKind::Number => format!("REF {name} \\r \\h"),
        XRefKind::Position => format!("REF {name} \\p \\h"),
    };
    insert_field(doc, at, &instruction, pages, now)
}

// ---- text box -------------------------------------------------------------------------------

fn emu(pt: f64) -> i64 {
    // Clamped to a page-sized range; the cast is exact after rounding.
    #[allow(clippy::cast_possible_truncation)]
    let v = (pt.clamp(1.0, 1584.0) * 12_700.0).round() as i64;
    v
}

fn text_box(
    doc: &mut Document,
    at: DocPos,
    text: &str,
    width_pt: f64,
    height_pt: f64,
) -> Result<(), CommandError> {
    let main = String::from_utf8_lossy(&doc.serialize_main()).into_owned();
    let id = main
        .split("<wp:docPr id=\"")
        .skip(1)
        .filter_map(|r| r.split('"').next().and_then(|v| v.parse::<u64>().ok()))
        .max()
        .unwrap_or(0);
    let id = id + 1;
    let (cx, cy) = (emu(width_pt), emu(height_pt));
    let paragraphs: String = if text.is_empty() {
        "<w:p/>".to_owned()
    } else {
        text.split('\n')
            .map(|l| {
                format!(
                    "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                    escape_xml(l)
                )
            })
            .collect()
    };
    let xml = format!(
        "<w:r><w:drawing xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\">\
<wp:anchor distT=\"0\" distB=\"0\" distL=\"114300\" distR=\"114300\" simplePos=\"0\" relativeHeight=\"251659264\" behindDoc=\"0\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\">\
<wp:simplePos x=\"0\" y=\"0\"/>\
<wp:positionH relativeFrom=\"margin\"><wp:align>center</wp:align></wp:positionH>\
<wp:positionV relativeFrom=\"paragraph\"><wp:posOffset>0</wp:posOffset></wp:positionV>\
<wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
<wp:wrapSquare wrapText=\"bothSides\"/>\
<wp:docPr id=\"{id}\" name=\"Text Box {id}\"/><wp:cNvGraphicFramePr/>\
<a:graphic><a:graphicData uri=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\">\
<wps:wsp><wps:cNvSpPr txBox=\"1\"/>\
<wps:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>\
<a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill>\
<a:ln w=\"9525\"><a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill></a:ln></wps:spPr>\
<wps:txbx><w:txbxContent>{paragraphs}</w:txbxContent></wps:txbx>\
<wps:bodyPr rot=\"0\" vert=\"horz\" wrap=\"square\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" anchor=\"t\"><a:noAutofit/></wps:bodyPr>\
</wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"
    );
    doc.insert_inline_xml(at, &xml).map_err(docx)
}

// ---- cover page -----------------------------------------------------------------------------

fn cover_para(ppr: &str, rpr: &str, text: &str) -> String {
    let run = if text.is_empty() {
        String::new()
    } else {
        format!(
            "<w:r><w:rPr>{rpr}</w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r>",
            escape_xml(text)
        )
    };
    format!("<w:p><w:pPr>{ppr}</w:pPr>{run}</w:p>")
}

fn cover_page(design: CoverDesign) -> String {
    let page_break = "<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>";
    let body = match design {
        CoverDesign::Classic => [
            cover_para("<w:spacing w:before=\"3600\" w:after=\"0\"/>", "", ""),
            cover_para(
                "<w:spacing w:after=\"240\"/><w:jc w:val=\"center\"/>",
                "<w:b/><w:color w:val=\"1F3864\"/><w:sz w:val=\"72\"/><w:szCs w:val=\"72\"/>",
                "[Document title]",
            ),
            cover_para(
                "<w:spacing w:after=\"2400\"/><w:jc w:val=\"center\"/>",
                "<w:color w:val=\"595959\"/><w:sz w:val=\"36\"/><w:szCs w:val=\"36\"/>",
                "[Document subtitle]",
            ),
            cover_para(
                "<w:spacing w:after=\"0\"/><w:jc w:val=\"center\"/>",
                "<w:sz w:val=\"28\"/><w:szCs w:val=\"28\"/>",
                "[Author name]",
            ),
            cover_para(
                "<w:jc w:val=\"center\"/>",
                "<w:color w:val=\"595959\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/>",
                "[Date]",
            ),
        ]
        .concat(),
        CoverDesign::Banner => [
            cover_para("<w:spacing w:before=\"2400\" w:after=\"0\"/>", "", ""),
            cover_para(
                "<w:pBdr><w:top w:val=\"single\" w:sz=\"48\" w:space=\"12\" w:color=\"1F3864\"/>\
<w:left w:val=\"single\" w:sz=\"48\" w:space=\"12\" w:color=\"1F3864\"/>\
<w:bottom w:val=\"single\" w:sz=\"48\" w:space=\"12\" w:color=\"1F3864\"/>\
<w:right w:val=\"single\" w:sz=\"48\" w:space=\"12\" w:color=\"1F3864\"/></w:pBdr>\
<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"1F3864\"/><w:spacing w:after=\"360\"/>",
                "<w:b/><w:color w:val=\"FFFFFF\"/><w:sz w:val=\"64\"/><w:szCs w:val=\"64\"/>",
                "[Document title]",
            ),
            cover_para(
                "<w:spacing w:after=\"3000\"/>",
                "<w:color w:val=\"1F3864\"/><w:sz w:val=\"36\"/><w:szCs w:val=\"36\"/>",
                "[Document subtitle]",
            ),
            cover_para(
                "<w:spacing w:after=\"0\"/>",
                "<w:b/><w:sz w:val=\"28\"/><w:szCs w:val=\"28\"/>",
                "[Author name]",
            ),
            cover_para(
                "",
                "<w:color w:val=\"595959\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/>",
                "[Date]",
            ),
        ]
        .concat(),
        CoverDesign::Stripe => [
            cover_para("<w:spacing w:before=\"4200\" w:after=\"0\"/>", "", ""),
            cover_para(
                "<w:pBdr><w:left w:val=\"single\" w:sz=\"96\" w:space=\"18\" w:color=\"2E75B6\"/></w:pBdr>\
<w:spacing w:after=\"120\"/><w:ind w:left=\"360\"/>",
                "<w:b/><w:sz w:val=\"80\"/><w:szCs w:val=\"80\"/>",
                "[Document title]",
            ),
            cover_para(
                "<w:pBdr><w:left w:val=\"single\" w:sz=\"96\" w:space=\"18\" w:color=\"2E75B6\"/></w:pBdr>\
<w:spacing w:after=\"2800\"/><w:ind w:left=\"360\"/>",
                "<w:color w:val=\"2E75B6\"/><w:sz w:val=\"36\"/><w:szCs w:val=\"36\"/>",
                "[Document subtitle]",
            ),
            cover_para(
                "<w:spacing w:after=\"0\"/><w:ind w:left=\"360\"/>",
                "<w:sz w:val=\"26\"/><w:szCs w:val=\"26\"/>",
                "[Author name]",
            ),
            cover_para(
                "<w:ind w:left=\"360\"/>",
                "<w:color w:val=\"595959\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/>",
                "[Date]",
            ),
        ]
        .concat(),
    };
    format!("{body}{page_break}")
}

// ---- quick tables ---------------------------------------------------------------------------

fn quick_table(
    doc: &mut Document,
    pos: DocPos,
    cells: &[Vec<String>],
    text_width_pt: f64,
) -> Result<(), CommandError> {
    let rows = cells.len();
    let cols = cells.first().map_or(0, Vec::len);
    if rows == 0 || cols == 0 || cells.iter().any(|r| r.len() != cols) {
        return Err(fail("a quick table needs rows of equal length"));
    }
    doc.insert_table(
        pos,
        rows,
        cols,
        text_width_pt,
        open365_docx::AutoFitMode::Fixed,
    )
    .map_err(docx)?;
    let first = if pos.offset > 0 {
        pos.para + 1
    } else {
        pos.para
    };
    for (i, text) in cells.iter().flatten().enumerate() {
        if !text.is_empty() {
            doc.insert_text(first + i, 0, text).map_err(docx)?;
        }
    }
    Ok(())
}
