//! Writer Home tab operations the document model has no call for: highlight, underline styles,
//! change case, clear formatting, lists, line and paragraph spacing, shading, borders, sort and
//! the format painter. Each runs as one undo step (the caller keeps the previous document).

use open365_docx::{DocPos, Document, RawXml, XmlElement, parse_xml};
use serde::{Deserialize, Serialize};

use super::writer_xml::{
    self as x, Main, PPR_ORDER, RPR_ORDER, child, drop_empty_props, fail, para_mut, ppr, ppr_mut,
    rewrite, rpr_mut, set_ordered, w, w_val,
};
use crate::error::CommandError;

/// Which case a Change Case item gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseMode {
    /// Sentence case: the first letter of each sentence is a capital, the others are small.
    Sentence,
    Lower,
    Upper,
    /// Capitalize Each Word: the first letter of each word, the rest unchanged.
    CapitalizeEach,
    Toggle,
}

/// The list a Bullets, Numbering or Multilevel List button gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Bullet,
    Number,
    Multilevel,
}

impl ListKind {
    fn name(self) -> &'static str {
        match self {
            Self::Bullet => "open365-bullet",
            Self::Number => "open365-number",
            Self::Multilevel => "open365-multilevel",
        }
    }
}

/// A paragraph border of the Borders menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BorderKind {
    Bottom,
    Top,
    Left,
    Right,
    /// No Border: removes all four sides.
    None,
    /// All Borders and Outside Borders: a box round the paragraph.
    Box,
}

/// What a [`crate::Command::DocxHome`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum WriterHomeOp {
    /// Text Highlight Colour: a highlight name (`yellow`) or `RRGGBB`; `None` is No Color.
    Highlight {
        start: DocPos,
        end: DocPos,
        color: Option<String>,
    },
    /// An underline style (`single`, `double`, `thick`, `dotted`, `dash`, `wave`...) or `none`.
    Underline {
        start: DocPos,
        end: DocPos,
        style: String,
    },
    ChangeCase {
        start: DocPos,
        end: DocPos,
        mode: CaseMode,
    },
    /// Clear All Formatting: the text loses its direct formatting; the paragraphs go back to
    /// the Normal style without list, indents, spacing, borders or shading.
    ClearFormatting { start: DocPos, end: DocPos },
    /// Bullets, Numbering, Multilevel List: sets the list on the paragraphs or, when all of them
    /// already have this kind, takes it off.
    List {
        first: usize,
        last: usize,
        kind: ListKind,
    },
    /// Line and Paragraph Spacing: `line` is a multiple (1.0, 1.15...), `before` and `after` are
    /// points; `None` leaves a value as it is.
    Spacing {
        first: usize,
        last: usize,
        line: Option<f64>,
        before: Option<f64>,
        after: Option<f64>,
    },
    /// Paragraph dialog > Outline level: 1 to 9, or `None` for Body Text.
    OutlineLevel {
        first: usize,
        last: usize,
        level: Option<u8>,
    },
    /// Shading: a fill colour (`RRGGBB`) for the paragraphs, or `None` for No Color.
    Shading {
        first: usize,
        last: usize,
        fill: Option<String>,
    },
    Border {
        first: usize,
        last: usize,
        border: BorderKind,
    },
    /// Sort Text: orders paragraphs `first..=last` (one container, only paragraphs) by their
    /// text.
    Sort {
        first: usize,
        last: usize,
        descending: bool,
    },
    /// Format Painter: gives the text the run formatting `rpr` (the XML from
    /// [`char_format_xml`], or `None` for plain).
    FormatPainter {
        start: DocPos,
        end: DocPos,
        rpr: Option<String>,
    },
}

impl WriterHomeOp {
    /// Whether the operation discards content that exists.
    pub(crate) fn destroys(&self) -> bool {
        matches!(self, Self::ClearFormatting { .. } | Self::Sort { .. })
    }
}

/// Runs `op` on `doc`; `doc` is unchanged when it fails.
pub(crate) fn run(doc: &mut Document, op: &WriterHomeOp) -> Result<(), CommandError> {
    rewrite(doc, |m| apply(m, op))
}

