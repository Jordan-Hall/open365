//! Tokenizer and Pratt parser for the A1-style Excel formula grammar (ECMA-376 Part 1 §18.17).
//!
//! Precedence, tightest first: `:` (range), space (intersection), `,` (union, only inside
//! parentheses), unary `-`/`+`, `%`, `^`, `*` `/`, `+` `-`, `&`, comparisons. Unary minus binds
//! tighter than `^`, so `-2^2` is `4`. All binary operators are left associative, including `^`.
//!
//! Parsing never panics: every failure is a typed [`ParseError`]. Input is bounded
//! ([`MAX_FORMULA_CHARS`], nesting and tree height) because formulas are untrusted.

use std::fmt;

use crate::ast::{
    AreaKind, AreaRef, Ast, BinOp, CellRef, MAX_COL, MAX_ROW, UnaryOp, letters_to_col,
    parse_cell_word, parse_row_digits,
};
use crate::value::{ErrorKind, excel_literal};

/// Excel's formula length limit, in characters.
pub const MAX_FORMULA_CHARS: usize = 8192;
const MAX_NESTING: usize = 128;
const MAX_HEIGHT: usize = 1000;
const XLFN_PREFIX: &str = "_xlfn.";

/// What went wrong while parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// The formula is empty.
    Empty,
    /// The formula exceeds [`MAX_FORMULA_CHARS`].
    TooLong,
    /// Parentheses, prefix operators or operator chains nest too deeply.
    TooDeep,
    /// A character that cannot start a token.
    UnexpectedChar(char),
    /// A string literal without a closing quote.
    UnterminatedString,
    /// A quoted sheet name without closing quote and `!`.
    UnterminatedSheetName,
    /// A malformed or out-of-range number literal.
    InvalidNumber,
    /// A token that is not valid at this position.
    UnexpectedToken(String),
    /// The formula ended in the middle of an expression.
    UnexpectedEnd,
    /// A malformed reference or name.
    InvalidReference(String),
    /// An array constant whose rows have different lengths.
    RaggedArray,
    /// Valid Excel syntax that this engine does not support.
    Unsupported(&'static str),
}

/// A parse failure with the byte offset where it was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// What went wrong.
    pub kind: ParseErrorKind,
    /// Byte offset into the input given to [`parse_formula`].
    pub position: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ParseErrorKind::Empty => write!(f, "empty formula"),
            ParseErrorKind::TooLong => {
                write!(f, "formula longer than {MAX_FORMULA_CHARS} characters")
            }
            ParseErrorKind::TooDeep => write!(f, "formula nested too deeply"),
            ParseErrorKind::UnexpectedChar(c) => write!(f, "unexpected character {c:?}"),
            ParseErrorKind::UnterminatedString => write!(f, "unterminated string literal"),
            ParseErrorKind::UnterminatedSheetName => write!(f, "unterminated sheet name"),
            ParseErrorKind::InvalidNumber => write!(f, "invalid number"),
            ParseErrorKind::UnexpectedToken(t) => write!(f, "unexpected {t}"),
            ParseErrorKind::UnexpectedEnd => write!(f, "unexpected end of formula"),
            ParseErrorKind::InvalidReference(r) => write!(f, "invalid reference {r:?}"),
            ParseErrorKind::RaggedArray => write!(f, "array rows differ in length"),
            ParseErrorKind::Unsupported(what) => write!(f, "unsupported syntax: {what}"),
        }?;
        write!(f, " at byte {}", self.position)
    }
}

impl std::error::Error for ParseError {}

fn err<T>(kind: ParseErrorKind, position: usize) -> Result<T, ParseError> {
    Err(ParseError { kind, position })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Number(String),
    Str(String),
    Word(String),
    Func(String),
    Sheet(String),
    Error(ErrorKind),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Amp,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Percent,
    Colon,
    Comma,
    Semi,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Eof,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    pos: usize,
    space_before: bool,
}

fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || matches!(c, '_' | '\\' | '$')
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '\\' | '$' | '.')
}

