//! Paragraph layout: text and runs to shaped atoms to lines (independent of the page).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::Range;
use std::sync::Arc;

use open365_docx::{
    Alignment, Drawing, EffectiveParagraph, EffectiveRun, IndentOverride, Inline, Paragraph,
    ParagraphProps, RawXml, Run, RunContent, RunLayoutProps, Styles, TableCellFormat, VertAlign,
};
use open365_text::{
    BreakKind, FaceId, FontChain, FontLibrary, FontStyle, ShapeFeatures, break_opportunities,
    shape_run,
};

use crate::ParaRole;
use crate::metrics::{LineMetricModel, PictureLine, line_height};
use crate::numbering::{LabelAlign, ListLabel, Suffix};
use crate::props::{TabKind, TabLeader, TabStop};

const EPS: f64 = 1e-6;

/// The character that stands for a drawing in the paragraph text. Word counts a drawing as one
/// character of the story.
const OBJECT_CHAR: &str = "\u{fffc}";

/// A justified line may squeeze its spaces by up to this fraction of their width to take one
/// more word (Word 2013+ layout). Fitted to layout-align and layout-justify: 0.2 gives the best
/// match, but Word also weighs the alternative break, which is not modelled yet.
const SPACE_SHRINK: f64 = 0.24;

/// Word's justified lines end a little short of the margin, about this much per stretched space
/// (points): in `layout-justify` the glyphs of a 16-space line sit up to 0.6 pt right of Word's
/// at the line end, and the best constant over 110,000 glyphs is 0.02 pt (82.8 % of the glyphs
/// within 0.5 pt, 87.2 % with it; 0.01 gives 86.5 %, 0.035 85.3 %). An empirical fit, not a
/// mechanism: Word's advances and spaces are whole multiples of 0.24 pt, see the document.
const JUSTIFY_SHORTFALL: f64 = 0.02;
/// Lines with fewer stretched spaces (narrow columns) are not shortened: `layout-cols-line`.
const JUSTIFY_SHORTFALL_MIN_SPACES: usize = 10;

/// Word draws a super- or subscript run at this fraction of its size (measured:
/// 10, 12 and 20 pt runs give 6.48, 7.92 and 12.96 pt on Word's 0.24 pt grid).
const VERT_SIZE: f64 = 0.65;
/// Raise of a superscript run, in em of the unreduced size (measured at about 0.33).
const VERT_SUPER_RAISE: f64 = 1.0 / 3.0;
/// Drop of a subscript run, in em of the unreduced size (measured at about 0.08).
const VERT_SUB_LOWER: f64 = 1.0 / 12.0;

/// Resolved formatting of a run of text.
#[derive(Debug, Clone)]
pub(crate) struct RunStyle {
    /// Family name the document asked for.
    pub requested: String,
    pub chain: FontChain,
    pub face: FaceId,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<[u8; 3]>,
    /// Pair kerning on (`w:kern` at or below the run's size).
    pub kerning: bool,
    /// Extra character spacing in points (`w:spacing`).
    pub spacing: f64,
    /// Horizontal glyph scale, 1.0 for 100 percent (`w:w`).
    pub scale: f64,
    /// A note reference mark.
    pub superscript: bool,
    /// `w:vertAlign`: size is already reduced for super- and subscript.
    pub vert_align: open365_docx::VertAlign,
    /// Shift of the glyphs from the baseline in points, positive up (`vert_align`).
    pub raise: f64,
    /// Size the line metrics come from: [`Self::size`], except for the reduced capitals of
    /// small caps, which sit on a line sized for the full-size font.
    pub metric_size: f64,
    /// `w:caps` or `w:smallCaps`.
    pub case: CaseMode,
}

/// How a run's lower-case letters are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaseMode {
    Normal,
    /// `w:caps`: capitals of the run's size.
    Caps,
    /// `w:smallCaps`: capitals of [`small_cap_size`].
    SmallCaps,
}

/// Size of the reduced capitals of small caps: 0.8 of the font size, rounded to the nearest half
/// point. Measured with layout-caps (Calibri and Cambria, 8 to 36 pt): the PDF font sizes are
/// 6.5, 8, 9, 9.5, 11, 14.5, 19 and 29 pt for 8, 10, 11, 12, 14, 18, 24 and 36 pt text.
pub(crate) fn small_cap_size(size: f64) -> f64 {
    (size * 2.0 * 0.8).round().max(1.0) / 2.0
}

/// The capital a lower-case letter stands for, when it has exactly one.
fn capital(c: char) -> Option<char> {
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => Some(u),
        _ => None,
    }
}

/// Height a border takes: Word draws the composite styles with several rules, so their extent is
/// a multiple of `w:sz` plus fixed gaps. Fitted to layout-pbdr-styles (single rules and the
/// dotted and dashed ones take the width; double 3x, triple 5x; the thin and thick pairs and
/// triples a width plus 1.5, 3 or 2.4 pt, the medium-gap pair twice the width).
pub(crate) fn border_extent(b: &open365_docx::Border) -> f64 {
    if !b.is_visible() {
        return 0.0;
    }
    let w = b.width;
    match b.style.as_str() {
        "double" => 3.0 * w,
        "triple" => 5.0 * w,
        "thinThickSmallGap" | "thickThinSmallGap" => w + 1.5,
        "thinThickThinSmallGap" => w + 3.0,
        "thinThickMediumGap" | "thickThinMediumGap" => 2.0 * w,
        "thinThickThinMediumGap" => 3.0 * w,
        "thinThickLargeGap" | "thickThinLargeGap" => w + 2.4,
        "thinThickThinLargeGap" => 2.0 * w + 3.0,
        _ => w,
    }
}

impl ParaLayout {
    /// Height the top border and its distance from the text add above the first line.
    pub(crate) fn border_top(&self) -> f64 {
        self.flow
            .borders
            .top
            .as_ref()
            .map_or(0.0, |b| border_extent(b) + b.space)
    }

    /// Height the bottom border and its distance from the text add below the last line.
    pub(crate) fn border_bottom(&self) -> f64 {
        self.flow
            .borders
            .bottom
            .as_ref()
            .map_or(0.0, |b| border_extent(b) + b.space)
    }

    /// Height of the between rule and its distance on both sides, inside a group.
    pub(crate) fn between_extent(&self) -> f64 {
        self.flow
            .borders
            .between
            .as_ref()
            .map_or(0.0, |b| border_extent(b) + 2.0 * b.space)
    }

    /// The border box information a line of this paragraph carries. `origin_x` is the left of the
    /// area the paragraph was laid out in; `join_prev` and `join_next` say whether the
    /// neighbouring paragraphs continue the box.
    pub(crate) fn line_border(
        &self,
        lb: &LineBox,
        origin_x: f64,
        join_prev: bool,
        join_next: bool,
    ) -> Option<crate::rules::LineBorder> {
        let spec = self.border.clone()?;
        Some(crate::rules::LineBorder {
            spec,
            left: origin_x + f64::from(self.props.indent_left_pt),
            right: origin_x + self.fill.text_width - f64::from(self.props.indent_right_pt),
            first: lb.atoms.start == 0,
            last: lb.atoms.end >= self.atoms.len(),
            join_prev,
            join_next,
        })
    }

    /// Whether `next` continues this paragraph's border group: borders present and identical,
    /// and the same left and right indents (alignment does not matter; layout-pbdr-groups).
    pub(crate) fn joins(&self, next: &Self) -> bool {
        !self.flow.borders.is_empty()
            && self.flow.borders == next.flow.borders
            && self.props.indent_left_pt == next.props.indent_left_pt
            && self.props.indent_right_pt == next.props.indent_right_pt
    }
}

/// A stretch of a span shaped on its own.
struct Piece {
    range: Range<usize>,
    style: usize,
    /// Shape the capitals of the text, not the text itself.
    upper: bool,
}

