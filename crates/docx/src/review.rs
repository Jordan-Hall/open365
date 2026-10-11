//! Tracked changes (ECMA-376 Part 1 §17.13.5): listing, accepting and rejecting revisions
//! and the "final" and "markup" views of paragraph text.
//!
//! The model keeps revisions as they are in the file: `w:ins`, `w:del`, `w:moveFrom` and
//! `w:moveTo` are [`Group`]s, `w:delText` is raw run content, and the `*PrChange` elements
//! and paragraph-mark `w:ins`/`w:del` stay inside the raw property elements. An untouched
//! document therefore round-trips unchanged; only accepting, rejecting or recording a change
//! rewrites the main part.
//!
//! Revision indices are positions in [`Document::revisions`] (document order, a container
//! before the revisions nested in it, a paragraph's content before its property changes and
//! its paragraph mark). They are valid until the next accept, reject or tracked edit.
//!
//! Resulting text follows Word (measured with `tools/office/review-oracle.ps1`): accepting an
//! insertion keeps its text, rejecting it removes the text; the reverse for deletions. A
//! paragraph mark that goes (accepted deletion, rejected insertion) joins the paragraph with
//! the next one.

use std::collections::HashSet;

use crate::DocxError;
use crate::document::Document;
use crate::edit::needs_preserve;
use crate::model::{
    Block, Break, Cell, Group, Inline, Paragraph, Row, RowChild, Run, RunContent, SdtChild, Table,
    TableChild,
};
use crate::props::RunProps;
use crate::xml::{Attr, ElementHead, RawXml, XmlElement, sibling_name};

/// What kind of change a [`Revision`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevisionKind {
    /// `w:ins` around runs.
    Insertion,
    /// `w:del` around runs.
    Deletion,
    /// `w:moveFrom`: the source of a move (text is shown as deleted).
    MovedFrom,
    /// `w:moveTo`: the destination of a move.
    MovedTo,
    /// An inserted paragraph mark (`w:pPr/w:rPr/w:ins`).
    ParagraphMarkInserted,
    /// A deleted paragraph mark (`w:pPr/w:rPr/w:del`).
    ParagraphMarkDeleted,
    /// `w:rPrChange` on a run (or on a paragraph mark).
    RunFormat,
    /// `w:pPrChange`.
    ParagraphFormat,
    /// `w:tblPrChange`.
    TableFormat,
    /// `w:sectPrChange`.
    SectionFormat,
    /// `w:trPr/w:ins`: an inserted table row.
    RowInserted,
    /// `w:trPr/w:del`: a deleted table row.
    RowDeleted,
    /// `w:tcPr/w:cellIns`: an inserted table cell (Word: `CellInsertion`).
    CellInserted,
    /// `w:tcPr/w:cellDel`: a deleted table cell (Word: `CellDeletion`).
    CellDeleted,
    /// `w:tcPr/w:cellMerge`: a vertical merge or split of a cell (Word: `CellMerge`).
    CellMerged,
}

impl RevisionKind {
    /// Name of Word's `WdRevisionType` for this kind (`Revision.Type`).
    pub fn word_type_name(self) -> &'static str {
        match self {
            Self::Insertion | Self::ParagraphMarkInserted | Self::RowInserted => "Insert",
            Self::Deletion | Self::ParagraphMarkDeleted | Self::RowDeleted => "Delete",
            Self::MovedFrom => "MovedFrom",
            Self::MovedTo => "MovedTo",
            Self::RunFormat => "Property",
            Self::ParagraphFormat => "ParagraphProperty",
            Self::TableFormat => "TableProperty",
            Self::SectionFormat => "SectionProperty",
            Self::CellInserted => "CellInsertion",
            Self::CellDeleted => "CellDeletion",
            Self::CellMerged => "CellMerge",
        }
    }

    /// True for the kinds whose text is part of the final view.
    pub fn is_deletion_like(self) -> bool {
        matches!(self, Self::Deletion | Self::MovedFrom)
    }
}

/// One tracked change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// Position in the list; pass it to [`Document::accept_revision`].
    pub index: usize,
    pub kind: RevisionKind,
    pub author: String,
    /// `w:date` as written (ISO 8601).
    pub date: Option<String>,
    /// `w:id` as written.
    pub id: Option<String>,
    /// Paragraph the change starts in (see [`Document::paragraph`]).
    pub paragraph: usize,
    /// Last paragraph the change touches (equals `paragraph` for a change inside one).
    pub last_paragraph: usize,
    /// `w:name` of the move range this revision belongs to (moves only); both halves of a
    /// move share it.
    pub move_name: Option<String>,
    /// Names of the other moves whose range starts inside this move's range. Word accepts
    /// those too when it accepts this revision (rejecting leaves them).
    pub(crate) link_moves: Vec<String>,
    /// Character offset in the paragraph's final-view text where the change starts.
    pub offset: usize,
    /// The text the change covers: inserted text, deleted text, or the formatted text.
    pub text: String,
    /// Revision-bearing elements this revision covers, in walk order.
    pub(crate) atoms: Vec<usize>,
}

/// A paragraph of the final view (see [`Document::final_view_paragraphs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalParagraph {
    pub first: usize,
    pub count: usize,
    pub text: String,
}

/// Which text of a paragraph to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextView {
    /// Insertions shown, deletions hidden: what [`Paragraph::text`] returns and what every
    /// character offset in the editing API counts.
    Final,
    /// Insertions and deletions both shown, in document order.
    Markup,
}

/// How a span of text is marked up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanMark {
    pub kind: RevisionKind,
    pub author: String,
    pub date: Option<String>,
}

/// A stretch of paragraph text with one markup state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan {
    pub text: String,
    /// The innermost insertion, deletion or move that contains the text.
    pub mark: Option<SpanMark>,
    /// The text carries a tracked formatting change (`w:rPrChange`).
    pub format_changed: bool,
}

/// Settings for recording edits as tracked changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackChanges {
    pub author: String,
    /// `w:date` value for new changes; `None` stamps each change with the current UTC time.
    pub date: Option<String>,
}

impl TrackChanges {
    /// Records changes by `author`, stamped with the current time.
    pub fn new(author: &str) -> Self {
        Self {
            author: author.to_owned(),
            date: None,
        }
    }

    /// Records changes by `author` with a fixed date (`2026-01-02T03:04:00Z`).
    pub fn with_date(author: &str, date: &str) -> Self {
        Self {
            author: author.to_owned(),
            date: Some(date.to_owned()),
        }
    }

    pub(crate) fn stamp(&self) -> String {
        self.date.clone().unwrap_or_else(now_utc)
    }
}

/// The current UTC time as `YYYY-MM-DDTHH:MM:00Z` (Word stores minutes).
pub(crate) fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:00Z",
        rem / 3_600,
        rem % 3_600 / 60
    )
}

// ---------------------------------------------------------------------------
// Small XML helpers
// ---------------------------------------------------------------------------

pub(crate) fn child_el<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.children.iter().find_map(|c| match c {
        RawXml::Element(c) if c.head.is_w(local) => Some(c),
        _ => None,
    })
}

fn child_el_mut<'a>(e: &'a mut XmlElement, local: &str) -> Option<&'a mut XmlElement> {
    e.children.iter_mut().find_map(|c| match c {
        RawXml::Element(c) if c.head.is_w(local) => Some(c),
        _ => None,
    })
}

fn remove_children(e: &mut XmlElement, local: &str) -> bool {
    let before = e.children.len();
    e.children
        .retain(|c| !matches!(c, RawXml::Element(c) if c.head.is_w(local)));
    before != e.children.len()
}

fn revision_attrs(e: &ElementHead) -> (String, Option<String>, Option<String>) {
    (
        e.attr_w("author").unwrap_or_default().to_owned(),
        e.attr_w("date").map(str::to_owned),
        e.attr_w("id").map(str::to_owned),
    )
}

/// Head of a new revision element `local` (`w:ins`, `w:del`...) next to `owner`, carrying
/// `w:id`, `w:author` and `w:date`.
pub(crate) fn new_revision_head(
    owner: &ElementHead,
    local: &str,
    id: u64,
    track: &TrackChanges,
) -> ElementHead {
    let mut head = owner.sibling(local);
    head.set_attr_w("id", &id.to_string());
    head.set_attr_w("author", &track.author);
    head.set_attr_w("date", &track.stamp());
    head
}

/// Which revision group an inline is, when it is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupRev {
    Ins,
    Del,
    MoveFrom,
    MoveTo,
}

