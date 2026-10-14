//! HTML to and from [`RichText`]: what the Writer puts on the clipboard as "HTML Format" and
//! reads back from Word, browsers and other programs.
//!
//! Reading is tolerant and bounded: elements are interpreted for character and paragraph
//! formatting only. Images, scripts, styles' `url(...)`, forms and any other element are never
//! loaded or run, and `href` values are kept only for `http`, `https` and `mailto`.

use std::fmt::Write as _;

use crate::css::{self, StyleSheet};
use crate::html::{self, Tok, VOID};
use crate::limits::{MAX_CHARS, MAX_DEPTH, MAX_PARAS};
use crate::model::{Align, CharFmt, ListKind, Para, ParseError, Rgb, RichText, Run, safe_link};

const SKIP: &[&str] = &[
    "head", "title", "script", "style", "template", "xml", "object", "iframe", "noscript",
    "applet", "embed", "svg", "math", "select", "button",
];

#[derive(Debug, Clone, Default)]
struct Frame {
    name: String,
    fmt: CharFmt,
    link: Option<String>,
    skip: bool,
    pre: bool,
    align: Option<Align>,
    heading: Option<u8>,
    /// Set on `li`: the list kind of the enclosing list.
    item: Option<ListKind>,
    /// Set on `ul` and `ol`.
    list: Option<ListKind>,
    /// Paragraph count when a block frame opened (an empty block still yields a paragraph).
    mark: Option<usize>,
}

struct Reader {
    paras: Vec<Para>,
    cur: Option<Para>,
    last_space: bool,
    chars: usize,
    stack: Vec<Frame>,
    cell: usize,
    error: Option<ParseError>,
}

fn align_of(v: &str) -> Option<Align> {
    match v.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(Align::Left),
        "center" | "middle" => Some(Align::Center),
        "right" | "end" => Some(Align::Right),
        "justify" => Some(Align::Justify),
        _ => None,
    }
}

fn font_size_attr(v: &str) -> Option<f32> {
    let n: i32 = v.trim().parse().ok()?;
    Some(match n {
        i32::MIN..=1 => 7.5,
        2 => 10.0,
        3 => 12.0,
        4 => 13.5,
        5 => 18.0,
        6 => 24.0,
        _ => 36.0,
    })
}

/// Applies one declaration to character formatting. Returns the paragraph alignment it asks for.
fn apply_decl(k: &str, v: &str, fmt: &mut CharFmt, align: &mut Option<Align>, hidden: &mut bool) {
    let lv = v.to_ascii_lowercase();
    match k {
        "font-family" => {
            if let Some(f) = css::first_family(v) {
                fmt.font = Some(f);
            }
        }
        "font-size" => {
            if let Some(p) = css::length_pt(v) {
                fmt.size_pt = Some(p);
            }
        }
        "font-weight" => {
            fmt.bold = lv == "bold" || lv == "bolder" || lv.parse::<u32>().is_ok_and(|w| w >= 600);
        }
        "font-style" => fmt.italic = lv.starts_with("italic") || lv.starts_with("oblique"),
        "text-decoration" | "text-decoration-line" => {
            fmt.underline = lv.contains("underline");
            fmt.strike = lv.contains("line-through");
        }
        "color" => {
            if let Some(c) = css::parse_color(v)
                && lv != "windowtext"
            {
                fmt.color = Some(c);
            }
        }
        "background" | "background-color" | "mso-highlight" => {
            if lv != "windowtext"
                && let Some(c) = css::first_color(v)
            {
                fmt.highlight = Some(c);
            }
        }
        "vertical-align" => {
            fmt.superscript = lv == "super";
            fmt.subscript = lv == "sub";
        }
        "text-align" => *align = align_of(v),
        "display" if lv == "none" => *hidden = true,
        "mso-hide" if lv == "all" => *hidden = true,
        _ => {}
    }
}

impl Reader {
    fn top(&self) -> Frame {
        self.stack.last().cloned().unwrap_or_default()
    }