const HIGHLIGHTS: &[(&str, &str)] = &[
    ("yellow", "FFFF00"),
    ("green", "00FF00"),
    ("cyan", "00FFFF"),
    ("magenta", "FF00FF"),
    ("blue", "0000FF"),
    ("red", "FF0000"),
    ("darkBlue", "000080"),
    ("darkCyan", "008080"),
    ("darkGreen", "008000"),
    ("darkMagenta", "800080"),
    ("darkRed", "800000"),
    ("darkYellow", "808000"),
    ("darkGray", "808080"),
    ("lightGray", "C0C0C0"),
    ("black", "000000"),
    ("white", "FFFFFF"),
];

fn highlight_name(color: &str) -> Result<&'static str, CommandError> {
    let c = color.trim().trim_start_matches('#');
    HIGHLIGHTS
        .iter()
        .find(|(n, hex)| n.eq_ignore_ascii_case(c) || hex.eq_ignore_ascii_case(c))
        .map(|(n, _)| *n)
        .ok_or_else(|| fail(format!("{color} is not a highlight colour")))
}

fn ordered(a: DocPos, b: DocPos) -> (DocPos, DocPos) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Calls `f(para, from, to)` for each paragraph of the range with its character span.
fn each_span(
    m: &mut Main,
    start: DocPos,
    end: DocPos,
    mut f: impl FnMut(&mut XmlElement, usize, usize, usize) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let (s, e) = ordered(start, end);
    for para in s.para..=e.para {
        let p = para_mut(m.root(), para)?;
        let len = x::char_len(p);
        let from = if para == s.para { s.offset.min(len) } else { 0 };
        let to = if para == e.para {
            e.offset.min(len)
        } else {
            len
        };
        f(p, para, from, to)?;
    }
    Ok(())
}

