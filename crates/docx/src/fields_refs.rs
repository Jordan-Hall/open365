//! REF, PAGEREF, NOTEREF, SEQ, STYLEREF, IF, MERGEFIELD and HYPERLINK.
//!
//! The rules here were measured against Word (`tools/office/fields-oracle.ps1`, set `refs`);
//! see `docs/architecture/docx-fields.md`.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::document::Document;
use crate::fields::{FieldKind, GeneralFormat, Instruction, SkipReason};
use crate::fields_eval::{Evaluator, Outcome, Value, View, parse_number};
use crate::fields_formula::Num;
use crate::fields_scan::{Loc, RawField, Step, slots};
use crate::fields_snap::{NumInfo, Snapshot, format_number};
use crate::model::{Block, Inline, Paragraph, RowChild, TableChild};
use crate::ranges::DocPos;
use crate::xml::RawXml;

const REF_NOT_FOUND: &str = "Error! Reference source not found.";
const BOOKMARK_UNDEFINED: &str = "Error! Bookmark not defined.";

/// Character position of a marker: paragraph index and offset in `Paragraph::text()`.
pub(crate) fn position_of(doc: &Document, loc: &Loc) -> DocPos {
    let offset = doc
        .paragraph(loc.para)
        .map_or(0, |p| offset_in(p, &loc.path, loc.item));
    DocPos {
        para: loc.para,
        offset,
    }
}

fn offset_in(p: &Paragraph, path: &[usize], item: usize) -> usize {
    fn walk(content: &[Inline], path: &[usize], item: usize, acc: &mut usize) -> bool {
        let Some((&first, rest)) = path.split_first() else {
            return true;
        };
        for inline in content.iter().take(first) {
            *acc += inline.char_len();
        }
        match content.get(first) {
            Some(Inline::Group(g)) => walk(&g.content, rest, item, acc),
            Some(Inline::Run(r)) => {
                for c in r.content.iter().take(item) {
                    *acc += c.char_len();
                }
                true
            }
            _ => true,
        }
    }
    let mut acc = 0usize;
    walk(&p.content, path, item, &mut acc);
    acc
}

pub(crate) fn evaluate(
    ev: &Evaluator<'_>,
    view: &View<'_>,
    index: usize,
    inst: &Instruction,
    lang: &str,
) -> Outcome {
    match inst.kind {
        FieldKind::Ref => reference(ev, view, index, inst, lang),
        FieldKind::PageRef => page_ref(ev, view, inst, lang),
        FieldKind::NoteRef => note_ref(ev, view, index, inst, lang),
        FieldKind::Seq => seq(ev, view, index, inst, lang),
        FieldKind::StyleRef => style_ref(ev, view, index, inst, lang),
        FieldKind::If => conditional(ev, view, inst, lang),
        FieldKind::MergeField => merge_field(ev, inst, lang),
        FieldKind::Hyperlink | FieldKind::Tc => Outcome::Keep,
        FieldKind::Toc => crate::fields_toc::plan(ev, view, index, inst),
        _ => Outcome::Skip(SkipReason::UnsupportedType),
    }
}

fn text_outcome(ev: &Evaluator<'_>, inst: &Instruction, value: Value, lang: &str) -> Outcome {
    let merge = inst.general_formats().contains(&GeneralFormat::MergeFormat);
    Outcome::Text {
        text: ev.render(inst, value, lang, false),
        merge,
    }
}

fn plain(text: &str) -> Outcome {
    Outcome::Text {
        text: text.to_owned(),
        merge: false,
    }
}

fn int_value(n: i64) -> Value {
    #[allow(clippy::cast_precision_loss)]
    Value::Num(Num {
        value: n as f64,
        scale: 0,
    })
}

fn trim_periods(text: &str) -> &str {
    text.trim_end_matches('.')
}

/// Where the field sits.
fn field_pos(view: &View<'_>, index: usize) -> DocPos {
    position_of(view.doc, &view.raws[index].begin)
}

// ---------------------------------------------------------------------------
// Paragraph numbers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum NumberKind {
    /// `\n`: the paragraph's own number.
    Own,
    /// `\r`: relative to the field's own numbering.
    Relative,
    /// `\w`: all levels.
    Full,
}

