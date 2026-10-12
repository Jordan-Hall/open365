//! Creates a new chart part from a plain description. The XML follows the shape Excel itself
//! writes (series with references and caches, two axes with fixed ids), so a built chart opens
//! in Excel and reads back through [`ChartSpace::parse`].

use std::fmt::Write as _;

use crate::error::ChartError;
use crate::model::{ChartSpace, LegendPos};

/// Kind of chart to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    Column,
    ColumnStacked,
    ColumnPercent,
    Bar,
    BarStacked,
    BarPercent,
    Line,
    LineMarker,
    Area,
    AreaStacked,
    AreaPercent,
    Scatter,
    Bubble,
    Pie,
    Doughnut,
    Radar,
    Stock,
}

#[derive(Debug, Clone, Default)]
pub struct BuildSeries {
    pub name: String,
    pub values: Vec<f64>,
    /// X values of scatter and bubble series.
    pub xs: Vec<f64>,
    /// Bubble sizes.
    pub sizes: Vec<f64>,
}

/// Dates on the category axis: a time-scale (`c:dateAx`) axis over numeric categories.
#[derive(Debug, Clone)]
pub struct BuildDates {
    /// Date serials, one per category.
    pub serials: Vec<f64>,
    /// Number format of the category cells.
    pub cell_format: String,
    /// Tick label format; `None` follows the cells.
    pub axis_format: Option<String>,
    /// `c:baseTimeUnit` days; `false` leaves the base unit automatic.
    pub base_unit_days: bool,
}

/// Data labels of every series.
#[derive(Debug, Clone, Default)]
pub struct BuildLabels {
    /// `c:dLblPos` value, for example `outEnd`; `None` leaves Excel's default.
    pub position: Option<String>,
    pub show_value: bool,
    pub show_category: bool,
    pub show_percent: bool,
    /// Label text size in points.
    pub font_pt: Option<f64>,
    /// Label number format; `None` follows the cells.
    pub number_format: Option<String>,
}

/// Everything needed to make a chart part.
#[derive(Debug, Clone)]
pub struct ChartBuilder {
    pub kind: BuildKind,
    pub categories: Vec<String>,
    pub series: Vec<BuildSeries>,
    pub title: Option<String>,
    pub legend: Option<LegendPos>,
    pub category_title: Option<String>,
    pub value_title: Option<String>,
    /// Logarithmic value axis with this base.
    pub log_base: Option<f64>,
    /// Sheet the references point at.
    pub sheet: String,
    /// Chart text size in points and font family.
    pub font_pt: f64,
    pub font_family: String,
    /// Number format of the value cells (`c:formatCode` of the value cache).
    pub cell_format: Option<String>,
    /// Number format of the value axis; `None` follows the cells.
    pub value_axis_format: Option<String>,
    pub labels: Option<BuildLabels>,
    pub dates: Option<BuildDates>,
    /// Fixed inner plot rectangle as fractions `[x, y, w, h]` of the chart area
    /// (`c:layout` with `layoutTarget` inner).
    pub inner_layout: Option<[f64; 4]>,
    /// Horizontal bar charts list the first category at the top (`c:orientation` maxMin).
    /// Excel's own default puts it at the bottom.
    pub bar_first_at_top: bool,
}

impl ChartBuilder {
    pub fn new(kind: BuildKind) -> Self {
        Self {
            kind,
            categories: Vec::new(),
            series: Vec::new(),
            title: None,
            legend: Some(LegendPos::Right),
            category_title: None,
            value_title: None,
            log_base: None,
            sheet: "Sheet1".to_owned(),
            font_pt: 10.0,
            font_family: "Calibri".to_owned(),
            cell_format: None,
            value_axis_format: None,
            labels: None,
            dates: None,
            inner_layout: None,
            bar_first_at_top: true,
        }
    }

