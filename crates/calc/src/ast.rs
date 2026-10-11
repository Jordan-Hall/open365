//! Formula syntax tree and its canonical serialisation.

use crate::value::ErrorKind;

/// Last worksheet row (1-based).
pub const MAX_ROW: u32 = 1_048_576;
/// Last worksheet column (1-based, `XFD`).
pub const MAX_COL: u32 = 16_384;

/// A single-cell reference such as `$A$1` or `Sheet1!B2`. Rows and columns are 1-based.
#[derive(Debug, Clone, PartialEq)]
pub struct CellRef {
    /// Explicit sheet qualifier, if any.
    pub sheet: Option<String>,
    /// 1-based row.
    pub row: u32,
    /// 1-based column.
    pub col: u32,
    /// Row carries a `$`.
    pub row_abs: bool,
    /// Column carries a `$`.
    pub col_abs: bool,
}

/// How an area reference was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaKind {
    /// `A1:B3`
    Cells,
    /// `A:C` (all rows)
    Cols,
    /// `1:3` (all columns)
    Rows,
}

/// A rectangular reference such as `A1:B3`, `A:A` or `1:1`. Corner order is kept as written;
/// consumers normalise with min/max.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaRef {
    /// Explicit sheet qualifier, if any.
    pub sheet: Option<String>,
    /// Written form.
    pub kind: AreaKind,
    /// Row of the first corner (1 for whole-column references).
    pub row1: u32,
    /// Column of the first corner (1 for whole-row references).
    pub col1: u32,
    /// Row of the second corner ([`MAX_ROW`] for whole-column references).
    pub row2: u32,
    /// Column of the second corner ([`MAX_COL`] for whole-row references).
    pub col2: u32,
    /// First-corner row carries a `$`.
    pub row1_abs: bool,
    /// First-corner column carries a `$`.
    pub col1_abs: bool,
    /// Second-corner row carries a `$`.
    pub row2_abs: bool,
    /// Second-corner column carries a `$`.
    pub col2_abs: bool,
}

/// `value` (1-based) moved by `delta` when it is relative, wrapping inside `1..=max` as Excel does for the
/// relative references of a defined name.
pub(crate) fn shift_coord(value: u32, absolute: bool, delta: i64, max: u32) -> u32 {
    if absolute || delta == 0 {
        return value;
    }
    let m = i64::from(max);
    let moved = (i64::from(value) - 1 + delta).rem_euclid(m) + 1;
    u32::try_from(moved).unwrap_or(value)
}

/// An area with its relative corners moved by `(d_row, d_col)`, corners put in order.
pub(crate) fn shift_area(a: &AreaRef, d_row: i64, d_col: i64) -> AreaRef {
    let mut out = a.clone();
    if a.kind != AreaKind::Cols {
        out.row1 = shift_coord(a.row1, a.row1_abs, d_row, MAX_ROW);
        out.row2 = shift_coord(a.row2, a.row2_abs, d_row, MAX_ROW);
        if out.row1 > out.row2 {
            std::mem::swap(&mut out.row1, &mut out.row2);
        }
    }
    if a.kind != AreaKind::Rows {
        out.col1 = shift_coord(a.col1, a.col1_abs, d_col, MAX_COL);
        out.col2 = shift_coord(a.col2, a.col2_abs, d_col, MAX_COL);
        if out.col1 > out.col2 {
            std::mem::swap(&mut out.col1, &mut out.col2);
        }
    }
    out
}

/// Prefix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Unary plus.
    Plus,
    /// Unary minus.
    Neg,
}

/// Infix operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `^`
    Pow,
    /// `&`
    Concat,
    /// `=`
    Eq,
    /// `<>`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `:` between non-literal operands
    Range,
    /// Space (intersection)
    Intersect,
    /// `,` inside parentheses (union)
    Union,
}

