//! Reference functions: `ROWS`, `COLUMNS`, `ROW`, `COLUMN`, `ADDRESS`, `OFFSET` and `INDIRECT`.
//!
//! `OFFSET` and `INDIRECT` are volatile: the engine evaluates them on every pass, and keeps passing until the
//! cells they point at have settled (see `Engine::recalc_dirty`).

use super::{Res, arity, bool_arg, int_arg, optional, scalar, text_arg};
use crate::ast::{Ast, MAX_COL, MAX_ROW};
use crate::eval::{Arg, EvalContext, RangeRef};
use crate::parser::parse_formula;
use crate::value::{ErrorKind, Value};

/// The single area of a reference argument; unions of several areas are `#REF!`.
fn single_area(arg: &Arg) -> Res<Option<&RangeRef>> {
    match arg {
        Arg::Range(r) => Ok(Some(r)),
        Arg::Union(rs) if rs.len() == 1 => Ok(rs.first()),
        Arg::Union(_) => Err(ErrorKind::Ref),
        Arg::Value(Value::Error(e)) => Err(*e),
        _ => Ok(None),
    }
}

/// `(rows, columns)` of any argument: a reference, an array or a scalar (1 by 1).
fn dimensions(arg: &Arg) -> Res<(usize, usize)> {
    if let Some(r) = single_area(arg)? {
        return Ok((r.rows() as usize, r.cols() as usize));
    }
    Ok(match arg {
        Arg::Array(rows) => (rows.len(), rows.first().map_or(0, Vec::len)),
        Arg::Refs(refs) => (refs.len(), refs.first().map_or(0, Vec::len)),
        _ => (1, 1),
    })
}

pub(super) fn rows(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Number(dimensions(&args[0])?.0 as f64))
}

pub(super) fn columns(_ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    Ok(Value::Number(dimensions(&args[0])?.1 as f64))
}

/// `ROW` and `COLUMN`: the position of a reference (an array for a multi-cell one; the evaluator keeps only
/// its first element outside array contexts) or of the formula cell when there is no argument.
fn position(ctx: &EvalContext<'_>, args: &[Arg], vertical: bool) -> Res<Arg> {
    arity(args, 0, 1)?;
    let Some(arg) = args.first() else {
        let (row, col) = ctx.cell.ok_or(ErrorKind::Value)?;
        return Ok(Arg::Value(Value::Number(f64::from(if vertical {
            row
        } else {
            col
        }))));
    };
    let Some(r) = single_area(arg)? else {
        return Err(ErrorKind::Value);
    };
    let (first, count) = if vertical {
        (r.row1, r.rows())
    } else {
        (r.col1, r.cols())
    };
    let numbers = (0..count).map(|i| Value::Number(f64::from(first + i)));
    Ok(if count == 1 {
        Arg::Value(Value::Number(f64::from(first)))
    } else if vertical {
        Arg::Array(numbers.map(|n| vec![n]).collect())
    } else {
        Arg::Array(vec![numbers.collect()])
    })
}

pub(super) fn row(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    position(ctx, args, true).unwrap_or_else(Arg::error)
}

pub(super) fn column(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    position(ctx, args, false).unwrap_or_else(Arg::error)
}

// ---- ADDRESS -------------------------------------------------------------------------

/// Column letters of a 1-based column number (`27` is `AA`).
pub(crate) fn column_letters(mut col: u32) -> String {
    let mut letters = Vec::new();
    while col > 0 {
        col -= 1;
        letters.push(b'A' + (col % 26) as u8);
        col /= 26;
    }
    letters.reverse();
    String::from_utf8(letters).unwrap_or_default()
}

fn looks_like_cell(name: &str) -> bool {
    let letters = name.chars().take_while(char::is_ascii_alphabetic).count();
    let digits = &name[letters..];
    if letters == 0
        || letters > 3
        || digits.is_empty()
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return false;
    }
    let col = name[..letters]
        .to_ascii_uppercase()
        .bytes()
        .fold(0u32, |acc, b| acc * 26 + u32::from(b - b'A') + 1);
    let row: u64 = digits.parse().unwrap_or(u64::MAX);
    col <= MAX_COL && (1..=u64::from(MAX_ROW)).contains(&row)
}

