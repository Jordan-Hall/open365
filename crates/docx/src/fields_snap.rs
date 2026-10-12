//! A read-only view of the main story that field evaluation needs: paragraph texts and
//! styles, bookmarks with their text, styles by name, and numbering labels.

#![allow(dead_code)] // groundwork for the REF-family and TOC evaluators, which are not wired yet

use std::collections::HashMap;

use crate::document::Document;
use crate::fields_number;
use crate::model::{Block, Body, Inline, Paragraph, RowChild, RunContent, SdtChild, TableChild};
use crate::numbering::Numbering;
use crate::ranges::DocPos;
use crate::xml::{self, RawXml};

/// A style as the fields need it.
#[derive(Debug, Clone, Default)]
pub(crate) struct StyleRow {
    pub id: String,
    pub name: String,
    pub based_on: Option<String>,
    pub outline: Option<u8>,
    pub lang: Option<String>,
    pub paragraph: bool,
}

/// Styles by id and name, plus the document's default language.
#[derive(Debug, Clone, Default)]
pub(crate) struct StyleTable {
    pub rows: Vec<StyleRow>,
    pub default_lang: Option<String>,
    pub default_paragraph: Option<String>,
}

impl StyleTable {
    pub(crate) fn load(doc: &Document) -> Self {
        let mut table = Self::default();
        let Some(styles) = doc.styles() else {
            return table;
        };
        let Some(part) = doc.package().part(styles.part_name().as_str()) else {
            return table;
        };
        let Ok(parsed) = xml::parse(part.data()) else {
            return table;
        };
        for child in parsed.root.children.iter().filter_map(RawXml::as_element) {
            if child.head.is_w("docDefaults") {
                table.default_lang = lang_in(child);
            } else if child.head.is_w("style") {
                let Some(id) = child.head.attr_w("styleId") else {
                    continue;
                };
                let val = |local: &str| {
                    child
                        .children
                        .iter()
                        .filter_map(RawXml::as_element)
                        .find(|e| e.head.is_w(local))
                        .and_then(|e| e.head.attr_w("val").map(str::to_owned))
                };
                let outline = child
                    .children
                    .iter()
                    .filter_map(RawXml::as_element)
                    .find(|e| e.head.is_w("pPr"))
                    .and_then(outline_of_ppr);
                let kind = child.head.attr_w("type").unwrap_or("paragraph");
                let is_default = matches!(child.head.attr_w("default"), Some("1" | "true" | "on"));
                if kind == "paragraph" && is_default {
                    table.default_paragraph = Some(id.to_owned());
                }
                table.rows.push(StyleRow {
                    id: id.to_owned(),
                    name: val("name").unwrap_or_else(|| id.to_owned()),
                    based_on: val("basedOn"),
                    outline,
                    lang: lang_in(child),
                    paragraph: kind == "paragraph",
                });
            }
        }
        table
    }

    pub(crate) fn by_id(&self, id: &str) -> Option<&StyleRow> {
        self.rows.iter().find(|r| r.id == id)
    }

