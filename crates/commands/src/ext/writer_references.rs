//! Writer References tab: tables of contents and figures, captions, footnotes and endnotes, the
//! index and the table of authorities. Each operation is one undo step (the command layer replays
//! the log to undo it) and only uses the public document API plus the field engine.
//!
//! Page numbers come from layout, which the command layer does not run: the caller passes the
//! page of every paragraph (`pages[paragraph]`, 1-based) it read from its own layout. That keeps
//! replay deterministic.

use std::collections::BTreeMap;

use open365_docx::{
    CivilDateTime, DocPos, Document, DocxError, Field, FieldContext, PageInfo, PageProvider,
    escape_xml,
};
use serde::{Deserialize, Serialize};

use crate::CommandError;
use crate::command::EffectClass;

/// Which fields an update touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldScope {
    /// Every field the engine can evaluate, the indexes and tables of authorities included.
    All,
    /// Tables of contents (`TOC` without `\c`).
    Toc,
    /// Tables of figures (`TOC \c`).
    Figures,
    /// Caption numbers (`SEQ`).
    Seq,
    /// `INDEX` fields.
    Index,
    /// `TOA` fields.
    Authorities,
}

/// One References-tab edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefOp {
    /// Table of contents at `at`: the optional title paragraph (style `TOC Heading`) and a `TOC`
    /// field, filled from the headings.
    InsertToc {
        at: DocPos,
        instruction: String,
        title: Option<String>,
        pages: Vec<u32>,
    },
    UpdateFields {
        scope: FieldScope,
        pages: Vec<u32>,
    },
    /// Remove Table of Contents: the table that holds paragraph `para`, with its title.
    RemoveToc {
        para: usize,
    },
    /// Footnote or endnote with its mark at `at`; `text` may be empty.
    InsertNote {
        at: DocPos,
        endnote: bool,
        text: String,
    },
    /// Caption paragraph (`Figure 1`) above or below the item at paragraph `para`; later
    /// captions of the label are renumbered.
    InsertCaption {
        para: usize,
        label: String,
        text: String,
        above: bool,
        pages: Vec<u32>,
    },
    InsertTableOfFigures {
        at: DocPos,
        label: String,
        pages: Vec<u32>,
    },
    /// Index entry (`XE` field, hidden text) at `at`.
    MarkIndexEntry {
        at: DocPos,
        entry: String,
        cross_reference: Option<String>,
    },
    InsertIndex {
        at: DocPos,
        pages: Vec<u32>,
    },
    /// Citation for the table of authorities (`TA` field, hidden text) at `at`.
    MarkCitation {
        at: DocPos,
        long: String,
        short: String,
        category: u8,
    },
    InsertTableOfAuthorities {
        at: DocPos,
        pages: Vec<u32>,
    },
}