fn number_kind(inst: &Instruction) -> Option<NumberKind> {
    // The last of the three wins, like Word's switch parsing.
    inst.switches
        .iter()
        .rev()
        .find_map(|s| match s.name.as_str() {
            "n" => Some(NumberKind::Own),
            "r" => Some(NumberKind::Relative),
            "w" => Some(NumberKind::Full),
            _ => None,
        })
}

fn own_label(snap: &Snapshot, para: usize) -> Option<&str> {
    snap.paras.get(para).and_then(|p| p.label.as_deref())
}

fn paragraph_number(snap: &Snapshot, para: usize, kind: NumberKind, from: DocPos) -> String {
    let Some(label) = own_label(snap, para) else {
        return "0".to_owned();
    };
    let info = snap.paras[para].num.as_ref();
    let out = match (kind, info) {
        (NumberKind::Own, _) | (_, None) => label.to_owned(),
        (NumberKind::Full, Some(n)) => full_context(n, label),
        (NumberKind::Relative, Some(n)) => {
            let mine = snap.paras.get(from.para).and_then(|p| p.num.as_ref());
            match mine {
                Some(m) if m.list == n.list && !n.self_contained => {
                    let common = n
                        .levels
                        .iter()
                        .zip(&m.levels)
                        .take_while(|(a, b)| a == b)
                        .count()
                        .min(n.ilvl);
                    n.levels[common..].concat()
                }
                _ => full_context(n, label),
            }
        }
    };
    trim_periods(&out).to_owned()
}

fn full_context(info: &NumInfo, label: &str) -> String {
    if info.self_contained {
        label.to_owned()
    } else {
        info.levels.concat()
    }
}

fn above_below(target: DocPos, from: DocPos) -> &'static str {
    if target <= from { "above" } else { "below" }
}

// ---------------------------------------------------------------------------
// REF
// ---------------------------------------------------------------------------

fn reference(
    ev: &Evaluator<'_>,
    view: &View<'_>,
    index: usize,
    inst: &Instruction,
    lang: &str,
) -> Outcome {
    let Some(name) = inst.first_arg() else {
        return plain("Error! No bookmark name given.");
    };
    let snap = view.snap();
    let Some(bookmark) = snap.bookmark(name) else {
        return plain(REF_NOT_FOUND);
    };
    let from = field_pos(view, index);
    let relative = inst.has_switch("p");
    let kind = number_kind(inst);
    let text = match kind {
        Some(kind) => {
            let number = paragraph_number(snap, bookmark.start.para, kind, from);
            if relative {
                format!("{number} {}", above_below(bookmark.start, from))
            } else {
                number
            }
        }
        None if relative => above_below(bookmark.start, from).to_owned(),
        None => bookmark.text.clone(),
    };
    text_outcome(ev, inst, Value::Text(text), lang)
}

// ---------------------------------------------------------------------------
// PAGEREF and NOTEREF
// ---------------------------------------------------------------------------

fn page_ref(ev: &Evaluator<'_>, view: &View<'_>, inst: &Instruction, lang: &str) -> Outcome {
    let Some(name) = inst.first_arg() else {
        return plain("Error! No bookmark name given.");
    };
    let Some(bookmark) = view.snap().bookmark(name) else {
        return plain(BOOKMARK_UNDEFINED);
    };
    let Some(info) = view.page_at(ev.ctx, bookmark.start) else {
        return Outcome::Skip(SkipReason::MissingInput);
    };
    let value = match info.label {
        Some(label) if inst.general_formats().is_empty() => Value::Text(label),
        _ => int_value(i64::from(info.number)),
    };
    if inst.has_switch("p") {
        let text = ev.render(inst, value, lang, false);
        return Outcome::Text {
            text: format!("on page {text}"),
            merge: false,
        };
    }
    text_outcome(ev, inst, value, lang)
}