fn apply(m: &mut Main, op: &WriterHomeOp) -> Result<(), CommandError> {
    match op {
        WriterHomeOp::Highlight { start, end, color } => {
            let name = color.as_deref().map(highlight_name).transpose()?;
            each_span(m, *start, *end, |p, _, from, to| {
                x::range_runs(p, from, to, &mut |r| {
                    let like = r.head.clone();
                    let rpr = rpr_mut(r);
                    match name {
                        Some(n) => set_ordered(rpr, RPR_ORDER, w_val(&like, "highlight", n)),
                        None => {
                            x::remove_child(rpr, "highlight");
                        }
                    }
                    drop_empty_props(r);
                });
                Ok(())
            })
        }
        WriterHomeOp::Underline { start, end, style } => {
            each_span(m, *start, *end, |p, _, from, to| {
                x::range_runs(p, from, to, &mut |r| {
                    let like = r.head.clone();
                    let rpr = rpr_mut(r);
                    set_ordered(rpr, RPR_ORDER, w_val(&like, "u", style));
                    drop_empty_props(r);
                });
                Ok(())
            })
        }
        WriterHomeOp::ChangeCase { start, end, mode } => change_case(m, *start, *end, *mode),
        WriterHomeOp::ClearFormatting { start, end } => {
            each_span(m, *start, *end, |p, _, from, to| {
                let (mut from, mut to) = (from, to);
                if from == to {
                    (from, to) = (0, x::char_len(p));
                }
                x::range_runs(p, from, to, &mut |r| {
                    if matches!(r.children.first(), Some(RawXml::Element(e)) if e.head.is_w("rPr"))
                    {
                        r.children.remove(0);
                    }
                });
                if let Some(props) = ppr(p).cloned() {
                    let keep: Vec<RawXml> = props
                        .children
                        .into_iter()
                        .filter(|c| matches!(c, RawXml::Element(e) if e.head.is_w("sectPr")))
                        .collect();
                    let slot = ppr_mut(p);
                    slot.children = keep;
                    drop_empty_props(p);
                }
                Ok(())
            })
        }
        WriterHomeOp::List { first, last, kind } => set_list(m, *first, *last, *kind),
        WriterHomeOp::Spacing {
            first,
            last,
            line,
            before,
            after,
        } => {
            for para in *first..=*last {
                let p = para_mut(m.root(), para)?;
                let like = p.head.clone();
                let props = ppr_mut(p);
                let mut sp = child(props, "spacing")
                    .cloned()
                    .unwrap_or_else(|| w(&like, "spacing"));
                if let Some(l) = line {
                    sp.head
                        .set_attr_w("line", &format!("{}", (l * 240.0).round() as i64));
                    sp.head.set_attr_w("lineRule", "auto");
                }
                if let Some(b) = before {
                    sp.head
                        .set_attr_w("before", &format!("{}", (b * 20.0).round() as i64));
                    sp.head.remove_attr_w("beforeAutospacing");
                }
                if let Some(a) = after {
                    sp.head
                        .set_attr_w("after", &format!("{}", (a * 20.0).round() as i64));
                    sp.head.remove_attr_w("afterAutospacing");
                }
                set_ordered(props, PPR_ORDER, sp);
            }
            Ok(())
        }
        WriterHomeOp::OutlineLevel { first, last, level } => {
            if level.is_some_and(|l| !(1..=9).contains(&l)) {
                return Err(fail("The outline level is Body Text or 1 to 9".to_owned()));
            }
            for para in *first..=*last {
                let p = para_mut(m.root(), para)?;
                let like = p.head.clone();
                let props = ppr_mut(p);
                match level {
                    Some(l) => set_ordered(
                        props,
                        PPR_ORDER,
                        w_val(&like, "outlineLvl", &(l - 1).to_string()),
                    ),
                    None => {
                        // Body Text on a heading style has to say so (level 9); elsewhere the
                        // paragraph simply has no level of its own.
                        x::remove_child(props, "outlineLvl");
                        let heading = child(props, "pStyle")
                            .and_then(|s| s.head.attr_w("val"))
                            .is_some_and(|id| id.to_ascii_lowercase().starts_with("heading"));
                        if heading {
                            set_ordered(props, PPR_ORDER, w_val(&like, "outlineLvl", "9"));
                        }
                    }
                }
                drop_empty_props(p);
            }
            Ok(())
        }
        WriterHomeOp::Shading { first, last, fill } => {
            let fill = fill
                .as_deref()
                .map(|f| {
                    let f = f.trim().trim_start_matches('#');
                    if f.len() == 6 && f.chars().all(|c| c.is_ascii_hexdigit()) {
                        Ok(f.to_ascii_uppercase())
                    } else {
                        Err(fail(format!("{f} is not a colour")))
                    }
                })
                .transpose()?;
            for para in *first..=*last {
                let p = para_mut(m.root(), para)?;
                let like = p.head.clone();
                let props = ppr_mut(p);
                match &fill {
                    Some(f) => {
                        let mut shd = w(&like, "shd");
                        shd.head.set_attr_w("val", "clear");
                        shd.head.set_attr_w("color", "auto");
                        shd.head.set_attr_w("fill", f);
                        set_ordered(props, PPR_ORDER, shd);
                    }
                    None => {
                        x::remove_child(props, "shd");
                    }
                }
                drop_empty_props(p);
            }
            Ok(())
        }
        WriterHomeOp::Border {
            first,
            last,
            border,
        } => set_border(m, *first, *last, *border),
        WriterHomeOp::Sort {
            first,
            last,
            descending,
        } => sort(m, *first, *last, *descending),
        WriterHomeOp::FormatPainter { start, end, rpr } => {
            let rpr = match rpr {
                Some(text) => Some(parse_xml(text.as_bytes())?.root),
                None => None,
            };
            each_span(m, *start, *end, |p, _, from, to| {
                x::range_runs(p, from, to, &mut |r| {
                    if matches!(r.children.first(), Some(RawXml::Element(e)) if e.head.is_w("rPr"))
                    {
                        r.children.remove(0);
                    }
                    if let Some(src) = &rpr {
                        let mut copy = src.clone();
                        copy.head.name = x::w(&r.head, "rPr").head.name;
                        copy.head.attrs.retain(|a| !a.name.starts_with("xmlns"));
                        r.children.insert(0, RawXml::Element(copy));
                    }
                });
                Ok(())
            })
        }
    }
}

// ---- change case ----------------------------------------------------------------------------

fn one(mut it: impl Iterator<Item = char>, same: char) -> char {
    match (it.next(), it.next()) {
        (Some(c), None) => c,
        _ => same,
    }
}

fn upper(c: char) -> char {
    one(c.to_uppercase(), c)
}