    /// A style by display name (`heading 1`, `Heading 1`) or id, case-insensitive.
    pub(crate) fn by_name(&self, name: &str) -> Option<&StyleRow> {
        let wanted = name.trim();
        self.rows
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(wanted))
            .or_else(|| self.rows.iter().find(|r| r.id.eq_ignore_ascii_case(wanted)))
    }

    /// Display name of the style `id`.
    pub(crate) fn name_of(&self, id: &str) -> String {
        self.by_id(id)
            .map_or_else(|| id.to_owned(), |r| r.name.clone())
    }

    /// First language found walking the `basedOn` chain from `id`.
    pub(crate) fn lang_of(&self, id: &str) -> Option<String> {
        let mut cur = self.by_id(id);
        let mut guard = 0;
        while let Some(row) = cur {
            if let Some(lang) = &row.lang {
                return Some(lang.clone());
            }
            cur = row.based_on.as_deref().and_then(|b| self.by_id(b));
            guard += 1;
            if guard > 32 {
                break;
            }
        }
        None
    }

    /// Outline level (0-based) the style assigns, walking `basedOn`.
    pub(crate) fn outline_of(&self, id: &str) -> Option<u8> {
        let mut cur = self.by_id(id);
        let mut guard = 0;
        while let Some(row) = cur {
            if let Some(level) = row.outline {
                return Some(level);
            }
            cur = row.based_on.as_deref().and_then(|b| self.by_id(b));
            guard += 1;
            if guard > 32 {
                break;
            }
        }
        None
    }

    /// True when `id` is `ancestor` or based on it.
    pub(crate) fn is_based_on(&self, id: &str, ancestor: &str) -> bool {
        let mut cur = self.by_id(id);
        let mut guard = 0;
        while let Some(row) = cur {
            if row.id == ancestor {
                return true;
            }
            cur = row.based_on.as_deref().and_then(|b| self.by_id(b));
            guard += 1;
            if guard > 32 {
                break;
            }
        }
        false
    }
}

fn lang_in(e: &crate::xml::XmlElement) -> Option<String> {
    fn find(e: &crate::xml::XmlElement) -> Option<String> {
        if e.head.is_w("lang") {
            return e.head.attr_w("val").map(str::to_owned);
        }
        e.children
            .iter()
            .filter_map(RawXml::as_element)
            .find_map(find)
    }
    find(e)
}

fn outline_of_ppr(ppr: &crate::xml::XmlElement) -> Option<u8> {
    ppr.children
        .iter()
        .filter_map(RawXml::as_element)
        .find(|e| e.head.is_w("outlineLvl"))
        .and_then(|e| e.head.attr_w("val"))
        .and_then(|v| v.parse().ok())
}

/// What the fields need to know about one paragraph.
#[derive(Debug, Clone, Default)]
pub(crate) struct ParaInfo {
    /// Text with `\t` for tabs and `\u{b}` for line breaks.
    pub text: String,
    pub style_id: Option<String>,
    /// `w:outlineLvl` set directly on the paragraph (0-based; 9 is body text).
    pub outline: Option<u8>,
    /// Numbering label, e.g. `2.1`, when the paragraph is numbered.
    pub label: Option<String>,
    /// Numbering detail for the n, r and w switches.
    pub num: Option<NumInfo>,
    pub in_table: bool,
}

/// Where a paragraph sits in its list.
#[derive(Debug, Clone, Default)]
pub(crate) struct NumInfo {
    pub list: i64,
    pub ilvl: usize,
    /// The label each level up to `ilvl` shows at this point (`1.`, `(a)`, `1.1`).
    pub levels: Vec<String>,
    /// Level texts that name an earlier level (`%1` in level 2): those labels are complete.
    pub self_contained: bool,
}

/// A footnote or endnote reference mark.
#[derive(Debug, Clone)]
pub(crate) struct NoteMark {
    pub pos: DocPos,
    pub endnote: bool,
    /// 1-based number among the notes of its kind.
    pub ordinal: usize,
}

/// One bookmark.
#[derive(Debug, Clone)]
pub(crate) struct Bookmark {
    pub id: String,
    pub name: String,
    pub start: DocPos,
    pub end: DocPos,
    /// The range ends after the paragraph mark of `end.para`.
    pub covers_mark: bool,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Snapshot {
    pub paras: Vec<ParaInfo>,
    pub bookmarks: Vec<Bookmark>,
    pub notes: Vec<NoteMark>,
}

impl Snapshot {
    pub(crate) fn build(doc: &Document, styles: &StyleTable) -> Self {
        let body = doc.body();
        let mut snap = Self::default();
        let mut walker = Walker {
            snap: &mut snap,
            open: HashMap::new(),
            pending_starts: Vec::new(),
            para: 0,
            last_len: 0,
            numbering: doc.numbering(),
            counters: HashMap::new(),
            foot_seen: 0,
            end_seen: 0,
            styles,
            doc,
        };
        walker.blocks(&body.blocks, false);
        walker.finish();
        snap
    }