/// `w:numFmt` of the footnote or endnote properties in settings.xml.
fn note_format(doc: &Document, endnote: bool) -> String {
    let default = if endnote { "lowerRoman" } else { "decimal" };
    let Some(part) = doc.package().part("/word/settings.xml") else {
        return default.to_owned();
    };
    let xml = String::from_utf8_lossy(part.data());
    let tag = if endnote {
        "w:endnotePr"
    } else {
        "w:footnotePr"
    };
    let Some(start) = xml.find(&format!("<{tag}")) else {
        return default.to_owned();
    };
    let rest = &xml[start..];
    let end = rest.find(&format!("</{tag}")).unwrap_or(rest.len());
    let block = &rest[..end];
    block
        .find("<w:numFmt")
        .and_then(|i| {
            let after = &block[i..];
            let v = after.find("w:val=\"")? + 7;
            let tail = &after[v..];
            Some(tail[..tail.find('"')?].to_owned())
        })
        .unwrap_or_else(|| default.to_owned())
}

fn note_ref(
    ev: &Evaluator<'_>,
    view: &View<'_>,
    index: usize,
    inst: &Instruction,
    lang: &str,
) -> Outcome {
    let Some(name) = inst.first_arg() else {
        return plain("Error! No bookmark name given.");
    };
    let snap = view.snap();
    let Some(bookmark) = snap.bookmark(name) else {
        return plain(BOOKMARK_UNDEFINED);
    };
    let mark = snap.notes.iter().find(|n| {
        n.pos.para >= bookmark.start.para
            && n.pos.para <= bookmark.end.para
            && n.pos >= bookmark.start
            && n.pos <= bookmark.end
    });
    let Some(mark) = mark else {
        return plain(BOOKMARK_UNDEFINED);
    };
    let fmt = note_format(view.doc, mark.endnote);
    let mut text = format_number(i64::try_from(mark.ordinal).unwrap_or(1), &fmt);
    if inst.has_switch("p") {
        let from = field_pos(view, index);
        text = format!("{text} {}", above_below(bookmark.start, from));
    }
    text_outcome(ev, inst, Value::Text(text), lang)
}

// ---------------------------------------------------------------------------
// SEQ
// ---------------------------------------------------------------------------

/// The numeric argument of a `\r` / `\s` style switch.
fn int_arg(inst: &Instruction, switch: &str) -> Option<i64> {
    inst.switch_arg(switch)?.trim().parse().ok()
}

/// State of one sequence while walking the fields in order.
#[derive(Default, Clone)]
struct SeqCounter {
    value: i64,
    para: Option<usize>,
}

fn is_seq_instruction(text: &str) -> bool {
    let t = text.trim_start();
    t.len() >= 3 && t.is_char_boundary(3) && t[..3].eq_ignore_ascii_case("seq")
}

fn seq(
    ev: &Evaluator<'_>,
    view: &View<'_>,
    index: usize,
    inst: &Instruction,
    lang: &str,
) -> Outcome {
    if inst.first_arg().is_none() {
        return plain("Error! No sequence specified.");
    }
    let snap = view.snap();
    let mut counters: HashMap<String, SeqCounter> = HashMap::new();
    let mut result = 0i64;
    for (i, raw) in view.raws.iter().enumerate().take(index + 1) {
        if !is_seq_instruction(&raw.instr) {
            continue;
        }
        let this = if i == index {
            inst.clone()
        } else {
            Instruction::parse(&raw.instr)
        };
        if this.kind != FieldKind::Seq {
            continue;
        }
        let Some(id) = this.first_arg() else { continue };
        let state = counters.entry(id.to_ascii_lowercase()).or_default();
        let para = raw.begin.para;
        let mut value = state.value + 1;
        if this.has_switch("c") {
            value = state.value.max(1);
        } else if let Some(n) = int_arg(&this, "r").filter(|n| *n >= 0) {
            value = n;
        } else if let Some(level) = int_arg(&this, "s").filter(|n| (1..=9).contains(n)) {
            let level = u8::try_from(level - 1).unwrap_or(0);
            let from = state.para.map_or(0, |p| p + 1);
            let reset = (from..=para).any(|p| snap.outline_level(&ev.styles, p) == Some(level));
            if reset {
                value = 1;
            }
        }
        state.value = value;
        state.para = Some(para);
        if i == index {
            result = value;
        }
    }
    if inst.has_switch("h") {
        return plain("");
    }
    text_outcome(ev, inst, int_value(result), lang)
}

// ---------------------------------------------------------------------------
// STYLEREF
// ---------------------------------------------------------------------------

