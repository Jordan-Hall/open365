//! Word table editing on the lossless model (ECMA-376 Part 1 §17.4): insert a table, insert
//! and delete rows and columns, merge and split cells, widths, heights, shading, borders,
//! alignment, auto-fit, and conversion between text and tables.
//!
//! Every rule here was measured against Microsoft Word with `tools/office/table-edit-oracle.ps1`
//! (reference output in `fixtures/reference/docx/tbledit-*`; `docs/architecture/docx-table-editing.md`
//! lists what matches and what is a documented divergence). Tables are addressed by their
//! position in document order (`Document::table`), rows by `w:tr` position and columns by
//! grid column. An edit that fails changes nothing: it runs on a copy of the table.
//!
//! With track changes on ([`Document::set_track_changes`]), inserted and deleted rows are
//! recorded as Word records them: `w:ins` or `w:del` in `w:trPr`, plus the same revision on
//! each cell paragraph mark (and `w:del` around the removed runs). Column edits, merges and
//! splits are not tracked (Word records them as property changes; see the architecture note).

#![allow(clippy::needless_range_loop, clippy::mut_range_bound)]

use std::ops::Range;

use crate::DocxError;
use crate::document::Document;
use crate::model::{
    Block, Cell, Group, Inline, Paragraph, Row, RowChild, SdtChild, Table, TableChild,
};
use crate::ranges::{DocPos, Path, Step, container, container_mut, locate_paragraph};
use crate::review::{TrackChanges, mark_text_deleted, new_revision_head};
use crate::table::{
    AutoFitMode, Border, BorderSet, HeightRule, Shading, TableJustification, TableLayoutType,
    TableLook, TableWidth, VerticalMerge,
};
use crate::xml::{ElementHead, RawXml, XmlElement, is_w_ns};

type R<T> = Result<T, DocxError>;

fn bad(message: impl Into<String>) -> DocxError {
    DocxError::UnsupportedRange(message.into())
}

/// Border width Word subtracts from the text width when it sizes a new table (the "Table
/// Grid" border is 0.5 pt, 10 twips).
const NEW_TABLE_ALLOWANCE: i64 = 10;
/// Column width Word gives to an empty column of an auto-fit-to-contents table.
const MIN_CONTENT_COLUMN: i64 = 222;
/// Fifty-ths of a percent: `w:type="pct"` 5000 is 100 %.
const FULL_PCT: i64 = 5000;
/// Most columns Word allows in a table.
const MAX_COLUMNS: usize = 63;

/// Which side of a row new rows go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSide {
    Above,
    Below,
}

/// Which side of a column new columns go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnSide {
    Left,
    Right,
}

/// A cell border side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderSide {
    Top,
    Left,
    Bottom,
    Right,
}

/// What absorbs the difference when one column's width changes (Word's `RulerStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnAdjust {
    /// Only this column changes; the table grows or shrinks (`wdAdjustNone`).
    None,
    /// The columns to the right share the difference in proportion to their widths
    /// (`wdAdjustProportional`).
    Proportional,
    /// The columns to the right share the difference equally (`wdAdjustSameWidth`).
    Equal,
    /// The next column absorbs the difference (`wdAdjustFirstColumn`).
    NextColumn,
}

/// How text is split into cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSeparator {
    /// Cells are separated by tab characters, rows by paragraphs.
    Tab,
    /// Cells are separated by commas (written as `, ` when a table becomes text).
    Comma,
    /// Any other single character.
    Char(char),
    /// Every paragraph is one cell; the column count fills the rows.
    Paragraph,
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

const TBL_PR_ORDER: &[&str] = &[
    "tblStyle",
    "tblpPr",
    "tblOverlap",
    "bidiVisual",
    "tblStyleRowBandSize",
    "tblStyleColBandSize",
    "tblW",
    "jc",
    "tblCellSpacing",
    "tblInd",
    "tblBorders",
    "shd",
    "tblLayout",
    "tblCellMar",
    "tblLook",
    "tblCaption",
    "tblDescription",
    "tblPrChange",
];
const TR_PR_ORDER: &[&str] = &[
    "cnfStyle",
    "divId",
    "gridBefore",
    "gridAfter",
    "wBefore",
    "wAfter",
    "cantSplit",
    "trHeight",
    "tblHeader",
    "tblCellSpacing",
    "jc",
    "hidden",
    "ins",
    "del",
    "trPrChange",
];
const TC_PR_ORDER: &[&str] = &[
    "cnfStyle",
    "tcW",
    "gridSpan",
    "hMerge",
    "vMerge",
    "tcBorders",
    "shd",
    "noWrap",
    "tcMar",
    "textDirection",
    "tcFitText",
    "vAlign",
    "hideMark",
    "headers",
    "cellIns",
    "cellDel",
    "cellMerge",
    "tcPrChange",
];
const BORDER_ORDER: &[&str] = &[
    "top", "start", "left", "bottom", "end", "right", "insideH", "insideV", "tl2br", "tr2bl",
];

fn rank(order: &[&str], node: &RawXml) -> usize {
    match node {
        RawXml::Element(e) if is_w_ns(e.head.ns.as_deref()) => order
            .iter()
            .position(|n| *n == e.head.local_name())
            .unwrap_or(order.len()),
        RawXml::Element(_) => order.len(),
        _ => 0,
    }
}

/// The child `w:local` of `parent`, created in schema position when missing.
fn upsert<'a>(parent: &'a mut XmlElement, order: &[&str], local: &str) -> &'a mut XmlElement {
    let existing = parent
        .children
        .iter()
        .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w(local)));
    let at = existing.unwrap_or_else(|| {
        let new_rank = order
            .iter()
            .position(|n| *n == local)
            .unwrap_or(order.len());
        let at = parent
            .children
            .iter()
            .position(|c| rank(order, c) > new_rank)
            .unwrap_or(parent.children.len());
        let element = parent.sibling(local);
        parent.children.insert(at, RawXml::Element(element));
        at
    });
    match &mut parent.children[at] {
        RawXml::Element(e) => e,
        // `at` was just found or inserted as an element.
        _ => unreachable!("upsert found an element"),
    }
}

fn remove_child(parent: &mut XmlElement, local: &str) -> bool {
    let before = parent.children.len();
    parent
        .children
        .retain(|c| !matches!(c, RawXml::Element(e) if e.head.is_w(local)));
    parent.children.len() != before
}

fn child_el<'a>(parent: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    parent.children.iter().find_map(|c| match c {
        RawXml::Element(e) if e.head.is_w(local) => Some(e),
        _ => None,
    })
}

fn attr_int(e: &XmlElement, name: &str) -> Option<i64> {
    e.head
        .attr_w(name)
        .and_then(|v| v.trim().parse::<i64>().ok())
}

fn mk(owner: &ElementHead, local: &str, attrs: &[(&str, &str)]) -> XmlElement {
    let mut head = owner.sibling(local);
    for (k, v) in attrs {
        head.set_attr_w(k, v);
    }
    head.into_empty()
}

fn set_attrs(e: &mut XmlElement, attrs: &[(&str, &str)]) {
    for (k, v) in attrs {
        e.head.set_attr_w(k, v);
    }
}

/// A `w:w` + `w:type` width (`w:tblW`, `w:tcW`, `w:wBefore`...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum W {
    Auto,
    Nil,
    Dxa(i64),
    Pct(i64),
}

fn read_w(e: &XmlElement) -> Option<W> {
    let value = attr_int(e, "w");
    Some(match e.head.attr_w("type").unwrap_or("dxa") {
        "auto" => W::Auto,
        "nil" => W::Nil,
        "pct" => W::Pct(value?),
        _ => W::Dxa(value?),
    })
}

fn write_w(e: &mut XmlElement, w: W) {
    let (v, t) = match w {
        W::Auto => (0, "auto"),
        W::Nil => (0, "nil"),
        W::Dxa(v) => (v, "dxa"),
        W::Pct(v) => (v, "pct"),
    };
    set_attrs(e, &[("w", &v.to_string()), ("type", t)]);
}

fn first_child_index(blocks: &[Block], local: &str) -> Option<usize> {
    blocks
        .iter()
        .position(|b| matches!(b, Block::Raw(RawXml::Element(e)) if e.head.is_w(local)))
}

// ---------------------------------------------------------------------------
// Accessors for the raw property elements
// ---------------------------------------------------------------------------

fn tbl_pr(t: &Table) -> Option<&XmlElement> {
    t.children.iter().find_map(|c| match c {
        TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPr") => Some(e),
        _ => None,
    })
}