/// Splits `range` of `text` into the stretches `w:caps` and `w:smallCaps` shape separately:
/// all of it as capitals, or the lower-case letters and the spaces as reduced capitals between
/// the other characters at full size (digits, punctuation and capitals are not reduced:
/// layout-caps-chars). Reduced styles are added to `styles` once per base style.
fn case_pieces(
    text: &str,
    range: &Range<usize>,
    style: usize,
    styles: &mut Vec<RunStyle>,
    reduced: &mut HashMap<usize, usize>,
) -> Vec<Piece> {
    match styles[style].case {
        CaseMode::Normal => vec![Piece {
            range: range.clone(),
            style,
            upper: false,
        }],
        CaseMode::Caps => vec![Piece {
            range: range.clone(),
            style,
            upper: true,
        }],
        CaseMode::SmallCaps => {
            let small = *reduced.entry(style).or_insert_with(|| {
                let mut st = styles[style].clone();
                st.size = small_cap_size(st.size);
                st.case = CaseMode::Normal;
                styles.push(st);
                styles.len() - 1
            });
            let mut pieces: Vec<Piece> = Vec::new();
            for (i, c) in text[range.clone()].char_indices() {
                let lower = c == ' ' || capital(c).is_some_and(|u| u != c);
                let at = range.start + i;
                match pieces.last_mut() {
                    Some(p) if p.upper == lower => p.range.end = at + c.len_utf8(),
                    _ => pieces.push(Piece {
                        range: at..at + c.len_utf8(),
                        style: if lower { small } else { style },
                        upper: lower,
                    }),
                }
            }
            pieces
        }
    }
}

/// `text` with every character that has one capital replaced by it, and for every byte of the
/// result the byte offset of its character in `text`.
fn capitals(text: &str) -> (String, Vec<u32>) {
    let mut out = String::with_capacity(text.len());
    let mut back = Vec::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        let u = capital(c).unwrap_or(c);
        out.push(u);
        back.extend(std::iter::repeat_n(i as u32, u.len_utf8()));
    }
    (out, back)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomClass {
    Text,
    Space,
    Tab,
    LineBreak,
    PageBreak,
    Mark,
    /// An inline picture (a box on the baseline) or the anchor of a floating one (zero width).
    Object,
}

/// A drawing in a paragraph: its box in points, and how it sits in the text.
#[derive(Debug, Clone)]
pub(crate) struct ObjectBox {
    pub drawing: Drawing,
    pub flags: crate::float::AnchorFlags,
    /// Advance of an inline picture: its width plus the effect extent.
    pub width: f64,
    /// Height of an inline picture above the baseline: its height plus the effect extent.
    pub height: f64,
    /// Index of the object's atom.
    pub atom: usize,
    /// The laid-out text of a text box shape.
    pub text_box: Option<crate::textbox::BoxRef>,
    /// A floating drawing in a table cell that Word positions against the page, not the cell:
    /// `layoutInCell="0"` in a document of compatibility mode 14 or lower.
    pub page_anchored: bool,
}

impl ObjectBox {
    pub(crate) fn inline(&self) -> bool {
        !self.drawing.anchored
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Atom {
    pub byte_start: usize,
    /// Natural advance in points (tabs and marks are computed at line time).
    pub adv: f64,
    pub style: usize,
    pub glyphs: Range<usize>,
    pub class: AtomClass,
    pub ch: char,
    /// Index into [`ParaLayout::objects`] for an [`AtomClass::Object`].
    pub object: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlacedGlyphRaw {
    pub face: FaceId,
    pub glyph_id: u32,
    pub x_offset: f64,
    pub y_offset: f64,
    pub adv: f64,
    /// Byte offset of the glyph's cluster in the shaped text.
    pub cluster: u32,
}

/// A line of a paragraph, positioned horizontally in text-area coordinates.
#[derive(Debug, Clone)]
pub(crate) struct LineBox {
    /// Story offsets (UTF-16 units) of the first and one past the last character.
    pub start: usize,
    pub end: usize,
    pub height: f64,
    /// Distance from the line top to the baseline.
    pub baseline_off: f64,
    /// X of the first character, text-area coordinates.
    pub left: f64,
    /// X of the last character's left edge.
    pub last_x: f64,
    pub atoms: Range<usize>,
    /// Space the page flow leaves above the line to clear a picture. Zero when the paragraph
    /// was laid out alone.
    pub lead: f64,
    /// Atom x positions (text-area coordinates), parallel to `atoms`.
    pub xs: Vec<f64>,
    /// True when the line ends with a page or column break; the next line starts a page (or a
    /// column, see `column_break_after`).
    pub page_break_after: bool,
    /// The break is a column break (`w:br w:type="column"`): the next line starts the next
    /// column, or a page in a single-column section.
    pub column_break_after: bool,
    /// Spacing below the text of the line that a page need not have room for: a multiple above
    /// one adds `(m - 1)` font heights under the descent, and Word keeps the line on the page
    /// when its text fits (layout-face-multiple).
    pub slack: f64,
    /// Tab leaders on the line.
    pub leaders: Vec<LeaderSpan>,
}

/// The gap a tab with a leader leaves, in text-area coordinates.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LeaderSpan {
    /// The tab atom.
    pub atom: usize,
    pub x0: f64,
    pub x1: f64,
    pub ch: char,
}

/// The glyph of a leader character in one run style and its pitch (Word's whole-pixel advance).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LeaderGlyph {
    pub style: usize,
    pub ch: char,
    pub face: FaceId,
    pub glyph_id: u32,
    pub pitch: f64,
}

/// What line breaking needs besides the atoms, so that a paragraph can be broken again when
/// pictures change the room beside a line (see [`ParaLayout::builder`]).
#[derive(Debug, Clone)]
pub(crate) struct FillData {
    pub text_width: f64,
    /// Break opportunities: byte offset and kind, sorted.
    pub opp: Vec<(usize, BreakKind)>,
    /// Story offset (UTF-16 units) of every atom start, and one past the end.
    pub story_of: Vec<usize>,
    /// Where the text of the first line starts when a list label precedes it.
    pub first_x: Option<f64>,
    /// Style of the list label, which also sizes the first line.
    pub label_style: Option<usize>,
    pub default_tab: f64,
    pub model: LineMetricModel,
    /// Ascent and descent of every style.
    pub metrics: Vec<(f64, f64)>,
}

impl ParaLayout {
    /// A line breaker over this paragraph.
    pub(crate) fn builder(&self) -> LineBuilder<'_> {
        LineBuilder {
            atoms: &self.atoms,
            props: &self.props,
            flow: &self.flow,
            objects: &self.objects,
            fd: &self.fill,
        }
    }
}

/// The list label of a numbered paragraph, drawn left of the first line.
#[derive(Debug)]
pub(crate) struct LabelLayout {
    /// Index into [`ParaLayout::styles`].
    pub style: usize,
    pub glyphs: Vec<PlacedGlyphRaw>,
    /// The label text its glyphs were shaped from.
    pub text: Arc<str>,
    /// X of the label's first glyph, text-area coordinates.
    pub x: f64,
}

/// A paragraph laid out into lines, ready for pagination.
#[derive(Debug)]
pub(crate) struct ParaLayout {
    /// Position among the paragraphs of its layout unit (a body paragraph or a whole table).
    pub index: usize,
    pub props: EffectiveParagraph,
    pub flow: ParagraphProps,
    /// Paragraph style id, the default paragraph style when unset.
    pub style_id: Option<String>,
    pub label: Option<LabelLayout>,
    pub lines: Vec<LineBox>,
    pub atoms: Vec<Atom>,
    pub glyphs: Vec<PlacedGlyphRaw>,
    pub styles: Vec<RunStyle>,
    /// Pictures of the paragraph in text order.
    pub objects: Vec<ObjectBox>,
    /// Everything needed to break lines again at another width or position.
    pub fill: FillData,
    pub story_end: usize,
    /// Footnote references: the atom of the mark and the footnote id.
    pub foot_refs: Vec<(usize, i64)>,
    /// The paragraph is a note separator: Word draws a rule of [`SEPARATOR_WIDTH`] where the
    /// separator character stands.
    pub separator: bool,
    /// The collected text of the paragraph (hidden field codes left out, mark included).
    pub text: Arc<str>,
    pub role: ParaRole,
    /// The paragraph's borders, `None` without any.
    pub border: Option<Arc<crate::rules::BorderSpec>>,
    /// The paragraph holds nothing but a section break: it takes no height and no spacing, and
    /// the spacing around it passes straight through.
    pub section_mark: bool,
    /// Glyphs of the tab leaders of the paragraph's tab stops, per style of its tabs.
    pub leader_glyphs: Vec<LeaderGlyph>,
}

