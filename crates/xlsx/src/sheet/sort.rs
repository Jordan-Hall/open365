//! Sort: Excel's value order, colour and custom-list keys, header guessing, left-to-right sorts, and the
//! `sortState` element.
//!
//! The rules follow Excel's output, measured through COM (`tools/office/make-sortfilter-refs.ps1`,
//! `fixtures/reference/xlsx/sortfilter-sort/scenarios.json`):
//!
//! - Ascending order is numbers, then text, then logical values (FALSE before TRUE), then errors (all equal);
//!   blank cells are last in both directions. Descending is the exact reverse of the first four groups. Equal keys
//!   keep their order (the sort is stable, also when descending).
//! - Text is compared case-insensitively with the locale's collation at accent strength; a case-sensitive sort
//!   adds the case level (lower case first). Hyphens and apostrophes carry no weight ("word sort"), ASCII
//!   punctuation sorts in Windows' order ahead of digits and letters, and a string that is equal to another one
//!   except for such characters sorts after it when it has more of them.
//! - A custom list puts the items of the list first, in list order (the match ignores case); every other value
//!   follows in the normal order. Descending reverses the whole order.
//! - A colour key puts the rows whose cell (or font) has the chosen colour first, or last when descending; the
//!   other rows keep their order unless a further key orders them.
//! - Cells move whole (value, formula, style); a moved formula is shifted as if copied, so references to other
//!   rows move with the formula. Row heights and hidden rows stay where they are.

use std::cmp::Ordering;

use open365_calc::value::parse_value_text_in;
use open365_locale::{CollationStrength, Collator, Locale};

use super::{Cell, CellValue, FormulaKind, Rect, Worksheet};
use crate::XlsxError;
use crate::format::{Color, ColorSource, Fill, FillPattern, Rgb};
use crate::refs::{parse_range, shift_formula};
use crate::structure::range_a1;
use crate::styles::Styles;
use crate::xml::{Attr, ElementHead, RawXml, XmlElement};

/// Most sort keys Excel accepts.
pub const MAX_SORT_KEYS: usize = 64;

/// What a sort key orders by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOn {
    #[default]
    Value,
    CellColor,
    FontColor,
    /// Conditional-format icon; recognised in files, not applied by [`Worksheet::sort_range`].
    Icon,
}

/// A colour a colour key or colour filter looks for. `Automatic` is "No Fill" for a cell colour and the
/// automatic (theme text 1) colour for a font; an explicit black is a different colour from the automatic one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorKey {
    #[default]
    Automatic,
    Rgb(Rgb),
}

/// Whether the first line of the range is a header that stays in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderMode {
    Yes,
    #[default]
    No,
    /// Excel's guess: the first line is a header when its cells differ in type or format from the second line.
    Guess,
}

/// Direction of the sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrientation {
    /// Rows move (Excel's default; keys are columns).
    #[default]
    TopToBottom,
    /// Columns move (keys are rows).
    LeftToRight,
}

/// Collation variant for East Asian text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMethod {
    #[default]
    Default,
    PinYin,
    Stroke,
}

/// One level of a sort.
#[derive(Debug, Clone, PartialEq)]
pub struct SortKey {
    /// Absolute 1-based column (top to bottom) or row (left to right) holding the key.
    pub index: u32,
    pub on: SortOn,
    pub descending: bool,
    /// The colour of a colour key.
    pub color: ColorKey,
    /// Items of a custom list (the sort order of Value keys).
    pub custom_list: Option<Vec<String>>,
}

impl SortKey {
    /// A plain value key.
    pub fn value(index: u32, descending: bool) -> Self {
        Self {
            index,
            on: SortOn::Value,
            descending,
            color: ColorKey::Automatic,
            custom_list: None,
        }
    }
}

/// A sort request.
#[derive(Debug, Clone, PartialEq)]
pub struct SortSpec {
    /// The cells that move, header line included when there is one.
    pub range: Rect,
    pub header: HeaderMode,
    pub orientation: SortOrientation,
    pub case_sensitive: bool,
    pub method: SortMethod,
    pub keys: Vec<SortKey>,
}