const BUILTIN_STYLES: &[&str] = &[
    "normal",
    "title",
    "subtitle",
    "caption",
    "quote",
    "intense quote",
    "list paragraph",
    "header",
    "footer",
    "table of figures",
    "strong",
    "emphasis",
];

fn is_builtin_style(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if BUILTIN_STYLES.contains(&lower.as_str()) {
        return true;
    }
    for prefix in ["heading ", "toc "] {
        if let Some(n) = lower.strip_prefix(prefix)
            && let Ok(n) = n.parse::<u8>()
        {
            return (1..=9).contains(&n);
        }
    }
    false
}

fn style_ref(
    ev: &Evaluator<'_>,
    view: &View<'_>,
    index: usize,
    inst: &Instruction,
    lang: &str,
) -> Outcome {
    let Some(arg) = inst.first_arg() else {
        return plain("Error! No style specified.");
    };
    // A bare number names the heading of that level.
    let wanted = match arg.parse::<u8>() {
        Ok(n) if (1..=9).contains(&n) => format!("heading {n}"),
        _ => arg.to_owned(),
    };
    let row = ev
        .styles
        .rows
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(&wanted));
    let known = row.is_some() || is_builtin_style(&wanted);
    let snap = view.snap();
    let from = field_pos(view, index);
    let matches_style = |p: usize| -> bool {
        let Some(info) = snap.paras.get(p) else {
            return false;
        };
        let id = info
            .style_id
            .as_deref()
            .or(ev.styles.default_paragraph.as_deref());
        let Some(id) = id else { return false };
        let name = ev.styles.name_of(id);
        name.eq_ignore_ascii_case(&wanted)
    };
    let non_empty =
        |p: usize| -> bool { snap.paras.get(p).is_some_and(|i| !i.text.trim().is_empty()) };
    let mut found: Option<usize> = (0..from.para)
        .rev()
        .find(|&p| matches_style(p) && non_empty(p));
    if found.is_none() {
        found = (from.para + 1..snap.paras.len()).find(|&p| matches_style(p) && non_empty(p));
    }
    if inst.has_switch("l")
        && let Some(page) = view.page_at(ev.ctx, from).map(|i| i.page)
    {
        let on_page = (0..snap.paras.len()).rev().find(|&p| {
            matches_style(p)
                && non_empty(p)
                && view
                    .page_at(ev.ctx, DocPos { para: p, offset: 0 })
                    .is_some_and(|i| i.page == page)
        });
        if on_page.is_some() {
            found = on_page;
        }
    }
    let Some(p) = found else {
        return if known {
            plain("Error! No text of specified style in document.")
        } else {
            plain(&format!(
                "Error! Use the Home tab to apply {arg} to the text that you want to appear here."
            ))
        };
    };
    let relative = inst.has_switch("p");
    let kind = number_kind(inst);
    let target = DocPos { para: p, offset: 0 };
    let text = match kind {
        Some(kind) => {
            let number = paragraph_number(snap, p, kind, from);
            if relative {
                format!("{number} {}", above_below(target, from))
            } else {
                number
            }
        }
        None if relative => above_below(target, from).to_owned(),
        None => snap.paras[p].text.clone(),
    };
    text_outcome(ev, inst, Value::Text(text), lang)
}

// ---------------------------------------------------------------------------
// IF
// ---------------------------------------------------------------------------

/// `*` and `?` wildcards, case-sensitive like the rest of the comparison.
fn wildcard_match(pattern: &[char], text: &[char]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some(('*', rest)) => (0..=text.len()).any(|k| wildcard_match(rest, &text[k..])),
        Some(('?', rest)) => !text.is_empty() && wildcard_match(rest, &text[1..]),
        Some((c, rest)) => text.first() == Some(c) && wildcard_match(rest, &text[1..]),
    }
}

/// Word compares text like a dictionary: letters first without case, then lower case before
/// upper case.
fn collate(a: &str, b: &str) -> Ordering {
    let fold = |s: &str| s.to_lowercase();
    match fold(a).cmp(&fold(b)) {
        Ordering::Equal => {
            for (x, y) in a.chars().zip(b.chars()) {
                if x != y {
                    return if x.is_lowercase() {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    };
                }
            }
            Ordering::Equal
        }
        other => other,
    }
}