impl RefOp {
    #[must_use]
    pub fn effect(&self) -> EffectClass {
        EffectClass::Edit
    }
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

fn bad(message: &str) -> CommandError {
    CommandError::Docx(DocxError::UnsupportedRange(message.to_owned()))
}

const CIVIL: CivilDateTime = CivilDateTime {
    year: 2000,
    month: 1,
    day: 1,
    hour: 0,
    minute: 0,
    second: 0,
};

/// Runs `op` on `doc`.
pub fn apply(doc: &mut Document, op: &RefOp) -> Result<(), CommandError> {
    match op {
        RefOp::InsertToc {
            at,
            instruction,
            title,
            pages,
        } => insert_toc(doc, *at, instruction, title.as_deref(), pages),
        RefOp::UpdateFields { scope, pages } => update_fields(doc, *scope, pages),
        RefOp::RemoveToc { para } => remove_toc(doc, *para),
        RefOp::InsertNote { at, endnote, text } => {
            doc.insert_note(*at, *endnote, text)?;
            Ok(())
        }
        RefOp::InsertCaption {
            para,
            label,
            text,
            above,
            pages,
        } => insert_caption(doc, *para, label, text, *above, pages),
        RefOp::InsertTableOfFigures { at, label, pages } => {
            ensure_styles(doc);
            let idx = insertion_index(doc, *at)?;
            let instr = format!("TOC \\h \\z \\c {}", quote(label));
            doc.insert_blocks_xml(
                idx,
                &field_paragraph(&instr, "No table of figures entries found."),
            )?;
            update_fields(doc, FieldScope::Figures, pages)
        }
        RefOp::MarkIndexEntry {
            at,
            entry,
            cross_reference,
        } => {
            let mut instr = format!("XE {}", quote(entry));
            if let Some(see) = cross_reference.as_deref().filter(|s| !s.is_empty()) {
                instr.push_str(&format!(" \\t {}", quote(see)));
            }
            doc.insert_inline_xml(*at, &hidden_field(&instr))?;
            Ok(())
        }
        RefOp::InsertIndex { at, pages } => {
            ensure_styles(doc);
            let idx = insertion_index(doc, *at)?;
            let instr = "INDEX \\h \"A\" \\c \"1\"";
            doc.insert_blocks_xml(idx, &field_paragraph(instr, "No index entries found."))?;
            update_fields(doc, FieldScope::Index, pages)
        }
        RefOp::MarkCitation {
            at,
            long,
            short,
            category,
        } => {
            let instr = format!(
                "TA \\l {} \\s {} \\c {}",
                quote(long),
                quote(short),
                (*category).clamp(1, 16)
            );
            doc.insert_inline_xml(*at, &hidden_field(&instr))?;
            Ok(())
        }
        RefOp::InsertTableOfAuthorities { at, pages } => {
            ensure_styles(doc);
            let idx = insertion_index(doc, *at)?;
            let instr = "TOA \\h \\c \"0\" \\p";
            doc.insert_blocks_xml(
                idx,
                &field_paragraph(instr, "No table of authorities entries found."),
            )?;
            update_fields(doc, FieldScope::Authorities, pages)
        }
    }
}

// ---- XML pieces -----------------------------------------------------------------------------

/// `text` in double quotes, a quote inside it turned into an apostrophe (the instruction syntax
/// has no portable escape for it).
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "'"))
}

fn text_run(text: &str) -> String {
    format!(
        "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
        escape_xml(text)
    )
}

fn marker(kind: &str, hidden: bool) -> String {
    format!(
        "<w:r>{}<w:fldChar w:fldCharType=\"{kind}\"/></w:r>",
        if hidden {
            "<w:rPr><w:vanish/></w:rPr>"
        } else {
            ""
        }
    )
}

fn instr_run(instruction: &str, hidden: bool) -> String {
    format!(
        "<w:r>{}<w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r>",
        if hidden {
            "<w:rPr><w:vanish/></w:rPr>"
        } else {
            ""
        },
        escape_xml(instruction)
    )
}

/// begin, instruction, separate, `result` (run-level XML), end.
fn complex_field(instruction: &str, result: &str) -> String {
    format!(
        "{}{}{}{result}{}",
        marker("begin", false),
        instr_run(instruction, false),
        marker("separate", false),
        marker("end", false)
    )
}

/// A field without a result (`XE`, `TA`), formatted as hidden text.
fn hidden_field(instruction: &str) -> String {
    format!(
        "{}{}{}",
        marker("begin", true),
        instr_run(instruction, true),
        marker("end", true)
    )
}

fn field_paragraph(instruction: &str, placeholder: &str) -> String {
    format!(
        "<w:p>{}</w:p>",
        complex_field(instruction, &text_run(placeholder))
    )
}

fn style_xml(id: &str, name: &str, based_on: &str, ppr: &str, rpr: &str, extra: &str) -> String {
    format!(
        "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/>\
<w:basedOn w:val=\"{based_on}\"/><w:next w:val=\"Normal\"/>{extra}\
{}{}</w:style>",
        if ppr.is_empty() {
            String::new()
        } else {
            format!("<w:pPr>{ppr}</w:pPr>")
        },
        if rpr.is_empty() {
            String::new()
        } else {
            format!("<w:rPr>{rpr}</w:rPr>")
        },
    )
}

