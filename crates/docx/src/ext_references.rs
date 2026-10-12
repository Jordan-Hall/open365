//! Editing primitives the References tab needs: inserting field markup and whole paragraphs from
//! WordprocessingML fragments, footnotes and endnotes, and styles the generated tables use.
//!
//! The fragments are built by the caller from escaped text (see [`escape_xml`]); they are parsed
//! here, so malformed markup is an error and never reaches the document.

use std::collections::HashSet;

use open365_opc::{PartName, Relationships, TargetMode, rels_part_name, resolve_target};

use crate::DocxError;
use crate::document::Document;
use crate::edit::split_runs_at;
use crate::fields_scan::{blocks_mut, slots};
use crate::model::{Block, Inline, RunContent, blocks_from};
use crate::ranges::DocPos;
use crate::xml::{self, Attr, ElementHead, RawXml, W_NS, XmlElement};

const REL_FOOTNOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
const REL_ENDNOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes";
const CT_FOOTNOTES: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
const CT_ENDNOTES: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml";
const CONTENT_TYPES_PART: &str = "[Content_Types].xml";

/// Escapes `text` for use in element content or a double-quoted attribute value.
#[must_use]
pub fn escape_xml(text: &str) -> String {
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

/// A footnote or endnote reference mark in the main story.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteRef {
    /// Where the mark sits (paragraph and character offset).
    pub pos: DocPos,
    /// `w:id` of the note.
    pub id: i64,
    pub endnote: bool,
}

fn parse_children(xml_fragment: &str) -> Result<Vec<RawXml>, DocxError> {
    let wrapped = format!("<w:root xmlns:w=\"{W_NS}\">{xml_fragment}</w:root>");
    Ok(xml::parse(wrapped.as_bytes())?.root.children)
}

fn parse_inlines(xml_fragment: &str) -> Result<Vec<Inline>, DocxError> {
    let wrapped = format!("<w:p>{xml_fragment}</w:p>");
    let mut blocks = blocks_from(parse_children(&wrapped)?);
    match blocks.pop() {
        Some(Block::Paragraph(p)) => Ok(p.content),
        _ => Err(DocxError::Xml(
            "fragment is not run-level content".to_owned(),
        )),
    }
}

fn insert_at_boundary(
    content: &mut Vec<Inline>,
    target: usize,
    acc: &mut usize,
    items: &mut Option<Vec<Inline>>,
) {
    let mut i = 0;
    while i < content.len() {
        if items.is_none() {
            return;
        }
        if *acc == target {
            if let Some(items) = items.take() {
                content.splice(i..i, items);
            }
            return;
        }
        match &mut content[i] {
            Inline::Run(r) => *acc += r.char_len(),
            Inline::Group(g) => {
                let len: usize = g.content.iter().map(Inline::char_len).sum();
                if len > 0 && target > *acc && target < *acc + len {
                    insert_at_boundary(&mut g.content, target, acc, items);
                } else {
                    *acc += len;
                }
            }
            Inline::Raw(_) => {}
        }
        i += 1;
    }
    if *acc == target
        && let Some(items) = items.take()
    {
        content.extend(items);
    }
}

fn walk_refs(content: &[Inline], para: usize, acc: &mut usize, out: &mut Vec<NoteRef>) {
    for inline in content {
        match inline {
            Inline::Run(run) => {
                for item in &run.content {
                    if let RunContent::Raw(RawXml::Element(e)) = item {
                        let endnote = e.head.is_w("endnoteReference");
                        if endnote || e.head.is_w("footnoteReference") {
                            let id = e
                                .head
                                .attr_w("id")
                                .and_then(|v| v.trim().parse().ok())
                                .unwrap_or(0);
                            out.push(NoteRef {
                                pos: DocPos { para, offset: *acc },
                                id,
                                endnote,
                            });
                        }
                    }
                }
                *acc += run.char_len();
            }
            Inline::Group(g) => walk_refs(&g.content, para, acc, out),
            Inline::Raw(_) => {}
        }
    }
}

fn attr(name: &str, value: String) -> Attr {
    Attr {
        name: name.to_owned(),
        ns: None,
        value,
    }
}

