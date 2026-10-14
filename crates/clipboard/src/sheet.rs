//! A copied cell range and its clipboard formats: tab-separated text, the HTML table Excel
//! reads and writes (values, number formats, fonts, fills, borders, alignment, merges), and
//! the parser for the same.
//!
//! Reading is bounded by [`crate::limits`] and never fetches anything: `<link>`, `<img>`,
//! `url(...)` and every other reference are ignored.

use std::collections::HashSet;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::css::{self, StyleSheet};
use crate::html::{self, Tok};
use crate::limits::{
    MAX_CELL_CHARS, MAX_CELLS, MAX_COLS, MAX_DEPTH, MAX_FONT_NAME, MAX_FORMAT_CODE, MAX_ROWS,
    MAX_TEXT_BYTES,
};
use crate::model::{ParseError, Rgb};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CellFont {
    pub name: Option<String>,
    pub size_pt: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
    /// `None` is automatic (black).
    pub color: Option<Rgb>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HAlign {
    Left,
    Center,
    Right,
    Justify,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VAlign {
    Top,
    Center,
    Bottom,
}

/// A border line, named as the worksheet file format names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Line {
    Thin,
    Medium,
    Thick,
    Hair,
    Dashed,
    MediumDashed,
    Dotted,
    Double,
    DashDot,
    MediumDashDot,
    DashDotDot,
    MediumDashDotDot,
    SlantDashDot,
}

impl Line {
    /// The OOXML `ST_BorderStyle` name.
    pub fn ooxml(self) -> &'static str {
        match self {
            Self::Thin => "thin",
            Self::Medium => "medium",
            Self::Thick => "thick",
            Self::Hair => "hair",
            Self::Dashed => "dashed",
            Self::MediumDashed => "mediumDashed",
            Self::Dotted => "dotted",
            Self::Double => "double",
            Self::DashDot => "dashDot",
            Self::MediumDashDot => "mediumDashDot",
            Self::DashDotDot => "dashDotDot",
            Self::MediumDashDotDot => "mediumDashDotDot",
            Self::SlantDashDot => "slantDashDot",
        }
    }

    fn css(self) -> &'static str {
        match self {
            Self::Thin => ".5pt solid",
            Self::Medium => "1.5pt solid",
            Self::Thick => "2.0pt solid",
            Self::Hair => ".5pt hairline",
            Self::Dashed => ".5pt dashed",
            Self::MediumDashed => "1.5pt dashed",
            Self::Dotted => ".5pt dotted",
            Self::Double => "2.25pt double",
            Self::DashDot | Self::SlantDashDot => ".5pt dot-dash",
            Self::MediumDashDot => "1.5pt dot-dash",
            Self::DashDotDot => ".5pt dot-dot-dash",
            Self::MediumDashDotDot => "1.5pt dot-dot-dash",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub line: Line,
    /// `None` is automatic (black).
    pub color: Option<Rgb>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Borders {
    pub left: Option<Edge>,
    pub right: Option<Edge>,
    pub top: Option<Edge>,
    pub bottom: Option<Edge>,
}

impl Borders {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum CellKind {
    #[default]
    Blank,
    Number(f64),
    Text,
    Bool(bool),
    Error,
    /// Text from a source that does not say what type it is (plain HTML, TSV): the editor
    /// decides when the text is entered.
    Unparsed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipCell {
    /// The text as displayed.
    pub text: String,
    /// What to enter in the destination: a formula (`=...`), the number, or the text.
    pub entry: String,
    pub kind: CellKind,
    pub number_format: Option<String>,
    pub font: CellFont,
    pub fill: Option<Rgb>,
    pub borders: Borders,
    pub halign: Option<HAlign>,
    pub valign: Option<VAlign>,
    pub wrap: bool,
    /// `(rows, columns)` merged from this cell.
    pub span: (u32, u32),
    /// True for a cell hidden under a merge.
    pub covered: bool,
}

impl Default for ClipCell {
    fn default() -> Self {
        Self {
            text: String::new(),
            entry: String::new(),
            kind: CellKind::Blank,
            number_format: None,
            font: CellFont::default(),
            fill: None,
            borders: Borders::default(),
            halign: None,
            valign: None,
            wrap: false,
            span: (1, 1),
            covered: false,
        }
    }
}

impl ClipCell {
    /// A cell that holds text typed as `entry`, with no formatting.
    pub fn plain(entry: &str) -> Self {
        Self {
            text: entry.to_owned(),
            entry: entry.to_owned(),
            kind: if entry.is_empty() {
                CellKind::Blank
            } else {
                CellKind::Unparsed
            },
            ..Self::default()
        }
    }

    /// True when the cell carries any formatting.
    pub fn is_formatted(&self) -> bool {
        self.number_format.is_some()
            || self.font != CellFont::default()
            || self.fill.is_some()
            || !self.borders.is_empty()
            || self.halign.is_some()
            || self.valign.is_some()
            || self.wrap
    }
}

/// A copied range.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SheetClip {
    /// 1-based row and column of the top-left source cell, when the source is known (formulas
    /// shift by the distance to the destination).
    pub origin: Option<(u32, u32)>,
    pub rows: Vec<Vec<ClipCell>>,
}

fn tsv_field(text: &str, out: &mut String) {
    if text.contains(['\t', '\n', '\r', '"']) {
        out.push('"');
        out.push_str(&text.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(text);
    }
}

/// Splits tab-separated text into rows of fields. Fields in double quotes may hold tabs, line
/// breaks and doubled quotes. A final line break does not start another row.
pub fn parse_tsv(text: &str) -> Result<Vec<Vec<String>>, ParseError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(ParseError::TooLarge);
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut at_start = true;
    let mut cells = 0usize;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if at_start => {
                quoted = true;
                at_start = false;
            }
            '\t' | '\n' | '\r' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                cells += 1;
                if cells > MAX_CELLS || row.len() > MAX_COLS {
                    return Err(ParseError::TooManyItems);
                }
                at_start = true;
                if c != '\t' {
                    rows.push(std::mem::take(&mut row));
                    if rows.len() > MAX_ROWS {
                        return Err(ParseError::TooManyItems);
                    }
                }
            }
            c => {
                field.push(c);
                at_start = false;
            }
        }
        if field.len() > MAX_CELL_CHARS * 4 {
            return Err(ParseError::TooLarge);
        }
    }
    if any && (!field.is_empty() || !row.is_empty() || quoted) {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut sp = true;
    for c in s.chars() {
        if c.is_whitespace() || c == '\u{a0}' {
            if !sp {
                out.push(' ');
            }
            sp = true;
        } else if !c.is_control() {
            out.push(c);
            sp = false;
        }
    }
    out.truncate(out.trim_end().len());
    out
}

fn decode_number_format(raw: &str) -> Option<String> {
    let mut v = raw.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v = &v[1..v.len() - 1];
    }
    let mut out = String::new();
    let mut it = v.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let hex: String = {
            let mut tmp = it.clone();
            (0..4)
                .map_while(|_| tmp.next().filter(char::is_ascii_hexdigit))
                .collect()
        };
        if hex.len() == 4
            && let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
        {
            out.push(ch);
            for _ in 0..4 {
                it.next();
            }
        } else if let Some(n) = it.next() {
            out.push(n);
        }
    }
    let code = match out.as_str() {
        "General" | "" => return None,
        "Percent" => "0%".to_owned(),
        "Fixed" => "0.00".to_owned(),
        "Standard" => "#,##0.00".to_owned(),
        "Scientific" => "0.00E+00".to_owned(),
        "Short Date" => "m/d/yyyy".to_owned(),
        "Short Time" => "h:mm".to_owned(),
        "@" => "@".to_owned(),
        _ => out,
    };
    (code.chars().count() <= MAX_FORMAT_CODE && !code.chars().any(char::is_control)).then_some(code)
}