/// The styles the generated tables and captions use, when the document lacks them.
fn ensure_styles(doc: &mut Document) {
    let styles = [
        (
            "Caption",
            style_xml(
                "Caption",
                "caption",
                "Normal",
                "<w:spacing w:after=\"200\" w:line=\"240\" w:lineRule=\"auto\"/>",
                "<w:i/><w:iCs/><w:color w:val=\"44546A\"/><w:sz w:val=\"18\"/><w:szCs w:val=\"18\"/>",
                "<w:uiPriority w:val=\"35\"/><w:unhideWhenUsed/><w:qFormat/>",
            ),
        ),
        (
            "TOCHeading",
            style_xml(
                "TOCHeading",
                "TOC Heading",
                "Normal",
                "<w:keepNext/><w:keepLines/><w:spacing w:before=\"240\" w:after=\"0\"/><w:outlineLvl w:val=\"9\"/>",
                "<w:rFonts w:asciiTheme=\"majorHAnsi\" w:eastAsiaTheme=\"majorEastAsia\" \
w:hAnsiTheme=\"majorHAnsi\" w:cstheme=\"majorBidi\"/><w:color w:val=\"2F5496\"/>\
<w:sz w:val=\"32\"/><w:szCs w:val=\"32\"/>",
                "<w:uiPriority w:val=\"39\"/><w:unhideWhenUsed/><w:qFormat/>",
            ),
        ),
        (
            "TableofFigures",
            style_xml(
                "TableofFigures",
                "table of figures",
                "Normal",
                "<w:spacing w:after=\"0\"/>",
                "",
                "<w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>",
            ),
        ),
        (
            "Index1",
            style_xml(
                "Index1",
                "index 1",
                "Normal",
                "<w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/><w:ind w:left=\"220\" w:hanging=\"220\"/>",
                "",
                "<w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>",
            ),
        ),
        (
            "Index2",
            style_xml(
                "Index2",
                "index 2",
                "Normal",
                "<w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/><w:ind w:left=\"440\" w:hanging=\"220\"/>",
                "",
                "<w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>",
            ),
        ),
        (
            "IndexHeading",
            style_xml(
                "IndexHeading",
                "index heading",
                "Normal",
                "",
                "<w:rFonts w:asciiTheme=\"majorHAnsi\" w:eastAsiaTheme=\"majorEastAsia\" \
w:hAnsiTheme=\"majorHAnsi\" w:cstheme=\"majorBidi\"/><w:b/><w:bCs/>",
                "<w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>",
            ),
        ),
        (
            "TableofAuthorities",
            style_xml(
                "TableofAuthorities",
                "table of authorities",
                "Normal",
                "<w:spacing w:after=\"0\"/><w:ind w:left=\"220\" w:hanging=\"220\"/>",
                "",
                "<w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>",
            ),
        ),
        (
            "TOAHeading",
            style_xml(
                "TOAHeading",
                "toa heading",
                "Normal",
                "<w:spacing w:before=\"120\"/>",
                "<w:rFonts w:asciiTheme=\"majorHAnsi\" w:eastAsiaTheme=\"majorEastAsia\" \
w:hAnsiTheme=\"majorHAnsi\" w:cstheme=\"majorBidi\"/><w:b/><w:bCs/>",
                "<w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>",
            ),
        ),
    ];
    for (id, xml) in styles {
        doc.ensure_style_xml(id, &xml);
    }
}

// ---- insertion ------------------------------------------------------------------------------

/// The paragraph index new block content at `at` goes before: the caret paragraph when the caret
/// is at its start, the next one when at its end; inside a paragraph it is split first.
fn insertion_index(doc: &mut Document, at: DocPos) -> Result<usize, DocxError> {
    let len =
        doc.paragraph(at.para)
            .map(|p| p.char_len())
            .ok_or(DocxError::ParagraphOutOfRange {
                index: at.para,
                count: doc.paragraph_count(),
            })?;
    if at.offset == 0 {
        Ok(at.para)
    } else if at.offset >= len {
        Ok(at.para + 1)
    } else {
        doc.split_paragraph(at)?;
        Ok(at.para + 1)
    }
}

fn insert_toc(
    doc: &mut Document,
    at: DocPos,
    instruction: &str,
    title: Option<&str>,
    pages: &[u32],
) -> Result<(), CommandError> {
    ensure_styles(doc);
    let idx = insertion_index(doc, at)?;
    let mut xml = String::new();
    if let Some(title) = title.filter(|t| !t.is_empty()) {
        xml.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"TOCHeading\"/></w:pPr>{}</w:p>",
            text_run(title)
        ));
    }
    xml.push_str(&field_paragraph(
        instruction,
        "No table of contents entries found.",
    ));
    doc.insert_blocks_xml(idx, &xml)?;
    update_fields(doc, FieldScope::Toc, pages)
}

