//! The DrawingML chart model (`c:chartSpace`) read from `xl/charts/chartN.xml`.
//!
//! The model is a typed *view* over the lossless XML tree: [`ChartSpace`] keeps the parsed
//! document ([`ParsedXml`]) and [`ChartSpace::to_bytes`] writes it back, so every element the
//! model does not read (extension lists, 3-D settings, shape properties, vendor markup) is
//! preserved exactly. The typed fields are what rendering needs.

use open365_xlsx::{ParsedXml, XmlElement, parse_xml};

use crate::error::ChartError;

const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const CHART_NS_STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/chart";
const DML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const DML_NS_STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/main";

/// Default text size of chart text when nothing says otherwise, in points.
pub const DEFAULT_TEXT_PT: f64 = 10.0;
/// Default chart title size in points (bold) when the chart states none; measured from Excel.
pub const DEFAULT_TITLE_PT: f64 = 18.0;

fn is_chart_ns(e: &XmlElement) -> bool {
    matches!(e.head.ns.as_deref(), Some(CHART_NS | CHART_NS_STRICT))
}

fn is_dml_ns(e: &XmlElement) -> bool {
    matches!(e.head.ns.as_deref(), Some(DML_NS | DML_NS_STRICT))
}

/// Child `c:local` of `e`.
fn child<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.elements()
        .find(|c| is_chart_ns(c) && c.head.local_name() == local)
}

fn children<'a>(e: &'a XmlElement, local: &'a str) -> impl Iterator<Item = &'a XmlElement> {
    e.elements()
        .filter(move |c| is_chart_ns(c) && c.head.local_name() == local)
}

fn val<'a>(e: &'a XmlElement, local: &str) -> Option<&'a str> {
    child(e, local).and_then(|c| c.head.attr("val"))
}

fn val_f64(e: &XmlElement, local: &str) -> Option<f64> {
    val(e, local)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

fn val_u32(e: &XmlElement, local: &str) -> Option<u32> {
    val(e, local).and_then(|v| v.trim().parse().ok())
}

/// `CT_Boolean`: a present element without `val` is true.
fn val_bool(e: &XmlElement, local: &str) -> Option<bool> {
    child(e, local).map(|c| !matches!(c.head.attr("val"), Some("0" | "false")))
}

fn text_of(e: &XmlElement) -> String {
    e.own_text()
}

/// An sRGB colour, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    fn parse(hex: &str) -> Option<Self> {
        let h = hex.trim();
        if h.len() != 6 {
            return None;
        }
        let n = u32::from_str_radix(h, 16).ok()?;
        Some(Self((n >> 16) as u8, (n >> 8) as u8, n as u8))
    }
}

/// The chart family of one `c:*Chart` element in the plot area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    Bar,
    Line,
    Area,
    Pie,
    Doughnut,
    Scatter,
    /// A chart type this model does not draw (bubble, radar, stock, surface, 3-D). The
    /// element stays in the tree.
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grouping {
    #[default]
    Standard,
    Clustered,
    Stacked,
    PercentStacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarDirection {
    #[default]
    Column,
    Bar,
}

/// One value of a series cache or of a resolved range.
#[derive(Debug, Clone, PartialEq)]
pub enum PointValue {
    Empty,
    Number(f64),
    Text(String),
}

impl PointValue {
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The point as label text; numbers use the shortest round-trip form.
    pub fn text(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Number(n) => open365_xlsx_number(*n),
            Self::Text(t) => t.clone(),
        }
    }
}

fn open365_xlsx_number(n: f64) -> String {
    let mut out = String::new();
    open365_xlsx::format_number(n, &mut out);
    out
}