fn encode_number_format(code: &str) -> String {
    let mut out = String::from("\"");
    for c in code.chars() {
        match c {
            '"' => out.push_str("\\0022"),
            '\\' => out.push_str("\\\\"),
            c if c.is_ascii_alphanumeric() || c == ' ' => out.push(c),
            c if c.is_ascii() => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn parse_edge(v: &str) -> Option<Option<Edge>> {
    let mut width: Option<f32> = None;
    let mut style: Option<&str> = None;
    let mut color: Option<Rgb> = None;
    let mut auto_color = false;
    let mut depth = 0;
    let mut toks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in v.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    for t in &toks {
        let l = t.to_ascii_lowercase();
        match l.as_str() {
            "none" | "hidden" => return Some(None),
            "solid" | "groove" | "ridge" | "inset" | "outset" => style = Some("solid"),
            "dashed" => style = Some("dashed"),
            "dotted" => style = Some("dotted"),
            "double" => style = Some("double"),
            "hairline" => style = Some("hairline"),
            "dot-dash" => style = Some("dot-dash"),
            "dot-dot-dash" => style = Some("dot-dot-dash"),
            "windowtext" | "black" => auto_color = true,
            "thin" => width = Some(0.5),
            "medium" => width = Some(1.5),
            "thick" => width = Some(2.0),
            _ => {
                if let Some(w) = css::length_pt(&l) {
                    width = Some(w);
                } else if let Some(c) = css::parse_color(&l) {
                    color = Some(c);
                }
            }
        }
    }
    if style.is_none() && width.is_none() && color.is_none() && !auto_color {
        return None;
    }
    let w = width.unwrap_or(0.5);
    let heavy = w > 0.9;
    let line = match style.unwrap_or("solid") {
        "dashed" => {
            if heavy {
                Line::MediumDashed
            } else {
                Line::Dashed
            }
        }
        "dotted" => Line::Dotted,
        "double" => Line::Double,
        "hairline" => Line::Hair,
        "dot-dash" => {
            if heavy {
                Line::MediumDashDot
            } else {
                Line::DashDot
            }
        }
        "dot-dot-dash" => {
            if heavy {
                Line::MediumDashDotDot
            } else {
                Line::DashDotDot
            }
        }
        _ => {
            if w <= 0.9 {
                Line::Thin
            } else if w <= 1.75 {
                Line::Medium
            } else {
                Line::Thick
            }
        }
    };
    let color = color.filter(|c| *c != Rgb::BLACK);
    Some(Some(Edge { line, color }))
}

#[derive(Default)]
struct CellBuilder {
    cell: ClipCell,
    text: String,
    raw_num: Option<String>,
    bare_num: bool,
    is_str: bool,
    bool_attr: Option<String>,
    err_attr: Option<String>,
    fmla: Option<String>,
}

fn apply_cell_decl(b: &mut CellBuilder, k: &str, v: &str) {
    let c = &mut b.cell;
    let lv = v.to_ascii_lowercase();
    match k {
        "mso-number-format" => c.number_format = decode_number_format(v),
        "font-family" => {
            if let Some(f) = css::first_family(v) {
                c.font.name = Some(f.chars().take(MAX_FONT_NAME).collect());
            }
        }
        "font-size" => {
            if let Some(p) = css::length_pt(v) {
                c.font.size_pt = Some(p.clamp(1.0, 409.0));
            }
        }
        "font-weight" => {
            c.font.bold =
                lv == "bold" || lv == "bolder" || lv.parse::<u32>().is_ok_and(|w| w >= 600);
        }
        "font-style" => c.font.italic = lv.starts_with("italic") || lv.starts_with("oblique"),
        "text-decoration" | "text-decoration-line" => {
            c.font.strike = lv.contains("line-through");
            c.font.underline = if lv.contains("underline") {
                if c.font.underline == Underline::Double {
                    Underline::Double
                } else {
                    Underline::Single
                }
            } else {
                Underline::None
            };
        }
        "text-underline-style" if lv == "double" || lv == "double-accounting" => {
            c.font.underline = Underline::Double;
        }
        "color" => {
            c.font.color = css::parse_color(v).filter(|c| *c != Rgb::BLACK);
        }
        "background" | "background-color" => {
            c.fill = if lv.starts_with("none") || lv.starts_with("transparent") || lv == "auto" {
                None
            } else {
                css::first_color(v)
            };
        }
        "text-align" => {
            c.halign = match lv.as_str() {
                "left" => Some(HAlign::Left),
                "center" => Some(HAlign::Center),
                "right" => Some(HAlign::Right),
                "justify" => Some(HAlign::Justify),
                "fill" => Some(HAlign::Fill),
                _ => None,
            }
        }
        "vertical-align" => {
            c.valign = match lv.as_str() {
                "top" => Some(VAlign::Top),
                "middle" | "center" => Some(VAlign::Center),
                "bottom" => Some(VAlign::Bottom),
                _ => None,
            }
        }
        "white-space" => c.wrap = lv == "normal" || lv == "pre-wrap" || lv == "pre-line",
        "border" => {
            if let Some(e) = parse_edge(v) {
                c.borders = Borders {
                    left: e,
                    right: e,
                    top: e,
                    bottom: e,
                };
            }
        }
        "border-left" => {
            if let Some(e) = parse_edge(v) {
                c.borders.left = e;
            }
        }
        "border-right" => {
            if let Some(e) = parse_edge(v) {
                c.borders.right = e;
            }
        }
        "border-top" => {
            if let Some(e) = parse_edge(v) {
                c.borders.top = e;
            }
        }
        "border-bottom" => {
            if let Some(e) = parse_edge(v) {
                c.borders.bottom = e;
            }
        }
        _ => {}
    }
}

fn start_cell(attrs: &[(String, String)], sheet: &StyleSheet, name: &str) -> CellBuilder {
    let mut b = CellBuilder::default();
    let classes: Vec<&str> = html::attr(attrs, "class")
        .map(|c| c.split_whitespace().collect())
        .unwrap_or_default();
    let mut decls = sheet.matching(name, &classes);
    if name == "th" {
        let mut base = sheet.matching("td", &classes);
        base.extend(decls);
        decls = base;
    }
    if let Some(a) = html::attr(attrs, "align") {
        decls.insert(0, ("text-align".to_owned(), a.to_owned()));
    }
    if let Some(a) = html::attr(attrs, "valign") {
        decls.insert(0, ("vertical-align".to_owned(), a.to_owned()));
    }
    if let Some(bg) = html::attr(attrs, "bgcolor") {
        decls.insert(0, ("background".to_owned(), bg.to_owned()));
    }
    if let Some(s) = html::attr(attrs, "style") {
        decls.extend(css::declarations(s));
    }
    for (k, v) in &decls {
        apply_cell_decl(&mut b, k, v);
    }
    let span = |n: &str| {
        html::attr(attrs, n)
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(1)
            .max(1)
    };
    b.cell.span = (
        span("rowspan").min(MAX_ROWS as u32),
        span("colspan").min(MAX_COLS as u32),
    );
    for (k, v) in attrs {
        match k.as_str() {
            "x:num" => {
                if v.trim().is_empty() {
                    b.bare_num = true;
                } else {
                    b.raw_num = Some(v.trim().to_owned());
                }
            }
            "x:str" => b.is_str = true,
            "x:bool" => b.bool_attr = Some(v.clone()),
            "x:err" => b.err_attr = Some(v.clone()),
            "x:fmla" => b.fmla = Some(v.clone()),
            _ => {}
        }
    }
    if b.is_str
        && let Some(v) = html::attr(attrs, "x:str").filter(|v| !v.is_empty())
    {
        b.text = v.chars().take(MAX_CELL_CHARS).collect();
    }
    b
}

fn finish_cell(mut b: CellBuilder, text_raw: &str) -> ClipCell {
    let shown = if b.text.is_empty() {
        let t = collapse_keep_breaks(text_raw);
        t.chars().take(MAX_CELL_CHARS).collect::<String>()
    } else {
        std::mem::take(&mut b.text)
    };
    let mut cell = b.cell;
    cell.text = shown.clone();
    let number = if let Some(raw) = &b.raw_num {
        raw.parse::<f64>().ok()
    } else if b.bare_num {
        shown.trim().replace(',', "").parse::<f64>().ok()
    } else {
        None
    }
    .filter(|n| n.is_finite());
    cell.kind = if let Some(n) = number {
        cell.entry = format!("{n}");
        CellKind::Number(n)
    } else if let Some(e) = &b.err_attr {
        cell.entry = if e.is_empty() {
            shown.clone()
        } else {
            e.clone()
        };
        CellKind::Error
    } else if let Some(v) = &b.bool_attr {
        let t = if v.is_empty() { &shown } else { v };
        let on = t.eq_ignore_ascii_case("true") || t == "1";
        cell.entry = if on { "TRUE" } else { "FALSE" }.to_owned();
        CellKind::Bool(on)
    } else if b.is_str {
        cell.entry = shown.clone();
        CellKind::Text
    } else if shown.is_empty() {
        CellKind::Blank
    } else {
        cell.entry = shown.clone();
        CellKind::Unparsed
    };
    if let Some(f) = b.fmla {
        let f = f.trim();
        if !f.is_empty() && f.len() <= 8192 {
            cell.entry = if f.starts_with('=') {
                f.to_owned()
            } else {
                format!("={f}")
            };
        }
    }
    cell
}

/// Collapses runs of white space but keeps the `\n` that `<br>` produced.
fn collapse_keep_breaks(s: &str) -> String {
    s.split('\n')
        .map(collapse)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_owned()
}

impl SheetClip {
    pub fn height(&self) -> usize {
        self.rows.len()
    }

    pub fn width(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.rows
            .iter()
            .all(|r| r.iter().all(|c| c.entry.is_empty() && !c.is_formatted()))
    }

    /// Plain-text grid: no formatting, every cell unparsed.
    pub fn from_grid(rows: &[Vec<String>]) -> Self {
        Self {
            origin: None,
            rows: rows
                .iter()
                .map(|r| r.iter().map(|t| ClipCell::plain(t)).collect())
                .collect(),
        }
    }

    pub fn from_tsv(text: &str) -> Result<Self, ParseError> {
        Ok(Self::from_grid(&parse_tsv(text)?))
    }

    /// The displayed text, tab separated, one CR LF ended line per row.
    pub fn to_tsv(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            for (i, c) in row.iter().enumerate() {
                if i > 0 {
                    out.push('\t');
                }
                tsv_field(&c.text, &mut out);
            }
            out.push_str("\r\n");
        }
        out
    }

    /// Rows and columns swapped (Paste Special > Transpose): borders and merges turn with
    /// the cells. Short rows are padded with empty cells.
    pub fn transposed(&self) -> Self {
        let (h, w) = (self.height(), self.width());
        let mut rows: Vec<Vec<ClipCell>> = vec![Vec::with_capacity(h); w];
        for r in 0..h {
            for (c, out) in rows.iter_mut().enumerate() {
                let mut cell = self.rows[r].get(c).cloned().unwrap_or_default();
                let b = cell.borders;
                cell.borders = Borders {
                    left: b.top,
                    right: b.bottom,
                    top: b.left,
                    bottom: b.right,
                };
                cell.span = (cell.span.1, cell.span.0);
                out.push(cell);
            }
        }
        Self {
            origin: self.origin,
            rows,
        }
    }

    /// What the destination should enter, row by row (formulas as `=...`).
    pub fn entries(&self) -> Vec<Vec<String>> {
        self.rows
            .iter()
            .map(|r| r.iter().map(|c| c.entry.clone()).collect())
            .collect()
    }

    /// Checks and repairs a clip that did not come from our own code.
    pub fn sanitize(&mut self) -> bool {
        let w = self.width();
        if self.rows.len() > MAX_ROWS
            || w > MAX_COLS
            || self.rows.len().saturating_mul(w) > MAX_CELLS
        {
            return false;
        }
        for c in self.rows.iter_mut().flatten() {
            c.text = c
                .text
                .chars()
                .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                .take(MAX_CELL_CHARS)
                .collect();
            c.entry = c
                .entry
                .chars()
                .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                .take(MAX_CELL_CHARS)
                .collect();
            if let Some(f) = &c.number_format
                && (f.chars().count() > MAX_FORMAT_CODE || f.chars().any(char::is_control))
            {
                c.number_format = None;
            }
            if let Some(n) = &mut c.font.name {
                *n = n
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_FONT_NAME)
                    .collect();
            }
            if let Some(s) = &mut c.font.size_pt {
                *s = if s.is_finite() {
                    s.clamp(1.0, 409.0)
                } else {
                    11.0
                };
            }
            if let CellKind::Number(n) = c.kind
                && !n.is_finite()
            {
                c.kind = CellKind::Unparsed;
            }
            c.span = (
                c.span.0.clamp(1, MAX_ROWS as u32),
                c.span.1.clamp(1, MAX_COLS as u32),
            );
        }
        true
    }

    /// An HTML table fragment in the form Excel writes for a copied range.
    pub fn to_html(&self) -> String {
        let mut out = String::from(
            "<table border=0 cellpadding=0 cellspacing=0 style='border-collapse:collapse'>\r\n",
        );
        for row in &self.rows {
            out.push_str("<tr>");
            for c in row {
                if c.covered {
                    continue;
                }
                out.push_str("<td");
                if c.span.0 > 1 {
                    let _ = write!(out, " rowspan={}", c.span.0);
                }
                if c.span.1 > 1 {
                    let _ = write!(out, " colspan={}", c.span.1);
                }
                match c.kind {
                    CellKind::Number(n) => {
                        let _ = write!(out, " x:num=\"{n}\"");
                    }
                    CellKind::Text => out.push_str(" x:str"),
                    CellKind::Bool(b) => {
                        let _ = write!(out, " x:bool=\"{}\"", if b { "TRUE" } else { "FALSE" });
                    }
                    CellKind::Error => {
                        let _ = write!(out, " x:err=\"{}\"", attr_escape(&c.entry));
                    }
                    CellKind::Blank | CellKind::Unparsed => {}
                }
                if c.entry.starts_with('=') {
                    let _ = write!(out, " x:fmla=\"{}\"", attr_escape(&c.entry));
                }
                let style = cell_css(c);
                if !style.is_empty() {
                    let _ = write!(out, " style='{}'", style.replace('\'', "&#39;"));
                }
                out.push('>');
                html_text(&c.text, &mut out);
                out.push_str("</td>");
            }
            out.push_str("</tr>\r\n");
        }
        out.push_str("</table>");
        out
    }

    /// Reads the first table of an HTML document or fragment (Excel, browsers, other
    /// spreadsheets).
    pub fn from_html(src: &str) -> Result<Self, ParseError> {
        let toks = html::tokenize(src)?;
        let mut sheet = StyleSheet::default();
        for s in html::style_text(&toks) {
            sheet.add(s);
        }
        let frag = html::fragment(&toks);
        let start = frag
            .iter()
            .position(|t| matches!(t, Tok::Open { name, .. } if name == "table"))
            .ok_or(ParseError::Malformed("no table"))?;
        let mut rows: Vec<Vec<ClipCell>> = Vec::new();
        let mut occupied: HashSet<(usize, usize)> = HashSet::new();
        let mut cells = 0usize;
        let mut depth = 0usize;
        let mut col = 0usize;
        let mut builder: Option<(CellBuilder, String)> = None;
        let mut done = false;
        for t in &frag[start..] {
            if done {
                break;
            }
            match t {
                Tok::Open { name, attrs, .. } => match name.as_str() {
                    "table" => {
                        depth += 1;
                        if depth > MAX_DEPTH {
                            return Err(ParseError::TooDeep);
                        }
                        if depth > 1
                            && let Some((_, s)) = &mut builder
                        {
                            s.push(' ');
                        }
                    }
                    "tr" if depth == 1 => {
                        if let Some((b, s)) = builder.take() {
                            push_cell(&mut rows, &mut col, &mut occupied, &mut cells, b, &s)?;
                        }
                        rows.push(Vec::new());
                        col = 0;
                        if rows.len() > MAX_ROWS {
                            return Err(ParseError::TooManyItems);
                        }
                    }
                    "td" | "th" if depth == 1 => {
                        if let Some((b, s)) = builder.take() {
                            push_cell(&mut rows, &mut col, &mut occupied, &mut cells, b, &s)?;
                        }
                        if rows.is_empty() {
                            rows.push(Vec::new());
                        }
                        builder = Some((start_cell(attrs, &sheet, name), String::new()));
                    }
                    "br" => {
                        if let Some((_, s)) = &mut builder {
                            s.push('\n');
                        }
                    }
                    "b" | "strong" | "i" | "em" | "u" | "s" | "strike" => {
                        if let Some((b, _)) = &mut builder {
                            match name.as_str() {
                                "b" | "strong" => b.cell.font.bold = true,
                                "i" | "em" => b.cell.font.italic = true,
                                "u" => b.cell.font.underline = Underline::Single,
                                _ => b.cell.font.strike = true,
                            }
                        }
                    }
                    _ => {}
                },
                Tok::Close(name) => match name.as_str() {
                    "table" => {
                        if depth == 1 {
                            done = true;
                        }
                        depth = depth.saturating_sub(1);
                    }
                    "td" | "th" if depth == 1 => {
                        if let Some((b, s)) = builder.take() {
                            push_cell(&mut rows, &mut col, &mut occupied, &mut cells, b, &s)?;
                        }
                    }
                    _ => {}
                },
                Tok::Text(s) => {
                    if let Some((_, acc)) = &mut builder {
                        if acc.len() + s.len() > MAX_CELL_CHARS * 8 {
                            return Err(ParseError::TooLarge);
                        }
                        acc.push_str(s);
                    }
                }
                _ => {}
            }
        }
        if let Some((b, s)) = builder.take() {
            push_cell(&mut rows, &mut col, &mut occupied, &mut cells, b, &s)?;
        }
        let mut clip = SheetClip { origin: None, rows };
        if !clip.sanitize() {
            return Err(ParseError::TooManyItems);
        }
        Ok(clip)
    }
}

fn push_cell(
    rows: &mut [Vec<ClipCell>],
    col: &mut usize,
    occupied: &mut HashSet<(usize, usize)>,
    cells: &mut usize,
    b: CellBuilder,
    text: &str,
) -> Result<(), ParseError> {
    let r = rows.len().saturating_sub(1);
    while occupied.contains(&(r, *col)) {
        fill_to(rows, r, *col + 1, true, cells)?;
        *col += 1;
    }
    fill_to(rows, r, *col, false, cells)?;
    let cell = finish_cell(b, text);
    let (rs, cs) = (cell.span.0 as usize, cell.span.1 as usize);
    if *col + cs > MAX_COLS {
        return Err(ParseError::TooManyItems);
    }
    rows[r].push(cell);
    *cells += 1;
    for _ in 1..cs {
        rows[r].push(ClipCell {
            covered: true,
            ..ClipCell::default()
        });
        *cells += 1;
    }
    // Rows below hold the rest of the merge; bounded so a huge `rowspan` cannot allocate.
    for dr in 1..rs.min(1024) {
        for dc in 0..cs {
            occupied.insert((r + dr, *col + dc));
        }
        if occupied.len() > MAX_CELLS {
            return Err(ParseError::TooManyItems);
        }
    }
    *col += cs;
    if *cells > MAX_CELLS || rows.len() > MAX_ROWS {
        return Err(ParseError::TooManyItems);
    }
    Ok(())
}

fn fill_to(
    rows: &mut [Vec<ClipCell>],
    r: usize,
    len: usize,
    covered: bool,
    cells: &mut usize,
) -> Result<(), ParseError> {
    while rows[r].len() < len {
        rows[r].push(ClipCell {
            covered,
            ..ClipCell::default()
        });
        *cells += 1;
        if *cells > MAX_CELLS {
            return Err(ParseError::TooManyItems);
        }
    }
    Ok(())
}

fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn html_text(text: &str, out: &mut String) {
    let mut prev_space = true;
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("<br>"),
            ' ' => out.push_str(if prev_space { "&nbsp;" } else { " " }),
            '\t' => out.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
            c => out.push(c),
        }
        prev_space = c == ' ' || c == '\n';
    }
}

