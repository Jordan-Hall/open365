//! Chart layout and drawing: from a [`ChartSpace`] and a size to a [`DisplayList`].
//!
//! Coordinates. Excel reports chart rectangles (`PlotArea.InsideLeft`, `Legend.Top`, ...) in a
//! chart-area system that is 5 pt narrower and shorter than the chart object, and this module
//! lays out in that system (`area = object size - 5 pt`), keeping the offsets Excel shows:
//! 7 pt left and top plot margin, 10 pt right and bottom, 11 pt between plot and legend, 2 pt
//! title top. The display list is drawn 1:1 in the object frame. The numeric constants in
//! this file were fitted to Excel's rectangles (`fixtures/reference/xlsx/chart-*/layout.json`);
//! `docs/architecture/charts.md` records what matches and what does not.

use crate::axis::{AxisScale, max_intervals, resolve_scale};
use crate::data::{CellSource, resolve};
use crate::display::{
    ApproxMeasure, DisplayItem, DisplayList, FontSpec, PathCmd, Rect, Rgba, Stroke, TextAnchor,
    TextMeasure, TextRun,
};
use crate::model::{
    AxisKind, BarDirection, Chart, ChartGroup, ChartSpace, DEFAULT_TEXT_PT, DEFAULT_TITLE_PT,
    DataLabels, GroupKind, Grouping, LegendPos, PointValue, Series, Title,
};

/// The Office 2013+ default series colours (accent 1 to 6).
pub const DEFAULT_PALETTE: [Rgba; 6] = [
    Rgba(0x44, 0x72, 0xC4, 255),
    Rgba(0xED, 0x7D, 0x31, 255),
    Rgba(0xA5, 0xA5, 0xA5, 255),
    Rgba(0xFF, 0xC0, 0x00, 255),
    Rgba(0x5B, 0x9B, 0xD5, 255),
    Rgba(0x70, 0xAD, 0x47, 255),
];

/// Pt between the chart object and the area Excel reports rectangles in.
pub const AREA_INSET: f64 = 5.0;

const MARGIN_LEFT: f64 = 7.0;
const MARGIN_TOP: f64 = 7.0;
const MARGIN_RIGHT: f64 = 10.0;
const MARGIN_BOTTOM: f64 = 10.0;
const LEGEND_GAP: f64 = 11.0;
const LEGEND_EDGE: f64 = 5.0;
const TITLE_TOP: f64 = 2.0;
/// Title box height beyond its text line.
const TITLE_PAD_H: f64 = 3.0;
const TITLE_PAD_W: f64 = 6.0;
/// Legend entry height beyond the text line, and key + padding width beyond the text.
const LEGEND_ENTRY_PAD_H: f64 = 5.87;
const LEGEND_ENTRY_PAD_W: f64 = 16.98;
/// A horizontal legend is this much wider than the sum of its entries.
const LEGEND_ROW_EXTRA: f64 = -4.2;
/// Each entry of a horizontal legend is this much wider than its vertical form.
const LEGEND_ROW_ENTRY_EXTRA: f64 = 2.58;
/// Line and scatter keys (line segment with marker) are this much wider than a square key.
const LEGEND_LINE_KEY_EXTRA: f64 = 13.0;
/// Axis label box: pad beyond the text.
const CAT_LABEL_PAD_H: f64 = 1.76;
const VAL_LABEL_PAD_W: f64 = 4.73;
/// Extra space on the side of an axis that has a title.
const AXIS_TITLE_PLOT_PAD: f64 = 6.0;
const AXIS_TITLE_PAD: f64 = 3.0;

/// How to render.
pub struct RenderOptions<'a> {
    pub measure: &'a dyn TextMeasure,
    pub cells: &'a dyn CellSource,
    pub font_family: String,
    pub palette: Vec<Rgba>,
}

impl std::fmt::Debug for RenderOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderOptions")
            .field("font_family", &self.font_family)
            .field("palette", &self.palette)
            .finish_non_exhaustive()
    }
}

impl<'a> RenderOptions<'a> {
    pub fn new(cells: &'a dyn CellSource, measure: &'a dyn TextMeasure) -> Self {
        Self {
            measure,
            cells,
            font_family: "Calibri".to_owned(),
            palette: DEFAULT_PALETTE.to_vec(),
        }
    }
}

struct Resolved {
    name: String,
    color: Rgba,
    line_color: Option<Rgba>,
    values: Vec<Option<f64>>,
    cats: Vec<String>,
    xs: Vec<Option<f64>>,
    labels: Option<DataLabels>,
    series: Series,
    format_code: Option<String>,
}

struct Group<'c> {
    def: &'c ChartGroup,
    series: Vec<Resolved>,
}

fn shade(c: Rgba, f: f64) -> Rgba {
    let g = |v: u8| (f64::from(v) * f).round().clamp(0.0, 255.0) as u8;
    Rgba(g(c.0), g(c.1), g(c.2), c.3)
}

fn series_color(
    opts: &RenderOptions<'_>,
    index: usize,
    explicit: Option<crate::model::Rgb>,
) -> Rgba {
    if let Some(c) = explicit {
        return Rgba::rgb(c.0, c.1, c.2);
    }
    let pal = &opts.palette;
    if pal.is_empty() {
        return Rgba::BLACK;
    }
    let base = pal[index % pal.len()];
    match index / pal.len() {
        0 => base,
        1 => shade(base, 0.6),
        _ => shade(base, 1.4),
    }
}

fn resolve_group<'c>(
    def: &'c ChartGroup,
    opts: &RenderOptions<'_>,
    first_index: usize,
) -> Group<'c> {
    let mut series = Vec::new();
    for (i, s) in def.series.iter().enumerate() {
        let values: Vec<Option<f64>> = s
            .values
            .as_ref()
            .map(|d| resolve(d, opts.cells))
            .unwrap_or_default()
            .iter()
            .map(PointValue::number)
            .collect();
        let (cats, xs) = match &s.categories {
            Some(c) => {
                let pts = resolve(c, opts.cells);
                let xs = pts.iter().map(PointValue::number).collect();
                (pts.iter().map(PointValue::text).collect(), xs)
            }
            None => (Vec::new(), Vec::new()),
        };
        let name = s
            .name
            .as_ref()
            .map(|d| {
                resolve(d, opts.cells)
                    .first()
                    .map(PointValue::text)
                    .unwrap_or_default()
            })
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Series{}", s.idx + 1));
        let color = series_color(opts, first_index + i, s.fill);
        series.push(Resolved {
            name,
            color,
            line_color: s.line.map(|c| Rgba::rgb(c.0, c.1, c.2)),
            values,
            cats,
            xs,
            labels: s.data_labels.clone().or_else(|| def.data_labels.clone()),
            series: s.clone(),
            format_code: s.values.as_ref().and_then(|d| d.format_code.clone()),
        });
    }
    Group { def, series }
}

