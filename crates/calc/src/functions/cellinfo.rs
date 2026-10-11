//! `CELL`, `INFO`, `SHEET` and `SHEETS`: what the workbook knows about a cell, the application and the sheets.
//!
//! Measured on Excel 16 (corpus groups `cell`, `info`, `sheet`):
//!
//! * `CELL` takes the first cell of a reference; a union reports `$A$1,$B$2:$C$3` (the first area by its
//!   top-left cell, later areas whole). A reference to a missing sheet is `#N/A` before the info type is looked
//!   at; a value that is not a reference, or an unknown or non-text info type, is `#VALUE!`. `"color"` is
//!   `#VALUE!` in this build. A reference on another sheet is written `[Book1]Sheet2!$B$3`.
//! * `"prefix"` is only set for constant text; a formula cell reports `""`. `"width"` is the rounded width with a
//!   second element, whether it is the standard width.
//! * `INFO` depends on the machine: only the type and shape of `directory`, `numfile`, `osversion` are stable.
//! * `SHEET`/`SHEETS` take a reference or (`SHEET` only) a sheet name; 3-D references are not parsed.

use super::refs::{column_letters, sheet_prefix};
use super::{Res, arity, optional, scalar};
use crate::eval::{Arg, EvalContext, RangeRef};
use crate::numfmt::cell_format_code;
use crate::style::HAlign;
use crate::value::{ErrorKind, Value};

/// The workbook name `CELL("address")` prints for a reference to another sheet (the workbook is unsaved).
const WORKBOOK: &str = "Book1";

fn cell_address(row: u32, col: u32) -> String {
    format!("${}${}", column_letters(col), row)
}

fn area_address(r: &RangeRef) -> String {
    if r.row1 == r.row2 && r.col1 == r.col2 {
        cell_address(r.row1, r.col1)
    } else {
        format!(
            "{}:{}",
            cell_address(r.row1, r.col1),
            cell_address(r.row2, r.col2)
        )
    }
}

fn qualified(sheet: &str) -> String {
    let p = sheet_prefix(sheet);
    match p.strip_prefix('\'') {
        Some(rest) => format!("'[{WORKBOOK}]{rest}"),
        None => format!("[{WORKBOOK}]{p}"),
    }
}

/// `CELL(info_type, [reference])`.
pub(super) fn cell(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 2)?;
    // Without a reference Excel uses the last changed cell; the formula's own cell is the closest stand-in.
    if matches!(args.get(1), Some(Arg::Missing)) {
        return Err(ErrorKind::Value);
    }
    let own;
    let areas: Vec<&RangeRef> = match optional(args, 1) {
        None => {
            let (row, col) = ctx.cell.unwrap_or((1, 1));
            own = RangeRef::new(ctx.sheet, row, col, row, col);
            vec![&own]
        }
        Some(Arg::Range(r)) => vec![r],
        Some(Arg::Union(rs)) if !rs.is_empty() => rs.iter().collect(),
        Some(Arg::Value(Value::Error(e))) => return Err(*e),
        Some(_) => return Err(ErrorKind::Value),
    };
    let first = areas[0];
    if !ctx.source.sheet_exists(&first.sheet) {
        return Err(ErrorKind::NA);
    }
    let kind = match scalar(ctx, &args[0])? {
        Value::Text(t) => t.to_ascii_lowercase(),
        _ => return Err(ErrorKind::Value),
    };
    let (sheet, row, col) = (first.sheet.as_str(), first.row1, first.col1);
    let value = || ctx.source.cell(sheet, row, col);
    Ok(match kind.as_str() {
        "address" => {
            let mut text = String::new();
            if !first.sheet.eq_ignore_ascii_case(ctx.sheet) {
                text.push_str(&qualified(&first.sheet));
            }
            text.push_str(&cell_address(row, col));
            for r in &areas[1..] {
                text.push(',');
                text.push_str(&area_address(r));
            }
            Value::Text(text)
        }
        "col" => Value::Number(f64::from(col)),
        "row" => Value::Number(f64::from(row)),
        "contents" => match value() {
            Value::Empty => Value::Number(0.0),
            v => v,
        },
        "type" => Value::Text(
            match value() {
                Value::Empty => "b",
                Value::Text(_) => "l",
                _ => "v",
            }
            .to_owned(),
        ),
        "prefix" => {
            let constant_text =
                matches!(value(), Value::Text(_)) && !ctx.source.has_formula(sheet, row, col);
            Value::Text(if constant_text {
                match ctx.source.cell_style(sheet, row, col).halign {
                    HAlign::Right => "\"",
                    HAlign::Center | HAlign::CenterAcrossSelection => "^",
                    HAlign::Fill => "\\",
                    _ => "'",
                }
                .to_owned()
            } else {
                String::new()
            })
        }
        "protect" => Value::Number(f64::from(u8::from(
            ctx.source.cell_style(sheet, row, col).locked,
        ))),
        "format" | "parentheses" => {
            let style = ctx.source.cell_style(sheet, row, col);
            let code = style.number_format.as_deref().map_or_else(
                || "G".to_owned(),
                |f| cell_format_code(f, &ctx.locale.currency),
            );
            if kind == "format" {
                Value::Text(code)
            } else {
                Value::Number(f64::from(u8::from(code.ends_with("()"))))
            }
        }
        "width" => {
            let (width, standard) = ctx.source.column_width(sheet, col);
            Value::Array(vec![vec![
                Value::Number(width.round()),
                Value::Bool(standard),
            ]])
        }
        "filename" => Value::Text(String::new()),
        _ => return Err(ErrorKind::Value),
    })
}

