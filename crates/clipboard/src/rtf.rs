//! RTF ("Rich Text Format") to and from [`RichText`].
//!
//! The reader is a small state machine over control words: character formatting, paragraph
//! alignment, bullets and numbers (from the list text Word writes), hyperlink fields and tab
//! separated table cells. Pictures, objects, headers, footnotes and every unknown destination
//! are skipped, never decoded or loaded. Nesting and size are bounded by [`crate::limits`].
//! Single-byte escapes (`\'hh`) are read as Windows-1252.

use std::fmt::Write as _;

use crate::limits::{MAX_CHARS, MAX_DEPTH, MAX_PARAS, MAX_RTF_BYTES};
use crate::model::{Align, CharFmt, ListKind, Para, ParseError, Rgb, RichText, Run, safe_link};

const CP1252: [char; 32] = [
    '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}', '\u{8f}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
];

fn cp1252(b: u8) -> char {
    match b {
        0x80..=0x9f => CP1252[usize::from(b - 0x80)],
        b => char::from(b),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dest {
    Normal,
    Skip,
    FontTbl,
    ColorTbl,
    ListText,
    FldInst,
}

#[derive(Debug, Clone)]
struct St {
    fmt: CharFmt,
    font: Option<i32>,
    uc: usize,
    skip: usize,
    dest: Dest,
    align: Align,
}

const SKIP_DEST: &[&str] = &[
    "stylesheet",
    "info",
    "pict",
    "object",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "footnote",
    "annotation",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "datastore",
    "listtable",
    "listoverridetable",
    "rsidtbl",
    "generator",
    "xmlnstbl",
    "filetbl",
    "revtbl",
    "pgdsctbl",
    "background",
    "shppict",
    "nonshppict",
    "shp",
    "private",
    "bkmkstart",
    "bkmkend",
    "mmath",
    "falt",
    "panose",
    "fldtype",
];

const UNDERLINE: &[&str] = &[
    "ul",
    "uld",
    "uldash",
    "uldashd",
    "uldashdd",
    "uldb",
    "ulhwave",
    "ulldash",
    "ulth",
    "ulthd",
    "ulthdash",
    "ulthdashd",
    "ulthdashdd",
    "ulthldash",
    "ululdbwave",
    "ulw",
    "ulwave",
];

struct Reader<'a> {
    d: &'a [u8],
    i: usize,
    stack: Vec<St>,
    fonts: Vec<(i32, String)>,
    colors: Vec<Option<Rgb>>,
    paras: Vec<Para>,
    cur: Para,
    cur_used: bool,
    chars: usize,
    pending_high: Option<u16>,
    font_idx: i32,
    font_name: String,
    color_cur: [Option<u8>; 3],
    list_buf: String,
    fld_buf: String,
    field_link: Option<(String, usize)>,
}

fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

impl Reader<'_> {
    fn st(&mut self) -> &mut St {
        let n = self.stack.len();
        &mut self.stack[n - 1]
    }

    fn end_para(&mut self) -> Result<(), ParseError> {
        if self.paras.len() >= MAX_PARAS {
            return Err(ParseError::TooManyItems);
        }
        let mut p = std::mem::take(&mut self.cur);
        p.runs.retain(|r| !r.text.is_empty());
        self.paras.push(p);
        self.cur_used = false;
        // Paragraph properties carry to the next paragraph until \pard.
        let align = self.stack.last().map_or(Align::Left, |s| s.align);
        self.cur.align = align;
        Ok(())
    }

    fn put(&mut self, c: char) -> Result<(), ParseError> {
        let dest = self.stack.last().map_or(Dest::Normal, |s| s.dest);
        match dest {
            Dest::Skip => {}
            Dest::FontTbl => {
                if c == ';' {
                    let name = self.font_name.trim().to_owned();
                    if !name.is_empty() {
                        self.fonts.push((self.font_idx, name));
                    }
                    self.font_name.clear();
                } else if self.font_name.len() < 128 {
                    self.font_name.push(c);
                }
            }
            Dest::ColorTbl => {
                if c == ';' {
                    let [r, g, b] = self.color_cur;
                    self.colors.push(match (r, g, b) {
                        (None, None, None) => None,
                        (r, g, b) => Some(Rgb::new(r.unwrap_or(0), g.unwrap_or(0), b.unwrap_or(0))),
                    });
                    self.color_cur = [None; 3];
                }
            }
            Dest::ListText => {
                if self.list_buf.len() < 32 {
                    self.list_buf.push(c);
                }
            }
            Dest::FldInst => {
                if self.fld_buf.len() < 4096 {
                    self.fld_buf.push(c);
                }
            }
            Dest::Normal => {
                if c.is_control() && !matches!(c, '\t' | '\n') {
                    return Ok(());
                }
                self.chars += 1;
                if self.chars > MAX_CHARS {
                    return Err(ParseError::TooLarge);
                }
                let Some(st) = self.stack.last() else {
                    return Ok(());
                };
                let font = st
                    .font
                    .and_then(|fi| self.fonts.iter().find(|(i, _)| *i == fi))
                    .map(|(_, n)| n);
                let link = self.field_link.as_ref().map(|(u, _)| u.as_str());
                self.cur_used = true;
                let same = self.cur.runs.last().is_some_and(|r| {
                    let (a, b) = (&r.fmt, &st.fmt);
                    a.font.as_ref() == font
                        && a.size_pt == b.size_pt
                        && a.bold == b.bold
                        && a.italic == b.italic
                        && a.underline == b.underline
                        && a.strike == b.strike
                        && a.superscript == b.superscript
                        && a.subscript == b.subscript
                        && a.color == b.color
                        && a.highlight == b.highlight
                        && r.link.as_deref() == link
                });
                if same {
                    if let Some(r) = self.cur.runs.last_mut() {
                        r.text.push(c);
                    }
                } else {
                    let mut fmt = st.fmt.clone();
                    fmt.font = font.cloned();
                    self.cur.runs.push(Run {
                        text: c.to_string(),
                        fmt,
                        link: link.map(str::to_owned),
                    });
                }
            }
        }
        Ok(())
    }

    /// A character from the file's text, honouring the `\uc` fallback skip.
    fn text_char(&mut self, c: char) -> Result<(), ParseError> {
        if self.stack.last().is_some_and(|s| s.skip > 0) {
            self.st().skip -= 1;
            return Ok(());
        }
        self.put(c)
    }

    fn unicode(&mut self, n: i32) -> Result<(), ParseError> {
        let unit = (n & 0xFFFF) as u16;
        let c = match unit {
            0xD800..=0xDBFF => {
                self.pending_high = Some(unit);
                None
            }
            0xDC00..=0xDFFF => self.pending_high.take().and_then(|h| {
                char::from_u32(
                    0x10000 + ((u32::from(h) - 0xD800) << 10) + (u32::from(unit) - 0xDC00),
                )
            }),
            u => char::from_u32(u32::from(u)),
        };
        if let Some(c) = c {
            self.put(c)?;
        }
        let uc = self.st().uc;
        self.st().skip = uc;
        Ok(())
    }

    fn control(&mut self, name: &str, param: Option<i32>, star: bool) -> Result<(), ParseError> {
        let dest = self.stack.last().map_or(Dest::Normal, |s| s.dest);
        if dest == Dest::Skip {
            return Ok(());
        }
        // Destinations.
        if star && name == "fldinst" {
            self.st().dest = Dest::FldInst;
            self.fld_buf.clear();
            return Ok(());
        }
        if star {
            self.st().dest = Dest::Skip;
            return Ok(());
        }
        match name {
            "fonttbl" => {
                self.st().dest = Dest::FontTbl;
                return Ok(());
            }
            "colortbl" => {
                self.st().dest = Dest::ColorTbl;
                self.color_cur = [None; 3];
                // A table that starts with `;` has the automatic colour as entry 0.
                return Ok(());
            }
            "listtext" | "pntext" => {
                self.st().dest = Dest::ListText;
                self.list_buf.clear();
                return Ok(());
            }
            n if SKIP_DEST.contains(&n) => {
                self.st().dest = Dest::Skip;
                return Ok(());
            }
            _ => {}
        }
        match dest {
            Dest::FontTbl => {
                if name == "f" {
                    self.font_idx = param.unwrap_or(0);
                    self.font_name.clear();
                }
                return Ok(());
            }
            Dest::ColorTbl => {
                let v = param.map(clamp_u8);
                match name {
                    "red" => self.color_cur[0] = v,
                    "green" => self.color_cur[1] = v,
                    "blue" => self.color_cur[2] = v,
                    _ => {}
                }
                return Ok(());
            }
            Dest::ListText => {
                if name == "u" {
                    self.unicode(param.unwrap_or(0))?;
                }
                return Ok(());
            }
            Dest::FldInst => return Ok(()),
            _ => {}
        }
        let on = param != Some(0);
        match name {
            "b" => self.st().fmt.bold = on,
            "i" => self.st().fmt.italic = on,
            "strike" | "striked" => self.st().fmt.strike = on,
            "ulnone" => self.st().fmt.underline = false,
            n if UNDERLINE.contains(&n) => self.st().fmt.underline = on,
            "super" => {
                self.st().fmt.superscript = true;
                self.st().fmt.subscript = false;
            }
            "sub" => {
                self.st().fmt.subscript = true;
                self.st().fmt.superscript = false;
            }
            "nosupersub" => {
                self.st().fmt.superscript = false;
                self.st().fmt.subscript = false;
            }
            "plain" => {
                let st = self.st();
                st.fmt = CharFmt::default();
                st.font = None;
            }
            "f" => self.st().font = param,
            "fs" => {
                if let Some(p) = param {
                    self.st().fmt.size_pt = Some(p as f32 / 2.0);
                }
            }
            "cf" => {
                let c = param
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| self.colors.get(i).copied().flatten());
                self.st().fmt.color = c;
            }
            "highlight" | "chcbpat" => {
                let c = param
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| self.colors.get(i).copied().flatten());
                self.st().fmt.highlight = c;
            }
            "uc" => self.st().uc = param.unwrap_or(1).clamp(0, 8) as usize,
            "u" => self.unicode(param.unwrap_or(0))?,
            "ql" => self.set_align(Align::Left),
            "qc" => self.set_align(Align::Center),
            "qr" => self.set_align(Align::Right),
            "qj" => self.set_align(Align::Justify),
            "pard" => {
                self.set_align(Align::Left);
                self.cur.list = None;
                self.cur.heading = None;
            }
            "outlinelevel" => {
                if let Some(l @ 0..=5) = param {
                    self.cur.heading = Some(l as u8 + 1);
                }
            }
            "par" | "row" | "sect" => self.end_para()?,
            "line" => self.put('\n')?,
            "cell" | "nestcell" | "tab" => self.put('\t')?,
            "emdash" => self.put('\u{2014}')?,
            "endash" => self.put('\u{2013}')?,
            "emspace" => self.put('\u{2003}')?,
            "enspace" => self.put('\u{2002}')?,
            "bullet" => self.put('\u{2022}')?,
            "lquote" => self.put('\u{2018}')?,
            "rquote" => self.put('\u{2019}')?,
            "ldblquote" => self.put('\u{201c}')?,
            "rdblquote" => self.put('\u{201d}')?,
            _ => {}
        }
        Ok(())
    }

    fn set_align(&mut self, a: Align) {
        self.st().align = a;
        self.cur.align = a;
    }

    fn close_group(&mut self) {
        let Some(g) = self.stack.pop() else { return };
        match g.dest {
            Dest::ListText => {
                let first = self.list_buf.trim().chars().next();
                if let Some(c) = first {
                    self.cur.list = Some(if c.is_alphanumeric() {
                        ListKind::Numbered
                    } else {
                        ListKind::Bullet
                    });
                }
            }
            Dest::FldInst if self.stack.last().map(|s| s.dest) != Some(Dest::FldInst) => {
                let f = self.fld_buf.clone();
                if let Some(p) = f.to_ascii_uppercase().find("HYPERLINK") {
                    let rest = &f[p + 9..];
                    if let Some(a) = rest.find('"')
                        && let Some(b) = rest[a + 1..].find('"')
                        && let Some(url) = safe_link(&rest[a + 1..a + 1 + b])
                    {
                        self.field_link = Some((url, self.stack.len()));
                    }
                }
            }
            _ => {}
        }
        if let Some((_, depth)) = &self.field_link
            && self.stack.len() < *depth
        {
            self.field_link = None;
        }
    }
}

