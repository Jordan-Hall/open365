//! A scanner for the common shape of `sheetData`, used instead of the general XML
//! reader because it is several times faster on large sheets.
//!
//! It accepts only plain, unprefixed `row`/`c`/`f`/`v` markup with `r`, `s`, `t` on cells
//! and no namespace declarations, comments, CDATA or unknown cell children. On anything
//! else, and on any error, it returns `None` having changed nothing, and the general
//! reader parses the same bytes. Errors are therefore always the general reader's.

use std::borrow::Cow;
use std::sync::Arc;

use crate::refs::{MAX_COL, MAX_ROW, col_number};
use crate::sheet::{CellParts, Row, TAttr, build_formula, finish_cell};
use crate::xml::{Attr, ElementHead, MAX_XML_DEPTH, Parser};

/// Rows scanned and the byte offset just after `</sheetData>`.
pub(crate) struct Scanned {
    pub rows: Vec<Row>,
    pub end: usize,
}

struct Scan<'t> {
    b: &'t [u8],
    s: &'t str,
    i: usize,
}

fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

/// Decodes the predefined entities and character references of `raw`.
fn decode_entities(raw: &str, out: &mut String) -> Option<()> {
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp + 1..];
        let semi = rest.find(';')?;
        let name = &rest[..semi];
        match name {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "apos" => out.push('\''),
            "quot" => out.push('"'),
            _ => {
                let digits = name.strip_prefix('#')?;
                let code = match digits.strip_prefix('x') {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => digits.parse().ok()?,
                };
                out.push(char::from_u32(code)?);
            }
        }
        rest = &rest[semi + 1..];
    }
    out.push_str(rest);
    Some(())
}

impl<'t> Scan<'t> {
    fn ws(&mut self) {
        while self.b.get(self.i).is_some_and(|&c| is_ws(c)) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> Option<()> {
        (self.peek()? == c).then(|| self.i += 1)
    }

    fn starts(&self, lit: &[u8]) -> bool {
        self.b.get(self.i..).is_some_and(|r| r.starts_with(lit))
    }

    /// Consumes `<name` when it is followed by whitespace, `>` or `/`.
    fn open(&mut self, name: &[u8]) -> Option<()> {
        if self.peek()? != b'<' {
            return None;
        }
        let end = self.i + 1 + name.len();
        if self.b.get(self.i + 1..end)? != name {
            return None;
        }
        match self.b.get(end)? {
            c if is_ws(*c) => {}
            b'>' | b'/' => {}
            _ => return None,
        }
        self.i = end;
        Some(())
    }

    /// Consumes `</name>` (whitespace allowed before `>`).
    fn close(&mut self, name: &[u8]) -> Option<()> {
        if !self.starts(b"</") {
            return None;
        }
        let end = self.i + 2 + name.len();
        if self.b.get(self.i + 2..end)? != name {
            return None;
        }
        let save = self.i;
        self.i = end;
        self.ws();
        let closed = self.eat(b'>');
        if closed.is_none() {
            self.i = save;
        }
        closed
    }

    /// Next attribute, or `None` at the end of the tag (`self.i` is then past `>` or `/>`
    /// and `empty` tells which). Returns `Err(())` on anything unexpected.
    fn attr(&mut self, empty: &mut bool) -> Result<Option<(&'t str, &'t str)>, ()> {
        self.ws();
        match self.peek().ok_or(())? {
            b'>' => {
                self.i += 1;
                *empty = false;
                return Ok(None);
            }
            b'/' => {
                self.i += 1;
                self.eat(b'>').ok_or(())?;
                *empty = true;
                return Ok(None);
            }
            _ => {}
        }
        let start = self.i;
        while self
            .peek()
            .is_some_and(|c| !is_ws(c) && c != b'=' && c != b'>' && c != b'/')
        {
            self.i += 1;
        }
        let name = self.s.get(start..self.i).ok_or(())?;
        if name.is_empty() {
            return Err(());
        }
        self.ws();
        self.eat(b'=').ok_or(())?;
        self.ws();
        let quote = self.peek().ok_or(())?;
        if quote != b'"' && quote != b'\'' {
            return Err(());
        }
        self.i += 1;
        let vstart = self.i;
        while self.peek().ok_or(())? != quote {
            // `<` is illegal in a value; whitespace characters would be normalised.
            if matches!(self.b[self.i], b'<' | b'\t' | b'\n' | b'\r') {
                return Err(());
            }
            self.i += 1;
        }
        let value = self.s.get(vstart..self.i).ok_or(())?;
        self.i += 1;
        Ok(Some((name, value)))
    }

    /// Text up to the next `<`, entity-decoded.
    fn text(&mut self) -> Option<Cow<'t, str>> {
        let start = self.i;
        while self.peek()? != b'<' {
            if self.b[self.i] == b'\r' {
                return None;
            }
            self.i += 1;
        }
        let raw = self.s.get(start..self.i)?;
        if raw.contains('&') {
            let mut out = String::with_capacity(raw.len());
            decode_entities(raw, &mut out)?;
            Some(Cow::Owned(out))
        } else {
            Some(Cow::Borrowed(raw))
        }
    }
}