fn tbl_pr_mut(t: &mut Table) -> &mut XmlElement {
    let at = t
        .children
        .iter()
        .position(|c| matches!(c, TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPr")))
        .unwrap_or_else(|| {
            let element = t.head.sibling("tblPr").into_empty();
            t.children
                .insert(0, TableChild::Raw(RawXml::Element(element)));
            0
        });
    match &mut t.children[at] {
        TableChild::Raw(RawXml::Element(e)) => e,
        _ => unreachable!("tblPr was found or inserted"),
    }
}

fn tbl_grid_mut(t: &mut Table) -> &mut XmlElement {
    let at = t
        .children
        .iter()
        .position(|c| matches!(c, TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblGrid")))
        .unwrap_or_else(|| {
            let element = t.head.sibling("tblGrid").into_empty();
            let after_pr = t
                .children
                .iter()
                .position(
                    |c| matches!(c, TableChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPr")),
                )
                .map_or(0, |i| i + 1);
            t.children
                .insert(after_pr, TableChild::Raw(RawXml::Element(element)));
            after_pr
        });
    match &mut t.children[at] {
        TableChild::Raw(RawXml::Element(e)) => e,
        _ => unreachable!("tblGrid was found or inserted"),
    }
}

fn set_grid(t: &mut Table, widths: &[i64]) {
    let owner = t.head.clone();
    let grid = tbl_grid_mut(t);
    grid.children.clear();
    for w in widths {
        grid.children.push(RawXml::Element(mk(
            &owner,
            "gridCol",
            &[("w", &w.to_string())],
        )));
    }
}

fn row_pr(r: &Row) -> Option<&XmlElement> {
    r.children.iter().find_map(|c| match c {
        RowChild::Raw(RawXml::Element(e)) if e.head.is_w("trPr") => Some(e),
        _ => None,
    })
}

fn row_pr_mut(r: &mut Row) -> &mut XmlElement {
    let at = r
        .children
        .iter()
        .position(|c| matches!(c, RowChild::Raw(RawXml::Element(e)) if e.head.is_w("trPr")))
        .unwrap_or_else(|| {
            let element = r.head.sibling("trPr").into_empty();
            let at = usize::from(r.children.first().is_some_and(
                |c| matches!(c, RowChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPrEx")),
            ));
            r.children
                .insert(at, RowChild::Raw(RawXml::Element(element)));
            at
        });
    match &mut r.children[at] {
        RowChild::Raw(RawXml::Element(e)) => e,
        _ => unreachable!("trPr was found or inserted"),
    }
}

fn cell_pr(c: &Cell) -> Option<&XmlElement> {
    c.blocks.iter().find_map(|b| match b {
        Block::Raw(RawXml::Element(e)) if e.head.is_w("tcPr") => Some(e),
        _ => None,
    })
}

fn cell_pr_mut(c: &mut Cell) -> &mut XmlElement {
    let at = first_child_index(&c.blocks, "tcPr").unwrap_or_else(|| {
        let element = c.head.sibling("tcPr").into_empty();
        c.blocks.insert(0, Block::Raw(RawXml::Element(element)));
        0
    });
    match &mut c.blocks[at] {
        Block::Raw(RawXml::Element(e)) => e,
        _ => unreachable!("tcPr was found or inserted"),
    }
}

/// Drops an empty `w:trPr` / `w:tcPr` left behind by an edit.
fn tidy_row_pr(r: &mut Row) {
    r.children.retain(
        |c| !matches!(c, RowChild::Raw(RawXml::Element(e)) if e.head.is_w("trPr") && e.children.is_empty()),
    );
}

fn grid_before(r: &Row) -> usize {
    row_pr(r)
        .and_then(|p| child_el(p, "gridBefore"))
        .and_then(|e| attr_int(e, "val"))
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0)
}

fn grid_after(r: &Row) -> usize {
    row_pr(r)
        .and_then(|p| child_el(p, "gridAfter"))
        .and_then(|e| attr_int(e, "val"))
        .and_then(|v| usize::try_from(v).ok())
        .unwrap_or(0)
}

fn cell_width(c: &Cell) -> Option<W> {
    cell_pr(c).and_then(|p| child_el(p, "tcW")).and_then(read_w)
}

fn cell_span(c: &Cell) -> usize {
    cell_pr(c)
        .and_then(|p| child_el(p, "gridSpan"))
        .and_then(|e| attr_int(e, "val"))
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v >= 1)
        .unwrap_or(1)
}

fn cell_vmerge(c: &Cell) -> VerticalMerge {
    match cell_pr(c).and_then(|p| child_el(p, "vMerge")) {
        None => VerticalMerge::None,
        Some(m) => match m.head.attr_w("val") {
            Some("restart") => VerticalMerge::Restart,
            _ => VerticalMerge::Continue,
        },
    }
}

fn set_cell_span(c: &mut Cell, span: usize) {
    let pr = cell_pr_mut(c);
    if span <= 1 {
        remove_child(pr, "gridSpan");
    } else {
        let e = upsert(pr, TC_PR_ORDER, "gridSpan");
        set_attrs(e, &[("val", &span.to_string())]);
    }
}

fn set_cell_vmerge(c: &mut Cell, vm: VerticalMerge) {
    let pr = cell_pr_mut(c);
    match vm {
        VerticalMerge::None => {
            remove_child(pr, "vMerge");
        }
        VerticalMerge::Restart => {
            let e = upsert(pr, TC_PR_ORDER, "vMerge");
            e.head.attrs.clear();
            set_attrs(e, &[("val", "restart")]);
        }
        VerticalMerge::Continue => {
            let e = upsert(pr, TC_PR_ORDER, "vMerge");
            e.head.attrs.clear();
        }
    }
}

fn set_cell_width(c: &mut Cell, w: W) {
    let pr = cell_pr_mut(c);
    let e = upsert(pr, TC_PR_ORDER, "tcW");
    write_w(e, w);
}

fn first_paragraph(c: &Cell) -> Option<&Paragraph> {
    c.blocks.iter().find_map(|b| match b {
        Block::Paragraph(p) => Some(p),
        _ => None,
    })
}

fn cell_is_empty(c: &Cell) -> bool {
    c.blocks.iter().all(|b| match b {
        Block::Paragraph(p) => p.content.is_empty(),
        Block::Raw(RawXml::Element(e)) => e.head.is_w("tcPr"),
        _ => false,
    })
}

// ---------------------------------------------------------------------------
// Rows, slots and the grid
// ---------------------------------------------------------------------------

/// A cell placed on the grid.
#[derive(Debug, Clone, Copy)]
struct Slot {
    /// Index in `Row::children`.
    child: usize,
    col: usize,
    span: usize,
    vm: VerticalMerge,
}

fn row_slots(row: &Row) -> Vec<Slot> {
    let mut col = grid_before(row);
    let mut out = Vec::new();
    for (i, c) in row.children.iter().enumerate() {
        if let RowChild::Cell(cell) = c {
            let span = cell_span(cell);
            out.push(Slot {
                child: i,
                col,
                span,
                vm: cell_vmerge(cell),
            });
            col += span;
        }
    }
    out
}

fn row_children(t: &Table) -> Vec<usize> {
    t.children
        .iter()
        .enumerate()
        .filter_map(|(i, c)| matches!(c, TableChild::Row(_)).then_some(i))
        .collect()
}

fn row_ref(t: &Table, row: usize) -> Option<&Row> {
    t.rows().nth(row)
}

fn row_mut(t: &mut Table, row: usize) -> Option<&mut Row> {
    t.children
        .iter_mut()
        .filter_map(|c| match c {
            TableChild::Row(r) => Some(r),
            TableChild::Raw(_) => None,
        })
        .nth(row)
}

fn cell_mut(row: &mut Row, child: usize) -> Option<&mut Cell> {
    match row.children.get_mut(child)? {
        RowChild::Cell(c) => Some(c),
        RowChild::Raw(_) => None,
    }
}

fn cell_at(row: &Row, child: usize) -> Option<&Cell> {
    match row.children.get(child)? {
        RowChild::Cell(c) => Some(c),
        RowChild::Raw(_) => None,
    }
}

fn all_slots(t: &Table) -> Vec<Vec<Slot>> {
    t.rows().map(row_slots).collect()
}

fn slot_covering(slots: &[Slot], col: usize) -> Option<Slot> {
    slots
        .iter()
        .copied()
        .find(|s| s.col <= col && col < s.col + s.span)
}

fn slot_starting_at(slots: &[Slot], col: usize) -> Option<Slot> {
    slots.iter().copied().find(|s| s.col == col)
}

/// The rows a vertical merge covers, given any cell of it.
fn merge_group(slots: &[Vec<Slot>], row: usize, col: usize) -> (usize, usize) {
    let mut first = row;
    while first > 0
        && slots[first]
            .iter()
            .any(|s| s.col == col && s.vm == VerticalMerge::Continue)
        && slots[first - 1].iter().any(|s| s.col == col)
    {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < slots.len()
        && slots[last + 1]
            .iter()
            .any(|s| s.col == col && s.vm == VerticalMerge::Continue)
    {
        last += 1;
    }
    (first, last)
}

/// Makes every vertical merge well formed: a continuation without a start becomes the start,
/// a start without a continuation becomes a plain cell.
fn fix_vmerge(t: &mut Table) {
    let slots = all_slots(t);
    let n = slots.len();
    let mut finals: Vec<Vec<VerticalMerge>> = Vec::with_capacity(n);
    for r in 0..n {
        let mut row = Vec::new();
        for s in &slots[r] {
            let member_above = r > 0
                && slots[r - 1]
                    .iter()
                    .enumerate()
                    .any(|(k, up)| up.col == s.col && finals[r - 1][k] != VerticalMerge::None);
            row.push(match s.vm {
                VerticalMerge::Continue if !member_above => VerticalMerge::Restart,
                other => other,
            });
        }
        finals.push(row);
    }
    for r in (0..n).rev() {
        for (k, s) in slots[r].iter().enumerate() {
            if finals[r][k] == VerticalMerge::Restart {
                let continues = r + 1 < n
                    && slots[r + 1].iter().enumerate().any(|(j, down)| {
                        down.col == s.col && finals[r + 1][j] == VerticalMerge::Continue
                    });
                if !continues {
                    finals[r][k] = VerticalMerge::None;
                }
            }
        }
    }
    for (r, row_slots) in slots.iter().enumerate() {
        for (k, s) in row_slots.iter().enumerate() {
            if finals[r][k] != s.vm
                && let Some(row) = row_mut(t, r)
                && let Some(cell) = cell_mut(row, s.child)
            {
                set_cell_vmerge(cell, finals[r][k]);
            }
        }
    }
}

/// Removes grid columns that no row starts a cell at, merging each into its left neighbour.
fn normalize_grid(t: &mut Table) {
    if t.rows().any(|r| grid_before(r) > 0 || grid_after(r) > 0) {
        return;
    }
    let mut grid = t.grid_twips();
    let mut j = grid.len();
    while j > 1 {
        j -= 1;
        let slots = all_slots(t);
        let used = slots.iter().any(|row| row.iter().any(|s| s.col == j));
        if used {
            continue;
        }
        grid[j - 1] += grid[j];
        grid.remove(j);
        for (r, row) in slots.iter().enumerate() {
            for s in row {
                if s.col < j
                    && s.col + s.span > j
                    && let Some(row_m) = row_mut(t, r)
                    && let Some(cell) = cell_mut(row_m, s.child)
                {
                    set_cell_span(cell, s.span - 1);
                }
            }
        }
    }
    set_grid(t, &grid);
}

/// Left edges of the grid columns plus the right edge of the last: `[0, w0, w0+w1, ...]`.
fn grid_edges(grid: &[i64]) -> Vec<i64> {
    let mut edges = vec![0i64];
    let mut acc = 0;
    for w in grid {
        acc += w;
        edges.push(acc);
    }
    edges
}

fn distribute(total: i64, n: usize) -> Vec<i64> {
    let n_i = n as i64;
    let base = total.div_euclid(n_i);
    let rem = usize::try_from(total.rem_euclid(n_i)).unwrap_or(0);
    (0..n)
        .map(|i| if i >= n - rem { base + 1 } else { base })
        .collect()
}

fn span_width(grid: &[i64], col: usize, span: usize) -> i64 {
    grid.iter().skip(col).take(span).sum()
}

// ---------------------------------------------------------------------------
// Revisions
// ---------------------------------------------------------------------------

struct Rev<'a> {
    track: &'a TrackChanges,
    next: u64,
}

impl Rev<'_> {
    fn head(&mut self, owner: &ElementHead, local: &str) -> ElementHead {
        let id = self.next;
        self.next += 1;
        new_revision_head(owner, local, id, self.track)
    }
}

const MARK_REVISIONS: &[&str] = &["ins", "del", "moveFrom", "moveTo"];

fn paragraph_mark_rpr(p: &mut Paragraph) -> &mut XmlElement {
    crate::ppr::upsert_child(p.props_mut(), "rPr", |_| {});
    let props = p.props_mut();
    let at = props
        .children
        .iter()
        .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w("rPr")))
        .unwrap_or(0);
    match &mut props.children[at] {
        RawXml::Element(e) => e,
        _ => unreachable!("rPr was found or inserted"),
    }
}

fn mark_paragraph_inserted(p: &mut Paragraph, rev: &mut Rev<'_>) {
    let head = rev.head(&p.head.clone(), "ins");
    let rpr = paragraph_mark_rpr(p);
    rpr.children.retain(
        |c| !matches!(c, RawXml::Element(e) if MARK_REVISIONS.iter().any(|n| e.head.is_w(n))),
    );
    rpr.children.insert(0, RawXml::Element(head.into_empty()));
}

fn mark_paragraph_deleted(p: &mut Paragraph, rev: &mut Rev<'_>) {
    let owner = p.head.clone();
    delete_content(&mut p.content, &owner, rev);
    let head = rev.head(&owner, "del");
    let rpr = paragraph_mark_rpr(p);
    if child_el(rpr, "del").is_some() {
        return;
    }
    let at = rpr
        .children
        .iter()
        .position(|c| !matches!(c, RawXml::Element(e) if e.head.is_w("ins")))
        .unwrap_or(rpr.children.len());
    rpr.children.insert(at, RawXml::Element(head.into_empty()));
}

fn flush_deleted(
    out: &mut Vec<Inline>,
    pending: &mut Vec<Inline>,
    owner: &ElementHead,
    rev: &mut Rev<'_>,
) {
    if !pending.is_empty() {
        out.push(Inline::Group(Group {
            head: rev.head(owner, "del"),
            content: std::mem::take(pending),
        }));
    }
}

fn delete_content(content: &mut Vec<Inline>, owner: &ElementHead, rev: &mut Rev<'_>) {
    let old = std::mem::take(content);
    let mut pending: Vec<Inline> = Vec::new();
    for inline in old {
        match inline {
            Inline::Run(mut run) if !run.content.is_empty() => {
                mark_text_deleted(&mut run);
                pending.push(Inline::Run(run));
            }
            Inline::Group(mut g) if !g.head.is_w("del") && !g.head.is_w("moveFrom") => {
                flush_deleted(content, &mut pending, owner, rev);
                delete_content(&mut g.content, owner, rev);
                content.push(Inline::Group(g));
            }
            other => {
                flush_deleted(content, &mut pending, owner, rev);
                content.push(other);
            }
        }
    }
    flush_deleted(content, &mut pending, owner, rev);
}

fn mark_row(row: &mut Row, local: &str, rev: &mut Rev<'_>) {
    let head = rev.head(&row.head.clone(), local);
    let pr = row_pr_mut(row);
    remove_child(pr, "ins");
    remove_child(pr, "del");
    let el = upsert(pr, TR_PR_ORDER, local);
    *el = head.into_empty();
}

fn row_revision<'a>(row: &'a Row, local: &str) -> Option<&'a XmlElement> {
    row_pr(row).and_then(|p| child_el(p, local))
}