fn lower(c: char) -> char {
    one(c.to_lowercase(), c)
}

/// The text after Change Case. `start` says a sentence starts at the first character.
fn cased(mode: CaseMode, chars: &[char], mut start: bool) -> Vec<char> {
    let mut prev_word = !start;
    chars
        .iter()
        .map(|&c| match mode {
            CaseMode::Lower => lower(c),
            CaseMode::Upper => upper(c),
            CaseMode::Toggle => {
                if c.is_uppercase() {
                    lower(c)
                } else {
                    upper(c)
                }
            }
            CaseMode::CapitalizeEach => {
                let word = c.is_alphanumeric() || c == '\'' || c == '\u{2019}';
                let out = if word && !prev_word { upper(c) } else { c };
                prev_word = word;
                out
            }
            CaseMode::Sentence => {
                if c.is_alphabetic() {
                    let out = if start { upper(c) } else { lower(c) };
                    start = false;
                    out
                } else {
                    if matches!(c, '.' | '?' | '!') {
                        start = true;
                    } else if c.is_numeric() {
                        start = false;
                    }
                    c
                }
            }
        })
        .collect()
}

fn change_case(m: &mut Main, a: DocPos, b: DocPos, mode: CaseMode) -> Result<(), CommandError> {
    let (s, _) = ordered(a, b);
    each_span(m, a, b, |p, para, from, to| {
        let before: Vec<char> = x::para_text(p).chars().take(from).collect();
        let sentence_start = if para != s.para || from == 0 {
            true
        } else {
            let t: String = before.iter().collect();
            let t = t.trim_end();
            t.is_empty() || t.ends_with(['.', '?', '!'])
        };
        let old = x::range_text(p, from, to);
        let new = cased(mode, &old, sentence_start);
        if new != old {
            x::write_range_text(p, from, to, &new);
        }
        Ok(())
    })
}

// ---- lists ----------------------------------------------------------------------------------

const NUMBERING_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
const NUMBERING_CT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";

fn w_ns() -> &'static str {
    open365_docx::W_NS
}

fn lvl_xml(kind: ListKind, i: usize) -> String {
    let (fmt, text, font) = match kind {
        ListKind::Bullet => {
            let (c, f) = match i % 3 {
                0 => ("\u{f0b7}", "Symbol"),
                1 => ("o", "Courier New"),
                _ => ("\u{f0a7}", "Wingdings"),
            };
            ("bullet", c.to_owned(), Some(f))
        }
        ListKind::Number => {
            let fmt = ["decimal", "lowerLetter", "lowerRoman"][i % 3];
            (fmt, format!("%{}.", i + 1), None)
        }
        ListKind::Multilevel => {
            let fmt = ["decimal", "lowerLetter", "lowerRoman"][i % 3];
            let text = match i {
                0..=2 => format!("%{})", i + 1),
                3..=5 => format!("(%{})", i + 1),
                _ => format!("%{}.", i + 1),
            };
            (fmt, text, None)
        }
    };
    let left = if kind == ListKind::Multilevel {
        360 * (i + 1)
    } else {
        720 * (i + 1)
    };
    let rpr = font.map_or(String::new(), |f| {
        format!("<w:rPr><w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:hint=\"default\"/></w:rPr>")
    });
    format!(
        "<w:lvl w:ilvl=\"{i}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{left}\" w:hanging=\"360\"/></w:pPr>{rpr}</w:lvl>",
        x::xml_escape(&text)
    )
}

fn abstract_xml(kind: ListKind, id: i64) -> String {
    let multi = if kind == ListKind::Multilevel {
        "multilevel"
    } else {
        "hybridMultilevel"
    };
    let levels: String = (0..9).map(|i| lvl_xml(kind, i)).collect();
    format!(
        "<w:abstractNum xmlns:w=\"{}\" w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"{multi}\"/><w:name w:val=\"{}\"/>{levels}</w:abstractNum>",
        w_ns(),
        kind.name()
    )
}

