//! Recording edits as tracked changes (see [`Document::set_track_changes`]).
//!
//! Offsets are final-view characters, like every other editing call. The rules follow Word:
//! typing next to or inside the author's own insertion extends it; typing inside another
//! author's insertion splits it; deleting text the author inserted removes it outright;
//! other text is wrapped in `w:del` (nested inside a foreign `w:ins` when needed) and its
//! `w:t` becomes `w:delText`; a paragraph mark the author inserted is removed outright when
//! deleted, any other mark gets a `w:del`.

use crate::DocxError;
use crate::document::Document;
use crate::edit::{pieces_of, split_runs_at};
use crate::model::{Block, Group, Inline, Paragraph, Run};
use crate::ranges::{DocPos, container, container_mut, locate_paragraph};
use crate::review::{
    TrackChanges, child_el, join_paragraphs, mark_text_deleted, new_revision_head,
};
use crate::xml::{ElementHead, RawXml, XmlElement};

/// Hands out revision ids.
pub(crate) struct Ids(pub u64);

impl Ids {
    fn next(&mut self) -> u64 {
        let id = self.0;
        self.0 += 1;
        id
    }
}

fn is_own_ins(g: &Group, track: &TrackChanges) -> bool {
    g.head.is_w("ins") && g.head.attr_w("author") == Some(track.author.as_str())
}

fn is_hidden(g: &Group) -> bool {
    g.head.is_w("del") || g.head.is_w("moveFrom")
}

fn visible_len(g: &Group) -> usize {
    g.content.iter().map(Inline::char_len).sum()
}

/// How the insertion point relates to the group being searched.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Top level or inside a group that is neither an insertion nor a move target.
    Plain,
    /// Inside the author's own insertion: the run goes in bare.
    Own,
    /// Inside another author's insertion or a move target: it must be split.
    Foreign,
}

enum Placed {
    Done,
    /// The insertion point is at this index of a foreign group's content.
    Split(usize),
}

struct Insertion<'a> {
    run: Option<Run>,
    head: &'a ElementHead,
    track: &'a TrackChanges,
    ids: &'a mut Ids,
}

impl Insertion<'_> {
    fn take(&mut self) -> Inline {
        Inline::Run(
            self.run
                .take()
                .unwrap_or_else(|| unreachable_run(self.head)),
        )
    }

    fn new_ins(&mut self, inner: Vec<Inline>) -> Inline {
        Inline::Group(Group {
            head: new_revision_head(self.head, "ins", self.ids.next(), self.track),
            content: inner,
        })
    }
}

/// Only reached if a run were placed twice, which `place` rules out.
fn unreachable_run(head: &ElementHead) -> Run {
    Run {
        head: head.sibling("r"),
        props: crate::props::RunProps::default(),
        content: Vec::new(),
    }
}

fn place(
    content: &mut Vec<Inline>,
    target: usize,
    acc: &mut usize,
    mode: Mode,
    ins: &mut Insertion<'_>,
) -> Option<Placed> {
    let mut i = 0;
    while i < content.len() {
        let hidden = matches!(&content[i], Inline::Group(g) if is_hidden(g));
        if *acc == target && !hidden {
            return Some(insert_here(content, i, mode, ins));
        }
        let mut split_right: Option<Group> = None;
        match &mut content[i] {
            Inline::Run(r) => *acc += r.char_len(),
            Inline::Group(g) => {
                let len = visible_len(g);
                if len > 0 && target > *acc && target < *acc + len {
                    let inner = if is_own_ins(g, ins.track) {
                        Mode::Own
                    } else if g.head.is_w("ins") || g.head.is_w("moveTo") {
                        Mode::Foreign
                    } else {
                        Mode::Plain
                    };
                    match place(&mut g.content, target, acc, inner, ins) {
                        Some(Placed::Done) => return Some(Placed::Done),
                        Some(Placed::Split(j)) => {
                            let tail = g.content.split_off(j);
                            let mut right = Group {
                                head: g.head.clone(),
                                content: tail,
                            };
                            right.head.set_attr_w("id", &ins.ids.next().to_string());
                            split_right = Some(right);
                        }
                        None => {}
                    }
                } else {
                    *acc += len;
                }
            }
            Inline::Raw(_) => {}
        }
        if let Some(right) = split_right {
            let run = ins.take();
            let middle = ins.new_ins(vec![run]);
            content.splice(i + 1..i + 1, [middle, Inline::Group(right)]);
            return Some(Placed::Done);
        }
        i += 1;
    }
    (*acc == target).then(|| insert_here(content, content.len(), mode, ins))
}