fn for_each_paragraph(row: &mut Row, mut f: impl FnMut(&mut Paragraph)) {
    for child in &mut row.children {
        if let RowChild::Cell(cell) = child {
            for block in &mut cell.blocks {
                if let Block::Paragraph(p) = block {
                    f(p);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

/// Properties of a paragraph mark without revision marks, section breaks or property changes.
fn clean_ppr(mut e: XmlElement) -> XmlElement {
    remove_child(&mut e, "sectPr");
    remove_child(&mut e, "pPrChange");
    for c in &mut e.children {
        if let RawXml::Element(rpr) = c
            && rpr.head.is_w("rPr")
        {
            for local in ["ins", "del", "moveFrom", "moveTo", "rPrChange"] {
                remove_child(rpr, local);
            }
        }
    }
    e.children.retain(
        |c| !matches!(c, RawXml::Element(r) if r.head.is_w("rPr") && r.children.is_empty()),
    );
    e
}

fn blank_paragraph(src: Option<&Paragraph>, owner: &ElementHead) -> Paragraph {
    let props = src
        .and_then(|p| p.props.clone())
        .map(clean_ppr)
        .filter(|e| !e.children.is_empty());
    Paragraph {
        head: owner.sibling("p"),
        props,
        content: Vec::new(),
    }
}

/// A cell with `tc_pr` and one empty paragraph that keeps the paragraph properties of
/// `like`'s first paragraph.
fn blank_cell(like: &Cell, tc_pr: Option<XmlElement>) -> Cell {
    let mut blocks = Vec::new();
    if let Some(pr) = tc_pr {
        blocks.push(Block::Raw(RawXml::Element(pr)));
    }
    blocks.push(Block::Paragraph(blank_paragraph(
        first_paragraph(like),
        &like.head,
    )));
    Cell {
        head: like.head.sibling("tc"),
        blocks,
    }
}

fn tc_pr_with_width(owner: &ElementHead, w: W) -> XmlElement {
    let mut pr = owner.sibling("tcPr").into_empty();
    let e = upsert(&mut pr, TC_PR_ORDER, "tcW");
    write_w(e, w);
    pr
}

fn look_element(owner: &ElementHead, look: TableLook) -> XmlElement {
    let mut mask = 0u32;
    for (on, bit) in [
        (look.first_row, 0x0020),
        (look.last_row, 0x0040),
        (look.first_column, 0x0080),
        (look.last_column, 0x0100),
        (look.no_h_band, 0x0200),
        (look.no_v_band, 0x0400),
    ] {
        if on {
            mask |= bit;
        }
    }
    let flag = |b: bool| if b { "1" } else { "0" };
    mk(
        owner,
        "tblLook",
        &[
            ("val", &format!("{mask:04X}")),
            ("firstRow", flag(look.first_row)),
            ("lastRow", flag(look.last_row)),
            ("firstColumn", flag(look.first_column)),
            ("lastColumn", flag(look.last_column)),
            ("noHBand", flag(look.no_h_band)),
            ("noVBand", flag(look.no_v_band)),
        ],
    )
}

fn border_element(owner: &ElementHead, side: &str, b: &Border) -> XmlElement {
    if b.style == "nil" {
        return mk(owner, side, &[("val", "nil")]);
    }
    let color = b.color.map_or_else(
        || "auto".to_owned(),
        |c| format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
    );
    mk(
        owner,
        side,
        &[
            ("val", &b.style),
            ("sz", &(b.width * 8.0).round().to_string()),
            ("space", &b.space.round().to_string()),
            ("color", &color),
        ],
    )
}

/// A new table the way Word's Insert Table makes it (measured): style "Table Grid" (or direct
/// single borders when the document has no such style), automatic table width, the default
/// `w:tblLook`, one empty paragraph per cell.
fn build_table(
    owner: &ElementHead,
    rows: usize,
    cols: usize,
    text_width: i64,
    mode: AutoFitMode,
    grid_style: bool,
    mut rev: Option<&mut Rev<'_>>,
) -> Table {
    let usable = (text_width - NEW_TABLE_ALLOWANCE).max(cols as i64);
    let widths = match mode {
        AutoFitMode::Content => vec![MIN_CONTENT_COLUMN; cols],
        _ => distribute(usable, cols),
    };
    let pcts = distribute(FULL_PCT, cols);

    let mut pr = owner.sibling("tblPr").into_empty();
    if grid_style {
        let e = upsert(&mut pr, TBL_PR_ORDER, "tblStyle");
        set_attrs(e, &[("val", "TableGrid")]);
    }
    {
        let e = upsert(&mut pr, TBL_PR_ORDER, "tblW");
        write_w(
            e,
            if mode == AutoFitMode::Window {
                W::Pct(FULL_PCT)
            } else {
                W::Auto
            },
        );
    }
    if !grid_style {
        let borders = upsert(&mut pr, TBL_PR_ORDER, "tblBorders");
        let single = Border {
            style: "single".to_owned(),
            width: 0.5,
            space: 0.0,
            color: None,
        };
        for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
            borders
                .children
                .push(RawXml::Element(border_element(owner, side, &single)));
        }
    }
    pr.children
        .push(RawXml::Element(look_element(owner, TableLook::default())));

    let mut table = Table {
        head: owner.sibling("tbl"),
        children: vec![TableChild::Raw(RawXml::Element(pr))],
    };
    set_grid(&mut table, &widths);
    for _ in 0..rows {
        let mut row = Row {
            head: owner.sibling("tr"),
            children: Vec::new(),
        };
        for c in 0..cols {
            let w = match mode {
                AutoFitMode::Fixed => W::Dxa(widths[c]),
                AutoFitMode::Content => W::Auto,
                AutoFitMode::Window => W::Pct(pcts[c]),
            };
            let mut paragraph = Paragraph {
                head: owner.sibling("p"),
                props: None,
                content: Vec::new(),
            };
            if let Some(rev) = rev.as_deref_mut() {
                mark_paragraph_inserted(&mut paragraph, rev);
            }
            row.children.push(RowChild::Cell(Cell {
                head: owner.sibling("tc"),
                blocks: vec![
                    Block::Raw(RawXml::Element(tc_pr_with_width(owner, w))),
                    Block::Paragraph(paragraph),
                ],
            }));
        }
        if let Some(rev) = rev.as_deref_mut() {
            mark_row(&mut row, "ins", rev);
        }
        table.children.push(TableChild::Row(row));
    }
    table
}

// ---------------------------------------------------------------------------
// Table width bookkeeping
// ---------------------------------------------------------------------------

fn tbl_w(t: &Table) -> Option<W> {
    tbl_pr(t).and_then(|p| child_el(p, "tblW")).and_then(read_w)
}

fn set_tbl_w(t: &mut Table, w: W) {
    let pr = tbl_pr_mut(t);
    let e = upsert(pr, TBL_PR_ORDER, "tblW");
    write_w(e, w);
}

/// Percent widths of every cell from the grid (auto-fit to window).
fn refresh_pct_cells(t: &mut Table) {
    let grid = t.grid_twips();
    let total: i64 = grid.iter().sum();
    if total <= 0 {
        return;
    }
    let slots = all_slots(t);
    for (r, row_slots) in slots.iter().enumerate() {
        for s in row_slots {
            let w = span_width(&grid, s.col, s.span);
            let pct = (w as f64 / total as f64 * FULL_PCT as f64).round() as i64;
            if let Some(row) = row_mut(t, r)
                && let Some(cell) = cell_mut(row, s.child)
            {
                set_cell_width(cell, W::Pct(pct));
            }
        }
    }
}

/// Word's rule for `w:tblW` after the grid changed from `old_total` twips: a table with a
/// width in twips follows the grid, an automatic one becomes a width in twips once it is wider
/// than the text area, a percentage scales and its cells follow the grid.
fn finish_widths(t: &mut Table, old_total: i64, text_width: i64) {
    let total: i64 = t.grid_twips().iter().sum();
    match tbl_w(t) {
        Some(W::Dxa(_)) => set_tbl_w(t, W::Dxa(total)),
        Some(W::Auto) | None => {
            if total > text_width {
                set_tbl_w(t, W::Dxa(total));
            }
        }
        Some(W::Pct(old)) => {
            if old_total > 0 {
                let scaled = (old as f64 * total as f64 / old_total as f64).round() as i64;
                set_tbl_w(t, W::Pct(scaled));
            }
            refresh_pct_cells(t);
        }
        Some(W::Nil) => {}
    }
}

// ---------------------------------------------------------------------------
// Row operations
// ---------------------------------------------------------------------------

fn clean_row_pr(pr: &mut XmlElement) {
    for local in ["ins", "del", "trPrChange"] {
        remove_child(pr, local);
    }
}

/// A blank copy of `src`: same row and cell properties and paragraph formatting, no text.
/// `vm_for` decides the vertical merge of the copy per start column.
fn blank_row(src: &Row, vm_for: impl Fn(usize, VerticalMerge) -> VerticalMerge) -> Row {
    let slots = row_slots(src);
    let mut children = Vec::new();
    for (i, c) in src.children.iter().enumerate() {
        match c {
            RowChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPrEx") => {
                children.push(RowChild::Raw(RawXml::Element(e.clone())));
            }
            RowChild::Raw(RawXml::Element(e)) if e.head.is_w("trPr") => {
                let mut pr = e.clone();
                clean_row_pr(&mut pr);
                if !pr.children.is_empty() {
                    children.push(RowChild::Raw(RawXml::Element(pr)));
                }
            }
            RowChild::Cell(cell) => {
                let slot = slots.iter().find(|s| s.child == i).copied();
                let mut pr = cell_pr(cell).cloned();
                if let (Some(p), Some(s)) = (pr.as_mut(), slot) {
                    for local in ["cellIns", "cellDel", "cellMerge", "tcPrChange"] {
                        remove_child(p, local);
                    }
                    remove_child(p, "vMerge");
                    match vm_for(s.col, s.vm) {
                        VerticalMerge::None => {}
                        VerticalMerge::Restart => {
                            let e = upsert(p, TC_PR_ORDER, "vMerge");
                            set_attrs(e, &[("val", "restart")]);
                        }
                        VerticalMerge::Continue => {
                            upsert(p, TC_PR_ORDER, "vMerge");
                        }
                    }
                }
                children.push(RowChild::Cell(blank_cell(cell, pr)));
            }
            RowChild::Raw(_) => {}
        }
    }
    Row {
        head: src.head.sibling("tr"),
        children,
    }
}

fn insert_rows_in(
    t: &mut Table,
    row: usize,
    side: RowSide,
    count: usize,
    mut rev: Option<&mut Rev<'_>>,
) -> R<usize> {
    let positions = row_children(t);
    let n = positions.len();
    if row >= n {
        return Err(bad(format!("row {row} out of range (table has {n} rows)")));
    }
    if count == 0 {
        return Ok(row);
    }
    let slots = all_slots(t);
    let p = match side {
        RowSide::Above => row,
        RowSide::Below => row + 1,
    };
    let prev: &[Slot] = if p > 0 { &slots[p - 1] } else { &[] };
    let next: &[Slot] = if p < n { &slots[p] } else { &[] };
    // A new row inside a vertical merge continues it; elsewhere its cells are plain.
    let vm_for = |col: usize, own: VerticalMerge| {
        if own == VerticalMerge::None {
            return VerticalMerge::None;
        }
        let above = prev
            .iter()
            .any(|s| s.col == col && s.vm != VerticalMerge::None);
        let below = next
            .iter()
            .any(|s| s.col == col && s.vm == VerticalMerge::Continue);
        if above && below {
            VerticalMerge::Continue
        } else {
            VerticalMerge::None
        }
    };
    let Some(reference) = row_ref(t, row) else {
        return Err(bad("row not found"));
    };
    let template = blank_row(reference, vm_for);
    let at = if p < n {
        positions[p]
    } else {
        positions[n - 1] + 1
    };
    for k in 0..count {
        let mut fresh = template.clone();
        if let Some(rev) = rev.as_deref_mut() {
            mark_row(&mut fresh, "ins", rev);
            for_each_paragraph(&mut fresh, |para| mark_paragraph_inserted(para, rev));
        }
        t.children.insert(at + k, TableChild::Row(fresh));
    }
    Ok(p)
}

fn is_own_insertion(row: &Row, rev: &Rev<'_>) -> bool {
    row_revision(row, "ins")
        .is_some_and(|i| i.head.attr_w("author") == Some(rev.track.author.as_str()))
}

fn delete_rows_in(t: &mut Table, range: Range<usize>, rev: Option<&mut Rev<'_>>) -> R<()> {
    let n = row_children(t).len();
    if range.start >= range.end || range.end > n {
        return Err(bad(format!(
            "rows {}..{} out of range (table has {n} rows)",
            range.start, range.end
        )));
    }
    let mut remove: Vec<usize> = Vec::new();
    match rev {
        Some(rev) => {
            for r in range {
                let Some(row) = row_mut(t, r) else { continue };
                // A row the same author inserted while tracking is removed outright.
                if is_own_insertion(row, rev) {
                    remove.push(r);
                    continue;
                }
                mark_row(row, "del", rev);
                for_each_paragraph(row, |para| mark_paragraph_deleted(para, rev));
            }
        }
        None => remove.extend(range),
    }
    let positions = row_children(t);
    for r in remove.into_iter().rev() {
        t.children.remove(positions[r]);
    }
    fix_vmerge(t);
    Ok(())
}

// ---------------------------------------------------------------------------
// Column operations
// ---------------------------------------------------------------------------

fn insert_column_in(t: &mut Table, col: usize, side: ColumnSide, text_width: i64) -> R<()> {
    let mut grid = t.grid_twips();
    let ncols = grid.len();
    if col >= ncols {
        return Err(bad(format!(
            "column {col} out of range (table has {ncols})"
        )));
    }
    let old_total: i64 = grid.iter().sum();
    let gi = match side {
        ColumnSide::Left => col,
        ColumnSide::Right => col + 1,
    };
    let slots = all_slots(t);
    // Auto-width cells (auto-fit to contents) get Word's minimum column.
    let content_like = t
        .rows()
        .next()
        .is_some_and(|r| r.cells().any(|c| matches!(cell_width(c), Some(W::Auto))));
    let new_w = if content_like {
        MIN_CONTENT_COLUMN
    } else {
        grid[col]
    };
    for (r, row_slots_r) in slots.iter().enumerate() {
        let Some(row) = row_mut(t, r) else { continue };
        let reference = slot_covering(row_slots_r, col);
        let template = reference.and_then(|s| cell_at(row, s.child).cloned());
        let width_kind = template.as_ref().and_then(cell_width);
        let new_cell_w = match width_kind {
            Some(W::Auto) => W::Auto,
            Some(W::Pct(_)) => W::Pct(0),
            _ => W::Dxa(new_w),
        };
        if let Some(starts) = slot_starting_at(row_slots_r, gi) {
            if let Some(tpl) = &template {
                let cell = blank_cell(tpl, Some(tc_pr_with_width(&tpl.head, new_cell_w)));
                row.children.insert(starts.child, RowChild::Cell(cell));
            }
        } else if let Some(ends) = row_slots_r.iter().copied().find(|s| s.col + s.span == gi) {
            if let Some(tpl) = &template {
                let cell = blank_cell(tpl, Some(tc_pr_with_width(&tpl.head, new_cell_w)));
                row.children.insert(ends.child + 1, RowChild::Cell(cell));
            }
        } else if let Some(inside) = row_slots_r
            .iter()
            .copied()
            .find(|s| s.col < gi && gi < s.col + s.span)
        {
            // The new column falls inside a spanning cell: the cell grows by it.
            if let Some(cell) = cell_mut(row, inside.child) {
                set_cell_span(cell, inside.span + 1);
                if let Some(W::Dxa(v)) = cell_width(cell) {
                    set_cell_width(cell, W::Dxa(v + new_w));
                }
            }
        } else {
            let before = grid_before(row);
            if gi <= before {
                bump_blank(row, "gridBefore", "wBefore", new_w);
            } else {
                bump_blank(row, "gridAfter", "wAfter", new_w);
            }
        }
    }
    grid.insert(gi, new_w);
    set_grid(t, &grid);
    finish_widths(t, old_total, text_width);
    Ok(())
}

/// Adds one blank grid column at the start or end of a row (`w:gridBefore` / `w:gridAfter`).
fn bump_blank(row: &mut Row, count: &str, width: &str, w: i64) {
    let pr = row_pr_mut(row);
    let current = child_el(pr, count)
        .and_then(|e| attr_int(e, "val"))
        .unwrap_or(0);
    let e = upsert(pr, TR_PR_ORDER, count);
    set_attrs(e, &[("val", &(current + 1).to_string())]);
    let old_w = child_el(pr, width).and_then(read_w);
    let e = upsert(pr, TR_PR_ORDER, width);
    let total = match old_w {
        Some(W::Dxa(v)) => v + w,
        _ => w,
    };
    write_w(e, W::Dxa(total));
}

fn delete_columns_in(t: &mut Table, range: Range<usize>, text_width: i64) -> R<()> {
    let mut grid = t.grid_twips();
    let ncols = grid.len();
    if range.start >= range.end || range.end > ncols {
        return Err(bad(format!(
            "columns {}..{} out of range (table has {ncols})",
            range.start, range.end
        )));
    }
    let old_total: i64 = grid.iter().sum();
    let removed = range.end - range.start;
    let slots = all_slots(t);
    let mut empty_rows = Vec::new();
    for (r, row_slots_r) in slots.iter().enumerate() {
        let Some(row) = row_mut(t, r) else { continue };
        let mut doomed = Vec::new();
        for s in row_slots_r {
            let lo = s.col.max(range.start);
            let hi = (s.col + s.span).min(range.end);
            if lo >= hi {
                continue;
            }
            let overlap = hi - lo;
            if overlap == s.span {
                doomed.push(s.child);
            } else if let Some(cell) = cell_mut(row, s.child) {
                set_cell_span(cell, s.span - overlap);
                if let Some(W::Dxa(v)) = cell_width(cell) {
                    let cut = span_width(&grid, lo, overlap);
                    set_cell_width(cell, W::Dxa((v - cut).max(0)));
                }
            }
        }
        for child in doomed.iter().rev() {
            row.children.remove(*child);
        }
        let remaining: usize = row.cells().map(cell_span).sum();
        if remaining == 0 {
            empty_rows.push(r);
            continue;
        }
        let covered = grid_before(row) + remaining + grid_after(row);
        let wanted = ncols - removed;
        if covered < wanted {
            let deficit = wanted - covered;
            let pr = row_pr_mut(row);
            let current = child_el(pr, "gridAfter")
                .and_then(|e| attr_int(e, "val"))
                .unwrap_or(0);
            let e = upsert(pr, TR_PR_ORDER, "gridAfter");
            set_attrs(e, &[("val", &(current + deficit as i64).to_string())]);
        }
    }
    grid.drain(range);
    set_grid(t, &grid);
    let positions = row_children(t);
    for r in empty_rows.into_iter().rev() {
        t.children.remove(positions[r]);
    }
    fix_vmerge(t);
    finish_widths(t, old_total, text_width);
    Ok(())
}

fn set_column_width_in(
    t: &mut Table,
    col: usize,
    width: i64,
    adjust: ColumnAdjust,
    text_width: i64,
) -> R<()> {
    let mut grid = t.grid_twips();
    let n = grid.len();
    if col >= n {
        return Err(bad(format!("column {col} out of range (table has {n})")));
    }
    if width <= 0 {
        return Err(bad("column width must be positive"));
    }
    let old_total: i64 = grid.iter().sum();
    let old = grid.clone();
    let delta = width - grid[col];
    grid[col] = width;
    let right: Vec<usize> = (col + 1..n).collect();
    if !right.is_empty() {
        match adjust {
            ColumnAdjust::None => {}
            ColumnAdjust::NextColumn => grid[col + 1] -= delta,
            ColumnAdjust::Equal => {
                let share = distribute(delta.abs(), right.len());
                for (k, i) in right.iter().enumerate() {
                    grid[*i] -= delta.signum() * share[k];
                }
            }
            ColumnAdjust::Proportional => {
                let sum: i64 = right.iter().map(|i| old[*i]).sum();
                let mut left = delta;
                for (k, i) in right.iter().enumerate() {
                    let part = if k + 1 == right.len() {
                        left
                    } else {
                        (delta as f64 * old[*i] as f64 / sum as f64).round() as i64
                    };
                    grid[*i] -= part;
                    left -= part;
                }
            }
        }
    }
    if grid.iter().any(|w| *w <= 0) {
        return Err(bad("the other columns cannot absorb that width"));
    }
    let slots = all_slots(t);
    for (r, row_slots_r) in slots.iter().enumerate() {
        for s in row_slots_r {
            let changed = (s.col..s.col + s.span).any(|c| grid.get(c) != old.get(c));
            if !changed {
                continue;
            }
            if let Some(row) = row_mut(t, r)
                && let Some(cell) = cell_mut(row, s.child)
                && let Some(W::Dxa(_)) = cell_width(cell)
            {
                set_cell_width(cell, W::Dxa(span_width(&grid, s.col, s.span)));
            }
        }
    }
    set_grid(t, &grid);
    finish_widths(t, old_total, text_width);
    Ok(())
}

// ---------------------------------------------------------------------------
// Merge and split
// ---------------------------------------------------------------------------

fn merge_cells_in(t: &mut Table, from: (usize, usize), to: (usize, usize)) -> R<()> {
    let slots = all_slots(t);
    let nrows = slots.len();
    let ncols = t.grid_twips().len();
    let (mut r0, mut r1) = (from.0.min(to.0), from.0.max(to.0));
    let (mut c0, mut c1) = (from.1.min(to.1), from.1.max(to.1));
    if r1 >= nrows || c1 >= ncols {
        return Err(bad("the cells to merge are outside the table"));
    }
    // Grow the rectangle until it holds whole cells and whole vertical merges.
    loop {
        let mut changed = false;
        for r in r0..=r1 {
            for s in &slots[r] {
                if s.col + s.span <= c0 || s.col > c1 {
                    continue;
                }
                if s.col < c0 {
                    c0 = s.col;
                    changed = true;
                }
                if s.col + s.span - 1 > c1 {
                    c1 = s.col + s.span - 1;
                    changed = true;
                }
                let (g0, g1) = merge_group(&slots, r, s.col);
                if g0 < r0 {
                    r0 = g0;
                    changed = true;
                }
                if g1 > r1 {
                    r1 = g1;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let inside = |s: &Slot| s.col >= c0 && s.col + s.span - 1 <= c1;
    let top: Vec<Slot> = slots[r0].iter().copied().filter(inside).collect();
    if top.is_empty() {
        return Err(bad("no cell at the top left of the selection"));
    }
    let single = r0 == r1 && top.len() == 1;
    if single {
        return Ok(());
    }

    // Content: the first cell's paragraphs, then those of every other non-empty cell in order.
    let mut gathered: Vec<Block> = Vec::new();
    let mut widths: Vec<W> = Vec::new();
    for r in r0..=r1 {
        for s in slots[r].iter().filter(|s| inside(s)) {
            let Some(cell) = row_ref(t, r).and_then(|row| cell_at(row, s.child)) else {
                continue;
            };
            if r == r0
                && let Some(w) = cell_width(cell)
            {
                widths.push(w);
            }
            if s.vm == VerticalMerge::Continue || cell_is_empty(cell) {
                continue;
            }
            gathered.extend(
                cell.blocks
                    .iter()
                    .filter(|b| matches!(b, Block::Paragraph(_) | Block::Table(_) | Block::Sdt(_)))
                    .cloned(),
            );
        }
    }
    let grid = t.grid_twips();
    let span = c1 - c0 + 1;
    let merged_width = match widths.as_slice() {
        ws if !ws.is_empty() && ws.iter().all(|w| matches!(w, W::Dxa(_))) => W::Dxa(
            ws.iter()
                .map(|w| if let W::Dxa(v) = w { *v } else { 0 })
                .sum(),
        ),
        ws if !ws.is_empty() && ws.iter().all(|w| matches!(w, W::Pct(_))) => W::Pct(
            ws.iter()
                .map(|w| if let W::Pct(v) = w { *v } else { 0 })
                .sum(),
        ),
        ws if ws.iter().any(|w| matches!(w, W::Auto)) => W::Auto,
        _ => W::Dxa(span_width(&grid, c0, span)),
    };

    // Rewrite the rows.
    for r in r0..=r1 {
        let doomed: Vec<Slot> = slots[r].iter().copied().filter(inside).collect();
        let Some(row) = row_mut(t, r) else { continue };
        let Some(first) = doomed.first().copied() else {
            return Err(bad("a row of the selection has no cell in it"));
        };
        if r == r0 {
            if let Some(cell) = cell_mut(row, first.child) {
                if !gathered.is_empty() {
                    cell.blocks.retain(|b| matches!(b, Block::Raw(_)));
                    cell.blocks.extend(gathered.clone());
                    // A cell must end in a paragraph.
                    if !matches!(cell.blocks.last(), Some(Block::Paragraph(_))) {
                        let head = cell.head.sibling("p");
                        cell.blocks.push(Block::Paragraph(Paragraph {
                            head,
                            props: None,
                            content: Vec::new(),
                        }));
                    }
                }
                set_cell_span(cell, span);
                set_cell_width(cell, merged_width);
                set_cell_vmerge(
                    cell,
                    if r1 > r0 {
                        VerticalMerge::Restart
                    } else {
                        VerticalMerge::None
                    },
                );
            }
            for s in doomed.iter().skip(1).rev() {
                row.children.remove(s.child);
            }
        } else {
            let template = cell_at(row, first.child).cloned();
            for s in doomed.iter().skip(1).rev() {
                row.children.remove(s.child);
            }
            if let Some(tpl) = template {
                let mut pr = tc_pr_with_width(&tpl.head, merged_width);
                if span > 1 {
                    let e = upsert(&mut pr, TC_PR_ORDER, "gridSpan");
                    set_attrs(e, &[("val", &span.to_string())]);
                }
                upsert(&mut pr, TC_PR_ORDER, "vMerge");
                let mut cont = blank_cell(&tpl, Some(pr));
                // The continuation holds one plain empty paragraph.
                for b in &mut cont.blocks {
                    if let Block::Paragraph(p) = b {
                        p.props = None;
                    }
                }
                row.children[first.child] = RowChild::Cell(cont);
            }
        }
    }
    // Rows that only continue the merge above have nothing left to show; Word drops them.
    let positions = row_children(t);
    let mut drop_rows = Vec::new();
    for r in r0 + 1..=r1 {
        if let Some(row) = row_ref(t, r) {
            let cells: Vec<&Cell> = row.cells().collect();
            if !cells.is_empty()
                && cells
                    .iter()
                    .all(|c| cell_vmerge(c) == VerticalMerge::Continue)
            {
                drop_rows.push(r);
            }
        }
    }
    for r in drop_rows.into_iter().rev() {
        t.children.remove(positions[r]);
    }
    fix_vmerge(t);
    normalize_grid(t);
    Ok(())
}

/// Splits grid column `g` at `x` twips from its left edge: both halves keep the cells that
/// spanned it, which now span one more column.
fn refine_grid(t: &mut Table, g: usize, left: i64) {
    let mut grid = t.grid_twips();
    let slots = all_slots(t);
    for (r, row_slots_r) in slots.iter().enumerate() {
        for s in row_slots_r {
            if s.col <= g
                && g < s.col + s.span
                && let Some(row) = row_mut(t, r)
                && let Some(cell) = cell_mut(row, s.child)
            {
                set_cell_span(cell, s.span + 1);
            }
        }
    }
    let right = grid[g] - left;
    grid[g] = left;
    grid.insert(g + 1, right);
    set_grid(t, &grid);
}

fn split_cell_in(t: &mut Table, row: usize, col: usize, nrows: usize, ncols: usize) -> R<()> {
    let nrows = nrows.max(1);
    let ncols = ncols.max(1);
    if ncols > MAX_COLUMNS {
        return Err(bad("a table cannot have more than 63 columns"));
    }
    let slots = all_slots(t);
    let n = slots.len();
    if row >= n {
        return Err(bad(format!("row {row} out of range (table has {n} rows)")));
    }
    let Some(slot) = slot_covering(&slots[row], col) else {
        return Err(bad("there is no cell at that position"));
    };
    let (g0, g1) = if slot.vm == VerticalMerge::None {
        (row, row)
    } else {
        merge_group(&slots, row, slot.col)
    };
    let group = g1 - g0 + 1;
    if group >= 2 && nrows > group {
        return Err(bad(format!(
            "a cell merged over {group} rows splits into at most {group} rows"
        )));
    }
    if nrows == 1 && ncols == 1 {
        return Ok(());
    }

    // The cell's extent in twips.
    let grid = t.grid_twips();
    let x0: i64 = grid.iter().take(slot.col).sum();
    let width = span_width(&grid, slot.col, slot.span);

    // Horizontal split: new boundaries, refining the grid where they do not exist yet.
    if ncols > 1 {
        let bounds: Vec<i64> = if ncols == slot.span {
            let mut acc = 0;
            let mut b = vec![0];
            for w in grid.iter().skip(slot.col).take(slot.span) {
                acc += w;
                b.push(acc);
            }
            b
        } else {
            (0..=ncols as i64)
                .map(|i| width * i / ncols as i64)
                .collect()
        };
        for b in bounds.iter().skip(1).take(ncols - 1) {
            let x = x0 + b;
            let mut acc = 0;
            for (g, w) in t.grid_twips().iter().enumerate() {
                if x > acc && x < acc + w {
                    refine_grid(t, g, x - acc);
                    break;
                }
                acc += w;
            }
        }
        let edges = grid_edges(&t.grid_twips());
        let col_of = |x: i64| edges.iter().position(|e| *e == x).unwrap_or(0);
        let first_col = col_of(x0);
        // Replace the cell in every row of its vertical merge by `ncols` cells.
        for r in g0..=g1 {
            let slots_now = all_slots(t);
            let Some(s) = slots_now[r].iter().copied().find(|s| s.col == first_col) else {
                continue;
            };
            let Some(row_m) = row_mut(t, r) else { continue };
            let Some(original) = cell_at(row_m, s.child).cloned() else {
                continue;
            };
            let width_kind = cell_width(&original);
            let mut pieces: Vec<Cell> = Vec::new();
            for i in 0..ncols {
                let from = col_of(x0 + bounds[i]);
                let to = col_of(x0 + bounds[i + 1]);
                let mut c = if i == 0 {
                    original.clone()
                } else {
                    let mut pr = cell_pr(&original).cloned();
                    if let Some(p) = pr.as_mut() {
                        for local in ["cellIns", "cellDel", "cellMerge", "tcPrChange"] {
                            remove_child(p, local);
                        }
                    }
                    blank_cell(&original, pr)
                };
                set_cell_span(&mut c, to.saturating_sub(from).max(1));
                if matches!(width_kind, Some(W::Dxa(_)) | None) {
                    set_cell_width(&mut c, W::Dxa(bounds[i + 1] - bounds[i]));
                }
                pieces.push(c);
            }
            row_m
                .children
                .splice(s.child..=s.child, pieces.into_iter().map(RowChild::Cell));
        }
        if matches!(tbl_w(t), Some(W::Pct(_))) {
            refresh_pct_cells(t);
        }
    }

    // Vertical split.
    if nrows > 1 {
        let slots_now = all_slots(t);
        // The grid columns the cell (or its pieces) covers.
        let piece_cols: Vec<usize> = {
            let edges = grid_edges(&t.grid_twips());
            let first = edges.iter().position(|e| *e == x0).unwrap_or(slot.col);
            let last = edges
                .iter()
                .position(|e| *e == x0 + width)
                .unwrap_or(first + 1);
            (first..last).collect()
        };
        let in_pieces = |s: &Slot| piece_cols.contains(&s.col);
        if group >= 2 {
            // Re-divide the merged rows into `nrows` groups.
            let q = group / nrows;
            let rem = group % nrows;
            let mut at = g0;
            for k in 0..nrows {
                let size = q + usize::from(k < rem);
                for (j, r) in (at..at + size).enumerate() {
                    let vm = if size == 1 {
                        VerticalMerge::None
                    } else if j == 0 {
                        VerticalMerge::Restart
                    } else {
                        VerticalMerge::Continue
                    };
                    for s in slots_now[r].iter().filter(|s| in_pieces(s)) {
                        if let Some(row_m) = row_mut(t, r)
                            && let Some(cell) = cell_mut(row_m, s.child)
                        {
                            set_cell_vmerge(cell, vm);
                        }
                    }
                }
                at += size;
            }
        } else {
            // Insert `nrows - 1` rows below; every other cell of the row spans them.
            let ref_slots = slots_now[row].clone();
            let Some(reference) = row_ref(t, row).cloned() else {
                return Err(bad("row not found"));
            };
            let other_cols: Vec<usize> = ref_slots
                .iter()
                .filter(|s| !in_pieces(s))
                .map(|s| s.col)
                .collect();
            let template = blank_row(&reference, |c, _| {
                if other_cols.contains(&c) {
                    VerticalMerge::Continue
                } else {
                    VerticalMerge::None
                }
            });
            // Cells of the new rows that belong to the pieces are plain; the rest continue.
            let positions = row_children(t);
            let at = positions[row] + 1;
            for k in 0..nrows - 1 {
                t.children.insert(at + k, TableChild::Row(template.clone()));
            }
            if let Some(row_m) = row_mut(t, row) {
                for s in ref_slots.iter().filter(|s| !in_pieces(s)) {
                    if let Some(cell) = cell_mut(row_m, s.child)
                        && cell_vmerge(cell) == VerticalMerge::None
                    {
                        set_cell_vmerge(cell, VerticalMerge::Restart);
                    }
                }
            }
            fix_vmerge(t);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-fit and table-level properties
// ---------------------------------------------------------------------------

fn set_autofit_in(t: &mut Table, mode: AutoFitMode, text_width: i64) {
    match mode {
        AutoFitMode::Fixed => {
            let pr = tbl_pr_mut(t);
            let e = upsert(pr, TBL_PR_ORDER, "tblLayout");
            set_attrs(e, &[("type", "fixed")]);
            set_tbl_w(t, W::Auto);
            // Percentage or automatic cell widths become widths in twips.
            let grid = t.grid_twips();
            let slots = all_slots(t);
            for (r, row_slots_r) in slots.iter().enumerate() {
                for s in row_slots_r {
                    if let Some(row) = row_mut(t, r)
                        && let Some(cell) = cell_mut(row, s.child)
                        && matches!(cell_width(cell), Some(W::Pct(_) | W::Auto) | None)
                    {
                        set_cell_width(cell, W::Dxa(span_width(&grid, s.col, s.span)));
                    }
                }
            }
        }
        AutoFitMode::Content => {
            remove_child(tbl_pr_mut(t), "tblLayout");
            set_tbl_w(t, W::Auto);
            for child in &mut t.children {
                if let TableChild::Row(row) = child {
                    for rc in &mut row.children {
                        if let RowChild::Cell(cell) = rc {
                            set_cell_width(cell, W::Auto);
                        }
                    }
                }
            }
        }
        AutoFitMode::Window => {
            remove_child(tbl_pr_mut(t), "tblLayout");
            set_tbl_w(t, W::Pct(FULL_PCT));
            let grid = t.grid_twips();
            let total: i64 = grid.iter().sum();
            let usable = (text_width - NEW_TABLE_ALLOWANCE).max(1);
            // Word derives the percentages from the widths as they were, then fits the grid.
            refresh_pct_cells(t);
            if total > 0 && total != usable {
                let scaled: Vec<i64> = grid
                    .iter()
                    .scan(0i64, |acc, w| {
                        let before = *acc;
                        *acc += w;
                        let a = (before as f64 * usable as f64 / total as f64).round() as i64;
                        let b = (*acc as f64 * usable as f64 / total as f64).round() as i64;
                        Some(b - a)
                    })
                    .collect();
                set_grid(t, &scaled);
            }
        }
    }
}

fn border_side_name(side: BorderSide) -> &'static str {
    match side {
        BorderSide::Top => "top",
        BorderSide::Left => "left",
        BorderSide::Bottom => "bottom",
        BorderSide::Right => "right",
    }
}

fn opposite(side: BorderSide) -> BorderSide {
    match side {
        BorderSide::Top => BorderSide::Bottom,
        BorderSide::Bottom => BorderSide::Top,
        BorderSide::Left => BorderSide::Right,
        BorderSide::Right => BorderSide::Left,
    }
}

fn write_cell_border(cell: &mut Cell, side: BorderSide, border: Option<&Border>) {
    let owner = cell.head.clone();
    let pr = cell_pr_mut(cell);
    let name = border_side_name(side);
    match border {
        Some(b) => {
            let borders = upsert(pr, TC_PR_ORDER, "tcBorders");
            remove_child(borders, name);
            if name == "left" {
                remove_child(borders, "start");
            }
            if name == "right" {
                remove_child(borders, "end");
            }
            let el = border_element(&owner, name, b);
            let new_rank = BORDER_ORDER
                .iter()
                .position(|n| *n == name)
                .unwrap_or(BORDER_ORDER.len());
            let at = borders
                .children
                .iter()
                .position(|c| rank(BORDER_ORDER, c) > new_rank)
                .unwrap_or(borders.children.len());
            borders.children.insert(at, RawXml::Element(el));
        }
        None => {
            if let Some(RawXml::Element(borders)) = pr
                .children
                .iter_mut()
                .find(|c| matches!(c, RawXml::Element(e) if e.head.is_w("tcBorders")))
            {
                remove_child(borders, name);
            }
            pr.children
                .retain(|c| !matches!(c, RawXml::Element(e) if e.head.is_w("tcBorders") && e.children.is_empty()));
        }
    }
}

fn set_cell_border_in(
    t: &mut Table,
    row: usize,
    col: usize,
    side: BorderSide,
    border: Option<&Border>,
) -> R<()> {
    let slots = all_slots(t);
    let n = slots.len();
    if row >= n {
        return Err(bad(format!("row {row} out of range (table has {n} rows)")));
    }
    let Some(s) = slot_covering(&slots[row], col) else {
        return Err(bad("there is no cell at that position"));
    };
    if let Some(r) = row_mut(t, row)
        && let Some(cell) = cell_mut(r, s.child)
    {
        write_cell_border(cell, side, border);
    }
    // Word writes the same border on the cell that shares the edge.
    let neighbour = match side {
        BorderSide::Top if row > 0 => slot_covering(&slots[row - 1], s.col).map(|x| (row - 1, x)),
        BorderSide::Bottom if row + 1 < n => {
            slot_covering(&slots[row + 1], s.col).map(|x| (row + 1, x))
        }
        BorderSide::Left if s.col > 0 => slot_covering(&slots[row], s.col - 1).map(|x| (row, x)),
        BorderSide::Right => slot_covering(&slots[row], s.col + s.span).map(|x| (row, x)),
        _ => None,
    };
    if let Some((r, x)) = neighbour
        && let Some(row_m) = row_mut(t, r)
        && let Some(cell) = cell_mut(row_m, x.child)
    {
        write_cell_border(cell, opposite(side), border);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Locating tables in the document
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TableLoc {
    path: Path,
    block: usize,
}

fn find_table(blocks: &[Block], target: usize, seen: &mut usize, path: &mut Path) -> Option<usize> {
    for (bi, block) in blocks.iter().enumerate() {
        match block {
            Block::Table(t) => {
                if *seen == target {
                    return Some(bi);
                }
                *seen += 1;
                for (ci, child) in t.children.iter().enumerate() {
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
                        if let Some(found) = find_table(&cell.blocks, target, seen, path) {
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
                    if let Some(found) = find_table(&content.blocks, target, seen, path) {
                        return Some(found);
                    }
                    path.pop();
                }
            }
            _ => {}
        }
    }
    None
}

fn count_tables(blocks: &[Block]) -> usize {
    blocks
        .iter()
        .map(|b| match b {
            Block::Table(t) => {
                1 + t
                    .rows()
                    .flat_map(Row::cells)
                    .map(|c| count_tables(&c.blocks))
                    .sum::<usize>()
            }
            Block::Sdt(s) => s.content().map(|c| count_tables(&c.blocks)).sum(),
            _ => 0,
        })
        .sum()
}

/// Paragraphs before table number `target` in document order.
fn paragraphs_before(
    blocks: &[Block],
    target: usize,
    seen: &mut usize,
    paras: &mut usize,
) -> Option<usize> {
    for block in blocks {
        match block {
            Block::Paragraph(_) => *paras += 1,
            Block::Table(t) => {
                if *seen == target {
                    return Some(*paras);
                }
                *seen += 1;
                for cell in t.rows().flat_map(Row::cells) {
                    if let Some(found) = paragraphs_before(&cell.blocks, target, seen, paras) {
                        return Some(found);
                    }
                }
            }
            Block::Sdt(s) => {
                for content in s.content() {
                    if let Some(found) = paragraphs_before(&content.blocks, target, seen, paras) {
                        return Some(found);
                    }
                }
            }
            Block::Raw(_) => {}
        }
    }
    None
}

/// Index of the table at `(path, block)` in document order.
fn table_index_of(
    blocks: &[Block],
    path: &Path,
    block: usize,
    here: &mut Path,
    n: &mut usize,
) -> Option<usize> {
    for (bi, b) in blocks.iter().enumerate() {
        match b {
            Block::Table(t) => {
                if here == path && bi == block {
                    return Some(*n);
                }
                *n += 1;
                for (ci, child) in t.children.iter().enumerate() {
                    let TableChild::Row(row) = child else {
                        continue;
                    };
                    for (ri, rc) in row.children.iter().enumerate() {
                        let RowChild::Cell(cell) = rc else { continue };
                        here.push(Step::Cell {
                            block: bi,
                            child: ci,
                            cell: ri,
                        });
                        if let Some(f) = table_index_of(&cell.blocks, path, block, here, n) {
                            return Some(f);
                        }
                        here.pop();
                    }
                }
            }
            Block::Sdt(sdt) => {
                for (ci, child) in sdt.children.iter().enumerate() {
                    let SdtChild::Content(content) = child else {
                        continue;
                    };
                    here.push(Step::Sdt {
                        block: bi,
                        child: ci,
                    });
                    if let Some(f) = table_index_of(&content.blocks, path, block, here, n) {
                        return Some(f);
                    }
                    here.pop();
                }
            }
            _ => {}
        }
    }
    None
}

fn text_width_twips(sections: &[crate::Section], para: usize) -> i64 {
    let section = sections
        .iter()
        .find(|s| s.end_paragraph.is_none_or(|e| e >= para))
        .or_else(|| sections.last());
    let Some(s) = section else { return 9360 };
    let page = i64::from(s.page.width_twips);
    let margins = i64::from(s.margins.left_twips)
        + i64::from(s.margins.right_twips)
        + i64::from(s.margins.gutter_twips);
    let mut width = page - margins;
    if s.columns.count > 1 && s.columns.equal_width {
        let n = i64::from(s.columns.count);
        width = (width - i64::from(s.columns.space_twips) * (n - 1)) / n;
    }
    width.max(720)
}

// ---------------------------------------------------------------------------
// Document API
// ---------------------------------------------------------------------------

impl Document {
    /// Number of tables, nested ones included, in document order.
    pub fn table_count(&self) -> usize {
        count_tables(&self.body().blocks)
    }

    /// Table number `index` in document order (a table before the tables nested in it).
    pub fn table(&self, index: usize) -> Option<&Table> {
        let loc = self.table_loc(index).ok()?;
        match container(self.body(), &loc.path)?.get(loc.block)? {
            Block::Table(t) => Some(t),
            _ => None,
        }
    }

    fn table_loc(&self, index: usize) -> R<TableLoc> {
        let mut path = Vec::new();
        let mut seen = 0;
        let count = self.table_count();
        find_table(&self.body().blocks, index, &mut seen, &mut path)
            .map(|block| TableLoc { path, block })
            .ok_or_else(|| bad(format!("table {index} out of range (document has {count})")))
    }

    /// Width of the text area at paragraph `para`, in points (page width minus margins and
    /// gutter; one column when the section has several).
    pub fn text_width_pt(&self, para: usize) -> f64 {
        text_width_twips(&self.sections(), para) as f64 / 20.0
    }

    fn table_text_width(&self, index: usize) -> i64 {
        let mut seen = 0;
        let mut paras = 0;
        let before =
            paragraphs_before(&self.body().blocks, index, &mut seen, &mut paras).unwrap_or(0);
        text_width_twips(&self.sections(), before)
    }

    /// Runs `edit` on a copy of table `index` and stores it when `edit` succeeds.
    fn with_table<T>(&mut self, index: usize, edit: impl FnOnce(&mut Table) -> R<T>) -> R<T> {
        let loc = self.table_loc(index)?;
        let blocks =
            container_mut(self.body_mut(), &loc.path).ok_or_else(|| bad("container not found"))?;
        let Some(Block::Table(table)) = blocks.get_mut(loc.block) else {
            return Err(bad("table not found"));
        };
        let mut copy = table.clone();
        let out = edit(&mut copy)?;
        *table = copy;
        self.edited();
        Ok(out)
    }

    /// Like [`Self::with_table`] with a revision counter when track changes is on.
    fn with_table_tracked<T>(
        &mut self,
        index: usize,
        edit: impl FnOnce(&mut Table, Option<&mut Rev<'_>>) -> R<T>,
    ) -> R<T> {
        let Some(track) = self.tracking.clone() else {
            return self.with_table(index, |t| edit(t, None));
        };
        let first = self.alloc_annotation_id();
        let mut rev = Rev {
            track: &track,
            next: first,
        };
        let out = self.with_table(index, |t| edit(t, Some(&mut rev)));
        self.next_annotation_id = Some(rev.next);
        out
    }

    /// Inserts a table before the paragraph at `pos`, splitting that paragraph first when `pos`
    /// is not at its start (as Word does). The new table has Word's defaults: "Table Grid"
    /// style (direct single borders when the document has no such style), automatic table
    /// width, the default `w:tblLook`, and columns that share the text width less 10 twips;
    /// `mode` picks fixed widths (twips), automatic widths or percentage widths. Returns the
    /// table's index in document order.
    pub fn insert_table(
        &mut self,
        pos: DocPos,
        rows: usize,
        cols: usize,
        text_width_pt: f64,
        mode: AutoFitMode,
    ) -> R<usize> {
        self.check_pos(pos)?;
        if rows == 0 || cols == 0 {
            return Err(bad("a table needs at least one row and one column"));
        }
        if cols > MAX_COLUMNS {
            return Err(bad("a table cannot have more than 63 columns"));
        }
        if rows > 32_767 {
            return Err(bad("a table cannot have more than 32767 rows"));
        }
        if !text_width_pt.is_finite() || text_width_pt <= 0.0 {
            return Err(bad("the text width must be positive"));
        }
        let text_width = (text_width_pt * 20.0).round() as i64;
        let target = if pos.offset > 0 {
            self.split_paragraph(pos)?;
            pos.para + 1
        } else {
            pos.para
        };
        let (path, bi) =
            locate_paragraph(self.body(), target).ok_or_else(|| bad("paragraph not found"))?;
        let grid_style = self.styles().is_some_and(|s| s.has_style("TableGrid"));
        let owner = self.body().head.clone();
        let table = match self.tracking.clone() {
            Some(track) => {
                let first = self.alloc_annotation_id();
                let mut rev = Rev {
                    track: &track,
                    next: first,
                };
                let table = build_table(
                    &owner,
                    rows,
                    cols,
                    text_width,
                    mode,
                    grid_style,
                    Some(&mut rev),
                );
                self.next_annotation_id = Some(rev.next);
                table
            }
            None => build_table(&owner, rows, cols, text_width, mode, grid_style, None),
        };
        let blocks =
            container_mut(self.body_mut(), &path).ok_or_else(|| bad("container not found"))?;
        blocks.insert(bi, Block::Table(table));
        self.edited();
        table_index_of(&self.body().blocks, &path, bi, &mut Vec::new(), &mut 0)
            .ok_or_else(|| bad("the new table was not found"))
    }

    /// Inserts `count` rows above or below row `row`. The new rows copy the row's properties,
    /// cell properties and paragraph formatting (without text); inside a vertical merge they
    /// continue it. Returns the index of the first new row.
    pub fn insert_rows(
        &mut self,
        table: usize,
        row: usize,
        side: RowSide,
        count: usize,
    ) -> R<usize> {
        self.with_table_tracked(table, |t, rev| insert_rows_in(t, row, side, count, rev))
    }

    /// Inserts `count` columns left or right of grid column `col`. A new cell copies the width
    /// of the cell at `col` and the paragraph formatting of its first paragraph; the table
    /// grows (Word does not shrink the others). Column edits are not tracked.
    pub fn insert_columns(
        &mut self,
        table: usize,
        col: usize,
        side: ColumnSide,
        count: usize,
    ) -> R<()> {
        let text_width = self.table_text_width(table);
        self.with_table(table, |t| {
            for _ in 0..count {
                insert_column_in(t, col, side, text_width)?;
            }
            Ok(())
        })
    }

    /// Deletes rows. With track changes on the rows are marked deleted (`w:trPr/w:del`, deleted
    /// runs and paragraph marks); a row the same author inserted while tracking is removed.
    /// Deleting every row deletes the table.
    pub fn delete_rows(&mut self, table: usize, rows: Range<usize>) -> R<()> {
        let n = self
            .table(table)
            .map(|t| t.rows().count())
            .ok_or_else(|| bad(format!("table {table} out of range")))?;
        if rows.start == 0 && rows.end == n && n > 0 {
            return self.delete_table(table);
        }
        self.with_table_tracked(table, |t, rev| delete_rows_in(t, rows, rev))
    }

    /// Deletes grid columns (cells inside the range go, cells spanning it get narrower). Deleting
    /// every column deletes the table. Column edits are not tracked.
    pub fn delete_columns(&mut self, table: usize, cols: Range<usize>) -> R<()> {
        let n = self
            .table(table)
            .map(|t| t.grid_twips().len())
            .ok_or_else(|| bad(format!("table {table} out of range")))?;
        if cols.start == 0 && cols.end == n && n > 0 {
            return self.delete_table(table);
        }
        let text_width = self.table_text_width(table);
        self.with_table(table, |t| delete_columns_in(t, cols, text_width))
    }

    /// Deletes a table. With track changes on, every row is marked deleted instead (a table
    /// the same author inserted while tracking is removed).
    pub fn delete_table(&mut self, table: usize) -> R<()> {
        let all_own = match (&self.tracking, self.table(table)) {
            (Some(track), Some(t)) => t.rows().all(|r| {
                row_revision(r, "ins")
                    .is_some_and(|i| i.head.attr_w("author") == Some(track.author.as_str()))
            }),
            _ => false,
        };
        if self.tracking.is_some() && !all_own {
            let n = self
                .table(table)
                .map(|t| t.rows().count())
                .ok_or_else(|| bad(format!("table {table} out of range")))?;
            return self.with_table_tracked(table, |t, rev| delete_rows_in(t, 0..n, rev));
        }
        let loc = self.table_loc(table)?;
        let owner = self.body().head.clone();
        let blocks =
            container_mut(self.body_mut(), &loc.path).ok_or_else(|| bad("container not found"))?;
        blocks.remove(loc.block);
        // A table cell must end in a paragraph.
        if matches!(loc.path.last(), Some(Step::Cell { .. }))
            && !matches!(blocks.last(), Some(Block::Paragraph(_)))
        {
            blocks.push(Block::Paragraph(blank_paragraph(None, &owner)));
        }
        self.edited();
        Ok(())
    }

    /// Merges the cells in the rectangle `from`..=`to` (each `(row, grid column)`, either
    /// order) into one: `w:gridSpan` for columns, `w:vMerge` for rows. The rectangle grows to
    /// whole cells. The merged cell keeps the first cell's properties; the paragraphs of the
    /// other non-empty cells follow its own.
    pub fn merge_cells(&mut self, table: usize, from: (usize, usize), to: (usize, usize)) -> R<()> {
        self.with_table(table, |t| merge_cells_in(t, from, to))
    }

    /// Splits the cell at `(row, col)` into `rows` x `cols` cells. Columns divide the cell's
    /// width equally (Word rounds down cumulatively) and refine the grid; rows add rows below
    /// (the other cells of the row merge down over them) or, for a vertically merged cell,
    /// re-divide its rows.
    pub fn split_cell(
        &mut self,
        table: usize,
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    ) -> R<()> {
        self.with_table(table, |t| split_cell_in(t, row, col, rows, cols))
    }

    /// Sets the width of grid column `col`; `adjust` says what absorbs the difference.
    pub fn set_column_width(
        &mut self,
        table: usize,
        col: usize,
        width_pt: f64,
        adjust: ColumnAdjust,
    ) -> R<()> {
        if !width_pt.is_finite() {
            return Err(bad("the width must be a number"));
        }
        let width = (width_pt * 20.0).round() as i64;
        let text_width = self.table_text_width(table);
        self.with_table(table, |t| {
            set_column_width_in(t, col, width, adjust, text_width)
        })
    }

    /// Sets (`Some`) or clears (`None` or `HeightRule::Auto`) the height of a row.
    pub fn set_row_height(
        &mut self,
        table: usize,
        row: usize,
        height: Option<(f64, HeightRule)>,
    ) -> R<()> {
        self.with_table(table, |t| {
            let r = row_mut(t, row).ok_or_else(|| bad(format!("row {row} out of range")))?;
            match height {
                None | Some((_, HeightRule::Auto)) => {
                    remove_child(row_pr_mut(r), "trHeight");
                }
                Some((pt, rule)) => {
                    let e = upsert(row_pr_mut(r), TR_PR_ORDER, "trHeight");
                    e.head.attrs.clear();
                    set_attrs(e, &[("val", &((pt * 20.0).round() as i64).to_string())]);
                    if rule == HeightRule::Exact {
                        set_attrs(e, &[("hRule", "exact")]);
                    }
                }
            }
            tidy_row_pr(r);
            Ok(())
        })
    }

    /// Repeats the row at the top of every page the table continues on (`w:tblHeader`).
    pub fn set_row_header(&mut self, table: usize, row: usize, on: bool) -> R<()> {
        self.set_row_flag(table, row, "tblHeader", on)
    }

    /// Stops the row breaking across pages (`w:cantSplit`).
    pub fn set_row_cant_split(&mut self, table: usize, row: usize, on: bool) -> R<()> {
        self.set_row_flag(table, row, "cantSplit", on)
    }

    fn set_row_flag(&mut self, table: usize, row: usize, local: &str, on: bool) -> R<()> {
        self.with_table(table, |t| {
            let r = row_mut(t, row).ok_or_else(|| bad(format!("row {row} out of range")))?;
            if on {
                upsert(row_pr_mut(r), TR_PR_ORDER, local);
            } else {
                remove_child(row_pr_mut(r), local);
            }
            tidy_row_pr(r);
            Ok(())
        })
    }

    /// Sets or clears (`None`) the shading of the cell at `(row, col)`.
    pub fn set_cell_shading(
        &mut self,
        table: usize,
        row: usize,
        col: usize,
        shading: Option<Shading>,
    ) -> R<()> {
        self.with_table(table, |t| {
            let slots = all_slots(t);
            let s = slots
                .get(row)
                .and_then(|r| slot_covering(r, col))
                .ok_or_else(|| bad("there is no cell at that position"))?;
            let cell = row_mut(t, row)
                .and_then(|r| cell_mut(r, s.child))
                .ok_or_else(|| bad("cell not found"))?;
            let pr = cell_pr_mut(cell);
            match shading {
                None => {
                    remove_child(pr, "shd");
                }
                Some(sh) => {
                    let hex = |c: Option<[u8; 3]>| {
                        c.map_or_else(
                            || "auto".to_owned(),
                            |c| format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]),
                        )
                    };
                    let e = upsert(pr, TC_PR_ORDER, "shd");
                    e.head.attrs.clear();
                    set_attrs(
                        e,
                        &[
                            ("val", &sh.pattern),
                            ("color", &hex(sh.color)),
                            ("fill", &hex(sh.fill)),
                        ],
                    );
                }
            }
            Ok(())
        })
    }

    /// Sets (`Some`) or removes (`None`) one border of the cell at `(row, col)`; the cell that
    /// shares the edge gets the same border on its side, as Word writes it. A border with style
    /// `"nil"` switches the line off explicitly.
    pub fn set_cell_border(
        &mut self,
        table: usize,
        row: usize,
        col: usize,
        side: BorderSide,
        border: Option<&Border>,
    ) -> R<()> {
        self.with_table(table, |t| set_cell_border_in(t, row, col, side, border))
    }

    /// Sets the table borders (`w:tblBorders`) the set states; sides that are `None` stay as
    /// they were.
    pub fn set_table_borders(&mut self, table: usize, borders: &BorderSet) -> R<()> {
        self.with_table(table, |t| {
            let owner = t.head.clone();
            let pr = tbl_pr_mut(t);
            let element = upsert(pr, TBL_PR_ORDER, "tblBorders");
            for (name, b) in [
                ("top", &borders.top),
                ("left", &borders.left),
                ("bottom", &borders.bottom),
                ("right", &borders.right),
                ("insideH", &borders.inside_h),
                ("insideV", &borders.inside_v),
            ] {
                let Some(b) = b else { continue };
                remove_child(element, name);
                let new_rank = BORDER_ORDER.iter().position(|n| *n == name).unwrap_or(0);
                let at = element
                    .children
                    .iter()
                    .position(|c| rank(BORDER_ORDER, c) > new_rank)
                    .unwrap_or(element.children.len());
                element
                    .children
                    .insert(at, RawXml::Element(border_element(&owner, name, b)));
            }
            Ok(())
        })
    }

    /// Aligns the table between the margins (`w:jc`; Word also writes it on every row). Left
    /// removes it.
    pub fn set_table_alignment(
        &mut self,
        table: usize,
        alignment: Option<TableJustification>,
    ) -> R<()> {
        self.with_table(table, |t| {
            let name = match alignment {
                Some(TableJustification::Center) => Some("center"),
                Some(TableJustification::Right) => Some("right"),
                Some(TableJustification::Left) | None => None,
            };
            let pr = tbl_pr_mut(t);
            match name {
                Some(v) => {
                    let e = upsert(pr, TBL_PR_ORDER, "jc");
                    set_attrs(e, &[("val", v)]);
                }
                None => {
                    remove_child(pr, "jc");
                }
            }
            for child in &mut t.children {
                if let TableChild::Row(r) = child {
                    match name {
                        Some(v) => {
                            let e = upsert(row_pr_mut(r), TR_PR_ORDER, "jc");
                            set_attrs(e, &[("val", v)]);
                        }
                        None => {
                            remove_child(row_pr_mut(r), "jc");
                            tidy_row_pr(r);
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// Sets (`Some`) or clears the left indent of the table in points (`w:tblInd`).
    pub fn set_table_indent(&mut self, table: usize, indent_pt: Option<f64>) -> R<()> {
        self.with_table(table, |t| {
            let pr = tbl_pr_mut(t);
            match indent_pt {
                Some(pt) => {
                    let e = upsert(pr, TBL_PR_ORDER, "tblInd");
                    write_w(e, W::Dxa((pt * 20.0).round() as i64));
                }
                None => {
                    remove_child(pr, "tblInd");
                }
            }
            Ok(())
        })
    }

    /// Sets or clears (`None`) the table style. The style must exist in the document.
    pub fn set_table_style(&mut self, table: usize, style: Option<&str>) -> R<()> {
        if let Some(id) = style
            && !self.styles().is_some_and(|s| s.has_style(id))
        {
            return Err(DocxError::UnknownStyle(id.to_owned()));
        }
        self.with_table(table, |t| {
            let pr = tbl_pr_mut(t);
            match style {
                Some(id) => {
                    let e = upsert(pr, TBL_PR_ORDER, "tblStyle");
                    set_attrs(e, &[("val", id)]);
                }
                None => {
                    remove_child(pr, "tblStyle");
                }
            }
            Ok(())
        })
    }

    /// Sets the preferred table width (`w:tblW`). The grid is left alone.
    pub fn set_table_width(&mut self, table: usize, width: TableWidth) -> R<()> {
        self.with_table(table, |t| {
            let w = match width {
                TableWidth::Auto => W::Auto,
                TableWidth::Nil => W::Nil,
                TableWidth::Points(p) => W::Dxa((p * 20.0).round() as i64),
                TableWidth::Percent(p) => W::Pct((p * 50.0).round() as i64),
            };
            set_tbl_w(t, w);
            Ok(())
        })
    }

    /// Sets the layout algorithm (`w:tblLayout`): fixed, or automatic (the element is removed).
    pub fn set_table_layout(&mut self, table: usize, layout: TableLayoutType) -> R<()> {
        self.with_table(table, |t| {
            let pr = tbl_pr_mut(t);
            match layout {
                TableLayoutType::Fixed => {
                    let e = upsert(pr, TBL_PR_ORDER, "tblLayout");
                    set_attrs(e, &[("type", "fixed")]);
                }
                TableLayoutType::AutoFit => {
                    remove_child(pr, "tblLayout");
                }
            }
            Ok(())
        })
    }

    /// Applies one of Word's auto-fit behaviours: fixed column width (fixed layout, automatic
    /// table width), auto-fit to contents (automatic widths), or auto-fit to window (100 %
    /// table width, percentage cell widths, grid scaled to the text area at the table).
    pub fn set_table_autofit(&mut self, table: usize, mode: AutoFitMode) -> R<()> {
        let text_width = self.table_text_width(table);
        self.with_table(table, |t| {
            set_autofit_in(t, mode, text_width);
            Ok(())
        })
    }

    /// Converts paragraphs `first..=last` (one container, only paragraphs) into a table.
    /// Rows come from paragraphs and cells from `separator` (for `Paragraph`, `columns` is
    /// required and every paragraph is a cell); the table is made like [`Self::insert_table`].
    /// Not available with track changes on. Returns the table's index.
    pub fn convert_text_to_table(
        &mut self,
        first: usize,
        last: usize,
        separator: TextSeparator,
        columns: Option<usize>,
        text_width_pt: f64,
        mode: AutoFitMode,
    ) -> R<usize> {
        if self.tracking.is_some() {
            return Err(bad(
                "converting text to a table is not tracked; turn track changes off",
            ));
        }
        if first > last {
            return Err(bad("the paragraph range is reversed"));
        }
        if !text_width_pt.is_finite() || text_width_pt <= 0.0 {
            return Err(bad("the text width must be positive"));
        }
        self.check_pos(DocPos::new(first, 0))?;
        self.check_pos(DocPos::new(last, 0))?;
        let (path, bi_first) =
            locate_paragraph(self.body(), first).ok_or_else(|| bad("paragraph not found"))?;
        let (path_last, bi_last) =
            locate_paragraph(self.body(), last).ok_or_else(|| bad("paragraph not found"))?;
        if path != path_last {
            return Err(bad("the paragraphs are not in one container"));
        }
        {
            let blocks = container(self.body(), &path).ok_or_else(|| bad("container not found"))?;
            if !blocks[bi_first..=bi_last]
                .iter()
                .all(|b| matches!(b, Block::Paragraph(_)))
            {
                return Err(bad("the range holds something other than paragraphs"));
            }
        }
        let sep_char = match separator {
            TextSeparator::Tab => Some('\t'),
            TextSeparator::Comma => Some(','),
            TextSeparator::Char(c) => Some(c),
            TextSeparator::Paragraph => None,
        };
        // Split on the separator, last paragraph first so earlier indices stay valid.
        let mut counts = Vec::new();
        for q in first..=last {
            let text = self.paragraph_text(q).unwrap_or_default();
            let n = sep_char.map_or(1, |c| text.chars().filter(|x| *x == c).count() + 1);
            counts.push(n);
        }
        if let Some(c) = sep_char {
            for q in (first..=last).rev() {
                let text = self.paragraph_text(q).unwrap_or_default();
                let offsets: Vec<usize> = text
                    .chars()
                    .enumerate()
                    .filter_map(|(i, x)| (x == c).then_some(i))
                    .collect();
                for off in offsets.into_iter().rev() {
                    self.delete_range(DocPos::new(q, off), DocPos::new(q, off + 1))?;
                    self.split_paragraph(DocPos::new(q, off))?;
                }
            }
        }
        let total: usize = counts.iter().sum();
        let (rows, cols, shape): (usize, usize, Vec<usize>) = match separator {
            TextSeparator::Paragraph => {
                let cols =
                    columns.ok_or_else(|| bad("a paragraph separator needs a column count"))?;
                if cols == 0 {
                    return Err(bad("a table needs at least one column"));
                }
                let rows = total.div_ceil(cols);
                (rows, cols, vec![cols; rows])
            }
            _ => {
                let cols = columns.unwrap_or_else(|| counts.iter().copied().max().unwrap_or(1));
                (counts.len(), cols.max(1), counts.clone())
            }
        };
        if cols > MAX_COLUMNS {
            return Err(bad("a table cannot have more than 63 columns"));
        }
        // Take the (now split) paragraphs out of the container and wrap them in cells.
        let last_new = bi_first + total - 1;
        let blocks =
            container_mut(self.body_mut(), &path).ok_or_else(|| bad("container not found"))?;
        let mut paragraphs: Vec<Paragraph> = blocks
            .drain(bi_first..=last_new)
            .filter_map(|b| match b {
                Block::Paragraph(p) => Some(p),
                _ => None,
            })
            .collect();
        let owner = paragraphs
            .first()
            .map(|p| p.head.clone())
            .ok_or_else(|| bad("no paragraphs to convert"))?;
        let grid_style = self.styles().is_some_and(|s| s.has_style("TableGrid"));
        let text_width = (text_width_pt * 20.0).round() as i64;
        let mut table = build_table(&owner, rows, cols, text_width, mode, grid_style, None);
        let mut it = paragraphs.drain(..);
        let mut r = 0;
        for child in &mut table.children {
            let TableChild::Row(row) = child else {
                continue;
            };
            let in_row = shape.get(r).copied().unwrap_or(0);
            let mut k = 0;
            for rc in &mut row.children {
                let RowChild::Cell(cell) = rc else { continue };
                if k < in_row
                    && let Some(p) = it.next()
                {
                    cell.blocks.retain(|b| matches!(b, Block::Raw(_)));
                    cell.blocks.push(Block::Paragraph(p));
                }
                k += 1;
            }
            r += 1;
        }
        let blocks =
            container_mut(self.body_mut(), &path).ok_or_else(|| bad("container not found"))?;
        blocks.insert(bi_first, Block::Table(table));
        self.edited();
        table_index_of(
            &self.body().blocks,
            &path,
            bi_first,
            &mut Vec::new(),
            &mut 0,
        )
        .ok_or_else(|| bad("the new table was not found"))
    }

    /// Replaces a table with its text: each row becomes paragraphs, with `separator` between
    /// the cells (a paragraph break for `Paragraph`; the last paragraph of one cell and the
    /// first of the next share a line otherwise). Vertically merged continuations are empty
    /// cells. Not available with track changes on or for tables holding tables.
    pub fn convert_table_to_text(&mut self, table: usize, separator: TextSeparator) -> R<()> {
        if self.tracking.is_some() {
            return Err(bad(
                "converting a table to text is not tracked; turn track changes off",
            ));
        }
        let loc = self.table_loc(table)?;
        let t = self
            .table(table)
            .cloned()
            .ok_or_else(|| bad("table not found"))?;
        let sep_text = match separator {
            TextSeparator::Tab => Some("\t".to_owned()),
            TextSeparator::Comma => Some(", ".to_owned()),
            TextSeparator::Char(c) => Some(c.to_string()),
            TextSeparator::Paragraph => None,
        };
        let mut out: Vec<Paragraph> = Vec::new();
        for row in t.rows() {
            let mut line: Vec<Paragraph> = Vec::new();
            for (k, cell) in row.cells().enumerate() {
                if cell.blocks.iter().any(|b| matches!(b, Block::Table(_))) {
                    return Err(bad("a cell holds a table; convert that one first"));
                }
                let mut paras: Vec<Paragraph> = cell
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Paragraph(p) => Some(p.clone()),
                        _ => None,
                    })
                    .collect();
                if paras.is_empty() {
                    paras.push(blank_paragraph(None, &cell.head));
                }
                match &sep_text {
                    Some(text) if k > 0 => {
                        let mut joined = line
                            .pop()
                            .unwrap_or_else(|| blank_paragraph(None, &cell.head));
                        let pieces = crate::edit::pieces_of(text)?;
                        let mut run = joined.new_run();
                        run.content = run.items_for(pieces);
                        joined.content.push(Inline::Run(run));
                        let mut rest = paras.into_iter();
                        if let Some(first) = rest.next() {
                            joined.content.extend(first.content);
                        }
                        line.push(joined);
                        line.extend(rest);
                    }
                    _ => line.extend(paras),
                }
            }
            out.extend(line);
        }
        if out.is_empty() {
            out.push(blank_paragraph(None, &t.head));
        }
        let blocks =
            container_mut(self.body_mut(), &loc.path).ok_or_else(|| bad("container not found"))?;
        blocks.splice(loc.block..=loc.block, out.into_iter().map(Block::Paragraph));
        self.edited();
        Ok(())
    }

    /// The canonical property dump of the body-level paragraphs and tables, the format
    /// `tools/office/table-edit-oracle.ps1` writes for Word's output (see
    /// [`Table::property_dump`]). Used to compare edits against Word property by property.
    pub fn table_dump(&self) -> String {
        dump_blocks(&self.body().blocks)
    }
}

// ---------------------------------------------------------------------------
// Property dump (the format of the Word oracle)
// ---------------------------------------------------------------------------

fn escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if (' '..='~').contains(&c) => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
        }
    }
    out
}

/// `name(attr=value,...){children}`: attributes in the WordprocessingML namespace sorted by
/// local name, without `id`, `author`, `date` and `rsid*`.
fn node_str(e: &XmlElement) -> String {
    let mut attrs: Vec<(String, &str)> = e
        .head
        .attrs
        .iter()
        .filter(|a| is_w_ns(a.ns.as_deref()))
        .map(|a| (a.local_name().to_owned(), a.value.as_str()))
        .filter(|(n, _)| !matches!(n.as_str(), "id" | "author" | "date") && !n.starts_with("rsid"))
        .collect();
    attrs.sort();
    let mut s = e.head.local_name().to_owned();
    if !attrs.is_empty() {
        s.push('(');
        s.push_str(
            &attrs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(","),
        );
        s.push(')');
    }
    let kids: Vec<String> = e
        .children
        .iter()
        .filter_map(RawXml::as_element)
        .map(node_str)
        .collect();
    if !kids.is_empty() {
        s.push('{');
        s.push_str(&kids.join(" "));
        s.push('}');
    }
    s
}

fn prop_str(e: Option<&XmlElement>) -> String {
    e.map_or_else(|| "-".to_owned(), node_str)
}

fn dump_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut ti = 0;
    for block in blocks {
        match block {
            Block::Paragraph(p) => {
                out.push_str(&format!("p\t{}\n", escape(&p.text())));
            }
            Block::Table(t) => {
                out.push_str(&t.property_dump(ti));
                ti += 1;
            }
            _ => {}
        }
    }
    out
}

impl Table {
    /// Lines describing the table's structure and properties: `tbl` (grid in twips, row count,
    /// `w:tblPr`), then per row `row` (`w:trPr`, `w:tblPrEx`) and per cell `cell` (`w:tcPr`,
    /// the first paragraph's `w:pPr`, the paragraphs' text). The same format
    /// `tools/office/table-edit-oracle.ps1` writes for Word's saved output.
    pub fn property_dump(&self, index: usize) -> String {
        let grid = self
            .grid_twips()
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let rows: Vec<&Row> = self.rows().collect();
        let mut out = format!(
            "tbl\t{index}\tgrid={grid}\trows={}\tpr={}\n",
            rows.len(),
            prop_str(tbl_pr(self))
        );
        for (ri, row) in rows.iter().enumerate() {
            let ex = row.children.iter().find_map(|c| match c {
                RowChild::Raw(RawXml::Element(e)) if e.head.is_w("tblPrEx") => Some(e),
                _ => None,
            });
            out.push_str(&format!(
                "row\t{ri}\tpr={}\tex={}\n",
                prop_str(row_pr(row)),
                prop_str(ex)
            ));
            for (ci, cell) in row.cells().enumerate() {
                let ppr = first_paragraph(cell).and_then(|p| p.props.as_ref());
                let text = cell
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Paragraph(p) => Some(p.text()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\u{b6}");
                out.push_str(&format!(
                    "cell\t{ri}\t{ci}\tpr={}\tppr={}\ttxt={}\n",
                    prop_str(cell_pr(cell)),
                    prop_str(ppr),
                    escape(&text)
                ));
            }
        }
        out
    }
}