fn numbering_part(m: &Main) -> Option<String> {
    let rels = m.text_of(&x::rels_name(&m.name))?;
    let mut rest = rels.as_str();
    while let Some(i) = rest.find("<Relationship ") {
        let end = rest[i..].find('>').map_or(rest.len(), |e| i + e + 1);
        let tag = &rest[i..end];
        if tag.contains(&format!("Type=\"{NUMBERING_TYPE}\""))
            && let Some(at) = tag.find("Target=\"")
        {
            let t = &tag[at + 8..];
            let target = &t[..t.find('"')?];
            return Some(match target.strip_prefix('/') {
                Some(abs) => abs.to_owned(),
                None => {
                    let dir = m.name.as_str().rsplit_once('/').map_or("", |(d, _)| d);
                    format!("{}/{target}", dir.trim_start_matches('/'))
                        .trim_start_matches('/')
                        .to_owned()
                }
            });
        }
        rest = &rest[end..];
    }
    None
}

fn int_val(e: &XmlElement, attr: &str) -> Option<i64> {
    e.head.attr_w(attr).and_then(|v| v.trim().parse().ok())
}

fn elements<'a>(root: &'a XmlElement, local: &'a str) -> impl Iterator<Item = &'a XmlElement> {
    root.children
        .iter()
        .filter_map(RawXml::as_element)
        .filter(move |e| e.head.is_w(local))
}

/// The kind of the list `num_id` names, read from its abstract definition.
fn kind_of(numbering: &XmlElement, num_id: i64) -> Option<ListKind> {
    let num = elements(numbering, "num").find(|n| int_val(n, "numId") == Some(num_id))?;
    let abs_id = int_val(child(num, "abstractNumId")?, "val")?;
    let abs =
        elements(numbering, "abstractNum").find(|a| int_val(a, "abstractNumId") == Some(abs_id))?;
    match child(abs, "name").and_then(|n| n.head.attr_w("val")) {
        Some("open365-bullet") => return Some(ListKind::Bullet),
        Some("open365-number") => return Some(ListKind::Number),
        Some("open365-multilevel") => return Some(ListKind::Multilevel),
        _ => {}
    }
    let lvl = elements(abs, "lvl").find(|l| int_val(l, "ilvl") == Some(0))?;
    if child(lvl, "numFmt")?.head.attr_w("val") == Some("bullet") {
        Some(ListKind::Bullet)
    } else if child(abs, "multiLevelType").and_then(|t| t.head.attr_w("val")) == Some("multilevel")
    {
        Some(ListKind::Multilevel)
    } else {
        Some(ListKind::Number)
    }
}

fn num_id_of(p: &XmlElement) -> Option<i64> {
    let np = child(ppr(p)?, "numPr")?;
    int_val(child(np, "numId")?, "val").filter(|n| *n != 0)
}