fn tokenize(src: &str, base: usize) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let at = |i: usize| chars.get(i).map(|&(_, c)| c);
    let off = |i: usize| base + chars.get(i).map_or(src.len(), |&(p, _)| p);
    let mut out = Vec::new();
    let mut i = 0;
    let mut space = false;
    while i < chars.len() {
        let (_, c) = chars[i];
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            space = true;
            i += 1;
            continue;
        }
        let pos = off(i);
        let tok = match c {
            '"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match at(i) {
                        None => return err(ParseErrorKind::UnterminatedString, pos),
                        Some('"') if at(i + 1) == Some('"') => {
                            s.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                Tok::Str(s)
            }
            '\'' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match at(i) {
                        None => return err(ParseErrorKind::UnterminatedSheetName, pos),
                        Some('\'') if at(i + 1) == Some('\'') => {
                            s.push('\'');
                            i += 2;
                        }
                        Some('\'') => {
                            i += 1;
                            break;
                        }
                        Some(ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                if at(i) != Some('!') || s.is_empty() {
                    return err(ParseErrorKind::UnterminatedSheetName, pos);
                }
                i += 1;
                Tok::Sheet(s)
            }
            '#' => {
                let rest = &src[chars[i].0..];
                let found = ErrorKind::ALL.iter().find(|k| {
                    rest.get(..k.as_str().len())
                        .is_some_and(|p| p.eq_ignore_ascii_case(k.as_str()))
                });
                let Some(kind) = found else {
                    return err(ParseErrorKind::UnexpectedChar('#'), pos);
                };
                i += kind.as_str().chars().count();
                Tok::Error(*kind)
            }
            c if c.is_ascii_digit()
                || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit())) =>
            {
                let start = i;
                while at(i).is_some_and(|d| d.is_ascii_digit()) {
                    i += 1;
                }
                if at(i) == Some('.') {
                    i += 1;
                    while at(i).is_some_and(|d| d.is_ascii_digit()) {
                        i += 1;
                    }
                }
                if matches!(at(i), Some('e' | 'E')) {
                    let mut j = i + 1;
                    if matches!(at(j), Some('+' | '-')) {
                        j += 1;
                    }
                    if at(j).is_some_and(|d| d.is_ascii_digit()) {
                        while at(j).is_some_and(|d| d.is_ascii_digit()) {
                            j += 1;
                        }
                        i = j;
                    }
                }
                if at(i).is_some_and(|n| n.is_alphabetic() || n == '_' || n == '$') {
                    return err(ParseErrorKind::InvalidNumber, pos);
                }
                let end = chars.get(i).map_or(src.len(), |&(p, _)| p);
                Tok::Number(src[chars[start].0..end].to_owned())
            }
            c if is_word_start(c) => {
                let start = i;
                while at(i).is_some_and(is_word_char) {
                    i += 1;
                }
                let end = chars.get(i).map_or(src.len(), |&(p, _)| p);
                let word = src[chars[start].0..end].to_owned();
                if at(i) == Some('(') && !word.contains('$') {
                    Tok::Func(word)
                } else if at(i) == Some('!') {
                    i += 1;
                    Tok::Sheet(word)
                } else {
                    Tok::Word(word)
                }
            }
            _ => {
                let two = (c, at(i + 1));
                let (tok, len) = match two {
                    ('<', Some('=')) => (Tok::Le, 2),
                    ('<', Some('>')) => (Tok::Ne, 2),
                    ('>', Some('=')) => (Tok::Ge, 2),
                    ('+', _) => (Tok::Plus, 1),
                    ('-', _) => (Tok::Minus, 1),
                    ('*', _) => (Tok::Star, 1),
                    ('/', _) => (Tok::Slash, 1),
                    ('^', _) => (Tok::Caret, 1),
                    ('&', _) => (Tok::Amp, 1),
                    ('=', _) => (Tok::Eq, 1),
                    ('<', _) => (Tok::Lt, 1),
                    ('>', _) => (Tok::Gt, 1),
                    ('%', _) => (Tok::Percent, 1),
                    (':', _) => (Tok::Colon, 1),
                    (',', _) => (Tok::Comma, 1),
                    (';', _) => (Tok::Semi, 1),
                    ('(', _) => (Tok::LParen, 1),
                    (')', _) => (Tok::RParen, 1),
                    ('{', _) => (Tok::LBrace, 1),
                    ('}', _) => (Tok::RBrace, 1),
                    ('[', _) => {
                        return err(
                            ParseErrorKind::Unsupported("external workbook reference"),
                            pos,
                        );
                    }
                    ('@', _) => {
                        return err(
                            ParseErrorKind::Unsupported("implicit intersection operator"),
                            pos,
                        );
                    }
                    (other, _) => return err(ParseErrorKind::UnexpectedChar(other), pos),
                };
                i += len;
                tok
            }
        };
        out.push(Token {
            tok,
            pos,
            space_before: space,
        });
        space = false;
    }
    out.push(Token {
        tok: Tok::Eof,
        pos: base + src.len(),
        space_before: space,
    });
    Ok(out)
}

/// Parses a formula (with or without a leading `=`) into an [`Ast`].
pub fn parse_formula(input: &str) -> Result<Ast, ParseError> {
    if input.chars().count() > MAX_FORMULA_CHARS {
        return err(ParseErrorKind::TooLong, 0);
    }
    let trimmed = input.trim_start();
    let lead = input.len() - trimmed.len();
    let (body, base) = match trimmed.strip_prefix('=') {
        Some(rest) => (rest, lead + 1),
        None => (trimmed, lead),
    };
    if body.trim().is_empty() {
        return err(ParseErrorKind::Empty, base);
    }
    let toks = tokenize(body, base)?;
    let mut parser = Parser {
        toks,
        i: 0,
        nesting: 0,
    };
    let ast = parser.expr(0, false)?;
    let t = parser.peek();
    if t.tok != Tok::Eof {
        return err(ParseErrorKind::UnexpectedToken(describe(&t.tok)), t.pos);
    }
    if ast.height() > MAX_HEIGHT {
        return err(ParseErrorKind::TooDeep, base);
    }
    Ok(ast)
}

fn describe(tok: &Tok) -> String {
    match tok {
        Tok::Number(n) => format!("number {n}"),
        Tok::Str(_) => "string".to_owned(),
        Tok::Word(w) | Tok::Func(w) => format!("name {w:?}"),
        Tok::Sheet(s) => format!("sheet qualifier {s:?}"),
        Tok::Error(e) => format!("error literal {e}"),
        Tok::Eof => "end of formula".to_owned(),
        other => format!("token {other:?}"),
    }
}

