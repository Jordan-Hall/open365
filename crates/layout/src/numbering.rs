//! List numbering: resolves `word/numbering.xml` (abstract numbers, numbering instances, level
//! overrides, style-linked lists) and computes the label of every numbered paragraph.
//!
//! `open365-docx` does not expose numbering yet, so this module reads `word/numbering.xml`,
//! the `w:numPr` of paragraph styles and of paragraphs straight from the package. It should move
//! into `open365-docx` as typed accessors (see the report of the change that added it).
//!
//! Rules (checked against Word with the `layout-lists-*` fixtures and
//! `fixtures/reference/layout/*.lists.json`):
//!
//! * Counters belong to the abstract numbering, so two `w:num` that share one continue the same
//!   sequence unless the second carries a `w:startOverride` for the level.
//! * A paragraph at level L adds one to its level's counter and restarts the deeper levels
//!   (honouring `w:lvlRestart`). A level that has not been used shows its start value.
//! * Letters repeat after `z` (`aa`, `bb`), roman numerals stop at 3999.

use std::collections::{HashMap, HashSet};

use open365_docx::{Document, Paragraph, RawXml, XmlElement, parse_xml};

const LEVELS: usize = 9;

/// What follows the label (`w:suff`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Suffix {
    Tab,
    Space,
    Nothing,
}

/// Label alignment around the number position (`w:lvlJc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LabelAlign {
    Left,
    Center,
    Right,
}

/// Indents in points that a numbering level (or a paragraph) states; `None` means "not stated".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct IndentOverride {
    pub left: Option<f64>,
    pub right: Option<f64>,
    /// First-line offset: positive for `w:firstLine`, negative for `w:hanging`.
    pub first_line: Option<f64>,
}

impl IndentOverride {
    fn from_ind(e: &XmlElement) -> Self {
        let pt = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| e.head.attr_w(n))
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(|t| t / 20.0)
        };
        let hanging = pt(&["hanging"]);
        let first = pt(&["firstLine"]);
        Self {
            left: pt(&["left", "start"]),
            right: pt(&["right", "end"]),
            first_line: hanging.map(|h| -h).or(first),
        }
    }

    /// `self` wins where it states a value, `lower` fills the rest.
    pub(crate) fn over(self, lower: Self) -> Self {
        Self {
            left: self.left.or(lower.left),
            right: self.right.or(lower.right),
            first_line: self.first_line.or(lower.first_line),
        }
    }
}

/// Character formatting a level states for its label (`w:lvl/w:rPr`); `None` keeps the
/// paragraph mark's value.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LabelRun {
    pub font: Option<String>,
    pub size_half_points: Option<u32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<[u8; 3]>,
}

fn on_off(e: &XmlElement) -> bool {
    !matches!(e.head.attr_w("val"), Some("0" | "false" | "off" | "none"))
}

fn child<'a>(e: &'a XmlElement, local: &'a str) -> Option<&'a XmlElement> {
    children(e, local).next()
}

fn children<'a>(e: &'a XmlElement, local: &'a str) -> impl Iterator<Item = &'a XmlElement> {
    e.children
        .iter()
        .filter_map(RawXml::as_element)
        .filter(move |c| c.head.is_w(local))
}

fn val<'a>(e: &'a XmlElement, local: &'a str) -> Option<&'a str> {
    child(e, local).and_then(|c| c.head.attr_w("val"))
}

fn int(v: Option<&str>) -> Option<i64> {
    v.and_then(|v| v.trim().parse().ok())
}