fn font(opts: &RenderOptions<'_>, size: f64, bold: bool) -> FontSpec {
    FontSpec {
        family: opts.font_family.clone(),
        size_pt: size,
        bold,
        italic: false,
    }
}

struct Canvas<'a> {
    items: Vec<DisplayItem>,
    opts: &'a RenderOptions<'a>,
}

impl Canvas<'_> {
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        anchor: TextAnchor,
        font: &FontSpec,
        color: Rgba,
        rotation_deg: f64,
    ) -> Rect {
        let width = self.opts.measure.width(text, font);
        let height = self.opts.measure.line_height(font);
        self.items.push(DisplayItem::Text(TextRun {
            text: text.to_owned(),
            x,
            y,
            width,
            height,
            anchor,
            font: font.clone(),
            color,
            rotation_deg,
        }));
        let left = match anchor {
            TextAnchor::Start => x,
            TextAnchor::Middle => x - width / 2.0,
            TextAnchor::End => x - width,
        };
        Rect::new(left, y, width, height)
    }

    fn rect(&mut self, rect: Rect, fill: Option<Rgba>, stroke: Option<Stroke>) {
        self.items.push(DisplayItem::Rect { rect, fill, stroke });
    }

    fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, stroke: Stroke) {
        self.items.push(DisplayItem::Path {
            cmds: vec![PathCmd::MoveTo(x1, y1), PathCmd::LineTo(x2, y2)],
            fill: None,
            stroke: Some(stroke),
        });
    }
}

const TEXT_COLOR: Rgba = Rgba(0x59, 0x59, 0x59, 255);
const GRID_COLOR: Rgba = Rgba(0xD9, 0xD9, 0xD9, 255);
const AXIS_COLOR: Rgba = Rgba(0xBF, 0xBF, 0xBF, 255);

fn format_number(v: f64, code: &str, date1904: bool) -> String {
    use open365_calc::Value;
    let code = if code.is_empty() { "General" } else { code };
    open365_calc::numfmt::format_value(&Value::Number(v), code, date1904)
}

fn is_cartesian(k: GroupKind) -> bool {
    matches!(
        k,
        GroupKind::Bar | GroupKind::Line | GroupKind::Area | GroupKind::Scatter
    )
}

/// Renders with the cached values only and an estimated text measure.
pub fn render_preview(chart: &ChartSpace, width_pt: f64, height_pt: f64) -> DisplayList {
    let opts = RenderOptions::new(&crate::data::NoCells, &ApproxMeasure);
    render(chart, width_pt, height_pt, &opts)
}

