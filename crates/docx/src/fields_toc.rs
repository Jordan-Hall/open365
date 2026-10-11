//! TOC generation: the entries a `TOC` field shows (heading styles, outline levels, `\t`
//! styles, `TC` fields, `SEQ` captions), and the paragraphs Word writes for them.
//!
//! The rules were measured against Word (`tools/office/fields-oracle.ps1`, set `toc`); see
//! `docs/architecture/docx-fields.md`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::DocxError;
use crate::document::Document;
use crate::fields::{Instruction, SkipReason};
use crate::fields_eval::{Evaluator, Outcome, View};
use crate::fields_scan::{RawField, Scanner, blocks_mut, paragraph_at, slots};
use crate::fields_snap::{Snapshot, StyleTable};
use crate::fields_toc_styles::TOC_STYLES;
use crate::fields_update::{Policy, merge_region};
use crate::model::{Block, Inline, Paragraph, blocks_from};
use crate::ranges::DocPos;
use crate::xml::{self, RawXml, W_NS};

/// One line of the table.
#[derive(Debug, Clone)]
pub(crate) struct TocEntry {
    /// 1-based level, which picks the style `TOC n`.
    pub level: usize,
    /// Numbering label of the source paragraph, shown before the text.
    pub label: Option<String>,
    pub text: String,
    /// The paragraph the entry points at (numbering of the story as it is now).
    pub para: usize,
    /// Page number text; `None` when the entry has no page number.
    pub page: Option<String>,
}

/// The entries a TOC field will show and how to lay them out.
#[derive(Debug, Clone, Default)]
pub(crate) struct TocPlan {
    pub entries: Vec<TocEntry>,
    /// `\h`: entries are hyperlinks.
    pub hyperlink: bool,
    /// `\z`: tab leader and page numbers are hidden in web layout.
    pub hide_web: bool,
    /// `\p`: text between the entry and its page number instead of a right-aligned tab.
    pub separator: Option<String>,
    /// `\c`: a table of figures (style `Table of Figures`).
    pub figures: bool,
    /// What the field shows when there are no entries.
    pub message: String,
}

