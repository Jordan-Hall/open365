//! Typed, read-only cell formatting: font, fill, borders and alignment resolved through
//! `cellXfs` and `cellStyleXfs`, plus the colour model (RGB, theme with tint, palette index).
//!
//! Nothing here writes: the tables are read from the preserved `styles.xml` tree and the
//! theme part, so the bytes of an unedited workbook do not change.
//!
//! Inheritance: a `cellXfs/xf` names its font, fill and border by index. When the attribute
//! is absent the cell style (`cellStyleXfs/xf[xfId]`) supplies it; when it is present the
//! cell format's own index is used whatever `applyFont`/`applyFill`/`applyBorder` say (Excel
//! does the same: those flags only steer style propagation when a user edits a style). The
//! `alignment` child follows the same rule: the cell format's own, else the style's.

use crate::xml::XmlElement;

/// An opaque colour with 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Self = Self::new(0, 0, 0);
    pub const WHITE: Self = Self::new(255, 255, 255);

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `0xRRGGBB`.
    pub const fn rgb_u32(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// The integer Excel's COM `Color` properties use: `0x00BBGGRR`.
    pub const fn bgr_u32(self) -> u32 {
        ((self.b as u32) << 16) | ((self.g as u32) << 8) | self.r as u32
    }

    pub(crate) fn from_hex(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = if s.len() == 8 { &s[2..] } else { s };
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Self::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }
}

/// Where a colour comes from before the theme, palette and tint are applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorSource {
    /// Automatic: the system text or window colour (black for text and borders).
    Auto,
    Rgb(Rgb),
    /// Index into the theme: 0 = lt1, 1 = dk1, 2 = lt2, 3 = dk2, 4 to 9 = accent1 to 6,
    /// 10 = hyperlink, 11 = followed hyperlink.
    Theme(u32),
    /// Index into the legacy palette (0 to 63; 64 and 65 are the system colours).
    Indexed(u32),
}

/// A colour element: a source and a tint in `-1.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub source: ColorSource,
    pub tint: f64,
}

impl Color {
    pub const AUTO: Self = Self {
        source: ColorSource::Auto,
        tint: 0.0,
    };

    pub(crate) fn parse(e: &XmlElement) -> Self {
        let h = &e.head;
        let tint = h
            .attr("tint")
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0);
        let source = if let Some(rgb) = h.attr("rgb").and_then(Rgb::from_hex) {
            ColorSource::Rgb(rgb)
        } else if let Some(t) = h.attr("theme").and_then(|v| v.trim().parse().ok()) {
            ColorSource::Theme(t)
        } else if let Some(i) = h.attr("indexed").and_then(|v| v.trim().parse().ok()) {
            ColorSource::Indexed(i)
        } else {
            ColorSource::Auto
        };
        Self { source, tint }
    }
}

/// The twelve theme colours in theme index order (see [`ColorSource::Theme`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    colors: [Rgb; 12],
}

impl Default for Theme {
    /// The Office 2013 to 2022 theme, used when a workbook has no theme part.
    fn default() -> Self {
        let c = |v: u32| Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8);
        Self {
            colors: [
                c(0xFFFFFF),
                c(0x000000),
                c(0xE7E6E6),
                c(0x44546A),
                c(0x4472C4),
                c(0xED7D31),
                c(0xA5A5A5),
                c(0xFFC000),
                c(0x5B9BD5),
                c(0x70AD47),
                c(0x0563C1),
                c(0x954F72),
            ],
        }
    }
}

impl Theme {
    /// Reads `a:clrScheme` from a theme part; missing entries keep the default theme's.
    pub(crate) fn parse(bytes: &[u8]) -> Self {
        let mut theme = Self::default();
        let Ok(doc) = crate::xml::parse(bytes) else {
            return theme;
        };
        let scheme = find_descendant(&doc.root, "clrScheme");
        let Some(scheme) = scheme else {
            return theme;
        };
        // File order: dk1 lt1 dk2 lt2 accent1..6 hlink folHlink; theme index order swaps the
        // first two pairs.
        const NAMES: [(&str, usize); 12] = [
            ("lt1", 0),
            ("dk1", 1),
            ("lt2", 2),
            ("dk2", 3),
            ("accent1", 4),
            ("accent2", 5),
            ("accent3", 6),
            ("accent4", 7),
            ("accent5", 8),
            ("accent6", 9),
            ("hlink", 10),
            ("folHlink", 11),
        ];
        for el in scheme.elements() {
            let Some(&(_, slot)) = NAMES.iter().find(|(n, _)| *n == el.head.local_name()) else {
                continue;
            };
            let Some(color) = el.elements().next() else {
                continue;
            };
            let hex = match color.head.local_name() {
                "srgbClr" => color.head.attr("val"),
                "sysClr" => color.head.attr("lastClr"),
                _ => None,
            };
            if let Some(rgb) = hex.and_then(Rgb::from_hex) {
                theme.colors[slot] = rgb;
            }
        }
        theme
    }