fn insert_here(
    content: &mut Vec<Inline>,
    at: usize,
    mode: Mode,
    ins: &mut Insertion<'_>,
) -> Placed {
    match mode {
        Mode::Own => {
            let run = ins.take();
            content.insert(at, run);
            Placed::Done
        }
        Mode::Foreign => Placed::Split(at),
        Mode::Plain => {
            let before_own =
                at > 0 && matches!(&content[at - 1], Inline::Group(g) if is_own_ins(g, ins.track));
            if before_own {
                let run = ins.take();
                if let Inline::Group(g) = &mut content[at - 1] {
                    g.content.push(run);
                }
                return Placed::Done;
            }
            let at_own =
                matches!(content.get(at), Some(Inline::Group(g)) if is_own_ins(g, ins.track));
            if at_own {
                let run = ins.take();
                if let Inline::Group(g) = &mut content[at] {
                    g.content.insert(0, run);
                }
                return Placed::Done;
            }
            let run = ins.take();
            let group = ins.new_ins(vec![run]);
            content.insert(at, group);
            Placed::Done
        }
    }
}

fn check_range(len: usize, start: usize, end: usize) -> Result<(), DocxError> {
    if start > end || end > len {
        Err(DocxError::RangeOutOfBounds { start, end, len })
    } else {
        Ok(())
    }
}

impl Paragraph {
    /// Inserts `text` at final-view `offset` as a tracked insertion.
    fn tracked_insert(
        &mut self,
        offset: usize,
        text: &str,
        track: &TrackChanges,
        ids: &mut Ids,
    ) -> Result<(), DocxError> {
        check_range(self.char_len(), offset, offset)?;
        let pieces = pieces_of(text)?;
        if pieces.is_empty() {
            return Ok(());
        }
        let props = self.run_props_at(offset);
        let mut run = self.new_run();
        run.props = props;
        run.content = run.items_for(pieces);
        split_runs_at(&mut self.content, offset, &mut 0);
        let head = self.head.clone();
        let mut ins = Insertion {
            run: Some(run),
            head: &head,
            track,
            ids,
        };
        let mut acc = 0;
        place(&mut self.content, offset, &mut acc, Mode::Plain, &mut ins);
        Ok(())
    }

    /// Marks final-view characters `lo..hi` as deleted.
    fn tracked_delete(
        &mut self,
        lo: usize,
        hi: usize,
        track: &TrackChanges,
        ids: &mut Ids,
    ) -> Result<(), DocxError> {
        check_range(self.char_len(), lo, hi)?;
        if lo == hi {
            return Ok(());
        }
        for boundary in [lo, hi] {
            split_runs_at(&mut self.content, boundary, &mut 0);
        }
        let head = self.head.clone();
        let mut ctx = Deletion {
            lo,
            hi,
            head: &head,
            track,
            ids,
        };
        let mut acc = 0;
        delete_in(&mut self.content, &mut acc, false, &mut ctx);
        Ok(())
    }

    /// Adds `w:ins` to the paragraph mark.
    fn mark_inserted(&mut self, track: &TrackChanges, id: u64) {
        let head = new_revision_head(&self.head, "ins", id, track);
        let rpr = rpr_mut(self);
        rpr.children.retain(|c| {
            !matches!(c, RawXml::Element(e)
                if ["ins", "del", "moveFrom", "moveTo"].iter().any(|n| e.head.is_w(n)))
        });
        rpr.children.insert(
            0,
            RawXml::Element(XmlElement {
                head,
                children: Vec::new(),
            }),
        );
    }