impl LabelRun {
    fn from_rpr(rpr: &XmlElement) -> Self {
        let mut out = Self::default();
        for e in rpr.children.iter().filter_map(RawXml::as_element) {
            match e.head.local_name() {
                "rFonts" if e.head.is_w("rFonts") => {
                    out.font = e
                        .head
                        .attr_w("ascii")
                        .or_else(|| e.head.attr_w("hAnsi"))
                        .map(str::to_owned);
                }
                "sz" if e.head.is_w("sz") => {
                    out.size_half_points =
                        int(e.head.attr_w("val")).and_then(|v| v.try_into().ok());
                }
                "b" if e.head.is_w("b") => out.bold = Some(on_off(e)),
                "i" if e.head.is_w("i") => out.italic = Some(on_off(e)),
                "u" if e.head.is_w("u") => {
                    out.underline = Some(e.head.attr_w("val") != Some("none"))
                }
                "strike" if e.head.is_w("strike") => out.strike = Some(on_off(e)),
                "color" if e.head.is_w("color") => {
                    out.color = e.head.attr_w("val").and_then(parse_hex_color);
                }
                _ => {}
            }
        }
        out
    }
}

fn parse_hex_color(v: &str) -> Option<[u8; 3]> {
    if v.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(v, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

#[derive(Debug, Clone)]
struct Level {
    start: i64,
    fmt: String,
    text: String,
    suffix: Suffix,
    align: LabelAlign,
    is_legal: bool,
    /// `w:lvlRestart`: `None` restarts after any higher level, `Some(0)` never.
    restart: Option<usize>,
    pstyle: Option<String>,
    indent: IndentOverride,
    run: LabelRun,
}

impl Level {
    fn from_lvl(lvl: &XmlElement) -> Self {
        let indent = child(lvl, "pPr")
            .and_then(|p| child(p, "ind"))
            .map(IndentOverride::from_ind)
            .unwrap_or_default();
        Self {
            start: int(val(lvl, "start")).unwrap_or(1),
            fmt: val(lvl, "numFmt").unwrap_or("decimal").to_owned(),
            text: val(lvl, "lvlText").unwrap_or_default().to_owned(),
            suffix: match val(lvl, "suff") {
                Some("space") => Suffix::Space,
                Some("nothing") => Suffix::Nothing,
                _ => Suffix::Tab,
            },
            align: match val(lvl, "lvlJc") {
                Some("right" | "end") => LabelAlign::Right,
                Some("center") => LabelAlign::Center,
                _ => LabelAlign::Left,
            },
            is_legal: child(lvl, "isLgl").is_some_and(on_off),
            restart: int(val(lvl, "lvlRestart")).and_then(|v| usize::try_from(v).ok()),
            pstyle: val(lvl, "pStyle").map(str::to_owned),
            indent,
            run: child(lvl, "rPr")
                .map(LabelRun::from_rpr)
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Default)]
struct Abstract {
    levels: Vec<Option<Level>>,
    num_style_link: Option<String>,
}

#[derive(Debug, Default)]
struct Num {
    abstract_id: i64,
    start_overrides: HashMap<usize, i64>,
    level_overrides: HashMap<usize, Level>,
}

#[derive(Debug, Default, Clone)]
struct StyleNum {
    based_on: Option<String>,
    num_id: Option<i64>,
    ilvl: Option<usize>,
}

/// A paragraph's computed list label.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListLabel {
    pub text: String,
    pub suffix: Suffix,
    pub align: LabelAlign,
    /// Indents of the numbering level; direct paragraph formatting still wins.
    pub indent: IndentOverride,
    pub run: LabelRun,
    /// True for a bullet: the text is drawn as is.
    pub bullet: bool,
}

/// Counters while the paragraphs are walked in document order.
#[derive(Debug, Default)]
pub(crate) struct NumberingState {
    counters: HashMap<i64, [Option<i64>; LEVELS]>,
    starts: HashMap<i64, [Option<i64>; LEVELS]>,
    seen_nums: HashSet<i64>,
}

/// Numbering definitions of a document.
#[derive(Debug, Default)]
pub(crate) struct Numbering {
    abstracts: HashMap<i64, Abstract>,
    nums: HashMap<i64, Num>,
    styles: HashMap<String, StyleNum>,
    default_style: Option<String>,
}

fn part_data<'a>(doc: &'a Document, name: &str) -> Option<&'a [u8]> {
    doc.package().part(name).map(|p| p.data()).or_else(|| {
        doc.package()
            .parts()
            .find(|p| p.name().as_str().ends_with(name.trim_start_matches('/')))
            .map(|p| p.data())
    })
}