    /// The colour at a theme index, `None` past the twelve.
    pub fn color(&self, index: u32) -> Option<Rgb> {
        self.colors.get(usize::try_from(index).ok()?).copied()
    }
}

fn find_descendant<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    if e.head.local_name() == local {
        return Some(e);
    }
    e.elements().find_map(|c| find_descendant(c, local))
}

/// Excel's default 64-entry palette plus the two system colours (indexes 64 and 65).
const DEFAULT_PALETTE: [u32; 64] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000,
    0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000,
    0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC,
    0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
    0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF,
    0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600,
    0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399,
    0x333333,
];

/// Colour of palette `index`, with `overrides` (the style sheet's `indexedColors`) first.
pub(crate) fn palette_color(overrides: &[Rgb], index: u32) -> Rgb {
    let i = usize::try_from(index).unwrap_or(usize::MAX);
    if let Some(c) = overrides.get(i) {
        return *c;
    }
    match index {
        64 => Rgb::BLACK,
        65 => Rgb::WHITE,
        _ => DEFAULT_PALETTE.get(i).map_or(Rgb::BLACK, |v| {
            Rgb::new((v >> 16) as u8, (v >> 8) as u8, *v as u8)
        }),
    }
}

const HLS_MAX: i64 = 240;
const RGB_MAX: i64 = 255;

/// Excel's tint. Excel converts to HLS with the Windows integer scale (hue, luminance and
/// saturation 0 to 240), moves the luminance towards black (`tint < 0`, luminance times
/// `1 + tint`, truncated) or white (`tint > 0`, luminance times `1 - tint` truncated plus
/// `240 - round(240 * (1 - tint))`), and converts back with the same integer rounding.
/// Checked against Excel for every theme colour and tint in `fixtures/xlsx/grid-styles.xlsx`.
pub fn apply_tint(color: Rgb, tint: f64) -> Rgb {
    if tint == 0.0 || !tint.is_finite() {
        return color;
    }
    let (h, l, s) = rgb_to_hls(color);
    let lf = l as f64;
    let max = HLS_MAX as f64;
    let new_l = if tint < 0.0 {
        (lf * (1.0 + tint)).floor() as i64
    } else {
        (lf * (1.0 - tint)).floor() as i64 + HLS_MAX - (max * (1.0 - tint) + 0.5).floor() as i64
    };
    hls_to_rgb(h, new_l.clamp(0, HLS_MAX), s)
}

fn rgb_to_hls(c: Rgb) -> (i64, i64, i64) {
    let (r, g, b) = (i64::from(c.r), i64::from(c.g), i64::from(c.b));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let sum = max + min;
    let l = (sum * HLS_MAX + RGB_MAX) / (2 * RGB_MAX);
    if max == min {
        return (HLS_MAX * 2 / 3, l, 0);
    }
    let d = max - min;
    let s = if l <= HLS_MAX / 2 {
        (d * HLS_MAX + sum / 2) / sum
    } else {
        (d * HLS_MAX + (2 * RGB_MAX - sum) / 2) / (2 * RGB_MAX - sum)
    };
    let delta = |v: i64| ((max - v) * (HLS_MAX / 6) + d / 2) / d;
    let (rd, gd, bd) = (delta(r), delta(g), delta(b));
    let mut h = if r == max {
        bd - gd
    } else if g == max {
        HLS_MAX / 3 + rd - bd
    } else {
        2 * HLS_MAX / 3 + gd - rd
    };
    if h < 0 {
        h += HLS_MAX;
    }
    if h > HLS_MAX {
        h -= HLS_MAX;
    }
    (h, l, s)
}