/// What a sort did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortOutcome {
    /// Whether the first line was kept as a header.
    pub header: bool,
    /// `order[k]` is the 0-based data line (counted from the first line after the header) that now sits at
    /// position `k`.
    pub order: Vec<u32>,
    /// Cells whose formula text changed because the cell moved.
    pub formulas_shifted: usize,
}

// ---- text order ----------------------------------------------------------------------------------------

/// Windows' order of the ASCII symbols that carry weight (space first); measured through Excel.
const PUNCT_ORDER: &str = " !\"#$%&()*,./:;?@[\\]^_`{|}~+<=>";

fn is_ignorable(c: char) -> bool {
    matches!(c, '-' | '\'' | '\u{ad}')
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Punct(u8),
    Run(String),
}

/// A string prepared for [`TextOrder::compare`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextKey {
    toks: Vec<Tok>,
    ignorable: Vec<(u32, char)>,
}

/// Locale-aware text order the way Excel's sort and filter compare text.
#[derive(Debug)]
pub struct TextOrder {
    primary: Collator,
    secondary: Collator,
    tertiary: Option<Collator>,
}

fn locale_error(e: impl std::fmt::Display) -> XlsxError {
    XlsxError::UnsupportedEdit(format!("collation: {e}"))
}

impl TextOrder {
    /// A text order for `locale`. `case_sensitive` adds the case level (lower case before upper case).
    pub fn new(
        locale: &Locale,
        method: SortMethod,
        case_sensitive: bool,
    ) -> Result<Self, XlsxError> {
        let tagged;
        let locale = match method {
            SortMethod::Default => locale,
            SortMethod::PinYin | SortMethod::Stroke => {
                let name = if method == SortMethod::PinYin {
                    "pinyin"
                } else {
                    "stroke"
                };
                let base = locale.bcp47().split("-u-").next().unwrap_or("zh");
                tagged =
                    Locale::from_bcp47(&format!("{base}-u-co-{name}")).map_err(locale_error)?;
                &tagged
            }
        };
        Ok(Self {
            primary: locale
                .collator_with(CollationStrength::Primary)
                .map_err(locale_error)?,
            secondary: locale
                .collator_with(CollationStrength::Secondary)
                .map_err(locale_error)?,
            tertiary: if case_sensitive {
                Some(
                    locale
                        .collator_with(CollationStrength::Tertiary)
                        .map_err(locale_error)?,
                )
            } else {
                None
            },
        })
    }

    /// Splits a string into weighted symbols and runs of ordinary text.
    pub fn key(&self, text: &str) -> TextKey {
        let mut toks = Vec::new();
        let mut ignorable = Vec::new();
        let mut run = String::new();
        for (i, c) in text.chars().enumerate() {
            if is_ignorable(c) {
                ignorable.push((u32::try_from(i).unwrap_or(u32::MAX), c));
            } else if let Some(rank) = PUNCT_ORDER.find(c) {
                if !run.is_empty() {
                    toks.push(Tok::Run(std::mem::take(&mut run)));
                }
                toks.push(Tok::Punct(u8::try_from(rank).unwrap_or(u8::MAX)));
            } else {
                // Excel sorts these ligatures as their letter pairs, with no difference at any level.
                match c {
                    'æ' | 'Æ' => run.push_str(if c == 'æ' { "ae" } else { "AE" }),
                    'œ' | 'Œ' => run.push_str(if c == 'œ' { "oe" } else { "OE" }),
                    'ß' => run.push_str("ss"),
                    _ => run.push(c),
                }
            }
        }
        if !run.is_empty() {
            toks.push(Tok::Run(run));
        }
        TextKey { toks, ignorable }
    }

    fn pass(a: &[Tok], b: &[Tok], collator: &Collator) -> Ordering {
        for (x, y) in a.iter().zip(b) {
            let o = match (x, y) {
                (Tok::Punct(p), Tok::Punct(q)) => p.cmp(q),
                (Tok::Punct(_), Tok::Run(_)) => Ordering::Less,
                (Tok::Run(_), Tok::Punct(_)) => Ordering::Greater,
                (Tok::Run(s), Tok::Run(t)) => collator.compare(s, t),
            };
            if o != Ordering::Equal {
                return o;
            }
        }
        a.len().cmp(&b.len())
    }