/// An operand: quoted text stays text; a bare word naming a bookmark is replaced by the
/// bookmark's text.
fn operand(view: &View<'_>, arg: &crate::fields::Arg) -> (String, bool) {
    if arg.quoted {
        return (arg.text.clone(), true);
    }
    match view.snap().bookmark(&arg.text) {
        Some(b) => (b.text.trim_end_matches('\r').to_owned(), false),
        None => (arg.text.clone(), false),
    }
}

fn conditional(ev: &Evaluator<'_>, view: &View<'_>, inst: &Instruction, lang: &str) -> Outcome {
    let args = &inst.args;
    if args.len() < 3 {
        return plain("Error! Missing comparison.");
    }
    let op = args[1].text.as_str();
    if !matches!(op, "=" | "<>" | "<" | ">" | "<=" | ">=") {
        // No operator: a number is an error, any other text is true when it is not empty and
        // the next two words are the answers (`IF { x } ...` reads the brace as that text).
        let (left, left_quoted) = operand(view, &args[0]);
        if !left_quoted && parse_number(&left).is_some() {
            return plain("Error! Unknown op code for conditional.");
        }
        let pick = if left.is_empty() {
            args.get(2)
        } else {
            args.get(1)
        };
        let text = pick.map_or_else(String::new, |a| a.text.clone());
        return text_outcome(ev, inst, Value::Text(text), lang);
    }
    let (left, left_quoted) = operand(view, &args[0]);
    let (right, right_quoted) = operand(view, &args[2]);
    let numbers = if left_quoted || right_quoted {
        None
    } else {
        parse_number(&left).zip(parse_number(&right))
    };
    let truth = if let Some((a, b)) = numbers {
        match op {
            "=" => a.value == b.value,
            "<>" => a.value != b.value,
            "<" => a.value < b.value,
            ">" => a.value > b.value,
            "<=" => a.value <= b.value,
            _ => a.value >= b.value,
        }
    } else {
        let l: Vec<char> = left.chars().collect();
        let r: Vec<char> = right.chars().collect();
        match op {
            "=" => wildcard_match(&r, &l),
            "<>" => !wildcard_match(&r, &l),
            "<" => collate(&left, &right) == Ordering::Less,
            ">" => collate(&left, &right) == Ordering::Greater,
            "<=" => collate(&left, &right) != Ordering::Greater,
            _ => collate(&left, &right) != Ordering::Less,
        }
    };
    let pick = if truth { args.get(3) } else { args.get(4) };
    let text = pick.map_or_else(String::new, |a| a.text.clone());
    text_outcome(ev, inst, Value::Text(text), lang)
}

// ---------------------------------------------------------------------------
// MERGEFIELD
// ---------------------------------------------------------------------------

fn merge_field(ev: &Evaluator<'_>, inst: &Instruction, lang: &str) -> Outcome {
    let Some(name) = inst.first_arg() else {
        return plain("Error! No merge field name given.");
    };
    let before = inst.switch_arg("b").unwrap_or_default();
    let after = inst.switch_arg("f").unwrap_or_default();
    let data = ev.ctx.merge.and_then(|m| m.value(name));
    let body = match data {
        Some(value) if value.is_empty() => return plain(""),
        Some(value) => ev.render(inst, Value::Text(value), lang, false),
        None => ev.render(
            inst,
            Value::Text(format!("\u{ab}{name}\u{bb}")),
            lang,
            false,
        ),
    };
    let text = format!("{before}{body}{after}");
    let merge = inst.general_formats().contains(&GeneralFormat::MergeFormat);
    Outcome::Text { text, merge }
}

// ---------------------------------------------------------------------------
// Table formulas
// ---------------------------------------------------------------------------