    fn para_attrs(&self) -> Para {
        let mut p = Para::default();
        for f in self.stack.iter().rev() {
            if p.heading.is_none() {
                p.heading = f.heading;
            }
            if p.list.is_none() {
                p.list = f.item;
            }
        }
        p.align = self
            .stack
            .iter()
            .rev()
            .find_map(|f| f.align)
            .unwrap_or_default();
        p
    }

    fn ensure(&mut self) {
        if self.cur.is_none() {
            self.cur = Some(self.para_attrs());
            self.last_space = true;
        }
    }

    fn flush(&mut self) {
        if let Some(mut p) = self.cur.take() {
            if let Some(last) = p.runs.last_mut() {
                let t = last.text.trim_end_matches(' ').len();
                last.text.truncate(t);
            }
            p.runs.retain(|r| !r.text.is_empty());
            if self.paras.len() >= MAX_PARAS {
                self.error = Some(ParseError::TooManyItems);
                return;
            }
            self.paras.push(p);
        }
    }

    fn push_char(&mut self, c: char) {
        self.chars += 1;
        if self.chars > MAX_CHARS {
            self.error = Some(ParseError::TooLarge);
            return;
        }
        self.ensure();
        let top = self.stack.last();
        let Some(p) = self.cur.as_mut() else { return };
        let same = match (p.runs.last(), top) {
            (Some(r), Some(f)) => r.fmt == f.fmt && r.link == f.link,
            (Some(r), None) => r.fmt == CharFmt::default() && r.link.is_none(),
            _ => false,
        };
        if same {
            if let Some(r) = p.runs.last_mut() {
                r.text.push(c);
            }
        } else {
            p.runs.push(Run {
                text: c.to_string(),
                fmt: top.map(|f| f.fmt.clone()).unwrap_or_default(),
                link: top.and_then(|f| f.link.clone()),
            });
        }
    }

    fn text(&mut self, s: &str) {
        let (skip, pre) = self
            .stack
            .last()
            .map_or((false, false), |f| (f.skip, f.pre));
        if skip {
            return;
        }
        for c in s.chars() {
            if self.error.is_some() {
                return;
            }
            if pre {
                match c {
                    '\r' => {}
                    '\n' => self.push_char('\n'),
                    c if c.is_control() && c != '\t' => {}
                    c => self.push_char(c),
                }
            } else if c == '\u{a0}' {
                self.ensure();
                self.last_space = false;
                self.push_char(' ');
            } else if c.is_whitespace() {
                // White space at the start of a paragraph (or between blocks) is dropped.
                if self.cur.is_none() {
                    continue;
                }
                if !self.last_space {
                    self.last_space = true;
                    self.push_char(' ');
                }
            } else if !c.is_control() {
                self.ensure();
                self.last_space = false;
                self.push_char(c);
            }
        }
    }