    /// Compares two prepared strings.
    pub fn compare(&self, a: &TextKey, b: &TextKey) -> Ordering {
        Self::pass(&a.toks, &b.toks, &self.primary)
            .then_with(|| Self::pass(&a.toks, &b.toks, &self.secondary))
            .then_with(|| match &self.tertiary {
                Some(t) => Self::pass(&a.toks, &b.toks, t),
                None => Ordering::Equal,
            })
            .then_with(|| a.ignorable.len().cmp(&b.ignorable.len()))
            .then_with(|| a.ignorable.cmp(&b.ignorable))
    }

    /// Compares two strings.
    pub fn compare_text(&self, a: &str, b: &str) -> Ordering {
        self.compare(&self.key(a), &self.key(b))
    }
}

// ---- cell classification and colours ---------------------------------------------------------------------

/// What a cell holds, as far as ordering and filtering care.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Kind<'a> {
    Blank,
    Number(f64),
    Text(&'a str),
    Bool(bool),
    Error,
}

pub(crate) fn kind_of(cell: Option<&Cell>) -> Kind<'_> {
    let Some(cell) = cell else {
        return Kind::Blank;
    };
    match &cell.value {
        CellValue::Empty => Kind::Blank,
        CellValue::Number(n) => Kind::Number(*n),
        CellValue::Bool(b) => Kind::Bool(*b),
        CellValue::Error(_) => Kind::Error,
        other => other.as_text().map_or(Kind::Blank, Kind::Text),
    }
}

fn fill_key(fill: &Fill, styles: &Styles) -> ColorKey {
    if fill.gradient || fill.pattern == FillPattern::None {
        return ColorKey::Automatic;
    }
    ColorKey::Rgb(styles.resolve_color(&fill.fg))
}

fn is_automatic(color: &Color) -> bool {
    match color.source {
        ColorSource::Auto => true,
        ColorSource::Theme(1) => color.tint == 0.0,
        ColorSource::Indexed(64..) => true,
        _ => false,
    }
}

fn color_key(color: &Color, styles: &Styles) -> ColorKey {
    if is_automatic(color) {
        ColorKey::Automatic
    } else {
        ColorKey::Rgb(styles.resolve_color(color))
    }
}

/// The fill colour of a cell with style index `xf`.
pub(crate) fn cell_fill_key(styles: &Styles, xf: Option<u32>) -> ColorKey {
    fill_key(&styles.cell_format(xf.unwrap_or(0)).fill, styles)
}

/// The font colour of a cell with style index `xf`.
pub(crate) fn cell_font_key(styles: &Styles, xf: Option<u32>) -> ColorKey {
    color_key(&styles.cell_format(xf.unwrap_or(0)).font.color, styles)
}

/// The colour a differential format stands for. Excel keeps the colour of a font colour filter in the fill of
/// the dxf; a sort condition uses the font element.
pub(crate) fn dxf_key(styles: &Styles, dxf_id: u32, font: bool) -> ColorKey {
    let Some(dxf) = styles.dxf(dxf_id) else {
        return ColorKey::Automatic;
    };
    if font && let Some(c) = &dxf.font.color {
        return color_key(c, styles);
    }
    let Some(fill) = &dxf.fill else {
        return ColorKey::Automatic;
    };
    if fill.pattern == FillPattern::None {
        return ColorKey::Automatic;
    }
    let color = if fill.pattern == FillPattern::Solid {
        fill.fg.or(fill.bg)
    } else {
        fill.bg.or(fill.fg)
    };
    color.map_or(ColorKey::Automatic, |c| color_key(&c, styles))
}

// ---- the sort ---------------------------------------------------------------------------------------

#[derive(Debug)]
enum Val {
    Blank,
    Num(f64),
    Text(TextKey),
    Bool(bool),
    Err,
}

impl Val {
    fn rank(&self) -> u8 {
        match self {
            Self::Num(_) => 0,
            Self::Text(_) => 1,
            Self::Bool(_) => 2,
            Self::Err => 3,
            Self::Blank => 4,
        }
    }
}