fn hls_to_rgb(h: i64, l: i64, s: i64) -> Rgb {
    let byte = |v: i64| v.clamp(0, RGB_MAX) as u8;
    if s == 0 {
        let v = byte((l * RGB_MAX + HLS_MAX / 2) / HLS_MAX);
        return Rgb::new(v, v, v);
    }
    let m2 = if l <= HLS_MAX / 2 {
        (l * (HLS_MAX + s) + HLS_MAX / 2) / HLS_MAX
    } else {
        l + s - (l * s + HLS_MAX / 2) / HLS_MAX
    };
    let m1 = 2 * l - m2;
    let channel = |mut hue: i64| {
        if hue < 0 {
            hue += HLS_MAX;
        }
        if hue > HLS_MAX {
            hue -= HLS_MAX;
        }
        let v = if hue < HLS_MAX / 6 {
            m1 + ((m2 - m1) * hue + HLS_MAX / 12) / (HLS_MAX / 6)
        } else if hue < HLS_MAX / 2 {
            m2
        } else if hue < HLS_MAX * 2 / 3 {
            m1 + ((m2 - m1) * (HLS_MAX * 2 / 3 - hue) + HLS_MAX / 12) / (HLS_MAX / 6)
        } else {
            m1
        };
        byte((v * RGB_MAX + HLS_MAX / 2) / HLS_MAX)
    };
    Rgb::new(
        channel(h + HLS_MAX / 3),
        channel(h),
        channel(h - HLS_MAX / 3),
    )
}

/// Resolves a colour against the theme and palette. Automatic is black.
pub(crate) fn resolve(color: &Color, theme: &Theme, palette: &[Rgb]) -> Rgb {
    let base = match color.source {
        ColorSource::Auto => return Rgb::BLACK,
        ColorSource::Rgb(c) => c,
        ColorSource::Theme(i) => theme.color(i).unwrap_or(Rgb::BLACK),
        ColorSource::Indexed(i) => palette_color(palette, i),
    };
    apply_tint(base, color.tint)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    SingleAccounting,
    DoubleAccounting,
}

/// A `font` record.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub name: String,
    /// Size in points.
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
    pub color: Color,
}

impl Default for Font {
    fn default() -> Self {
        Self {
            name: "Calibri".to_owned(),
            size: 11.0,
            bold: false,
            italic: false,
            underline: Underline::None,
            strike: false,
            color: Color::AUTO,
        }
    }
}

fn bool_child(e: &XmlElement, local: &str) -> bool {
    e.child_main(local)
        .is_some_and(|c| !matches!(c.head.attr("val").map(str::trim), Some("0" | "false")))
}

impl Font {
    fn parse(e: &XmlElement) -> Self {
        let mut f = Self::default();
        if let Some(v) = e.child_main("name").and_then(|c| c.head.attr("val")) {
            f.name = v.to_owned();
        }
        if let Some(v) = e
            .child_main("sz")
            .and_then(|c| c.head.attr("val"))
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
        {
            f.size = v;
        }
        f.bold = bool_child(e, "b");
        f.italic = bool_child(e, "i");
        f.strike = bool_child(e, "strike");
        f.underline = match e.child_main("u") {
            None => Underline::None,
            Some(u) => match u.head.attr("val") {
                None | Some("single") => Underline::Single,
                Some("double") => Underline::Double,
                Some("singleAccounting") => Underline::SingleAccounting,
                Some("doubleAccounting") => Underline::DoubleAccounting,
                _ => Underline::None,
            },
        };
        f.color = e.child_main("color").map_or(Color::AUTO, Color::parse);
        f
    }
}

/// The `patternType` of a fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillPattern {
    #[default]
    None,
    Solid,
    MediumGray,
    DarkGray,
    LightGray,
    DarkHorizontal,
    DarkVertical,
    DarkDown,
    DarkUp,
    DarkGrid,
    DarkTrellis,
    LightHorizontal,
    LightVertical,
    LightDown,
    LightUp,
    LightGrid,
    LightTrellis,
    Gray125,
    Gray0625,
}