    /// Heading level (0-based, below 9) of paragraph `para`: its own outline level, else the
    /// one its style gives, else the number in a `heading N` style name.
    pub(crate) fn outline_level(&self, styles: &StyleTable, para: usize) -> Option<u8> {
        let info = self.paras.get(para)?;
        if let Some(level) = info.outline {
            return (level < 9).then_some(level);
        }
        let id = info
            .style_id
            .as_deref()
            .or(styles.default_paragraph.as_deref())?;
        if let Some(level) = styles.outline_of(id) {
            return (level < 9).then_some(level);
        }
        let name = styles.name_of(id).to_ascii_lowercase();
        let n: u8 = name.strip_prefix("heading ")?.parse().ok()?;
        (1..=9).contains(&n).then(|| n - 1)
    }

    pub(crate) fn bookmark(&self, name: &str) -> Option<&Bookmark> {
        self.bookmarks
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(name))
    }

    /// Text of the bookmark with Word's rules (paragraph marks as `\r`).
    pub(crate) fn text(&self, start: DocPos, end: DocPos, covers_mark: bool) -> String {
        let mut out = String::new();
        for p in start.para..=end.para {
            let Some(info) = self.paras.get(p) else { break };
            let chars: Vec<char> = info.text.chars().collect();
            let from = if p == start.para {
                start.offset.min(chars.len())
            } else {
                0
            };
            let to = if p == end.para {
                end.offset.min(chars.len())
            } else {
                chars.len()
            };
            if from < to {
                out.extend(&chars[from..to]);
            }
            if p < end.para || covers_mark {
                out.push('\r');
            }
        }
        out
    }
}

struct Walker<'a> {
    snap: &'a mut Snapshot,
    /// Open bookmarks by id: (name, start).
    open: HashMap<String, (String, DocPos)>,
    /// Block-level starts waiting for the next paragraph.
    pending_starts: Vec<(String, String)>,
    para: usize,
    last_len: usize,
    numbering: Option<&'a Numbering>,
    counters: HashMap<i64, Vec<i64>>,
    foot_seen: usize,
    end_seen: usize,
    styles: &'a StyleTable,
    doc: &'a Document,
}