    fn open(&mut self, name: &str, attrs: &[(String, String)], sheet: &StyleSheet) {
        let parent = self.top();
        let mut f = Frame {
            name: name.to_owned(),
            fmt: parent.fmt.clone(),
            link: parent.link.clone(),
            skip: parent.skip || SKIP.contains(&name) || name.starts_with("v:"),
            pre: parent.pre,
            ..Frame::default()
        };
        let mut hidden = false;
        let mut block_align = None;
        match name {
            "b" | "strong" => f.fmt.bold = true,
            "i" | "em" | "cite" | "var" | "dfn" => f.fmt.italic = true,
            "u" | "ins" => f.fmt.underline = true,
            "s" | "strike" | "del" => f.fmt.strike = true,
            "sup" => {
                f.fmt.superscript = true;
                f.fmt.subscript = false;
            }
            "sub" => {
                f.fmt.subscript = true;
                f.fmt.superscript = false;
            }
            "mark" => f.fmt.highlight = Some(Rgb::new(255, 255, 0)),
            "code" | "tt" | "kbd" | "samp" => f.fmt.font = Some("Courier New".into()),
            "pre" => f.pre = true,
            "a" => {
                if let Some(h) = html::attr(attrs, "href") {
                    f.link = safe_link(h);
                }
            }
            "font" => {
                if let Some(face) = html::attr(attrs, "face")
                    && let Some(fam) = css::first_family(face)
                {
                    f.fmt.font = Some(fam);
                }
                if let Some(s) = html::attr(attrs, "size").and_then(font_size_attr) {
                    f.fmt.size_pt = Some(s);
                }
                if let Some(c) = html::attr(attrs, "color").and_then(css::parse_color) {
                    f.fmt.color = Some(c);
                }
            }
            "ul" => f.list = Some(ListKind::Bullet),
            "ol" => f.list = Some(ListKind::Numbered),
            "li" => {
                f.item = Some(
                    self.stack
                        .iter()
                        .rev()
                        .find_map(|p| p.list)
                        .unwrap_or(ListKind::Bullet),
                );
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                f.heading = name[1..].parse().ok();
            }
            _ => {}
        }
        if let Some(a) = html::attr(attrs, "align").and_then(align_of) {
            block_align = Some(a);
        }
        let classes: Vec<&str> = html::attr(attrs, "class")
            .map(|c| c.split_whitespace().collect())
            .unwrap_or_default();
        let mut decls = sheet.matching(name, &classes);
        if let Some(s) = html::attr(attrs, "style") {
            decls.extend(css::declarations(s));
        }
        for (k, v) in &decls {
            apply_decl(k, v, &mut f.fmt, &mut block_align, &mut hidden);
        }
        if decls
            .iter()
            .any(|(k, v)| k == "white-space" && v.starts_with("pre"))
        {
            f.pre = true;
        }
        f.fmt.sanitize();
        if hidden {
            f.skip = true;
        }
        f.align = block_align;
        let block = matches!(
            name,
            "p" | "div"
                | "li"
                | "tr"
                | "pre"
                | "blockquote"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "ul"
                | "ol"
                | "table"
                | "center"
                | "dd"
                | "dt"
                | "hr"
                | "address"
        );
        if name == "center" {
            f.align = Some(Align::Center);
        }
        if block && !f.skip {
            self.flush();
            if matches!(
                name,
                "p" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre"
            ) {
                f.mark = Some(self.paras.len());
            }
        }
        if name == "tr" {
            self.cell = 0;
        }
        if matches!(name, "td" | "th") && !f.skip {
            if self.cell > 0 {
                self.last_space = false;
                self.push_char('\t');
            }
            self.cell += 1;
        }
        if self.stack.len() >= MAX_DEPTH {
            self.error = Some(ParseError::TooDeep);
            return;
        }
        self.stack.push(f);
    }

    fn close(&mut self, name: &str) {
        let Some(idx) = self.stack.iter().rposition(|f| f.name == name) else {
            return;
        };
        let attrs = self.para_attrs();
        let frame = self.stack[idx].clone();
        self.stack.truncate(idx);
        let block = matches!(
            name,
            "p" | "div"
                | "li"
                | "tr"
                | "pre"
                | "blockquote"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "ul"
                | "ol"
                | "table"
                | "center"
                | "dd"
                | "dt"
                | "address"
        );
        if block && !frame.skip {
            self.flush();
            if let Some(m) = frame.mark
                && self.paras.len() == m
                && self.error.is_none()
            {
                // The block held nothing: Word writes an empty paragraph this way.
                self.paras.push(attrs);
            }
        }
    }

    fn line_break(&mut self) {
        if self.top().skip {
            return;
        }
        self.push_char('\n');
        self.last_space = true;
    }
}

/// Reads an HTML document or fragment. `HTML Format` data goes through
/// [`crate::cfhtml::unwrap`] first.
pub fn parse(html_src: &str) -> Result<RichText, ParseError> {
    let toks = html::tokenize(html_src)?;
    let mut sheet = StyleSheet::default();
    for s in html::style_text(&toks) {
        sheet.add(s);
    }
    let mut r = Reader {
        paras: Vec::new(),
        cur: None,
        last_space: true,
        chars: 0,
        stack: Vec::new(),
        cell: 0,
        error: None,
    };
    for t in html::fragment(&toks) {
        if let Some(e) = r.error.take() {
            return Err(e);
        }
        match t {
            Tok::Open {
                name,
                attrs,
                self_close,
            } => {
                if name == "br" {
                    r.line_break();
                } else if name == "hr" {
                    r.flush();
                } else if VOID.contains(&name.as_str()) {
                    // Images and other void elements carry no text and are never fetched.
                } else {
                    r.open(name, attrs, &sheet);
                    if *self_close {
                        r.close(name);
                    }
                }
            }
            Tok::Close(name) => r.close(name),
            Tok::Text(s) => r.text(s),
            _ => {}
        }
    }
    if let Some(e) = r.error.take() {
        return Err(e);
    }
    r.flush();
    let mut rt = RichText { paras: r.paras };
    if let Some(e) = r.error {
        return Err(e);
    }
    rt.sanitize();
    rt.normalize();
    Ok(rt)
}