impl BinOp {
    /// Binding strength; higher binds tighter. Matches the Excel precedence table.
    pub fn precedence(self) -> u8 {
        match self {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 1,
            BinOp::Concat => 2,
            BinOp::Add | BinOp::Sub => 3,
            BinOp::Mul | BinOp::Div => 4,
            BinOp::Pow => 5,
            BinOp::Union => 8,
            BinOp::Intersect => 9,
            BinOp::Range => 10,
        }
    }

    fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Pow => "^",
            BinOp::Concat => "&",
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Range => ":",
            BinOp::Intersect => " ",
            BinOp::Union => ",",
        }
    }
}

/// A parsed formula expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Ast {
    /// Numeric literal (non-negative except inside array constants).
    Number(f64),
    /// String literal.
    Text(String),
    /// `TRUE` / `FALSE`.
    Bool(bool),
    /// Error literal such as `#N/A`.
    Error(ErrorKind),
    /// Single cell reference.
    Ref(CellRef),
    /// Area reference.
    Area(AreaRef),
    /// Defined name.
    Name(String),
    /// Structured table reference such as `Sales[[#This Row],[Qty]]`, resolved when evaluated.
    Structured {
        /// Table name as written.
        table: String,
        /// The bracketed part as written, outer brackets included: `[Qty]`, `[[#Totals],[Qty]]`.
        spec: String,
    },
    /// Function call. `xlfn` records a `_xlfn.` prefix that was stripped from `name`.
    Func {
        /// Upper-case function name without the `_xlfn.` prefix.
        name: String,
        /// The source carried the `_xlfn.` future-function prefix.
        xlfn: bool,
        /// Arguments; an omitted argument is [`Ast::Missing`].
        args: Vec<Ast>,
    },
    /// An omitted function argument, as in `IF(A1,,2)`.
    Missing,
    /// Prefix operator application.
    Unary {
        /// Operator.
        op: UnaryOp,
        /// Operand.
        operand: Box<Ast>,
    },
    /// Postfix percent.
    Percent(Box<Ast>),
    /// Infix operator application.
    Binary {
        /// Operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Ast>,
        /// Right operand.
        rhs: Box<Ast>,
    },
    /// Array constant, stored as rows.
    Array(Vec<Vec<Ast>>),
    /// A call of a computed value, as in `LAMBDA(x,x+1)(2)`.
    Invoke {
        /// The expression producing the function value.
        callee: Box<Ast>,
        /// Arguments; an omitted argument is [`Ast::Missing`].
        args: Vec<Ast>,
    },
}