impl Walker<'_> {
    fn blocks(&mut self, blocks: &[Block], in_table: bool) {
        for block in blocks {
            match block {
                Block::Paragraph(p) => self.paragraph(p, in_table),
                Block::Table(t) => {
                    for row in &t.children {
                        let TableChild::Row(row) = row else { continue };
                        for cell in &row.children {
                            let RowChild::Cell(cell) = cell else { continue };
                            self.blocks(&cell.blocks, true);
                        }
                    }
                }
                Block::Sdt(sdt) => {
                    for child in &sdt.children {
                        if let SdtChild::Content(c) = child {
                            self.blocks(&c.blocks, in_table);
                        }
                    }
                }
                Block::Raw(RawXml::Element(e)) if e.head.is_w("bookmarkStart") => {
                    if let (Some(id), Some(name)) = (e.head.attr_w("id"), e.head.attr_w("name")) {
                        self.pending_starts.push((id.to_owned(), name.to_owned()));
                    }
                }
                Block::Raw(RawXml::Element(e)) if e.head.is_w("bookmarkEnd") => {
                    if let Some(id) = e.head.attr_w("id") {
                        // After the previous paragraph's mark.
                        let para = self.para.saturating_sub(1);
                        self.close(
                            id,
                            DocPos {
                                para,
                                offset: self.last_len,
                            },
                            true,
                        );
                    }
                }
                Block::Raw(_) => {}
            }
        }
    }

    fn close(&mut self, id: &str, end: DocPos, covers_mark: bool) {
        if let Some((name, start)) = self.open.remove(id) {
            self.snap.bookmarks.push(Bookmark {
                id: id.to_owned(),
                name,
                start,
                end,
                covers_mark,
                text: String::new(),
            });
        }
    }

    fn paragraph(&mut self, p: &Paragraph, in_table: bool) {
        let index = self.para;
        for (id, name) in std::mem::take(&mut self.pending_starts) {
            self.open.insert(
                id,
                (
                    name,
                    DocPos {
                        para: index,
                        offset: 0,
                    },
                ),
            );
        }
        let mut offset = 0usize;
        self.inlines(&p.content, index, &mut offset);
        let text = p.text().replace('\n', "\u{b}");
        self.last_len = p.char_len();
        let style_id = p.style_id().map(str::to_owned);
        let outline = p.props.as_ref().and_then(outline_of_ppr);
        let (label, num) = self.number_label(p);
        self.snap.paras.push(ParaInfo {
            text,
            style_id,
            outline,
            label,
            num,
            in_table,
        });
        self.para += 1;
    }

    fn inlines(&mut self, content: &[Inline], para: usize, offset: &mut usize) {
        for inline in content {
            match inline {
                Inline::Run(r) => {
                    for item in &r.content {
                        if let RunContent::Raw(RawXml::Element(e)) = item {
                            let foot = e.head.is_w("footnoteReference");
                            if foot || e.head.is_w("endnoteReference") {
                                let seen = if foot {
                                    &mut self.foot_seen
                                } else {
                                    &mut self.end_seen
                                };
                                *seen += 1;
                                let ordinal = *seen;
                                self.snap.notes.push(NoteMark {
                                    pos: DocPos {
                                        para,
                                        offset: *offset,
                                    },
                                    endnote: !foot,
                                    ordinal,
                                });
                            }
                        }
                        *offset += item.char_len();
                    }
                }
                Inline::Group(g) => self.inlines(&g.content, para, offset),
                Inline::Raw(RawXml::Element(e)) if e.head.is_w("bookmarkStart") => {
                    if let (Some(id), Some(name)) = (e.head.attr_w("id"), e.head.attr_w("name")) {
                        self.open.insert(
                            id.to_owned(),
                            (
                                name.to_owned(),
                                DocPos {
                                    para,
                                    offset: *offset,
                                },
                            ),
                        );
                    }
                }
                Inline::Raw(RawXml::Element(e)) if e.head.is_w("bookmarkEnd") => {
                    if let Some(id) = e.head.attr_w("id") {
                        self.close(
                            id,
                            DocPos {
                                para,
                                offset: *offset,
                            },
                            false,
                        );
                    }
                }
                Inline::Raw(_) => {}
            }
        }
    }

    fn finish(&mut self) {
        // Fill texts now that every paragraph is known.
        let bookmarks = std::mem::take(&mut self.snap.bookmarks);
        let snap: &Snapshot = self.snap;
        let filled: Vec<Bookmark> = bookmarks
            .into_iter()
            .map(|mut b| {
                b.text = snap.text(b.start, b.end, b.covers_mark);
                b
            })
            .collect();
        self.snap.bookmarks = filled;
        // Unclosed bookmarks end at the document end.
        let open: Vec<(String, (String, DocPos))> = self.open.drain().collect();
        for (id, (name, start)) in open {
            let last = self.snap.paras.len().saturating_sub(1);
            let end = DocPos {
                para: last,
                offset: self
                    .snap
                    .paras
                    .get(last)
                    .map_or(0, |p| p.text.chars().count()),
            };
            let text = self.snap.text(start, end, false);
            self.snap.bookmarks.push(Bookmark {
                id,
                name,
                start,
                end,
                covers_mark: false,
                text,
            });
        }
    }

    /// The numbering label of `p` and the label of each level up to its own.
    fn number_label(&mut self, p: &Paragraph) -> (Option<String>, Option<NumInfo>) {
        let Some(numbering) = self.numbering else {
            return (None, None);
        };
        let Some(pn) = numbering.paragraph_numbering(self.doc.styles(), p) else {
            return (None, None);
        };
        let Some(resolved) = numbering.resolve(self.doc.styles(), pn.num_id) else {
            return (None, None);
        };
        let Some(level) = resolved.level(pn.ilvl) else {
            return (None, None);
        };
        if level.is_bullet() {
            return (None, None);
        }
        let counters = self.counters.entry(resolved.abstract_id).or_default();
        counters.resize(crate::numbering::LEVELS.max(counters.len()), 0);
        // Restart: a new item at level L resets the deeper levels.
        for count in counters.iter_mut().skip(pn.ilvl + 1) {
            *count = 0;
        }
        let start = level.start;
        counters[pn.ilvl] = if counters[pn.ilvl] == 0 {
            start
        } else {
            counters[pn.ilvl] + 1
        };
        let counters = counters.clone();
        let render = |lvl_idx: usize| -> String {
            let Some(lv) = resolved.level(lvl_idx) else {
                return String::new();
            };
            let mut label = String::new();
            let mut chars = lv.text.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '%'
                    && let Some(d) = chars.peek().and_then(|d| d.to_digit(10))
                {
                    chars.next();
                    let lvl = usize::try_from(d).unwrap_or(1).saturating_sub(1);
                    let value = counters
                        .get(lvl)
                        .copied()
                        .unwrap_or(0)
                        .max(resolved.level(lvl).map_or(0, |l| l.start));
                    let fmt = resolved.level(lvl).map_or("decimal", |l| {
                        if lv.is_legal {
                            "decimal"
                        } else {
                            l.num_fmt.as_str()
                        }
                    });
                    label.push_str(&format_number(value, fmt));
                } else {
                    label.push(c);
                }
            }
            label
        };
        let label = render(pn.ilvl);
        let levels: Vec<String> = (0..=pn.ilvl).map(render).collect();
        let self_contained =
            pn.ilvl == 0 || (0..pn.ilvl).all(|l| level.text.contains(&format!("%{}", l + 1)));
        let info = NumInfo {
            list: resolved.abstract_id,
            ilvl: pn.ilvl,
            levels,
            self_contained,
        };
        (Some(label), Some(info))
    }
}

