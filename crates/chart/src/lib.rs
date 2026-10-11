//! Excel charts: a lossless DrawingML chart model, Excel-exact automatic axis scaling and a
//! backend-neutral display list.
//!
//! * [`ChartSpace::parse`] reads `xl/charts/chartN.xml`; unknown markup stays in the XML tree
//!   and [`ChartSpace::to_bytes`] writes it back.
//! * [`sheet_charts`] lists the charts of a worksheet with their anchor size in points.
//! * [`resolve`] reads series values from a [`CellSource`] (the workbook's stored values, or
//!   the calculation engine once wired) and falls back to the chart's own cache.
//! * [`render`] produces a [`DisplayList`] at a given size. [`axis`] holds the automatic
//!   scaling rules measured from Excel.
//!
//! See `docs/architecture/charts.md`.

pub mod axis;
mod build;
mod data;
mod display;
mod edit;
mod error;
mod layout;
mod model;
mod style;

pub use build::{BuildKind, BuildSeries, ChartBuilder};
pub use data::{AreaRef, CellLookup, CellSource, NoCells, WorkbookCells, parse_areas, resolve};
pub use display::{
    ApproxMeasure, DisplayItem, DisplayList, FontSpec, PathCmd, Rect, Rgba, Stroke, TextAnchor,
    TextMeasure, TextRun,
};
pub use edit::ScaleEdit;
pub use error::ChartError;
pub use layout::{AREA_INSET, DEFAULT_PALETTE, RenderOptions, render, render_preview};
pub use model::{
    Axis, AxisKind, AxisPos, BarDirection, Chart, ChartGroup, ChartSpace, DEFAULT_TEXT_PT,
    DEFAULT_TITLE_PT, DataLabels, DataRef, ErrorBars, GroupKind, Grouping, Legend, LegendPos,
    ManualLayout, PlotArea, PointValue, Rgb, Series, Title, Trendline, TrendlineKind,
};
pub use style::{ChartStyle, ColorStyle, StyleColor};

use open365_xlsx::{GridMetrics, Workbook};

/// A chart found on a worksheet.
#[derive(Debug, Clone)]
pub struct SheetChart {
    /// Name of the drawing object (`Chart 1`).
    pub name: String,
    /// Anchor rectangle on the sheet in points (x, y, width, height).
    pub anchor: open365_xlsx::RectPt,
    pub space: ChartSpace,
}

/// Reads every chart anchored on worksheet `sheet`. Charts whose part is missing or does not
/// parse are skipped. Sizes use the 96 dpi pixel grid of the workbook's default font.
pub fn sheet_charts(wb: &Workbook, sheet: usize) -> Vec<SheetChart> {
    let Some(ws) = wb.sheet(sheet) else {
        return Vec::new();
    };
    let metrics = GridMetrics::for_styles(wb.styles(), 96);
    wb.sheet_charts(sheet)
        .into_iter()
        .filter_map(|dc| {
            let part = wb.package().part(dc.chart_part.as_str())?;
            let space = ChartSpace::parse(part.data()).ok()?;
            Some(SheetChart {
                name: dc.name,
                anchor: dc.anchor.rect_pt(ws, metrics),
                space,
            })
        })
        .collect()
}