fn insert_caption(
    doc: &mut Document,
    para: usize,
    label: &str,
    text: &str,
    above: bool,
    pages: &[u32],
) -> Result<(), CommandError> {
    if label.trim().is_empty() {
        return Err(bad("a caption needs a label"));
    }
    ensure_styles(doc);
    let identifier = if label.contains(' ') {
        quote(label)
    } else {
        label.to_owned()
    };
    let field = complex_field(&format!("SEQ {identifier} \\* ARABIC"), &text_run("1"));
    let xml = format!(
        "<w:p><w:pPr><w:pStyle w:val=\"Caption\"/>{}</w:pPr>{}{field}{}</w:p>",
        if above { "<w:keepNext/>" } else { "" },
        text_run(&format!("{label} ")),
        if text.is_empty() {
            String::new()
        } else {
            text_run(text)
        },
    );
    if para >= doc.paragraph_count() {
        return Err(CommandError::Docx(DocxError::ParagraphOutOfRange {
            index: para,
            count: doc.paragraph_count(),
        }));
    }
    if doc.paragraph_in_table(para) {
        doc.insert_blocks_around(para, !above, &xml)?;
    } else if above {
        doc.insert_blocks_xml(para, &xml)?;
    } else {
        doc.insert_blocks_xml(para + 1, &xml)?;
    }
    let ctx = FieldContext::new(CIVIL);
    let pages = ParaPages(pages);
    let ctx = ctx.with_pages(&pages);
    let targets: Vec<usize> = doc
        .fields()
        .iter()
        .filter(|f| {
            f.instruction.name == "SEQ"
                && f.instruction
                    .first_arg()
                    .is_some_and(|a| a.eq_ignore_ascii_case(label))
        })
        .map(|f| f.index)
        .collect();
    for index in targets {
        doc.update_field(index, &ctx)?;
    }
    Ok(())
}

fn remove_toc(doc: &mut Document, para: usize) -> Result<(), CommandError> {
    let hit = doc.fields().into_iter().find_map(|f| {
        let (first, last) = doc.field_paragraph_span(f.index)?;
        (f.depth == 0
            && f.instruction.name == "TOC"
            && !f.instruction.has_switch("c")
            && first <= para
            && para <= last)
            .then_some((first, last))
    });
    let Some((mut first, last)) = hit else {
        return Err(bad("the caret is not in a table of contents"));
    };
    let titled = first > 0
        && doc
            .paragraph_style_name(first - 1)
            .is_some_and(|n| n.eq_ignore_ascii_case("toc heading"));
    if titled {
        first -= 1;
    }
    doc.remove_paragraphs(first, last)?;
    Ok(())
}

// ---- updating -------------------------------------------------------------------------------

fn update_fields(doc: &mut Document, scope: FieldScope, pages: &[u32]) -> Result<(), CommandError> {
    let pages = ParaPages(pages);
    let ctx = FieldContext::new(CIVIL);
    let ctx = ctx.with_pages(&pages);
    if scope == FieldScope::All {
        doc.update_fields(&ctx)?;
        update_authority_and_index(doc, FieldScope::Index, &pages)?;
        return update_authority_and_index(doc, FieldScope::Authorities, &pages);
    }
    if matches!(scope, FieldScope::Index | FieldScope::Authorities) {
        return update_authority_and_index(doc, scope, &pages);
    }
    let wanted: Vec<usize> = doc
        .fields()
        .iter()
        .filter(|f| {
            f.depth == 0
                && match scope {
                    FieldScope::Toc => {
                        f.instruction.name == "TOC" && !f.instruction.has_switch("c")
                    }
                    FieldScope::Figures => {
                        f.instruction.name == "TOC" && f.instruction.has_switch("c")
                    }
                    FieldScope::Seq => f.instruction.name == "SEQ",
                    _ => false,
                }
        })
        .map(|f| f.index)
        .collect();
    // Last first: a table that grows moves only the fields after it.
    for index in wanted.into_iter().rev() {
        doc.update_field(index, &ctx)?;
    }
    Ok(())
}