enum KeyColumn {
    Values(Vec<(Option<usize>, Val)>),
    Match(Vec<bool>),
}

fn cmp_val(a: &Val, b: &Val, order: &TextOrder) -> Ordering {
    match (a, b) {
        (Val::Num(x), Val::Num(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Val::Text(x), Val::Text(y)) => order.compare(x, y),
        (Val::Bool(x), Val::Bool(y)) => x.cmp(y),
        _ => a.rank().cmp(&b.rank()),
    }
}

fn has_shared_formula(ws: &Worksheet, rect: Rect) -> bool {
    let (r1, c1, r2, c2) = rect;
    ws.rows
        .iter()
        .filter(|r| (r1..=r2).contains(&r.r))
        .any(|r| {
            r.cells.iter().any(|c| {
                (c1..=c2).contains(&c.col)
                    && c.formula
                        .as_deref()
                        .is_some_and(|f| matches!(f.kind, FormulaKind::Shared { .. }))
            })
        })
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3
}

impl Worksheet {
    /// Merged ranges of the sheet.
    pub(crate) fn merged_ranges(&self) -> Vec<Rect> {
        self.after
            .iter()
            .filter_map(RawXml::as_element)
            .filter(|e| e.head.is_main("mergeCells"))
            .flat_map(XmlElement::elements)
            .filter(|e| e.head.is_main("mergeCell"))
            .filter_map(|e| e.head.attr("ref").and_then(parse_range))
            .collect()
    }

    /// Excel's header guess for `Range.Sort` (and the Sort dialog): the first line is a header when, in some
    /// column, its cell and the cell below differ in type (blank cells do not count; text that reads as a number
    /// counts as a number in the first line) or in format (font, fill, number format, alignment, bottom border).
    pub fn guess_header(&self, range: Rect, orientation: SortOrientation, styles: &Styles) -> bool {
        let (r1, c1, r2, c2) = range;
        let pairs: Vec<((u32, u32), (u32, u32))> = match orientation {
            SortOrientation::TopToBottom => {
                if r2 <= r1 {
                    return true;
                }
                (c1..=c2).map(|c| ((r1, c), (r1 + 1, c))).collect()
            }
            SortOrientation::LeftToRight => {
                if c2 <= c1 {
                    return true;
                }
                (r1..=r2).map(|r| ((r, c1), (r, c1 + 1))).collect()
            }
        };
        let class = |kind: Kind<'_>, first: bool| -> Option<u8> {
            match kind {
                Kind::Blank => None,
                Kind::Number(_) => Some(0),
                Kind::Text(t) if first && parse_value_text_in(t, false).is_some() => Some(0),
                Kind::Text(_) => Some(1),
                Kind::Bool(_) => Some(2),
                Kind::Error => Some(3),
            }
        };
        pairs.into_iter().any(|(a, b)| {
            let (ca, cb) = (self.cell(a.0, a.1), self.cell(b.0, b.1));
            if let (Some(x), Some(y)) = (class(kind_of(ca), true), class(kind_of(cb), false))
                && x != y
            {
                return true;
            }
            let fa = styles.cell_format(ca.and_then(|c| c.style).unwrap_or(0));
            let fb = styles.cell_format(cb.and_then(|c| c.style).unwrap_or(0));
            fa.font != fb.font
                || fa.fill != fb.fill
                || fa.num_fmt_id != fb.num_fmt_id
                || fa.alignment != fb.alignment
                || fa.border.bottom != fb.border.bottom
        })
    }

    /// Sorts `spec.range`. See the module documentation for the order. Fails, leaving the sheet as it was, for a
    /// range that cuts an array formula or a merged cell, a key outside the range, or no key at all.
    pub fn sort_range(
        &mut self,
        spec: &SortSpec,
        styles: &Styles,
        locale: &Locale,
    ) -> Result<SortOutcome, XlsxError> {
        let (r1, c1, r2, c2) = spec.range;
        let bad = |why: &str| Err(XlsxError::UnsupportedEdit(format!("sort: {why}")));
        if r1 < 1 || c1 < 1 || r2 < r1 || c2 < c1 || r2 > crate::MAX_ROW || c2 > crate::MAX_COL {
            return bad("the range is outside the worksheet");
        }
        if spec.keys.is_empty() {
            return bad("no sort key");
        }
        if spec.keys.len() > MAX_SORT_KEYS {
            return bad("more than 64 sort keys");
        }
        let ttb = spec.orientation == SortOrientation::TopToBottom;
        let (p1, p2, s1, s2) = if ttb {
            (r1, r2, c1, c2)
        } else {
            (c1, c2, r1, r2)
        };
        for key in &spec.keys {
            if !(s1..=s2).contains(&key.index) {
                return bad("a sort key lies outside the range");
            }
            if key.on == SortOn::Icon {
                return bad("sorting by icon is not supported");
            }
        }
        if self.locked.iter().any(|&l| intersects((r1, c1, r2, c2), l)) {
            return bad("part of an array formula or data table is in the range");
        }
        if self
            .merged_ranges()
            .iter()
            .any(|&m| intersects((r1, c1, r2, c2), m))
        {
            return bad("the range contains merged cells");
        }
        // Excel (through COM) keeps no header column in a left-to-right sort.
        let header = ttb
            && match spec.header {
                HeaderMode::Yes => true,
                HeaderMode::No => false,
                HeaderMode::Guess => self.guess_header(spec.range, spec.orientation, styles),
            };
        let first = p1 + u32::from(header);
        if first >= p2 {
            // Nothing to put in order: at most one line below the header.
            return Ok(SortOutcome {
                header,
                order: (0..u32::from(first == p2)).collect(),
                formulas_shifted: 0,
            });
        }
        let order = TextOrder::new(locale, spec.method, spec.case_sensitive)?;
        if has_shared_formula(self, (r1, c1, r2, c2)) {
            self.materialise_all_shared();
        }

        // Lift the cells out of the sheet, one line per moving row (or column).
        let n = (p2 - first + 1) as usize;
        let width = (s2 - s1 + 1) as usize;
        let mut lines: Vec<Vec<Option<Cell>>> = (0..n).map(|_| vec![None; width]).collect();
        if ttb {
            for row in self.rows.iter_mut().filter(|r| (first..=p2).contains(&r.r)) {
                let line = (row.r - first) as usize;
                for cell in std::mem::take(&mut row.cells) {
                    if (c1..=c2).contains(&cell.col) {
                        let slot = (cell.col - c1) as usize;
                        lines[line][slot] = Some(cell);
                    } else {
                        row.cells.push(cell);
                    }
                }
            }
        } else {
            for row in self.rows.iter_mut().filter(|r| (r1..=r2).contains(&r.r)) {
                let slot = (row.r - r1) as usize;
                for cell in std::mem::take(&mut row.cells) {
                    if (first..=c2).contains(&cell.col) {
                        let line = (cell.col - first) as usize;
                        lines[line][slot] = Some(cell);
                    } else {
                        row.cells.push(cell);
                    }
                }
            }
        }

        // Key columns.
        let columns: Vec<KeyColumn> = spec
            .keys
            .iter()
            .map(|key| {
                let slot = (key.index - s1) as usize;
                let cells = lines.iter().map(|l| l[slot].as_ref());
                match key.on {
                    SortOn::Value | SortOn::Icon => {
                        let list: Option<Vec<String>> = key
                            .custom_list
                            .as_ref()
                            .map(|l| l.iter().map(|s| s.trim().to_lowercase()).collect());
                        KeyColumn::Values(
                            cells
                                .map(|c| {
                                    let kind = kind_of(c);
                                    let idx = match (kind, &list) {
                                        (Kind::Text(t), Some(items)) => {
                                            let t = t.trim().to_lowercase();
                                            items.iter().position(|i| *i == t)
                                        }
                                        _ => None,
                                    };
                                    let val = match kind {
                                        Kind::Blank => Val::Blank,
                                        Kind::Number(n) => Val::Num(n),
                                        Kind::Text(t) => Val::Text(order.key(t)),
                                        Kind::Bool(b) => Val::Bool(b),
                                        Kind::Error => Val::Err,
                                    };
                                    (idx, val)
                                })
                                .collect(),
                        )
                    }
                    SortOn::CellColor => KeyColumn::Match(
                        cells
                            .map(|c| cell_fill_key(styles, c.and_then(|c| c.style)) == key.color)
                            .collect(),
                    ),
                    SortOn::FontColor => KeyColumn::Match(
                        cells
                            .map(|c| cell_font_key(styles, c.and_then(|c| c.style)) == key.color)
                            .collect(),
                    ),
                }
            })
            .collect();

        let mut perm: Vec<usize> = (0..n).collect();
        perm.sort_by(|&a, &b| {
            for (key, col) in spec.keys.iter().zip(&columns) {
                let ord = match col {
                    KeyColumn::Values(v) => {
                        let (ia, va) = &v[a];
                        let (ib, vb) = &v[b];
                        match (matches!(va, Val::Blank), matches!(vb, Val::Blank)) {
                            (true, true) => Ordering::Equal,
                            (true, false) => return Ordering::Greater,
                            (false, true) => return Ordering::Less,
                            (false, false) => {
                                let asc = match (ia, ib) {
                                    (Some(x), Some(y)) => x.cmp(y),
                                    (Some(_), None) => Ordering::Less,
                                    (None, Some(_)) => Ordering::Greater,
                                    (None, None) => cmp_val(va, vb, &order),
                                };
                                if key.descending { asc.reverse() } else { asc }
                            }
                        }
                    }
                    KeyColumn::Match(m) => {
                        let asc = m[b].cmp(&m[a]);
                        if key.descending { asc.reverse() } else { asc }
                    }
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            Ordering::Equal
        });

        // Put the lines back in their new places.
        let mut shifted = 0usize;
        let mut lines: Vec<Option<Vec<Option<Cell>>>> = lines.into_iter().map(Some).collect();
        for (k, &old) in perm.iter().enumerate() {
            let delta = k as i64 - old as i64;
            let Some(line) = lines[old].take() else {
                continue;
            };
            for (slot, cell) in line.into_iter().enumerate() {
                let Some(mut cell) = cell else { continue };
                if delta != 0
                    && let Some(f) = cell.formula.as_deref_mut()
                    && !f.text.is_empty()
                {
                    let (dr, dc) = if ttb { (delta, 0) } else { (0, delta) };
                    let text = shift_formula(&f.text, dr, dc);
                    if text != f.text {
                        f.text = text;
                        shifted += 1;
                    }
                }
                let (row, col) = if ttb {
                    (first + k as u32, c1 + slot as u32)
                } else {
                    (r1 + slot as u32, first + k as u32)
                };
                cell.col = col;
                *self.cell_slot(row, col) = cell;
            }
        }
        self.index_formulas();
        Ok(SortOutcome {
            header,
            order: perm.iter().map(|&i| i as u32).collect(),
            formulas_shifted: shifted,
        })
    }
}

// ---- sortState ---------------------------------------------------------------------------------------

/// What a sort condition orders by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Value,
    CellColor,
    FontColor,
    Icon,
}

/// One `sortCondition`.
#[derive(Debug, Clone, PartialEq)]
pub struct SortCondition {
    /// The key cells, for example `B2:B21`.
    pub range: Rect,
    pub descending: bool,
    pub sort_by: SortBy,
    /// `customList`: the items, comma separated.
    pub custom_list: Option<String>,
    pub dxf_id: Option<u32>,
    pub icon_set: Option<String>,
    pub icon_id: Option<u32>,
}

/// The `sortState` element of a worksheet, auto filter or table: the last sort applied there.
#[derive(Debug, Clone, PartialEq)]
pub struct SortState {
    /// The sorted cells, header line not included.
    pub range: Rect,
    pub column_sort: bool,
    pub case_sensitive: bool,
    pub method: SortMethod,
    pub conditions: Vec<SortCondition>,
    /// Attributes this model does not know (`xr:uid`).
    other_attrs: Vec<Attr>,
    ext: Vec<RawXml>,
}

fn flag(head: &ElementHead, name: &str) -> bool {
    matches!(head.attr(name).map(str::trim), Some("1" | "true"))
}

fn num(head: &ElementHead, name: &str) -> Option<u32> {
    head.attr(name).and_then(|v| v.trim().parse().ok())
}

fn bad_part(reason: String) -> XlsxError {
    XlsxError::Malformed {
        part: "worksheet".to_owned(),
        reason,
    }
}

impl SortState {
    /// A sort state with no conditions yet.
    pub fn new(range: Rect) -> Self {
        Self {
            range,
            column_sort: false,
            case_sensitive: false,
            method: SortMethod::Default,
            conditions: Vec::new(),
            other_attrs: Vec::new(),
            ext: Vec::new(),
        }
    }

    /// Reads a `sortState` element.
    pub fn from_element(el: &XmlElement) -> Result<Self, XlsxError> {
        let range = el
            .head
            .attr("ref")
            .and_then(parse_range)
            .ok_or_else(|| bad_part("sortState without a valid ref".to_owned()))?;
        let mut state = Self::new(range);
        state.column_sort = flag(&el.head, "columnSort");
        state.case_sensitive = flag(&el.head, "caseSensitive");
        state.method = match el.head.attr("sortMethod") {
            Some("pinYin") => SortMethod::PinYin,
            Some("stroke") => SortMethod::Stroke,
            _ => SortMethod::Default,
        };
        state.other_attrs = el
            .head
            .attrs
            .iter()
            .filter(|a| {
                a.ns.is_some()
                    || !matches!(
                        a.name.as_str(),
                        "ref" | "columnSort" | "caseSensitive" | "sortMethod"
                    )
            })
            .cloned()
            .collect();
        for child in &el.children {
            match child {
                RawXml::Element(c) if c.head.is_main("sortCondition") => {
                    let range =
                        c.head.attr("ref").and_then(parse_range).ok_or_else(|| {
                            bad_part("sortCondition without a valid ref".to_owned())
                        })?;
                    state.conditions.push(SortCondition {
                        range,
                        descending: flag(&c.head, "descending"),
                        sort_by: match c.head.attr("sortBy") {
                            Some("cellColor") => SortBy::CellColor,
                            Some("fontColor") => SortBy::FontColor,
                            Some("icon") => SortBy::Icon,
                            _ => SortBy::Value,
                        },
                        custom_list: c.head.attr("customList").map(str::to_owned),
                        dxf_id: num(&c.head, "dxfId"),
                        icon_set: c.head.attr("iconSet").map(str::to_owned),
                        icon_id: num(&c.head, "iconId"),
                    });
                }
                RawXml::Element(c) if c.head.local_name() == "extLst" => {
                    state.ext.push(child.clone());
                }
                _ => {}
            }
        }
        Ok(state)
    }

    /// The element, named like `like` (same namespace and prefix).
    pub fn to_element(&self, like: &ElementHead) -> XmlElement {
        let mut el = XmlElement {
            head: like.sibling("sortState"),
            children: Vec::new(),
        };
        if self.column_sort {
            el.head.set_attr("columnSort", "1");
        }
        if self.case_sensitive {
            el.head.set_attr("caseSensitive", "1");
        }
        match self.method {
            SortMethod::Default => {}
            SortMethod::PinYin => el.head.set_attr("sortMethod", "pinYin"),
            SortMethod::Stroke => el.head.set_attr("sortMethod", "stroke"),
        }
        el.head.set_attr("ref", &range_a1(self.range));
        el.head.attrs.extend(self.other_attrs.iter().cloned());
        for c in &self.conditions {
            let mut cond = XmlElement {
                head: like.sibling("sortCondition"),
                children: Vec::new(),
            };
            if c.descending {
                cond.head.set_attr("descending", "1");
            }
            match c.sort_by {
                SortBy::Value => {}
                SortBy::CellColor => cond.head.set_attr("sortBy", "cellColor"),
                SortBy::FontColor => cond.head.set_attr("sortBy", "fontColor"),
                SortBy::Icon => cond.head.set_attr("sortBy", "icon"),
            }
            cond.head.set_attr("ref", &range_a1(c.range));
            if let Some(list) = &c.custom_list {
                cond.head.set_attr("customList", list);
            }
            if let Some(id) = c.dxf_id {
                cond.head.set_attr("dxfId", &id.to_string());
            }
            if let Some(set) = &c.icon_set {
                cond.head.set_attr("iconSet", set);
            }
            if let Some(id) = c.icon_id {
                cond.head.set_attr("iconId", &id.to_string());
            }
            el.children.push(RawXml::Element(cond));
        }
        el.children.extend(self.ext.iter().cloned());
        el
    }

    /// The sort this state records, ready for [`Worksheet::sort_range`]: the cells in `range` move and none of
    /// them is a header. Colour conditions take their colour from `styles` through `dxfId`.
    pub fn to_spec(&self, styles: &Styles) -> SortSpec {
        let keys = self
            .conditions
            .iter()
            .map(|c| {
                let index = if self.column_sort {
                    c.range.0
                } else {
                    c.range.1
                };
                let (on, font) = match c.sort_by {
                    SortBy::Value => (SortOn::Value, false),
                    SortBy::CellColor => (SortOn::CellColor, false),
                    SortBy::FontColor => (SortOn::FontColor, true),
                    SortBy::Icon => (SortOn::Icon, false),
                };
                SortKey {
                    index,
                    on,
                    descending: c.descending,
                    color: c
                        .dxf_id
                        .map_or(ColorKey::Automatic, |id| dxf_key(styles, id, font)),
                    custom_list: c
                        .custom_list
                        .as_ref()
                        .map(|l| l.split(',').map(str::to_owned).collect()),
                }
            })
            .collect();
        SortSpec {
            range: self.range,
            header: HeaderMode::No,
            orientation: if self.column_sort {
                SortOrientation::LeftToRight
            } else {
                SortOrientation::TopToBottom
            },
            case_sensitive: self.case_sensitive,
            method: self.method,
            keys,
        }
    }
}

/// Local names of the worksheet children that follow `sheetData`, in schema order.
const AFTER_SHEET_DATA: [&str; 33] = [
    "sheetCalcPr",
    "sheetProtection",
    "protectedRanges",
    "scenarios",
    "autoFilter",
    "sortState",
    "dataConsolidate",
    "customSheetViews",
    "mergeCells",
    "phoneticPr",
    "conditionalFormatting",
    "dataValidations",
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];

impl Worksheet {
    pub(crate) fn root_element(&self, local: &str) -> Option<&XmlElement> {
        self.after
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main(local))
    }

    /// Replaces (or removes, with `None`) the worksheet child `local`, keeping the schema order.
    pub(crate) fn put_root_element(&mut self, local: &str, element: Option<XmlElement>) {
        let at = self
            .after
            .iter()
            .position(|n| n.as_element().is_some_and(|e| e.head.is_main(local)));
        match (at, element) {
            (Some(i), Some(el)) => self.after[i] = RawXml::Element(el),
            (Some(i), None) => {
                self.after.remove(i);
            }
            (None, Some(el)) => {
                let rank = |name: &str| AFTER_SHEET_DATA.iter().position(|n| *n == name);
                let mine = rank(local).unwrap_or(AFTER_SHEET_DATA.len());
                let pos = self
                    .after
                    .iter()
                    .position(|n| {
                        n.as_element()
                            .and_then(|e| rank(e.head.local_name()))
                            .is_some_and(|r| r > mine)
                    })
                    .unwrap_or(self.after.len());
                self.after.insert(pos, RawXml::Element(el));
            }
            (None, None) => {}
        }
    }

    /// The worksheet-level `sortState`, when there is one.
    pub fn sort_state(&self) -> Option<SortState> {
        SortState::from_element(self.root_element("sortState")?).ok()
    }

    /// Sets or removes the worksheet-level `sortState`.
    pub fn set_sort_state(&mut self, state: Option<&SortState>) {
        let el = state.map(|s| s.to_element(&self.sheet_data_head));
        self.put_root_element("sortState", el);
    }
}