impl Document {
    /// Inserts the block-level `xml` (one or more `w:p` elements, without namespace
    /// declarations) before paragraph `before`, in the same container (body, table cell or
    /// content control). `before == paragraph_count()` appends after the last body paragraph.
    pub fn insert_blocks_xml(
        &mut self,
        before: usize,
        xml_fragment: &str,
    ) -> Result<(), DocxError> {
        let blocks = blocks_from(parse_children(xml_fragment)?);
        let count = self.paragraph_count();
        if before > count {
            return Err(DocxError::ParagraphOutOfRange {
                index: before,
                count,
            });
        }
        if before < count {
            let slot = slots(self.body()).into_iter().nth(before).ok_or(
                DocxError::ParagraphOutOfRange {
                    index: before,
                    count,
                },
            )?;
            let list = blocks_mut(self.body_mut(), &slot.steps).ok_or_else(|| {
                DocxError::UnsupportedRange("the paragraph is not addressable".to_owned())
            })?;
            list.splice(slot.pos..slot.pos, blocks);
        } else {
            let list = &mut self.body_mut().blocks;
            let at = match list.last() {
                Some(Block::Raw(RawXml::Element(e))) if e.head.is_w("sectPr") => list.len() - 1,
                _ => list.len(),
            };
            list.splice(at..at, blocks);
        }
        self.edited();
        Ok(())
    }

    /// Inserts the block-level `xml` next to the outermost block that holds paragraph `para`:
    /// before it, or after it when `after`. For a paragraph in a table this is beside the
    /// table, not inside the cell (a caption below a table).
    pub fn insert_blocks_around(
        &mut self,
        para: usize,
        after: bool,
        xml_fragment: &str,
    ) -> Result<(), DocxError> {
        let blocks = blocks_from(parse_children(xml_fragment)?);
        let count = self.paragraph_count();
        let slot = slots(self.body())
            .into_iter()
            .nth(para)
            .ok_or(DocxError::ParagraphOutOfRange { index: para, count })?;
        let at = match slot.steps.first() {
            Some(
                crate::fields_scan::Step::Cell { block, .. }
                | crate::fields_scan::Step::Sdt { block, .. },
            ) => *block,
            None => slot.pos,
        };
        let list = &mut self.body_mut().blocks;
        let at = (at + usize::from(after)).min(list.len());
        list.splice(at..at, blocks);
        self.edited();
        Ok(())
    }

    /// Inserts run-level `xml` (runs, field markers; no namespace declarations) at the
    /// character position `pos`, splitting a run when `pos` is inside one.
    pub fn insert_inline_xml(&mut self, pos: DocPos, xml_fragment: &str) -> Result<(), DocxError> {
        let items = parse_inlines(xml_fragment)?;
        self.check_pos(pos)?;
        let para = self.paragraph_mut(pos.para)?;
        split_runs_at(&mut para.content, pos.offset, &mut 0);
        let mut pending = Some(items);
        insert_at_boundary(&mut para.content, pos.offset, &mut 0, &mut pending);
        if let Some(items) = pending {
            para.content.extend(items);
        }
        self.edited();
        Ok(())
    }

    /// The first and last paragraph of field `index` of [`Document::fields`] (begin marker to end
    /// marker); `None` for an unknown field or one that is never closed.
    #[must_use]
    pub fn field_paragraph_span(&self, index: usize) -> Option<(usize, usize)> {
        let raws = crate::fields_scan::Scanner::scan(self.body());
        let raw = raws.get(index)?;
        let end = raw.end.as_ref()?;
        Some((raw.begin.para, end.para))
    }

    /// Whether paragraph `para` is inside a table cell.
    #[must_use]
    pub fn paragraph_in_table(&self, para: usize) -> bool {
        slots(self.body()).get(para).is_some_and(|s| {
            s.steps
                .iter()
                .any(|st| matches!(st, crate::fields_scan::Step::Cell { .. }))
        })
    }

    /// Removes paragraphs `first..=last`. They must share one container and nothing else may
    /// refer to them (the caller removes whole generated blocks, such as an index result).
    pub fn remove_paragraphs(&mut self, first: usize, last: usize) -> Result<(), DocxError> {
        let count = self.paragraph_count();
        if first > last || last >= count {
            return Err(DocxError::ParagraphOutOfRange { index: last, count });
        }
        let all = slots(self.body());
        let (head, tail) = (all[first].clone(), all[last].clone());
        if head.steps != tail.steps {
            return Err(DocxError::UnsupportedRange(
                "the paragraphs are in different containers".to_owned(),
            ));
        }
        let list = blocks_mut(self.body_mut(), &head.steps)
            .ok_or_else(|| DocxError::UnsupportedRange("not addressable".to_owned()))?;
        list.drain(head.pos..=tail.pos);
        self.edited();
        Ok(())
    }