#[derive(Clone, Copy)]
enum Infix {
    Bin(BinOp),
    Percent,
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
    nesting: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        // The token list always ends with `Eof`, which is never consumed.
        &self.toks[self.i.min(self.toks.len() - 1)]
    }

    fn peek_at(&self, offset: usize) -> &Token {
        &self.toks[(self.i + offset).min(self.toks.len() - 1)]
    }

    fn advance(&mut self) -> Token {
        let t = self.peek().clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }

    fn unexpected<T>(&self, t: &Token) -> Result<T, ParseError> {
        if t.tok == Tok::Eof {
            err(ParseErrorKind::UnexpectedEnd, t.pos)
        } else {
            err(ParseErrorKind::UnexpectedToken(describe(&t.tok)), t.pos)
        }
    }

    fn infix(&self, t: &Token, union_ok: bool) -> Option<(u8, Infix)> {
        let op = match t.tok {
            Tok::Plus => BinOp::Add,
            Tok::Minus => BinOp::Sub,
            Tok::Star => BinOp::Mul,
            Tok::Slash => BinOp::Div,
            Tok::Caret => BinOp::Pow,
            Tok::Amp => BinOp::Concat,
            Tok::Eq => BinOp::Eq,
            Tok::Ne => BinOp::Ne,
            Tok::Lt => BinOp::Lt,
            Tok::Le => BinOp::Le,
            Tok::Gt => BinOp::Gt,
            Tok::Ge => BinOp::Ge,
            Tok::Colon => BinOp::Range,
            Tok::Comma if union_ok => BinOp::Union,
            Tok::Percent => return Some((6, Infix::Percent)),
            Tok::Word(_) | Tok::Sheet(_) | Tok::Func(_) | Tok::LParen if t.space_before => {
                BinOp::Intersect
            }
            // `A:A 1:1` - only a row range may follow an intersection space.
            Tok::Number(_) if t.space_before && self.peek_at(1).tok == Tok::Colon => {
                BinOp::Intersect
            }
            _ => return None,
        };
        Some((op.precedence(), Infix::Bin(op)))
    }

    fn expr(&mut self, min_bp: u8, union_ok: bool) -> Result<Ast, ParseError> {
        self.nesting += 1;
        if self.nesting > MAX_NESTING {
            return err(ParseErrorKind::TooDeep, self.peek().pos);
        }
        let result = self.expr_inner(min_bp, union_ok);
        self.nesting -= 1;
        result
    }

    fn expr_inner(&mut self, min_bp: u8, union_ok: bool) -> Result<Ast, ParseError> {
        let mut lhs = self.prefix(union_ok)?;
        loop {
            let tok = self.peek().clone();
            let Some((bp, kind)) = self.infix(&tok, union_ok) else {
                break;
            };
            if bp < min_bp {
                break;
            }
            match kind {
                Infix::Percent => {
                    self.advance();
                    lhs = Ast::Percent(Box::new(lhs));
                }
                Infix::Bin(op) => {
                    if op != BinOp::Intersect {
                        self.advance();
                    }
                    let rhs = self.expr(bp + 1, union_ok)?;
                    lhs = combine(op, lhs, rhs);
                }
            }
        }
        Ok(lhs)
    }

    fn prefix(&mut self, union_ok: bool) -> Result<Ast, ParseError> {
        let t = self.advance();
        match t.tok.clone() {
            Tok::Number(raw) => {
                if let Some(row) = row_from_number(&raw)
                    && let Some(area) = self.finish_rows(row, false, None)
                {
                    return Ok(area);
                }
                match excel_literal(&raw) {
                    Some(n) => Ok(Ast::Number(n)),
                    None => err(ParseErrorKind::InvalidNumber, t.pos),
                }
            }
            Tok::Str(s) => Ok(Ast::Text(s)),
            Tok::Error(e) => Ok(Ast::Error(e)),
            Tok::Minus | Tok::Plus => {
                let operand = self.expr(7, union_ok)?;
                let op = if t.tok == Tok::Minus {
                    UnaryOp::Neg
                } else {
                    UnaryOp::Plus
                };
                Ok(Ast::Unary {
                    op,
                    operand: Box::new(operand),
                })
            }
            Tok::LParen => {
                let inner = self.expr(0, true)?;
                let close = self.advance();
                if close.tok != Tok::RParen {
                    return self.unexpected(&close);
                }
                Ok(inner)
            }
            Tok::LBrace => self.array(),
            Tok::Func(name) => self.call(&name),
            Tok::Sheet(sheet) => self.sheet_reference(sheet),
            Tok::Word(w) => self.word(&w, None, t.pos),
            _ => self.unexpected(&t),
        }
    }

    fn call(&mut self, raw_name: &str) -> Result<Ast, ParseError> {
        let open = self.advance();
        if open.tok != Tok::LParen {
            return self.unexpected(&open);
        }
        let (name, xlfn) = match raw_name.get(..XLFN_PREFIX.len()) {
            Some(p) if p.eq_ignore_ascii_case(XLFN_PREFIX) => {
                (raw_name[XLFN_PREFIX.len()..].to_ascii_uppercase(), true)
            }
            _ => (raw_name.to_ascii_uppercase(), false),
        };
        let mut args = Vec::new();
        if self.peek().tok == Tok::RParen {
            self.advance();
            return Ok(Ast::Func { name, xlfn, args });
        }
        loop {
            if matches!(self.peek().tok, Tok::Comma | Tok::RParen) {
                args.push(Ast::Missing);
            } else {
                args.push(self.expr(0, false)?);
            }
            let sep = self.advance();
            match sep.tok {
                Tok::Comma => {}
                Tok::RParen => break,
                _ => return self.unexpected(&sep),
            }
        }
        Ok(Ast::Func { name, xlfn, args })
    }

    fn array(&mut self) -> Result<Ast, ParseError> {
        let start = self.peek().pos;
        let mut rows: Vec<Vec<Ast>> = vec![Vec::new()];
        loop {
            let item = self.array_item()?;
            if let Some(row) = rows.last_mut() {
                row.push(item);
            }
            let sep = self.advance();
            match sep.tok {
                Tok::Comma => {}
                Tok::Semi => rows.push(Vec::new()),
                Tok::RBrace => break,
                _ => return self.unexpected(&sep),
            }
        }
        let width = rows.first().map_or(0, Vec::len);
        if rows.iter().any(|r| r.len() != width) {
            return err(ParseErrorKind::RaggedArray, start);
        }
        Ok(Ast::Array(rows))
    }

    fn array_item(&mut self) -> Result<Ast, ParseError> {
        let t = self.advance();
        let (sign, t) = match t.tok {
            Tok::Minus | Tok::Plus => {
                let negative = t.tok == Tok::Minus;
                (Some(negative), self.advance())
            }
            _ => (None, t),
        };
        match t.tok.clone() {
            Tok::Number(raw) => match excel_literal(&raw) {
                Some(n) => Ok(Ast::Number(if sign == Some(true) { -n } else { n })),
                None => err(ParseErrorKind::InvalidNumber, t.pos),
            },
            _ if sign.is_some() => self.unexpected(&t),
            Tok::Str(s) => Ok(Ast::Text(s)),
            Tok::Error(e) => Ok(Ast::Error(e)),
            Tok::Word(w) if w.eq_ignore_ascii_case("TRUE") => Ok(Ast::Bool(true)),
            Tok::Word(w) if w.eq_ignore_ascii_case("FALSE") => Ok(Ast::Bool(false)),
            _ => self.unexpected(&t),
        }
    }

    fn sheet_reference(&mut self, sheet: String) -> Result<Ast, ParseError> {
        let t = self.advance();
        match t.tok.clone() {
            Tok::Word(w) => self.word(&w, Some(sheet), t.pos),
            Tok::Number(raw) => {
                if let Some(row) = row_from_number(&raw)
                    && let Some(area) = self.finish_rows(row, false, Some(sheet.clone()))
                {
                    return Ok(area);
                }
                err(
                    ParseErrorKind::InvalidReference(format!("{sheet}!{raw}")),
                    t.pos,
                )
            }
            Tok::Error(e) => Ok(Ast::Error(e)),
            _ => self.unexpected(&t),
        }
    }

    fn word(&mut self, w: &str, sheet: Option<String>, pos: usize) -> Result<Ast, ParseError> {
        if let Some((row, abs)) = row_from_word(w)
            && let Some(area) = self.finish_rows(row, abs, sheet.clone())
        {
            return Ok(area);
        }
        if let Some((col, col_abs, row, row_abs)) = parse_cell_word(w) {
            return Ok(Ast::Ref(CellRef {
                sheet,
                row,
                col,
                row_abs,
                col_abs,
            }));
        }
        if sheet.is_none() && w.eq_ignore_ascii_case("TRUE") {
            return Ok(Ast::Bool(true));
        }
        if sheet.is_none() && w.eq_ignore_ascii_case("FALSE") {
            return Ok(Ast::Bool(false));
        }
        if let Some((col, abs)) = col_from_word(w)
            && let Some(area) = self.finish_cols(col, abs, sheet.clone())
        {
            return Ok(area);
        }
        if w.contains('$') || w.starts_with(|c: char| c.is_ascii_digit()) {
            return err(ParseErrorKind::InvalidReference(w.to_owned()), pos);
        }
        if sheet.is_some() {
            return err(
                ParseErrorKind::Unsupported("sheet-scoped defined name"),
                pos,
            );
        }
        Ok(Ast::Name(w.to_owned()))
    }

    /// Row operand (`5` or `$5`) at `offset` tokens ahead.
    fn row_operand_at(&self, offset: usize) -> Option<(u32, bool)> {
        match &self.peek_at(offset).tok {
            Tok::Number(raw) => row_from_number(raw).map(|r| (r, false)),
            Tok::Word(w) => row_from_word(w),
            _ => None,
        }
    }

    fn col_operand_at(&self, offset: usize) -> Option<(u32, bool)> {
        match &self.peek_at(offset).tok {
            Tok::Word(w) => col_from_word(w),
            _ => None,
        }
    }

    /// After a row operand was consumed: `:row` completes a whole-row reference.
    fn finish_rows(&mut self, row1: u32, abs1: bool, sheet: Option<String>) -> Option<Ast> {
        if self.peek().tok != Tok::Colon {
            return None;
        }
        let (row2, abs2) = self.row_operand_at(1)?;
        self.advance();
        self.advance();
        Some(Ast::Area(AreaRef {
            sheet,
            kind: AreaKind::Rows,
            row1,
            col1: 1,
            row2,
            col2: MAX_COL,
            row1_abs: abs1,
            col1_abs: false,
            row2_abs: abs2,
            col2_abs: false,
        }))
    }

    fn finish_cols(&mut self, col1: u32, abs1: bool, sheet: Option<String>) -> Option<Ast> {
        if self.peek().tok != Tok::Colon {
            return None;
        }
        let (col2, abs2) = self.col_operand_at(1)?;
        self.advance();
        self.advance();
        Some(Ast::Area(AreaRef {
            sheet,
            kind: AreaKind::Cols,
            row1: 1,
            col1,
            row2: MAX_ROW,
            col2,
            row1_abs: false,
            col1_abs: abs1,
            row2_abs: false,
            col2_abs: abs2,
        }))
    }
}