/// Length of the footnote and endnote separator rule in points (measured: the paragraph mark
/// after it lands 144 pt from the text area's left edge, for any page width).
pub(crate) const SEPARATOR_WIDTH: f64 = 144.0;

/// The values Word recomputes in headers and footers when it lays a page out.
#[derive(Clone, Copy, Default)]
pub(crate) struct FieldEnv {
    /// `None` keeps every field's cached result (body text, which is laid out page-independently).
    pub page: Option<usize>,
    pub num_pages: usize,
    pub section_pages: usize,
    pub section: usize,
}

/// Ordinals of the note references in document order.
#[derive(Default, Debug)]
pub(crate) struct NoteOrder {
    pub foot: HashMap<i64, usize>,
    pub end: HashMap<i64, usize>,
}

impl NoteOrder {
    /// The mark text: footnotes count 1, 2, 3; endnotes i, ii, iii.
    pub(crate) fn label(&self, end: bool, id: i64) -> String {
        let map = if end { &self.end } else { &self.foot };
        mark_text(end, map.get(&id).copied().unwrap_or(0))
    }
}

pub(crate) fn mark_text(end: bool, ordinal: usize) -> String {
    if !end {
        return ordinal.to_string();
    }
    let mut n = ordinal;
    let mut out = String::new();
    for (v, r) in [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ] {
        while n >= v {
            out.push_str(r);
            n -= v;
        }
    }
    out
}

#[derive(Clone, Copy)]
pub(crate) struct Context<'a> {
    pub fonts: &'a FontLibrary,
    pub styles: Option<&'a Styles>,
    /// Formatting the table style gives the paragraphs of the cell being laid out.
    pub table: Option<&'a TableCellFormat>,
    /// `w:defaultTabStop` in points.
    pub default_tab: f64,
    pub model: LineMetricModel,
    pub fields: FieldEnv,
    pub order: Option<&'a NoteOrder>,
    /// (is endnote, ordinal) of the note being laid out: what its `w:footnoteRef` shows.
    pub own_mark: Option<(bool, usize)>,
    /// External hyperlink targets of the main part by relationship id.
    pub links: Option<&'a HashMap<String, String>>,
    /// `w:compatibilityMode` of the document: 15 and later ignore `layoutInCell`.
    pub compat: u32,
}

fn fallback_run(er: Option<EffectiveRun>) -> EffectiveRun {
    er.unwrap_or(EffectiveRun {
        font: "Calibri".to_owned(),
        size_half_points: 22,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        color: None,
    })
}

fn style_from(ctx: &Context<'_>, er: &EffectiveRun, lay: &RunLayoutProps) -> RunStyle {
    let fs = FontStyle {
        bold: er.bold,
        italic: er.italic,
    };
    let chain = ctx.fonts.chain(&er.font, fs);
    let face = chain
        .faces
        .first()
        .copied()
        .unwrap_or_else(|| FaceId::from_index(0));
    let full_size = f64::from(er.size_half_points) / 2.0;
    let (size_factor, raise_em) = match lay.vert_align {
        VertAlign::Baseline => (1.0, 0.0),
        VertAlign::Superscript => (VERT_SIZE, VERT_SUPER_RAISE),
        VertAlign::Subscript => (VERT_SIZE, -VERT_SUB_LOWER),
    };
    RunStyle {
        requested: er.font.clone(),
        chain,
        face,
        size: full_size * size_factor,
        bold: er.bold,
        italic: er.italic,
        underline: er.underline,
        strike: er.strike,
        color: er.color,
        kerning: lay.kern_half_points > 0 && er.size_half_points >= lay.kern_half_points,
        spacing: f64::from(lay.spacing_pt()),
        scale: f64::from(lay.scale_percent) / 100.0,
        superscript: false,
        vert_align: lay.vert_align,
        raise: full_size * raise_em,
        metric_size: full_size,
        case: if lay.caps {
            CaseMode::Caps
        } else if lay.small_caps {
            CaseMode::SmallCaps
        } else {
            CaseMode::Normal
        },
    }
}

struct Collector<'a, 'b> {
    ctx: &'a Context<'b>,
    para: &'a Paragraph,
    text: String,
    spans: Vec<(Range<usize>, usize)>,
    styles: Vec<RunStyle>,
    /// Story characters that are not drawn (field codes, the separator character) as
    /// (byte position in `text`, UTF-16 units); a note mark of n characters counts as one.
    hidden: Vec<(usize, isize)>,
    /// Open fields, innermost last.
    fields: Vec<FieldState>,
    /// Note references: byte position of the mark, endnote or not, id.
    refs: Vec<(usize, bool, i64)>,
    separator: bool,
    /// Drawings: byte position of the object character in `text` and the drawing.
    objects: Vec<(usize, Drawing, crate::float::AnchorFlags)>,
    /// The next `push` starts a new span (after an object character).
    split: bool,
}

#[derive(Default)]
struct FieldState {
    instr: String,
    in_result: bool,
    /// Text Word computes for the field, still to be shown.
    replacement: Option<String>,
    /// The replacement was shown: the cached result is dropped.
    suppress: bool,
}

/// Faces whose ascent Word measures as the full em (see `layout_paragraph`).
fn east_asian_em_box(family: &str) -> bool {
    matches!(
        family,
        "MS Gothic" | "MS PGothic" | "MS UI Gothic" | "MS Mincho" | "MS PMincho"
    )
}

/// Ballot box characters (U+2610 to U+2612). Word sets them in an East Asian font when the run
/// asks for it (`w:rFonts w:hint="eastAsia"`, which the model does not expose) and the run's own
/// font lacks them; the template checkboxes do both.
fn is_ballot(c: char) -> bool {
    ('\u{2610}'..='\u{2612}').contains(&c)
}

/// Family Word's East Asian fallback sets the ballot boxes in. The PDF names MS Mincho; the
/// machine's MS Gothic has the same metrics (ascent 0.859 em, descent 0.141 em).
const BALLOT_FONT: &str = "MS Gothic";

impl Collector<'_, '_> {
    /// Style for ballot boxes of a run in `style`: its own when the font has the glyph.
    fn ballot_style(&mut self, style: usize) -> usize {
        let base = &self.styles[style];
        if self.ctx.fonts.face(base.face).has_glyph('\u{2610}') {
            return style;
        }
        let mut st = base.clone();
        let fs = FontStyle {
            bold: st.bold,
            italic: st.italic,
        };
        st.chain = self.ctx.fonts.chain(BALLOT_FONT, fs);
        st.face = st
            .chain
            .faces
            .first()
            .copied()
            .unwrap_or_else(|| FaceId::from_index(0));
        st.requested = BALLOT_FONT.to_owned();
        self.styles.push(st);
        self.styles.len() - 1
    }

    fn push(&mut self, s: &str, style: usize) {
        if s.chars().any(is_ballot) {
            let mut rest = s;
            while !rest.is_empty() {
                let ballot = rest.chars().next().is_some_and(is_ballot);
                let n = rest
                    .char_indices()
                    .find(|&(_, c)| is_ballot(c) != ballot)
                    .map_or(rest.len(), |(i, _)| i);
                let st = if ballot {
                    self.ballot_style(style)
                } else {
                    style
                };
                self.push_plain(&rest[..n], st);
                rest = &rest[n..];
            }
        } else {
            self.push_plain(s, style);
        }
    }

