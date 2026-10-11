//! Editing across runs and paragraphs: range deletion with paragraph merging, paragraph
//! splitting, snapshots for cheap undo, and the field/marker bookkeeping that keeps the
//! result a package Word opens without repair.
//!
//! Rules for zero-width content inside a deleted range (positions are [`DocPos`]; a unit is
//! *interior* when it lies strictly between the range ends):
//! * bookmarks, comment/permission range markers and comment references are kept;
//! * a complex field is deleted whole when the range covers its result, and a deletion that
//!   would cut it partially is refused with [`DocxError::FieldCut`];
//! * drawings and other content are removed and recorded in [`Document::edit_log`].

use serde::{Deserialize, Serialize};

use crate::DocxError;
use crate::document::Document;
use crate::edit::needs_preserve;
use crate::model::{
    Block, Body, Group, Inline, Paragraph, RowChild, Run, RunContent, SdtChild, TableChild,
};
use crate::props::RunPropsPatch;
use crate::xml::{RawXml, XmlElement};

/// A position in the document: paragraph index and character offset (Unicode scalar values)
/// in that paragraph's text.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct DocPos {
    pub para: usize,
    pub offset: usize,
}

impl DocPos {
    pub fn new(para: usize, offset: usize) -> Self {
        Self { para, offset }
    }
}

/// Content an edit removed that the user may not have seen going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditLogEntry {
    /// Paragraph the content was in when it was removed.
    pub para: usize,
    pub description: String,
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

/// One hop into a container: a table cell (block index, table child index, row child
/// index) or the content of a block-level control (block index, control child index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Cell {
        block: usize,
        child: usize,
        cell: usize,
    },
    Sdt {
        block: usize,
        child: usize,
    },
}

type Path = Vec<Step>;

/// Index of paragraph number `target` within its container; `path` leads to the container.
fn find_paragraph(
    blocks: &[Block],
    seen: &mut usize,
    target: usize,
    path: &mut Path,
) -> Option<usize> {
    for (bi, block) in blocks.iter().enumerate() {
        match block {
            Block::Paragraph(_) => {
                if *seen == target {
                    return Some(bi);
                }
                *seen += 1;
            }
            Block::Table(table) => {
                for (ci, child) in table.children.iter().enumerate() {
                    let TableChild::Row(row) = child else {
                        continue;
                    };
                    for (ri, rc) in row.children.iter().enumerate() {
                        let RowChild::Cell(cell) = rc else { continue };
                        path.push(Step::Cell {
                            block: bi,
                            child: ci,
                            cell: ri,
                        });
                        if let Some(found) = find_paragraph(&cell.blocks, seen, target, path) {
                            return Some(found);
                        }
                        path.pop();
                    }
                }
            }
            Block::Sdt(sdt) => {
                for (ci, child) in sdt.children.iter().enumerate() {
                    let SdtChild::Content(content) = child else {
                        continue;
                    };
                    path.push(Step::Sdt {
                        block: bi,
                        child: ci,
                    });
                    if let Some(found) = find_paragraph(&content.blocks, seen, target, path) {
                        return Some(found);
                    }
                    path.pop();
                }
            }
            Block::Raw(_) => {}
        }
    }
    None
}

fn locate_paragraph(body: &Body, para: usize) -> Option<(Path, usize)> {
    let mut path = Vec::new();
    let mut seen = 0;
    let index = find_paragraph(&body.blocks, &mut seen, para, &mut path)?;
    Some((path, index))
}

fn container_mut<'a>(body: &'a mut Body, path: &[Step]) -> Option<&'a mut Vec<Block>> {
    let mut blocks = &mut body.blocks;
    for step in path {
        blocks = match *step {
            Step::Cell { block, child, cell } => {
                let Block::Table(table) = blocks.get_mut(block)? else {
                    return None;
                };
                let TableChild::Row(row) = table.children.get_mut(child)? else {
                    return None;
                };
                let RowChild::Cell(cell) = row.children.get_mut(cell)? else {
                    return None;
                };
                &mut cell.blocks
            }
            Step::Sdt { block, child } => {
                let Block::Sdt(sdt) = blocks.get_mut(block)? else {
                    return None;
                };
                let SdtChild::Content(content) = sdt.children.get_mut(child)? else {
                    return None;
                };
                &mut content.blocks
            }
        };
    }
    Some(blocks)
}