impl Numbering {
    /// Reads the numbering part and the styles' `w:numPr`. `None` when the document has no
    /// numbering part.
    pub(crate) fn new(doc: &Document) -> Option<Self> {
        let bytes = part_data(doc, "word/numbering.xml")?;
        let parsed = parse_xml(bytes).ok()?;
        let mut out = Self::default();
        for e in parsed.root.children.iter().filter_map(RawXml::as_element) {
            if e.head.is_w("abstractNum") {
                let Some(id) = int(e.head.attr_w("abstractNumId")) else {
                    continue;
                };
                let mut levels: Vec<Option<Level>> = vec![None; LEVELS];
                for lvl in children(e, "lvl") {
                    if let Some(i) = int(lvl.head.attr_w("ilvl"))
                        .and_then(|i| usize::try_from(i).ok())
                        .filter(|i| *i < LEVELS)
                    {
                        levels[i] = Some(Level::from_lvl(lvl));
                    }
                }
                out.abstracts.insert(
                    id,
                    Abstract {
                        levels,
                        num_style_link: val(e, "numStyleLink").map(str::to_owned),
                    },
                );
            } else if e.head.is_w("num") {
                let Some(id) = int(e.head.attr_w("numId")) else {
                    continue;
                };
                let mut num = Num {
                    abstract_id: int(val(e, "abstractNumId")).unwrap_or(-1),
                    ..Num::default()
                };
                for o in children(e, "lvlOverride") {
                    let Some(i) = int(o.head.attr_w("ilvl"))
                        .and_then(|i| usize::try_from(i).ok())
                        .filter(|i| *i < LEVELS)
                    else {
                        continue;
                    };
                    if let Some(s) = int(val(o, "startOverride")) {
                        num.start_overrides.insert(i, s);
                    }
                    if let Some(lvl) = child(o, "lvl") {
                        num.level_overrides.insert(i, Level::from_lvl(lvl));
                    }
                }
                out.nums.insert(id, num);
            }
        }
        out.read_styles(doc);
        Some(out)
    }

    fn read_styles(&mut self, doc: &Document) {
        let Some(name) = doc.styles().map(|s| s.part_name().as_str().to_owned()) else {
            return;
        };
        let Some(parsed) = part_data(doc, &name).and_then(|b| parse_xml(b).ok()) else {
            return;
        };
        for style in children(&parsed.root, "style") {
            let Some(id) = style.head.attr_w("styleId") else {
                continue;
            };
            if style.head.attr_w("type").unwrap_or("paragraph") != "paragraph" {
                continue;
            }
            if style.head.attr_w("default") == Some("1") {
                self.default_style = Some(id.to_owned());
            }
            let num_pr = child(style, "pPr").and_then(|p| child(p, "numPr"));
            self.styles.insert(
                id.to_owned(),
                StyleNum {
                    based_on: val(style, "basedOn").map(str::to_owned),
                    num_id: num_pr.and_then(|n| int(val(n, "numId"))),
                    ilvl: num_pr
                        .and_then(|n| int(val(n, "ilvl")))
                        .and_then(|i| usize::try_from(i).ok()),
                },
            );
        }
    }

    /// `numId` and level a style (or one it is based on) assigns, with the style that holds it.
    fn style_numbering(&self, style: &str) -> Option<(i64, Option<usize>, String)> {
        let mut current = Some(style);
        let mut seen = 0;
        while let Some(id) = current {
            seen += 1;
            if seen > 32 {
                return None;
            }
            let s = self.styles.get(id)?;
            if let Some(n) = s.num_id {
                return Some((n, s.ilvl, id.to_owned()));
            }
            current = s.based_on.as_deref();
        }
        None
    }