    fn push_plain(&mut self, s: &str, style: usize) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        let end = self.text.len();
        let split = std::mem::take(&mut self.split);
        match self.spans.last_mut() {
            Some((r, st)) if *st == style && r.end == start && !split => r.end = end,
            _ => self.spans.push((start..end, style)),
        }
    }

    /// A drawing: one object character of its own span.
    fn drawing(&mut self, e: &open365_docx::XmlElement, style: usize) {
        if self.suppressed() {
            return;
        }
        let Some(d) = Drawing::from_element(e) else {
            return;
        };
        let flags = crate::float::AnchorFlags::read(e);
        self.add_drawing(d, flags, style);
    }

    fn add_drawing(&mut self, d: Drawing, flags: crate::float::AnchorFlags, style: usize) {
        if self.suppressed() {
            return;
        }
        self.split = true;
        self.objects.push((self.text.len(), d, flags));
        self.push(OBJECT_CHAR, style);
        self.split = true;
    }

    fn hide(&mut self, units: usize) {
        if units > 0 {
            self.hidden.push((self.text.len(), units as isize));
        }
    }

    /// What Word shows for a field in a header or footer, from its instruction.
    fn computed(&self, instr: &str) -> Option<String> {
        let env = self.ctx.fields;
        let page = env.page?;
        let word = instr.split_whitespace().next()?.to_ascii_uppercase();
        match word.as_str() {
            "PAGE" => Some(page.to_string()),
            "NUMPAGES" => Some(env.num_pages.to_string()),
            "SECTIONPAGES" => Some(env.section_pages.to_string()),
            "SECTION" => Some((env.section + 1).to_string()),
            _ => None,
        }
    }

    fn suppressed(&self) -> bool {
        self.fields.iter().any(|f| f.in_result && f.suppress)
    }

    fn mark(&mut self, text: &str, style_of: &RunStyle, style_idx: usize) {
        // Measured (probes/probe-links): a note reference whose run has no superscript style is
        // plain text in Word, drawn at the run's size on the baseline with the digit's own
        // advance (5.52 pt at 11.04 pt, where the shrunk mark gave 3.55).
        if style_of.vert_align == VertAlign::Baseline {
            self.push(text, style_idx);
            let extra = text.chars().count() as isize - 1;
            if extra > 0 {
                self.hidden.push((self.text.len(), -extra));
            }
            return;
        }
        let mut st = style_of.clone();
        // A reference style that already says `w:vertAlign` has shrunk `size`; start from the full one.
        let full = if st.vert_align == VertAlign::Baseline {
            st.size
        } else {
            st.metric_size
        };
        let half = (full * 2.0 * 0.65).round().max(1.0);
        st.size = half / 2.0;
        st.metric_size = st.size;
        st.case = CaseMode::Normal;
        st.superscript = true;
        st.vert_align = open365_docx::VertAlign::Baseline;
        st.raise = 0.0;
        self.styles.push(st);
        let style = self.styles.len() - 1;
        self.push(text, style);
        let extra = text.chars().count() as isize - 1;
        if extra > 0 {
            self.hidden.push((self.text.len(), -extra));
        }
    }

    fn raw(&mut self, e: &open365_docx::XmlElement, style: usize) {
        let h = &e.head;
        if h.is_w("fldChar") {
            match h.attr_w("fldCharType") {
                Some("begin") => {
                    self.hide(1);
                    self.fields.push(FieldState::default());
                }
                Some("separate") => {
                    self.hide(1);
                    let instr = self
                        .fields
                        .last()
                        .map(|f| f.instr.clone())
                        .unwrap_or_default();
                    let rep = self.computed(&instr);
                    if let Some(f) = self.fields.last_mut() {
                        f.in_result = true;
                        f.replacement = rep;
                    }
                }
                Some("end") => {
                    self.hide(1);
                    if let Some(f) = self.fields.pop()
                        && let Some(rep) = f.replacement
                    {
                        self.push(&rep, style);
                    }
                }
                _ => {}
            }
        } else if h.is_w("instrText") {
            let t: String = e
                .children
                .iter()
                .filter_map(|c| match c {
                    RawXml::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect();
            self.hide(utf16_len(&t));
            if let Some(f) = self.fields.last_mut()
                && !f.in_result
            {
                f.instr.push_str(&t);
            }
        } else if (h.is_w("footnoteReference") || h.is_w("endnoteReference"))
            && let Some(id) = h.attr_w("id").and_then(|v| v.trim().parse::<i64>().ok())
        {
            let end = h.is_w("endnoteReference");
            let label = self.ctx.order.map(|o| o.label(end, id)).unwrap_or_default();
            self.refs.push((self.text.len(), end, id));
            let base = self.styles[style].clone();
            self.mark(&label, &base, style);
        } else if h.is_w("footnoteRef") || h.is_w("endnoteRef") {
            let label = self
                .ctx
                .own_mark
                .map(|(end, n)| mark_text(end, n))
                .unwrap_or_default();
            let base = self.styles[style].clone();
            self.mark(&label, &base, style);
        } else if h.is_w("softHyphen") {
            // An optional hyphen is one character of Word's story offsets (measured on a
            // template whose every later line starts one offset late without it); it draws
            // nothing in the middle of a line.
            if !self.suppressed() {
                self.hide(1);
            }
        } else if h.is_w("noBreakHyphen") {
            if !self.suppressed() {
                self.push("\u{2011}", style);
            }
        } else if h.is_w("separator") || h.is_w("continuationSeparator") {
            self.separator = true;
            self.hide(1);
        } else if let Some(d) = crate::float::alternate_drawing(e) {
            // A text box: the choice holds the `w:drawing`, the fallback a VML copy.
            let flags = crate::float::AnchorFlags::read(e);
            self.add_drawing(d, flags, style);
        }
    }

    fn run(&mut self, run: &Run) {
        let eff = self
            .ctx
            .styles
            .map(|s| s.effective_run_props_in(self.para, run, self.ctx.table));
        let er = fallback_run(eff);
        let lay = self
            .ctx
            .styles
            .map(|s| s.effective_run_layout_in(self.para, run, self.ctx.table))
            .unwrap_or_default();
        let style = {
            let st = style_from(self.ctx, &er, &lay);
            self.styles.push(st);
            self.styles.len() - 1
        };
        for item in &run.content {
            match item {
                RunContent::Text { text, .. } => {
                    if !self.suppressed() {
                        self.show_or_replace(text, style);
                    }
                }
                RunContent::Tab => {
                    if !self.suppressed() {
                        self.push("\t", style);
                    }
                }
                RunContent::Break(b) => {
                    if self.suppressed() {
                        continue;
                    }
                    match b.kind() {
                        open365_docx::BreakKind::TextWrapping => self.push("\u{b}", style),
                        open365_docx::BreakKind::Page => self.push("\u{c}", style),
                        open365_docx::BreakKind::Column => self.push("\u{e}", style),
                    }
                }
                RunContent::Raw(RawXml::Element(e)) if e.head.is_w("drawing") => {
                    self.drawing(e, style);
                }
                RunContent::Raw(RawXml::Element(e)) => self.raw(e, style),
                RunContent::Raw(_) => {}
            }
        }
    }

    /// Shows `text`, or the field value Word computes when the run is the first of a result.
    fn show_or_replace(&mut self, text: &str, style: usize) {
        if let Some(f) = self.fields.iter_mut().rev().find(|f| f.in_result)
            && let Some(rep) = f.replacement.take()
        {
            f.suppress = true;
            self.push(&rep, style);
            return;
        }
        self.push(text, style);
    }

    /// The instruction Word builds for a `w:hyperlink`: ` HYPERLINK "url" \l "anchor" <TAB> "frame"
    /// \o "tip"` (only the length is observable).
    fn hyperlink_code(&self, head: &open365_docx::ElementHead) -> String {
        let mut code = String::from(" HYPERLINK");
        let target = head
            .attrs
            .iter()
            .find(|a| a.local_name() == "id" && a.name.contains(':') && !a.name.starts_with("w:"))
            .and_then(|a| self.ctx.links?.get(&a.value));
        if let Some(url) = target {
            let _ = write!(code, " \"{url}\"");
        }
        if !head
            .attr_w("history")
            .is_some_and(|v| matches!(v, "1" | "true" | "on"))
        {
            code.push_str(" \\h");
        }
        for (attr, switch) in [("anchor", 'l'), ("tgtFrame", 't'), ("tooltip", 'o')] {
            if let Some(v) = head.attr_w(attr) {
                let _ = write!(code, " \\{switch} \"{v}\"");
            }
        }
        code
    }

    fn inlines(&mut self, content: &[Inline]) {
        for inline in content {
            match inline {
                Inline::Run(r) => self.run(r),
                Inline::Group(g) if g.head.is_w("fldSimple") => {
                    let instr = g.head.attr_w("instr").unwrap_or_default().to_owned();
                    self.hide(1 + utf16_len(&instr) + 1);
                    let rep = self.computed(&instr);
                    self.fields.push(FieldState {
                        instr,
                        in_result: true,
                        replacement: rep,
                        suppress: false,
                    });
                    self.inlines(&g.content);
                    if let Some(f) = self.fields.pop()
                        && let Some(rep) = f.replacement
                    {
                        let style = self.styles.len().saturating_sub(1);
                        self.push(&rep, style);
                    }
                    self.hide(1);
                }
                Inline::Group(g) if g.head.is_w("hyperlink") => {
                    // Word reads a hyperlink as a field: begin, code, separator, result, end
                    // (`layout-anc-hyperlink`).
                    self.hide(1 + utf16_len(&self.hyperlink_code(&g.head)) + 1);
                    self.inlines(&g.content);
                    self.hide(1);
                }
                Inline::Group(g) if g.head.is_w("sdt") => {
                    // A content control: its start and end each count as one story character
                    // (`layout-sdt`), the content lays out like plain content.
                    self.hide(1);
                    self.inlines(&g.content);
                    self.hide(1);
                }
                Inline::Group(g) => self.inlines(&g.content),
                Inline::Raw(_) if open365_docx::is_sdt_marker(inline) => self.hide(1),
                Inline::Raw(_) => {}
            }
        }
    }
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// Lay out one paragraph at `text_width` (width of the section's text area).
pub(crate) fn layout_paragraph(
    ctx: &Context<'_>,
    para: &Paragraph,
    index: usize,
    story_start: usize,
    text_width: f64,
    label: Option<&ListLabel>,
) -> ParaLayout {
    let mut props = ctx
        .styles
        .map(|s| s.effective_para_props_in(para, ctx.table))
        .unwrap_or_default();
    if let Some(l) = label {
        // Numbering level indents sit between the style's and the paragraph's own.
        let below = IndentOverride {
            left: Some(f64::from(props.indent_left_pt)),
            right: Some(f64::from(props.indent_right_pt)),
            first_line: Some(f64::from(props.first_line_pt)),
        };
        let merged = para.direct_indent().over(l.indent).over(below);
        props.indent_left_pt = merged.left.unwrap_or_default() as f32;
        props.indent_right_pt = merged.right.unwrap_or_default() as f32;
        props.first_line_pt = merged.first_line.unwrap_or_default() as f32;
    }
    let flow = ctx
        .styles
        .map(|s| s.paragraph_flow_props_in(para, ctx.table))
        .unwrap_or_default();
    let style_id = para.style_id().map(str::to_owned).or_else(|| {
        ctx.styles
            .and_then(|s| s.default_paragraph_style().map(str::to_owned))
    });

    let mut col = Collector {
        ctx,
        para,
        text: String::new(),
        spans: Vec::new(),
        styles: Vec::new(),
        hidden: Vec::new(),
        fields: Vec::new(),
        refs: Vec::new(),
        separator: false,
        objects: Vec::new(),
        split: false,
    };
    col.inlines(&para.content);

    // Paragraph mark style: defaults, the paragraph style's run properties, then the mark's own
    // `w:pPr/w:rPr`.
    let mark = ctx
        .styles
        .map(|s| s.paragraph_mark_props_in(para, ctx.table));
    let mark_lay = mark.as_ref().map(|m| m.layout).unwrap_or_default();
    let mark_er = fallback_run(mark.map(|m| m.run));
    let mark_style = {
        col.styles.push(style_from(ctx, &mark_er, &mark_lay));
        col.styles.len() - 1
    };
    // The label takes the paragraph mark's formatting, then the level's own run properties.
    let label_style = label.map(|l| {
        let mut er = mark_er.clone();
        let r = &l.run;
        if let Some(f) = &r.font {
            er.font.clone_from(f);
        }
        er.size_half_points = r.size_half_points.unwrap_or(er.size_half_points);
        er.bold = r.bold.unwrap_or(er.bold);
        er.italic = r.italic.unwrap_or(er.italic);
        er.underline = r.underline.unwrap_or(er.underline);
        er.strike = r.strike.unwrap_or(er.strike);
        if r.color.is_some() {
            er.color = r.color;
        }
        col.styles.push(style_from(ctx, &er, &mark_lay));
        col.styles.len() - 1
    });
    col.push("\r", mark_style);
    let Collector {
        text,
        spans,
        mut styles,
        hidden,
        refs,
        separator,
        objects: raw_objects,
        ..
    } = col;

    // Shape every span, then group glyphs into atoms by cluster.
    let mut glyphs: Vec<PlacedGlyphRaw> = Vec::new();
    let mut atoms: Vec<Atom> = Vec::new();
    let mut objects: Vec<ObjectBox> = Vec::new();
    let mut reduced: HashMap<usize, usize> = HashMap::new();
    for (range, style_idx) in &spans {
        let seg = &text[range.clone()];
        if seg == OBJECT_CHAR
            && let Some((_, d, flags)) = raw_objects.iter().find(|o| o.0 == range.start)
        {
            let (w, h) = d.size_pt();
            let [el, et, er, eb] = d.effect_extent_pt();
            let inline = !d.anchored;
            objects.push(ObjectBox {
                drawing: d.clone(),
                flags: flags.clone(),
                width: if inline { w + el + er } else { 0.0 },
                height: if inline { h + et + eb } else { 0.0 },
                atom: atoms.len(),
                text_box: d
                    .text_box
                    .as_ref()
                    .map(|tb| crate::textbox::BoxRef::new(ctx, tb, w)),
                page_anchored: !inline
                    && ctx.table.is_some()
                    && ctx.compat < 15
                    && !flags.layout_in_cell,
            });
            atoms.push(Atom {
                byte_start: range.start,
                adv: objects.last().map_or(0.0, |o| o.width),
                style: *style_idx,
                glyphs: glyphs.len()..glyphs.len(),
                class: AtomClass::Object,
                ch: '\u{fffc}',
                object: Some(objects.len() - 1),
            });
            continue;
        }
        for piece in case_pieces(&text, range, *style_idx, &mut styles, &mut reduced) {
            let st = &styles[piece.style];
            let (shaped_text, back) = if piece.upper {
                let (up, back) = capitals(&text[piece.range.clone()]);
                (std::borrow::Cow::Owned(up), Some(back))
            } else {
                (std::borrow::Cow::Borrowed(&text[piece.range.clone()]), None)
            };
            let shaped = shape_run(
                ctx.fonts,
                &st.chain,
                &shaped_text,
                ShapeFeatures {
                    kerning: st.kerning,
                    ligatures: true,
                },
            );
            // Clusters of capitals point back at the characters they were made from.
            let source = |c: u32| back.as_ref().map_or(c, |b| b[c as usize]) as usize;
            let mut idx = 0;
            let gl = &shaped.glyphs;
            while idx < gl.len() {
                let cluster = gl[idx].cluster as usize;
                let mut j = idx + 1;
                while j < gl.len() && gl[j].cluster as usize == cluster {
                    j += 1;
                }
                let byte_start = piece.range.start + source(gl[idx].cluster);
                let ch = text[byte_start..].chars().next().unwrap_or(' ');
                let class = match ch {
                    '\t' => AtomClass::Tab,
                    '\u{b}' => AtomClass::LineBreak,
                    '\u{c}' | '\u{e}' => AtomClass::PageBreak,
                    '\r' => AtomClass::Mark,
                    ' ' => AtomClass::Space,
                    _ => AtomClass::Text,
                };
                let gstart = glyphs.len();
                let mut adv_total = 0.0;
                if matches!(class, AtomClass::Text | AtomClass::Space) {
                    for g in &gl[idx..j] {
                        let upem = f64::from(ctx.fonts.face(g.face).units_per_em());
                        let adv = f64::from(g.x_advance) * st.size / upem * st.scale + st.spacing;
                        glyphs.push(PlacedGlyphRaw {
                            face: g.face,
                            glyph_id: g.glyph_id,
                            x_offset: f64::from(g.x_offset) * st.size / upem * st.scale,
                            y_offset: f64::from(g.y_offset) * st.size / upem + st.raise,
                            adv,
                            cluster: (byte_start - range.start) as u32,
                        });
                        adv_total += adv;
                    }
                }
                atoms.push(Atom {
                    byte_start,
                    adv: adv_total,
                    style: piece.style,
                    glyphs: gstart..glyphs.len(),
                    class,
                    ch,
                    object: None,
                });
                idx = j;
            }
        }
    }

    // Story offsets of every atom start (UTF-16 units): the drawn text plus the characters Word
    // counts but does not draw (field codes). Hidden characters at an atom's own position
    // belong to the line before it, so a line that starts at a field starts at the field code.
    let mut story_of: Vec<usize> = Vec::with_capacity(atoms.len() + 1);
    {
        let mut u = story_start as isize;
        let mut b = 0usize;
        let mut h = 0usize;
        for a in &atoms {
            if a.byte_start > b {
                u += utf16_len(&text[b..a.byte_start]) as isize;
                b = a.byte_start;
            }
            while h < hidden.len() && hidden[h].0 < a.byte_start {
                u += hidden[h].1;
                h += 1;
            }
            story_of.push(u.max(0) as usize);
        }
        u += utf16_len(&text[b..]) as isize;
        u += hidden[h..].iter().map(|x| x.1).sum::<isize>();
        story_of.push(u.max(0) as usize);
    }
    let story_end = *story_of.last().unwrap_or(&story_start);
    let foot_refs: Vec<(usize, i64)> = refs
        .iter()
        .filter(|r| !r.1)
        .filter_map(|r| {
            atoms
                .iter()
                .position(|a| a.byte_start == r.0)
                .map(|i| (i, r.2))
        })
        .collect();

    let breaks = break_opportunities(&text);
    let mut opp: Vec<(usize, BreakKind)> = breaks.iter().map(|b| (b.offset, b.kind)).collect();
    opp.sort_by_key(|o| o.0);

    // Shape the label and find where the text of the first line starts.
    let label_shape = label.zip(label_style).map(|(l, ls)| {
        let st = &styles[ls];
        // Measured (layout-lists-formats): Word does not kern a label (`v.`, `r.`, `o.` keep their
        // unkerned gap), whatever the paragraph's `w:kern`.
        let unkerned = RunStyle {
            kerning: false,
            ..st.clone()
        };
        let (glyphs, width) = shape_glyphs(ctx, &unkerned, &l.text);
        // Measured (layout-lists-suff, `suff="space"`): Word sets the space after a label in
        // Arial at the label's size, whatever the label's font (a 3.12 pt gap at 11 pt where
        // Calibri's space is 2.4 pt).
        let arial = RunStyle {
            chain: ctx.fonts.chain(
                "Arial",
                FontStyle {
                    bold: st.bold,
                    italic: st.italic,
                },
            ),
            ..st.clone()
        };
        let space = shape_glyphs(ctx, &arial, " ").1;
        (l, ls, glyphs, width, space)
    });
    let metrics: Vec<(f64, f64)> = styles
        .iter()
        .map(|st| {
            let face = ctx.fonts.face(st.face);
            let (a, d) = ctx.model.ascent_descent(face.metrics(), st.metric_size);
            // Measured (corpus checkboxes, MS Gothic at 11 pt and MS Mincho at 10 pt): Word's
            // line over the Japanese bitmap faces rises a whole em above the baseline, 1.54 and
            // 1.30 pt higher than their 0.859 em ascent.
            if east_asian_em_box(face.family()) {
                (a + d, d)
            } else {
                (a, d)
            }
        })
        .collect();
    let mut fd = FillData {
        text_width,
        opp,
        story_of,
        first_x: None,
        label_style,
        default_tab: ctx.default_tab,
        model: ctx.model,
        metrics,
    };
    let mut label_layout = None;
    if let Some((l, ls, glyphs, width, space)) = label_shape {
        let at = f64::from(props.indent_left_pt) + f64::from(props.first_line_pt);
        let x = match l.align {
            LabelAlign::Left => at,
            LabelAlign::Center => at - width / 2.0,
            LabelAlign::Right => at - width,
        };
        let end = x + width;
        let first_x = match l.suffix {
            Suffix::Nothing => end,
            Suffix::Space => end + space,
            Suffix::Tab => {
                let lb = LineBuilder {
                    atoms: &atoms,
                    props: &props,
                    flow: &flow,
                    objects: &objects,
                    fd: &fd,
                };
                lb.next_stop(end).pos_twips as f64 / 20.0
            }
        };
        fd.first_x = Some(first_x);
        label_layout = Some(LabelLayout {
            style: ls,
            glyphs,
            text: Arc::from(l.text.as_str()),
            x,
        });
    }
    let mut lines = LineBuilder {
        atoms: &atoms,
        props: &props,
        flow: &flow,
        objects: &objects,
        fd: &fd,
    }
    .build();
    // Measured (layout-sect-empty, layout-sect-space, layout-sect-tail): a paragraph that holds
    // nothing but a section break (continuous, next, even or odd page) takes no height and none of
    // its spacing, and never moves to another page; Word's line of the paragraph before it ends
    // after the break character.
    let section_mark = atoms.len() == 1
        && objects.is_empty()
        && label_layout.is_none()
        && para.props.as_ref().is_some_and(|p| {
            p.children
                .iter()
                .filter_map(RawXml::as_element)
                .any(|e| e.head.is_w("sectPr"))
        });
    if section_mark {
        for l in &mut lines {
            l.height = 0.0;
            l.lead = 0.0;
            l.baseline_off = 0.0;
        }
        props.space_before_pt = 0.0;
        props.space_after_pt = 0.0;
    }
    let role = if label_layout.is_some() {
        ParaRole::ListItem
    } else {
        heading_level(ctx.styles, para.style_id()).map_or(ParaRole::Plain, ParaRole::Heading)
    };

    let border = crate::rules::BorderSpec::from_docx(&flow.borders);
    let leader_glyphs = leader_glyphs(ctx, &flow, &atoms, &styles);
    ParaLayout {
        index,
        props,
        flow,
        style_id,
        label: label_layout,
        lines,
        atoms,
        glyphs,
        styles,
        objects,
        fill: fd,
        story_end,
        foot_refs,
        separator,
        text: Arc::from(text.as_str()),
        role,
        border,
        section_mark,
        leader_glyphs,
    }
}

/// The level of a `heading N` style, looked up through `basedOn`.
fn heading_level(styles: Option<&Styles>, style_id: Option<&str>) -> Option<u8> {
    let styles = styles?;
    let mut id = style_id?.to_owned();
    for _ in 0..8 {
        let info = styles.style(&id)?;
        let name = info.name.to_lowercase();
        if let Some(rest) = name.strip_prefix("heading ")
            && let Ok(n) = rest.trim().parse::<u8>()
            && n >= 1
        {
            return Some(n.min(6));
        }
        id = info.based_on.clone()?;
    }
    None
}

pub(crate) struct LineBuilder<'a> {
    atoms: &'a [Atom],
    props: &'a EffectiveParagraph,
    flow: &'a ParagraphProps,
    objects: &'a [ObjectBox],
    fd: &'a FillData,
}

impl LineBuilder<'_> {
    fn opportunity_at(&self, byte: usize) -> Option<BreakKind> {
        self.fd
            .opp
            .binary_search_by_key(&byte, |o| o.0)
            .ok()
            .map(|i| self.fd.opp[i].1)
    }

    fn build(&self) -> Vec<LineBox> {
        let mut lines = Vec::new();
        let mut a0 = 0usize;
        let n = self.atoms.len();
        while a0 < n {
            let first = lines.is_empty();
            let line = self.fill(a0, first);
            a0 = line.atoms.end;
            lines.push(line);
        }
        if lines.is_empty() {
            // Cannot happen: every paragraph has a mark atom. Keep layout total anyway.
            lines.push(LineBox {
                start: 0,
                end: 0,
                height: 0.0,
                baseline_off: 0.0,
                left: 0.0,
                last_x: 0.0,
                atoms: 0..0,
                lead: 0.0,
                xs: Vec::new(),
                page_break_after: false,
                column_break_after: false,
                slack: 0.0,
                leaders: Vec::new(),
            });
        }
        lines
    }

    /// Next tab stop after `x` (text-area coordinates).
    fn next_stop(&self, x: f64) -> TabStop {
        let mut best: Option<TabStop> = None;
        for t in &self.flow.tabs {
            if tab_pos(t) > x + 0.01 && !matches!(t.kind, TabKind::Bar) {
                best = Some(*t);
                break;
            }
        }
        // A hanging indent makes the left indent an implicit tab stop.
        let ind = self.props.indent_left_pt as f64;
        if self.props.first_line_pt < 0.0
            && ind > x + 0.01
            && best.is_none_or(|b| ind < tab_pos(&b))
        {
            best = Some(TabStop {
                pos_twips: (ind * 20.0).round() as i32,
                kind: TabKind::Left,
                leader: TabLeader::None,
            });
        }
        best.unwrap_or_else(|| {
            let dt = self.fd.default_tab.max(1.0);
            let k = ((x + 0.01) / dt).floor() + 1.0;
            TabStop {
                pos_twips: (k * dt * 20.0).round() as i32,
                kind: TabKind::Left,
                leader: TabLeader::None,
            }
        })
    }

    /// Advance of the tab at atom `i` placed at `x`.
    fn tab_advance(&self, i: usize, x: f64, right_limit: f64) -> f64 {
        let stop = self.next_stop(x);
        let target = match stop.kind {
            TabKind::Left | TabKind::Bar | TabKind::Num => tab_pos(&stop),
            TabKind::Center | TabKind::Right | TabKind::Decimal => {
                // Width of the text that follows, up to the next tab or line end.
                let mut w = 0.0;
                let mut before_dec: Option<f64> = None;
                let mut trailing_space = 0.0;
                let mut j = i + 1;
                while j < self.atoms.len() {
                    let a = &self.atoms[j];
                    if matches!(
                        a.class,
                        AtomClass::Tab
                            | AtomClass::LineBreak
                            | AtomClass::PageBreak
                            | AtomClass::Mark
                    ) {
                        break;
                    }
                    if stop.kind == TabKind::Decimal && a.ch == '.' && before_dec.is_none() {
                        before_dec = Some(w);
                    }
                    w += a.adv;
                    if a.class == AtomClass::Space {
                        trailing_space += a.adv;
                    } else {
                        trailing_space = 0.0;
                    }
                    j += 1;
                }
                w -= trailing_space;
                let start = match stop.kind {
                    TabKind::Center => tab_pos(&stop) - w / 2.0,
                    TabKind::Right => tab_pos(&stop) - w,
                    _ => tab_pos(&stop) - before_dec.unwrap_or(w),
                };
                start.max(x)
            }
        };
        let _ = right_limit;
        (target - x).max(0.0)
    }

    fn fill(&self, a0: usize, first: bool) -> LineBox {
        self.fill_in(a0, first, (0.0, 0.0))
    }

    /// The line starting at atom `a0`, with `inset.0` taken from the left and `inset.1` from
    /// the right of the text area (room a picture occupies).
    pub(crate) fn fill_in(&self, a0: usize, first: bool, inset: (f64, f64)) -> LineBox {
        let n = self.atoms.len();
        let indent_left = f64::from(self.props.indent_left_pt);
        let indent_right = f64::from(self.props.indent_right_pt);
        let first_line = f64::from(self.props.first_line_pt);
        let left_edge = inset.0
            + match (first, self.fd.first_x) {
                (true, Some(x)) => x,
                (true, None) => indent_left + first_line,
                (false, _) => indent_left,
            };
        let right_limit = self.fd.text_width - indent_right - inset.1;

        let mut xs: Vec<f64> = Vec::new();
        let mut advs: Vec<f64> = Vec::new();
        let mut x = left_edge;
        let mut vis_end_x = left_edge;
        let mut i = a0;
        let mut cand: Option<usize> = None; // best break: atom index boundary
        let mut end = n;
        let mut forced = false;
        let mut page_break_after = false;
        let mut column_break_after = false;
        let justify = self.props.alignment == Alignment::Justify;
        let mut space_sum = 0.0;
        let mut tab_leaders: Vec<(usize, TabLeader)> = Vec::new();
        while i < n {
            let a = &self.atoms[i];
            let adv = match a.class {
                AtomClass::Tab => self.tab_advance(i, x, right_limit),
                AtomClass::LineBreak | AtomClass::PageBreak | AtomClass::Mark => 0.0,
                _ => a.adv,
            };
            // A justified line may squeeze its spaces to take one more word.
            let squeeze = if justify {
                space_sum * SPACE_SHRINK
            } else {
                0.0
            };
            if a.class == AtomClass::Space {
                space_sum += a.adv;
            }
            let boxed = a.class == AtomClass::Text || (a.class == AtomClass::Object && adv > 0.0);
            if boxed && i > a0 && x + adv > right_limit + squeeze + EPS {
                end = cand.unwrap_or(i);
                break;
            }
            if a.class == AtomClass::Tab {
                tab_leaders.push((i, self.next_stop(x).leader));
            }
            xs.push(x);
            advs.push(adv);
            x += adv;
            if matches!(
                a.class,
                AtomClass::Text | AtomClass::Tab | AtomClass::Object
            ) {
                vis_end_x = x;
            }
            i += 1;
            match a.class {
                AtomClass::LineBreak | AtomClass::PageBreak | AtomClass::Mark => {
                    if a.class == AtomClass::PageBreak {
                        page_break_after = true;
                        column_break_after = a.ch == '\u{e}';
                        // The paragraph mark right after a page break stays on this line.
                        if i < n && self.atoms[i].class == AtomClass::Mark {
                            xs.push(x);
                            advs.push(0.0);
                            i += 1;
                            page_break_after = true;
                        }
                    }
                    end = i;
                    forced = true;
                    break;
                }
                _ => {}
            }
            // Word breaks before a tab, not after it: the tab goes to the next line together
            // with the text it leads to (`layout-tabs`, "Twelve" + tab + "Thirteen").
            if i < n {
                let before_tab =
                    self.atoms[i].class == AtomClass::Tab && a.class == AtomClass::Text;
                let after_tab = a.class == AtomClass::Tab;
                if before_tab
                    || (!after_tab && self.opportunity_at(self.atoms[i].byte_start).is_some())
                {
                    cand = Some(i);
                }
            }
        }
        if !forced && end == n && i >= n {
            end = n;
        }
        // Truncate positions when we broke at an earlier candidate.
        let count = end - a0;
        xs.truncate(count);
        advs.truncate(count);
        if count == 0 {
            // Always make progress.
            let a = &self.atoms[a0];
            let adv = if a.class == AtomClass::Text || a.class == AtomClass::Space {
                a.adv
            } else {
                0.0
            };
            xs.push(left_edge);
            advs.push(adv);
            end = a0 + 1;
        }

        // Alignment.
        let last_atom = end - 1;
        let mut vis_end = left_edge;
        for (k, a) in self.atoms[a0..end].iter().enumerate() {
            if matches!(
                a.class,
                AtomClass::Text | AtomClass::Tab | AtomClass::Object
            ) {
                vis_end = xs[k] + advs[k];
            }
        }
        let _ = vis_end_x;
        let avail = right_limit - left_edge;
        let vis_w = vis_end - left_edge;
        let ends_paragraph =
            self.atoms[last_atom].class == AtomClass::Mark || (page_break_after && end == n);
        let ends_manual = matches!(
            self.atoms[last_atom].class,
            AtomClass::LineBreak | AtomClass::PageBreak
        );
        match self.props.alignment {
            Alignment::Left => {}
            Alignment::Center => {
                let shift = ((avail - vis_w) / 2.0).max(0.0);
                shift_all(&mut xs, shift);
            }
            Alignment::Right => {
                let shift = (avail - vis_w).max(0.0);
                shift_all(&mut xs, shift);
            }
            Alignment::Justify => {
                if !ends_paragraph && !ends_manual {
                    // Stretch the spaces between the first and last visible atoms.
                    let last_vis = self.atoms[a0..end]
                        .iter()
                        .rposition(|a| a.class != AtomClass::Space)
                        .unwrap_or(0);
                    let spaces: Vec<usize> = (0..last_vis)
                        .filter(|k| self.atoms[a0 + k].class == AtomClass::Space)
                        .collect();
                    if !spaces.is_empty() && (avail - vis_w).abs() > EPS {
                        let slack = avail - vis_w;
                        let mut per = slack / spaces.len() as f64;
                        if slack > 0.0 && spaces.len() >= JUSTIFY_SHORTFALL_MIN_SPACES {
                            per = (per - JUSTIFY_SHORTFALL).max(0.0);
                        }
                        let mut added = 0.0;
                        for k in 0..xs.len() {
                            xs[k] += added;
                            if spaces.binary_search(&k).is_ok() {
                                advs[k] += per;
                                added += per;
                            }
                        }
                    }
                }
            }
        }

        let leaders: Vec<LeaderSpan> = tab_leaders
            .iter()
            .filter(|(i, _)| *i < end)
            .filter_map(|&(i, l)| {
                let ch = leader_char(l)?;
                let k = i - a0;
                Some(LeaderSpan {
                    atom: i,
                    x0: xs[k],
                    x1: xs[k] + advs[k],
                    ch,
                })
            })
            .collect();

        // Height from the fonts on the line.
        let mut asc: f64 = 0.0;
        let mut desc: f64 = 0.0;
        let mut seen: Vec<usize> = Vec::new();
        // The paragraph mark sizes its line only when nothing else is on it (measured with the
        // layout-marks probe: a 28 pt mark after 10 pt text leaves the line at 10 pt).
        let only_mark = self.atoms[a0..end].iter().all(|a| {
            a.class == AtomClass::Mark
                || a.object
                    .and_then(|o| self.objects.get(o))
                    .is_some_and(|o| !o.inline())
        });
        for a in &self.atoms[a0..end] {
            if a.class == AtomClass::Mark && !only_mark {
                continue;
            }
            // A picture has no font: only its height counts (below), and the descent of the
            // line comes from the text (measured: a line holding just a picture and the
            // paragraph mark has no descent).
            if a.class == AtomClass::Object {
                continue;
            }
            if seen.contains(&a.style) {
                continue;
            }
            seen.push(a.style);
            let (ca, cd) = self.fd.metrics[a.style];
            asc = asc.max(ca);
            desc = desc.max(cd);
        }
        // An inline picture stands on the baseline. Its run's font also sizes the line (see
        // `line_height`), so keep the tallest such run.
        let text_asc = asc;
        let text_h = asc + desc;
        let mut pic = 0.0f64;
        let mut run_h = 0.0f64;
        for a in &self.atoms[a0..end] {
            if let Some(o) = a.object.and_then(|o| self.objects.get(o))
                && o.inline()
            {
                pic = pic.max(o.height);
                let (ca, cd) = self.fd.metrics[a.style];
                run_h = run_h.max(ca + cd);
            }
        }
        if first && let Some(ls) = self.fd.label_style {
            // Measured (layout-lists-suff): a 20 pt label raises the first baseline but its
            // descent does not deepen the line.
            let (ca, _) = self.fd.metrics[ls];
            asc = asc.max(ca);
        }
        let (height, baseline_off) = line_height(
            asc,
            desc,
            self.props.line,
            self.fd.model,
            PictureLine {
                height: pic,
                run_h,
                text_asc,
                text_h,
            },
        );

        let last_x = xs.last().copied().unwrap_or(left_edge);
        let slack = match self.props.line {
            open365_docx::LineSpacing::Multiple(m) if m > 1.0 => {
                (height - baseline_off - desc).max(0.0)
            }
            _ => 0.0,
        };
        LineBox {
            start: self.fd.story_of[a0],
            end: self.fd.story_of[end],
            height,
            baseline_off,
            left: xs.first().copied().unwrap_or(left_edge),
            last_x,
            atoms: a0..end,
            lead: 0.0,
            xs,
            page_break_after,
            column_break_after,
            slack,
            leaders,
        }
    }
}