fn escape(text: &str, out: &mut String) {
    let mut prev_space = true;
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("<span style=\"white-space:pre\">\t</span>"),
            '\n' => out.push_str("<br>"),
            ' ' => {
                // Keep runs of spaces and leading spaces.
                out.push_str(if prev_space { "&nbsp;" } else { " " });
            }
            c => out.push(c),
        }
        prev_space = c == ' ';
    }
}

fn run_css(f: &CharFmt) -> String {
    let mut css = String::new();
    if let Some(n) = &f.font {
        let _ = write!(
            css,
            "font-family:'{}';",
            n.replace(['\'', '"', '<', '>', ';', '&'], "")
        );
    }
    if let Some(s) = f.size_pt {
        let _ = write!(css, "font-size:{}pt;", trim_f(s));
    }
    if f.bold {
        css.push_str("font-weight:bold;");
    }
    if f.italic {
        css.push_str("font-style:italic;");
    }
    match (f.underline, f.strike) {
        (true, true) => css.push_str("text-decoration:underline line-through;"),
        (true, false) => css.push_str("text-decoration:underline;"),
        (false, true) => css.push_str("text-decoration:line-through;"),
        _ => {}
    }
    if f.superscript {
        css.push_str("vertical-align:super;");
    } else if f.subscript {
        css.push_str("vertical-align:sub;");
    }
    if let Some(c) = f.color {
        let _ = write!(css, "color:{};", c.css());
    }
    if let Some(c) = f.highlight {
        let _ = write!(css, "background:{};", c.css());
    }
    css.pop();
    css
}