    /// Adds the paragraph or character style `id` from its `w:style` element `style_xml` when
    /// the document has no such style. Returns true when the style exists afterwards.
    pub fn ensure_style_xml(&mut self, id: &str, style_xml: &str) -> bool {
        let Some(styles) = self.styles() else {
            return false;
        };
        if styles.has_style(id) {
            return true;
        }
        let Ok(text) = String::from_utf8(styles.serialize()) else {
            return false;
        };
        let Some(at) = text.rfind("</w:styles>") else {
            return false;
        };
        let mut next = String::with_capacity(text.len() + style_xml.len());
        next.push_str(&text[..at]);
        next.push_str(style_xml);
        next.push_str(&text[at..]);
        self.styles_mut(next.into_bytes())
    }

    /// Every footnote and endnote reference mark of the main story, in document order.
    #[must_use]
    pub fn note_references(&self) -> Vec<NoteRef> {
        let mut out = Vec::new();
        for (para, p) in self.body().paragraphs().into_iter().enumerate() {
            walk_refs(&p.content, para, &mut 0, &mut out);
        }
        out
    }

    /// Inserts a footnote (or endnote when `endnote`) whose reference mark sits at `pos` and
    /// whose body is `text` after the note's own number. Creates the notes part (with the two
    /// separators) when the document has none. Returns the new note's `w:id`.
    pub fn insert_note(
        &mut self,
        pos: DocPos,
        endnote: bool,
        text: &str,
    ) -> Result<i64, DocxError> {
        self.check_pos(pos)?;
        let (rel_type, content_type, local, file) = if endnote {
            (REL_ENDNOTES, CT_ENDNOTES, "endnote", "endnotes.xml")
        } else {
            (REL_FOOTNOTES, CT_FOOTNOTES, "footnote", "footnotes.xml")
        };
        let (text_style, ref_style) = if endnote {
            ("EndnoteText", "EndnoteReference")
        } else {
            ("FootnoteText", "FootnoteReference")
        };
        self.ensure_style_xml(
            text_style,
            &format!(
                "<w:style w:type=\"paragraph\" w:styleId=\"{text_style}\"><w:name w:val=\"{}\"/>\
<w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>\
<w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>\
<w:rPr><w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/></w:rPr></w:style>",
                if endnote {
                    "endnote text"
                } else {
                    "footnote text"
                }
            ),
        );
        self.ensure_style_xml(
            ref_style,
            &format!(
                "<w:style w:type=\"character\" w:styleId=\"{ref_style}\"><w:name w:val=\"{}\"/>\
<w:basedOn w:val=\"DefaultParagraphFont\"/><w:uiPriority w:val=\"99\"/><w:unhideWhenUsed/>\
<w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr></w:style>",
                if endnote {
                    "endnote reference"
                } else {
                    "footnote reference"
                }
            ),
        );
        let (part_name, existing) = self.notes_part(rel_type);
        let dir = self
            .main_part()
            .as_str()
            .rsplit_once('/')
            .map_or(String::new(), |(d, _)| format!("{d}/"));
        let name = match part_name {
            Some(n) => n,
            None => PartName::new(&format!("{dir}{file}"))?,
        };
        let mut parsed = match existing {
            Some(bytes) => xml::parse(&bytes)?,
            None => xml::parse(new_notes_part(local).as_bytes())?,
        };
        let created = self.package().part(name.as_str()).is_none();
        let used: HashSet<i64> = parsed
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .filter_map(|e| e.head.attr_w("id").and_then(|v| v.trim().parse().ok()))
            .collect();
        let id = used.iter().copied().max().map_or(1, |m| (m + 1).max(1));
        let body = format!(
            "<w:{local} w:id=\"{id}\"><w:p><w:pPr><w:pStyle w:val=\"{text_style}\"/></w:pPr>\
<w:r><w:rPr><w:rStyle w:val=\"{ref_style}\"/></w:rPr><w:{local}Ref/></w:r>{}</w:p></w:{local}>",
            if text.is_empty() {
                String::new()
            } else {
                format!(
                    "<w:r><w:t xml:space=\"preserve\"> {}</w:t></w:r>",
                    escape_xml(text)
                )
            }
        );
        for child in parse_children(&body)? {
            parsed.root.children.push(child);
        }
        // The mark in the text first: a bad position must not leave a note nobody refers to.
        self.insert_inline_xml(
            pos,
            &format!(
                "<w:r><w:rPr><w:rStyle w:val=\"{ref_style}\"/></w:rPr><w:{local}Reference w:id=\"{id}\"/></w:r>"
            ),
        )?;
        let main = self.main_part().clone();
        let package = self.package_mut();
        if created {
            let target = name
                .as_str()
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned();
            add_relationship(package, &main, rel_type, &target)?;
            add_override(package, name.as_str(), content_type)?;
        }
        package.set_part(name, parsed.to_bytes());
        self.edited();
        Ok(id)
    }