    /// Follows `w:numStyleLink`: a list that only points at a style uses the abstract numbering
    /// that carries `w:styleLink` for it.
    fn resolve_abstract(&self, mut id: i64) -> Option<i64> {
        for _ in 0..8 {
            let a = self.abstracts.get(&id)?;
            let Some(link) = a.num_style_link.as_deref() else {
                return Some(id);
            };
            let (num_id, _, _) = self.style_numbering(link)?;
            let next = self.nums.get(&num_id)?.abstract_id;
            if next == id {
                return Some(id);
            }
            id = next;
        }
        None
    }

    fn level<'a>(&'a self, num: &'a Num, abstract_id: i64, ilvl: usize) -> Option<&'a Level> {
        num.level_overrides
            .get(&ilvl)
            .or_else(|| self.abstracts.get(&abstract_id)?.levels.get(ilvl)?.as_ref())
    }

    /// The numbering instance and level `para` belongs to, if any.
    fn paragraph_numbering(&self, para: &Paragraph) -> Option<(i64, usize)> {
        let direct = para
            .props
            .as_ref()
            .and_then(|p| child(p, "numPr"))
            .map(|n| (int(val(n, "numId")), int(val(n, "ilvl"))));
        let style = para
            .style_id()
            .map(str::to_owned)
            .or_else(|| self.default_style.clone());
        let from_style = style.as_deref().and_then(|s| self.style_numbering(s));
        let (num_id, ilvl) = match (direct, from_style) {
            (Some((Some(n), l)), _) => (n, l.and_then(|l| usize::try_from(l).ok())),
            // Direct numPr without numId keeps the style's list at the stated level.
            (Some((None, l)), Some((n, _, _))) => (n, l.and_then(|l| usize::try_from(l).ok())),
            (Some((None, _)), None) => return None,
            (None, Some((n, l, holder))) => {
                let level = l.or_else(|| self.level_for_style(n, &holder));
                (n, level)
            }
            (None, None) => return None,
        };
        if num_id <= 0 {
            return None;
        }
        Some((num_id, ilvl.unwrap_or(0).min(LEVELS - 1)))
    }

    /// Level whose `w:pStyle` names `style`, for a style that links to a list without a level.
    fn level_for_style(&self, num_id: i64, style: &str) -> Option<usize> {
        let num = self.nums.get(&num_id)?;
        let abs = self.resolve_abstract(num.abstract_id)?;
        self.abstracts.get(&abs)?.levels.iter().position(|l| {
            l.as_ref()
                .is_some_and(|l| l.pstyle.as_deref() == Some(style))
        })
    }

    /// Label of `para`, advancing the counters; `None` for an unnumbered paragraph.
    pub(crate) fn label(&self, state: &mut NumberingState, para: &Paragraph) -> Option<ListLabel> {
        let (num_id, ilvl) = self.paragraph_numbering(para)?;
        let num = self.nums.get(&num_id)?;
        let abstract_id = self.resolve_abstract(num.abstract_id)?;
        let level = self.level(num, abstract_id, ilvl)?;

        let counters = state.counters.entry(abstract_id).or_default();
        let starts = state.starts.entry(abstract_id).or_default();
        if state.seen_nums.insert(num_id) {
            // The first paragraph of an instance applies its start overrides.
            for (&l, &s) in &num.start_overrides {
                counters[l..].fill(None);
                starts[l] = Some(s);
            }
        }
        let start_of = |l: usize, starts: &[Option<i64>; LEVELS]| {
            starts[l].unwrap_or_else(|| self.level(num, abstract_id, l).map_or(1, |lv| lv.start))
        };
        counters[ilvl] = Some(match counters[ilvl] {
            Some(v) => v + 1,
            None => start_of(ilvl, starts),
        });
        for d in ilvl + 1..LEVELS {
            let resets = match self.level(num, abstract_id, d).and_then(|l| l.restart) {
                None => true,
                Some(0) => false,
                Some(k) => ilvl < k,
            };
            if resets {
                counters[d] = None;
                // A restarted level starts from the level's own start from now on.
                starts[d] = None;
            }
        }

        let bullet = level.fmt == "bullet";
        let text = if bullet {
            level.text.clone()
        } else {
            let value = |l: usize| counters[l].unwrap_or_else(|| start_of(l, starts));
            let mut out = String::new();
            let mut chars = level.text.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '%'
                    && let Some(d) = chars.peek().and_then(|d| d.to_digit(10))
                    && (1..=LEVELS as u32).contains(&d)
                {
                    chars.next();
                    let l = d as usize - 1;
                    let fmt = if level.is_legal && l != ilvl {
                        "decimal"
                    } else {
                        self.level(num, abstract_id, l)
                            .map_or("decimal", |lv| lv.fmt.as_str())
                    };
                    out.push_str(&format_number(value(l), fmt));
                } else {
                    out.push(c);
                }
            }
            out
        };
        Some(ListLabel {
            text,
            suffix: level.suffix,
            align: level.align,
            indent: level.indent,
            run: level.run.clone(),
            bullet,
        })
    }
}

