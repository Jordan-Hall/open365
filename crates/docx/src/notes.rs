//! Headers, footers, footnotes and endnotes (ECMA-376 Part 1 §17.10, §17.11): the parts are
//! found through the main part's relationships and read into the same block model as the body.

use open365_opc::{Relationships, TargetMode, rels_part_name, resolve_target};

use crate::document::Document;
use crate::model::{Block, blocks_from};
use crate::xml::{self, RawXml};

const REL_FOOTNOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
const REL_ENDNOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes";

/// What a `w:footnote` / `w:endnote` is (`w:type`, §17.11.20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Normal,
    Separator,
    ContinuationSeparator,
    ContinuationNotice,
}

/// One footnote or endnote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub kind: NoteKind,
    pub blocks: Vec<Block>,
}

impl Document {
    fn related_part(&self, pick: impl Fn(&str, &str) -> bool) -> Option<Vec<u8>> {
        let main = self.main_part();
        let rels = Relationships::parse(
            self.package()
                .part(rels_part_name(Some(main)).as_str())?
                .data(),
        )
        .ok()?;
        let rel = rels
            .iter()
            .find(|r| r.target_mode == TargetMode::Internal && pick(&r.id, &r.rel_type))?;
        let name = resolve_target(Some(main), &rel.target).ok()?;
        Some(self.package().part(name.as_str())?.data().to_vec())
    }

    /// The blocks of the header or footer part that relationship `rel_id` of the main part
    /// names; `None` when the part is missing or unreadable.
    pub fn header_footer_blocks(&self, rel_id: &str) -> Option<Vec<Block>> {
        let bytes = self.related_part(|id, _| id == rel_id)?;
        let root = xml::parse(&bytes).ok()?.root;
        Some(blocks_from(root.children))
    }

    /// The footnotes part's notes, separators included, in file order.
    pub fn footnotes(&self) -> Vec<Note> {
        self.notes(REL_FOOTNOTES, "footnote")
    }

    /// The endnotes part's notes, separators included, in file order.
    pub fn endnotes(&self) -> Vec<Note> {
        self.notes(REL_ENDNOTES, "endnote")
    }

    fn notes(&self, rel_type: &str, local: &str) -> Vec<Note> {
        let Some(bytes) = self.related_part(|_, t| t == rel_type) else {
            return Vec::new();
        };
        let Ok(parsed) = xml::parse(&bytes) else {
            return Vec::new();
        };
        parsed
            .root
            .children
            .into_iter()
            .filter_map(|c| match c {
                RawXml::Element(e) if e.head.is_w(local) => Some(e),
                _ => None,
            })
            .map(|e| Note {
                id: e
                    .head
                    .attr_w("id")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0),
                kind: match e.head.attr_w("type") {
                    Some("separator") => NoteKind::Separator,
                    Some("continuationSeparator") => NoteKind::ContinuationSeparator,
                    Some("continuationNotice") => NoteKind::ContinuationNotice,
                    _ => NoteKind::Normal,
                },
                blocks: blocks_from(e.children),
            })
            .collect()
    }
}