impl Ast {
    /// Convenience constructor for a binary node.
    pub fn binary(op: BinOp, lhs: Ast, rhs: Ast) -> Ast {
        Ast::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    /// Convenience constructor for a function call without a `_xlfn.` prefix.
    pub fn call(name: &str, args: Vec<Ast>) -> Ast {
        Ast::Func {
            name: name.to_ascii_uppercase(),
            xlfn: false,
            args,
        }
    }

    /// Nesting height of the tree (a leaf is 1). Iterative, safe on very deep trees.
    pub fn height(&self) -> usize {
        let mut max = 0;
        let mut stack: Vec<(&Ast, usize)> = vec![(self, 1)];
        while let Some((node, depth)) = stack.pop() {
            max = max.max(depth);
            match node {
                Ast::Func { args, .. } => stack.extend(args.iter().map(|a| (a, depth + 1))),
                Ast::Invoke { callee, args } => {
                    stack.push((callee, depth + 1));
                    stack.extend(args.iter().map(|a| (a, depth + 1)));
                }
                Ast::Unary { operand, .. } => stack.push((operand, depth + 1)),
                Ast::Percent(inner) => stack.push((inner, depth + 1)),
                Ast::Binary { lhs, rhs, .. } => {
                    stack.push((lhs, depth + 1));
                    stack.push((rhs, depth + 1));
                }
                Ast::Array(rows) => {
                    stack.extend(rows.iter().flatten().map(|a| (a, depth + 1)));
                }
                _ => {}
            }
        }
        max
    }
}

impl CellRef {
    /// A relative reference without a sheet qualifier.
    pub fn new(row: u32, col: u32) -> CellRef {
        CellRef {
            sheet: None,
            row,
            col,
            row_abs: false,
            col_abs: false,
        }
    }
}

/// Converts a 1-based column number to letters (`1` -> `A`, `27` -> `AA`).
pub fn col_to_letters(mut col: u32) -> String {
    let mut out = Vec::new();
    while col > 0 {
        let rem = (col - 1) % 26;
        out.push(b'A' + rem as u8);
        col = (col - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Converts column letters (1 to 3 ASCII letters, any case) to a 1-based column number,
/// rejecting columns past `XFD`.
pub fn letters_to_col(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 || !letters.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return None;
    }
    let col = letters.bytes().fold(0u32, |acc, b| {
        acc * 26 + u32::from(b.to_ascii_uppercase() - b'A') + 1
    });
    (col <= MAX_COL).then_some(col)
}

/// Parses an A1-style address (`B3`, `$B$3`) into `(row, col)`.
pub fn parse_cell_address(text: &str) -> Option<(u32, u32)> {
    let (col, _, row, _) = parse_cell_word(text)?;
    Some((row, col))
}

/// Splits an A1-style word into `(col, col_abs, row, row_abs)`.
pub(crate) fn parse_cell_word(word: &str) -> Option<(u32, bool, u32, bool)> {
    let mut rest = word;
    let col_abs = rest.starts_with('$');
    if col_abs {
        rest = &rest[1..];
    }
    let letters_end = rest.find(|c: char| !c.is_ascii_alphabetic())?;
    let col = letters_to_col(&rest[..letters_end])?;
    rest = &rest[letters_end..];
    let row_abs = rest.starts_with('$');
    if row_abs {
        rest = &rest[1..];
    }
    let row = parse_row_digits(rest)?;
    Some((col, col_abs, row, row_abs))
}

/// Parses digits as a 1-based row number within the sheet limit.
pub(crate) fn parse_row_digits(digits: &str) -> Option<u32> {
    if digits.is_empty() || digits.len() > 8 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (1..=MAX_ROW).contains(&row).then_some(row)
}

/// Serialises an expression canonically: upper-case function names, minimal parentheses,
/// `_xlfn.` prefixes restored, no leading `=`.
pub fn to_formula_string(ast: &Ast) -> String {
    let mut out = String::new();
    write_ast(ast, &mut out);
    out
}

/// The formula as Excel stores and shows it (`FORMULATEXT`): like [`to_formula_string`], but a plus sign in
/// front of a number literal is dropped (`=+5` is stored as `=5`).
pub fn to_stored_formula_string(ast: &Ast) -> String {
    fn strip(ast: &mut Ast) {
        match ast {
            Ast::Unary { op, operand } => {
                strip(operand);
                if *op == UnaryOp::Plus && matches!(**operand, Ast::Number(_)) {
                    let inner = std::mem::replace(&mut **operand, Ast::Missing);
                    *ast = inner;
                }
            }
            Ast::Percent(inner) => strip(inner),
            Ast::Binary { lhs, rhs, .. } => {
                strip(lhs);
                strip(rhs);
            }
            Ast::Func { args, .. } => args.iter_mut().for_each(strip),
            Ast::Invoke { callee, args } => {
                strip(callee);
                args.iter_mut().for_each(strip);
            }
            Ast::Array(rows) => rows.iter_mut().flatten().for_each(strip),
            _ => {}
        }
    }
    let mut copy = ast.clone();
    strip(&mut copy);
    to_formula_string(&copy)
}

fn node_precedence(ast: &Ast) -> u8 {
    match ast {
        // A union always prints its own parentheses, so it is atomic for its parent.
        Ast::Binary {
            op: BinOp::Union, ..
        } => 100,
        Ast::Binary { op, .. } => op.precedence(),
        Ast::Percent(_) => 6,
        Ast::Unary { .. } => 7,
        _ => 100,
    }
}

fn write_child(ast: &Ast, min_precedence: u8, out: &mut String) {
    if node_precedence(ast) < min_precedence {
        out.push('(');
        write_ast(ast, out);
        out.push(')');
    } else {
        write_ast(ast, out);
    }
}

fn write_ast(ast: &Ast, out: &mut String) {
    match ast {
        Ast::Number(n) => out.push_str(&format_number_literal(*n)),
        Ast::Text(t) => {
            out.push('"');
            out.push_str(&t.replace('"', "\"\""));
            out.push('"');
        }
        Ast::Bool(b) => out.push_str(if *b { "TRUE" } else { "FALSE" }),
        Ast::Error(e) => out.push_str(e.as_str()),
        Ast::Ref(r) => {
            write_sheet_prefix(r.sheet.as_deref(), out);
            write_cell(r.col, r.col_abs, r.row, r.row_abs, out);
        }
        Ast::Area(a) => {
            write_sheet_prefix(a.sheet.as_deref(), out);
            match a.kind {
                AreaKind::Cells => {
                    write_cell(a.col1, a.col1_abs, a.row1, a.row1_abs, out);
                    out.push(':');
                    write_cell(a.col2, a.col2_abs, a.row2, a.row2_abs, out);
                }
                AreaKind::Cols => {
                    write_col(a.col1, a.col1_abs, out);
                    out.push(':');
                    write_col(a.col2, a.col2_abs, out);
                }
                AreaKind::Rows => {
                    write_row(a.row1, a.row1_abs, out);
                    out.push(':');
                    write_row(a.row2, a.row2_abs, out);
                }
            }
        }
        Ast::Name(n) => out.push_str(n),
        Ast::Structured { table, spec } => {
            out.push_str(table);
            out.push_str(spec);
        }
        Ast::Func { name, xlfn, args } => {
            if *xlfn {
                out.push_str("_xlfn.");
            }
            out.push_str(name);
            out.push('(');
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_ast(arg, out);
            }
            out.push(')');
        }
        Ast::Missing => {}
        Ast::Unary { op, operand } => {
            out.push(if *op == UnaryOp::Neg { '-' } else { '+' });
            write_child(operand, 7, out);
        }
        Ast::Percent(inner) => {
            write_child(inner, 6, out);
            out.push('%');
        }
        Ast::Binary {
            op: BinOp::Union, ..
        } => {
            let mut items = Vec::new();
            let mut cur = ast;
            while let Ast::Binary {
                op: BinOp::Union,
                lhs,
                rhs,
            } = cur
            {
                items.push(&**rhs);
                cur = lhs;
            }
            items.push(cur);
            out.push('(');
            for (i, item) in items.iter().rev().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_child(item, 9, out);
            }
            out.push(')');
        }
        Ast::Binary { op, lhs, rhs } => {
            let p = op.precedence();
            write_child(lhs, p, out);
            out.push_str(op.symbol());
            write_child(rhs, p + 1, out);
        }
        Ast::Array(rows) => {
            out.push('{');
            for (r, row) in rows.iter().enumerate() {
                if r > 0 {
                    out.push(';');
                }
                for (c, item) in row.iter().enumerate() {
                    if c > 0 {
                        out.push(',');
                    }
                    write_ast(item, out);
                }
            }
            out.push('}');
        }
        Ast::Invoke { callee, args } => {
            // A callee that is not itself a call needs its parentheses to stay a callee.
            if matches!(**callee, Ast::Func { .. } | Ast::Invoke { .. }) {
                write_ast(callee, out);
            } else {
                out.push('(');
                write_ast(callee, out);
                out.push(')');
            }
            out.push('(');
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_ast(arg, out);
            }
            out.push(')');
        }
    }
}