fn container<'a>(body: &'a Body, path: &[Step]) -> Option<&'a Vec<Block>> {
    let mut blocks = &body.blocks;
    for step in path {
        blocks = match *step {
            Step::Cell { block, child, cell } => {
                let Block::Table(table) = blocks.get(block)? else {
                    return None;
                };
                let TableChild::Row(row) = table.children.get(child)? else {
                    return None;
                };
                let RowChild::Cell(cell) = row.children.get(cell)? else {
                    return None;
                };
                &cell.blocks
            }
            Step::Sdt { block, child } => {
                let Block::Sdt(sdt) = blocks.get(block)? else {
                    return None;
                };
                let SdtChild::Content(content) = sdt.children.get(child)? else {
                    return None;
                };
                &content.blocks
            }
        };
    }
    Some(blocks)
}

/// The blocks of a paragraph range, saved so an edit can be undone without replay.
#[derive(Debug, Clone)]
pub struct Snapshot {
    path: Path,
    start: usize,
    blocks: Vec<Block>,
    container_len: usize,
    log_len: usize,
}

// ---------------------------------------------------------------------------
// Classification of zero-width content
// ---------------------------------------------------------------------------

/// Range markers: kept (and so moved to the join point) when a deletion covers them.
fn is_marker(raw: &RawXml) -> bool {
    match raw {
        RawXml::Element(e) => {
            let name = e.head.local_name();
            matches!(
                name,
                "bookmarkStart" | "bookmarkEnd" | "permStart" | "permEnd"
            ) || name.ends_with("RangeStart")
                || name.ends_with("RangeEnd")
        }
        _ => true,
    }
}

/// Run content that stays with the run it is in.
const KEEP_IN_RUN: &[&str] = &[
    "commentReference",
    "delText",
    "delInstrText",
    "annotationRef",
    "footnoteRef",
    "endnoteRef",
    "separator",
    "continuationSeparator",
];