fn trim_f(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// An HTML fragment for the paragraphs (no `<html>` wrapper).
pub fn to_html(rt: &RichText) -> String {
    let mut out = String::new();
    let mut open_list: Option<ListKind> = None;
    for p in &rt.paras {
        if open_list.is_some() && open_list != p.list {
            out.push_str(if open_list == Some(ListKind::Numbered) {
                "</ol>\r\n"
            } else {
                "</ul>\r\n"
            });
            open_list = None;
        }
        if let (Some(k), None) = (p.list, open_list) {
            out.push_str(if k == ListKind::Numbered {
                "<ol>"
            } else {
                "<ul>"
            });
            open_list = Some(k);
        }
        let tag = if p.list.is_some() {
            "li"
        } else {
            match p.heading {
                Some(h @ 1..=6) => ["h1", "h2", "h3", "h4", "h5", "h6"][usize::from(h) - 1],
                _ => "p",
            }
        };
        let align = match p.align {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "justify",
        };
        let _ = write!(out, "<{tag} style=\"margin:0;text-align:{align}\">");
        for r in &p.runs {
            let css = run_css(&r.fmt);
            let link = r.link.as_deref().and_then(safe_link);
            if let Some(l) = &link {
                out.push_str("<a href=\"");
                out.push_str(
                    &l.replace('&', "&amp;")
                        .replace('"', "&quot;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;"),
                );
                out.push_str("\">");
            }
            if css.is_empty() {
                escape(&r.text, &mut out);
            } else {
                let _ = write!(out, "<span style=\"{css}\">");
                escape(&r.text, &mut out);
                out.push_str("</span>");
            }
            if link.is_some() {
                out.push_str("</a>");
            }
        }
        let _ = write!(out, "</{tag}>\r\n");
    }
    if let Some(k) = open_list {
        out.push_str(if k == ListKind::Numbered {
            "</ol>"
        } else {
            "</ul>"
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt_of(rt: &RichText, para: usize, run: usize) -> &CharFmt {
        &rt.paras[para].runs[run].fmt
    }

    #[test]
    fn word_style_paragraphs() {
        let html = "<html><head><style><!-- p.MsoNormal {margin:0in; font-size:11.0pt; font-family:\"Calibri\",sans-serif;} --></style></head><body>\
            <!--StartFragment--><p class=MsoNormal align=center>Hello <b>bold</b> <span style='color:#FF0000;font-style:italic'>red</span></p>\
            <p class=MsoNormal><o:p>&nbsp;</o:p></p><!--EndFragment--></body></html>";
        let rt = parse(html).unwrap();
        assert_eq!(rt.paras.len(), 2);
        assert_eq!(rt.paras[0].align, Align::Center);
        assert_eq!(rt.paras[0].text(), "Hello bold red");
        assert_eq!(fmt_of(&rt, 0, 0).font.as_deref(), Some("Calibri"));
        assert_eq!(fmt_of(&rt, 0, 0).size_pt, Some(11.0));
        assert!(fmt_of(&rt, 0, 1).bold);
        assert_eq!(fmt_of(&rt, 0, 3).color, Some(Rgb::new(255, 0, 0)));
        assert!(fmt_of(&rt, 0, 3).italic);
        assert!(rt.paras[1].runs.is_empty());
    }

    #[test]
    fn lists_headings_and_breaks() {
        let rt = parse("<h2>T</h2><ul><li>a</li><li>b<br>c</li></ul><ol><li>n</li></ol>").unwrap();
        assert_eq!(rt.paras[0].heading, Some(2));
        assert_eq!(rt.paras[1].list, Some(ListKind::Bullet));
        assert_eq!(rt.paras[2].text(), "b\nc");
        assert_eq!(rt.paras[3].list, Some(ListKind::Numbered));
    }

    #[test]
    fn tables_flatten_to_tabs() {
        let rt = parse("<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>")
            .unwrap();
        assert_eq!(rt.plain(), "a\tb\nc\td");
    }

    #[test]
    fn dangerous_content_is_dropped() {
        let rt = parse(
            "<p><a href=\"javascript:alert(1)\">x</a><a href=\"file:///c:/a\">y</a><a href=\"https://example.com/\">z</a>\
             <script>evil()</script><img src=\"http://example.com/t.png\"><iframe>q</iframe>\
             <div style=\"display:none\">hidden</div></p>",
        )
        .unwrap();
        assert_eq!(rt.plain(), "xyz");
        let links: Vec<_> = rt.paras[0].runs.iter().map(|r| r.link.clone()).collect();
        assert!(links.contains(&Some("https://example.com/".to_owned())));
        assert!(
            !links
                .iter()
                .flatten()
                .any(|l| l.starts_with("javascript") || l.starts_with("file"))
        );
    }

    #[test]
    fn nesting_limit() {
        let deep = "<div>".repeat(MAX_DEPTH + 5);
        assert_eq!(parse(&deep), Err(ParseError::TooDeep));
    }

    #[test]
    fn serializer_round_trips() {
        let mut rt = RichText::default();
        rt.paras.push(Para {
            runs: vec![
                Run {
                    text: "A <b> & ".into(),
                    fmt: CharFmt {
                        font: Some("Arial".into()),
                        size_pt: Some(12.5),
                        bold: true,
                        color: Some(Rgb::new(1, 2, 3)),
                        ..CharFmt::default()
                    },
                    link: None,
                },
                Run {
                    text: "link".into(),
                    fmt: CharFmt {
                        underline: true,
                        highlight: Some(Rgb::new(255, 255, 0)),
                        ..CharFmt::default()
                    },
                    link: Some("https://example.com/?a=1&b=2".into()),
                },
            ],
            align: Align::Right,
            heading: None,
            list: None,
        });
        rt.paras.push(Para {
            runs: vec![Run {
                text: "x\ty".into(),
                ..Run::default()
            }],
            list: Some(ListKind::Bullet),
            ..Para::default()
        });
        let back = parse(&to_html(&rt)).unwrap();
        assert_eq!(back, rt);
    }
}
