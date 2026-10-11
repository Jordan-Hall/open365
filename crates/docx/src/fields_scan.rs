//! Finding fields in the typed tree and rewriting their results in place.

use crate::model::{Block, Body, BreakKind, Group, Inline, Paragraph, Run, RunContent, SdtChild};
use crate::model::{RowChild, TableChild};
use crate::props::RunProps;
use crate::xml::{ElementHead, RawXml, XmlElement};

/// Where a marker sits: paragraph index, path of inline indices (the last one is the run, or
/// the group for `w:fldSimple`) and the index of the item in the run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Loc {
    pub para: usize,
    pub path: Vec<usize>,
    pub item: usize,
}

/// A field as found in the tree, before any interpretation.
#[derive(Debug, Clone)]
pub(crate) struct RawField {
    pub simple: bool,
    pub begin: Loc,
    pub separate: Option<Loc>,
    pub end: Option<Loc>,
    /// Instruction text: `w:instrText` pieces plus the visible text of nested results.
    pub instr: String,
    pub depth: usize,
    pub parent: Option<usize>,
    pub locked: bool,
    pub dirty: bool,
    pub result_text: String,
    /// Properties of the first run that carries `w:instrText` (what Word gives a fresh result).
    pub code_props: Option<RunProps>,
    /// The runs of the current result with their text, for `\* MERGEFORMAT`.
    pub result_runs: Vec<(RunProps, String)>,
    /// Element head of the run holding the begin marker (to name new runs like it).
    pub run_head: Option<ElementHead>,
    /// True once the separator was seen (always for simple fields).
    pub in_result: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Code,
    Result,
}

struct Frame {
    field: usize,
    phase: Phase,
}

pub(crate) struct Scanner {
    pub fields: Vec<RawField>,
    stack: Vec<Frame>,
}

impl Scanner {
    /// The fields of one paragraph, assuming no field is open when it starts. Indices and
    /// parents are relative to the returned list.
    pub(crate) fn scan_one(index: usize, paragraph: &Paragraph) -> Vec<RawField> {
        let mut scanner = Self {
            fields: Vec::new(),
            stack: Vec::new(),
        };
        scanner.paragraph(index, paragraph);
        scanner.fields
    }

    pub(crate) fn scan(body: &Body) -> Vec<RawField> {
        let mut scanner = Self {
            fields: Vec::new(),
            stack: Vec::new(),
        };
        for (index, paragraph) in body.paragraphs().into_iter().enumerate() {
            scanner.paragraph(index, paragraph);
        }
        scanner.fields
    }

    fn paragraph(&mut self, index: usize, paragraph: &Paragraph) {
        let mut path = Vec::new();
        self.content(index, &paragraph.content, &mut path);
        // The paragraph mark belongs to every open result.
        self.add_visible("\r");
    }

    fn content(&mut self, para: usize, content: &[Inline], path: &mut Vec<usize>) {
        for (i, inline) in content.iter().enumerate() {
            path.push(i);
            match inline {
                Inline::Run(run) => self.run(para, run, path),
                Inline::Group(group) => self.group(para, group, path),
                Inline::Raw(_) => {}
            }
            path.pop();
        }
    }

    fn group(&mut self, para: usize, group: &Group, path: &mut Vec<usize>) {
        if group.head.is_w("del") || group.head.is_w("moveFrom") {
            return;
        }
        if group.head.is_w("fldSimple") {
            let instr = group.head.attr_w("instr").unwrap_or_default().to_owned();
            let parent = self.stack.last().map(|f| f.field);
            let idx = self.fields.len();
            self.fields.push(RawField {
                simple: true,
                begin: Loc {
                    para,
                    path: path.clone(),
                    item: 0,
                },
                separate: None,
                end: None,
                instr,
                depth: self.stack.len(),
                parent,
                locked: matches!(group.head.attr_w("fldLock"), Some("1" | "true" | "on")),
                dirty: matches!(group.head.attr_w("dirty"), Some("1" | "true" | "on")),
                result_text: String::new(),
                code_props: None,
                result_runs: Vec::new(),
                run_head: None,
                in_result: true,
            });
            self.stack.push(Frame {
                field: idx,
                phase: Phase::Result,
            });
            self.content(para, &group.content, path);
            self.stack.pop();
            return;
        }
        self.content(para, &group.content, path);
    }