fn write_col(col: u32, abs: bool, out: &mut String) {
    if abs {
        out.push('$');
    }
    out.push_str(&col_to_letters(col));
}

fn write_row(row: u32, abs: bool, out: &mut String) {
    if abs {
        out.push('$');
    }
    out.push_str(&row.to_string());
}

fn write_cell(col: u32, col_abs: bool, row: u32, row_abs: bool, out: &mut String) {
    write_col(col, col_abs, out);
    write_row(row, row_abs, out);
}

fn write_sheet_prefix(sheet: Option<&str>, out: &mut String) {
    let Some(name) = sheet else { return };
    if sheet_needs_quotes(name) {
        out.push('\'');
        out.push_str(&name.replace('\'', "''"));
        out.push('\'');
    } else {
        out.push_str(name);
    }
    out.push('!');
}

/// True when a sheet name must be written in single quotes.
pub fn sheet_needs_quotes(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return true;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return true;
    }
    if name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE") {
        return true;
    }
    if parse_cell_word(name).is_some() {
        return true;
    }
    // R1C1-style names such as R, C, R2, C3 or R2C3 would be ambiguous.
    let lower = name.to_ascii_lowercase();
    let b = lower.as_bytes();
    if matches!(b.first(), Some(b'r' | b'c')) {
        let tail = &lower[1..];
        let digits_or_rc = tail
            .bytes()
            .all(|c| c.is_ascii_digit() || c == b'r' || c == b'c');
        if digits_or_rc {
            return true;
        }
    }
    false
}