fn set_list(m: &mut Main, first: usize, last: usize, kind: ListKind) -> Result<(), CommandError> {
    let part = numbering_part(m).unwrap_or_else(|| "word/numbering.xml".to_owned());
    let mut numbering = match m.text_of(&part) {
        Some(text) => parse_xml(text.as_bytes())?,
        None => parse_xml(
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"{}\"/>", w_ns())
                .as_bytes(),
        )?,
    };
    let all_have = (first..=last).all(|i| {
        para_mut(m.root(), i)
            .ok()
            .and_then(|p| num_id_of(p))
            .and_then(|n| kind_of(&numbering.root, n))
            == Some(kind)
    });
    if all_have {
        for i in first..=last {
            let p = para_mut(m.root(), i)?;
            let props = ppr_mut(p);
            x::remove_child(props, "numPr");
            if child(props, "pStyle").and_then(|s| s.head.attr_w("val")) == Some("ListParagraph") {
                x::remove_child(props, "pStyle");
            }
            drop_empty_props(p);
        }
        return Ok(());
    }
    // Join the list of the paragraph above when it is of this kind.
    let joined = first
        .checked_sub(1)
        .and_then(|i| para_mut(m.root(), i).ok().and_then(|p| num_id_of(p)))
        .filter(|n| kind_of(&numbering.root, *n) == Some(kind));
    let num_id = match joined {
        Some(n) => n,
        None => {
            let root = &mut numbering.root;
            let name = kind.name();
            let existing = elements(root, "abstractNum")
                .find(|a| child(a, "name").and_then(|n| n.head.attr_w("val")) == Some(name))
                .and_then(|a| int_val(a, "abstractNumId"));
            let abs_id = match existing {
                Some(id) => id,
                None => {
                    let id = elements(root, "abstractNum")
                        .filter_map(|a| int_val(a, "abstractNumId"))
                        .max()
                        .map_or(0, |v| v + 1);
                    let el = parse_xml(abstract_xml(kind, id).as_bytes())?.root;
                    let at = root
                        .children
                        .iter()
                        .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w("num") || e.head.is_w("numIdMacAtCleanup")))
                        .unwrap_or(root.children.len());
                    root.children.insert(at, RawXml::Element(el));
                    id
                }
            };
            let id = elements(root, "num")
                .filter_map(|n| int_val(n, "numId"))
                .max()
                .map_or(1, |v| v + 1);
            let restart = if kind == ListKind::Bullet {
                String::new()
            } else {
                "<w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride>"
                    .to_owned()
            };
            let el = parse_xml(
                format!("<w:num xmlns:w=\"{}\" w:numId=\"{id}\"><w:abstractNumId w:val=\"{abs_id}\"/>{restart}</w:num>", w_ns()).as_bytes(),
            )?
            .root;
            let at = root
                .children
                .iter()
                .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w("numIdMacAtCleanup")))
                .unwrap_or(root.children.len());
            root.children.insert(at, RawXml::Element(el));
            id
        }
    };
    m.put(
        &part,
        String::from_utf8_lossy(&numbering.to_bytes()).into_owned(),
    )?;
    m.content_override(&part, NUMBERING_CT)?;
    let target = part.strip_prefix("word/").unwrap_or(&part).to_owned();
    m.add_rel(NUMBERING_TYPE, &target, false)?;
    m.ensure_style(
        "ListParagraph",
        "<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"34\"/><w:qFormat/><w:pPr><w:ind w:left=\"720\"/><w:contextualSpacing/></w:pPr></w:style>",
    )?;
    for i in first..=last {
        let p = para_mut(m.root(), i)?;
        let like = p.head.clone();
        let props = ppr_mut(p);
        if child(props, "pStyle").is_none() {
            set_ordered(props, PPR_ORDER, w_val(&like, "pStyle", "ListParagraph"));
        }
        let mut np = w(&like, "numPr");
        np.children.push(RawXml::Element(w_val(&like, "ilvl", "0")));
        np.children
            .push(RawXml::Element(w_val(&like, "numId", &num_id.to_string())));
        set_ordered(props, PPR_ORDER, np);
    }
    Ok(())
}

// ---- borders and sort -----------------------------------------------------------------------

fn set_border(
    m: &mut Main,
    first: usize,
    last: usize,
    kind: BorderKind,
) -> Result<(), CommandError> {
    const SIDES: [&str; 4] = ["top", "left", "bottom", "right"];
    let side_of = |k: BorderKind| match k {
        BorderKind::Top => Some("top"),
        BorderKind::Left => Some("left"),
        BorderKind::Bottom => Some("bottom"),
        BorderKind::Right => Some("right"),
        _ => None,
    };
    // A single side toggles: it comes off when every paragraph has it.
    let remove_single = match side_of(kind) {
        Some(side) => (first..=last).all(|i| {
            para_mut(m.root(), i)
                .ok()
                .and_then(|p| {
                    ppr(p)
                        .and_then(|pp| child(pp, "pBdr"))
                        .map(|b| child(b, side).is_some())
                })
                .unwrap_or(false)
        }),
        None => false,
    };
    for i in first..=last {
        let p = para_mut(m.root(), i)?;
        let like = p.head.clone();
        let props = ppr_mut(p);
        let mut bdr = child(props, "pBdr")
            .cloned()
            .unwrap_or_else(|| w(&like, "pBdr"));
        let make = |side: &str| {
            let mut e = w(&like, side);
            e.head.set_attr_w("val", "single");
            e.head.set_attr_w("sz", "4");
            e.head.set_attr_w(
                "space",
                if side == "left" || side == "right" {
                    "4"
                } else {
                    "1"
                },
            );
            e.head.set_attr_w("color", "auto");
            e
        };
        let add = |bdr: &mut XmlElement, side: &str| {
            x::remove_child(bdr, side);
            let rank = |n: &str| {
                ["top", "left", "bottom", "right", "between", "bar"]
                    .iter()
                    .position(|o| *o == n)
                    .unwrap_or(9)
            };
            let at = bdr
                .children
                .iter()
                .position(
                    |c| matches!(c, RawXml::Element(e) if rank(e.head.local_name()) > rank(side)),
                )
                .unwrap_or(bdr.children.len());
            bdr.children.insert(at, RawXml::Element(make(side)));
        };
        match kind {
            BorderKind::None => bdr.children.clear(),
            BorderKind::Box => SIDES.iter().for_each(|s| add(&mut bdr, s)),
            single => {
                let side = side_of(single).ok_or_else(|| fail("no such border"))?;
                if remove_single {
                    x::remove_child(&mut bdr, side);
                } else {
                    add(&mut bdr, side);
                }
            }
        }
        if bdr.children.is_empty() {
            x::remove_child(props, "pBdr");
        } else {
            set_ordered(props, PPR_ORDER, bdr);
        }
        drop_empty_props(p);
    }
    Ok(())
}