    fn run(&mut self, para: usize, run: &Run, path: &[usize]) {
        for (item, content) in run.content.iter().enumerate() {
            match content {
                RunContent::Text { text, .. } => {
                    self.add_visible(text);
                    self.note_result_run(run, text);
                }
                RunContent::Tab => {
                    self.add_visible("\t");
                    self.note_result_run(run, "\t");
                }
                RunContent::Break(b) => {
                    let c = match b.kind() {
                        BreakKind::TextWrapping => "\u{b}",
                        BreakKind::Page => "\u{c}",
                        BreakKind::Column => "\u{e}",
                    };
                    self.add_visible(c);
                    self.note_result_run(run, c);
                }
                RunContent::Raw(RawXml::Element(e)) => {
                    let loc = || Loc {
                        para,
                        path: path.to_vec(),
                        item,
                    };
                    if e.head.is_w("fldChar") {
                        match e.head.attr_w("fldCharType") {
                            Some("begin") => self.begin(loc(), e, run),
                            Some("separate") => self.separate(loc()),
                            Some("end") => self.end(loc()),
                            _ => {}
                        }
                    } else if e.head.is_w("instrText") {
                        let text: String = e
                            .children
                            .iter()
                            .filter_map(|c| match c {
                                RawXml::Text(t) => Some(t.as_str()),
                                _ => None,
                            })
                            .collect();
                        self.add_instr(&text, run);
                    } else if e.head.is_w("noBreakHyphen") {
                        self.add_visible("-");
                    }
                }
                RunContent::Raw(_) => {}
            }
        }
    }

    fn begin(&mut self, loc: Loc, e: &XmlElement, run: &Run) {
        let idx = self.fields.len();
        let parent = self.stack.last().map(|f| f.field);
        self.fields.push(RawField {
            simple: false,
            begin: loc,
            separate: None,
            end: None,
            instr: String::new(),
            depth: self.stack.len(),
            parent,
            locked: matches!(e.head.attr_w("fldLock"), Some("1" | "true" | "on")),
            dirty: matches!(e.head.attr_w("dirty"), Some("1" | "true" | "on")),
            result_text: String::new(),
            code_props: None,
            result_runs: Vec::new(),
            run_head: Some(run.head.clone()),
            in_result: false,
        });
        self.stack.push(Frame {
            field: idx,
            phase: Phase::Code,
        });
    }

    fn separate(&mut self, loc: Loc) {
        if let Some(top) = self.stack.last_mut()
            && top.phase == Phase::Code
        {
            top.phase = Phase::Result;
            let field = &mut self.fields[top.field];
            field.separate = Some(loc);
            field.in_result = true;
        }
    }

    fn end(&mut self, loc: Loc) {
        if let Some(top) = self.stack.pop() {
            self.fields[top.field].end = Some(loc);
        }
    }

    fn add_instr(&mut self, text: &str, run: &Run) {
        if let Some(top) = self.stack.last()
            && top.phase == Phase::Code
        {
            let field = &mut self.fields[top.field];
            field.instr.push_str(text);
            if field.code_props.is_none() {
                field.code_props = Some(run.props.clone());
            }
        }
    }

    /// Visible text goes to every open result above the nearest open code, and into that
    /// code's instruction when it is a nested result.
    fn add_visible(&mut self, text: &str) {
        for frame in self.stack.iter().rev() {
            match frame.phase {
                Phase::Result => self.fields[frame.field].result_text.push_str(text),
                Phase::Code => {
                    // Nested result text is part of the displayed instruction; the paragraph
                    // mark never is.
                    if text != "\r" {
                        self.fields[frame.field].instr.push_str(text);
                    }
                    break;
                }
            }
        }
    }

    fn note_result_run(&mut self, run: &Run, text: &str) {
        // Only the innermost open result owns the run for MERGEFORMAT purposes, and only when
        // no code frame sits above it.
        if let Some(top) = self.stack.last()
            && top.phase == Phase::Result
        {
            self.fields[top.field]
                .result_runs
                .push((run.props.clone(), text.to_owned()));
        }
    }
}