/// Characters Word repeats across the gap of a tab with a leader (`w:leader`).
fn leader_char(l: TabLeader) -> Option<char> {
    match l {
        TabLeader::None => None,
        TabLeader::Dot => Some('.'),
        TabLeader::Hyphen => Some('-'),
        TabLeader::Underscore | TabLeader::Heavy => Some('_'),
        TabLeader::MiddleDot => Some('\u{b7}'),
    }
}

/// Shapes the leader characters of `flow`'s tab stops for every style a tab of the paragraph
/// has. Word draws a leader as glyphs of the tab's own font, one every whole-pixel advance of
/// the character at 300 dpi (0.24 pt pixels): '.' every 2.88 pt, '-' every 3.36 pt at 11 pt
/// Calibri (`layout-tabs`).
fn leader_glyphs(
    ctx: &Context<'_>,
    flow: &ParagraphProps,
    atoms: &[Atom],
    styles: &[RunStyle],
) -> Vec<LeaderGlyph> {
    let mut chars: Vec<char> = flow
        .tabs
        .iter()
        .filter_map(|t| leader_char(t.leader))
        .collect();
    chars.sort_unstable();
    chars.dedup();
    let mut out = Vec::new();
    if chars.is_empty() {
        return out;
    }
    let mut seen: Vec<usize> = Vec::new();
    for a in atoms.iter().filter(|a| a.class == AtomClass::Tab) {
        if seen.contains(&a.style) {
            continue;
        }
        seen.push(a.style);
        let st = &styles[a.style];
        for &ch in &chars {
            let (glyphs, width) = shape_glyphs(ctx, st, &ch.to_string());
            let Some(g) = glyphs.first() else { continue };
            let pitch = (width / 0.24).round() * 0.24;
            if pitch > 0.0 {
                out.push(LeaderGlyph {
                    style: a.style,
                    ch,
                    face: g.face,
                    glyph_id: g.glyph_id,
                    pitch,
                });
            }
        }
    }
    out
}