    /// Adds `w:del` to the paragraph mark unless it already has one.
    fn mark_deleted(&mut self, track: &TrackChanges, id: u64) {
        if self
            .props
            .as_ref()
            .is_some_and(|p| child_el(p, "rPr").is_some_and(|r| child_el(r, "del").is_some()))
        {
            return;
        }
        let head = new_revision_head(&self.head, "del", id, track);
        let rpr = rpr_mut(self);
        let at = rpr
            .children
            .iter()
            .position(|c| !matches!(c, RawXml::Element(e) if e.head.is_w("ins")))
            .unwrap_or(rpr.children.len());
        rpr.children.insert(
            at,
            RawXml::Element(XmlElement {
                head,
                children: Vec::new(),
            }),
        );
    }

    fn own_inserted_mark(&self, track: &TrackChanges) -> bool {
        self.props
            .as_ref()
            .and_then(|p| child_el(p, "rPr"))
            .and_then(|r| child_el(r, "ins"))
            .is_some_and(|i| i.head.attr_w("author") == Some(track.author.as_str()))
    }
}

/// The paragraph mark's `w:rPr`, created when missing.
fn rpr_mut(p: &mut Paragraph) -> &mut XmlElement {
    crate::ppr::upsert_child(p.props_mut(), "rPr", |_| {});
    let props = p.props_mut();
    let pos = props
        .children
        .iter()
        .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w("rPr")))
        .unwrap_or(0);
    match &mut props.children[pos] {
        RawXml::Element(e) => e,
        _ => unreachable!("position found an element"),
    }
}

struct Deletion<'a> {
    lo: usize,
    hi: usize,
    head: &'a ElementHead,
    track: &'a TrackChanges,
    ids: &'a mut Ids,
}

impl Deletion<'_> {
    fn flush(&mut self, out: &mut Vec<Inline>, pending: &mut Vec<Inline>) {
        if pending.is_empty() {
            return;
        }
        out.push(Inline::Group(Group {
            head: new_revision_head(self.head, "del", self.ids.next(), self.track),
            content: std::mem::take(pending),
        }));
    }
}

fn delete_in(content: &mut Vec<Inline>, acc: &mut usize, own_ins: bool, ctx: &mut Deletion<'_>) {
    let old = std::mem::take(content);
    let mut pending: Vec<Inline> = Vec::new();
    for inline in old {
        match inline {
            Inline::Run(mut r) => {
                let len = r.char_len();
                let start = *acc;
                *acc += len;
                if len > 0 && start >= ctx.lo && start + len <= ctx.hi {
                    if !own_ins {
                        mark_text_deleted(&mut r);
                        pending.push(Inline::Run(r));
                    }
                } else {
                    ctx.flush(content, &mut pending);
                    content.push(Inline::Run(r));
                }
            }
            Inline::Group(mut g) => {
                ctx.flush(content, &mut pending);
                let len = visible_len(&g);
                if len == 0 || *acc >= ctx.hi || *acc + len <= ctx.lo {
                    *acc += len;
                    content.push(Inline::Group(g));
                    continue;
                }
                let inner_own = own_ins || is_own_ins(&g, ctx.track);
                let had = !g.content.is_empty();
                delete_in(&mut g.content, acc, inner_own, ctx);
                if !(had && g.content.is_empty()) {
                    content.push(Inline::Group(g));
                }
            }
            raw @ Inline::Raw(_) => {
                ctx.flush(content, &mut pending);
                content.push(raw);
            }
        }
    }
    ctx.flush(content, &mut pending);
}

impl Document {
    /// The next unused `w:id` for a revision element.
    pub(crate) fn alloc_annotation_id(&mut self) -> u64 {
        let next = match self.next_annotation_id {
            Some(n) => n,
            None => max_annotation_id(&self.serialize_main()) + 1,
        };
        self.next_annotation_id = Some(next + 1);
        next
    }

    pub(crate) fn tracked_insert_text(
        &mut self,
        para: usize,
        offset: usize,
        text: &str,
        track: &TrackChanges,
    ) -> Result<(), DocxError> {
        let mut ids = Ids(self.alloc_annotation_id());
        let before = ids.0;
        self.paragraph_mut(para)?
            .tracked_insert(offset, text, track, &mut ids)?;
        self.next_annotation_id = Some(ids.0.max(before));
        self.edited();
        Ok(())
    }