// ---------------------------------------------------------------------------
// Paragraph addressing
// ---------------------------------------------------------------------------

/// One step from a block list into a nested block list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// `blocks[block]` is a table; descend into row child `row`, cell child `cell`.
    Cell {
        block: usize,
        row: usize,
        cell: usize,
    },
    /// `blocks[block]` is a content control; descend into its child `child`.
    Sdt { block: usize, child: usize },
}

/// A paragraph's place in the tree: the steps to its block list and its index there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    pub steps: Vec<Step>,
    pub pos: usize,
}

pub(crate) fn slots(body: &Body) -> Vec<Slot> {
    fn walk(blocks: &[Block], steps: &mut Vec<Step>, out: &mut Vec<Slot>) {
        for (bi, block) in blocks.iter().enumerate() {
            match block {
                Block::Paragraph(_) => out.push(Slot {
                    steps: steps.clone(),
                    pos: bi,
                }),
                Block::Table(t) => {
                    for (ri, rc) in t.children.iter().enumerate() {
                        let TableChild::Row(row) = rc else { continue };
                        for (ci, cc) in row.children.iter().enumerate() {
                            let RowChild::Cell(cell) = cc else { continue };
                            steps.push(Step::Cell {
                                block: bi,
                                row: ri,
                                cell: ci,
                            });
                            walk(&cell.blocks, steps, out);
                            steps.pop();
                        }
                    }
                }
                Block::Sdt(sdt) => {
                    for (ci, child) in sdt.children.iter().enumerate() {
                        let SdtChild::Content(content) = child else {
                            continue;
                        };
                        steps.push(Step::Sdt {
                            block: bi,
                            child: ci,
                        });
                        walk(&content.blocks, steps, out);
                        steps.pop();
                    }
                }
                Block::Raw(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&body.blocks, &mut Vec::new(), &mut out);
    out
}

/// The block list at the end of `steps`.
pub(crate) fn blocks_mut<'a>(body: &'a mut Body, steps: &[Step]) -> Option<&'a mut Vec<Block>> {
    let mut blocks = &mut body.blocks;
    for step in steps {
        blocks = match *step {
            Step::Cell { block, row, cell } => {
                let Block::Table(t) = blocks.get_mut(block)? else {
                    return None;
                };
                let TableChild::Row(r) = t.children.get_mut(row)? else {
                    return None;
                };
                let RowChild::Cell(c) = r.children.get_mut(cell)? else {
                    return None;
                };
                &mut c.blocks
            }
            Step::Sdt { block, child } => {
                let Block::Sdt(s) = blocks.get_mut(block)? else {
                    return None;
                };
                let SdtChild::Content(c) = s.children.get_mut(child)? else {
                    return None;
                };
                &mut c.blocks
            }
        };
    }
    Some(blocks)
}

pub(crate) fn paragraph_at<'a>(body: &'a mut Body, slot: &Slot) -> Option<&'a mut Paragraph> {
    match blocks_mut(body, &slot.steps)?.get_mut(slot.pos)? {
        Block::Paragraph(p) => Some(p),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Run navigation
// ---------------------------------------------------------------------------

/// The inline list that owns `path`'s last element, and that element's index.
pub(crate) fn inlines_mut<'a>(
    content: &'a mut Vec<Inline>,
    path: &[usize],
) -> Option<(&'a mut Vec<Inline>, usize)> {
    let (last, parents) = path.split_last()?;
    let mut list = content;
    for &i in parents {
        list = match list.get_mut(i)? {
            Inline::Group(g) => &mut g.content,
            _ => return None,
        };
    }
    Some((list, *last))
}

pub(crate) fn run_at_mut<'a>(content: &'a mut Vec<Inline>, path: &[usize]) -> Option<&'a mut Run> {
    let (list, i) = inlines_mut(content, path)?;
    match list.get_mut(i)? {
        Inline::Run(r) => Some(r),
        _ => None,
    }
}

pub(crate) fn group_at_mut<'a>(
    content: &'a mut Vec<Inline>,
    path: &[usize],
) -> Option<&'a mut Group> {
    let (list, i) = inlines_mut(content, path)?;
    match list.get_mut(i)? {
        Inline::Group(g) => Some(g),
        _ => None,
    }
}