fn cell_css(c: &ClipCell) -> String {
    let mut s = String::new();
    if let Some(f) = &c.number_format {
        let _ = write!(s, "mso-number-format:{};", encode_number_format(f));
    }
    if let Some(n) = &c.font.name {
        let _ = write!(
            s,
            "font-family:{};",
            n.replace(['\'', '"', ';', '<', '>', '&', '{', '}'], "")
        );
    }
    if let Some(p) = c.font.size_pt {
        let _ = write!(s, "font-size:{p}pt;");
    }
    if c.font.bold {
        s.push_str("font-weight:700;");
    }
    if c.font.italic {
        s.push_str("font-style:italic;");
    }
    match (c.font.underline, c.font.strike) {
        (Underline::None, false) => {}
        (u, strike) => {
            s.push_str("text-decoration:");
            if u != Underline::None {
                s.push_str("underline");
            }
            if strike {
                if u != Underline::None {
                    s.push(' ');
                }
                s.push_str("line-through");
            }
            s.push(';');
            if u == Underline::Double {
                s.push_str("text-underline-style:double;");
            }
        }
    }
    if let Some(col) = c.font.color {
        let _ = write!(s, "color:{};", col.css());
    }
    if let Some(f) = c.fill {
        let _ = write!(s, "background:{};mso-pattern:black none;", f.css());
    }
    let edge = |name: &str, e: &Option<Edge>, s: &mut String| {
        if let Some(e) = e {
            let col = e.color.map_or("windowtext".to_owned(), Rgb::css);
            let _ = write!(s, "border-{name}:{} {col};", e.line.css());
        }
    };
    edge("top", &c.borders.top, &mut s);
    edge("right", &c.borders.right, &mut s);
    edge("bottom", &c.borders.bottom, &mut s);
    edge("left", &c.borders.left, &mut s);
    if let Some(a) = c.halign {
        s.push_str(match a {
            HAlign::Left => "text-align:left;",
            HAlign::Center => "text-align:center;",
            HAlign::Right => "text-align:right;",
            HAlign::Justify => "text-align:justify;",
            HAlign::Fill => "text-align:fill;",
        });
    }
    if let Some(a) = c.valign {
        s.push_str(match a {
            VAlign::Top => "vertical-align:top;",
            VAlign::Center => "vertical-align:middle;",
            VAlign::Bottom => "vertical-align:bottom;",
        });
    }
    if c.wrap {
        s.push_str("white-space:normal;");
    }
    s.pop();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formatted() -> SheetClip {
        let mut num = ClipCell::plain("");
        num.text = "$1,234.50".into();
        num.entry = "1234.5".into();
        num.kind = CellKind::Number(1234.5);
        num.number_format = Some("\"$\"#,##0.00".into());
        num.font = CellFont {
            name: Some("Cambria".into()),
            size_pt: Some(14.0),
            bold: true,
            italic: true,
            underline: Underline::Double,
            strike: true,
            color: Some(Rgb::new(255, 0, 0)),
        };
        num.fill = Some(Rgb::new(255, 255, 0));
        num.borders = Borders {
            left: Some(Edge {
                line: Line::Thin,
                color: None,
            }),
            right: Some(Edge {
                line: Line::Thick,
                color: Some(Rgb::new(0, 0, 255)),
            }),
            top: Some(Edge {
                line: Line::Double,
                color: None,
            }),
            bottom: Some(Edge {
                line: Line::MediumDashed,
                color: None,
            }),
        };
        num.halign = Some(HAlign::Right);
        num.valign = Some(VAlign::Center);
        num.wrap = true;
        let mut f = ClipCell::plain("=A1+1");
        f.text = "6".into();
        f.kind = CellKind::Number(6.0);
        let mut t = ClipCell::plain("a <b> & \"q\"\nline 2");
        t.kind = CellKind::Text;
        SheetClip {
            origin: None,
            rows: vec![vec![num, f], vec![t, ClipCell::default()]],
        }
    }

    #[test]
    fn html_round_trip_keeps_values_and_formats() {
        let clip = formatted();
        let back = SheetClip::from_html(&clip.to_html()).unwrap();
        assert_eq!(back.rows[0][0], clip.rows[0][0]);
        assert_eq!(back.rows[0][1].entry, "=A1+1");
        assert_eq!(back.rows[0][1].kind, CellKind::Number(6.0));
        assert_eq!(back.rows[1][0].text, "a <b> & \"q\"\nline 2");
        assert_eq!(back.rows[1][0].kind, CellKind::Text);
        assert_eq!(back.rows[1][1], ClipCell::default());
    }

    #[test]
    fn reads_excel_style_html() {
        let html = "<html xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><head><style>\
            <!--table {mso-displayed-decimal-separator:\"\\.\";} \
            .xl65 {mso-number-format:\"0\\.00\"; font-weight:700; background:#FFFF00; mso-pattern:black none;} \
            td {padding:0px; color:windowtext; font-size:11.0pt; font-family:Calibri, sans-serif; text-align:general; vertical-align:bottom; white-space:nowrap;} \
            .xl66 {border-top:.5pt solid windowtext; border-bottom:1.5pt solid #FF0000; text-align:center;} --></style></head><body>\
            <!--StartFragment--><table border=0 cellpadding=0 cellspacing=0 width=128>\
            <tr height=20><td class=xl65 align=right x:num=\"1.5\">1.50</td><td class=xl66 x:str>00123</td><td x:num>42</td></tr>\
            <tr><td colspan=2 class=xl66>merged</td><td x:fmla=\"=SUM(A1:C1)\" x:num=\"45.5\">45.5</td></tr>\
            </table><!--EndFragment--></body></html>";
        let c = SheetClip::from_html(html).unwrap();
        assert_eq!(c.rows.len(), 2);
        let a1 = &c.rows[0][0];
        assert_eq!(a1.kind, CellKind::Number(1.5));
        assert_eq!(a1.text, "1.50");
        assert_eq!(a1.number_format.as_deref(), Some("0.00"));
        assert!(a1.font.bold);
        assert_eq!(a1.font.name.as_deref(), Some("Calibri"));
        assert_eq!(a1.font.size_pt, Some(11.0));
        assert_eq!(a1.fill, Some(Rgb::new(255, 255, 0)));
        // The `General` alignment of the style overrides the `align` hint: not an explicit choice.
        assert_eq!(a1.halign, None);
        let b1 = &c.rows[0][1];
        assert_eq!(b1.kind, CellKind::Text);
        assert_eq!(b1.entry, "00123");
        assert_eq!(b1.borders.top.unwrap().line, Line::Thin);
        assert_eq!(b1.borders.bottom.unwrap().line, Line::Medium);
        assert_eq!(b1.borders.bottom.unwrap().color, Some(Rgb::new(255, 0, 0)));
        assert_eq!(c.rows[0][2].kind, CellKind::Number(42.0));
        let a2 = &c.rows[1][0];
        assert_eq!(a2.span, (1, 2));
        assert!(c.rows[1][1].covered);
        assert_eq!(c.rows[1][2].entry, "=SUM(A1:C1)");
    }

    #[test]
    fn rowspan_shifts_the_next_row() {
        let c = SheetClip::from_html(
            "<table><tr><td rowspan=2>a</td><td>b</td></tr><tr><td>c</td></tr></table>",
        )
        .unwrap();
        assert_eq!(c.rows[1].len(), 2);
        assert!(c.rows[1][0].covered);
        assert_eq!(c.rows[1][1].text, "c");
    }

    #[test]
    fn untrusted_html_is_bounded_and_inert() {
        assert!(SheetClip::from_html("<p>no table</p>").is_err());
        let c = SheetClip::from_html(
            "<table><tr><td style=\"background:url(http://example.com/x.png)\">\
             <img src=\"http://example.com/t.gif\"><script>x()</script>ok</td></tr></table>",
        )
        .unwrap();
        assert_eq!(c.rows[0][0].text, "ok");
        assert_eq!(c.rows[0][0].fill, None);
        let wide = format!(
            "<table><tr>{}</tr></table>",
            "<td colspan=16384>x</td>".repeat(2)
        );
        assert_eq!(SheetClip::from_html(&wide), Err(ParseError::TooManyItems));
        let huge_span = "<table><tr><td rowspan=999999999 colspan=16384>x</td></tr></table>";
        let _ = SheetClip::from_html(huge_span);
        let nested = "<table>".repeat(MAX_DEPTH + 2);
        assert_eq!(SheetClip::from_html(&nested), Err(ParseError::TooDeep));
    }

    #[test]
    fn tsv_round_trip_and_quoting() {
        let rows = parse_tsv("a\t\"b\tc\"\r\n\"x\"\"y\"\t\n").unwrap();
        assert_eq!(rows, vec![vec!["a", "b\tc"], vec!["x\"y", ""]]);
        let clip = SheetClip::from_grid(&rows);
        assert_eq!(parse_tsv(&clip.to_tsv()).unwrap(), rows);
        assert_eq!(parse_tsv("").unwrap(), Vec::<Vec<String>>::new());
        assert_eq!(parse_tsv("a\tb").unwrap(), vec![vec!["a", "b"]]);
    }

    #[test]
    fn number_format_codes() {
        assert_eq!(decode_number_format("\"0\\.00\"").as_deref(), Some("0.00"));
        assert_eq!(
            decode_number_format("\"\\0022\\$\\0022\\#\\,\\#\\#0\"").as_deref(),
            Some("\"$\"#,##0")
        );
        assert_eq!(decode_number_format("General"), None);
        assert_eq!(decode_number_format("Percent").as_deref(), Some("0%"));
        let code = "[$-409]d\\-mmm\\-yy;@";
        assert_eq!(
            decode_number_format(&encode_number_format(code)).as_deref(),
            Some(code)
        );
    }
}