fn sort(m: &mut Main, first: usize, last: usize, descending: bool) -> Result<(), CommandError> {
    if first >= last {
        return Ok(());
    }
    let root = m.root();
    let (path, i0) = x::locate(root, first).ok_or_else(|| fail("no such paragraph"))?;
    let (path1, i1) = x::locate(root, last).ok_or_else(|| fail("no such paragraph"))?;
    if path != path1 || i1 < i0 {
        return Err(fail("the paragraphs are not in one container"));
    }
    let blocks = x::container_mut(root, &path).ok_or_else(|| fail("container not found"))?;
    let slice = &mut blocks[i0..=i1];
    if !slice
        .iter()
        .all(|b| matches!(b, RawXml::Element(e) if e.head.is_w("p")))
    {
        return Err(fail("the range holds something other than paragraphs"));
    }
    let key = |b: &RawXml| match b {
        RawXml::Element(e) => x::para_text(e).to_lowercase(),
        _ => String::new(),
    };
    slice.sort_by_key(key);
    if descending {
        slice.reverse();
    }
    Ok(())
}

// ---- format painter source --------------------------------------------------------------------

/// The run formatting (`w:rPr` as XML) of the character before `pos` (the first character at
/// the start of a paragraph); `None` when the text has none.
#[must_use]
pub fn char_format_xml(doc: &Document, pos: DocPos) -> Option<String> {
    let mut parsed = parse_xml(&doc.serialize_main()).ok()?;
    let p = para_mut(&mut parsed.root, pos.para).ok()?;
    let len = x::char_len(p);
    let (from, to) = if pos.offset > 0 {
        (pos.offset.min(len) - 1, pos.offset.min(len))
    } else {
        (0, 1.min(len))
    };
    let mut found: Option<XmlElement> = None;
    x::range_runs(p, from, to, &mut |r| {
        if found.is_none()
            && let Some(RawXml::Element(e)) = r.children.first()
            && e.head.is_w("rPr")
        {
            found = Some(e.clone());
        }
    });
    let mut rpr = found?;
    rpr.head.attrs.push(open365_docx::Attr {
        name: "xmlns:w".to_owned(),
        ns: None,
        value: w_ns().to_owned(),
    });
    let mut out = String::new();
    rpr.write(&mut out);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_case_capitalises_after_a_full_stop() {
        let t: Vec<char> = "hELLO wORLD. tHIS is it".chars().collect();
        let out: String = cased(CaseMode::Sentence, &t, true).into_iter().collect();
        assert_eq!(out, "Hello world. This is it");
    }

    #[test]
    fn capitalize_each_word_keeps_the_rest() {
        let t: Vec<char> = "the QUICK fox".chars().collect();
        let out: String = cased(CaseMode::CapitalizeEach, &t, true)
            .into_iter()
            .collect();
        assert_eq!(out, "The QUICK Fox");
    }

    #[test]
    fn toggle_swaps_every_letter() {
        let t: Vec<char> = "aB c".chars().collect();
        let out: String = cased(CaseMode::Toggle, &t, true).into_iter().collect();
        assert_eq!(out, "Ab C");
    }
}