/// Lays out and draws `chart` into a `width_pt` by `height_pt` object.
pub fn render(
    space: &ChartSpace,
    width_pt: f64,
    height_pt: f64,
    opts: &RenderOptions<'_>,
) -> DisplayList {
    let chart = &space.chart;
    let family_opts;
    let opts = match &space.text_family {
        Some(f) if *f != opts.font_family => {
            family_opts = RenderOptions {
                measure: opts.measure,
                cells: opts.cells,
                font_family: f.clone(),
                palette: opts.palette.clone(),
            };
            &family_opts
        }
        _ => opts,
    };
    let base_pt = space.text_pt.unwrap_or(DEFAULT_TEXT_PT);
    let area_w = (width_pt - AREA_INSET).max(1.0);
    let area_h = (height_pt - AREA_INSET).max(1.0);
    let mut cv = Canvas {
        items: Vec::new(),
        opts,
    };
    cv.rect(
        Rect::new(0.0, 0.0, width_pt, height_pt),
        Some(Rgba::WHITE),
        Some(Stroke::solid(Rgba(0xD9, 0xD9, 0xD9, 255), 0.75)),
    );

    // Resolve data.
    let mut groups: Vec<Group<'_>> = Vec::new();
    let mut counted = 0;
    for def in &chart.plot_area.groups {
        if def.kind == GroupKind::Unsupported {
            continue;
        }
        let g = resolve_group(def, opts, counted);
        counted += g.series.len();
        groups.push(g);
    }
    let mut list = DisplayList {
        width: width_pt,
        height: height_pt,
        ..DisplayList::default()
    };
    if groups.is_empty() {
        list.items = cv.items;
        return list;
    }

    // Title.
    let center_x = (MARGIN_LEFT + area_w - MARGIN_RIGHT) / 2.0;
    let mut title_rect = None;
    let title_font;
    let title_text = chart_title(chart, &groups);
    if let Some(t) = &title_text {
        let pt = chart
            .title
            .as_ref()
            .and_then(|t| t.font_pt)
            .unwrap_or(DEFAULT_TITLE_PT);
        title_font = font(
            opts,
            pt,
            chart.title.as_ref().and_then(|t| t.bold).unwrap_or(true),
        );
        let w = opts.measure.width(t, &title_font) + TITLE_PAD_W;
        let h = opts.measure.line_height(&title_font) + TITLE_PAD_H;
        title_rect = Some(Rect::new(center_x - w / 2.0, TITLE_TOP, w, h));
    } else {
        title_font = font(opts, DEFAULT_TITLE_PT, true);
    }

    // Legend.
    let legend_entries = legend_entries(&groups);
    let mut legend_rect: Option<Rect> = None;
    let mut legend_font = font(opts, base_pt, false);
    if let (Some(l), false) = (&chart.legend, legend_entries.is_empty()) {
        legend_font = font(opts, l.font_pt.unwrap_or(base_pt), false);
        let entry_h = opts.measure.line_height(&legend_font) + LEGEND_ENTRY_PAD_H;
        let widths: Vec<f64> = legend_entries
            .iter()
            .map(|e| {
                opts.measure.width(&e.name, &legend_font)
                    + e.key_pad(matches!(l.position, LegendPos::Top | LegendPos::Bottom))
            })
            .collect();
        let (w, h) = match l.position {
            LegendPos::Left | LegendPos::Right | LegendPos::TopRight => (
                widths.iter().cloned().fold(0.0, f64::max),
                entry_h * legend_entries.len() as f64,
            ),
            LegendPos::Top | LegendPos::Bottom => (
                widths.iter().sum::<f64>()
                    + LEGEND_ROW_ENTRY_EXTRA * widths.len() as f64
                    + LEGEND_ROW_EXTRA,
                entry_h,
            ),
        };
        legend_rect = Some(Rect::new(0.0, 0.0, w, h));
    }

    // Plot outer rectangle.
    let mut outer = Rect::new(
        MARGIN_LEFT,
        MARGIN_TOP,
        area_w - MARGIN_LEFT - MARGIN_RIGHT,
        area_h - MARGIN_TOP - MARGIN_BOTTOM,
    );
    let mut top = MARGIN_TOP;
    let mut bottom = area_h - MARGIN_BOTTOM;
    let mut left = MARGIN_LEFT;
    let mut right = area_w - MARGIN_RIGHT;
    if let Some(t) = title_rect
        && chart.title.as_ref().is_none_or(|t| !t.overlay)
    {
        top = t.bottom() + LEGEND_GAP;
    }
    if let (Some(l), Some(lr)) = (&chart.legend, legend_rect.as_mut())
        && !l.overlay
    {
        match l.position {
            LegendPos::Right | LegendPos::TopRight => {
                lr.x = area_w - LEGEND_EDGE - lr.width;
                right = lr.x - LEGEND_GAP;
            }
            LegendPos::Left => {
                lr.x = TITLE_TOP;
                left = lr.right() + LEGEND_GAP;
            }
            LegendPos::Top => {
                lr.y = title_rect.map_or(TITLE_TOP, |t| t.bottom() + 6.0);
                top = lr.bottom() + LEGEND_GAP;
            }
            LegendPos::Bottom => {
                lr.y = area_h - LEGEND_EDGE - lr.height;
                bottom = lr.y - LEGEND_GAP;
            }
        }
    }
    outer.x = left;
    outer.y = top;
    outer.width = (right - left).max(1.0);
    outer.height = (bottom - top).max(1.0);
    if let Some(lr) = legend_rect.as_mut() {
        match chart.legend.as_ref().map(|l| l.position) {
            Some(LegendPos::Right | LegendPos::Left | LegendPos::TopRight) => {
                lr.y = outer.center_y() - lr.height / 2.0;
            }
            _ => lr.x = outer.center_x() - lr.width / 2.0,
        }
    }
    if let Some(lay) = chart.plot_area.layout {
        outer = manual_rect(lay, area_w, area_h, outer);
    }

    // Plot.
    let first = &groups[0];
    let cartesian = is_cartesian(first.def.kind);
    let inside;
    let plot_outer;
    let mut scales = None;
    if cartesian {
        let layout = cartesian_layout(chart, &groups, outer, base_pt, opts, space.date1904);
        inside = layout.inside;
        plot_outer = layout.outer;
        scales = Some(layout);
    } else {
        // Pie and doughnut: a square centred in the available rectangle.
        let label_pad = groups[0]
            .series
            .first()
            .and_then(|s| s.labels.as_ref())
            .filter(|l| l.visible())
            .map_or(0.0, |_| {
                opts.measure.line_height(&font(opts, base_pt, false)) + 2.07
            });
        let side = (outer.width.min(outer.height) - 2.0 * label_pad).max(1.0);
        let sq = Rect::new(
            outer.center_x() - side / 2.0,
            outer.center_y() - side / 2.0,
            side,
            side,
        );
        inside = sq;
        plot_outer = sq;
    }

    // Draw axes, series.
    if let Some(sc) = &scales {
        draw_axes(&mut cv, sc, chart, base_pt, space.date1904);
        draw_cartesian(&mut cv, &groups, sc, base_pt, space.date1904);
    } else {
        draw_pie(&mut cv, first, inside, base_pt, space.date1904);
    }

    // Title and legend on top.
    if let (Some(t), Some(r)) = (&title_text, title_rect) {
        cv.text(
            t,
            r.center_x(),
            r.y + TITLE_PAD_H / 2.0,
            TextAnchor::Middle,
            &title_font,
            TEXT_COLOR,
            0.0,
        );
    }
    if let Some(lr) = legend_rect {
        draw_legend(&mut cv, lr, &legend_entries, &legend_font, chart);
    }

    list.items = cv.items;
    list.plot_inside = inside;
    list.plot_outer = plot_outer;
    list.legend = legend_rect;
    list.title = title_rect;
    list
}

fn manual_rect(lay: crate::model::ManualLayout, w: f64, h: f64, default: Rect) -> Rect {
    let pick = |v: Option<f64>, edge: bool, total: f64, dflt: f64| match (v, edge) {
        (Some(v), true) => v * total,
        (Some(v), false) => dflt + v * total,
        (None, _) => dflt,
    };
    Rect::new(
        pick(lay.x, lay.x_edge, w, default.x),
        pick(lay.y, lay.y_edge, h, default.y),
        lay.w.map_or(default.width, |v| v * w),
        lay.h.map_or(default.height, |v| v * h),
    )
}