    /// The part as XML text.
    pub fn to_xml(&self) -> String {
        let mut x = String::new();
        x.push_str(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<c:chartSpace \
             xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" \
             xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
             xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
             <c:date1904 val=\"0\"/><c:roundedCorners val=\"0\"/><c:chart>",
        );
        match &self.title {
            Some(t) => {
                let _ = write!(
                    x,
                    "<c:title>{}<c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>",
                    rich(t, Some(1800), true)
                );
            }
            None => x.push_str("<c:autoTitleDeleted val=\"1\"/>"),
        }
        match self.inner_layout {
            Some([lx, ly, lw, lh]) => {
                let _ = write!(
                    x,
                    "<c:plotArea><c:layout><c:manualLayout><c:layoutTarget val=\"inner\"/><c:xMode val=\"edge\"/><c:yMode val=\"edge\"/><c:x val=\"{lx}\"/><c:y val=\"{ly}\"/><c:w val=\"{lw}\"/><c:h val=\"{lh}\"/></c:manualLayout></c:layout>"
                );
            }
            None => x.push_str("<c:plotArea><c:layout/>"),
        }
        self.plot(&mut x);
        x.push_str("</c:plotArea>");
        if let Some(l) = self.legend {
            let pos = match l {
                LegendPos::Right => "r",
                LegendPos::Left => "l",
                LegendPos::Top => "t",
                LegendPos::Bottom => "b",
                LegendPos::TopRight => "tr",
            };
            let _ = write!(
                x,
                "<c:legend><c:legendPos val=\"{pos}\"/><c:overlay val=\"0\"/></c:legend>"
            );
        }
        x.push_str("<c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"gap\"/></c:chart>");
        let _ = write!(
            x,
            "<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"{}\"><a:latin typeface=\"{}\"/></a:defRPr></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr>",
            (self.font_pt * 100.0).round() as i64,
            escape(&self.font_family)
        );
        x.push_str("</c:chartSpace>");
        x
    }