impl GroupRev {
    fn of(g: &Group) -> Option<Self> {
        if g.head.is_w("ins") {
            Some(Self::Ins)
        } else if g.head.is_w("del") {
            Some(Self::Del)
        } else if g.head.is_w("moveFrom") {
            Some(Self::MoveFrom)
        } else if g.head.is_w("moveTo") {
            Some(Self::MoveTo)
        } else {
            None
        }
    }

    fn kind(self) -> RevisionKind {
        match self {
            Self::Ins => RevisionKind::Insertion,
            Self::Del => RevisionKind::Deletion,
            Self::MoveFrom => RevisionKind::MovedFrom,
            Self::MoveTo => RevisionKind::MovedTo,
        }
    }

    fn is_deletion(self) -> bool {
        matches!(self, Self::Del | Self::MoveFrom)
    }
}

fn element_text(e: &XmlElement) -> String {
    e.children
        .iter()
        .filter_map(|c| match c {
            RawXml::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

/// Text of a run including `w:delText` (the markup view).
pub(crate) fn run_markup_text(run: &Run, out: &mut String) {
    for item in &run.content {
        match item {
            RunContent::Text { text, .. } => out.push_str(text),
            RunContent::Tab => out.push('\t'),
            RunContent::Break(_) => out.push('\n'),
            // `w:t`, `w:tab` and `w:br` are raw only inside a `w:moveFrom`.
            RunContent::Raw(RawXml::Element(e)) if e.head.is_w("delText") || e.head.is_w("t") => {
                out.push_str(&element_text(e));
            }
            RunContent::Raw(RawXml::Element(e)) if e.head.is_w("tab") => out.push('\t'),
            RunContent::Raw(RawXml::Element(e)) if e.head.is_w("br") => out.push('\n'),
            RunContent::Raw(_) => {}
        }
    }
}

fn markup_text(content: &[Inline]) -> String {
    let mut out = String::new();
    for inline in content {
        match inline {
            Inline::Run(r) => run_markup_text(r, &mut out),
            Inline::Group(g) => out.push_str(&markup_text(&g.content)),
            Inline::Raw(_) => {}
        }
    }
    out
}

impl Paragraph {
    /// Text with insertions and deletions both shown (the markup view); deleted text is
    /// included at its original position. [`Paragraph::text`] is the final view.
    pub fn markup_text(&self) -> String {
        markup_text(&self.content)
    }

    /// Text in `view`.
    pub fn text_in_view(&self, view: TextView) -> String {
        match view {
            TextView::Final => self.text(),
            TextView::Markup => self.markup_text(),
        }
    }

    /// The paragraph text split into spans of equal markup state. The final view leaves out
    /// deleted and moved-from text; the markup view keeps it, tagged.
    pub fn spans(&self, view: TextView) -> Vec<TextSpan> {
        fn walk(
            content: &[Inline],
            view: TextView,
            mark: &Option<SpanMark>,
            out: &mut Vec<TextSpan>,
        ) {
            for inline in content {
                match inline {
                    Inline::Run(r) => {
                        let mut text = String::new();
                        let hidden = mark.as_ref().is_some_and(|m| m.kind.is_deletion_like());
                        if view == TextView::Markup || !hidden {
                            if view == TextView::Markup {
                                run_markup_text(r, &mut text);
                            } else {
                                text = r.text();
                            }
                        }
                        if text.is_empty() {
                            continue;
                        }
                        let format_changed = r.props.any_w_element("rPrChange").is_some();
                        match out.last_mut() {
                            Some(last)
                                if last.mark == *mark && last.format_changed == format_changed =>
                            {
                                last.text.push_str(&text);
                            }
                            _ => out.push(TextSpan {
                                text,
                                mark: mark.clone(),
                                format_changed,
                            }),
                        }
                    }
                    Inline::Group(g) => match GroupRev::of(g) {
                        Some(rev) => {
                            let inner = Some(SpanMark {
                                kind: rev.kind(),
                                author: g.head.attr_w("author").unwrap_or_default().to_owned(),
                                date: g.head.attr_w("date").map(str::to_owned),
                            });
                            // A deletion inside an insertion is a deletion; an insertion
                            // inside a deletion stays deleted.
                            let effective = match mark {
                                Some(m) if m.kind.is_deletion_like() => mark.clone(),
                                _ => inner,
                            };
                            walk(&g.content, view, &effective, out);
                        }
                        None => walk(&g.content, view, mark, out),
                    },
                    Inline::Raw(_) => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.content, view, &None, &mut out);
        out
    }

    /// The revision on the paragraph mark, when it is inserted or deleted. A mark that is
    /// both (inserted by one author, deleted by another) reports the deletion.
    pub fn mark_revision(&self) -> Option<RevisionKind> {
        let rpr = self.props.as_ref().and_then(|p| child_el(p, "rPr"))?;
        if child_el(rpr, "del").is_some() || child_el(rpr, "moveFrom").is_some() {
            Some(RevisionKind::ParagraphMarkDeleted)
        } else if child_el(rpr, "ins").is_some() || child_el(rpr, "moveTo").is_some() {
            Some(RevisionKind::ParagraphMarkInserted)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// Revisions of one family by one author over contiguous content are a single revision, as
/// Word counts them (`Revisions.Count`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Ins,
    Del,
    MoveFrom,
    MoveTo,
    RunFmt,
    ParaFmt,
    TblFmt,
    SectFmt,
    RowIns,
    RowDel,
    CellIns,
    CellDel,
    CellMerge,
}

impl Family {
    fn merges(self) -> bool {
        !matches!(
            self,
            Self::TblFmt | Self::SectFmt | Self::CellIns | Self::CellDel | Self::CellMerge
        )
    }
}

/// A cell-level revision element of `w:tcPr`, in schema order.
const CELL_MARKS: [(&str, Family, RevisionKind); 3] = [
    ("cellIns", Family::CellIns, RevisionKind::CellInserted),
    ("cellDel", Family::CellDel, RevisionKind::CellDeleted),
    ("cellMerge", Family::CellMerge, RevisionKind::CellMerged),
];

/// The `w:tcPr` block of a cell.
fn cell_tc_pr(cell: &Cell) -> Option<&XmlElement> {
    cell.blocks.iter().find_map(|b| match b {
        Block::Raw(RawXml::Element(e)) if e.head.is_w("tcPr") => Some(e),
        _ => None,
    })
}

fn cell_tc_pr_mut(cell: &mut Cell) -> Option<&mut XmlElement> {
    cell.blocks.iter_mut().find_map(|b| match b {
        Block::Raw(RawXml::Element(e)) if e.head.is_w("tcPr") => Some(e),
        _ => None,
    })
}

/// Paragraphs of `blocks`, counted as the lister counts them (nested tables included).
fn count_paragraphs(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(_) => 1,
            Block::Table(t) => t
                .rows()
                .flat_map(Row::cells)
                .map(|c| count_paragraphs(&c.blocks))
                .sum(),
            Block::Sdt(s) => s.content().map(|c| count_paragraphs(&c.blocks)).sum(),
            Block::Raw(_) => 0,
        })
        .sum()
}

/// Paragraph text of a cell, joined by newlines.
fn cell_text(cell: &Cell) -> String {
    cell.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.markup_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl GroupRev {
    fn family(self) -> Family {
        match self {
            Self::Ins => Family::Ins,
            Self::Del => Family::Del,
            Self::MoveFrom => Family::MoveFrom,
            Self::MoveTo => Family::MoveTo,
        }
    }
}

/// Paragraphs a Word range must touch to resolve an atom, and the row it is resolved with.
type AtomRange = ((usize, usize), Option<(usize, usize)>);

/// Where a `w:moveFromRangeStart` / `w:moveToRangeStart` range starts and ends.
struct MoveRange {
    name: String,
    to: bool,
    id: Option<String>,
    start: usize,
    end: Option<usize>,
}

/// Walks the story in the order [`Pass`] does and numbers every revision-bearing element
/// (an "atom"); the atoms of one Word revision are collected into a [`Revision`].
struct Lister {
    out: Vec<Revision>,
    para: usize,
    atom: usize,
    /// The open revision that the next atom may extend: family, author, index in `out`.
    prev: Option<(Family, String, usize)>,
    /// Inside an inserted or deleted row: atoms of this family belong to the row's revision.
    absorb: Option<(Family, usize)>,
    /// First paragraph of the section being walked.
    sect_start: usize,
    /// Paragraphs a Word range must touch to resolve an atom (first, last), by atom number.
    atom_span: Vec<(usize, usize)>,
    /// Family of every atom, by atom number (`None` for hidden atoms).
    atom_family: Vec<Option<Family>>,
    /// Paragraphs of the table row an atom belongs to, for the atoms Word resolves per row:
    /// resolving one through a range resolves every revision in the row.
    atom_row: Vec<Option<(usize, usize)>>,
    /// Paragraphs of the row being walked.
    row_span: Option<(usize, usize)>,
    /// Name of the last move range opened in the paragraph being walked.
    cur_move: Option<String>,
    /// Index in `move_ranges` of the move range opened last in the paragraph being walked.
    cur_range: Option<usize>,
    /// Every move range marker seen so far, as positions in a running marker count.
    move_ranges: Vec<MoveRange>,
    seq: usize,
    /// The table-format revision of the table being walked: every table, grid, row, row
    /// exception and cell property change of one table is a single revision in Word.
    table_rev: Option<usize>,
}

impl Lister {
    fn atom(
        &mut self,
        family: Family,
        kind: RevisionKind,
        head: &ElementHead,
        offset: usize,
        text: &str,
    ) {
        let id = self.atom;
        self.atom += 1;
        self.atom_span.push((self.para, self.para));
        self.atom_family.push(Some(family));
        self.atom_row.push(None);
        if family == Family::TblFmt
            && let Some(idx) = self.table_rev
        {
            self.out[idx].atoms.push(id);
            return;
        }
        if let Some((fam, idx)) = self.absorb
            && fam == family
        {
            self.out[idx].atoms.push(id);
            self.out[idx].last_paragraph = self.para;
            return;
        }
        let (author, date, rid) = revision_attrs(head);
        if family.merges()
            && let Some((f, a, idx)) = &self.prev
            && *f == family
            && *a == author
        {
            let idx = *idx;
            self.out[idx].atoms.push(id);
            if self.out[idx].last_paragraph != self.para && !text.is_empty() {
                self.out[idx].text.push(char::from(10));
            }
            self.out[idx].text.push_str(text);
            self.out[idx].last_paragraph = self.para;
            return;
        }
        let index = self.out.len();
        self.out.push(Revision {
            index,
            kind,
            author: author.clone(),
            date,
            id: rid,
            paragraph: self.para,
            last_paragraph: self.para,
            move_name: matches!(family, Family::MoveFrom | Family::MoveTo)
                .then(|| self.cur_move.clone())
                .flatten(),
            link_moves: Vec::new(),
            offset,
            text: text.to_owned(),
            atoms: vec![id],
        });
        if family == Family::TblFmt {
            self.table_rev = Some(index);
        }
        self.prev = Some((family, author, index));
    }

    /// Counts an element that Word resolves but does not list (the mark of a cell's last
    /// paragraph is the end-of-cell mark, not a paragraph mark).
    fn atom_hidden(&mut self) {
        self.atom += 1;
        self.atom_span.push((self.para, self.para));
        self.atom_family.push(None);
        self.atom_row.push(None);
    }

    /// Sets the paragraphs a Word range must touch to resolve the atom listed last. `row`:
    /// the atom belongs to the table row of that span.
    fn span_last(&mut self, span: (usize, usize), row: bool) {
        let id = self.atom - 1;
        self.atom_span[id] = span;
        self.atom_row[id] = row.then_some(span);
    }

    /// Reserves the number of a cell revision, listed after the cell's content.
    fn reserve_atom(&mut self, family: Family) -> usize {
        let id = self.atom;
        self.atom += 1;
        self.atom_span.push((self.para, self.para));
        self.atom_family.push(Some(family));
        self.atom_row.push(None);
        id
    }

    /// Lists the cell revision `id` once its cell content (atoms `inner`) has been walked.
    /// Word's range for it runs from the previous cell to the end of the row. Resolving
    /// it also resolves the cell's own insertions (`cellIns`) or deletions (`cellDel`).
    fn cell_revision(
        &mut self,
        id: usize,
        kind: RevisionKind,
        head: &ElementHead,
        span: (usize, usize),
        inner: std::ops::Range<usize>,
        text: String,
    ) {
        let own = match kind {
            RevisionKind::CellInserted => Some(Family::Ins),
            RevisionKind::CellDeleted => Some(Family::Del),
            _ => None,
        };
        let mut atoms = vec![id];
        atoms.extend(inner.filter(|&a| own.is_some() && self.atom_family[a] == own));
        self.atom_span[id] = span;
        let (author, date, rid) = revision_attrs(head);
        // Word lists vertically stacked merges by one author as one revision.
        if kind == RevisionKind::CellMerged
            && let Some(prev) = self.out.last_mut()
            && prev.kind == RevisionKind::CellMerged
            && prev.author == author
        {
            prev.atoms.extend(atoms);
            prev.last_paragraph = prev.last_paragraph.max(span.1);
            prev.text.push_str(&text);
            self.prev = None;
            return;
        }
        let index = self.out.len();
        self.out.push(Revision {
            index,
            kind,
            author,
            date,
            id: rid,
            paragraph: span.0,
            last_paragraph: span.1,
            move_name: None,
            link_moves: Vec::new(),
            offset: 0,
            text,
            atoms,
        });
        self.prev = None;
    }

    /// A move range marker (`w:moveFromRangeStart`, `w:moveToRangeEnd`...), inline or between
    /// paragraphs.
    fn move_marker(&mut self, e: &XmlElement) {
        let local = e.head.local_name();
        let to = local.starts_with("moveTo");
        self.seq += 1;
        if local.ends_with("RangeStart") {
            // A new move range starts a new revision, as in Word.
            self.gap();
            let name = e.head.attr_w("name").unwrap_or_default().to_owned();
            self.cur_move = Some(name.clone());
            self.cur_range = Some(self.move_ranges.len());
            self.move_ranges.push(MoveRange {
                name,
                to,
                id: e.head.attr_w("id").map(str::to_owned),
                start: self.seq,
                end: None,
            });
        } else if local.ends_with("RangeEnd") {
            let id = e.head.attr_w("id");
            if let Some(r) = self
                .move_ranges
                .iter_mut()
                .rev()
                .find(|r| r.to == to && r.id.as_deref() == id && r.end.is_none())
            {
                r.end = Some(self.seq);
            }
        }
    }

    /// Fills `link_moves` of the move revisions.
    fn link_moves(&mut self) {
        for r in &mut self.out {
            let Some(name) = &r.move_name else { continue };
            let to =
                r.kind == RevisionKind::MovedTo || r.kind == RevisionKind::ParagraphMarkInserted;
            let Some(range) = self
                .move_ranges
                .iter()
                .find(|m| m.name == *name && m.to == to)
            else {
                continue;
            };
            let end = range.end.unwrap_or(range.start);
            r.link_moves = self
                .move_ranges
                .iter()
                .filter(|m| m.name != *name && m.start > range.start && m.start < end)
                .map(|m| m.name.clone())
                .collect();
        }
    }

    /// Plain text follows: the open revision cannot continue.
    fn gap(&mut self) {
        self.prev = None;
    }

    /// Sets the start paragraph of the section revision just added.
    fn set_section_start(&mut self) {
        let start = self.sect_start;
        if let Some(last) = self.out.last_mut() {
            last.paragraph = start;
        }
    }
}

fn list_inlines(content: &[Inline], l: &mut Lister, acc: &mut usize, covered: bool) {
    for inline in content {
        match inline {
            Inline::Run(r) => {
                if let Some(change) = r.props.any_w_element("rPrChange") {
                    let mut text = String::new();
                    run_markup_text(r, &mut text);
                    l.atom(
                        Family::RunFmt,
                        RevisionKind::RunFormat,
                        &change.head,
                        *acc,
                        &text,
                    );
                } else if !covered && r.char_len() > 0 {
                    l.gap();
                }
                *acc += r.char_len();
            }
            Inline::Group(g) => {
                let rev = GroupRev::of(g);
                if let Some(rev) = rev {
                    l.atom(
                        rev.family(),
                        rev.kind(),
                        &g.head,
                        *acc,
                        &markup_text(&g.content),
                    );
                }
                list_inlines(&g.content, l, acc, covered || rev.is_some());
            }
            Inline::Raw(RawXml::Element(e))
                if matches!(
                    e.head.local_name(),
                    "moveFromRangeStart"
                        | "moveToRangeStart"
                        | "moveFromRangeEnd"
                        | "moveToRangeEnd"
                ) =>
            {
                l.move_marker(e);
            }
            Inline::Raw(_) => {}
        }
    }
}

/// `w:numPr` of a paragraph carries `w:numberingChange` or `w:ins`. Word lists no revision for
/// these (measured: `rev-numbering-xml`) and drops both from the paragraph when all changes
/// are accepted or rejected, so they are hidden atoms.
fn has_numbering_marks(p: &Paragraph) -> bool {
    p.props
        .as_ref()
        .and_then(|ppr| child_el(ppr, "numPr"))
        .is_some_and(|n| child_el(n, "numberingChange").is_some() || child_el(n, "ins").is_some())
}

fn list_paragraph(p: &Paragraph, l: &mut Lister, cell_end: bool) {
    l.cur_move = None;
    l.cur_range = None;
    let mut acc = 0;
    let text = p.markup_text();
    let mut marks: Vec<Family> = Vec::new();
    let mut has_section = false;
    // Word sorts a paragraph's property change at the start of the paragraph.
    if let Some(change) = p.props.as_ref().and_then(|ppr| child_el(ppr, "pPrChange")) {
        l.atom(
            Family::ParaFmt,
            RevisionKind::ParagraphFormat,
            &change.head,
            0,
            &text,
        );
        marks.push(Family::ParaFmt);
    }
    if has_numbering_marks(p) {
        l.atom_hidden();
    }
    list_inlines(&p.content, l, &mut acc, false);
    if let Some(ppr) = &p.props {
        if let Some(sect) = child_el(ppr, "sectPr") {
            has_section = true;
            if let Some(change) = child_el(sect, "sectPrChange") {
                l.atom(
                    Family::SectFmt,
                    RevisionKind::SectionFormat,
                    &change.head,
                    0,
                    "",
                );
                l.set_section_start();
            }
        }
        if let Some(rpr) = child_el(ppr, "rPr") {
            if let Some(change) = child_el(rpr, "rPrChange") {
                l.atom(
                    Family::RunFmt,
                    RevisionKind::RunFormat,
                    &change.head,
                    acc,
                    "",
                );
                marks.push(Family::RunFmt);
            }
            for (local, family, kind) in [
                ("ins", Family::Ins, RevisionKind::ParagraphMarkInserted),
                (
                    "moveTo",
                    Family::MoveTo,
                    RevisionKind::ParagraphMarkInserted,
                ),
                ("del", Family::Del, RevisionKind::ParagraphMarkDeleted),
                (
                    "moveFrom",
                    Family::MoveFrom,
                    RevisionKind::ParagraphMarkDeleted,
                ),
            ] {
                if let Some(mark) = child_el(rpr, local) {
                    if cell_end {
                        l.atom_hidden();
                    } else {
                        l.atom(family, kind, &mark.head, acc, "");
                        marks.push(family);
                    }
                }
            }
        }
    }
    if has_section {
        l.sect_start = l.para + 1;
    }
    // A paragraph mark outside the open revision ends it (unless a row absorbs everything).
    if l.absorb.is_none() && l.prev.as_ref().is_some_and(|(f, _, _)| !marks.contains(f)) {
        l.gap();
    }
}

fn list_table(t: &Table, l: &mut Lister) {
    let outer_rev = l.table_rev.take();
    let outer_row = l.row_span.take();
    let first = l.para;
    let total: usize = t
        .rows()
        .flat_map(Row::cells)
        .map(|c| count_paragraphs(&c.blocks))
        .sum();
    let table_span = (first, first + total.saturating_sub(1));
    for child in &t.children {
        match child {
            TableChild::Raw(RawXml::Element(e))
                if e.head.is_w("tblPr") || e.head.is_w("tblGrid") =>
            {
                let local = if e.head.is_w("tblPr") {
                    "tblPrChange"
                } else {
                    "tblGridChange"
                };
                if let Some(change) = child_el(e, local) {
                    l.atom(
                        Family::TblFmt,
                        RevisionKind::TableFormat,
                        &change.head,
                        0,
                        "",
                    );
                    l.span_last(table_span, false);
                }
            }
            TableChild::Raw(_) => {}
            TableChild::Row(row) => {
                let mut absorbing = false;
                let row_first = l.para;
                let row_total: usize = row.cells().map(|c| count_paragraphs(&c.blocks)).sum();
                let span = (row_first, row_first + row_total.saturating_sub(1));
                l.row_span = Some(span);
                for rc in &row.children {
                    if let RowChild::Raw(RawXml::Element(e)) = rc
                        && e.head.is_w("tblPrEx")
                        && let Some(change) = child_el(e, "tblPrExChange")
                    {
                        l.atom(
                            Family::TblFmt,
                            RevisionKind::TableFormat,
                            &change.head,
                            0,
                            "",
                        );
                        l.span_last(span, true);
                    }
                }
                if let Some(trpr) = row_props(row) {
                    if let Some(change) = child_el(trpr, "trPrChange") {
                        l.atom(
                            Family::TblFmt,
                            RevisionKind::TableFormat,
                            &change.head,
                            0,
                            "",
                        );
                        l.span_last(span, true);
                    }
                    for (local, family, kind) in [
                        ("ins", Family::RowIns, RevisionKind::RowInserted),
                        ("del", Family::RowDel, RevisionKind::RowDeleted),
                    ] {
                        if let Some(mark) = child_el(trpr, local) {
                            l.atom(family, kind, &mark.head, 0, &row_text(row));
                            l.span_last(span, true);
                            if let Some((_, _, idx)) = l.prev {
                                let absorbed = if family == Family::RowIns {
                                    Family::Ins
                                } else {
                                    Family::Del
                                };
                                l.absorb = Some((absorbed, idx));
                                absorbing = true;
                            }
                        }
                    }
                }
                list_cells(row, l);
                l.row_span = None;
                if absorbing {
                    l.absorb = None;
                }
            }
        }
    }
    if let Some(idx) = l.table_rev {
        l.out[idx].last_paragraph = table_span.1;
    }
    l.table_rev = outer_rev;
    l.row_span = outer_row;
}

/// Walks the cells of a row and lists their cell revisions after the cell content.
fn list_cells(row: &Row, l: &mut Lister) {
    let cells: Vec<&Cell> = row.cells().collect();
    let mut starts = Vec::with_capacity(cells.len());
    let mut counts = Vec::with_capacity(cells.len());
    let mut at = l.para;
    for c in &cells {
        let n = count_paragraphs(&c.blocks);
        starts.push(at);
        counts.push(n);
        at += n;
    }
    // Word resolves the first cell's insertion or deletion of a row whose every cell carries the
    // same one together with the whole row (measured: accepting the first of three cell
    // deletions removes the row, accepting the second removes only its own cell).
    let row_kind = {
        let mut kinds = cells.iter().map(|c| {
            let pr = cell_tc_pr(c)?;
            let mut it = CELL_MARKS
                .iter()
                .filter(|(local, _, _)| child_el(pr, local).is_some())
                .map(|(_, _, kind)| *kind);
            let kind = it.next()?;
            it.next().is_none().then_some(kind)
        });
        let first = kinds.next().flatten();
        first.filter(|k| {
            matches!(k, RevisionKind::CellInserted | RevisionKind::CellDeleted)
                && cells.len() > 1
                && kinds.all(|o| o == Some(*k))
        })
    };
    let mut rest_start = None;
    for (k, cell) in cells.iter().enumerate() {
        if k == 1 {
            rest_start = Some(l.out.len());
        }
        let marks: Vec<(usize, RevisionKind, &ElementHead)> = cell_tc_pr(cell)
            .map(|pr| {
                CELL_MARKS
                    .iter()
                    .filter_map(|(local, family, kind)| {
                        child_el(pr, local).map(|m| (l.reserve_atom(*family), *kind, &m.head))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let first = l.atom;
        list_blocks(&cell.blocks, l, true);
        let last = l.atom;
        let from = starts[0];
        // The range covers the whole row (measured: any paragraph of the row reaches it).
        let to = at.saturating_sub(1).max(starts[k]);
        for (id, kind, head) in marks {
            l.cell_revision(id, kind, head, (from, to), first..last, cell_text(cell));
        }
    }
    if let (Some(kind), Some(rest)) = (row_kind, rest_start)
        && let Some(head) = l.out[..rest].iter().rposition(|r| r.kind == kind)
    {
        let extra: Vec<usize> = l.out[rest..]
            .iter()
            .filter(|r| r.kind == kind)
            .flat_map(|r| r.atoms.iter().copied())
            .collect();
        l.out[head].atoms.extend(extra);
    }
}

/// Cell paragraphs of a row, joined by newlines.
fn row_text(row: &Row) -> String {
    row.cells()
        .flat_map(|c| c.blocks.iter())
        .filter_map(|b| match b {
            Block::Paragraph(p) => Some(p.markup_text()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn row_props(row: &Row) -> Option<&XmlElement> {
    row.children.iter().find_map(|c| match c {
        RowChild::Raw(RawXml::Element(e)) if e.head.is_w("trPr") => Some(e),
        _ => None,
    })
}

fn list_blocks(blocks: &[Block], l: &mut Lister, in_cell: bool) {
    for (bi, block) in blocks.iter().enumerate() {
        match block {
            Block::Paragraph(p) => {
                list_paragraph(p, l, in_cell && bi + 1 == blocks.len());
                l.para += 1;
            }
            Block::Table(t) => list_table(t, l),
            Block::Sdt(sdt) => {
                for c in sdt.content() {
                    list_blocks(&c.blocks, l, false);
                }
            }
            Block::Raw(RawXml::Element(e)) if e.head.is_w("tcPr") => {
                if let Some(change) = child_el(e, "tcPrChange") {
                    l.atom(
                        Family::TblFmt,
                        RevisionKind::TableFormat,
                        &change.head,
                        0,
                        "",
                    );
                    if let Some(span) = l.row_span {
                        l.span_last(span, true);
                    }
                }
            }
            Block::Raw(RawXml::Element(e))
                if matches!(
                    e.head.local_name(),
                    "moveFromRangeEnd"
                        | "moveToRangeEnd"
                        | "moveFromRangeStart"
                        | "moveToRangeStart"
                ) =>
            {
                l.move_marker(e);
            }
            Block::Raw(RawXml::Element(e)) if e.head.is_w("sectPr") => {
                if let Some(change) = child_el(e, "sectPrChange") {
                    l.atom(
                        Family::SectFmt,
                        RevisionKind::SectionFormat,
                        &change.head,
                        0,
                        "",
                    );
                    l.set_section_start();
                }
            }
            Block::Raw(_) => {}
        }
    }
}

/// Word lists a section change where its section starts; move ours there.
fn place_section_revisions(out: &mut Vec<Revision>) {
    let sections: Vec<Revision> = out
        .iter()
        .filter(|r| r.kind == RevisionKind::SectionFormat)
        .cloned()
        .collect();
    out.retain(|r| r.kind != RevisionKind::SectionFormat);
    for s in sections {
        let at = out
            .iter()
            .position(|r| r.paragraph >= s.paragraph)
            .unwrap_or(out.len());
        out.insert(at, s);
    }
    for (i, r) in out.iter_mut().enumerate() {
        r.index = i;
    }
}

// ---------------------------------------------------------------------------
// Accept / reject
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Accept,
    Reject,
}

/// Resolves the atoms in `selected` (every atom when `None`). Atoms are numbered in the order
/// [`Lister`] numbers them, so every element is counted whether or not its parent goes away.
struct Pass {
    action: Action,
    selected: Option<HashSet<usize>>,
    counter: usize,
    touched: bool,
}

impl Pass {
    /// Counts the next atom; true when it is to be resolved.
    fn hit(&mut self) -> bool {
        let i = self.counter;
        self.counter += 1;
        let hit = self.selected.as_ref().is_none_or(|s| s.contains(&i));
        self.touched |= hit;
        hit
    }
}

/// Resolves the property-change element `change` inside `props`. Accepting drops it;
/// rejecting puts the old properties it holds back, keeping the children named in `front`
/// (before) and `back` (after) from the current properties.
fn resolve_prop_change(
    props: &mut XmlElement,
    change: &str,
    front: &[&str],
    back: &[&str],
    action: Action,
) {
    let Some(pos) = props
        .children
        .iter()
        .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w(change)))
    else {
        return;
    };
    let RawXml::Element(change_el) = props.children.remove(pos) else {
        return;
    };
    if action == Action::Accept {
        return;
    }
    let old: Vec<RawXml> = change_el
        .children
        .into_iter()
        .find_map(|c| match c {
            RawXml::Element(e) => Some(e.children),
            _ => None,
        })
        .unwrap_or_default();
    let current = std::mem::take(&mut props.children);
    let keep = |c: &RawXml, names: &[&str]| matches!(c, RawXml::Element(e) if names.iter().any(|n| e.head.is_w(n)));
    let mut rebuilt: Vec<RawXml> = current.iter().filter(|c| keep(c, front)).cloned().collect();
    rebuilt.extend(old);
    rebuilt.extend(current.iter().filter(|c| keep(c, back)).cloned());
    props.children = rebuilt;
}

/// `w:delText` back to `w:t`, `w:delInstrText` to `w:instrText`, in every run of `content`.
fn restore_deleted_text(content: &mut [Inline]) {
    for inline in content {
        match inline {
            Inline::Run(r) => {
                for item in &mut r.content {
                    let RunContent::Raw(RawXml::Element(e)) = item else {
                        continue;
                    };
                    if e.head.is_w("tab") && e.children.is_empty() && e.head.attrs.is_empty() {
                        *item = RunContent::Tab;
                    } else if e.head.is_w("br") && e.children.is_empty() {
                        *item = RunContent::Break(Break {
                            attrs: std::mem::take(&mut e.head.attrs),
                        });
                    } else if e.head.is_w("delText") || e.head.is_w("t") {
                        let text = element_text(e);
                        let explicit = e.head.attrs.iter().any(|a| a.is_xml_space());
                        let attrs = e
                            .head
                            .attrs
                            .iter()
                            .filter(|a| !a.is_xml_space())
                            .cloned()
                            .collect();
                        *item = RunContent::Text {
                            preserve_space: explicit || needs_preserve(&text),
                            text,
                            attrs,
                        };
                    } else if e.head.is_w("delInstrText") {
                        e.head.name = sibling_name(&e.head.name, "instrText");
                    }
                }
            }
            Inline::Group(g) => restore_deleted_text(&mut g.content),
            Inline::Raw(_) => {}
        }
    }
}

/// `w:t` to `w:delText`, `w:instrText` to `w:delInstrText`, in every run of `content`.
pub(crate) fn mark_text_deleted(run: &mut Run) {
    let owner = run.head.clone();
    for item in &mut run.content {
        match item {
            RunContent::Text {
                text,
                preserve_space,
                attrs,
            } => {
                let mut head = owner.sibling("delText");
                head.attrs.clone_from(attrs);
                if *preserve_space || needs_preserve(text) {
                    head.attrs.push(Attr {
                        name: "xml:space".to_owned(),
                        ns: Some("http://www.w3.org/XML/1998/namespace".into()),
                        value: "preserve".to_owned(),
                    });
                }
                let element = XmlElement {
                    head,
                    children: if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![RawXml::Text(std::mem::take(text))]
                    },
                };
                *item = RunContent::Raw(RawXml::Element(element));
            }
            RunContent::Raw(RawXml::Element(e)) if e.head.is_w("instrText") => {
                e.head.name = sibling_name(&e.head.name, "delInstrText");
            }
            _ => {}
        }
    }
}

fn pass_inlines(content: &mut Vec<Inline>, pass: &mut Pass) {
    let old = std::mem::take(content);
    for inline in old {
        match inline {
            Inline::Run(mut r) => {
                if r.props.any_w_element("rPrChange").is_some() && pass.hit() {
                    match (pass.action, r.props.take_rpr_change()) {
                        (Action::Reject, Some(change)) => {
                            let old_props = change.children.into_iter().find_map(|c| match c {
                                RawXml::Element(e) if e.head.is_w("rPr") => Some(e),
                                _ => None,
                            });
                            r.props = match old_props {
                                Some(e) if !e.children.is_empty() => RunProps::from_element(e),
                                _ => RunProps::default(),
                            };
                        }
                        _ => {
                            if !r.props.has_items() {
                                r.props = RunProps::default();
                            }
                        }
                    }
                }
                content.push(Inline::Run(r));
            }
            Inline::Group(mut g) => {
                let rev = GroupRev::of(&g);
                let hit = rev.is_some() && pass.hit();
                let had = !g.content.is_empty();
                pass_inlines(&mut g.content, pass);
                match rev {
                    Some(rev) if hit => {
                        let remove = matches!(
                            (rev.is_deletion(), pass.action),
                            (false, Action::Reject) | (true, Action::Accept)
                        );
                        if remove {
                            continue;
                        }
                        if rev.is_deletion() {
                            restore_deleted_text(&mut g.content);
                        }
                        content.append(&mut g.content);
                    }
                    _ => {
                        if !(had && g.content.is_empty()) {
                            content.push(Inline::Group(g));
                        }
                    }
                }
            }
            raw @ Inline::Raw(_) => content.push(raw),
        }
    }
}

/// What to do with a paragraph after its own revisions are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Stays,
    /// The paragraph mark is gone: join with the next paragraph.
    JoinNext,
}

fn pass_paragraph(p: &mut Paragraph, pass: &mut Pass) -> Mark {
    let numbering_marks = has_numbering_marks(p);
    // Same order as the lister: the property change first, then the content.
    if let Some(ppr) = &mut p.props
        && child_el(ppr, "pPrChange").is_some()
        && pass.hit()
    {
        resolve_prop_change(ppr, "pPrChange", &[], &["rPr", "sectPr"], pass.action);
    }
    if numbering_marks
        && pass.hit()
        && let Some(num) = p.props.as_mut().and_then(|ppr| child_el_mut(ppr, "numPr"))
    {
        remove_children(num, "numberingChange");
        remove_children(num, "ins");
    }
    pass_inlines(&mut p.content, pass);
    let mut mark = Mark::Stays;
    let Some(ppr) = &mut p.props else {
        return mark;
    };
    if let Some(sect) = child_el_mut(ppr, "sectPr")
        && child_el(sect, "sectPrChange").is_some()
        && pass.hit()
    {
        resolve_prop_change(
            sect,
            "sectPrChange",
            &["headerReference", "footerReference"],
            &[],
            pass.action,
        );
    }
    if let Some(rpr) = child_el_mut(ppr, "rPr") {
        if child_el(rpr, "rPrChange").is_some() && pass.hit() {
            resolve_prop_change(
                rpr,
                "rPrChange",
                &["ins", "del", "moveFrom", "moveTo"],
                &[],
                pass.action,
            );
        }
        for (local, inserted) in [
            ("ins", true),
            ("moveTo", true),
            ("del", false),
            ("moveFrom", false),
        ] {
            if child_el(rpr, local).is_some() && pass.hit() {
                remove_children(rpr, local);
                let gone = matches!(
                    (inserted, pass.action),
                    (true, Action::Reject) | (false, Action::Accept)
                );
                if gone {
                    mark = Mark::JoinNext;
                }
            }
        }
    }
    mark
}

/// Joins `blocks[i]` with the paragraph after it because the first one's mark is gone. The
/// surviving mark is the second paragraph's, so its properties win (as in Word), unless the
/// first paragraph ends a section.
fn join_with_next(blocks: &mut Vec<Block>, i: usize) {
    // Range markers (`w:moveFromRangeEnd`...) can sit between the two paragraphs; they stay.
    let mut j = i + 1;
    while matches!(blocks.get(j), Some(Block::Raw(_))) {
        j += 1;
    }
    if !matches!(blocks.get(j), Some(Block::Paragraph(_))) {
        return;
    }
    let Block::Paragraph(next) = blocks.remove(j) else {
        return;
    };
    let Some(Block::Paragraph(first)) = blocks.get_mut(i) else {
        return;
    };
    let first_has_section = first
        .props
        .as_ref()
        .is_some_and(|p| child_el(p, "sectPr").is_some());
    first.content.extend(next.content);
    if !first_has_section {
        first.props = next.props;
        first.head = next.head;
    }
}

/// Joins the paragraph at `blocks[i]` with the next one (used by tracked deletion when it
/// removes a paragraph mark inserted by the same author).
pub(crate) fn join_paragraphs(blocks: &mut Vec<Block>, i: usize) {
    join_with_next(blocks, i);
}

/// Sets the vertical merge state a `w:cellMerge` names (`cont`: merged into the cell above,
/// `rest`: not merged) in `w:tcPr`.
fn set_vertical_merge(pr: &mut XmlElement, state: &str) {
    let pos = pr
        .children
        .iter()
        .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w("vMerge")));
    match (state, pos) {
        ("cont", Some(i)) => {
            if let RawXml::Element(e) = &mut pr.children[i] {
                e.head.remove_attr_w("val");
            }
        }
        ("cont", None) => {
            let at = pr
                .children
                .iter()
                .rposition(|c| {
                    matches!(c, RawXml::Element(e)
                        if ["cnfStyle", "tcW", "gridSpan", "hMerge"].iter().any(|n| e.head.is_w(n)))
                })
                .map_or(0, |i| i + 1);
            let merge = pr.head.sibling("vMerge").into_empty();
            pr.children.insert(at, RawXml::Element(merge));
        }
        ("rest", Some(i)) => {
            let restart = matches!(&pr.children[i],
                RawXml::Element(e) if e.head.attr_w("val") == Some("restart"));
            if !restart {
                pr.children.remove(i);
            }
        }
        _ => {}
    }
}

/// Resolves the `w:cellIns`, `w:cellDel` and `w:cellMerge` of a cell; true when the cell goes.
fn pass_cell_marks(cell: &mut Cell, pass: &mut Pass) -> bool {
    let Some(pr) = cell_tc_pr_mut(cell) else {
        return false;
    };
    let mut remove = false;
    for (local, family, _) in CELL_MARKS {
        let Some(mark) = child_el(pr, local) else {
            continue;
        };
        let target = match pass.action {
            Action::Accept => mark.head.attr_w("vMerge"),
            Action::Reject => mark.head.attr_w("vMergeOrig"),
        }
        .map(str::to_owned);
        if !pass.hit() {
            continue;
        }
        remove_children(pr, local);
        match (family, pass.action) {
            (Family::CellIns, Action::Reject) | (Family::CellDel, Action::Accept) => remove = true,
            (Family::CellMerge, _) => {
                if let Some(state) = target {
                    set_vertical_merge(pr, &state);
                }
            }
            _ => {}
        }
    }
    remove
}

/// Grid span and width of a cell, for handing them to a neighbour when the cell goes.
fn cell_extent(cell: &Cell) -> (u32, Option<u32>) {
    let Some(pr) = cell_tc_pr(cell) else {
        return (1, None);
    };
    let span = child_el(pr, "gridSpan")
        .and_then(|e| e.head.attr_w("val"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let width = child_el(pr, "tcW")
        .filter(|e| e.head.attr_w("type").is_none_or(|t| t == "dxa"))
        .and_then(|e| e.head.attr_w("w"))
        .and_then(|v| v.parse().ok());
    (span, width)
}

/// Removes the cells at `drop` from `row`. The space of a removed cell goes to the cell
/// before it (Word widens that cell and spans it over the freed grid column), or to the next
/// cell when the first one goes.
fn remove_cells(row: &mut Row, drop: &[usize]) {
    for &k in drop.iter().rev() {
        let RowChild::Cell(gone) = row.children.remove(k) else {
            continue;
        };
        let (span, width) = cell_extent(&gone);
        let heir = row.children[..k]
            .iter()
            .rposition(|c| matches!(c, RowChild::Cell(_)))
            .or_else(|| {
                row.children[k..]
                    .iter()
                    .position(|c| matches!(c, RowChild::Cell(_)))
                    .map(|i| i + k)
            });
        let Some(RowChild::Cell(heir)) = heir.map(|i| &mut row.children[i]) else {
            continue;
        };
        let (heir_span, heir_width) = cell_extent(heir);
        let Some(pr) = cell_tc_pr_mut(heir) else {
            continue;
        };
        if let (Some(w), Some(hw)) = (width, heir_width)
            && let Some(RawXml::Element(tcw)) = pr
                .children
                .iter_mut()
                .find(|c| matches!(c, RawXml::Element(e) if e.head.is_w("tcW")))
        {
            tcw.head.set_attr_w("w", &(w + hw).to_string());
        }
        // A continuation cell that no longer matches the cell above in extent is not merged
        // any more (measured: Word shows it as a cell of its own).
        if let Some(i) = pr.children.iter().position(|c| {
            matches!(c, RawXml::Element(e) if e.head.is_w("vMerge")
                && e.head.attr_w("val") != Some("restart"))
        }) {
            pr.children.remove(i);
        }
        if span + heir_span > 1 {
            let total = (span + heir_span).to_string();
            if let Some(RawXml::Element(gs)) = pr
                .children
                .iter_mut()
                .find(|c| matches!(c, RawXml::Element(e) if e.head.is_w("gridSpan")))
            {
                gs.head.set_attr_w("val", &total);
            } else {
                let at = pr
                    .children
                    .iter()
                    .rposition(|c| {
                        matches!(c, RawXml::Element(e)
                            if e.head.is_w("cnfStyle") || e.head.is_w("tcW"))
                    })
                    .map_or(0, |i| i + 1);
                let mut gs = pr.head.sibling("gridSpan").into_empty();
                gs.head.set_attr_w("val", &total);
                pr.children.insert(at, RawXml::Element(gs));
            }
        }
    }
}

fn pass_table(t: &mut Table, pass: &mut Pass) -> bool {
    let mut drop_rows = Vec::new();
    for (ci, child) in t.children.iter_mut().enumerate() {
        match child {
            TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPr") => {
                if child_el(e, "tblPrChange").is_some() && pass.hit() {
                    resolve_prop_change(e, "tblPrChange", &[], &[], pass.action);
                }
            }
            TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblGrid") => {
                if child_el(e, "tblGridChange").is_some() && pass.hit() {
                    resolve_prop_change(e, "tblGridChange", &[], &[], pass.action);
                }
            }
            TableChild::Raw(_) => {}
            TableChild::Row(row) => {
                let mut remove_row = false;
                for rc in &mut row.children {
                    let RowChild::Raw(RawXml::Element(ex)) = rc else {
                        continue;
                    };
                    if ex.head.is_w("tblPrEx")
                        && child_el(ex, "tblPrExChange").is_some()
                        && pass.hit()
                    {
                        resolve_prop_change(ex, "tblPrExChange", &[], &[], pass.action);
                    }
                }
                for rc in &mut row.children {
                    let RowChild::Raw(RawXml::Element(trpr)) = rc else {
                        continue;
                    };
                    if !trpr.head.is_w("trPr") {
                        continue;
                    }
                    if child_el(trpr, "trPrChange").is_some() && pass.hit() {
                        resolve_prop_change(trpr, "trPrChange", &[], &["ins", "del"], pass.action);
                    }
                    for (local, inserted) in [("ins", true), ("del", false)] {
                        if child_el(trpr, local).is_some() && pass.hit() {
                            remove_children(trpr, local);
                            if matches!(
                                (inserted, pass.action),
                                (true, Action::Reject) | (false, Action::Accept)
                            ) {
                                remove_row = true;
                            }
                        }
                    }
                }
                // The cells are walked even when the row goes, to keep the numbering.
                let mut drop_cells = Vec::new();
                for (k, rc) in row.children.iter_mut().enumerate() {
                    if let RowChild::Cell(cell) = rc {
                        if pass_cell_marks(cell, pass) {
                            drop_cells.push(k);
                        }
                        pass_blocks(&mut cell.blocks, pass);
                    }
                }
                remove_cells(row, &drop_cells);
                if remove_row {
                    drop_rows.push(ci);
                }
            }
        }
    }
    for ci in drop_rows.into_iter().rev() {
        t.children.remove(ci);
    }
    t.rows().next().is_some()
}

fn pass_blocks(blocks: &mut Vec<Block>, pass: &mut Pass) {
    let mut joins = Vec::new();
    let mut empty_tables = Vec::new();
    for (i, block) in blocks.iter_mut().enumerate() {
        match block {
            Block::Paragraph(p) => {
                if pass_paragraph(p, pass) == Mark::JoinNext {
                    joins.push(i);
                }
            }
            Block::Table(t) => {
                if !pass_table(t, pass) {
                    empty_tables.push(i);
                }
            }
            Block::Sdt(sdt) => {
                for child in &mut sdt.children {
                    if let SdtChild::Content(c) = child {
                        pass_blocks(&mut c.blocks, pass);
                    }
                }
            }
            Block::Raw(RawXml::Element(e)) if e.head.is_w("tcPr") => {
                if child_el(e, "tcPrChange").is_some() && pass.hit() {
                    resolve_prop_change(
                        e,
                        "tcPrChange",
                        &[],
                        &["cellIns", "cellDel", "cellMerge"],
                        pass.action,
                    );
                }
            }
            Block::Raw(RawXml::Element(e)) if e.head.is_w("sectPr") => {
                if child_el(e, "sectPrChange").is_some() && pass.hit() {
                    resolve_prop_change(
                        e,
                        "sectPrChange",
                        &["headerReference", "footerReference"],
                        &[],
                        pass.action,
                    );
                }
            }
            Block::Raw(_) => {}
        }
    }
    for i in joins.into_iter().rev() {
        join_with_next(blocks, i);
    }
    for i in empty_tables.into_iter().rev() {
        blocks.remove(i);
    }
}

/// Removes move-range markers when no moved text is left.
fn drop_orphan_move_markers(blocks: &mut Vec<Block>) {
    fn has_move(content: &[Inline]) -> bool {
        content.iter().any(|i| match i {
            Inline::Group(g) => {
                g.head.is_w("moveFrom") || g.head.is_w("moveTo") || has_move(&g.content)
            }
            _ => false,
        })
    }
    fn any_move(blocks: &[Block]) -> bool {
        blocks.iter().any(|b| match b {
            Block::Paragraph(p) => {
                has_move(&p.content)
                    || p.props
                        .as_ref()
                        .and_then(|ppr| child_el(ppr, "rPr"))
                        .is_some_and(|r| {
                            child_el(r, "moveFrom").is_some() || child_el(r, "moveTo").is_some()
                        })
            }
            Block::Table(t) => t.rows().any(|r| r.cells().any(|c| any_move(&c.blocks))),
            Block::Sdt(s) => s.content().any(|c| any_move(&c.blocks)),
            Block::Raw(_) => false,
        })
    }
    fn is_move_marker(i: &Inline) -> bool {
        matches!(i, Inline::Raw(RawXml::Element(e))
        if matches!(
            e.head.local_name(),
            "moveFromRangeStart" | "moveFromRangeEnd" | "moveToRangeStart" | "moveToRangeEnd"
        ))
    }
    fn strip(content: &mut Vec<Inline>) {
        content.retain(|i| !is_move_marker(i));
        for i in content {
            if let Inline::Group(g) = i {
                strip(&mut g.content);
            }
        }
    }
    fn strip_blocks(blocks: &mut Vec<Block>) {
        blocks.retain(|b| {
            !matches!(b, Block::Raw(RawXml::Element(e)) if matches!(
                e.head.local_name(),
                "moveFromRangeStart" | "moveFromRangeEnd" | "moveToRangeStart" | "moveToRangeEnd"
            ))
        });
        for b in blocks {
            match b {
                Block::Paragraph(p) => strip(&mut p.content),
                Block::Table(t) => {
                    for child in &mut t.children {
                        if let TableChild::Row(r) = child {
                            for rc in &mut r.children {
                                if let RowChild::Cell(c) = rc {
                                    strip_blocks(&mut c.blocks);
                                }
                            }
                        }
                    }
                }
                Block::Sdt(s) => {
                    for child in &mut s.children {
                        if let SdtChild::Content(c) = child {
                            strip_blocks(&mut c.blocks);
                        }
                    }
                }
                Block::Raw(_) => {}
            }
        }
    }
    if !any_move(blocks) {
        strip_blocks(blocks);
    }
}

/// Names of the moves resolved together with `hit`: their own, and on accept the moves that
/// start inside their range (measured: Word accepts those too, rejecting leaves them).
fn move_names(hit: &[&Revision], action: Action) -> HashSet<String> {
    let mut names = HashSet::new();
    for r in hit {
        if let Some(n) = &r.move_name {
            names.insert(n.clone());
            if action == Action::Accept {
                names.extend(r.link_moves.iter().cloned());
            }
        }
    }
    names
}

impl Document {
    /// Every tracked change in the main story, in document order. Adjacent changes of one
    /// kind by one author count as one, as in Word.
    pub fn revisions(&self) -> Vec<Revision> {
        self.list_revisions().0
    }

    /// The revisions plus the paragraph of every revision-bearing element (atom).
    fn list_revisions(&self) -> (Vec<Revision>, Vec<AtomRange>) {
        let mut l = Lister {
            out: Vec::new(),
            para: 0,
            atom: 0,
            prev: None,
            absorb: None,
            sect_start: 0,
            atom_span: Vec::new(),
            atom_family: Vec::new(),
            atom_row: Vec::new(),
            row_span: None,
            cur_move: None,
            cur_range: None,
            move_ranges: Vec::new(),
            seq: 0,
            table_rev: None,
        };
        list_blocks(&self.body().blocks, &mut l, false);
        l.link_moves();
        place_section_revisions(&mut l.out);
        let rows = l.atom_row;
        (l.out, l.atom_span.into_iter().zip(rows).collect())
    }

    /// Accepts the revision at `index` of [`Document::revisions`].
    pub fn accept_revision(&mut self, index: usize) -> Result<(), DocxError> {
        self.resolve(Action::Accept, Some(&[index])).map(drop)
    }

    /// Rejects the revision at `index` of [`Document::revisions`].
    pub fn reject_revision(&mut self, index: usize) -> Result<(), DocxError> {
        self.resolve(Action::Reject, Some(&[index])).map(drop)
    }

    /// Accepts every tracked change; returns how many revisions were resolved.
    pub fn accept_all_revisions(&mut self) -> usize {
        self.resolve(Action::Accept, None).unwrap_or(0)
    }

    /// Rejects every tracked change; returns how many revisions were resolved.
    pub fn reject_all_revisions(&mut self) -> usize {
        self.resolve(Action::Reject, None).unwrap_or(0)
    }

    /// Accepts the revisions at `indices` of [`Document::revisions`] (one call, so the
    /// indices all refer to the same list); returns how many were resolved.
    pub fn accept_revisions(&mut self, indices: &[usize]) -> Result<usize, DocxError> {
        self.resolve(Action::Accept, Some(indices))
    }

    /// Rejects the revisions at `indices` of [`Document::revisions`].
    pub fn reject_revisions(&mut self, indices: &[usize]) -> Result<usize, DocxError> {
        self.resolve(Action::Reject, Some(indices))
    }

    /// Indices of the revisions that touch any paragraph of `first..=last`, as Word's
    /// `Range.Revisions` does for a range of whole paragraphs.
    pub fn revisions_in_paragraphs(&self, first: usize, last: usize) -> Vec<usize> {
        self.revisions()
            .iter()
            .filter(|r| r.paragraph <= last && r.last_paragraph >= first)
            .map(|r| r.index)
            .collect()
    }

    /// Accepts the tracked changes located in paragraphs `first..=last`, as Word's
    /// `Range.Revisions.AcceptAll` does for a range of whole paragraphs: a revision that
    /// continues past the range is resolved only inside it. Returns the elements resolved.
    pub fn accept_revisions_in_paragraphs(
        &mut self,
        first: usize,
        last: usize,
    ) -> Result<usize, DocxError> {
        Ok(self.resolve_in_paragraphs(Action::Accept, first, last))
    }

    /// Rejects the tracked changes located in paragraphs `first..=last` (see
    /// [`Document::accept_revisions_in_paragraphs`]).
    pub fn reject_revisions_in_paragraphs(
        &mut self,
        first: usize,
        last: usize,
    ) -> Result<usize, DocxError> {
        Ok(self.resolve_in_paragraphs(Action::Reject, first, last))
    }

    fn resolve_in_paragraphs(&mut self, action: Action, first: usize, last: usize) -> usize {
        let (revs, ranges) = self.list_revisions();
        let mut atoms: HashSet<usize> = ranges
            .iter()
            .enumerate()
            .filter(|(_, ((a, b), _))| *a <= last && *b >= first)
            .map(|(i, _)| i)
            .collect();
        // Word resolves a table row's revision together with everything else in the row.
        let rows: HashSet<(usize, usize)> = atoms.iter().filter_map(|&a| ranges[a].1).collect();
        for (i, ((a, b), _)) in ranges.iter().enumerate() {
            if rows.iter().any(|(ra, rb)| a >= ra && b <= rb) {
                atoms.insert(i);
            }
        }
        // The other half of a move goes with it, wherever it is.
        let hit: Vec<&Revision> = revs
            .iter()
            .filter(|r| r.atoms.iter().any(|a| atoms.contains(a)))
            .collect();
        let names = move_names(&hit, action);
        for r in revs
            .iter()
            .filter(|r| r.move_name.as_ref().is_some_and(|n| names.contains(n)))
        {
            atoms.extend(r.atoms.iter().copied());
        }
        let n = atoms.len();
        self.run_pass(action, Some(atoms));
        n
    }

    fn run_pass(&mut self, action: Action, selected: Option<HashSet<usize>>) {
        let mut pass = Pass {
            action,
            selected,
            counter: 0,
            touched: false,
        };
        let blocks = &mut self.body_mut().blocks;
        pass_blocks(blocks, &mut pass);
        drop_orphan_move_markers(blocks);
        if pass.touched {
            self.edited();
        }
    }

    fn resolve(&mut self, action: Action, which: Option<&[usize]>) -> Result<usize, DocxError> {
        let revs = self.revisions();
        let selected = match which {
            Some(indices) => {
                let mut atoms: HashSet<usize> = HashSet::new();
                let mut chosen: Vec<&Revision> = Vec::new();
                for &index in indices {
                    let r = revs.get(index).ok_or(DocxError::NoSuchRevision {
                        index,
                        count: revs.len(),
                    })?;
                    atoms.extend(r.atoms.iter().copied());
                    chosen.push(r);
                    // Word resolves both halves of a move together. Halves are paired by the
                    // name of their move range; a half without a name pairs by order.
                    let partner = match r.kind {
                        RevisionKind::MovedFrom => Some(RevisionKind::MovedTo),
                        RevisionKind::MovedTo => Some(RevisionKind::MovedFrom),
                        _ => None,
                    };
                    if let (Some(other), None) = (partner, &r.move_name) {
                        let ordinal = revs[..index].iter().filter(|v| v.kind == r.kind).count();
                        if let Some(p) = revs.iter().filter(|v| v.kind == other).nth(ordinal) {
                            atoms.extend(p.atoms.iter().copied());
                        }
                    }
                }
                let names = move_names(&chosen, action);
                for v in revs
                    .iter()
                    .filter(|v| v.move_name.as_ref().is_some_and(|n| names.contains(n)))
                {
                    atoms.extend(v.atoms.iter().copied());
                }
                Some(atoms)
            }
            None => None,
        };
        self.run_pass(action, selected);
        Ok(which.map_or(revs.len(), <[usize]>::len))
    }

    /// The paragraphs of the final view: a paragraph whose mark is deleted joins the next one
    /// in the same container, as Word shows it. `first`/`count` index [`Document::paragraph`].
    pub fn final_view_paragraphs(&self) -> Vec<FinalParagraph> {
        fn walk(blocks: &[Block], idx: &mut usize, out: &mut Vec<FinalParagraph>) {
            let mut cur: Option<FinalParagraph> = None;
            for block in blocks {
                match block {
                    Block::Paragraph(p) => {
                        let c = cur.get_or_insert_with(|| FinalParagraph {
                            first: *idx,
                            count: 0,
                            text: String::new(),
                        });
                        c.count += 1;
                        c.text.push_str(&p.text());
                        *idx += 1;
                        if p.mark_revision() != Some(RevisionKind::ParagraphMarkDeleted) {
                            out.extend(cur.take());
                        }
                    }
                    other => {
                        out.extend(cur.take());
                        match other {
                            Block::Table(t) => {
                                for row in t.rows() {
                                    for cell in row.cells() {
                                        walk(&cell.blocks, idx, out);
                                    }
                                }
                            }
                            Block::Sdt(s) => {
                                for c in s.content() {
                                    walk(&c.blocks, idx, out);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            out.extend(cur.take());
        }
        let mut out = Vec::new();
        walk(&self.body().blocks, &mut 0, &mut out);
        out
    }

    /// Text of paragraph `index` in `view`.
    pub fn paragraph_text_view(&self, index: usize, view: TextView) -> Option<String> {
        self.paragraph(index).map(|p| p.text_in_view(view))
    }

    /// Spans of paragraph `index` in `view` (see [`Paragraph::spans`]).
    pub fn paragraph_spans(&self, index: usize, view: TextView) -> Option<Vec<TextSpan>> {
        self.paragraph(index).map(|p| p.spans(view))
    }

    /// Records later edits ([`Document::insert_text`], [`Document::delete_range`],
    /// [`Document::replace_range`], [`Document::split_paragraph`]) as tracked changes by
    /// `track`'s author; `None` turns recording off.
    pub fn set_track_changes(&mut self, track: Option<TrackChanges>) {
        self.tracking = track;
    }

    /// The recording settings, when edits are tracked.
    pub fn track_changes(&self) -> Option<&TrackChanges> {
        self.tracking.as_ref()
    }
}