/// A series data reference: the formula and the cached points Excel stored with it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataRef {
    /// `c:f`, for example `Data!$B$2:$B$6`; `None` for literal data (`c:numLit`, `c:strLit`).
    pub formula: Option<String>,
    /// Cached points by index; length is `c:ptCount`.
    pub cache: Vec<PointValue>,
    pub format_code: Option<String>,
    /// True for string data (`strRef`, `strLit`, `multiLvlStrRef`).
    pub is_text: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataLabels {
    pub show_value: bool,
    pub show_category: bool,
    pub show_series: bool,
    pub show_percent: bool,
    pub show_legend_key: bool,
    pub deleted: bool,
    /// `c:dLblPos`, for example `outEnd`.
    pub position: Option<String>,
    pub num_fmt: Option<String>,
    pub separator: Option<String>,
    pub font_pt: Option<f64>,
}

impl DataLabels {
    /// True when at least one label content flag is on.
    pub fn visible(&self) -> bool {
        !self.deleted
            && (self.show_value || self.show_category || self.show_series || self.show_percent)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Series {
    pub idx: u32,
    pub order: u32,
    /// `c:tx`: formula and cached name.
    pub name: Option<DataRef>,
    /// `c:cat`, or `c:xVal` of a scatter series.
    pub categories: Option<DataRef>,
    /// `c:val`, or `c:yVal` of a scatter series.
    pub values: Option<DataRef>,
    pub data_labels: Option<DataLabels>,
    pub fill: Option<Rgb>,
    pub line: Option<Rgb>,
    /// The series line is switched off (`a:ln` with `a:noFill`).
    pub line_none: bool,
    pub line_width_pt: Option<f64>,
    pub smooth: bool,
    pub explosion: Option<u32>,
    /// `c:marker/c:symbol`, for example `circle` or `none`.
    pub marker: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartGroup {
    pub kind: GroupKind,
    pub grouping: Grouping,
    pub direction: BarDirection,
    pub vary_colors: bool,
    pub gap_width: u32,
    pub overlap: i32,
    pub first_slice_angle: u32,
    pub hole_size: u32,
    /// `c:scatterStyle`, for example `lineMarker`.
    pub scatter_style: Option<String>,
    pub series: Vec<Series>,
    pub data_labels: Option<DataLabels>,
    pub axis_ids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisKind {
    Category,
    Value,
    Date,
    Series,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisPos {
    #[default]
    Bottom,
    Left,
    Top,
    Right,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Title {
    /// Plain text (paragraphs joined with `\n`), or the cached text of a reference.
    pub text: String,
    pub formula: Option<String>,
    pub overlay: bool,
    pub font_pt: Option<f64>,
    pub bold: Option<bool>,
    pub layout: Option<ManualLayout>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub id: u32,
    pub kind: AxisKind,
    pub position: AxisPos,
    pub deleted: bool,
    pub reversed: bool,
    pub log_base: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub major_unit: Option<f64>,
    pub minor_unit: Option<f64>,
    pub major_gridlines: bool,
    pub minor_gridlines: bool,
    pub title: Option<Title>,
    pub num_fmt: Option<String>,
    pub num_fmt_linked: bool,
    pub major_tick: String,
    pub minor_tick: String,
    pub tick_label_pos: String,
    pub cross_ax: Option<u32>,
    pub crosses: String,
    pub crosses_at: Option<f64>,
    /// `between` or `midCat` (value axes).
    pub cross_between: Option<String>,
    pub font_pt: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendPos {
    Right,
    Left,
    Top,
    Bottom,
    TopRight,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Legend {
    pub position: LegendPos,
    pub overlay: bool,
    pub layout: Option<ManualLayout>,
    pub font_pt: Option<f64>,
}

/// `c:manualLayout`; all values are fractions of the chart space. `x`/`y` are an edge position
/// when the mode is `edge` and an offset from the default position when it is `factor`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ManualLayout {
    /// `inner` (plot area without tick labels) or `outer`.
    pub inner: bool,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub x_edge: bool,
    pub y_edge: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotArea {
    pub layout: Option<ManualLayout>,
    pub groups: Vec<ChartGroup>,
    pub axes: Vec<Axis>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    pub title: Option<Title>,
    pub auto_title_deleted: bool,
    pub plot_area: PlotArea,
    pub legend: Option<Legend>,
    pub plot_visible_only: bool,
    pub blanks_as: String,
}

/// A parsed `c:chartSpace`.
#[derive(Debug, Clone)]
pub struct ChartSpace {
    pub date1904: bool,
    pub rounded_corners: bool,
    pub chart: Chart,
    /// Default chart text size (`c:chartSpace/c:txPr`), in points.
    pub text_pt: Option<f64>,
    /// Default chart font family (`a:latin/@typeface` of the chart-space `c:txPr`).
    pub text_family: Option<String>,
    xml: ParsedXml,
}

impl ChartSpace {
    /// Parses a chart part. Unknown elements are kept in the tree; a part that is not a
    /// `c:chartSpace` with a `c:chart` is an error.
    pub fn parse(bytes: &[u8]) -> Result<Self, ChartError> {
        let xml = parse_xml(bytes).map_err(|e| ChartError::Xml(e.to_string()))?;
        let root = &xml.root;
        if !is_chart_ns(root) || root.head.local_name() != "chartSpace" {
            return Err(ChartError::NotAChart);
        }
        let chart_el = child(root, "chart").ok_or(ChartError::NotAChart)?;
        let chart = parse_chart(chart_el);
        Ok(Self {
            date1904: val_bool(root, "date1904").unwrap_or(false),
            rounded_corners: val_bool(root, "roundedCorners").unwrap_or(false),
            text_pt: child(root, "txPr").and_then(text_size),
            text_family: child(root, "txPr").and_then(text_family),
            chart,
            xml,
        })
    }

    /// The lossless tree.
    pub fn xml(&self) -> &ParsedXml {
        &self.xml
    }

    /// The part bytes: the parsed document written back. Unedited, this is infoset-equal to
    /// the input and keeps every unknown element.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.xml.to_bytes()
    }
}

fn parse_chart(e: &XmlElement) -> Chart {
    let plot_el = child(e, "plotArea");
    Chart {
        title: child(e, "title").map(parse_title),
        auto_title_deleted: val_bool(e, "autoTitleDeleted").unwrap_or(false),
        plot_area: plot_el.map_or(
            PlotArea {
                layout: None,
                groups: Vec::new(),
                axes: Vec::new(),
            },
            parse_plot_area,
        ),
        legend: child(e, "legend").map(parse_legend),
        plot_visible_only: val_bool(e, "plotVisOnly").unwrap_or(true),
        blanks_as: val(e, "dispBlanksAs").unwrap_or("zero").to_owned(),
    }
}

/// First `a:defRPr`/`a:rPr` size (`sz`, hundredths of a point) below `e`.
fn text_size(e: &XmlElement) -> Option<f64> {
    fn walk(e: &XmlElement) -> Option<f64> {
        for c in e.elements() {
            if is_dml_ns(c)
                && matches!(c.head.local_name(), "defRPr" | "rPr")
                && let Some(sz) = c.head.attr("sz").and_then(|s| s.parse::<f64>().ok())
            {
                return Some(sz / 100.0);
            }
            if let Some(v) = walk(c) {
                return Some(v);
            }
        }
        None
    }
    walk(e)
}

fn text_family(e: &XmlElement) -> Option<String> {
    fn walk(e: &XmlElement) -> Option<String> {
        for c in e.elements() {
            if is_dml_ns(c)
                && c.head.local_name() == "latin"
                && let Some(t) = c.head.attr("typeface")
                && !t.starts_with('+')
            {
                return Some(t.to_owned());
            }
            if let Some(v) = walk(c) {
                return Some(v);
            }
        }
        None
    }
    walk(e)
}

fn text_bold(e: &XmlElement) -> Option<bool> {
    fn walk(e: &XmlElement) -> Option<bool> {
        for c in e.elements() {
            if is_dml_ns(c)
                && matches!(c.head.local_name(), "defRPr" | "rPr")
                && let Some(b) = c.head.attr("b")
            {
                return Some(matches!(b, "1" | "true"));
            }
            if let Some(v) = walk(c) {
                return Some(v);
            }
        }
        None
    }
    walk(e)
}

fn rich_text(rich: &XmlElement) -> String {
    let mut paragraphs = Vec::new();
    for p in rich
        .elements()
        .filter(|c| is_dml_ns(c) && c.head.local_name() == "p")
    {
        let mut line = String::new();
        for r in p.elements().filter(|c| is_dml_ns(c)) {
            match r.head.local_name() {
                "r" => {
                    for t in r
                        .elements()
                        .filter(|c| is_dml_ns(c) && c.head.local_name() == "t")
                    {
                        line.push_str(&text_of(t));
                    }
                }
                "br" => line.push('\n'),
                _ => {}
            }
        }
        paragraphs.push(line);
    }
    paragraphs.join("\n")
}

fn parse_title(e: &XmlElement) -> Title {
    let mut title = Title {
        overlay: val_bool(e, "overlay").unwrap_or(false),
        layout: child(e, "layout").and_then(parse_layout),
        ..Title::default()
    };
    if let Some(tx) = child(e, "tx") {
        if let Some(rich) = child(tx, "rich") {
            title.text = rich_text(rich);
            title.font_pt = text_size(rich);
            title.bold = text_bold(rich);
        } else if let Some(sr) = child(tx, "strRef") {
            let d = parse_data_ref(sr, true);
            title.text = d.cache.first().map(PointValue::text).unwrap_or_default();
            title.formula = d.formula;
        }
    }
    if let Some(tx_pr) = child(e, "txPr") {
        title.font_pt = title.font_pt.or_else(|| text_size(tx_pr));
        title.bold = title.bold.or_else(|| text_bold(tx_pr));
    }
    title
}

fn parse_layout(e: &XmlElement) -> Option<ManualLayout> {
    let m = child(e, "manualLayout")?;
    let edge = |mode: &str| val(m, mode) == Some("edge");
    Some(ManualLayout {
        inner: val(m, "layoutTarget") == Some("inner"),
        x: val_f64(m, "x"),
        y: val_f64(m, "y"),
        w: val_f64(m, "w"),
        h: val_f64(m, "h"),
        x_edge: edge("xMode"),
        y_edge: edge("yMode"),
    })
}

fn parse_legend(e: &XmlElement) -> Legend {
    Legend {
        position: match val(e, "legendPos") {
            Some("l") => LegendPos::Left,
            Some("t") => LegendPos::Top,
            Some("b") => LegendPos::Bottom,
            Some("tr") => LegendPos::TopRight,
            _ => LegendPos::Right,
        },
        overlay: val_bool(e, "overlay").unwrap_or(false),
        layout: child(e, "layout").and_then(parse_layout),
        font_pt: child(e, "txPr").and_then(text_size),
    }
}

fn parse_plot_area(e: &XmlElement) -> PlotArea {
    let mut groups = Vec::new();
    let mut axes = Vec::new();
    for c in e.elements().filter(|c| is_chart_ns(c)) {
        let name = c.head.local_name();
        if let Some(kind) = group_kind(name) {
            groups.push(parse_group(c, kind));
        } else if let Some(kind) = axis_kind(name) {
            axes.push(parse_axis(c, kind));
        }
    }
    PlotArea {
        layout: child(e, "layout").and_then(parse_layout),
        groups,
        axes,
    }
}

fn group_kind(name: &str) -> Option<GroupKind> {
    Some(match name {
        "barChart" => GroupKind::Bar,
        "lineChart" => GroupKind::Line,
        "areaChart" => GroupKind::Area,
        "pieChart" => GroupKind::Pie,
        "doughnutChart" => GroupKind::Doughnut,
        "scatterChart" => GroupKind::Scatter,
        "bar3DChart" | "line3DChart" | "area3DChart" | "pie3DChart" | "bubbleChart"
        | "radarChart" | "stockChart" | "surfaceChart" | "surface3DChart" | "ofPieChart" => {
            GroupKind::Unsupported
        }
        _ => return None,
    })
}

fn axis_kind(name: &str) -> Option<AxisKind> {
    Some(match name {
        "catAx" => AxisKind::Category,
        "valAx" => AxisKind::Value,
        "dateAx" => AxisKind::Date,
        "serAx" => AxisKind::Series,
        _ => return None,
    })
}

fn parse_group(e: &XmlElement, kind: GroupKind) -> ChartGroup {
    let grouping = match val(e, "grouping") {
        Some("clustered") => Grouping::Clustered,
        Some("stacked") => Grouping::Stacked,
        Some("percentStacked") => Grouping::PercentStacked,
        _ => Grouping::Standard,
    };
    ChartGroup {
        kind,
        grouping,
        direction: if val(e, "barDir") == Some("bar") {
            BarDirection::Bar
        } else {
            BarDirection::Column
        },
        vary_colors: val_bool(e, "varyColors").unwrap_or(false),
        gap_width: val_u32(e, "gapWidth").unwrap_or(150),
        overlap: val(e, "overlap")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0),
        first_slice_angle: val_u32(e, "firstSliceAng").unwrap_or(0),
        hole_size: val_u32(e, "holeSize").unwrap_or(10),
        scatter_style: val(e, "scatterStyle").map(str::to_owned),
        series: children(e, "ser").map(parse_series).collect(),
        data_labels: child(e, "dLbls").map(parse_data_labels),
        axis_ids: children(e, "axId")
            .filter_map(|a| a.head.attr("val")?.trim().parse().ok())
            .collect(),
    }
}

fn parse_data_labels(e: &XmlElement) -> DataLabels {
    DataLabels {
        show_value: val_bool(e, "showVal").unwrap_or(false),
        show_category: val_bool(e, "showCatName").unwrap_or(false),
        show_series: val_bool(e, "showSerName").unwrap_or(false),
        show_percent: val_bool(e, "showPercent").unwrap_or(false),
        show_legend_key: val_bool(e, "showLegendKey").unwrap_or(false),
        deleted: val_bool(e, "delete").unwrap_or(false),
        position: val(e, "dLblPos").map(str::to_owned),
        num_fmt: child(e, "numFmt").and_then(|n| n.head.attr("formatCode").map(str::to_owned)),
        separator: child(e, "separator").map(text_of),
        font_pt: child(e, "txPr").and_then(text_size),
    }
}

fn solid_rgb(sp_pr: &XmlElement, local: &str) -> Option<Rgb> {
    let holder = sp_pr
        .elements()
        .find(|c| is_dml_ns(c) && c.head.local_name() == local)?;
    let solid = holder
        .elements()
        .find(|c| is_dml_ns(c) && c.head.local_name() == "solidFill")
        .or((local == "solidFill").then_some(holder))?;
    let clr = solid
        .elements()
        .find(|c| is_dml_ns(c) && c.head.local_name() == "srgbClr")?;
    Rgb::parse(clr.head.attr("val")?)
}

fn parse_series(e: &XmlElement) -> Series {
    let sp_pr = child(e, "spPr");
    let data = |name: &str| {
        child(e, name).and_then(|c| {
            c.elements().find(|x| is_chart_ns(x)).map(|x| {
                let text = matches!(x.head.local_name(), "strRef" | "strLit" | "multiLvlStrRef");
                parse_data_ref(x, text)
            })
        })
    };
    let line_width = sp_pr
        .and_then(|s| {
            s.elements()
                .find(|c| is_dml_ns(c) && c.head.local_name() == "ln")
        })
        .and_then(|ln| ln.head.attr("w")?.parse::<f64>().ok())
        .map(|emu| emu / 12700.0);
    Series {
        idx: val_u32(e, "idx").unwrap_or(0),
        order: val_u32(e, "order").unwrap_or(0),
        name: data("tx"),
        categories: data("cat").or_else(|| data("xVal")),
        values: data("val").or_else(|| data("yVal")),
        data_labels: child(e, "dLbls").map(parse_data_labels),
        fill: sp_pr.and_then(|s| solid_rgb(s, "solidFill")),
        line: sp_pr.and_then(|s| solid_rgb(s, "ln")),
        line_none: sp_pr.is_some_and(line_is_none),
        line_width_pt: line_width,
        smooth: val_bool(e, "smooth").unwrap_or(false),
        explosion: val_u32(e, "explosion"),
        marker: child(e, "marker").and_then(|m| val(m, "symbol").map(str::to_owned)),
    }
}

/// Reads a `numRef`/`strRef`/`numLit`/`strLit`/`multiLvlStrRef` element.
fn parse_data_ref(e: &XmlElement, text: bool) -> DataRef {
    let mut out = DataRef {
        is_text: text,
        formula: child(e, "f").map(|f| text_of(f).trim().to_owned()),
        ..DataRef::default()
    };
    let cache = child(e, "numCache")
        .or_else(|| child(e, "strCache"))
        .or(matches!(e.head.local_name(), "numLit" | "strLit").then_some(e))
        .or_else(|| {
            // multiLvlStrRef: the innermost level is the first `c:lvl`.
            child(e, "multiLvlStrCache").and_then(|m| child(m, "lvl"))
        });
    if let Some(cache) = cache {
        out.format_code = child(cache, "formatCode").map(|f| text_of(f).trim().to_owned());
        let count = val_u32(cache, "ptCount").unwrap_or(0) as usize;
        let mut points: Vec<PointValue> = vec![PointValue::Empty; count.min(1 << 20)];
        for pt in children(cache, "pt") {
            let Some(idx) = pt.head.attr("idx").and_then(|i| i.parse::<usize>().ok()) else {
                continue;
            };
            if idx >= (1 << 20) {
                continue;
            }
            let raw = child(pt, "v").map(text_of).unwrap_or_default();
            let value = if text {
                PointValue::Text(raw)
            } else {
                raw.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .map_or(PointValue::Empty, PointValue::Number)
            };
            if idx >= points.len() {
                points.resize(idx + 1, PointValue::Empty);
            }
            points[idx] = value;
        }
        out.cache = points;
    }
    out
}

fn parse_axis(e: &XmlElement, kind: AxisKind) -> Axis {
    let scaling = child(e, "scaling");
    let sc = |local: &str| scaling.and_then(|s| val_f64(s, local));
    let num_fmt = child(e, "numFmt");
    Axis {
        id: val_u32(e, "axId").unwrap_or(0),
        kind,
        position: match val(e, "axPos") {
            Some("l") => AxisPos::Left,
            Some("t") => AxisPos::Top,
            Some("r") => AxisPos::Right,
            _ => AxisPos::Bottom,
        },
        deleted: val_bool(e, "delete").unwrap_or(false),
        reversed: scaling.and_then(|s| val(s, "orientation")) == Some("maxMin"),
        log_base: sc("logBase"),
        min: sc("min"),
        max: sc("max"),
        major_unit: val_f64(e, "majorUnit"),
        minor_unit: val_f64(e, "minorUnit"),
        major_gridlines: child(e, "majorGridlines").is_some(),
        minor_gridlines: child(e, "minorGridlines").is_some(),
        title: child(e, "title").map(parse_title),
        num_fmt: num_fmt.and_then(|n| n.head.attr("formatCode").map(str::to_owned)),
        num_fmt_linked: num_fmt
            .is_some_and(|n| matches!(n.head.attr("sourceLinked"), Some("1" | "true"))),
        major_tick: val(e, "majorTickMark").unwrap_or("cross").to_owned(),
        minor_tick: val(e, "minorTickMark").unwrap_or("cross").to_owned(),
        tick_label_pos: val(e, "tickLblPos").unwrap_or("nextTo").to_owned(),
        cross_ax: val_u32(e, "crossAx"),
        crosses: val(e, "crosses").unwrap_or("autoZero").to_owned(),
        crosses_at: val_f64(e, "crossesAt"),
        cross_between: val(e, "crossBetween").map(str::to_owned),
        font_pt: child(e, "txPr").and_then(text_size),
    }
}

fn line_is_none(sp_pr: &XmlElement) -> bool {
    sp_pr
        .elements()
        .find(|c| is_dml_ns(c) && c.head.local_name() == "ln")
        .is_some_and(|ln| {
            ln.elements()
                .any(|c| is_dml_ns(c) && c.head.local_name() == "noFill")
        })
}