/// The numbers of a table range for a formula field inside a table cell.
pub(crate) fn table_values(view: &View<'_>, raw: &RawField, reference: &str) -> Option<Vec<f64>> {
    let all = slots(view.doc.body());
    let slot = all.get(raw.begin.para)?;
    let Some(Step::Cell { block, row, cell }) = slot.steps.last().copied() else {
        return None;
    };
    // Blocks of the container that holds the table.
    let outer =
        crate::fields_scan::blocks_ref(view.doc.body(), &slot.steps[..slot.steps.len() - 1])?;
    let Block::Table(table) = outer.get(block)? else {
        return None;
    };
    let grid: Vec<Vec<String>> = table
        .children
        .iter()
        .filter_map(|c| match c {
            TableChild::Row(r) => Some(r),
            _ => None,
        })
        .map(|r| {
            r.children
                .iter()
                .filter_map(|c| match c {
                    RowChild::Cell(cell) => Some(
                        cell.blocks
                            .iter()
                            .filter_map(|b| match b {
                                Block::Paragraph(p) => Some(p.text()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\r"),
                    ),
                    _ => None,
                })
                .collect()
        })
        .collect();
    // Row and cell indices count children; map them to grid positions.
    let row_pos = table.children[..row]
        .iter()
        .filter(|c| matches!(c, TableChild::Row(_)))
        .count();
    let TableChild::Row(this_row) = table.children.get(row)? else {
        return None;
    };
    let col_pos = this_row.children[..cell]
        .iter()
        .filter(|c| matches!(c, RowChild::Cell(_)))
        .count();
    let at = |r: usize, c: usize| grid.get(r).and_then(|row| row.get(c)).map(String::as_str);
    let upper = reference.to_ascii_uppercase();
    // A direction runs over neighbouring cells and ends at the first cell that is blank or
    // not a plain number (measured with Word).
    let walk = |cells: Vec<&str>| -> Vec<f64> {
        let mut out = Vec::new();
        for text in cells {
            match plain_number(text) {
                Some(v) => out.push(v),
                None => break,
            }
        }
        out
    };
    let row_len = grid.get(row_pos).map_or(0, Vec::len);
    let out = match upper.as_str() {
        "ABOVE" => walk(
            (0..row_pos)
                .rev()
                .map(|r| at(r, col_pos).unwrap_or_default())
                .collect(),
        ),
        "BELOW" => walk(
            (row_pos + 1..grid.len())
                .map(|r| at(r, col_pos).unwrap_or_default())
                .collect(),
        ),
        "LEFT" => walk(
            (0..col_pos)
                .rev()
                .map(|c| at(row_pos, c).unwrap_or_default())
                .collect(),
        ),
        "RIGHT" => walk(
            (col_pos + 1..row_len)
                .map(|c| at(row_pos, c).unwrap_or_default())
                .collect(),
        ),
        other => {
            let (first, last) = other.split_once(':').unwrap_or((other, other));
            let (r1, c1) = cell_ref(first)?;
            let (r2, c2) = cell_ref(last)?;
            let mut out = Vec::new();
            for r in r1.min(r2)..=r1.max(r2) {
                for c in c1.min(c2)..=c1.max(c2) {
                    if let Some(v) = at(r, c).and_then(|t| {
                        plain_number(t.trim_start_matches(['$', '\u{a3}', '\u{20ac}']))
                    }) {
                        out.push(v);
                    }
                }
            }
            out
        }
    };
    Some(out)
}

/// A number as Word reads it in a table cell: optional sign, digits with thousands commas, a
/// decimal point and an optional percent sign.
fn plain_number(text: &str) -> Option<f64> {
    let t = text.trim();
    let (body, percent) = match t.strip_suffix('%') {
        Some(b) => (b, true),
        None => (t, false),
    };
    let digits = body.strip_prefix(['-', '+']).unwrap_or(body);
    if digits.is_empty()
        || !digits.chars().any(|c| c.is_ascii_digit())
        || !digits
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ','))
        || digits.matches('.').count() > 1
    {
        return None;
    }
    let value: f64 = body.replace(',', "").parse().ok()?;
    Some(if percent { value / 100.0 } else { value })
}

/// `B3` as zero-based (row, column).
fn cell_ref(text: &str) -> Option<(usize, usize)> {
    let letters: String = text.chars().take_while(char::is_ascii_alphabetic).collect();
    let digits = &text[letters.len()..];
    if letters.is_empty() || digits.is_empty() {
        return None;
    }
    let col = letters
        .chars()
        .fold(0usize, |acc, c| {
            acc * 26 + usize::from(c.to_ascii_uppercase() as u8 - b'A') + 1
        })
        .checked_sub(1)?;
    let row: usize = digits.parse().ok()?;
    Some((row.checked_sub(1)?, col))
}

#[allow(dead_code)]
fn unused(_: &RawXml) {}