fn plain_attr(name: &str) -> bool {
    !name.contains(':') && name != "xmlns"
}

fn attr_value(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    decode_entities(raw, &mut out)?;
    Some(out)
}

/// `A1` to `(row, col)` without allocating.
fn cell_ref(s: &str) -> Option<(u32, u32)> {
    let split = s.find(|c: char| !c.is_ascii_alphabetic())?;
    let col = col_number(&s[..split])?;
    let digits = &s[split..];
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (1..=MAX_ROW).contains(&row).then_some((row, col))
}

fn scan_cell(
    sc: &mut Scan<'_>,
    row: u32,
    prev_col: u32,
    ns: &Option<Arc<str>>,
    sst: &[Arc<str>],
    fmt: &mut String,
    text_buf: &mut String,
) -> Option<crate::sheet::Cell> {
    sc.open(b"c")?;
    let (mut col, mut style, mut t_src) = (None, None, TAttr::None);
    let mut empty = false;
    while let Some((name, value)) = sc.attr(&mut empty).ok()? {
        match name {
            "r" => {
                let (r, c) = cell_ref(value)?;
                if r != row {
                    return None;
                }
                col = Some(c);
            }
            "s" => style = Some(value.trim().parse::<u32>().ok()?),
            "t" => t_src = TAttr::parse(value).ok()?,
            _ => return None,
        }
    }
    let col = col?;
    if col <= prev_col || col > MAX_COL {
        return None;
    }
    let mut formula = None;
    let mut has_v = false;
    text_buf.clear();
    if !empty {
        loop {
            sc.ws();
            if sc.close(b"c").is_some() {
                break;
            }
            if sc.starts(b"<f") {
                if formula.is_some() {
                    return None;
                }
                sc.open(b"f")?;
                let mut attrs = Vec::new();
                let mut f_empty = false;
                while let Some((name, value)) = sc.attr(&mut f_empty).ok()? {
                    if !plain_attr(name) {
                        return None;
                    }
                    attrs.push(Attr::plain(name, &attr_value(value)?));
                }
                let text = if f_empty {
                    String::new()
                } else {
                    let t = sc.text()?.into_owned();
                    sc.close(b"f")?;
                    t
                };
                let head = ElementHead {
                    name: "f".to_owned(),
                    ns: ns.clone(),
                    attrs,
                };
                formula = Some(build_formula(head, text).ok()?);
            } else if sc.starts(b"<v") {
                if has_v {
                    return None;
                }
                sc.open(b"v")?;
                let mut v_empty = false;
                // `v` takes no attributes.
                if sc.attr(&mut v_empty).ok()?.is_some() {
                    return None;
                }
                if !v_empty {
                    let t = sc.text()?;
                    text_buf.push_str(&t);
                    sc.close(b"v")?;
                }
                has_v = true;
            } else {
                return None;
            }
        }
    }
    finish_cell(
        CellParts {
            col,
            style,
            t_src,
            attrs: Vec::new(),
            formula,
            has_v,
            inline: None,
            children: Vec::new(),
        },
        text_buf,
        sst,
        fmt,
    )
    .ok()
}

/// Scans `sheetData` content starting at byte `start` (just after its start tag).
pub(crate) fn scan_sheet_data(
    p: &Parser<'_>,
    start: usize,
    ns: &Option<Arc<str>>,
    sst: &[Arc<str>],
    fmt: &mut String,
    text_buf: &mut String,
) -> Option<Scanned> {
    // Rows and cells open scopes of their own; leave the general reader to report depth.
    if p.depth() + 3 > MAX_XML_DEPTH {
        return None;
    }
    let s = p.text();
    let mut sc = Scan {
        b: s.as_bytes(),
        s,
        i: start,
    };
    let mut rows: Vec<Row> = Vec::new();
    let mut prev_row = 0u32;
    loop {
        sc.ws();
        if sc.close(b"sheetData").is_some() {
            return Some(Scanned { rows, end: sc.i });
        }
        sc.open(b"row")?;
        let mut attrs: Vec<Attr> = Vec::new();
        let mut r = None;
        let mut empty = false;
        while let Some((name, value)) = sc.attr(&mut empty).ok()? {
            if name == "xmlns" || name.starts_with("xmlns:") {
                return None;
            }
            let value = attr_value(value)?;
            if name == "r" {
                let n: u32 = value.trim().parse().ok()?;
                if !(1..=MAX_ROW).contains(&n) || n <= prev_row {
                    return None;
                }
                r = Some(n);
            }
            attrs.push(Attr {
                ns: p.resolve_attr(name),
                name: name.to_owned(),
                value,
            });
        }
        let r = r?;
        prev_row = r;
        let mut row = Row::new(
            r,
            ElementHead {
                name: "row".to_owned(),
                ns: ns.clone(),
                attrs,
            },
        );
        if !empty {
            let mut prev_col = 0u32;
            loop {
                sc.ws();
                if sc.close(b"row").is_some() {
                    break;
                }
                let cell = scan_cell(&mut sc, r, prev_col, ns, sst, fmt, text_buf)?;
                prev_col = cell.col;
                row.cells.push(cell);
            }
        }
        rows.push(row);
    }
}