fn row_from_number(raw: &str) -> Option<u32> {
    parse_row_digits(raw)
}

fn row_from_word(w: &str) -> Option<(u32, bool)> {
    parse_row_digits(w.strip_prefix('$')?).map(|r| (r, true))
}

fn col_from_word(w: &str) -> Option<(u32, bool)> {
    let (letters, abs) = match w.strip_prefix('$') {
        Some(rest) => (rest, true),
        None => (w, false),
    };
    letters_to_col(letters).map(|c| (c, abs))
}

/// Builds a binary node, folding `Ref : Ref` on one sheet into an area reference.
fn combine(op: BinOp, lhs: Ast, rhs: Ast) -> Ast {
    if op == BinOp::Range
        && let (Ast::Ref(a), Ast::Ref(b)) = (&lhs, &rhs)
    {
        let same_sheet = match (&a.sheet, &b.sheet) {
            (_, None) => true,
            (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
            (None, Some(_)) => false,
        };
        if same_sheet {
            return Ast::Area(AreaRef {
                sheet: a.sheet.clone(),
                kind: AreaKind::Cells,
                row1: a.row,
                col1: a.col,
                row2: b.row,
                col2: b.col,
                row1_abs: a.row_abs,
                col1_abs: a.col_abs,
                row2_abs: b.row_abs,
                col2_abs: b.col_abs,
            });
        }
    }
    Ast::binary(op, lhs, rhs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::to_formula_string;

    fn p(s: &str) -> Ast {
        parse_formula(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn rt(s: &str) -> String {
        to_formula_string(&p(s))
    }

    fn kind(s: &str) -> ParseErrorKind {
        parse_formula(s).unwrap_err().kind
    }

    fn num(n: f64) -> Ast {
        Ast::Number(n)
    }

    fn cell(row: u32, col: u32) -> Ast {
        Ast::Ref(CellRef::new(row, col))
    }

    #[test]
    fn numbers() {
        assert_eq!(p("1"), num(1.0));
        assert_eq!(p("1.5"), num(1.5));
        assert_eq!(p(".5"), num(0.5));
        assert_eq!(p("5."), num(5.0));
        assert_eq!(p("1E+3"), num(1000.0));
        assert_eq!(p("1e3"), num(1000.0));
        assert_eq!(p("2.5E-2"), num(0.025));
        assert_eq!(rt("1E+3"), "1000");
        assert_eq!(kind("1E999"), ParseErrorKind::InvalidNumber);
        assert_eq!(kind("12abc"), ParseErrorKind::InvalidNumber);
    }

    #[test]
    fn strings_and_escapes() {
        assert_eq!(p("\"abc\""), Ast::Text("abc".into()));
        assert_eq!(p("\"a\"\"b\""), Ast::Text("a\"b".into()));
        assert_eq!(p("\"\""), Ast::Text(String::new()));
        assert_eq!(rt("\"a\"\"b\""), "\"a\"\"b\"");
        assert_eq!(kind("\"abc"), ParseErrorKind::UnterminatedString);
        assert_eq!(p("\"héllo ✓\""), Ast::Text("héllo ✓".into()));
    }

    #[test]
    fn booleans_and_errors() {
        assert_eq!(p("TRUE"), Ast::Bool(true));
        assert_eq!(p("false"), Ast::Bool(false));
        assert_eq!(p("#N/A"), Ast::Error(ErrorKind::NA));
        assert_eq!(p("#div/0!"), Ast::Error(ErrorKind::DivZero));
        assert_eq!(p("#REF!"), Ast::Error(ErrorKind::Ref));
        assert_eq!(p("#NAME?"), Ast::Error(ErrorKind::Name));
        assert_eq!(p("#NUM!"), Ast::Error(ErrorKind::Num));
        assert_eq!(p("#NULL!"), Ast::Error(ErrorKind::Null));
        assert_eq!(p("#VALUE!"), Ast::Error(ErrorKind::Value));
        assert_eq!(kind("#BOGUS!"), ParseErrorKind::UnexpectedChar('#'));
        assert_eq!(p("TRUE()"), Ast::call("TRUE", vec![]));
    }

    #[test]
    fn cell_references() {
        assert_eq!(p("A1"), cell(1, 1));
        assert_eq!(p("b2"), cell(2, 2));
        let abs = p("$A$1");
        assert_eq!(
            abs,
            Ast::Ref(CellRef {
                sheet: None,
                row: 1,
                col: 1,
                row_abs: true,
                col_abs: true
            })
        );
        assert_eq!(rt("$A1"), "$A1");
        assert_eq!(rt("A$1"), "A$1");
        assert_eq!(rt("xfd1048576"), "XFD1048576");
        // Out of range text is a defined name, not a reference.
        assert_eq!(p("XFE1"), Ast::Name("XFE1".into()));
        assert_eq!(p("A0"), Ast::Name("A0".into()));
    }

    #[test]
    fn ranges() {
        assert!(
            matches!(p("A1:B3"), Ast::Area(a) if a.kind == AreaKind::Cells && a.row2 == 3 && a.col2 == 2)
        );
        assert_eq!(rt("a1:b3"), "A1:B3");
        assert_eq!(rt("$A$1:$B$3"), "$A$1:$B$3");
        assert_eq!(rt("A:A"), "A:A");
        assert_eq!(rt("$A:$C"), "$A:$C");
        assert_eq!(rt("1:1"), "1:1");
        assert_eq!(rt("$1:$3"), "$1:$3");
        assert!(matches!(p("A:A"), Ast::Area(a) if a.kind == AreaKind::Cols && a.row2 == MAX_ROW));
        assert!(matches!(p("2:5"), Ast::Area(a) if a.kind == AreaKind::Rows && a.col2 == MAX_COL));
        assert_eq!(rt("SUM(A:A)"), "SUM(A:A)");
    }

    #[test]
    fn sheet_qualified() {
        assert_eq!(rt("Sheet1!A1"), "Sheet1!A1");
        assert_eq!(rt("'My Sheet'!A1:B2"), "'My Sheet'!A1:B2");
        assert_eq!(rt("'it''s'!$A$1"), "'it''s'!$A$1");
        assert_eq!(rt("Sheet1!A:A"), "Sheet1!A:A");
        assert_eq!(rt("Sheet1!1:3"), "Sheet1!1:3");
        assert_eq!(rt("'Sheet1'!A1"), "Sheet1!A1");
        assert_eq!(rt("'A1'!B2"), "'A1'!B2");
        match p("Sheet1!A1:B2") {
            Ast::Area(a) => assert_eq!(a.sheet.as_deref(), Some("Sheet1")),
            other => panic!("{other:?}"),
        }
        assert_eq!(p("Sheet1!#REF!"), Ast::Error(ErrorKind::Ref));
        assert_eq!(kind("'abc"), ParseErrorKind::UnterminatedSheetName);
        assert_eq!(kind("'abc'A1"), ParseErrorKind::UnterminatedSheetName);
    }

    #[test]
    fn names() {
        assert_eq!(p("TaxRate"), Ast::Name("TaxRate".into()));
        assert_eq!(p("my.name_2"), Ast::Name("my.name_2".into()));
        assert_eq!(p("_private"), Ast::Name("_private".into()));
        assert_eq!(rt("TaxRate*A1"), "TaxRate*A1");
    }

    #[test]
    fn function_calls() {
        assert_eq!(rt("sum(a1:a3, 2)"), "SUM(A1:A3,2)");
        assert_eq!(
            rt("IF(A1>1,SUM(B1,C1),MAX(1,2))"),
            "IF(A1>1,SUM(B1,C1),MAX(1,2))"
        );
        assert_eq!(p("NOW()"), Ast::call("NOW", vec![]));
        assert_eq!(
            p("IF(A1,,2)"),
            Ast::call("IF", vec![cell(1, 1), Ast::Missing, num(2.0)])
        );
        assert_eq!(p("F(1,)"), Ast::call("F", vec![num(1.0), Ast::Missing]));
        assert_eq!(p("F(,)"), Ast::call("F", vec![Ast::Missing, Ast::Missing]));
        // LOG10 is a valid cell address but is a function when followed by "(".
        assert_eq!(p("LOG10(100)"), Ast::call("LOG10", vec![num(100.0)]));
        assert_eq!(p("LOG10"), cell(10, 8509));
    }

    #[test]
    fn xlfn_prefix_is_stripped_and_remembered() {
        match p("_xlfn.XLOOKUP(1,A:A,B:B)") {
            Ast::Func { name, xlfn, args } => {
                assert_eq!(name, "XLOOKUP");
                assert!(xlfn);
                assert_eq!(args.len(), 3);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(rt("_xlfn.XLOOKUP(1,A:A,B:B)"), "_xlfn.XLOOKUP(1,A:A,B:B)");
        assert_eq!(rt("_XLFN.concat(\"a\")"), "_xlfn.CONCAT(\"a\")");
    }

    #[test]
    fn unary_and_percent() {
        assert_eq!(
            p("-A1"),
            Ast::Unary {
                op: UnaryOp::Neg,
                operand: Box::new(cell(1, 1))
            }
        );
        assert_eq!(rt("+A1"), "+A1");
        assert_eq!(rt("--A1"), "--A1");
        assert_eq!(p("50%"), Ast::Percent(Box::new(num(50.0))));
        assert_eq!(rt("5%%"), "5%%");
    }

    #[test]
    fn precedence_unary_minus_binds_tighter_than_pow() {
        // Excel: -2^2 = 4, i.e. (-2)^2.
        let expected = Ast::binary(
            BinOp::Pow,
            Ast::Unary {
                op: UnaryOp::Neg,
                operand: Box::new(num(2.0)),
            },
            num(2.0),
        );
        assert_eq!(p("-2^2"), expected);
        assert_eq!(rt("-2^2"), "-2^2");
        assert_eq!(rt("-(2^2)"), "-(2^2)");
        assert_eq!(rt("(-2)^2"), "-2^2");
    }

    #[test]
    fn precedence_table() {
        // ^ is left associative in Excel.
        assert_eq!(
            p("2^3^2"),
            Ast::binary(
                BinOp::Pow,
                Ast::binary(BinOp::Pow, num(2.0), num(3.0)),
                num(2.0)
            )
        );
        assert_eq!(rt("2^(3^2)"), "2^(3^2)");
        assert_eq!(rt("1+2*3"), "1+2*3");
        assert_eq!(rt("(1+2)*3"), "(1+2)*3");
        assert_eq!(rt("(1*2)+3"), "1*2+3");
        assert_eq!(rt("1-(2-3)"), "1-(2-3)");
        assert_eq!(rt("(1-2)-3"), "1-2-3");
        assert_eq!(rt("1+2&3"), "1+2&3");
        assert_eq!(rt("1&2+3"), "1&2+3");
        assert_eq!(rt("(1&2)+3"), "(1&2)+3");
        assert_eq!(rt("1&2=3"), "1&2=3");
        assert_eq!(rt("1=(2&3)"), "1=2&3");
        assert_eq!(rt("(1=2)=3"), "1=2=3");
        assert_eq!(rt("1=(2=3)"), "1=(2=3)");
        assert_eq!(rt("2^-2"), "2^-2");
        assert_eq!(rt("2^3%"), "2^3%");
        assert_eq!(rt("(2^3)%"), "(2^3)%");
        assert_eq!(rt("-50%"), "-50%");
        assert_eq!(rt("-(50%)"), "-(50%)");
        assert_eq!(rt("1--1"), "1--1");
        assert_eq!(rt("A1*-B1"), "A1*-B1");
        assert_eq!(rt("a<>b"), "a<>b");
        assert_eq!(rt("a<=b"), "a<=b");
        assert_eq!(rt("a>=b"), "a>=b");
        assert_eq!(rt("a<b"), "a<b");
        assert_eq!(rt("a>b"), "a>b");
    }

    #[test]
    fn range_operator_binds_tightest() {
        assert_eq!(rt("-A1:B2"), "-A1:B2");
        match p("-A1:B2") {
            Ast::Unary { operand, .. } => assert!(matches!(*operand, Ast::Area(_))),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            p("INDEX(A:A,1):INDEX(A:A,3)"),
            Ast::Binary {
                op: BinOp::Range,
                ..
            }
        ));
        assert_eq!(rt("INDEX(A:A,1):INDEX(A:A,3)"), "INDEX(A:A,1):INDEX(A:A,3)");
        assert!(matches!(
            p("A1:B2:C3"),
            Ast::Binary {
                op: BinOp::Range,
                ..
            }
        ));
    }

    #[test]
    fn union_and_intersection() {
        assert_eq!(rt("SUM((A1,B1))"), "SUM((A1,B1))");
        assert_eq!(rt("SUM((A1:A3,C1:C3,E1))"), "SUM((A1:A3,C1:C3,E1))");
        assert_eq!(rt("SUM(A1,B1)"), "SUM(A1,B1)");
        assert!(matches!(
            p("(A1,B1)"),
            Ast::Binary {
                op: BinOp::Union,
                ..
            }
        ));
        assert_eq!(rt("A1:B2 B1:C3"), "A1:B2 B1:C3");
        assert!(matches!(
            p("A1:B2 B1:C3"),
            Ast::Binary {
                op: BinOp::Intersect,
                ..
            }
        ));
        assert_eq!(rt("SUM(A:A   1:1)"), "SUM(A:A 1:1)");
        // A comma at the top level is not a union.
        assert!(matches!(kind("A1,B1"), ParseErrorKind::UnexpectedToken(_)));
    }

    #[test]
    fn array_constants() {
        assert_eq!(
            p("{1,2;3,4}"),
            Ast::Array(vec![vec![num(1.0), num(2.0)], vec![num(3.0), num(4.0)]])
        );
        assert_eq!(rt("{1,2;3,4}"), "{1,2;3,4}");
        assert_eq!(rt("{\"a\",TRUE,#N/A,-1.5}"), "{\"a\",TRUE,#N/A,-1.5}");
        assert_eq!(rt("SUM({1,2,3})"), "SUM({1,2,3})");
        assert_eq!(kind("{1,2;3}"), ParseErrorKind::RaggedArray);
        assert!(matches!(kind("{}"), ParseErrorKind::UnexpectedToken(_)));
        assert!(matches!(kind("{A1}"), ParseErrorKind::UnexpectedToken(_)));
        assert!(matches!(kind("{-}"), ParseErrorKind::UnexpectedToken(_)));
    }

    #[test]
    fn leading_equals_and_whitespace() {
        assert_eq!(p("=1+2"), p("1+2"));
        assert_eq!(p("  = 1 + 2 "), p("1+2"));
        assert_eq!(rt("= SUM( A1 , B1 )"), "SUM(A1,B1)");
    }

    #[test]
    fn typed_errors_never_panic() {
        assert_eq!(kind(""), ParseErrorKind::Empty);
        assert_eq!(kind("="), ParseErrorKind::Empty);
        assert_eq!(kind("   "), ParseErrorKind::Empty);
        assert_eq!(kind("1+"), ParseErrorKind::UnexpectedEnd);
        assert_eq!(kind("(1"), ParseErrorKind::UnexpectedEnd);
        assert_eq!(kind("SUM("), ParseErrorKind::UnexpectedEnd);
        assert_eq!(kind("SUM(1"), ParseErrorKind::UnexpectedEnd);
        assert!(matches!(kind("1 2"), ParseErrorKind::UnexpectedToken(_)));
        assert!(matches!(kind(")"), ParseErrorKind::UnexpectedToken(_)));
        assert!(matches!(kind("A1:"), ParseErrorKind::UnexpectedEnd));
        assert!(matches!(kind("*1"), ParseErrorKind::UnexpectedToken(_)));
        assert_eq!(kind("1 ? 2"), ParseErrorKind::UnexpectedChar('?'));
        assert!(matches!(
            kind("[1]Sheet1!A1"),
            ParseErrorKind::Unsupported(_)
        ));
        assert!(matches!(kind("@A1"), ParseErrorKind::Unsupported(_)));
        assert!(matches!(kind("$A$1$"), ParseErrorKind::InvalidReference(_)));
        assert!(matches!(kind("Sheet1!foo"), ParseErrorKind::Unsupported(_)));
        assert!(matches!(
            kind("Sheet1!TRUE"),
            ParseErrorKind::Unsupported(_)
        ));
        let e = parse_formula("1+").unwrap_err();
        assert!(e.to_string().contains("unexpected end"));
    }

    #[test]
    fn hostile_inputs_are_bounded() {
        let long = format!("1{}", "+1".repeat(MAX_FORMULA_CHARS));
        assert_eq!(kind(&long), ParseErrorKind::TooLong);
        let parens = format!("{}1{}", "(".repeat(4000), ")".repeat(4000));
        assert_eq!(kind(&parens), ParseErrorKind::TooDeep);
        let minus = format!("{}1", "-".repeat(4000));
        assert_eq!(kind(&minus), ParseErrorKind::TooDeep);
        let funcs = format!("{}1{}", "F(".repeat(2000), ")".repeat(2000));
        assert_eq!(kind(&funcs), ParseErrorKind::TooDeep);
        // A long flat chain is fine up to the height limit.
        let chain = format!("1{}", "+1".repeat(900));
        assert!(parse_formula(&chain).is_ok());
        let tall = format!("1{}", "+1".repeat(1100));
        assert_eq!(kind(&tall), ParseErrorKind::TooDeep);
        for junk in [
            "\u{0}",
            "\u{feff}",
            "é",
            "A1\u{2028}",
            "'",
            "\"",
            "{",
            "}",
            "!",
            "A1!",
        ] {
            let _ = parse_formula(junk);
        }
    }

    #[test]
    fn canonical_form_is_a_fixed_point() {
        let samples = [
            "SUM(A1:B2,IF(C1>0,\"x\",\"y\"))",
            "'My Sheet'!A1+Sheet2!$B$2*2",
            "-A1^2%",
            "IF(A1,,2)",
            "(A1:A3,C1:C3)",
            "A1:B2 B1:C3",
            "{1,2;3,4}&\"x\"",
            "A:A+1:1",
            "1E+30*2",
            "_xlfn.IFS(A1>1,1,TRUE,2)",
        ];
        for s in samples {
            let once = rt(s);
            assert_eq!(rt(&once), once, "{s}");
            assert_eq!(p(&once), p(s), "{s}");
        }
    }

    // ---- generated round-trip -------------------------------------------------------

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn gen_sheet(g: &mut Lcg) -> Option<String> {
        match g.below(4) {
            0 => Some("Sheet1".to_owned()),
            1 => Some("My Sheet".to_owned()),
            2 => Some("it's".to_owned()),
            _ => None,
        }
    }

    fn gen_ref(g: &mut Lcg) -> Ast {
        let sheet = gen_sheet(g);
        match g.below(4) {
            0 => Ast::Ref(CellRef {
                sheet,
                row: 1 + g.below(2000) as u32,
                col: 1 + g.below(300) as u32,
                row_abs: g.below(2) == 0,
                col_abs: g.below(2) == 0,
            }),
            1 => Ast::Area(AreaRef {
                sheet,
                kind: AreaKind::Cells,
                row1: 1 + g.below(50) as u32,
                col1: 1 + g.below(50) as u32,
                row2: 1 + g.below(50) as u32,
                col2: 1 + g.below(50) as u32,
                row1_abs: g.below(2) == 0,
                col1_abs: g.below(2) == 0,
                row2_abs: g.below(2) == 0,
                col2_abs: g.below(2) == 0,
            }),
            2 => Ast::Area(AreaRef {
                sheet,
                kind: AreaKind::Cols,
                row1: 1,
                col1: 1 + g.below(30) as u32,
                row2: MAX_ROW,
                col2: 1 + g.below(30) as u32,
                row1_abs: false,
                col1_abs: g.below(2) == 0,
                row2_abs: false,
                col2_abs: g.below(2) == 0,
            }),
            _ => Ast::Area(AreaRef {
                sheet,
                kind: AreaKind::Rows,
                row1: 1 + g.below(30) as u32,
                col1: 1,
                row2: 1 + g.below(30) as u32,
                col2: MAX_COL,
                row1_abs: g.below(2) == 0,
                col1_abs: false,
                row2_abs: g.below(2) == 0,
                col2_abs: false,
            }),
        }
    }

    fn gen_ast(g: &mut Lcg, depth: u32) -> Ast {
        if depth == 0 || g.below(5) == 0 {
            return match g.below(7) {
                0 => num([0.0, 1.0, 2.5, 1e21, 0.1, 123_456.789, 1e-7, 3e300][g.below(8) as usize]),
                1 => Ast::Text(["", "a", "a\"b", "x y", "é"][g.below(5) as usize].to_owned()),
                2 => Ast::Bool(g.below(2) == 0),
                3 => Ast::Error(ErrorKind::ALL[g.below(7) as usize]),
                4 => gen_ref(g),
                5 => Ast::Name(["alpha", "beta_x", "my.name"][g.below(3) as usize].to_owned()),
                _ => gen_ref(g),
            };
        }
        let d = depth - 1;
        match g.below(9) {
            0 => Ast::Unary {
                op: if g.below(2) == 0 {
                    UnaryOp::Neg
                } else {
                    UnaryOp::Plus
                },
                operand: Box::new(gen_ast(g, d)),
            },
            1 => Ast::Percent(Box::new(gen_ast(g, d))),
            2..=4 => {
                let ops = [
                    BinOp::Add,
                    BinOp::Sub,
                    BinOp::Mul,
                    BinOp::Div,
                    BinOp::Pow,
                    BinOp::Concat,
                    BinOp::Eq,
                    BinOp::Ne,
                    BinOp::Lt,
                    BinOp::Le,
                    BinOp::Gt,
                    BinOp::Ge,
                ];
                Ast::binary(ops[g.below(12) as usize], gen_ast(g, d), gen_ast(g, d))
            }
            5 | 6 => {
                let n = g.below(4) as usize;
                let mut args: Vec<Ast> = (0..n)
                    .map(|_| {
                        if n > 1 && g.below(6) == 0 {
                            Ast::Missing
                        } else {
                            gen_ast(g, d)
                        }
                    })
                    .collect();
                if args.len() == 1 && args[0] == Ast::Missing {
                    args[0] = num(1.0);
                }
                Ast::Func {
                    name: ["SUM", "IF", "XLOOKUP", "MY_FN"][g.below(4) as usize].to_owned(),
                    xlfn: g.below(4) == 0,
                    args,
                }
            }
            7 => {
                let rows = 1 + g.below(3) as usize;
                let cols = 1 + g.below(3) as usize;
                Ast::Array(
                    (0..rows)
                        .map(|_| {
                            (0..cols)
                                .map(|_| match g.below(4) {
                                    0 => num(g.below(100) as f64 - 50.0),
                                    1 => Ast::Text("t".to_owned()),
                                    2 => Ast::Bool(true),
                                    _ => Ast::Error(ErrorKind::NA),
                                })
                                .collect()
                        })
                        .collect(),
                )
            }
            _ => match g.below(3) {
                0 => {
                    // Range of non-reference operands so the parser cannot fold it.
                    let f = |g: &mut Lcg| {
                        if g.below(2) == 0 {
                            Ast::Name("alpha".to_owned())
                        } else {
                            Ast::call("OFFSET", vec![gen_ref(g)])
                        }
                    };
                    Ast::binary(BinOp::Range, f(g), f(g))
                }
                1 => Ast::binary(BinOp::Intersect, gen_ref(g), gen_ref(g)),
                _ => {
                    let n = 2 + g.below(3);
                    let mut it = (0..n).map(|_| gen_ref(g));
                    let first = it.next().unwrap_or(Ast::Missing);
                    it.fold(first, |acc, next| Ast::binary(BinOp::Union, acc, next))
                }
            },
        }
    }

    #[test]
    fn generated_asts_round_trip() {
        let mut g = Lcg(0x5eed);
        for case in 0..3000 {
            let ast = gen_ast(&mut g, 4);
            let text = to_formula_string(&ast);
            let reparsed =
                parse_formula(&text).unwrap_or_else(|e| panic!("case {case}: {text}: {e}"));
            assert_eq!(reparsed, ast, "case {case}: {text}");
            assert_eq!(to_formula_string(&reparsed), text);
        }
    }
}