impl TocPlan {
    /// The visible result: one line per entry, each ended by `\r`.
    pub(crate) fn result_text(&self) -> String {
        if self.entries.is_empty() {
            return self.message.clone();
        }
        let mut out = String::new();
        for e in &self.entries {
            if let Some(label) = &e.label {
                out.push_str(label);
                out.push('\t');
            }
            out.push_str(&e.text);
            if let Some(page) = &e.page {
                out.push_str(self.separator.as_deref().unwrap_or("\t"));
                out.push_str(page);
            }
            out.push('\r');
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

fn switch<'a>(inst: &'a Instruction, name: &str) -> Option<&'a crate::fields::Switch> {
    inst.switches
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
}

/// `1-3` or `2`, clamped to 1..=9.
fn parse_range(text: &str) -> Option<(usize, usize)> {
    let mut parts = text.trim().splitn(2, '-');
    let lo: usize = parts.next()?.trim().parse().ok()?;
    let hi: usize = match parts.next() {
        Some(h) => h.trim().parse().ok()?,
        None => lo,
    };
    (lo >= 1).then_some((lo.min(9), hi.clamp(lo.min(9), 9)))
}

/// `Quote,3,Heading 4,2` (the list separator may also be `;`).
fn parse_style_levels(text: &str) -> Vec<(String, usize)> {
    let parts: Vec<&str> = text.split([',', ';']).map(str::trim).collect();
    parts
        .chunks(2)
        .filter_map(|pair| {
            let level: usize = pair.get(1)?.parse().ok()?;
            (!pair[0].is_empty() && (1..=9).contains(&level)).then(|| (pair[0].to_owned(), level))
        })
        .collect()
}

fn heading_number(name: &str) -> Option<usize> {
    let n: usize = name
        .to_ascii_lowercase()
        .strip_prefix("heading ")?
        .trim()
        .parse()
        .ok()?;
    (1..=9).contains(&n).then_some(n)
}

pub(crate) fn plan(
    ev: &Evaluator<'_>,
    view: &View<'_>,
    index: usize,
    inst: &Instruction,
) -> Outcome {
    let snap = view.snap();
    let styles = &ev.styles;
    let o = switch(inst, "o");
    let use_outline = switch(inst, "u").is_some();
    let t_list = switch(inst, "t")
        .and_then(|s| s.arg.as_deref())
        .map(parse_style_levels)
        .unwrap_or_default();
    let tc_f = switch(inst, "f");
    let tc_l = switch(inst, "l");
    let figures_id = switch(inst, "c").and_then(|s| s.arg.clone());
    let keep_tabs = switch(inst, "w").is_some();
    let keep_breaks = switch(inst, "x").is_some();
    let hyperlink = switch(inst, "h").is_some();
    let hide_web = switch(inst, "z").is_some();
    let separator = switch(inst, "p").and_then(|s| s.arg.clone());
    let no_page = switch(inst, "n");
    let no_page_all = no_page.is_some_and(|s| s.arg.is_none());
    let no_page_range = no_page.and_then(|s| s.arg.as_deref()).and_then(parse_range);
    let heading_range = match o {
        Some(s) => s.arg.as_deref().and_then(parse_range).unwrap_or((1, 9)),
        None if use_outline => (1, 9),
        None => (0, 0),
    };
    let tc_range = tc_l
        .and_then(|s| s.arg.as_deref())
        .and_then(parse_range)
        .unwrap_or((1, 9));
    let bookmark_range = match switch(inst, "b").and_then(|s| s.arg.clone()) {
        Some(name) => match snap.bookmark(&name) {
            Some(b) => Some((b.start.para, b.end.para)),
            None => {
                return Outcome::Text {
                    text: "Error! Bookmark not defined.".to_owned(),
                    merge: false,
                };
            }
        },
        None => None,
    };
    let in_bookmark = |para: usize| bookmark_range.is_none_or(|(a, b)| para >= a && para <= b);
    let clean = |text: &str| -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            match c {
                '\t' if !keep_tabs => out.push(' '),
                '\u{b}' if !keep_breaks => out.push(' '),
                other => out.push(other),
            }
        }
        out
    };

    // (para, kind, order, entry without its page)
    let mut found: Vec<(usize, u8, usize, TocEntry, bool)> = Vec::new();
    let figures = figures_id.is_some();
    if let Some(id) = &figures_id {
        let mut seen: Vec<usize> = Vec::new();
        for raw in view.raws {
            let other = Instruction::parse(&raw.instr);
            if other.name == "SEQ"
                && other
                    .first_arg()
                    .is_some_and(|a| a.eq_ignore_ascii_case(id))
                && !seen.contains(&raw.begin.para)
                && in_bookmark(raw.begin.para)
            {
                seen.push(raw.begin.para);
            }
        }
        for para in seen {
            let Some(info) = snap.paras.get(para) else {
                continue;
            };
            let text = clean(&info.text);
            if text.trim().is_empty() {
                continue;
            }
            found.push((
                para,
                0,
                0,
                TocEntry {
                    level: 1,
                    label: None,
                    text,
                    para,
                    page: None,
                },
                true,
            ));
        }
    } else {
        for (para, info) in snap.paras.iter().enumerate() {
            if !in_bookmark(para) {
                continue;
            }
            let style_id = info
                .style_id
                .as_deref()
                .or(styles.default_paragraph.as_deref());
            let mut level: Option<usize> = None;
            if let Some(id) = style_id {
                let name = styles.name_of(id);
                level = t_list
                    .iter()
                    .find(|(s, _)| s.eq_ignore_ascii_case(&name) || s.eq_ignore_ascii_case(id))
                    .map(|(_, l)| *l);
            }
            let in_range = |n: usize| n >= heading_range.0 && n <= heading_range.1;
            if level.is_none() && use_outline {
                if let Some(direct) = info.outline {
                    if direct < 9 && in_range(usize::from(direct) + 1) {
                        level = Some(usize::from(direct) + 1);
                    }
                } else if let Some(l) = style_id.and_then(|id| styles.outline_of(id))
                    && l < 9
                    && in_range(usize::from(l) + 1)
                {
                    level = Some(usize::from(l) + 1);
                }
            }
            if level.is_none()
                && o.is_some()
                && let Some(n) = style_id.and_then(|id| heading_number(&styles.name_of(id)))
                && in_range(n)
                && info.outline.is_none_or(|d| d < 9 || !use_outline)
            {
                level = Some(n);
            }
            let Some(level) = level else { continue };
            let text = clean(&info.text);
            if text.trim().is_empty() {
                continue;
            }
            found.push((
                para,
                0,
                0,
                TocEntry {
                    level,
                    label: info.label.clone(),
                    text,
                    para,
                    page: None,
                },
                true,
            ));
        }
        if tc_f.is_some() || tc_l.is_some() {
            let want_id = tc_f
                .and_then(|s| s.arg.clone())
                .unwrap_or_else(|| "C".to_owned());
            for (i, raw) in view.raws.iter().enumerate() {
                if i == index || !in_bookmark(raw.begin.para) {
                    continue;
                }
                let tc = Instruction::parse(&raw.instr);
                if tc.name != "TC" {
                    continue;
                }
                let id = tc.switch_arg("f").unwrap_or("C");
                if !id.eq_ignore_ascii_case(&want_id) {
                    continue;
                }
                let level = tc
                    .switch_arg("l")
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 9);
                if level < tc_range.0 || level > tc_range.1 {
                    continue;
                }
                let Some(text) = tc.first_arg().map(clean) else {
                    continue;
                };
                if text.trim().is_empty() {
                    continue;
                }
                found.push((
                    raw.begin.para,
                    1,
                    i,
                    TocEntry {
                        level,
                        label: None,
                        text,
                        para: raw.begin.para,
                        page: None,
                    },
                    !tc.has_switch("n"),
                ));
            }
        }
    }
    found.sort_by_key(|(para, kind, order, _, _)| (*para, *kind, *order));
    let mut entries = Vec::with_capacity(found.len());
    for (_, _, _, mut entry, wants_page) in found {
        let hidden =
            no_page_all || no_page_range.is_some_and(|(a, b)| entry.level >= a && entry.level <= b);
        if wants_page && !hidden {
            let pos = DocPos {
                para: entry.para,
                offset: 0,
            };
            let Some(info) = view.page_at(ev.ctx, pos) else {
                return Outcome::Skip(SkipReason::MissingInput);
            };
            entry.page = Some(info.label.unwrap_or_else(|| info.number.to_string()));
        }
        entries.push(entry);
    }
    let message = if figures {
        "No table of figures entries found."
    } else {
        "No table of contents entries found."
    };
    Outcome::Toc(TocPlan {
        entries,
        hyperlink,
        hide_web,
        separator,
        figures,
        message: message.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// A run holding `text`; tabs and line breaks become `w:tab` and `w:br`.
fn text_run(text: &str, rpr: &str) -> String {
    let mut out = String::from("<w:r>");
    if !rpr.is_empty() {
        let _ = write!(out, "<w:rPr>{rpr}</w:rPr>");
    }
    let mut current = String::new();
    let flush = |current: &mut String, out: &mut String| {
        if !current.is_empty() {
            let _ = write!(out, "<w:t xml:space=\"preserve\">{}</w:t>", esc(current));
            current.clear();
        }
    };
    for c in text.chars() {
        match c {
            '\t' => {
                flush(&mut current, &mut out);
                out.push_str("<w:tab/>");
            }
            '\u{b}' | '\n' => {
                flush(&mut current, &mut out);
                out.push_str("<w:br/>");
            }
            c => current.push(c),
        }
    }
    flush(&mut current, &mut out);
    out.push_str("</w:r>");
    out
}

fn field_run(rpr: &str, inner: &str) -> String {
    format!("<w:r><w:rPr>{rpr}</w:rPr>{inner}</w:r>")
}

fn pageref_field(rpr: &str, bookmark: &str, page: &str) -> String {
    let mut out = String::new();
    out.push_str(&field_run(rpr, "<w:fldChar w:fldCharType=\"begin\"/>"));
    out.push_str(&field_run(
        rpr,
        &format!(
            "<w:instrText xml:space=\"preserve\"> PAGEREF {} \\h </w:instrText>",
            esc(bookmark)
        ),
    ));
    out.push_str(&field_run(rpr, "<w:fldChar w:fldCharType=\"separate\"/>"));
    out.push_str(&text_run(page, rpr));
    out.push_str(&field_run(rpr, "<w:fldChar w:fldCharType=\"end\"/>"));
    out
}

/// The paragraph Word writes for one entry.
fn entry_xml(plan: &TocPlan, entry: &TocEntry, bookmark: &str, right_tab: i64) -> String {
    let style = if plan.figures {
        "TableofFigures".to_owned()
    } else {
        format!("TOC{}", entry.level)
    };
    let mut tabs = String::new();
    if let Some(label) = &entry.label
        && !plan.figures
    {
        let indent = 220 * (i64::try_from(entry.level).unwrap_or(1) - 1);
        let extra = i64::try_from(label.chars().count().saturating_sub(1)).unwrap_or(0);
        let _ = write!(
            tabs,
            "<w:tab w:val=\"left\" w:pos=\"{}\"/>",
            indent + 440 + 150 * extra
        );
    }
    if entry.page.is_some() && plan.separator.is_none() {
        let _ = write!(
            tabs,
            "<w:tab w:val=\"right\" w:leader=\"dot\" w:pos=\"{right_tab}\"/>"
        );
    }
    let mut x = format!("<w:pPr><w:pStyle w:val=\"{style}\"/>");
    if !tabs.is_empty() {
        let _ = write!(x, "<w:tabs>{tabs}</w:tabs>");
    }
    x.push_str("<w:rPr><w:noProof/></w:rPr></w:pPr>");
    let plain = "<w:noProof/>";
    let link = if plan.hyperlink {
        "<w:rStyle w:val=\"Hyperlink\"/><w:noProof/>"
    } else {
        plain
    };
    let web = if plan.hide_web {
        "<w:noProof/><w:webHidden/>"
    } else {
        plain
    };
    let mut inner = String::new();
    if let Some(label) = &entry.label {
        inner.push_str(&text_run(label, link));
        inner.push_str(&text_run("\t", plain));
    }
    inner.push_str(&text_run(&entry.text, link));
    if let Some(page) = &entry.page {
        match &plan.separator {
            Some(sep) => inner.push_str(&text_run(sep, web)),
            None => inner.push_str(&text_run("\t", web)),
        }
        inner.push_str(&pageref_field(web, bookmark, page));
    }
    if plan.hyperlink {
        let _ = write!(
            x,
            "<w:hyperlink w:anchor=\"{}\" w:history=\"1\">{inner}</w:hyperlink>",
            esc(bookmark)
        );
    } else {
        x.push_str(&inner);
    }
    x
}

/// Parses `inner` as the content of a paragraph (`w:pPr` first, then runs).
fn parse_paragraph(inner: &str, w_prefix: bool) -> Option<Paragraph> {
    let wrapped = format!("<w:p xmlns:w=\"{W_NS}\">{inner}</w:p>");
    let mut root = xml::parse(wrapped.as_bytes()).ok()?.root;
    if w_prefix {
        root.head.attrs.retain(|a| a.name != "xmlns:w");
    }
    match blocks_from(vec![RawXml::Element(root)]).pop()? {
        Block::Paragraph(p) => Some(p),
        _ => None,
    }
}

/// Adds the TOC styles the plan needs when the document lacks them.
fn ensure_styles(doc: &mut Document, plan: &TocPlan) {
    let Some(styles) = doc.styles() else { return };
    let mut wanted: Vec<String> = Vec::new();
    if plan.figures {
        wanted.push("TableofFigures".to_owned());
    } else {
        for e in &plan.entries {
            let id = format!("TOC{}", e.level);
            if !wanted.contains(&id) {
                wanted.push(id);
            }
        }
    }
    if plan.hyperlink && !plan.entries.is_empty() {
        wanted.push("Hyperlink".to_owned());
    }
    wanted.retain(|id| !styles.has_style(id));
    if wanted.is_empty() {
        return;
    }
    if styles.has_style("Heading1") && !styles.has_style("TOCHeading") {
        wanted.push("TOCHeading".to_owned());
    }
    let Ok(text) = String::from_utf8(styles.serialize()) else {
        return;
    };
    let Some(close) = text.rfind("</w:styles>") else {
        return;
    };
    let mut added = String::new();
    for (id, xml) in TOC_STYLES {
        if wanted.iter().any(|w| w == id) {
            added.push_str(xml);
        }
    }
    if added.is_empty() {
        return;
    }
    let mut out = String::with_capacity(text.len() + added.len());
    out.push_str(&text[..close]);
    out.push_str(&added);
    out.push_str(&text[close..]);
    doc.styles_mut(out.into_bytes());
}

/// A `_Toc` bookmark around the text of each source paragraph (reusing one that is already
/// there). Returns the bookmark name per entry.
fn ensure_bookmarks(doc: &mut Document, plan: &TocPlan) -> Result<Vec<String>, DocxError> {
    let styles = StyleTable::load(doc);
    let snap = Snapshot::build(doc, &styles);
    let mut max_id = 0u64;
    let mut max_toc = 0u64;
    for b in &snap.bookmarks {
        if let Ok(id) = b.id.parse::<u64>() {
            max_id = max_id.max(id);
        }
        if let Some(n) = b
            .name
            .strip_prefix("_Toc")
            .and_then(|d| d.parse::<u64>().ok())
        {
            max_toc = max_toc.max(n);
        }
    }
    let mut by_para: BTreeMap<usize, String> = BTreeMap::new();
    let mut created: Vec<(usize, String, u64)> = Vec::new();
    for entry in &plan.entries {
        if by_para.contains_key(&entry.para) {
            continue;
        }
        let len = doc.paragraph(entry.para).map_or(0, Paragraph::char_len);
        let existing = snap.bookmarks.iter().find(|b| {
            b.name.starts_with("_Toc")
                && b.start.para == entry.para
                && b.start.offset == 0
                && b.end.para == entry.para
                && b.end.offset == len
                && !b.covers_mark
        });
        if let Some(b) = existing {
            by_para.insert(entry.para, b.name.clone());
            continue;
        }
        max_toc = if max_toc == 0 { 100_000_000 } else { max_toc } + 1;
        max_id += 1;
        let name = format!("_Toc{max_toc}");
        by_para.insert(entry.para, name.clone());
        created.push((entry.para, name, max_id));
    }
    if !created.is_empty() {
        let all = slots(doc.body());
        for (para, name, id) in &created {
            let slot = all
                .get(*para)
                .ok_or_else(|| DocxError::Xml("TOC source paragraph missing".to_owned()))?;
            let p = paragraph_at(doc.body_mut(), slot)
                .ok_or_else(|| DocxError::Xml("TOC source paragraph missing".to_owned()))?;
            let mut start = p.head.sibling("bookmarkStart");
            start.set_attr_w("id", &id.to_string());
            start.set_attr_w("name", name);
            let mut end = p.head.sibling("bookmarkEnd");
            end.set_attr_w("id", &id.to_string());
            p.content
                .insert(0, Inline::Raw(RawXml::Element(start.into_empty())));
            p.content
                .push(Inline::Raw(RawXml::Element(end.into_empty())));
        }
        doc.edited();
    }
    Ok(plan
        .entries
        .iter()
        .map(|e| by_para.get(&e.para).cloned().unwrap_or_default())
        .collect())
}

/// Width of the text column of the section holding paragraph `para`, less what Word leaves.
fn right_tab(doc: &Document, para: usize) -> i64 {
    let sections = doc.sections();
    let section = sections
        .iter()
        .find(|s| s.end_paragraph.is_none_or(|e| e >= para))
        .or_else(|| sections.last());
    let width = section.map_or(9026, |s| {
        i64::from(s.page.width_twips)
            - i64::from(s.margins.left_twips)
            - i64::from(s.margins.right_twips)
            - i64::from(s.margins.gutter_twips)
    });
    (if width > 0 { width } else { 9026 }) - 10
}

/// Replaces the result of the TOC field `index` with the entries of `plan`.
pub(crate) fn write_toc(doc: &mut Document, index: usize, plan: &TocPlan) -> Result<(), DocxError> {
    ensure_styles(doc, plan);
    let bookmarks = ensure_bookmarks(doc, plan)?;
    let structured = |raw: &RawField| {
        !raw.simple && raw.separate.as_ref().is_some_and(|s| s.path.len() == 1) && raw.end.is_some()
    };
    let mut raws = Scanner::scan(doc.body());
    let mut all_slots = slots(doc.body());
    let mut raw = raws
        .get(index)
        .cloned()
        .ok_or_else(|| DocxError::Xml("TOC field missing".to_owned()))?;
    if !structured(&raw) {
        // A simple field, or one without a separator: write the text first, which turns it
        // into a complex field, then lay the paragraphs out.
        let result = plan.result_text();
        let text = result.strip_suffix('\r').unwrap_or(&result);
        doc.write_result(&all_slots, &raws, index, text, Policy::Code)?;
        raws = Scanner::scan(doc.body());
        all_slots = slots(doc.body());
        raw = raws
            .get(index)
            .cloned()
            .ok_or_else(|| DocxError::Xml("TOC field missing".to_owned()))?;
        if !structured(&raw) {
            return Ok(());
        }
    }
    let (Some(sep), Some(end)) = (raw.separate.clone(), raw.end.clone()) else {
        return Ok(());
    };
    let (Some(sep_slot), Some(end_slot)) = (
        all_slots.get(sep.para).cloned(),
        all_slots.get(end.para).cloned(),
    ) else {
        return Err(DocxError::Xml(
            "field marker outside the paragraph list".to_owned(),
        ));
    };
    if sep_slot.steps != end_slot.steps || sep_slot.pos > end_slot.pos {
        return Err(DocxError::Xml(
            "field spans different containers".to_owned(),
        ));
    }
    let w_prefix = doc.body().head.name.starts_with("w:");
    let tab = right_tab(doc, raw.begin.para);
    let mut built: Vec<Paragraph> = Vec::new();
    for (entry, bookmark) in plan.entries.iter().zip(&bookmarks) {
        let p = parse_paragraph(&entry_xml(plan, entry, bookmark, tab), w_prefix)
            .ok_or_else(|| DocxError::Xml("generated TOC paragraph does not parse".to_owned()))?;
        built.push(p);
    }
    let message = if plan.entries.is_empty() {
        let run = text_run(&plan.message, "");
        parse_paragraph(&run, w_prefix).map(|p| p.content)
    } else {
        None
    };
    let steps = sep_slot.steps.clone();
    {
        let blocks = blocks_mut(doc.body_mut(), &steps)
            .ok_or_else(|| DocxError::Xml("bad field container".to_owned()))?;
        let (end_head, end_props) = match blocks.get(end_slot.pos) {
            Some(Block::Paragraph(p)) => (p.head.clone(), p.props.clone()),
            _ => return Err(DocxError::Xml("marker paragraph missing".to_owned())),
        };
        merge_region(blocks, &sep, &end, &sep_slot, &end_slot)?;
        let Some(Block::Paragraph(p)) = blocks.get_mut(sep_slot.pos) else {
            return Err(DocxError::Xml("marker paragraph missing".to_owned()));
        };
        let at = (sep.path[0] + 1).min(p.content.len());
        let tail = p.content.split_off(at);
        let mut replacement: Vec<Block> = Vec::new();
        if let Some(content) = message {
            let mut first = p.clone();
            first.props = end_props;
            first.content.extend(content);
            first.content.extend(tail);
            replacement.push(Block::Paragraph(first));
        } else {
            let head = std::mem::take(&mut p.content);
            let head_attrs = p.head.clone();
            for (k, mut para) in built.into_iter().enumerate() {
                if k == 0 {
                    let mut content = head.clone();
                    content.append(&mut para.content);
                    para.content = content;
                    para.head = head_attrs.clone();
                }
                replacement.push(Block::Paragraph(para));
            }
            let mut last_head = end_head;
            last_head.attrs.clear();
            replacement.push(Block::Paragraph(Paragraph {
                head: last_head,
                props: end_props,
                content: tail,
            }));
        }
        blocks.splice(sep_slot.pos..=sep_slot.pos, replacement);
    }
    doc.edited();
    Ok(())
}