impl FillPattern {
    pub(crate) fn parse(v: &str) -> Self {
        match v {
            "solid" => Self::Solid,
            "mediumGray" => Self::MediumGray,
            "darkGray" => Self::DarkGray,
            "lightGray" => Self::LightGray,
            "darkHorizontal" => Self::DarkHorizontal,
            "darkVertical" => Self::DarkVertical,
            "darkDown" => Self::DarkDown,
            "darkUp" => Self::DarkUp,
            "darkGrid" => Self::DarkGrid,
            "darkTrellis" => Self::DarkTrellis,
            "lightHorizontal" => Self::LightHorizontal,
            "lightVertical" => Self::LightVertical,
            "lightDown" => Self::LightDown,
            "lightUp" => Self::LightUp,
            "lightGrid" => Self::LightGrid,
            "lightTrellis" => Self::LightTrellis,
            "gray125" => Self::Gray125,
            "gray0625" => Self::Gray0625,
            _ => Self::None,
        }
    }
}

/// A `fill` record. `fg` is the pattern colour (the colour of a solid fill); `bg` the
/// background behind a pattern. Gradient fills read as [`FillPattern::None`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    pub pattern: FillPattern,
    pub fg: Color,
    pub bg: Color,
    pub gradient: bool,
}

impl Default for Fill {
    fn default() -> Self {
        Self {
            pattern: FillPattern::None,
            fg: Color {
                source: ColorSource::Indexed(64),
                tint: 0.0,
            },
            bg: Color {
                source: ColorSource::Indexed(65),
                tint: 0.0,
            },
            gradient: false,
        }
    }
}

impl Fill {
    fn parse(e: &XmlElement) -> Self {
        let mut f = Self::default();
        if let Some(p) = e.child_main("patternFill") {
            f.pattern = p
                .head
                .attr("patternType")
                .map_or(FillPattern::None, FillPattern::parse);
            if let Some(c) = p.child_main("fgColor") {
                f.fg = Color::parse(c);
            }
            if let Some(c) = p.child_main("bgColor") {
                f.bg = Color::parse(c);
            }
        } else if e.child_main("gradientFill").is_some() {
            f.gradient = true;
        }
        f
    }
}

/// A border line style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Thin,
    Medium,
    Dashed,
    Dotted,
    Thick,
    Double,
    Hair,
    MediumDashed,
    DashDot,
    MediumDashDot,
    DashDotDot,
    MediumDashDotDot,
    SlantDashDot,
}

impl BorderStyle {
    fn parse(v: &str) -> Self {
        match v {
            "thin" => Self::Thin,
            "medium" => Self::Medium,
            "dashed" => Self::Dashed,
            "dotted" => Self::Dotted,
            "thick" => Self::Thick,
            "double" => Self::Double,
            "hair" => Self::Hair,
            "mediumDashed" => Self::MediumDashed,
            "dashDot" => Self::DashDot,
            "mediumDashDot" => Self::MediumDashDot,
            "dashDotDot" => Self::DashDotDot,
            "mediumDashDotDot" => Self::MediumDashDotDot,
            "slantDashDot" => Self::SlantDashDot,
            _ => Self::None,
        }
    }

    /// Drawing weight used to settle which of two borders on the same edge wins: heavier
    /// lines win over lighter ones.
    pub fn rank(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Hair => 1,
            Self::Dotted => 2,
            Self::DashDotDot | Self::DashDot | Self::Dashed => 3,
            Self::Thin => 4,
            Self::MediumDashDotDot | Self::SlantDashDot | Self::MediumDashDot => 5,
            Self::MediumDashed => 6,
            Self::Medium => 7,
            Self::Double => 8,
            Self::Thick => 9,
        }
    }

    /// Line width in pixels at 100% zoom (1 for the thin family, 2 medium, 3 thick, 3 for a
    /// double line: two lines and the gap).
    pub fn width_px(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Thin
            | Self::Dashed
            | Self::Dotted
            | Self::Hair
            | Self::DashDot
            | Self::DashDotDot => 1,
            Self::Medium
            | Self::MediumDashed
            | Self::MediumDashDot
            | Self::MediumDashDotDot
            | Self::SlantDashDot => 2,
            Self::Thick => 3,
            Self::Double => 3,
        }
    }
}

/// One edge of a border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BorderEdge {
    pub style: BorderStyle,
    pub color: Color,
}

impl Default for BorderEdge {
    fn default() -> Self {
        Self {
            style: BorderStyle::None,
            color: Color::AUTO,
        }
    }
}

impl BorderEdge {
    pub(crate) fn parse(e: &XmlElement) -> Self {
        Self {
            style: e
                .head
                .attr("style")
                .map_or(BorderStyle::None, BorderStyle::parse),
            color: e.child_main("color").map_or(Color::AUTO, Color::parse),
        }
    }
}