fn shift_all(xs: &mut [f64], by: f64) {
    for x in xs {
        *x += by;
    }
}

/// Tab position in points, from twips without the `f32` rounding of `TabStop::pos_pt`.
fn tab_pos(t: &TabStop) -> f64 {
    f64::from(t.pos_twips) / 20.0
}

/// Shapes `text` in `st` and returns the glyphs with their advances and the total width.
fn shape_glyphs(ctx: &Context<'_>, st: &RunStyle, text: &str) -> (Vec<PlacedGlyphRaw>, f64) {
    let shaped = shape_run(
        ctx.fonts,
        &st.chain,
        text,
        ShapeFeatures {
            kerning: st.kerning,
            ligatures: true,
        },
    );
    let mut width = 0.0;
    let glyphs = shaped
        .glyphs
        .iter()
        .map(|g| {
            let upem = f64::from(ctx.fonts.face(g.face).units_per_em());
            let adv = f64::from(g.x_advance) * st.size / upem * st.scale + st.spacing;
            width += adv;
            PlacedGlyphRaw {
                face: g.face,
                glyph_id: g.glyph_id,
                x_offset: f64::from(g.x_offset) * st.size / upem * st.scale,
                y_offset: f64::from(g.y_offset) * st.size / upem + st.raise,
                adv,
                cluster: g.cluster,
            }
        })
        .collect();
    (glyphs, width)
}