fn chart_title(chart: &Chart, groups: &[Group<'_>]) -> Option<String> {
    match &chart.title {
        Some(Title { text, .. }) if !text.is_empty() => Some(text.clone()),
        Some(_) => groups
            .first()
            .and_then(|g| g.series.first())
            .map(|s| s.name.clone()),
        None => None,
    }
}

struct LegendEntry {
    name: String,
    color: Rgba,
    /// The key is a line segment with a marker rather than a square.
    line: bool,
}

impl LegendEntry {
    /// Width the key and its padding take beside the text.
    fn key_pad(&self, row: bool) -> f64 {
        let line = if row {
            LEGEND_LINE_KEY_EXTRA + 2.62
        } else {
            LEGEND_LINE_KEY_EXTRA
        };
        LEGEND_ENTRY_PAD_W + if self.line { line } else { 0.0 }
    }
}

fn legend_entries(groups: &[Group<'_>]) -> Vec<LegendEntry> {
    let Some(first) = groups.first() else {
        return Vec::new();
    };
    if matches!(first.def.kind, GroupKind::Pie | GroupKind::Doughnut) {
        let Some(s) = first.series.first() else {
            return Vec::new();
        };
        return s
            .values
            .iter()
            .enumerate()
            .map(|(i, _)| LegendEntry {
                name: s
                    .cats
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| (i + 1).to_string()),
                color: point_color(s, i),
                line: false,
            })
            .collect();
    }
    groups
        .iter()
        .flat_map(|g| g.series.iter().map(move |s| (g.def.kind, s)))
        .map(|(kind, s)| LegendEntry {
            name: s.name.clone(),
            color: s.color,
            line: matches!(kind, GroupKind::Line | GroupKind::Scatter),
        })
        .collect()
}

fn point_color(s: &Resolved, i: usize) -> Rgba {
    let pal = DEFAULT_PALETTE;
    match i / pal.len() {
        0 => pal[i % pal.len()],
        1 => shade(pal[i % pal.len()], 0.6),
        _ => shade(pal[i % pal.len()], 1.4),
    }
    .pipe(|c| {
        if s.series.fill.is_some() && i == 0 {
            s.color
        } else {
            c
        }
    })
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

fn draw_legend(
    cv: &mut Canvas<'_>,
    r: Rect,
    entries: &[LegendEntry],
    font: &FontSpec,
    chart: &Chart,
) {
    let horizontal = matches!(
        chart.legend.as_ref().map(|l| l.position),
        Some(LegendPos::Top | LegendPos::Bottom)
    );
    let line_h = cv.opts.measure.line_height(font);
    let entry_h = line_h + LEGEND_ENTRY_PAD_H;
    let (mut x, mut y) = (r.x, r.y);
    for e in entries {
        let name = &e.name;
        let ew = cv.opts.measure.width(name, font)
            + e.key_pad(horizontal)
            + if horizontal {
                LEGEND_ROW_ENTRY_EXTRA
            } else {
                0.0
            };
        let key = 7.0;
        let text_x = x + ew - cv.opts.measure.width(name, font) - 3.0;
        if e.line {
            let ky = y + entry_h / 2.0;
            cv.line(x + 4.0, ky, text_x - 4.0, ky, Stroke::solid(e.color, 2.25));
            cv.items.push(DisplayItem::Path {
                cmds: circle((x + text_x) / 2.0, ky, 2.5),
                fill: Some(e.color),
                stroke: None,
            });
        } else {
            cv.rect(
                Rect::new(x + 5.0, y + (entry_h - key) / 2.0, key, key),
                Some(e.color),
                None,
            );
        }
        cv.text(
            name,
            text_x,
            y + (entry_h - line_h) / 2.0,
            TextAnchor::Start,
            font,
            TEXT_COLOR,
            0.0,
        );
        if horizontal {
            x += ew;
        } else {
            y += entry_h;
        }
    }
}

struct CartLayout {
    outer: Rect,
    inside: Rect,
    horizontal_bars: bool,
    value: AxisScale,
    /// Category-axis scale for scatter; `None` for category axes.
    x_scale: Option<AxisScale>,
    categories: usize,
    cross_between_mid: bool,
    value_fmt: String,
    x_fmt: String,
    cat_labels: Vec<String>,
    val_title: Option<String>,
    cat_title: Option<String>,
    percent: bool,
    reversed_cats: bool,
    show_val_axis: bool,
    show_cat_axis: bool,
    gridlines: bool,
    label_font: FontSpec,
}

type Extent = Option<(f64, f64)>;

fn all_values(groups: &[Group<'_>]) -> (Extent, Extent) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut xlo = f64::INFINITY;
    let mut xhi = f64::NEG_INFINITY;
    for g in groups {
        let stacked = matches!(g.def.grouping, Grouping::Stacked | Grouping::PercentStacked)
            && g.def.kind != GroupKind::Scatter;
        if stacked {
            let n = g.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
            for i in 0..n {
                let (mut pos, mut neg) = (0.0, 0.0);
                for s in &g.series {
                    if let Some(Some(v)) = s.values.get(i) {
                        if *v >= 0.0 {
                            pos += v;
                        } else {
                            neg += v;
                        }
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                }
                lo = lo.min(neg).min(pos);
                hi = hi.max(pos).max(neg);
            }
        } else {
            for s in &g.series {
                for v in s.values.iter().flatten() {
                    lo = lo.min(*v);
                    hi = hi.max(*v);
                }
                if g.def.kind == GroupKind::Scatter {
                    for v in s.xs.iter().flatten() {
                        xlo = xlo.min(*v);
                        xhi = xhi.max(*v);
                    }
                }
            }
        }
    }
    let y = (lo <= hi).then_some((lo, hi));
    let x = (xlo <= xhi).then_some((xlo, xhi));
    (y, x)
}

fn cartesian_layout(
    chart: &Chart,
    groups: &[Group<'_>],
    outer: Rect,
    base_pt: f64,
    opts: &RenderOptions<'_>,
    date1904: bool,
) -> CartLayout {
    let first = &groups[0];
    let horizontal_bars =
        first.def.kind == GroupKind::Bar && first.def.direction == BarDirection::Bar;
    let scatter = first.def.kind == GroupKind::Scatter;
    let percent = first.def.grouping == Grouping::PercentStacked;
    let axes = &chart.plot_area.axes;
    let val_axis = axes.iter().find(|a| a.kind == AxisKind::Value);
    let cat_axis = axes
        .iter()
        .find(|a| matches!(a.kind, AxisKind::Category | AxisKind::Date))
        .or_else(|| {
            if scatter {
                axes.iter().find(|a| a.kind == AxisKind::Value)
            } else {
                None
            }
        });
    // For scatter charts, the first value axis listed is X (bottom) and the second is Y.
    let (val_axis, cat_axis) = if scatter {
        let mut v = axes.iter().filter(|a| a.kind == AxisKind::Value);
        let x = v.next();
        let y = v.next();
        (y.or(x), x)
    } else {
        (val_axis, cat_axis)
    };

    let label_pt = val_axis.and_then(|a| a.font_pt).unwrap_or(base_pt);
    let label_font = font(opts, label_pt, false);
    let line_h = opts.measure.line_height(&label_font);
    let cat_font = font(
        opts,
        cat_axis.and_then(|a| a.font_pt).unwrap_or(base_pt),
        false,
    );
    let cat_line_h = opts.measure.line_height(&cat_font);

    let val_title = val_axis
        .and_then(|a| a.title.as_ref())
        .map(|t| t.text.clone())
        .filter(|t| !t.is_empty());
    let cat_title = cat_axis
        .and_then(|a| a.title.as_ref())
        .map(|t| t.text.clone())
        .filter(|t| !t.is_empty());
    let n_cat = groups
        .iter()
        .flat_map(|g| g.series.iter())
        .map(|s| s.values.len().max(s.cats.len()))
        .max()
        .unwrap_or(0);
    let cat_labels: Vec<String> = if scatter {
        Vec::new()
    } else {
        let src = first.series.iter().max_by_key(|s| s.cats.len());
        (0..n_cat)
            .map(|i| {
                src.and_then(|s| s.cats.get(i).cloned())
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| (i + 1).to_string())
            })
            .collect()
    };

    let (yext, xext) = all_values(groups);
    let val_deleted = val_axis.is_some_and(|a| a.deleted);
    let cat_deleted = cat_axis.is_some_and(|a| a.deleted);
    let vfmt = val_axis
        .and_then(|a| (!a.num_fmt_linked).then(|| a.num_fmt.clone()).flatten())
        .or_else(|| {
            if percent {
                Some("0%".to_owned())
            } else {
                first.series.first().and_then(|s| s.format_code.clone())
            }
        })
        .unwrap_or_else(|| "General".to_owned());
    let xfmt = cat_axis
        .and_then(|a| (!a.num_fmt_linked).then(|| a.num_fmt.clone()).flatten())
        .unwrap_or_else(|| "General".to_owned());

    // Space the axis labels and titles take from the outer rectangle.
    let cat_h = if cat_deleted {
        0.0
    } else {
        cat_line_h + CAT_LABEL_PAD_H
    };
    let title_w = if val_title.is_some() {
        AXIS_TITLE_PLOT_PAD
    } else {
        0.0
    };
    let title_h = if cat_title.is_some() {
        AXIS_TITLE_PLOT_PAD
    } else {
        0.0
    };
    let mut o = outer;
    if val_title.is_some() {
        let w = line_h + AXIS_TITLE_PAD;
        let (a, b) = if horizontal_bars { (0.0, w) } else { (w, 0.0) };
        let _ = b;
        if !horizontal_bars {
            o.x += a;
            o.width -= a;
        } else {
            o.height -= w;
        }
    }
    if cat_title.is_some() {
        let w = cat_line_h + AXIS_TITLE_PAD;
        if !horizontal_bars {
            o.height -= w;
        } else {
            o.x += w;
            o.width -= w;
        }
    }

    let value;
    let mut x_scale = None;
    let inside;
    let half_label = |scale: &AxisScale, fmt: &str| -> f64 {
        scale.ticks().last().map_or(0.0, |t| {
            opts.measure
                .width(&format_number(*t, fmt, date1904), &label_font)
                / 2.0
        })
    };
    if horizontal_bars {
        // Categories run down the left edge; values along the bottom.
        let cat_w = if cat_deleted {
            0.0
        } else {
            cat_labels
                .iter()
                .map(|l| opts.measure.width(l, &cat_font))
                .fold(0.0, f64::max)
                + VAL_LABEL_PAD_W
        };
        let left_title = if cat_title.is_some() {
            AXIS_TITLE_PLOT_PAD
        } else {
            0.0
        };
        let base_w = (o.width - cat_w - left_title).max(1.0);
        let val_h = if val_deleted {
            0.0
        } else {
            line_h + CAT_LABEL_PAD_H
        };
        let in_h = (o.height
            - val_h
            - if val_title.is_some() {
                AXIS_TITLE_PLOT_PAD
            } else {
                0.0
            })
        .max(1.0);
        // The last value label is centred on the right edge and overhangs it.
        let mut in_w = base_w;
        let mut v = pick_scale(val_axis, yext, percent, max_intervals(in_w, true, label_pt));
        for _ in 0..2 {
            in_w = (base_w - half_label(&v, &vfmt)).max(1.0);
            v = pick_scale(val_axis, yext, percent, max_intervals(in_w, true, label_pt));
        }
        value = v;
        inside = Rect::new(o.x + cat_w + left_title, o.y, in_w, in_h);
    } else {
        let in_h = (o.height - cat_h - title_h).max(1.0);
        value = pick_scale(
            val_axis,
            yext,
            percent,
            max_intervals(in_h, false, label_pt),
        );
        let mut vw = if val_deleted {
            0.0
        } else {
            value
                .ticks()
                .iter()
                .map(|t| {
                    opts.measure
                        .width(&format_number(*t, &vfmt, date1904), &label_font)
                })
                .fold(0.0, f64::max)
                + VAL_LABEL_PAD_W
        };
        vw += title_w;
        let mut in_w = (o.width - vw).max(1.0);
        if scatter {
            let pick = |w: f64| {
                resolve_scale(
                    cat_axis.and_then(|a| a.min),
                    cat_axis.and_then(|a| a.max),
                    cat_axis.and_then(|a| a.major_unit),
                    cat_axis.and_then(|a| a.minor_unit),
                    xext,
                    max_intervals(w, true, label_pt),
                )
            };
            let base_w = in_w;
            let mut xs = pick(in_w);
            for _ in 0..2 {
                in_w = (base_w - half_label(&xs, &xfmt)).max(1.0);
                xs = pick(in_w);
            }
            x_scale = Some(xs);
        } else if first.def.kind == GroupKind::Area && cat_labels.len() > 1 && !cat_deleted {
            // Area charts put the first and last category on the plot edges; the last label
            // overhangs the right edge.
            let last = cat_labels
                .last()
                .map_or(0.0, |l| opts.measure.width(l, &cat_font));
            in_w = (in_w - last / 2.0).max(1.0);
        }
        inside = Rect::new(o.x + vw, o.y + 0.1, in_w, in_h - 0.1);
    }
    let cross_mid = val_axis
        .and_then(|a| a.cross_between.as_deref())
        .map_or(first.def.kind == GroupKind::Area, |c| c == "midCat");
    CartLayout {
        outer: o,
        inside,
        horizontal_bars,
        value,
        x_scale,
        categories: n_cat,
        cross_between_mid: cross_mid,
        value_fmt: vfmt,
        x_fmt: xfmt,
        cat_labels,
        val_title,
        cat_title,
        percent,
        reversed_cats: cat_axis.is_some_and(|a| a.reversed),
        show_val_axis: !val_deleted,
        show_cat_axis: !cat_deleted,
        gridlines: val_axis.is_none_or(|a| a.major_gridlines),
        label_font,
    }
}

fn pick_scale(
    axis: Option<&crate::model::Axis>,
    ext: Option<(f64, f64)>,
    percent: bool,
    n: u32,
) -> AxisScale {
    let ext = if percent { Some((0.0, 1.0)) } else { ext };
    if percent {
        let major = axis.and_then(|a| a.major_unit).unwrap_or_else(|| {
            let mut m = 0.1;
            for e in [0.1, 0.2, 0.5, 1.0] {
                m = e;
                if 1.0 / e <= f64::from(n) {
                    break;
                }
            }
            m
        });
        return AxisScale {
            min: axis.and_then(|a| a.min).unwrap_or(0.0),
            max: axis.and_then(|a| a.max).unwrap_or(1.0),
            major,
            minor: major / 5.0,
        };
    }
    resolve_scale(
        axis.and_then(|a| a.min),
        axis.and_then(|a| a.max),
        axis.and_then(|a| a.major_unit),
        axis.and_then(|a| a.minor_unit),
        ext,
        n,
    )
}

fn val_to(l: &CartLayout, v: f64) -> f64 {
    let s = &l.value;
    let t = ((v - s.min) / (s.max - s.min)).clamp(-0.5, 1.5);
    if l.horizontal_bars {
        l.inside.x + t * l.inside.width
    } else {
        l.inside.bottom() - t * l.inside.height
    }
}

fn draw_axes(cv: &mut Canvas<'_>, l: &CartLayout, chart: &Chart, base_pt: f64, date1904: bool) {
    let ins = l.inside;
    let grid = Stroke::solid(GRID_COLOR, 0.75);
    let axis_stroke = Stroke::solid(AXIS_COLOR, 0.75);
    let ticks = l.value.ticks();
    let font = l.label_font.clone();
    let lh = cv.opts.measure.line_height(&font);
    for t in &ticks {
        let p = val_to(l, *t);
        let label = format_number(*t, &l.value_fmt, date1904);
        if l.horizontal_bars {
            if l.gridlines {
                cv.line(p, ins.y, p, ins.bottom(), grid.clone());
            }
            if l.show_val_axis {
                cv.text(
                    &label,
                    p,
                    ins.bottom() + 1.76,
                    TextAnchor::Middle,
                    &font,
                    TEXT_COLOR,
                    0.0,
                );
            }
        } else if l.gridlines {
            cv.line(ins.x, p, ins.right(), p, grid.clone());
        }
        if !l.horizontal_bars && l.show_val_axis {
            cv.text(
                &label,
                ins.x - 4.0,
                p - lh / 2.0,
                TextAnchor::End,
                &font,
                TEXT_COLOR,
                0.0,
            );
        }
    }
    // Category axis (or scatter X).
    if let Some(xs) = &l.x_scale {
        for t in xs.ticks() {
            let x = ins.x + (t - xs.min) / (xs.max - xs.min) * ins.width;
            cv.line(x, ins.bottom(), x, ins.bottom() + 3.0, axis_stroke.clone());
            if l.show_cat_axis {
                let label = format_number(t, &l.x_fmt, date1904);
                cv.text(
                    &label,
                    x,
                    ins.bottom() + 1.76,
                    TextAnchor::Middle,
                    &font,
                    TEXT_COLOR,
                    0.0,
                );
            }
        }
    } else if l.categories > 0 {
        let n = l.categories as f64;
        for (i, label) in l.cat_labels.iter().enumerate() {
            let k = if l.reversed_cats {
                l.categories - 1 - i
            } else {
                i
            } as f64;
            if l.horizontal_bars {
                let band = ins.height / n;
                let y = ins.bottom() - (k + 0.5) * band;
                if l.show_cat_axis {
                    cv.text(
                        label,
                        ins.x - 4.0,
                        y - lh / 2.0,
                        TextAnchor::End,
                        &font,
                        TEXT_COLOR,
                        0.0,
                    );
                }
            } else {
                let x = cat_x(l, k);
                cv.line(
                    if l.cross_between_mid {
                        x
                    } else {
                        ins.x + (k + 1.0) * ins.width / n
                    },
                    ins.bottom(),
                    if l.cross_between_mid {
                        x
                    } else {
                        ins.x + (k + 1.0) * ins.width / n
                    },
                    ins.bottom() + 3.0,
                    axis_stroke.clone(),
                );
                if l.show_cat_axis {
                    cv.text(
                        label,
                        x,
                        ins.bottom() + 1.76,
                        TextAnchor::Middle,
                        &font,
                        TEXT_COLOR,
                        0.0,
                    );
                }
            }
        }
    }
    // Axis lines.
    if l.horizontal_bars {
        cv.line(ins.x, ins.y, ins.x, ins.bottom(), axis_stroke.clone());
    } else {
        cv.line(
            ins.x,
            ins.bottom(),
            ins.right(),
            ins.bottom(),
            axis_stroke.clone(),
        );
    }
    // Axis titles.
    let bold = font_with(&font, true);
    if let Some(t) = &l.val_title {
        if l.horizontal_bars {
            cv.text(
                t,
                ins.center_x(),
                ins.bottom() + lh + 4.0,
                TextAnchor::Middle,
                &bold,
                TEXT_COLOR,
                0.0,
            );
        } else {
            cv.text(
                t,
                7.0 + lh / 2.0,
                ins.center_y() - lh / 2.0,
                TextAnchor::Middle,
                &bold,
                TEXT_COLOR,
                90.0,
            );
        }
    }
    if let Some(t) = &l.cat_title {
        if l.horizontal_bars {
            cv.text(
                t,
                7.0 + lh / 2.0,
                ins.center_y() - lh / 2.0,
                TextAnchor::Middle,
                &bold,
                TEXT_COLOR,
                90.0,
            );
        } else {
            cv.text(
                t,
                ins.center_x(),
                ins.bottom() + lh + 4.0,
                TextAnchor::Middle,
                &bold,
                TEXT_COLOR,
                0.0,
            );
        }
    }
    let _ = (chart, base_pt);
}

fn font_with(f: &FontSpec, bold: bool) -> FontSpec {
    FontSpec { bold, ..f.clone() }
}

/// Centre of category `k` along the category axis of a column, line or area chart.
fn cat_x(l: &CartLayout, k: f64) -> f64 {
    let n = l.categories as f64;
    if l.cross_between_mid && l.categories > 1 {
        l.inside.x + k * l.inside.width / (n - 1.0)
    } else {
        l.inside.x + (k + 0.5) * l.inside.width / n
    }
}

fn draw_cartesian(
    cv: &mut Canvas<'_>,
    groups: &[Group<'_>],
    l: &CartLayout,
    base_pt: f64,
    date1904: bool,
) {
    let ins = l.inside;
    let n = l.categories.max(1) as f64;
    let label_font = font(cv.opts, base_pt, false);
    cv.items.push(DisplayItem::PushClip(Rect::new(
        ins.x - 1.0,
        ins.y - 1.0,
        ins.width + 2.0,
        ins.height + 2.0,
    )));
    for g in groups {
        let def = g.def;
        let ns = g.series.len().max(1) as f64;
        let stacked = matches!(def.grouping, Grouping::Stacked | Grouping::PercentStacked);
        let mut pos_base = vec![0.0f64; l.categories];
        let mut neg_base = vec![0.0f64; l.categories];
        let totals: Vec<f64> = (0..l.categories)
            .map(|i| {
                g.series
                    .iter()
                    .filter_map(|s| s.values.get(i).copied().flatten())
                    .map(f64::abs)
                    .sum()
            })
            .collect();
        for (si, s) in g.series.iter().enumerate() {
            match def.kind {
                GroupKind::Bar => {
                    let gap = f64::from(def.gap_width) / 100.0;
                    let overlap = if stacked {
                        1.0
                    } else {
                        f64::from(def.overlap) / 100.0
                    };
                    let band = if l.horizontal_bars {
                        ins.height / n
                    } else {
                        ins.width / n
                    };
                    let slots = if stacked { 1.0 } else { ns };
                    let bar = band / (slots - (slots - 1.0) * overlap + gap);
                    for (i, v) in s.values.iter().enumerate() {
                        let Some(v) = *v else { continue };
                        let k = if l.reversed_cats {
                            l.categories - 1 - i
                        } else {
                            i
                        } as f64;
                        let mut v0 = 0.0;
                        let mut v1 = v;
                        if stacked {
                            let scale = if l.percent && totals[i] > 0.0 {
                                1.0 / totals[i]
                            } else {
                                1.0
                            };
                            let v = v * scale;
                            if v >= 0.0 {
                                v0 = pos_base[i];
                                pos_base[i] += v;
                                v1 = pos_base[i];
                            } else {
                                v0 = neg_base[i];
                                neg_base[i] += v;
                                v1 = neg_base[i];
                            }
                        }
                        let slot = if stacked { 0.0 } else { si as f64 };
                        let off = (band - bar * (slots - (slots - 1.0) * overlap)) / 2.0
                            + bar * slot * (1.0 - overlap);
                        let (a, b) = (
                            val_to(l, v0.max(l.value.min).min(l.value.max)),
                            val_to(l, v1.clamp(l.value.min, l.value.max)),
                        );
                        let rect = if l.horizontal_bars {
                            let y = ins.bottom() - (k + 1.0) * band + off;
                            Rect::new(a.min(b), y, (a - b).abs(), bar)
                        } else {
                            let x = ins.x + k * band + off;
                            Rect::new(x, a.min(b), bar, (a - b).abs())
                        };
                        cv.rect(rect, Some(s.color), None);
                        if let Some(lb) = s.labels.as_ref().filter(|x| x.visible()) {
                            let text = label_text(lb, s, i, v, l, date1904);
                            let lh = cv.opts.measure.line_height(&label_font);
                            if l.horizontal_bars {
                                cv.text(
                                    &text,
                                    rect.right() + 2.0,
                                    rect.center_y() - lh / 2.0,
                                    TextAnchor::Start,
                                    &label_font,
                                    TEXT_COLOR,
                                    0.0,
                                );
                            } else if stacked {
                                cv.text(
                                    &text,
                                    rect.center_x(),
                                    rect.center_y() - lh / 2.0,
                                    TextAnchor::Middle,
                                    &label_font,
                                    TEXT_COLOR,
                                    0.0,
                                );
                            } else {
                                cv.text(
                                    &text,
                                    rect.center_x(),
                                    rect.y - lh - 1.0,
                                    TextAnchor::Middle,
                                    &label_font,
                                    TEXT_COLOR,
                                    0.0,
                                );
                            }
                        }
                    }
                }
                GroupKind::Line | GroupKind::Area => {
                    let mut pts: Vec<(f64, f64)> = Vec::new();
                    let mut base_pts: Vec<(f64, f64)> = Vec::new();
                    for (i, v) in s.values.iter().enumerate() {
                        let Some(v) = *v else { continue };
                        let x = cat_x(l, i as f64);
                        let mut y0 = 0.0;
                        let mut yv = v;
                        if stacked {
                            let scale = if l.percent && totals[i] > 0.0 {
                                1.0 / totals[i]
                            } else {
                                1.0
                            };
                            y0 = pos_base[i];
                            pos_base[i] += v * scale;
                            yv = pos_base[i];
                        }
                        pts.push((x, val_to(l, yv)));
                        base_pts.push((x, val_to(l, y0.max(l.value.min))));
                        if let Some(lb) = s.labels.as_ref().filter(|x| x.visible()) {
                            let text = label_text(lb, s, i, v, l, date1904);
                            let lh = cv.opts.measure.line_height(&label_font);
                            cv.text(
                                &text,
                                x,
                                val_to(l, yv) - lh - 2.0,
                                TextAnchor::Middle,
                                &label_font,
                                TEXT_COLOR,
                                0.0,
                            );
                        }
                    }
                    if pts.is_empty() {
                        continue;
                    }
                    if def.kind == GroupKind::Area {
                        let mut cmds = vec![PathCmd::MoveTo(pts[0].0, pts[0].1)];
                        cmds.extend(pts.iter().skip(1).map(|p| PathCmd::LineTo(p.0, p.1)));
                        cmds.extend(base_pts.iter().rev().map(|p| PathCmd::LineTo(p.0, p.1)));
                        cmds.push(PathCmd::Close);
                        cv.items.push(DisplayItem::Path {
                            cmds,
                            fill: Some(s.color),
                            stroke: None,
                        });
                    } else {
                        draw_polyline(cv, &pts, s);
                        if s.series.marker.as_deref() != Some("none") {
                            for p in &pts {
                                marker(cv, p.0, p.1, s.color);
                            }
                        }
                    }
                }
                GroupKind::Scatter => {
                    let xs = l.x_scale.unwrap_or(l.value);
                    let mut pts = Vec::new();
                    for (i, v) in s.values.iter().enumerate() {
                        let x = s.xs.get(i).copied().flatten().unwrap_or((i + 1) as f64);
                        let Some(v) = *v else { continue };
                        pts.push((
                            ins.x + (x - xs.min) / (xs.max - xs.min) * ins.width,
                            val_to(l, v),
                        ));
                    }
                    let lines = !s.series.line_none
                        && matches!(
                            def.scatter_style.as_deref(),
                            Some("line" | "lineMarker" | "smooth" | "smoothMarker")
                        )
                        && (s.line_color.is_some()
                            || s.series.line_width_pt.is_some()
                            || def.scatter_style.as_deref() == Some("line"));
                    if lines {
                        draw_polyline(cv, &pts, s);
                    }
                    if s.series.marker.as_deref() != Some("none") {
                        for p in &pts {
                            marker(cv, p.0, p.1, s.color);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    cv.items.push(DisplayItem::PopClip);
}

fn draw_polyline(cv: &mut Canvas<'_>, pts: &[(f64, f64)], s: &Resolved) {
    if pts.len() < 2 {
        return;
    }
    let mut cmds = vec![PathCmd::MoveTo(pts[0].0, pts[0].1)];
    cmds.extend(pts.iter().skip(1).map(|p| PathCmd::LineTo(p.0, p.1)));
    cv.items.push(DisplayItem::Path {
        cmds,
        fill: None,
        stroke: Some(Stroke::solid(
            s.line_color.unwrap_or(s.color),
            s.series.line_width_pt.unwrap_or(2.25),
        )),
    });
}

fn marker(cv: &mut Canvas<'_>, x: f64, y: f64, color: Rgba) {
    cv.items.push(DisplayItem::Path {
        cmds: circle(x, y, 2.5),
        fill: Some(color),
        stroke: None,
    });
}

fn circle(cx: f64, cy: f64, r: f64) -> Vec<PathCmd> {
    arc_sector(cx, cy, r, 0.0, std::f64::consts::TAU, false)
}

/// A cubic-Bezier circular sector (or full circle when the sweep is a full turn). Angles in
/// radians measured clockwise from 12 o'clock.
fn arc_sector(cx: f64, cy: f64, r: f64, start: f64, sweep: f64, wedge: bool) -> Vec<PathCmd> {
    let pt = |a: f64| (cx + r * a.sin(), cy - r * a.cos());
    let segments = (sweep.abs() / (std::f64::consts::FRAC_PI_2))
        .ceil()
        .max(1.0) as usize;
    let step = sweep / segments as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let (sx, sy) = pt(start);
    let mut cmds = Vec::new();
    if wedge {
        cmds.push(PathCmd::MoveTo(cx, cy));
        cmds.push(PathCmd::LineTo(sx, sy));
    } else {
        cmds.push(PathCmd::MoveTo(sx, sy));
    }
    for i in 0..segments {
        let a0 = start + step * i as f64;
        let a1 = a0 + step;
        let (x0, y0) = pt(a0);
        let (x1, y1) = pt(a1);
        // Tangent for a clockwise-from-top parameterisation.
        let (t0x, t0y) = (a0.cos(), a0.sin());
        let (t1x, t1y) = (a1.cos(), a1.sin());
        cmds.push(PathCmd::CubicTo(
            x0 + k * r * t0x,
            y0 + k * r * t0y,
            x1 - k * r * t1x,
            y1 - k * r * t1y,
            x1,
            y1,
        ));
    }
    cmds.push(PathCmd::Close);
    cmds
}

fn label_text(
    lb: &DataLabels,
    s: &Resolved,
    i: usize,
    v: f64,
    l: &CartLayout,
    date1904: bool,
) -> String {
    let sep = lb.separator.clone().unwrap_or_else(|| ", ".to_owned());
    let mut parts = Vec::new();
    if lb.show_series {
        parts.push(s.name.clone());
    }
    if lb.show_category
        && let Some(c) = l.cat_labels.get(i)
    {
        parts.push(c.clone());
    }
    if lb.show_value {
        let code = lb
            .num_fmt
            .clone()
            .or_else(|| s.format_code.clone())
            .unwrap_or_else(|| "General".to_owned());
        parts.push(format_number(v, &code, date1904));
    }
    parts.join(&sep)
}

fn draw_pie(cv: &mut Canvas<'_>, g: &Group<'_>, area: Rect, base_pt: f64, date1904: bool) {
    let Some(s) = g.series.first() else { return };
    let total: f64 = s.values.iter().flatten().filter(|v| **v > 0.0).sum();
    if total <= 0.0 {
        return;
    }
    let r = area.width.min(area.height) / 2.0;
    let (cx, cy) = (area.center_x(), area.center_y());
    let hole = if g.def.kind == GroupKind::Doughnut {
        r * f64::from(g.def.hole_size.clamp(10, 90)) / 100.0
    } else {
        0.0
    };
    let mut angle = f64::from(g.def.first_slice_angle).to_radians();
    let font = font(cv.opts, base_pt, false);
    let white = Stroke::solid(Rgba::WHITE, 0.75);
    for (i, v) in s.values.iter().enumerate() {
        let Some(v) = v.filter(|v| *v > 0.0) else {
            continue;
        };
        let sweep = v / total * std::f64::consts::TAU;
        let color = point_color(s, i);
        let cmds = if hole > 0.0 {
            let mut c = arc_sector(cx, cy, r, angle, sweep, false);
            c.pop();
            // Inner edge back.
            let mut inner = arc_sector(cx, cy, hole, angle + sweep, -sweep, false);
            if let Some(PathCmd::MoveTo(x, y)) = inner.first().copied() {
                c.push(PathCmd::LineTo(x, y));
            }
            inner.remove(0);
            c.extend(inner);
            c
        } else {
            arc_sector(cx, cy, r, angle, sweep, true)
        };
        cv.items.push(DisplayItem::Path {
            cmds,
            fill: Some(color),
            stroke: Some(white.clone()),
        });
        if let Some(lb) = s.labels.as_ref().filter(|x| x.visible()) {
            let mid = angle + sweep / 2.0;
            let mut parts = Vec::new();
            if lb.show_series {
                parts.push(s.name.clone());
            }
            if lb.show_category
                && let Some(c) = s.cats.get(i)
            {
                parts.push(c.clone());
            }
            if lb.show_value {
                parts.push(format_number(
                    v,
                    lb.num_fmt.as_deref().unwrap_or("General"),
                    date1904,
                ));
            }
            if lb.show_percent {
                parts.push(format_number(v / total, "0%", date1904));
            }
            let text = parts.join(lb.separator.as_deref().unwrap_or(", "));
            let lh = cv.opts.measure.line_height(&font);
            let rr = r + lh * 0.6;
            let (tx, ty) = (cx + rr * mid.sin(), cy - rr * mid.cos());
            let anchor = if mid.sin() > 0.3 {
                TextAnchor::Start
            } else if mid.sin() < -0.3 {
                TextAnchor::End
            } else {
                TextAnchor::Middle
            };
            cv.text(&text, tx, ty - lh / 2.0, anchor, &font, TEXT_COLOR, 0.0);
        }
        angle += sweep;
    }
}