/// Rebuilds every `INDEX` or `TOA` field: the engine leaves these two alone, so the result
/// paragraphs are generated here and replace the old ones.
fn update_authority_and_index(
    doc: &mut Document,
    scope: FieldScope,
    pages: &ParaPages<'_>,
) -> Result<(), CommandError> {
    let name = if scope == FieldScope::Index {
        "INDEX"
    } else {
        "TOA"
    };
    let all = doc.fields();
    let targets: Vec<&Field> = all
        .iter()
        .filter(|f| f.depth == 0 && f.instruction.name == name)
        .collect();
    for field in targets.into_iter().rev() {
        let first = field.paragraph;
        let last = doc
            .field_paragraph_span(field.index)
            .map_or(first, |(_, l)| l);
        let lines = if scope == FieldScope::Index {
            index_lines(&all, pages)
        } else {
            authority_lines(&all, pages, &field.instruction)
        };
        let xml = build_table(&field.instruction.text, &lines, scope == FieldScope::Index);
        let was_last = last + 1 >= doc.paragraph_count();
        doc.remove_paragraphs(first, last)?;
        let before = if was_last && first >= doc.paragraph_count() {
            doc.paragraph_count()
        } else {
            first
        };
        doc.insert_blocks_xml(before, &xml)?;
        // Later fields in `all` keep valid paragraph numbers: they are processed in reverse.
    }
    Ok(())
}

/// One generated paragraph: style id, text before the tab (or all of it) and the text after.
struct Line {
    style: &'static str,
    text: String,
    tab_text: Option<String>,
}

/// The value of switch `\name` in raw instruction text: a quoted string or a bare word. The
/// field parser only attaches values to switches it knows, and `XE` and `TA` are not among them.
fn raw_switch(text: &str, name: char) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut in_quote = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            in_quote = !in_quote;
        } else if !in_quote && c == '\\' && chars.get(i + 1) == Some(&name) {
            let mut j = i + 2;
            while chars.get(j) == Some(&' ') {
                j += 1;
            }
            let mut out = String::new();
            if chars.get(j) == Some(&'"') {
                j += 1;
                while let Some(&ch) = chars.get(j) {
                    if ch == '"' {
                        break;
                    }
                    out.push(ch);
                    j += 1;
                }
            } else {
                while let Some(&ch) = chars.get(j) {
                    if ch.is_whitespace() || ch == '\\' {
                        break;
                    }
                    out.push(ch);
                    j += 1;
                }
            }
            return Some(out);
        }
        i += 1;
    }
    None
}

fn page_of(pages: &ParaPages<'_>, para: usize) -> u32 {
    pages.0.get(para).copied().unwrap_or(1)
}