/// The edge drawn (and reported by Excel) where two cells meet. `near` belongs to the left or
/// upper cell, `far` to the right or lower one. Excel keeps both cells' copies of a shared edge
/// in step, so files it wrote never disagree; for files that do, the heavier line wins and on a
/// tie the later cell's (the right or lower one's) edge.
pub fn shared_edge(near: BorderEdge, far: BorderEdge) -> BorderEdge {
    if near.style == BorderStyle::None {
        far
    } else if far.style == BorderStyle::None || near.style.rank() > far.style.rank() {
        near
    } else {
        far
    }
}

/// A `border` record. `start` and `end` read as left and right.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Border {
    pub left: BorderEdge,
    pub right: BorderEdge,
    pub top: BorderEdge,
    pub bottom: BorderEdge,
    pub diagonal: BorderEdge,
}

impl Border {
    fn parse(e: &XmlElement) -> Self {
        let mut b = Self::default();
        for c in e.elements() {
            let edge = BorderEdge::parse(c);
            match c.head.local_name() {
                "left" | "start" => b.left = edge,
                "right" | "end" => b.right = edge,
                "top" => b.top = edge,
                "bottom" => b.bottom = edge,
                "diagonal" => b.diagonal = edge,
                _ => {}
            }
        }
        b
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HorizontalAlignment {
    #[default]
    General,
    Left,
    Center,
    Right,
    Fill,
    Justify,
    CenterContinuous,
    Distributed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlignment {
    Top,
    Center,
    #[default]
    Bottom,
    Justify,
    Distributed,
}

/// An `alignment` element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Alignment {
    pub horizontal: HorizontalAlignment,
    pub vertical: VerticalAlignment,
    pub wrap_text: bool,
    /// Indent in steps of three spaces' width.
    pub indent: u32,
    /// `textRotation` as stored: 0 to 90 counter-clockwise degrees, 91 to 180 clockwise
    /// (90 plus the angle), 255 stacked vertical text.
    pub text_rotation: u32,
    pub shrink_to_fit: bool,
}

impl Alignment {
    fn parse(e: &XmlElement) -> Self {
        let h = &e.head;
        let num = |n: &str| h.attr(n).and_then(|v| v.trim().parse::<u32>().ok());
        let flag = |n: &str| matches!(h.attr(n).map(str::trim), Some("1" | "true"));
        Self {
            horizontal: match h.attr("horizontal") {
                Some("left") => HorizontalAlignment::Left,
                Some("center") => HorizontalAlignment::Center,
                Some("right") => HorizontalAlignment::Right,
                Some("fill") => HorizontalAlignment::Fill,
                Some("justify") => HorizontalAlignment::Justify,
                Some("centerContinuous") => HorizontalAlignment::CenterContinuous,
                Some("distributed") => HorizontalAlignment::Distributed,
                _ => HorizontalAlignment::General,
            },
            vertical: match h.attr("vertical") {
                Some("top") => VerticalAlignment::Top,
                Some("center") => VerticalAlignment::Center,
                Some("justify") => VerticalAlignment::Justify,
                Some("distributed") => VerticalAlignment::Distributed,
                _ => VerticalAlignment::Bottom,
            },
            wrap_text: flag("wrapText"),
            indent: num("indent").unwrap_or(0).min(250),
            text_rotation: num("textRotation").unwrap_or(0),
            shrink_to_fit: flag("shrinkToFit"),
        }
    }

    /// Rotation as Excel's `Orientation`: degrees from -90 (down) to 90 (up), `None` for
    /// stacked vertical text.
    pub fn rotation_degrees(&self) -> Option<i32> {
        match self.text_rotation {
            r @ 0..=90 => Some(r as i32),
            r @ 91..=180 => Some(90 - r as i32),
            _ => None,
        }
    }
}

/// Everything a cell format (`cellXfs/xf`) says about how a cell looks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellFormat {
    pub num_fmt_id: u32,
    /// `xfId`: the cell style this format is based on.
    pub style_xf: u32,
    pub font: Font,
    pub fill: Fill,
    pub border: Border,
    pub alignment: Alignment,
}

/// The font, fill, border and format tables of a style sheet.
#[derive(Debug, Clone, Default)]
pub(crate) struct FormatTables {
    pub formats: Vec<CellFormat>,
    pub palette: Vec<Rgb>,
    default_format: CellFormat,
}

impl FormatTables {
    pub fn parse(root: &XmlElement) -> Self {
        let list = |name: &str, item: &str| -> Vec<XmlElement> {
            root.child_main(name)
                .map(|e| {
                    e.elements()
                        .filter(|c| c.head.is_main(item))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        let fonts: Vec<Font> = list("fonts", "font").iter().map(Font::parse).collect();
        let fills: Vec<Fill> = list("fills", "fill").iter().map(Fill::parse).collect();
        let borders: Vec<Border> = list("borders", "border")
            .iter()
            .map(Border::parse)
            .collect();
        let palette: Vec<Rgb> = root
            .child_main("colors")
            .and_then(|c| c.child_main("indexedColors"))
            .map(|c| {
                c.elements()
                    .map(|e| {
                        e.head
                            .attr("rgb")
                            .and_then(Rgb::from_hex)
                            .unwrap_or(Rgb::BLACK)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let style_xfs = list("cellStyleXfs", "xf");
        let cell_xfs = list("cellXfs", "xf");

        let id = |x: &XmlElement, name: &str| -> Option<usize> {
            x.head.attr(name)?.trim().parse().ok()
        };
        let align_of = |x: &XmlElement| x.child_main("alignment").map(Alignment::parse);
        let formats = cell_xfs
            .iter()
            .map(|xf| {
                let style_xf = id(xf, "xfId").unwrap_or(0);
                let style = style_xfs.get(style_xf);
                let pick = |name: &str| id(xf, name).or_else(|| style.and_then(|s| id(s, name)));
                let font = pick("fontId")
                    .and_then(|i| fonts.get(i))
                    .or_else(|| fonts.first())
                    .cloned()
                    .unwrap_or_default();
                let fill = pick("fillId")
                    .and_then(|i| fills.get(i))
                    .or_else(|| fills.first())
                    .copied()
                    .unwrap_or_default();
                let border = pick("borderId")
                    .and_then(|i| borders.get(i))
                    .or_else(|| borders.first())
                    .copied()
                    .unwrap_or_default();
                let alignment = align_of(xf)
                    .or_else(|| style.and_then(align_of))
                    .unwrap_or_default();
                CellFormat {
                    num_fmt_id: pick("numFmtId").map_or(0, |v| v as u32),
                    style_xf: style_xf as u32,
                    font,
                    fill,
                    border,
                    alignment,
                }
            })
            .collect();
        let default_format = CellFormat {
            font: fonts.first().cloned().unwrap_or_default(),
            ..CellFormat::default()
        };
        Self {
            formats,
            palette,
            default_format,
        }
    }

    pub fn format(&self, xf: u32) -> &CellFormat {
        usize::try_from(xf)
            .ok()
            .and_then(|i| self.formats.get(i))
            .unwrap_or(&self.default_format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_follows_excel_for_known_values() {
        // From the Excel oracle (fixtures/xlsx/grid-styles.xlsx).
        assert_eq!(
            apply_tint(Rgb::BLACK, 0.0999786370433668),
            Rgb::new(26, 26, 26)
        );
        assert_eq!(
            apply_tint(Rgb::new(0xE8, 0xE8, 0xE8), -0.8999908444471572),
            Rgb::new(22, 22, 22)
        );
        assert_eq!(
            apply_tint(Rgb::new(0x4E, 0xA7, 0x2E), 0.0999786370433668),
            Rgb::new(90, 190, 52)
        );
        assert_eq!(apply_tint(Rgb::new(1, 2, 3), 0.0), Rgb::new(1, 2, 3));
    }

    #[test]
    fn default_palette_has_the_documented_colours() {
        assert_eq!(palette_color(&[], 10), Rgb::new(255, 0, 0));
        assert_eq!(palette_color(&[], 64), Rgb::BLACK);
        assert_eq!(palette_color(&[], 65), Rgb::WHITE);
        assert_eq!(palette_color(&[Rgb::new(1, 2, 3)], 0), Rgb::new(1, 2, 3));
    }

    #[test]
    fn rotation_degrees_follow_the_stored_encoding() {
        let a = |r| Alignment {
            text_rotation: r,
            ..Alignment::default()
        };
        assert_eq!(a(45).rotation_degrees(), Some(45));
        assert_eq!(a(135).rotation_degrees(), Some(-45));
        assert_eq!(a(180).rotation_degrees(), Some(-90));
        assert_eq!(a(255).rotation_degrees(), None);
    }
}