/// `R`, `C`, `RC`, `R12`, `C3`, `R1C1`, `RC2` and `R4C`: names that read as R1C1 references.
fn looks_like_r1c1(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let mut rest = upper.as_str();
    let mut seen = false;
    for marker in ['R', 'C'] {
        if let Some(after) = rest.strip_prefix(marker) {
            rest = after.trim_start_matches(|c: char| c.is_ascii_digit());
            seen = true;
        }
    }
    seen && rest.is_empty()
}

/// A sheet name as it appears before `!`: quoted unless it is a plain identifier that cannot be mistaken
/// for a cell, an R1C1 reference or a boolean.
pub(crate) fn sheet_prefix(name: &str) -> String {
    let plain_start = |c: char| c.is_alphabetic() || c == '_' || c == '\\';
    let plain = name.chars().next().is_some_and(plain_start)
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '\\'))
        && !looks_like_cell(name)
        && !looks_like_r1c1(name)
        && !name.eq_ignore_ascii_case("TRUE")
        && !name.eq_ignore_ascii_case("FALSE");
    if name.is_empty() || plain {
        format!("{name}!")
    } else {
        format!("'{}'!", name.replace('\'', "''"))
    }
}

pub(super) fn address(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 2, 5)?;
    let row = int_arg(ctx, &args[0])?;
    let col = int_arg(ctx, &args[1])?;
    let absolute = match optional(args, 2) {
        None => 1,
        Some(a) => int_arg(ctx, a)?,
    };
    let a1 = match optional(args, 3) {
        None => true,
        Some(a) => bool_arg(ctx, a)?,
    };
    let sheet = optional(args, 4).map(|a| text_arg(ctx, a)).transpose()?;
    if !(1..=i64::from(MAX_ROW)).contains(&row)
        || !(1..=i64::from(MAX_COL)).contains(&col)
        || !(1..=4).contains(&absolute)
    {
        return Err(ErrorKind::Value);
    }
    // abs 1: $C$5, 2: C$5 (row absolute), 3: $C5 (column absolute), 4: C5.
    let (row_abs, col_abs) = match absolute {
        1 => (true, true),
        2 => (true, false),
        3 => (false, true),
        _ => (false, false),
    };
    let text = if a1 {
        format!(
            "{}{}{}{}",
            if col_abs { "$" } else { "" },
            column_letters(col as u32),
            if row_abs { "$" } else { "" },
            row
        )
    } else {
        // In R1C1 style a relative part is written in brackets and cannot reach the last row or column.
        if (!row_abs && row >= i64::from(MAX_ROW)) || (!col_abs && col >= i64::from(MAX_COL)) {
            return Err(ErrorKind::Value);
        }
        let part = |letter: char, n: i64, abs: bool| {
            if abs {
                format!("{letter}{n}")
            } else {
                format!("{letter}[{n}]")
            }
        };
        format!("{}{}", part('R', row, row_abs), part('C', col, col_abs))
    };
    Ok(Value::Text(match sheet {
        Some(s) => format!("{}{text}", sheet_prefix(&s)),
        None => text,
    }))
}

// ---- OFFSET --------------------------------------------------------------------------

pub(super) fn offset(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    match offset_inner(ctx, args) {
        Ok(r) => Arg::Range(r),
        Err(e) => Arg::error(e),
    }
}