/// Layout caches that are regenerated: dropped without a log entry.
const SILENT: &[&str] = &["lastRenderedPageBreak", "proofErr"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldPart {
    Begin,
    Separate,
    End,
    /// `w:instrText`.
    Instruction,
    /// A `w:fldChar` with an unknown type.
    Other,
}

fn field_part(raw: &RawXml) -> Option<FieldPart> {
    let RawXml::Element(e) = raw else { return None };
    if e.head.is_w("instrText") {
        return Some(FieldPart::Instruction);
    }
    if e.head.is_w("fldChar") {
        return Some(match e.head.attr_w("fldCharType") {
            Some("begin") => FieldPart::Begin,
            Some("separate") => FieldPart::Separate,
            Some("end") => FieldPart::End,
            _ => FieldPart::Other,
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Field plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct FieldEvent {
    pos: DocPos,
    part: FieldPart,
}

fn collect_events(content: &[Inline], para: usize, acc: &mut usize, out: &mut Vec<FieldEvent>) {
    for inline in content {
        match inline {
            Inline::Run(run) => {
                let mut a = *acc;
                for item in &run.content {
                    if let RunContent::Raw(raw) = item
                        && let Some(part) = field_part(raw)
                    {
                        out.push(FieldEvent {
                            pos: DocPos::new(para, a),
                            part,
                        });
                    }
                    a += item.char_len();
                }
                *acc = a;
            }
            Inline::Group(g) => collect_events(&g.content, para, acc, out),
            Inline::Raw(_) => {}
        }
    }
}

fn paragraph_events(p: &Paragraph, para: usize, out: &mut Vec<FieldEvent>) {
    let mut acc = 0;
    collect_events(&p.content, para, &mut acc, out);
}

/// Which field units a deletion removes, indexed by the unit's ordinal (document order over
/// all `w:fldChar` and `w:instrText` units).
#[derive(Debug, Default)]
struct FieldPlan {
    remove: Vec<bool>,
    kinds: Vec<Option<FieldPart>>,
    /// Ordinal of the first unit at or after the first affected paragraph.
    first_ordinal: usize,
}

impl FieldPlan {
    fn build(paragraphs: &[&Paragraph], s: DocPos, e: DocPos) -> Result<Self, DocxError> {
        // Cheap test first: a range without field units near it needs no document scan.
        let mut near = Vec::new();
        for (q, p) in paragraphs.iter().enumerate().take(e.para + 1).skip(s.para) {
            paragraph_events(p, q, &mut near);
        }
        if !near.iter().any(|ev| ev.pos >= s && ev.pos <= e) {
            return Ok(Self::default());
        }

        let mut events = Vec::new();
        let mut first_ordinal = 0;
        for (q, p) in paragraphs.iter().enumerate() {
            if q == s.para {
                first_ordinal = events.len();
            }
            paragraph_events(p, q, &mut events);
        }

        struct Field {
            begin: DocPos,
            end: Option<DocPos>,
            parts: Vec<usize>,
            instructions: Vec<usize>,
        }
        let mut fields: Vec<Field> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for (ordinal, ev) in events.iter().enumerate() {
            match ev.part {
                FieldPart::Begin => {
                    fields.push(Field {
                        begin: ev.pos,
                        end: None,
                        parts: vec![ordinal],
                        instructions: Vec::new(),
                    });
                    stack.push(fields.len() - 1);
                }
                FieldPart::Separate => {
                    if let Some(&top) = stack.last() {
                        fields[top].parts.push(ordinal);
                    }
                }
                FieldPart::End => {
                    if let Some(top) = stack.pop() {
                        fields[top].parts.push(ordinal);
                        fields[top].end = Some(ev.pos);
                    }
                }
                FieldPart::Instruction => {
                    if let Some(&top) = stack.last() {
                        fields[top].instructions.push(ordinal);
                    }
                }
                FieldPart::Other => {}
            }
        }

        let mut remove = vec![false; events.len()];
        for field in &fields {
            let covered = field
                .end
                .is_some_and(|end| field.begin >= s && end <= e && field.begin < end);
            if covered {
                for &o in field.parts.iter().chain(&field.instructions) {
                    remove[o] = true;
                }
            } else if field
                .parts
                .iter()
                .any(|&o| events[o].pos > s && events[o].pos < e)
            {
                return Err(DocxError::FieldCut);
            }
        }
        Ok(Self {
            remove,
            kinds: events.iter().map(|ev| Some(ev.part)).collect(),
            first_ordinal,
        })
    }

    fn removes(&self, ordinal: usize) -> bool {
        self.remove.get(ordinal).copied().unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// Pruning
// ---------------------------------------------------------------------------

enum Action {
    Keep,
    Drop,
}

struct Prune<'a> {
    s: DocPos,
    e: DocPos,
    plan: &'a FieldPlan,
    ordinal: usize,
    log: Vec<EditLogEntry>,
    /// Paragraph being processed and the character range to delete in it.
    para: usize,
    lo: usize,
    hi: usize,
    /// Character lengths of the first and last paragraph of the range.
    s_len: usize,
    e_len: usize,
}

impl Prune<'_> {
    /// True when a zero-width unit at `offset` of the current paragraph is inside the range.
    /// Units at the range ends are kept, except that a range reaching the very start of its
    /// first paragraph (or the very end of its last one) also takes the units that lead
    /// (or trail) the paragraph's text, so that selecting everything removes everything.
    fn interior(&self, offset: usize) -> bool {
        let pos = DocPos::new(self.para, offset);
        if pos > self.s && pos < self.e {
            return true;
        }
        let from_start =
            self.s.offset == 0 && (self.e.para > self.s.para || self.e.offset >= self.s_len);
        if self.para == self.s.para && offset == 0 && from_start {
            return true;
        }
        let to_end =
            self.e.offset >= self.e_len && (self.s.para < self.e.para || self.s.offset == 0);
        self.para == self.e.para && offset == self.e_len && to_end
    }

    fn note(&mut self, description: String) {
        self.log.push(EditLogEntry {
            para: self.para,
            description,
        });
    }

    /// Decides the fate of one raw item inside a run at character offset `at`.
    fn run_raw(&mut self, raw: &RawXml, at: usize) -> Action {
        if field_part(raw).is_some() {
            let ordinal = self.ordinal;
            self.ordinal += 1;
            if self.plan.removes(ordinal) {
                if self.plan.kinds.get(ordinal).copied().flatten() == Some(FieldPart::Begin) {
                    self.note("deleted a field with its result".to_owned());
                }
                return Action::Drop;
            }
            return Action::Keep;
        }
        let RawXml::Element(e) = raw else {
            return Action::Keep;
        };
        if !self.interior(at) {
            return Action::Keep;
        }
        let name = e.head.local_name();
        if KEEP_IN_RUN.contains(&name) {
            return Action::Keep;
        }
        if !SILENT.contains(&name) {
            self.note(format!("deleted <{}> content", e.head.name));
        }
        Action::Drop
    }

    fn run(&mut self, mut run: Run, acc: &mut usize) -> Option<Run> {
        let mut a = *acc;
        let mut changed = false;
        let old = std::mem::take(&mut run.content);
        let mut out = Vec::with_capacity(old.len());
        for item in old {
            match item {
                RunContent::Text {
                    text,
                    preserve_space,
                    attrs,
                } => {
                    let n = text.chars().count();
                    let from = self.lo.max(a);
                    let to = self.hi.min(a + n);
                    if from < to {
                        changed = true;
                        let kept: String = text
                            .chars()
                            .enumerate()
                            .filter(|(i, _)| a + i < from || a + i >= to)
                            .map(|(_, c)| c)
                            .collect();
                        if !kept.is_empty() {
                            out.push(RunContent::Text {
                                preserve_space: preserve_space || needs_preserve(&kept),
                                text: kept,
                                attrs,
                            });
                        }
                    } else {
                        out.push(RunContent::Text {
                            text,
                            preserve_space,
                            attrs,
                        });
                    }
                    a += n;
                }
                item @ (RunContent::Tab | RunContent::Break(_)) => {
                    if a >= self.lo && a < self.hi {
                        changed = true;
                    } else {
                        out.push(item);
                    }
                    a += 1;
                }
                RunContent::Raw(raw) => match self.run_raw(&raw, a) {
                    Action::Keep => out.push(RunContent::Raw(raw)),
                    Action::Drop => changed = true,
                },
            }
        }
        run.content = out;
        *acc = a;
        if changed && run.content.is_empty() {
            None
        } else {
            Some(run)
        }
    }

    fn inlines(&mut self, content: Vec<Inline>, acc: &mut usize) -> Vec<Inline> {
        let mut out = Vec::with_capacity(content.len());
        for inline in content {
            match inline {
                Inline::Run(run) => {
                    if let Some(run) = self.run(run, acc) {
                        out.push(Inline::Run(run));
                    }
                }
                Inline::Group(Group { head, content }) => {
                    let had_content = !content.is_empty();
                    let content = self.inlines(content, acc);
                    if !(had_content && content.is_empty()) {
                        out.push(Inline::Group(Group { head, content }));
                    }
                }
                Inline::Raw(raw) => {
                    if is_marker(&raw) || !self.interior(*acc) {
                        out.push(Inline::Raw(raw));
                        continue;
                    }
                    let RawXml::Element(e) = &raw else {
                        out.push(Inline::Raw(raw));
                        continue;
                    };
                    if SILENT.contains(&e.head.local_name()) {
                        continue;
                    }
                    self.note(format!("deleted <{}> content", e.head.name));
                    // Markers inside removed content must survive to keep ranges balanced.
                    let mut markers = Vec::new();
                    collect_markers(e, &mut markers);
                    out.extend(markers.into_iter().map(|m| Inline::Raw(RawXml::Element(m))));
                }
            }
        }
        out
    }
}

fn collect_markers(e: &XmlElement, out: &mut Vec<XmlElement>) {
    for child in &e.children {
        if let RawXml::Element(c) = child {
            if is_marker(child) {
                out.push(c.clone());
            } else {
                collect_markers(c, out);
            }
        }
    }
}

fn has_section_break(p: &Paragraph) -> bool {
    p.props
        .as_ref()
        .is_some_and(|props| crate::ppr::child(props, "sectPr").is_some())
}

// ---------------------------------------------------------------------------
// Splitting
// ---------------------------------------------------------------------------

/// True for range-start markers, which belong to the text after a split point.
fn starts_range(raw: &RawXml) -> bool {
    matches!(raw, RawXml::Element(e) if {
        let n = e.head.local_name();
        n == "bookmarkStart" || n == "permStart" || n.ends_with("RangeStart")
    })
}

/// Splits inline content at character `offset`. Zero-width items at the split point stay
/// left, except range starts, which go right. Groups spanning the point are cloned.
fn split_inlines(
    content: Vec<Inline>,
    offset: usize,
    acc: &mut usize,
    right_started: &mut bool,
) -> (Vec<Inline>, Vec<Inline>) {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for inline in content {
        if *right_started {
            *acc += inline.char_len();
            right.push(inline);
            continue;
        }
        match inline {
            Inline::Run(mut run) => {
                let len = run.char_len();
                if len == 0 || *acc + len <= offset {
                    left.push(Inline::Run(run));
                } else if *acc >= offset {
                    *right_started = true;
                    right.push(Inline::Run(run));
                } else {
                    let tail = run.split_at(offset - *acc);
                    *right_started = true;
                    left.push(Inline::Run(run));
                    right.push(Inline::Run(tail));
                }
                *acc += len;
            }
            Inline::Group(g) => {
                let len: usize = g.content.iter().map(Inline::char_len).sum();
                if *acc + len <= offset {
                    *acc += len;
                    left.push(Inline::Group(g));
                } else if *acc >= offset {
                    *right_started = true;
                    *acc += len;
                    right.push(Inline::Group(g));
                } else {
                    let (l, r) = split_inlines(g.content, offset, acc, right_started);
                    if !l.is_empty() {
                        left.push(Inline::Group(Group {
                            head: g.head.clone(),
                            content: l,
                        }));
                    }
                    if !r.is_empty() {
                        right.push(Inline::Group(Group {
                            head: g.head,
                            content: r,
                        }));
                    }
                }
            }
            Inline::Raw(raw) => {
                if *acc >= offset && starts_range(&raw) {
                    *right_started = true;
                    right.push(Inline::Raw(raw));
                } else {
                    left.push(Inline::Raw(raw));
                }
            }
        }
    }
    (left, right)
}

/// Attributes that identify one paragraph and must not be duplicated on a split.
fn strip_identity(p: &mut Paragraph) {
    p.head
        .attrs
        .retain(|a| !matches!(a.local_name(), "paraId" | "textId"));
}

// ---------------------------------------------------------------------------
// Document API
// ---------------------------------------------------------------------------

impl Document {
    fn check_pos(&self, pos: DocPos) -> Result<(), DocxError> {
        let count = self.paragraph_count();
        let para = self
            .paragraph(pos.para)
            .ok_or(DocxError::ParagraphOutOfRange {
                index: pos.para,
                count,
            })?;
        let len = para.char_len();
        if pos.offset > len {
            return Err(DocxError::RangeOutOfBounds {
                start: pos.offset,
                end: pos.offset,
                len,
            });
        }
        Ok(())
    }

    /// Content removed by edits that was not plain text (drawings, fields, section breaks).
    /// Undo takes entries back out.
    pub fn edit_log(&self) -> &[EditLogEntry] {
        &self.edit_log
    }

    /// Deletes the characters between two positions (either order), across runs and
    /// paragraphs. Paragraphs in the range merge into one; the first paragraph's properties
    /// win unless it is empty, in which case the last paragraph's do (as in Word). See the
    /// module documentation for how bookmarks, comments, fields and drawings are treated.
    ///
    /// The range must lie in one container (the body or one table cell) and may not cover a
    /// table or content control. On error nothing changes.
    pub fn delete_range(&mut self, start: DocPos, end: DocPos) -> Result<(), DocxError> {
        let (s, e) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.check_pos(s)?;
        self.check_pos(e)?;
        if s == e {
            return Ok(());
        }
        let plan = {
            let paragraphs = self.body().paragraphs();
            FieldPlan::build(&paragraphs, s, e)?
        };
        let (path, bi_s) = locate_paragraph(self.body(), s.para)
            .ok_or_else(|| DocxError::UnsupportedRange("paragraph not found".to_owned()))?;
        let (path_e, bi_e) = locate_paragraph(self.body(), e.para)
            .ok_or_else(|| DocxError::UnsupportedRange("paragraph not found".to_owned()))?;
        if path != path_e {
            return Err(DocxError::UnsupportedRange(
                "the range crosses a table or content control boundary".to_owned(),
            ));
        }
        let s_len = self.paragraph(s.para).map_or(0, Paragraph::char_len);
        let e_len = self.paragraph(e.para).map_or(0, Paragraph::char_len);
        let first_empty = self
            .paragraph(s.para)
            .is_some_and(|p| p.char_len() == 0 && s.offset == 0);

        let body = self.body_mut();
        let blocks = container_mut(body, &path)
            .ok_or_else(|| DocxError::UnsupportedRange("container not found".to_owned()))?;
        let mut paragraph_blocks = 0;
        for block in &blocks[bi_s..=bi_e] {
            match block {
                Block::Paragraph(_) => paragraph_blocks += 1,
                Block::Raw(raw) if is_marker(raw) => {}
                Block::Raw(_) | Block::Sdt(_) => {
                    return Err(DocxError::UnsupportedRange(
                        "the range covers a content control or other block".to_owned(),
                    ));
                }
                Block::Table(_) => {
                    return Err(DocxError::UnsupportedRange(
                        "the range covers a table".to_owned(),
                    ));
                }
            }
        }
        if paragraph_blocks != e.para - s.para + 1 {
            return Err(DocxError::UnsupportedRange(
                "paragraph numbering does not match the container".to_owned(),
            ));
        }

        let drained: Vec<Block> = blocks.drain(bi_s..=bi_e).collect();
        let mut prune = Prune {
            s,
            e,
            plan: &plan,
            ordinal: plan.first_ordinal,
            log: Vec::new(),
            para: s.para,
            lo: 0,
            hi: 0,
            s_len,
            e_len,
        };
        let mut pruned: Vec<Paragraph> = Vec::new();
        let mut raws: Vec<Block> = Vec::new();
        let mut q = s.para;
        for block in drained {
            match block {
                Block::Paragraph(mut p) => {
                    prune.para = q;
                    prune.lo = if q == s.para { s.offset } else { 0 };
                    prune.hi = if q == e.para { e.offset } else { usize::MAX };
                    let mut acc = 0;
                    p.content = prune.inlines(std::mem::take(&mut p.content), &mut acc);
                    pruned.push(p);
                    q += 1;
                }
                other => raws.push(other),
            }
        }

        let survivor_last = first_empty && pruned.len() > 1;
        let mut merged: Vec<Inline> = Vec::new();
        let mut dropped_sections = Vec::new();
        let count = pruned.len();
        let mut survivor: Option<Paragraph> = None;
        for (i, mut p) in pruned.into_iter().enumerate() {
            merged.append(&mut p.content);
            let is_survivor = if survivor_last {
                i + 1 == count
            } else {
                i == 0
            };
            if is_survivor {
                survivor = Some(p);
            } else if has_section_break(&p) {
                dropped_sections.push(s.para + i);
            }
        }
        let Some(mut survivor) = survivor else {
            return Err(DocxError::UnsupportedRange("empty range".to_owned()));
        };
        survivor.content = merged;
        let mut rebuilt: Vec<Block> = Vec::new();
        if survivor_last {
            rebuilt.extend(raws);
            rebuilt.push(Block::Paragraph(survivor));
        } else {
            rebuilt.push(Block::Paragraph(survivor));
            rebuilt.extend(raws);
        }
        blocks.splice(bi_s..bi_s, rebuilt);

        let mut log = prune.log;
        for para in dropped_sections {
            log.push(EditLogEntry {
                para,
                description: "removed a section break (the sections merged)".to_owned(),
            });
        }
        self.edit_log.extend(log);
        self.edited();
        Ok(())
    }

    /// Splits a paragraph at `pos` (the Enter key). The paragraph keeps the text before the
    /// caret; a new paragraph after it takes the text after, a copy of the paragraph
    /// properties and, when it is empty, the run properties at the caret as its paragraph
    /// mark. A section break stays with the second half.
    pub fn split_paragraph(&mut self, pos: DocPos) -> Result<(), DocxError> {
        self.check_pos(pos)?;
        let (path, bi) = locate_paragraph(self.body(), pos.para)
            .ok_or_else(|| DocxError::UnsupportedRange("paragraph not found".to_owned()))?;
        let body = self.body_mut();
        let blocks = container_mut(body, &path)
            .ok_or_else(|| DocxError::UnsupportedRange("container not found".to_owned()))?;
        let Some(Block::Paragraph(p)) = blocks.get_mut(bi) else {
            return Err(DocxError::UnsupportedRange(
                "paragraph not found".to_owned(),
            ));
        };
        let caret_props = p.run_props_at(pos.offset);
        let (left, right) = split_inlines(
            std::mem::take(&mut p.content),
            pos.offset,
            &mut 0,
            &mut false,
        );
        let mut tail = Paragraph {
            head: p.head.clone(),
            props: p.props.clone(),
            content: right,
        };
        strip_identity(&mut tail);
        p.content = left;
        // The section break (if any) ends the second half.
        if let Some(props) = &mut p.props {
            crate::ppr::remove_child(props, "sectPr");
        }
        if tail.char_len() == 0 && caret_props.is_present() {
            tail.set_mark_run_props(&caret_props);
        }
        blocks.insert(bi + 1, Block::Paragraph(tail));
        self.edited();
        Ok(())
    }

    /// Applies `patch` to the characters between two positions (either order), across
    /// paragraphs. Runs are split at the ends and keep their other properties.
    pub fn set_run_props(
        &mut self,
        start: DocPos,
        end: DocPos,
        patch: &RunPropsPatch,
    ) -> Result<(), DocxError> {
        let (s, e) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.check_pos(s)?;
        self.check_pos(e)?;
        if s == e || patch.is_empty() {
            return Ok(());
        }
        for q in s.para..=e.para {
            let para = self.paragraph_mut(q)?;
            let lo = if q == s.para { s.offset } else { 0 };
            let hi = if q == e.para {
                e.offset
            } else {
                para.char_len()
            };
            if lo < hi {
                para.apply_run_props(lo..hi, patch)?;
            }
        }
        self.edited();
        Ok(())
    }

    /// Replaces the range with `text` (typing over a selection, paste). `\n` in `text`
    /// starts a new paragraph, `\t` is a tab. Returns the position after the inserted text.
    pub fn replace_range(
        &mut self,
        start: DocPos,
        end: DocPos,
        text: &str,
    ) -> Result<DocPos, DocxError> {
        let (s, e) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        if let Some(bad) = text
            .chars()
            .find(|c| !matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'))
        {
            return Err(DocxError::InvalidText(bad));
        }
        self.delete_range(s, e)?;
        let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut at = s;
        for (i, line) in normalised.split('\n').enumerate() {
            if i > 0 {
                self.split_paragraph(at)?;
                at = DocPos::new(at.para + 1, 0);
            }
            if !line.is_empty() {
                self.insert_text(at.para, at.offset, line)?;
                at.offset += line.chars().count();
            }
        }
        Ok(at)
    }

    /// Saves paragraphs `first..=last` (which must share a container) for [`Document::restore`].
    pub fn capture(&self, first: usize, last: usize) -> Result<Snapshot, DocxError> {
        let count = self.paragraph_count();
        for index in [first, last] {
            if index >= count {
                return Err(DocxError::ParagraphOutOfRange { index, count });
            }
        }
        let (path, bi_first) = locate_paragraph(self.body(), first)
            .ok_or_else(|| DocxError::UnsupportedRange("paragraph not found".to_owned()))?;
        let (path_last, bi_last) = locate_paragraph(self.body(), last)
            .ok_or_else(|| DocxError::UnsupportedRange("paragraph not found".to_owned()))?;
        if path != path_last || bi_last < bi_first {
            return Err(DocxError::UnsupportedRange(
                "the paragraphs are not in one container".to_owned(),
            ));
        }
        let blocks = container(self.body(), &path)
            .ok_or_else(|| DocxError::UnsupportedRange("container not found".to_owned()))?;
        Ok(Snapshot {
            path,
            start: bi_first,
            blocks: blocks[bi_first..=bi_last].to_vec(),
            container_len: blocks.len(),
            log_len: self.edit_log.len(),
        })
    }

    /// Puts a captured range back, undoing the edit made since [`Document::capture`] provided
    /// the edit only changed those paragraphs (and possibly their number).
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), DocxError> {
        let body = self.body_mut();
        let blocks = container_mut(body, &snapshot.path)
            .ok_or_else(|| DocxError::UnsupportedRange("container not found".to_owned()))?;
        let now = (snapshot.blocks.len() + blocks.len()).checked_sub(snapshot.container_len);
        let end = now.map(|n| snapshot.start + n);
        let Some(end) = end.filter(|e| *e <= blocks.len()) else {
            return Err(DocxError::UnsupportedRange(
                "the document changed since the snapshot".to_owned(),
            ));
        };
        blocks.splice(snapshot.start..end, snapshot.blocks.iter().cloned());
        self.edit_log.truncate(snapshot.log_len);
        self.edited();
        Ok(())
    }
}