    pub(crate) fn tracked_delete_range(
        &mut self,
        s: DocPos,
        e: DocPos,
        track: &TrackChanges,
    ) -> Result<(), DocxError> {
        let (path, bi_s) = locate_paragraph(self.body(), s.para)
            .ok_or_else(|| DocxError::UnsupportedRange("paragraph not found".to_owned()))?;
        let (path_e, bi_e) = locate_paragraph(self.body(), e.para)
            .ok_or_else(|| DocxError::UnsupportedRange("paragraph not found".to_owned()))?;
        if path != path_e {
            return Err(DocxError::UnsupportedRange(
                "the range crosses a table or content control boundary".to_owned(),
            ));
        }
        let blocks = container(self.body(), &path)
            .ok_or_else(|| DocxError::UnsupportedRange("container not found".to_owned()))?;
        let mut paragraph_blocks = 0;
        for block in &blocks[bi_s..=bi_e] {
            match block {
                Block::Paragraph(_) => paragraph_blocks += 1,
                Block::Raw(_) => {}
                Block::Table(_) | Block::Sdt(_) => {
                    return Err(DocxError::UnsupportedRange(
                        "the range covers a table or content control".to_owned(),
                    ));
                }
            }
        }
        if paragraph_blocks != e.para - s.para + 1 {
            return Err(DocxError::UnsupportedRange(
                "paragraph numbering does not match the container".to_owned(),
            ));
        }

        let mut ids = Ids(self.alloc_annotation_id());
        for q in s.para..=e.para {
            let lo = if q == s.para { s.offset } else { 0 };
            let hi = if q == e.para {
                e.offset
            } else {
                self.paragraph(q).map_or(0, Paragraph::char_len)
            };
            self.paragraph_mut(q)?
                .tracked_delete(lo, hi, track, &mut ids)?;
        }

        // Paragraph marks between the two ends: the author's own inserted marks go, the
        // others are marked deleted.
        let mut joins = Vec::new();
        for q in s.para..e.para {
            let (_, bi) = locate_paragraph(self.body(), q)
                .ok_or_else(|| DocxError::UnsupportedRange("paragraph not found".to_owned()))?;
            let next_is_paragraph = container(self.body(), &path)
                .is_some_and(|b| matches!(b.get(bi + 1), Some(Block::Paragraph(_))));
            let own = self
                .paragraph(q)
                .is_some_and(|p| p.own_inserted_mark(track));
            if own && next_is_paragraph {
                joins.push(bi);
            } else {
                let id = ids.next();
                self.paragraph_mut(q)?.mark_deleted(track, id);
            }
        }
        if !joins.is_empty() {
            let blocks = container_mut(self.body_mut(), &path)
                .ok_or_else(|| DocxError::UnsupportedRange("container not found".to_owned()))?;
            for bi in joins.into_iter().rev() {
                join_paragraphs(blocks, bi);
            }
        }
        self.next_annotation_id = Some(ids.0);
        self.edited();
        Ok(())
    }

    /// Marks the left half of a split paragraph as an inserted paragraph mark.
    pub(crate) fn mark_split_inserted(
        &mut self,
        para: usize,
        track: &TrackChanges,
    ) -> Result<(), DocxError> {
        let id = self.alloc_annotation_id();
        self.paragraph_mut(para)?.mark_inserted(track, id);
        Ok(())
    }
}

/// Largest number found in a `:id="N"` attribute of `xml`, 0 when there is none.
fn max_annotation_id(xml: &[u8]) -> u64 {
    let mut max = 0;
    let needle = b":id=\"";
    let mut i = 0;
    while i + needle.len() < xml.len() {
        if xml[i..].starts_with(needle) {
            let mut j = i + needle.len();
            let mut value: u64 = 0;
            let mut digits = 0;
            while let Some(d) = xml.get(j).filter(|b| b.is_ascii_digit()) {
                value = value.saturating_mul(10).saturating_add(u64::from(d - b'0'));
                digits += 1;
                j += 1;
            }
            if digits > 0 && xml.get(j) == Some(&b'"') {
                max = max.max(value);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    max
}