/// Removes, in document order within `content`, every run item strictly between the
/// separator and the end marker. `inside` carries the state in and out so a field that
/// spans paragraphs can be stripped paragraph by paragraph. Runs and groups that become
/// empty are removed (never those holding a marker).
pub(crate) fn strip_between(
    content: &mut Vec<Inline>,
    path: &mut Vec<usize>,
    sep: Option<(&[usize], usize)>,
    end: Option<(&[usize], usize)>,
    inside: &mut bool,
) {
    // `i` indexes the current list; `orig` the original positions the markers refer to.
    let mut i = 0usize;
    let mut orig = 0usize;
    while i < content.len() {
        path.push(orig);
        orig += 1;
        let mut remove = false;
        match &mut content[i] {
            Inline::Run(run) => {
                let here_sep = sep.filter(|(p, _)| *p == path.as_slice()).map(|(_, it)| it);
                let here_end = end.filter(|(p, _)| *p == path.as_slice()).map(|(_, it)| it);
                let before = run.content.len();
                let mut keep = Vec::with_capacity(before);
                for (j, item) in std::mem::take(&mut run.content).into_iter().enumerate() {
                    let is_marker_boundary = Some(j) == here_sep || Some(j) == here_end;
                    if is_marker_boundary {
                        if Some(j) == here_sep {
                            *inside = true;
                        }
                        if Some(j) == here_end {
                            *inside = false;
                        }
                        keep.push(item);
                        continue;
                    }
                    if !*inside {
                        keep.push(item);
                    }
                }
                run.content = keep;
                let touched = run.content.len() != before;
                if touched && run.content.is_empty() {
                    remove = true;
                }
            }
            Inline::Group(group) => {
                if group.head.is_w("del") || group.head.is_w("moveFrom") {
                    // Not part of the visible result.
                } else {
                    let before = group.content.len();
                    strip_between(&mut group.content, path, sep, end, inside);
                    if group.content.is_empty() && before > 0 {
                        remove = true;
                    }
                }
            }
            Inline::Raw(_) => {
                // Bookmark markers and the like inside a stripped result go with it.
                if *inside {
                    remove = true;
                }
            }
        }
        path.pop();
        if remove {
            content.remove(i);
        } else {
            i += 1;
        }
    }
}

/// A new run named like `owner`, with the given properties and content.
pub(crate) fn new_run(owner: &ElementHead, props: RunProps, content: Vec<RunContent>) -> Run {
    Run {
        head: owner.sibling("r"),
        props,
        content,
    }
}

/// A `w:fldChar` item of the given type, named after `owner` (the run head).
pub(crate) fn fld_char(owner: &ElementHead, kind: &str) -> RunContent {
    let mut head = owner.sibling("fldChar");
    head.set_attr_w("fldCharType", kind);
    RunContent::Raw(RawXml::Element(head.into_empty()))
}

/// The block list at the end of `steps`, read-only.
pub(crate) fn blocks_ref<'a>(body: &'a Body, steps: &[Step]) -> Option<&'a Vec<Block>> {
    let mut blocks = &body.blocks;
    for step in steps {
        blocks = match *step {
            Step::Cell { block, row, cell } => {
                let Block::Table(t) = blocks.get(block)? else {
                    return None;
                };
                let TableChild::Row(r) = t.children.get(row)? else {
                    return None;
                };
                let RowChild::Cell(c) = r.children.get(cell)? else {
                    return None;
                };
                &c.blocks
            }
            Step::Sdt { block, child } => {
                let Block::Sdt(s) = blocks.get(block)? else {
                    return None;
                };
                let SdtChild::Content(c) = s.children.get(child)? else {
                    return None;
                };
                &c.blocks
            }
        };
    }
    Some(blocks)
}

pub(crate) fn paragraph_ref<'a>(body: &'a Body, slot: &Slot) -> Option<&'a Paragraph> {
    match blocks_ref(body, &slot.steps)?.get(slot.pos)? {
        Block::Paragraph(p) => Some(p),
        _ => None,
    }
}