/// A number in a `w:numFmt` format.
pub(crate) fn format_number(value: i64, fmt: &str) -> String {
    #[allow(clippy::cast_precision_loss)]
    let v = value as f64;
    match fmt {
        "upperRoman" => fields_number_roman(v, false),
        "lowerRoman" => fields_number_roman(v, true),
        "upperLetter" => letters(value, false),
        "lowerLetter" => letters(value, true),
        "decimalZero" => format!("{value:02}"),
        "ordinal" => fields_number::ordinal_number(v).unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

fn fields_number_roman(v: f64, lower: bool) -> String {
    fields_number::roman(v, lower).unwrap_or_else(|| (v as i64).to_string())
}

fn letters(value: i64, lower: bool) -> String {
    if value <= 0 {
        return String::new();
    }
    let n = usize::try_from((value - 1) / 26 + 1).unwrap_or(1);
    let idx = u8::try_from((value - 1) % 26).unwrap_or(0);
    let letter = char::from(if lower { b'a' } else { b'A' } + idx);
    std::iter::repeat_n(letter, n).collect()
}

/// Unused helper kept for symmetry with [`Body`] walkers.
#[allow(dead_code)]
fn paragraph_count(body: &Body) -> usize {
    body.paragraphs().len()
}

impl Document {
    /// The outline level of paragraph `para`, 0-based (0 is Level 1): its own `w:outlineLvl`, else
    /// the one its style gives. `None` is Body Text, or a paragraph that does not exist.
    #[must_use]
    pub fn paragraph_outline_level(&self, para: usize) -> Option<u8> {
        let styles = StyleTable::load(self);
        Snapshot::build(self, &styles).outline_level(&styles, para)
    }
}