/// `INFO(type)`. Values describe a Windows desktop Excel 16 session; the engine has no window or file.
pub(super) fn info(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 1, 1)?;
    let kind = match scalar(ctx, &args[0])? {
        Value::Text(t) => t.to_ascii_lowercase(),
        _ => return Err(ErrorKind::Value),
    };
    Ok(match kind.as_str() {
        "directory" => {
            let mut dir = std::env::current_dir()
                .map(|p| p.to_string_lossy().replace('/', "\\"))
                .unwrap_or_default();
            if !dir.ends_with('\\') {
                dir.push('\\');
            }
            Value::Text(dir)
        }
        "numfile" => Value::Number(ctx.source.sheet_count() as f64),
        "origin" => Value::Text("$A:$A$1".to_owned()),
        "osversion" => Value::Text("Windows (64-bit) NT 10.00".to_owned()),
        "recalc" => Value::Text("Automatic".to_owned()),
        "release" => Value::Text("16.0".to_owned()),
        "system" => Value::Text("pcdos".to_owned()),
        "memavail" | "memused" | "totmem" => return Err(ErrorKind::NA),
        _ => return Err(ErrorKind::Value),
    })
}

/// The sheet a reference argument covers: the sheet of its first area, `#VALUE!` for areas on several sheets.
fn reference_sheet<'a>(ctx: &EvalContext<'_>, arg: &'a Arg) -> Res<Option<&'a str>> {
    let first = match arg {
        Arg::Range(r) => r,
        Arg::Union(rs) => {
            let first = rs.first().ok_or(ErrorKind::Value)?;
            if rs
                .iter()
                .any(|r| !r.sheet.eq_ignore_ascii_case(&first.sheet))
            {
                return Err(ErrorKind::Value);
            }
            first
        }
        Arg::Value(Value::Error(e)) => return Err(*e),
        _ => return Ok(None),
    };
    if !ctx.source.sheet_exists(&first.sheet) {
        return Err(ErrorKind::NA);
    }
    Ok(Some(first.sheet.as_str()))
}

/// `SHEET([value])`: the position of the sheet of a reference or the sheet with that name.
pub(super) fn sheet(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 1)?;
    let name = match optional(args, 0) {
        None => ctx.sheet.to_owned(),
        Some(arg) => match reference_sheet(ctx, arg)? {
            Some(sheet) => sheet.to_owned(),
            None => match arg {
                Arg::Value(Value::Text(t)) => t.clone(),
                _ => return Err(ErrorKind::NA),
            },
        },
    };
    ctx.source
        .sheet_position(&name)
        .map(|i| Value::Number((i + 1) as f64))
        .ok_or(ErrorKind::NA)
}

/// `SHEETS([reference])`: the number of sheets a reference spans, or of the workbook.
pub(super) fn sheets(ctx: &EvalContext<'_>, args: &[Arg]) -> Res<Value> {
    arity(args, 0, 1)?;
    match optional(args, 0) {
        None => Ok(Value::Number(ctx.source.sheet_count() as f64)),
        Some(arg) => match reference_sheet(ctx, arg)? {
            Some(_) => Ok(Value::Number(1.0)),
            None => Err(ErrorKind::NA),
        },
    }
}
