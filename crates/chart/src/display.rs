//! A backend-neutral chart display list: rectangles, paths and text runs in points.
//!
//! Coordinates are points with the origin at the top left of the chart and y growing downward.
//! The list is drawn in order (painter's algorithm). A backend (egui painter, PDF content
//! stream, SVG) maps each item one to one; nothing here depends on a graphics API.

/// An sRGB colour with alpha, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const WHITE: Self = Self(255, 255, 255, 255);
    pub const BLACK: Self = Self(0, 0, 0, 255);

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(r, g, b, 255)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn center_x(&self) -> f64 {
        self.x + self.width / 2.0
    }

    pub fn center_y(&self) -> f64 {
        self.y + self.height / 2.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub color: Rgba,
    pub width: f64,
    /// Dash lengths in points (on, off, ...); empty is solid.
    pub dash: Vec<f64>,
}

impl Stroke {
    pub fn solid(color: Rgba, width: f64) -> Self {
        Self {
            color,
            width,
            dash: Vec::new(),
        }
    }
}

/// One path segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCmd {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    /// Cubic Bezier: two control points, then the end point.
    CubicTo(f64, f64, f64, f64, f64, f64),
    Close,
}

/// A font request: the display list names the font, the backend resolves it.
#[derive(Debug, Clone, PartialEq)]
pub struct FontSpec {
    pub family: String,
    pub size_pt: f64,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAnchor {
    Start,
    Middle,
    End,
}

/// A single line of text. `x` is the anchor point on the baseline's horizontal axis, `y` the
/// top of the line box; with `rotation_deg` the run turns counter-clockwise about `(x, y_mid)`
/// where `y_mid` is the vertical centre of the line box, and `width` stays the unrotated
/// advance.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub x: f64,
    pub y: f64,
    /// Measured advance width in points.
    pub width: f64,
    /// Line box height in points.
    pub height: f64,
    pub anchor: TextAnchor,
    pub font: FontSpec,
    pub color: Rgba,
    pub rotation_deg: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    Rect {
        rect: Rect,
        fill: Option<Rgba>,
        stroke: Option<Stroke>,
    },
    Path {
        cmds: Vec<PathCmd>,
        fill: Option<Rgba>,
        stroke: Option<Stroke>,
    },
    /// Clip subsequent items to a rectangle until the matching [`DisplayItem::PopClip`].
    PushClip(Rect),
    PopClip,
    Text(TextRun),
}

/// A data label as drawn: its text and the box Excel reports for it (`DataLabel.Left` and
/// friends: the text plus a 3 pt margin on every side).
#[derive(Debug, Clone, PartialEq)]
pub struct LabelBox {
    /// Series, counted across all chart groups.
    pub series: usize,
    pub point: usize,
    pub text: String,
    pub rect: Rect,
}

/// A major tick label of a date axis.
#[derive(Debug, Clone, PartialEq)]
pub struct DateTick {
    pub serial: f64,
    pub text: String,
    /// Centre of the label along the axis, in chart-area points.
    pub x: f64,
}

/// A rendered chart.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayList {
    /// Size of the chart object in points.
    pub width: f64,
    pub height: f64,
    pub items: Vec<DisplayItem>,
    /// Plot area without tick labels (`PlotArea.InsideLeft` and friends), in chart-area
    /// coordinates (see [`crate::layout`]).
    pub plot_inside: Rect,
    /// Plot area including tick labels (`PlotArea.Left` and friends).
    pub plot_outer: Rect,
    pub legend: Option<Rect>,
    pub title: Option<Rect>,
    /// The resolved value axis scale of a cartesian chart (the Y axis of scatter and bubble).
    pub value_scale: Option<crate::axis::AxisScale>,
    /// The resolved X axis scale of a scatter or bubble chart.
    pub x_scale: Option<crate::axis::AxisScale>,
    /// The automatic scale of a date category axis (`c:dateAx`) over the first series'
    /// numeric categories; the points are still laid out as evenly spaced categories.
    pub date_scale: Option<crate::axis::DateScale>,
    /// Data labels in drawing order.
    pub data_labels: Vec<LabelBox>,
    /// Texts of the value-axis major tick labels, from the minimum up.
    pub value_tick_texts: Vec<String>,
    /// Texts of the X-axis major tick labels of a scatter or bubble chart.
    pub x_tick_texts: Vec<String>,
    /// Tick labels of a date axis.
    pub date_ticks: Vec<DateTick>,
    /// Centre of every category of the first series along the category axis (columns, lines,
    /// areas), in chart-area points.
    pub category_x: Vec<f64>,
}

/// Measures text. Supply real font metrics for exact layout; [`ApproxMeasure`] needs none.
pub trait TextMeasure {
    /// Advance width of `text` in points.
    fn width(&self, text: &str, font: &FontSpec) -> f64;

    /// Height of one line (ascent + descent + line gap) in points.
    fn line_height(&self, font: &FontSpec) -> f64 {
        font.size_pt * 1.2207
    }
}

/// A metric-free estimate (Calibri-like average advances); good for a preview, not for the
/// ±1 pt layout checks.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApproxMeasure;

impl TextMeasure for ApproxMeasure {
    fn width(&self, text: &str, font: &FontSpec) -> f64 {
        let em: f64 = text
            .chars()
            .map(|c| match c {
                '0'..='9' => 0.507,
                ' ' => 0.226,
                '.' | ',' | ':' | ';' | 'i' | 'l' | 'j' | '\'' | '|' => 0.25,
                '%' => 0.715,
                'm' | 'w' | 'M' | 'W' => 0.8,
                c if c.is_uppercase() => 0.6,
                _ => 0.46,
            })
            .sum();
        em * font.size_pt * if font.bold { 1.04 } else { 1.0 }
    }
}