/// Reads an RTF document.
pub fn parse(data: &[u8]) -> Result<RichText, ParseError> {
    if data.len() > MAX_RTF_BYTES {
        return Err(ParseError::TooLarge);
    }
    let mut data = data;
    while let [rest @ .., 0] = data {
        data = rest;
    }
    if !data.starts_with(b"{\\rtf") {
        return Err(ParseError::Malformed("not RTF"));
    }
    let mut r = Reader {
        d: data,
        i: 0,
        stack: vec![St {
            fmt: CharFmt::default(),
            font: None,
            uc: 1,
            skip: 0,
            dest: Dest::Normal,
            align: Align::Left,
        }],
        fonts: Vec::new(),
        colors: Vec::new(),
        paras: Vec::new(),
        cur: Para::default(),
        cur_used: false,
        chars: 0,
        pending_high: None,
        font_idx: 0,
        font_name: String::new(),
        color_cur: [None; 3],
        list_buf: String::new(),
        fld_buf: String::new(),
        field_link: None,
    };
    r.i = 1; // the outer '{'
    let n = r.d.len();
    let mut star = false;
    while r.i < n {
        let c = r.d[r.i];
        r.i += 1;
        match c {
            b'{' => {
                if r.stack.len() >= MAX_DEPTH {
                    return Err(ParseError::TooDeep);
                }
                let top = r.stack.last().cloned().expect("group stack is never empty");
                r.stack.push(top);
                // A group stays in its parent's destination.
                r.st().skip = 0;
                star = false;
            }
            b'}' => {
                if r.stack.len() <= 1 {
                    break;
                }
                r.close_group();
            }
            b'\\' => {
                let Some(&nx) = r.d.get(r.i) else { break };
                if nx.is_ascii_alphabetic() {
                    let s = r.i;
                    while r.i < n && r.d[r.i].is_ascii_alphabetic() && r.i - s < 32 {
                        r.i += 1;
                    }
                    let name = String::from_utf8_lossy(&r.d[s..r.i]).into_owned();
                    let mut param = None;
                    let ps = r.i;
                    if r.i < n && r.d[r.i] == b'-' {
                        r.i += 1;
                    }
                    let ds = r.i;
                    while r.i < n && r.d[r.i].is_ascii_digit() && r.i - ds < 10 {
                        r.i += 1;
                    }
                    if r.i > ds {
                        param = std::str::from_utf8(&r.d[ps..r.i])
                            .ok()
                            .and_then(|t| t.parse::<i32>().ok());
                    } else {
                        r.i = ps;
                    }
                    if r.i < n && r.d[r.i] == b' ' {
                        r.i += 1;
                    }
                    r.control(&name, param, star)?;
                    star = false;
                } else {
                    r.i += 1;
                    match nx {
                        b'\'' => {
                            let hex =
                                r.d.get(r.i..r.i + 2)
                                    .and_then(|h| std::str::from_utf8(h).ok());
                            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                                r.i += 2;
                                if r.stack.last().map(|s| s.dest) != Some(Dest::Skip) {
                                    r.text_char(cp1252(v))?;
                                }
                            }
                        }
                        b'*' => star = true,
                        b'\\' | b'{' | b'}'
                            if r.stack.last().map(|s| s.dest) != Some(Dest::Skip) =>
                        {
                            r.text_char(char::from(nx))?;
                        }
                        b'~' => r.text_char('\u{a0}')?,
                        b'_' => r.text_char('-')?,
                        b'\n' | b'\r' => r.control("par", None, false)?,
                        _ => {}
                    }
                }
            }
            b'\r' | b'\n' => {}
            c => {
                if r.stack.last().map(|s| s.dest) != Some(Dest::Skip) {
                    r.text_char(cp1252(c))?;
                }
            }
        }
    }
    if r.cur_used || !r.cur.runs.is_empty() {
        r.end_para()?;
    }
    let mut rt = RichText { paras: r.paras };
    rt.sanitize();
    rt.normalize();
    Ok(rt)
}