fn page_list(mut list: Vec<u32>) -> String {
    list.sort_unstable();
    list.dedup();
    list.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[allow(clippy::type_complexity)]
fn index_lines(all: &[Field], pages: &ParaPages<'_>) -> Vec<Line> {
    // (levels lowercase sort key, levels, page, see-text)
    let mut entries: Vec<(Vec<String>, Vec<String>, Option<u32>, Option<String>)> = Vec::new();
    for f in all.iter().filter(|f| f.instruction.name == "XE") {
        let Some(text) = f.instruction.first_arg() else {
            continue;
        };
        let levels: Vec<String> = text
            .split(':')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        if levels.is_empty() {
            continue;
        }
        let see = raw_switch(&f.instruction.text, 't');
        let page = see.is_none().then(|| page_of(pages, f.paragraph));
        entries.push((
            levels.iter().map(|l| l.to_lowercase()).collect(),
            levels,
            page,
            see,
        ));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    // Merge equal entries; every prefix of a sub-entry gets its own line.
    let mut merged: BTreeMap<Vec<String>, (Vec<String>, Vec<u32>, Option<String>)> =
        BTreeMap::new();
    for (key, levels, page, see) in entries {
        for depth in 1..key.len() {
            merged
                .entry(key[..depth].to_vec())
                .or_insert_with(|| (levels[..depth].to_vec(), Vec::new(), None));
        }
        let slot = merged
            .entry(key)
            .or_insert_with(|| (levels, Vec::new(), None));
        if let Some(p) = page {
            slot.1.push(p);
        }
        if see.is_some() {
            slot.2 = see;
        }
    }
    let mut out = Vec::new();
    let mut letter: Option<char> = None;
    for (key, (levels, pages, see)) in merged {
        let depth = key.len();
        if depth == 1 {
            let first = levels[0]
                .chars()
                .next()
                .map(|c| c.to_uppercase().next().unwrap_or(c));
            let heading = first.filter(|c| c.is_alphabetic()).unwrap_or('#');
            if letter != Some(heading) {
                letter = Some(heading);
                out.push(Line {
                    style: "IndexHeading",
                    text: heading.to_string(),
                    tab_text: None,
                });
            }
        }
        let name = levels.last().cloned().unwrap_or_default();
        let mut text = name;
        if let Some(see) = see {
            text.push_str(", ");
            text.push_str(&see);
        } else if !pages.is_empty() {
            text.push_str(", ");
            text.push_str(&page_list(pages));
        }
        out.push(Line {
            style: if depth == 1 { "Index1" } else { "Index2" },
            text,
            tab_text: None,
        });
    }
    out
}

fn authority_lines(
    all: &[Field],
    pages: &ParaPages<'_>,
    toa: &open365_docx::Instruction,
) -> Vec<Line> {
    const NAMES: [&str; 8] = [
        "",
        "Cases",
        "Statutes",
        "Other Authorities",
        "Rules",
        "Treatises",
        "Regulations",
        "Constitutional Provisions",
    ];
    let want: u8 = toa
        .switch_arg("c")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let passim = toa.has_switch("p");
    let mut groups: BTreeMap<u8, BTreeMap<String, (String, Vec<u32>)>> = BTreeMap::new();
    for f in all.iter().filter(|f| f.instruction.name == "TA") {
        let category: u8 = raw_switch(&f.instruction.text, 'c')
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(1);
        if want != 0 && category != want {
            continue;
        }
        let long = raw_switch(&f.instruction.text, 'l').unwrap_or_default();
        if long.is_empty() {
            continue;
        }
        let slot = groups
            .entry(category)
            .or_default()
            .entry(long.to_lowercase())
            .or_insert_with(|| (long, Vec::new()));
        slot.1.push(page_of(pages, f.paragraph));
    }
    let mut out = Vec::new();
    for (category, entries) in groups {
        out.push(Line {
            style: "TOAHeading",
            text: NAMES
                .get(usize::from(category))
                .map_or_else(|| format!("Category {category}"), |s| (*s).to_owned()),
            tab_text: None,
        });
        for (_, (long, mut list)) in entries {
            list.sort_unstable();
            list.dedup();
            let shown = if passim && list.len() > 5 {
                "passim".to_owned()
            } else {
                page_list(list)
            };
            out.push(Line {
                style: "TableofAuthorities",
                text: long,
                tab_text: Some(shown),
            });
        }
    }
    out
}

/// The paragraphs of an `INDEX` / `TOA` field: begin, instruction and separator at the start of
/// the first, the end marker at the end of the last.
fn build_table(instruction: &str, lines: &[Line], index: bool) -> String {
    let empty = if index {
        "No index entries found."
    } else {
        "No table of authorities entries found."
    };
    if lines.is_empty() {
        return field_paragraph(instruction.trim(), empty);
    }
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let tabs = if line.tab_text.is_some() {
            "<w:tabs><w:tab w:val=\"right\" w:leader=\"dot\" w:pos=\"9350\"/></w:tabs>"
        } else {
            ""
        };
        out.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"{}\"/>{tabs}</w:pPr>",
            line.style
        ));
        if i == 0 {
            out.push_str(&marker("begin", false));
            out.push_str(&instr_run(instruction.trim(), false));
            out.push_str(&marker("separate", false));
        }
        out.push_str(&text_run(&line.text));
        if let Some(tab) = &line.tab_text {
            out.push_str("<w:r><w:tab/></w:r>");
            out.push_str(&text_run(tab));
        }
        if i + 1 == lines.len() {
            out.push_str(&marker("end", false));
        }
        out.push_str("</w:p>");
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Write as _;

    use super::*;
    use crate::{Command, Target};

    const CT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;
    const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;
    const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style></w:styles>"#;

    fn docx(paras: &[(&str, &str)]) -> Vec<u8> {
        let mut body = String::new();
        for (style, text) in paras {
            body.push_str(&format!(
                "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"
            ));
        }
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr/></w:body></w:document>"
        );
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in [
            ("[Content_Types].xml", CT),
            ("_rels/.rels", RELS),
            ("word/document.xml", document.as_str()),
            ("word/_rels/document.xml.rels", DOC_RELS),
            ("word/styles.xml", STYLES),
        ] {
            zip.start_file(name, opts).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn texts(t: &Target) -> Vec<String> {
        let d = t.docx().unwrap();
        (0..d.paragraph_count())
            .map(|i| d.paragraph_text(i).unwrap())
            .collect()
    }

    fn run(t: &mut Target, op: RefOp) {
        t.apply(Command::DocxReferences { op }).unwrap();
    }

    fn sample() -> Target {
        Target::open_docx(&docx(&[
            ("Heading1", "Intro"),
            ("Normal", "Body about apples."),
            ("Heading2", "Details"),
            ("Normal", "End."),
        ]))
        .unwrap()
    }

    #[test]
    fn toc_is_built_from_headings_and_undone() {
        let mut t = sample();
        let before = texts(&t);
        run(
            &mut t,
            RefOp::InsertToc {
                at: DocPos::new(3, 4),
                instruction: r#"TOC \o "1-3" \h \z \u"#.to_owned(),
                title: Some("Contents".to_owned()),
                pages: vec![1, 1, 2, 2],
            },
        );
        let after = texts(&t);
        println!("{after:?}");
        assert!(after.contains(&"Contents".to_owned()));
        assert!(
            after
                .iter()
                .any(|x| x.starts_with("Intro") && x.ends_with('1'))
        );
        assert!(
            after
                .iter()
                .any(|x| x.starts_with("Details") && x.ends_with('2'))
        );
        t.undo().unwrap();
        assert_eq!(texts(&t), before);
    }

    #[test]
    fn toc_is_removed_with_its_title() {
        let mut t = sample();
        let before = texts(&t);
        run(
            &mut t,
            RefOp::InsertToc {
                at: DocPos::new(3, 4),
                instruction: r#"TOC \o "1-3" \h \z \u"#.to_owned(),
                title: Some("Contents".to_owned()),
                pages: vec![1, 1, 2, 2],
            },
        );
        run(&mut t, RefOp::RemoveToc { para: 5 });
        assert_eq!(texts(&t), before);
        assert!(
            t.apply(Command::DocxReferences {
                op: RefOp::RemoveToc { para: 0 }
            })
            .is_err()
        );
    }

    #[test]
    fn caption_numbers_follow_order() {
        let mut t = sample();
        run(
            &mut t,
            RefOp::InsertCaption {
                para: 1,
                label: "Figure".to_owned(),
                text: String::new(),
                above: false,
                pages: vec![1; 5],
            },
        );
        assert_eq!(texts(&t)[2], "Figure 1");
        run(
            &mut t,
            RefOp::InsertCaption {
                para: 0,
                label: "Figure".to_owned(),
                text: String::new(),
                above: false,
                pages: vec![1; 6],
            },
        );
        let after = texts(&t);
        assert_eq!(after[1], "Figure 1", "{after:?}");
        assert!(after.contains(&"Figure 2".to_owned()), "{after:?}");
    }

    #[test]
    fn notes_create_their_part() {
        let mut t = sample();
        run(
            &mut t,
            RefOp::InsertNote {
                at: DocPos::new(1, 4),
                endnote: false,
                text: "Source".to_owned(),
            },
        );
        let bytes = t.snapshot_bytes().unwrap();
        let doc = open365_docx::Document::open(&bytes, &open365_opc::Limits::default()).unwrap();
        assert_eq!(doc.note_references().len(), 1);
        let notes = doc.footnotes();
        assert_eq!(notes.len(), 3, "two separators and the note");
    }

    #[test]
    fn index_and_authorities_are_generated() {
        let mut t = sample();
        run(
            &mut t,
            RefOp::MarkIndexEntry {
                at: DocPos::new(1, 18),
                entry: "apples".to_owned(),
                cross_reference: None,
            },
        );
        run(
            &mut t,
            RefOp::MarkCitation {
                at: DocPos::new(3, 4),
                long: "End".to_owned(),
                short: "End".to_owned(),
                category: 1,
            },
        );
        run(
            &mut t,
            RefOp::InsertIndex {
                at: DocPos::new(3, 4),
                pages: vec![1, 1, 2, 2],
            },
        );
        run(
            &mut t,
            RefOp::InsertTableOfAuthorities {
                at: DocPos::new(3, 4),
                pages: vec![1, 1, 2, 2],
            },
        );
        let after = texts(&t);
        println!("{after:?}");
        assert!(after.contains(&"A".to_owned()));
        assert!(after.contains(&"apples, 1".to_owned()));
        assert!(after.contains(&"Cases".to_owned()));
        assert!(after.iter().any(|x| x.starts_with("End\t2")));
    }
}