fn format_number_literal(n: f64) -> String {
    let plain = format!("{n}");
    if plain.len() <= 24 {
        return plain;
    }
    let sci = format!("{n:e}");
    match sci.split_once('e') {
        Some((mantissa, exp)) => {
            if let Some(neg) = exp.strip_prefix('-') {
                format!("{mantissa}E-{neg}")
            } else {
                format!("{mantissa}E+{exp}")
            }
        }
        None => plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters_round_trip() {
        for col in [1, 2, 26, 27, 52, 702, 703, 16_384] {
            assert_eq!(letters_to_col(&col_to_letters(col)), Some(col));
        }
        assert_eq!(col_to_letters(16_384), "XFD");
        assert_eq!(letters_to_col("XFE"), None);
        assert_eq!(letters_to_col("a"), Some(1));
        assert_eq!(letters_to_col(""), None);
    }

    #[test]
    fn cell_address_parsing() {
        assert_eq!(parse_cell_address("B3"), Some((3, 2)));
        assert_eq!(parse_cell_address("$B$3"), Some((3, 2)));
        assert_eq!(parse_cell_address("XFD1048576"), Some((1_048_576, 16_384)));
        assert_eq!(parse_cell_address("XFD1048577"), None);
        assert_eq!(parse_cell_address("A0"), None);
        assert_eq!(parse_cell_address("A"), None);
        assert_eq!(parse_cell_address("1A"), None);
        assert_eq!(parse_cell_address("ABCD1"), None);
    }

    #[test]
    fn sheet_quoting() {
        assert!(!sheet_needs_quotes("Sheet1"));
        assert!(sheet_needs_quotes("My Sheet"));
        assert!(sheet_needs_quotes("A1"));
        assert!(sheet_needs_quotes("2020"));
        assert!(sheet_needs_quotes("R2C3"));
        assert!(sheet_needs_quotes("it's"));
        assert!(sheet_needs_quotes("true"));
        assert!(sheet_needs_quotes(""));
    }

    #[test]
    fn number_literals_are_exact() {
        for n in [0.1, 1e21, 1.5e-300, 123456.789, 1e300, 0.0] {
            let text = format_number_literal(n);
            let parsed: f64 = text.replace("E+", "e").replace('E', "e").parse().unwrap();
            assert_eq!(parsed, n, "{text}");
        }
    }

    #[test]
    fn height_of_deep_chain_is_iterative() {
        let mut ast = Ast::Number(1.0);
        for _ in 0..500 {
            ast = Ast::binary(BinOp::Add, ast, Ast::Number(1.0));
        }
        assert_eq!(ast.height(), 501);
    }
}