fn rtf_escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            '\n' => out.push_str("\\line "),
            c if c.is_control() => {}
            c if c.is_ascii() => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{}?", *u as i16);
                }
            }
        }
    }
}

/// Serializes the paragraphs as an RTF document.
pub fn to_rtf(rt: &RichText) -> String {
    let mut fonts: Vec<&str> = vec!["Calibri"];
    let mut colors: Vec<Rgb> = Vec::new();
    for r in rt.paras.iter().flat_map(|p| p.runs.iter()) {
        if let Some(f) = r.fmt.font.as_deref()
            && !fonts.contains(&f)
        {
            fonts.push(f);
        }
        for c in [r.fmt.color, r.fmt.highlight].into_iter().flatten() {
            if !colors.contains(&c) {
                colors.push(c);
            }
        }
    }
    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1{\\fonttbl");
    for (i, f) in fonts.iter().enumerate() {
        let mut name = String::new();
        rtf_escape(&f.replace(';', ""), &mut name);
        let _ = write!(out, "{{\\f{i}\\fnil\\fcharset0 {name};}}");
    }
    out.push_str("}{\\colortbl;");
    for c in &colors {
        let _ = write!(out, "\\red{}\\green{}\\blue{};", c.r, c.g, c.b);
    }
    out.push_str("}\r\n");
    let mut number = 0;
    for p in &rt.paras {
        out.push_str("\\pard\\plain");
        out.push_str(match p.align {
            Align::Left => "\\ql",
            Align::Center => "\\qc",
            Align::Right => "\\qr",
            Align::Justify => "\\qj",
        });
        if let Some(h @ 1..=6) = p.heading {
            let _ = write!(out, "\\outlinelevel{}", h - 1);
        }
        if p.list.is_some() {
            out.push_str("\\li720\\fi-360");
        }
        out.push(' ');
        match p.list {
            Some(ListKind::Bullet) => {
                number = 0;
                out.push_str("\\u8226?\\tab ");
            }
            Some(ListKind::Numbered) => {
                number += 1;
                let _ = write!(out, "{number}.\\tab ");
            }
            None => number = 0,
        }
        for r in &p.runs {
            let link = r.link.as_deref().and_then(safe_link);
            if let Some(l) = &link {
                let mut esc = String::new();
                rtf_escape(l, &mut esc);
                let _ = write!(
                    out,
                    "{{\\field{{\\*\\fldinst{{HYPERLINK \"{esc}\"}}}}{{\\fldrslt"
                );
            }
            out.push('{');
            let ctrl_start = out.len();
            let f = &r.fmt;
            if let Some(n) = f.font.as_deref()
                && let Some(i) = fonts.iter().position(|x| *x == n)
            {
                let _ = write!(out, "\\f{i}");
            }
            if let Some(s) = f.size_pt {
                let _ = write!(out, "\\fs{}", (s * 2.0).round() as i32);
            }
            if f.bold {
                out.push_str("\\b");
            }
            if f.italic {
                out.push_str("\\i");
            }
            if f.underline {
                out.push_str("\\ul");
            }
            if f.strike {
                out.push_str("\\strike");
            }
            if f.superscript {
                out.push_str("\\super");
            } else if f.subscript {
                out.push_str("\\sub");
            }
            if let Some(c) = f.color
                && let Some(i) = colors.iter().position(|x| *x == c)
            {
                let _ = write!(out, "\\cf{}", i + 1);
            }
            if let Some(c) = f.highlight
                && let Some(i) = colors.iter().position(|x| *x == c)
            {
                let _ = write!(out, "\\highlight{}", i + 1);
            }
            if out.len() > ctrl_start {
                out.push(' ');
            }
            rtf_escape(&r.text, &mut out);
            out.push('}');
            if link.is_some() {
                out.push_str("}}");
            }
        }
        out.push_str("\\par\r\n");
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_word_style_rtf() {
        let rtf = br"{\rtf1\ansi\ansicpg1252\deff0\nouicompat{\fonttbl{\f0\fnil\fcharset0 Calibri;}{\f1\fswiss Arial;}}
{\colortbl ;\red255\green0\blue0;\red0\green0\blue255;}
{\*\generator Riched20 10.0}\viewkind4\uc1
\pard\qc\f1\fs24 Hello \b bold\b0  and {\cf1\i red}\par
\pard\ql caf\'e9 \u8364?\u-10179?\u-8704? end\par
}";
        let rt = parse(rtf).unwrap();
        assert_eq!(rt.paras.len(), 2);
        assert_eq!(rt.paras[0].align, Align::Center);
        assert_eq!(rt.paras[0].text(), "Hello bold and red");
        let r0 = &rt.paras[0].runs[0];
        assert_eq!(r0.fmt.font.as_deref(), Some("Arial"));
        assert_eq!(r0.fmt.size_pt, Some(12.0));
        let red = rt.paras[0].runs.last().unwrap();
        assert_eq!(red.fmt.color, Some(Rgb::new(255, 0, 0)));
        assert!(red.fmt.italic);
        assert_eq!(rt.paras[1].align, Align::Left);
        assert_eq!(rt.paras[1].text(), "caf\u{e9} \u{20ac}\u{1F600} end");
    }

    #[test]
    fn skips_pictures_and_unknown_destinations() {
        let rtf =
            br"{\rtf1{\pict\wmetafile8 0102030405}{\*\unknown secret}{\header hidden}visible\par}";
        assert_eq!(parse(rtf).unwrap().plain(), "visible");
    }

    #[test]
    fn list_text_and_fields() {
        let rtf = br#"{\rtf1{\fonttbl{\f0 Arial;}}\pard{\listtext\f0 1.\tab}\ls1 One\par
\pard{\listtext\'b7\tab}Two\par
\pard{\field{\*\fldinst{HYPERLINK "https://example.com/x"}}{\fldrslt site}} after\par}"#;
        let rt = parse(rtf).unwrap();
        assert_eq!(rt.paras[0].list, Some(ListKind::Numbered));
        assert_eq!(rt.paras[1].list, Some(ListKind::Bullet));
        assert_eq!(rt.paras[0].text(), "One");
        assert_eq!(rt.paras[2].text(), "site after");
        assert_eq!(
            rt.paras[2].runs[0].link.as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(rt.paras[2].runs[1].link, None);
    }

    #[test]
    fn bad_input_is_refused_not_crashed() {
        assert!(parse(b"plain").is_err());
        let deep = format!("{{\\rtf1{}", "{".repeat(MAX_DEPTH + 2));
        assert_eq!(parse(deep.as_bytes()), Err(ParseError::TooDeep));
        for s in [
            &b"{\\rtf1\\"[..],
            b"{\\rtf1\\'",
            b"{\\rtf1\\u",
            b"{\\rtf1{",
            b"{\\rtf1\\u99999999999 x}",
        ] {
            let _ = parse(s);
        }
    }

    #[test]
    fn serializer_round_trips() {
        let mut rt = RichText::default();
        rt.paras.push(Para {
            runs: vec![
                Run {
                    text: "A {b} \\ \u{e9}\u{1F600}\ttab".into(),
                    fmt: CharFmt {
                        font: Some("Arial".into()),
                        size_pt: Some(12.5),
                        bold: true,
                        underline: true,
                        color: Some(Rgb::new(1, 2, 3)),
                        highlight: Some(Rgb::new(255, 255, 0)),
                        ..CharFmt::default()
                    },
                    link: None,
                },
                Run {
                    text: "site".into(),
                    fmt: CharFmt::default(),
                    link: Some("https://example.com/a".into()),
                },
            ],
            align: Align::Justify,
            heading: None,
            list: None,
        });
        rt.paras.push(Para {
            runs: vec![Run {
                text: "item".into(),
                ..Run::default()
            }],
            list: Some(ListKind::Numbered),
            ..Para::default()
        });
        let back = parse(to_rtf(&rt).as_bytes()).unwrap();
        assert_eq!(back.paras[0], rt.paras[0]);
        // Lists travel as visible text in RTF; the HTML and private formats keep the list.
        assert_eq!(back.paras[1].text(), "1.\titem");
    }
}