fn offset_inner(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<RangeRef> {
    arity(args, 3, 5)?;
    let base = match &args[0] {
        Arg::Range(r) => r,
        Arg::Value(Value::Error(e)) => return Err(*e),
        _ => return Err(ErrorKind::Value),
    };
    let d_rows = int_arg(ctx, &args[1])?;
    let d_cols = int_arg(ctx, &args[2])?;
    let height = match optional(args, 3) {
        None => i64::from(base.rows()),
        Some(a) => int_arg(ctx, a)?,
    };
    let width = match optional(args, 4) {
        None => i64::from(base.cols()),
        Some(a) => int_arg(ctx, a)?,
    };
    if height == 0 || width == 0 {
        return Err(ErrorKind::Ref);
    }
    // A negative size extends upward or to the left from the moved top-left cell.
    let span = |start: i64, size: i64| {
        if size > 0 {
            (start, start + size - 1)
        } else {
            (start + size + 1, start)
        }
    };
    let (row1, row2) = span(i64::from(base.row1) + d_rows, height);
    let (col1, col2) = span(i64::from(base.col1) + d_cols, width);
    if row1 < 1 || col1 < 1 || row2 > i64::from(MAX_ROW) || col2 > i64::from(MAX_COL) {
        return Err(ErrorKind::Ref);
    }
    Ok(RangeRef::new(
        &base.sheet,
        row1 as u32,
        col1 as u32,
        row2 as u32,
        col2 as u32,
    ))
}

// ---- INDIRECT ------------------------------------------------------------------------

pub(super) fn indirect(ctx: &EvalContext<'_>, args: &[Arg]) -> Arg {
    match indirect_inner(ctx, args) {
        Ok(r) => Arg::Range(r),
        Err(e) => Arg::error(e),
    }
}

fn indirect_inner(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<RangeRef> {
    arity(args, 1, 2)?;
    let text = match scalar(ctx, &args[0])? {
        Value::Text(t) => t,
        _ => String::new(),
    };
    let a1 = match args.get(1) {
        None => true,
        Some(a) => bool_arg(ctx, a)?,
    };
    let range = if a1 {
        parse_a1(ctx, &text)
    } else {
        parse_r1c1(ctx, &text)
    }
    .ok_or(ErrorKind::Ref)?;
    if ctx.source.sheet_exists(&range.sheet) {
        Ok(range)
    } else {
        Err(ErrorKind::Ref)
    }
}

/// An A1-style reference, optionally with a sheet, or a defined name that refers to one.
fn parse_a1(ctx: &EvalContext<'_>, text: &str) -> Option<RangeRef> {
    let first = text.chars().next()?;
    if first.is_whitespace()
        || matches!(first, '=' | '(' | '+' | '-' | '@')
        || text.contains('(')
        || text.matches('!').count() > 1
    {
        return None;
    }
    let ast = parse_formula(text).ok()?;
    let resolved = match &ast {
        Ast::Name(n) => {
            let target = ctx.source.name_in(ctx.sheet, n)?;
            // A name's relative references move with the formula cell.
            let (dr, dc) = ctx
                .cell
                .map_or((0, 0), |(r, c)| (i64::from(r) - 1, i64::from(c) - 1));
            match target {
                Ast::Ref(r) => Ast::Ref(crate::ast::CellRef {
                    row: crate::ast::shift_coord(r.row, r.row_abs, dr, MAX_ROW),
                    col: crate::ast::shift_coord(r.col, r.col_abs, dc, MAX_COL),
                    ..r
                }),
                Ast::Area(a) => Ast::Area(crate::ast::shift_area(&a, dr, dc)),
                other => other,
            }
        }
        _ => ast,
    };
    let sheet_of = |s: &Option<String>| s.clone().unwrap_or_else(|| ctx.sheet.to_owned());
    match resolved {
        Ast::Ref(r) => Some(RangeRef::new(
            &sheet_of(&r.sheet),
            r.row,
            r.col,
            r.row,
            r.col,
        )),
        Ast::Area(a) => {
            let sheet = sheet_of(&a.sheet);
            Some(match a.kind {
                crate::ast::AreaKind::Cells => {
                    RangeRef::new(&sheet, a.row1, a.col1, a.row2, a.col2)
                }
                crate::ast::AreaKind::Cols => RangeRef::new(&sheet, 1, a.col1, MAX_ROW, a.col2),
                crate::ast::AreaKind::Rows => RangeRef::new(&sheet, a.row1, 1, a.row2, MAX_COL),
            })
        }
        _ => None,
    }
}

/// One coordinate of an R1C1 reference.
#[derive(Clone, Copy)]
enum Coord {
    Abs(u32),
    Rel(i64),
}

fn parse_coord(rest: &mut &str, marker: char) -> Option<Option<Coord>> {
    let s = rest
        .strip_prefix(marker.to_ascii_lowercase())
        .or_else(|| rest.strip_prefix(marker))?;
    if let Some(inner) = s.strip_prefix('[') {
        let end = inner.find(']')?;
        let body = &inner[..end];
        let digits = body.strip_prefix(['+', '-']).unwrap_or(body);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *rest = &inner[end + 1..];
        return Some(Some(Coord::Rel(body.trim_start_matches('+').parse().ok()?)));
    }
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    *rest = &s[digits..];
    Some(Some(if digits == 0 {
        Coord::Rel(0)
    } else {
        Coord::Abs(s[..digits].parse().ok()?)
    }))
}

/// `R1C1`, `R[-1]C`, `RC`, `R3` (whole row), `C2` (whole column) in one piece: `(row, col)` coordinates, each
/// absent when the piece does not mention it.
fn parse_r1c1_piece(piece: &str) -> Option<(Option<Coord>, Option<Coord>)> {
    let mut rest = piece;
    let row = if rest.starts_with(['R', 'r']) {
        parse_coord(&mut rest, 'R')?
    } else {
        None
    };
    let col = if rest.starts_with(['C', 'c']) {
        parse_coord(&mut rest, 'C')?
    } else {
        None
    };
    (rest.is_empty() && (row.is_some() || col.is_some())).then_some((row, col))
}

fn resolve(coord: Coord, current: u32, max: u32) -> Option<u32> {
    match coord {
        Coord::Abs(n) => (1..=max).contains(&n).then_some(n),
        // A relative reference wraps around the edge of the sheet.
        Coord::Rel(off) => {
            let max = i64::from(max);
            Some(((i64::from(current) - 1 + off).rem_euclid(max) + 1) as u32)
        }
    }
}

fn parse_r1c1(ctx: &EvalContext<'_>, text: &str) -> Option<RangeRef> {
    if text.starts_with(char::is_whitespace) {
        return None;
    }
    let (sheet, body) = match text.rfind('!') {
        Some(i) => {
            let name = &text[..i];
            let name = match name.strip_prefix('\'') {
                Some(q) => q.strip_suffix('\'')?.replace("''", "'"),
                None => name.to_owned(),
            };
            if name.is_empty() {
                return None;
            }
            (name, &text[i + 1..])
        }
        None => (ctx.sheet.to_owned(), text),
    };
    let body = body.trim_end();
    let (cur_row, cur_col) = ctx.cell.unwrap_or((1, 1));
    let (first, second) = match body.split_once(':') {
        Some((a, b)) => (a, Some(b)),
        None => (body, None),
    };
    let a = parse_r1c1_piece(first)?;
    let b = match second {
        Some(s) => parse_r1c1_piece(s)?,
        None => a,
    };
    let kind = |p: &(Option<Coord>, Option<Coord>)| (p.0.is_some(), p.1.is_some());
    if kind(&a) != kind(&b) {
        return None;
    }
    let row_of = |p: &(Option<Coord>, Option<Coord>)| match p.0 {
        Some(c) => resolve(c, cur_row, MAX_ROW).map(Some),
        None => Some(None),
    };
    let col_of = |p: &(Option<Coord>, Option<Coord>)| match p.1 {
        Some(c) => resolve(c, cur_col, MAX_COL).map(Some),
        None => Some(None),
    };
    let (r1, r2) = (row_of(&a)?, row_of(&b)?);
    let (c1, c2) = (col_of(&a)?, col_of(&b)?);
    Some(RangeRef::new(
        &sheet,
        r1.unwrap_or(1),
        c1.unwrap_or(1),
        r2.unwrap_or(MAX_ROW),
        c2.unwrap_or(MAX_COL),
    ))
}