    /// The part of the notes relationship `rel_type` (name and bytes), when the document has one.
    fn notes_part(&self, rel_type: &str) -> (Option<PartName>, Option<Vec<u8>>) {
        let main = self.main_part();
        let Some(rels) = self
            .package()
            .part(rels_part_name(Some(main)).as_str())
            .and_then(|p| Relationships::parse(p.data()).ok())
        else {
            return (None, None);
        };
        let Some(rel) = rels
            .iter()
            .find(|r| r.target_mode == TargetMode::Internal && r.rel_type == rel_type)
        else {
            return (None, None);
        };
        let Ok(name) = resolve_target(Some(main), &rel.target) else {
            return (None, None);
        };
        let bytes = self
            .package()
            .part(name.as_str())
            .map(|p| p.data().to_vec());
        (Some(name), bytes)
    }
}

fn new_notes_part(local: &str) -> String {
    let sep = |kind: &str, id: i32, mark: &str| {
        format!(
            "<w:{local} w:type=\"{kind}\" w:id=\"{id}\"><w:p><w:pPr><w:spacing w:after=\"0\" \
w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:r><w:{mark}/></w:r></w:p></w:{local}>"
        )
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:{local}s xmlns:w=\"{W_NS}\">{}{}</w:{local}s>",
        sep("separator", -1, "separator"),
        sep("continuationSeparator", 0, "continuationSeparator"),
    )
}

fn add_relationship(
    package: &mut open365_opc::Package,
    main: &PartName,
    rel_type: &str,
    target: &str,
) -> Result<(), DocxError> {
    let name = rels_part_name(Some(main));
    let mut parsed = match package.part(name.as_str()) {
        Some(p) => xml::parse(p.data())?,
        None => xml::parse(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
        )?,
    };
    let used: HashSet<String> = parsed
        .root
        .children
        .iter()
        .filter_map(RawXml::as_element)
        .flat_map(|e| e.head.attrs.iter())
        .filter(|a| a.name == "Id")
        .map(|a| a.value.clone())
        .collect();
    let mut n = 1;
    while used.contains(&format!("rId{n}")) {
        n += 1;
    }
    let head = ElementHead {
        name: "Relationship".to_owned(),
        ns: parsed.root.head.ns.clone(),
        attrs: vec![
            attr("Id", format!("rId{n}")),
            attr("Type", rel_type.to_owned()),
            attr("Target", target.to_owned()),
        ],
    };
    parsed.root.children.push(RawXml::Element(XmlElement {
        head,
        children: Vec::new(),
    }));
    package.set_part(name, parsed.to_bytes());
    Ok(())
}

fn add_override(
    package: &mut open365_opc::Package,
    part: &str,
    content_type: &str,
) -> Result<(), DocxError> {
    let Some(types) = package.part(CONTENT_TYPES_PART) else {
        return Err(DocxError::Xml(
            "package has no [Content_Types].xml".to_owned(),
        ));
    };
    let mut parsed = xml::parse(types.data())?;
    let head = ElementHead {
        name: "Override".to_owned(),
        ns: parsed.root.head.ns.clone(),
        attrs: vec![
            attr("PartName", format!("/{part}")),
            attr("ContentType", content_type.to_owned()),
        ],
    };
    parsed.root.children.push(RawXml::Element(XmlElement {
        head,
        children: Vec::new(),
    }));
    package.set_part(PartName::new(CONTENT_TYPES_PART)?, parsed.to_bytes());
    Ok(())
}