    /// The part as bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_xml().into_bytes()
    }

    /// The built chart as a model.
    pub fn build(&self) -> Result<ChartSpace, ChartError> {
        ChartSpace::parse(&self.to_bytes())
    }

    fn plot(&self, x: &mut String) {
        use BuildKind as K;
        let n = self.categories.len().max(
            self.series
                .iter()
                .map(|s| s.values.len())
                .max()
                .unwrap_or(0),
        );
        let (open, close, axes): (String, &str, u8) = match self.kind {
            K::Column
            | K::ColumnStacked
            | K::ColumnPercent
            | K::Bar
            | K::BarStacked
            | K::BarPercent => {
                let bar = matches!(self.kind, K::Bar | K::BarStacked | K::BarPercent);
                let grouping = match self.kind {
                    K::ColumnStacked | K::BarStacked => "stacked",
                    K::ColumnPercent | K::BarPercent => "percentStacked",
                    _ => "clustered",
                };
                (
                    format!(
                        "<c:barChart><c:barDir val=\"{}\"/><c:grouping val=\"{grouping}\"/><c:varyColors val=\"0\"/>",
                        if bar { "bar" } else { "col" }
                    ),
                    if grouping == "clustered" {
                        "<c:gapWidth val=\"219\"/><c:overlap val=\"-27\"/>%AX%</c:barChart>"
                    } else {
                        "<c:gapWidth val=\"150\"/><c:overlap val=\"100\"/>%AX%</c:barChart>"
                    },
                    if bar { 2 } else { 1 },
                )
            }
            K::Line | K::LineMarker => (
                "<c:lineChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/>".to_owned(),
                "<c:marker val=\"1\"/>%AX%</c:lineChart>",
                1,
            ),
            K::Stock => (
                "<c:stockChart>".to_owned(),
                "<c:hiLowLines/>%AX%</c:stockChart>",
                1,
            ),
            K::Area | K::AreaStacked | K::AreaPercent => (
                format!(
                    "<c:areaChart><c:grouping val=\"{}\"/><c:varyColors val=\"0\"/>",
                    match self.kind {
                        K::AreaStacked => "stacked",
                        K::AreaPercent => "percentStacked",
                        _ => "standard",
                    }
                ),
                "%AX%</c:areaChart>",
                3,
            ),
            K::Scatter => (
                "<c:scatterChart><c:scatterStyle val=\"lineMarker\"/><c:varyColors val=\"0\"/>"
                    .to_owned(),
                "%AX%</c:scatterChart>",
                4,
            ),
            K::Bubble => (
                "<c:bubbleChart><c:varyColors val=\"0\"/>".to_owned(),
                "<c:bubbleScale val=\"100\"/><c:showNegBubbles val=\"0\"/>%AX%</c:bubbleChart>",
                4,
            ),
            K::Pie => (
                "<c:pieChart><c:varyColors val=\"1\"/>".to_owned(),
                "<c:firstSliceAng val=\"0\"/></c:pieChart>",
                0,
            ),
            K::Doughnut => (
                "<c:doughnutChart><c:varyColors val=\"1\"/>".to_owned(),
                "<c:firstSliceAng val=\"0\"/><c:holeSize val=\"75\"/></c:doughnutChart>",
                0,
            ),
            K::Radar => (
                "<c:radarChart><c:radarStyle val=\"marker\"/><c:varyColors val=\"0\"/>".to_owned(),
                "%AX%</c:radarChart>",
                1,
            ),
        };
        x.push_str(&open);
        let xy = matches!(self.kind, K::Scatter | K::Bubble);
        for (i, s) in self.series.iter().enumerate() {
            let col = col_letter(i + 2);
            let _ = write!(x, "<c:ser><c:idx val=\"{i}\"/><c:order val=\"{i}\"/>");
            let _ = write!(
                x,
                "<c:tx><c:strRef><c:f>{}!${col}$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>{}</c:v></c:pt></c:strCache></c:strRef></c:tx>",
                self.sheet,
                escape(&s.name)
            );
            if self.kind == K::Line {
                x.push_str("<c:marker><c:symbol val=\"none\"/></c:marker>");
            }
            if let Some(l) = &self.labels {
                x.push_str(&dlbls(l));
            }
            if xy {
                let xs = if s.xs.is_empty() {
                    (1..=n).map(|v| v as f64).collect()
                } else {
                    s.xs.clone()
                };
                let _ = write!(x, "<c:xVal>{}</c:xVal>", num_ref(&self.sheet, "A", &xs));
                let _ = write!(
                    x,
                    "<c:yVal>{}</c:yVal>",
                    num_ref_fmt(
                        &self.sheet,
                        &col,
                        &s.values,
                        self.cell_format.as_deref().unwrap_or("General")
                    )
                );
                if self.kind == K::Bubble {
                    let sz = if s.sizes.is_empty() {
                        vec![1.0; s.values.len()]
                    } else {
                        s.sizes.clone()
                    };
                    let _ = write!(
                        x,
                        "<c:bubbleSize>{}</c:bubbleSize><c:bubble3D val=\"0\"/>",
                        num_ref(&self.sheet, &col_letter(i + 3), &sz)
                    );
                }
            } else {
                if let Some(d) = &self.dates {
                    let _ = write!(
                        x,
                        "<c:cat>{}</c:cat>",
                        num_ref_fmt(&self.sheet, "A", &d.serials, &d.cell_format)
                    );
                } else if !self.categories.is_empty() {
                    let _ = write!(
                        x,
                        "<c:cat>{}</c:cat>",
                        str_ref(&self.sheet, "A", &self.categories)
                    );
                }
                let _ = write!(
                    x,
                    "<c:val>{}</c:val>",
                    num_ref_fmt(
                        &self.sheet,
                        &col,
                        &s.values,
                        self.cell_format.as_deref().unwrap_or("General")
                    )
                );
            }
            if matches!(self.kind, K::Line | K::LineMarker | K::Radar) {
                x.push_str("<c:smooth val=\"0\"/>");
            }
            x.push_str("</c:ser>");
        }
        let ax = if axes == 0 {
            ""
        } else {
            "<c:axId val=\"111\"/><c:axId val=\"222\"/>"
        };
        x.push_str(&close.replace("%AX%", ax));
        let horizontal = axes == 2;
        let (cat_pos, val_pos) = if horizontal { ("l", "b") } else { ("b", "l") };
        let between = if axes == 3 { "midCat" } else { "between" };
        let log = self
            .log_base
            .map_or(String::new(), |b| format!("<c:logBase val=\"{b}\"/>"));
        match axes {
            1..=3 => {
                let title = self.category_title.as_deref().map_or(String::new(), |t| {
                    format!(
                        "<c:title>{}<c:overlay val=\"0\"/></c:title>",
                        rich(t, None, false)
                    )
                });
                if let Some(d) = &self.dates {
                    let _ = write!(
                        x,
                        "<c:dateAx><c:axId val=\"111\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/>{title}<c:numFmt formatCode=\"{}\" sourceLinked=\"{}\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"222\"/><c:crosses val=\"autoZero\"/><c:auto val=\"1\"/><c:lblOffset val=\"100\"/>{}</c:dateAx>",
                        escape(d.axis_format.as_deref().unwrap_or(&d.cell_format)),
                        u8::from(d.axis_format.is_none()),
                        if d.base_unit_days {
                            "<c:baseTimeUnit val=\"days\"/>"
                        } else {
                            ""
                        }
                    );
                } else {
                    let _ = write!(
                        x,
                        "<c:catAx><c:axId val=\"111\"/><c:scaling><c:orientation val=\"{}\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{cat_pos}\"/>{}<c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"222\"/><c:crosses val=\"autoZero\"/><c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/><c:noMultiLvlLbl val=\"0\"/></c:catAx>",
                        if horizontal && self.bar_first_at_top {
                            "maxMin"
                        } else {
                            "minMax"
                        },
                        title
                    );
                }
                let _ = write!(
                    x,
                    "<c:valAx><c:axId val=\"222\"/><c:scaling>{log}<c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{val_pos}\"/><c:majorGridlines/>{}<c:numFmt formatCode=\"{}\" sourceLinked=\"{}\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"111\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"{between}\"/></c:valAx>",
                    self.value_title
                        .as_deref()
                        .map_or(String::new(), |t| format!(
                            "<c:title>{}<c:overlay val=\"0\"/></c:title>",
                            rich(t, None, false)
                        )),
                    escape(
                        if matches!(self.kind, K::ColumnPercent | K::BarPercent | K::AreaPercent) {
                            "0%"
                        } else {
                            self.value_axis_format.as_deref().unwrap_or("General")
                        }
                    ),
                    u8::from(
                        !matches!(self.kind, K::ColumnPercent | K::BarPercent | K::AreaPercent)
                            && self.value_axis_format.is_none()
                    ),
                );
            }
            4 => {
                let _ = write!(
                    x,
                    "<c:valAx><c:axId val=\"111\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/>{}<c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"222\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"midCat\"/></c:valAx>",
                    self.category_title
                        .as_deref()
                        .map_or(String::new(), |t| format!(
                            "<c:title>{}<c:overlay val=\"0\"/></c:title>",
                            rich(t, None, false)
                        ))
                );
                let _ = write!(
                    x,
                    "<c:valAx><c:axId val=\"222\"/><c:scaling>{log}<c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/><c:majorGridlines/>{}<c:numFmt formatCode=\"{}\" sourceLinked=\"{}\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"111\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"midCat\"/></c:valAx>",
                    self.value_title
                        .as_deref()
                        .map_or(String::new(), |t| format!(
                            "<c:title>{}<c:overlay val=\"0\"/></c:title>",
                            rich(t, None, false)
                        )),
                    escape(self.value_axis_format.as_deref().unwrap_or("General")),
                    u8::from(self.value_axis_format.is_none())
                );
            }
            _ => {}
        }
    }
}