/// The paragraph's own `w:ind`, in points.
pub(crate) fn direct_indent(para: &Paragraph) -> IndentOverride {
    para.props
        .as_ref()
        .and_then(|p| child(p, "ind"))
        .map(IndentOverride::from_ind)
        .unwrap_or_default()
}

fn roman(mut n: i64, upper: bool) -> Option<String> {
    if !(1..4000).contains(&n) {
        return None;
    }
    const TABLE: [(i64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (v, s) in TABLE {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    Some(if upper { out } else { out.to_lowercase() })
}

fn letters(n: i64, upper: bool) -> Option<String> {
    if n < 1 {
        return None;
    }
    let base = if upper { b'A' } else { b'a' };
    let c = char::from(base + ((n - 1) % 26) as u8);
    Some(std::iter::repeat_n(c, ((n - 1) / 26 + 1) as usize).collect())
}

fn ordinal_suffix(n: i64) -> &'static str {
    match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    }
}

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
const ORDINAL_ONES: [&str; 20] = [
    "zeroth",
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
    "eleventh",
    "twelfth",
    "thirteenth",
    "fourteenth",
    "fifteenth",
    "sixteenth",
    "seventeenth",
    "eighteenth",
    "nineteenth",
];

/// English words for 0..=99, cardinal or ordinal.
fn words(n: i64, ordinal: bool) -> Option<String> {
    if !(0..100).contains(&n) {
        return None;
    }
    let n = n as usize;
    let s = if n < 20 {
        (if ordinal { ORDINAL_ONES[n] } else { ONES[n] }).to_owned()
    } else {
        let (t, o) = (n / 10, n % 10);
        match (o, ordinal) {
            (0, false) => TENS[t].to_owned(),
            (0, true) => format!("{}ieth", TENS[t].trim_end_matches('y')),
            (_, false) => format!("{}-{}", TENS[t], ONES[o]),
            (_, true) => format!("{}-{}", TENS[t], ORDINAL_ONES[o]),
        }
    };
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
}

/// One counter value in a `w:numFmt`.
pub(crate) fn format_number(n: i64, fmt: &str) -> String {
    let formatted = match fmt {
        "upperRoman" => roman(n, true),
        "lowerRoman" => roman(n, false),
        "upperLetter" => letters(n, true),
        "lowerLetter" => letters(n, false),
        "decimalZero" => Some(format!("{n:02}")),
        "ordinal" => Some(format!("{n}{}", ordinal_suffix(n))),
        "cardinalText" => words(n, false),
        "ordinalText" => words(n, true),
        "none" => Some(String::new()),
        _ => None,
    };
    formatted.unwrap_or_else(|| n.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers_like_word() {
        assert_eq!(format_number(27, "lowerLetter"), "aa");
        assert_eq!(format_number(28, "lowerLetter"), "bb");
        assert_eq!(format_number(38, "upperRoman"), "XXXVIII");
        assert_eq!(format_number(4, "lowerRoman"), "iv");
        assert_eq!(format_number(3, "decimalZero"), "03");
        assert_eq!(format_number(12, "ordinal"), "12th");
        assert_eq!(format_number(22, "ordinal"), "22nd");
        assert_eq!(format_number(7, "unknownFormat"), "7");
    }
}