fn rich(text: &str, size: Option<u32>, bold: bool) -> String {
    let sz = size.map_or(String::new(), |s| format!(" sz=\"{s}\""));
    format!(
        "<c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr{sz} b=\"{}\"/></a:pPr><a:r><a:rPr lang=\"en-US\"{sz} b=\"{}\"/><a:t>{}</a:t></a:r></a:p></c:rich></c:tx>",
        u8::from(bold),
        u8::from(bold),
        escape(text)
    )
}

fn col_letter(n: usize) -> String {
    let mut n = n;
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    s
}

fn num_ref(sheet: &str, col: &str, v: &[f64]) -> String {
    num_ref_fmt(sheet, col, v, "General")
}

fn dlbls(l: &BuildLabels) -> String {
    let mut s = String::from("<c:dLbls>");
    if let Some(f) = &l.number_format {
        let _ = write!(
            s,
            "<c:numFmt formatCode=\"{}\" sourceLinked=\"0\"/>",
            escape(f)
        );
    }
    if let Some(pt) = l.font_pt {
        let _ = write!(
            s,
            "<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"{}\"/></a:pPr><a:endParaRPr lang=\"en-US\"/></a:p></c:txPr>",
            (pt * 100.0).round() as i64
        );
    }
    if let Some(p) = &l.position {
        let _ = write!(s, "<c:dLblPos val=\"{p}\"/>");
    }
    let _ = write!(
        s,
        "<c:showLegendKey val=\"0\"/><c:showVal val=\"{}\"/><c:showCatName val=\"{}\"/><c:showSerName val=\"0\"/><c:showPercent val=\"{}\"/><c:showBubbleSize val=\"0\"/></c:dLbls>",
        u8::from(l.show_value),
        u8::from(l.show_category),
        u8::from(l.show_percent)
    );
    s
}

fn num_ref_fmt(sheet: &str, col: &str, v: &[f64], format: &str) -> String {
    let mut s = format!(
        "<c:numRef><c:f>{sheet}!${col}$2:${col}${}</c:f><c:numCache><c:formatCode>{}</c:formatCode><c:ptCount val=\"{}\"/>",
        v.len() + 1,
        escape(format),
        v.len()
    );
    for (i, n) in v.iter().enumerate() {
        let mut t = String::new();
        open365_xlsx::format_number(*n, &mut t);
        let _ = write!(s, "<c:pt idx=\"{i}\"><c:v>{t}</c:v></c:pt>");
    }
    s.push_str("</c:numCache></c:numRef>");
    s
}

fn str_ref(sheet: &str, col: &str, v: &[String]) -> String {
    let mut s = format!(
        "<c:strRef><c:f>{sheet}!${col}$2:${col}${}</c:f><c:strCache><c:ptCount val=\"{}\"/>",
        v.len() + 1,
        v.len()
    );
    for (i, t) in v.iter().enumerate() {
        let _ = write!(s, "<c:pt idx=\"{i}\"><c:v>{}</c:v></c:pt>", escape(t));
    }
    s.push_str("</c:strCache></c:strRef>");
    s
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
