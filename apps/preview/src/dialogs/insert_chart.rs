//! Insert Chart: which chart types the Insert > Charts menus offer and how Excel reads the
//! selected cells into series (headers, categories, orientation).

use open365_chart::BuildKind;

use crate::sheet::sheet_ext::insert::Kind;

/// The chart type of a menu item (`insert.charts.<control>.<item>`); `None` for an item no chart
/// engine exists for (waterfall, funnel, surface, bubble ...) and for unknown items.
/// An empty item is the control's own default (what its button inserts).
#[must_use]
pub fn kind_of(control: &str, item: &str) -> Option<BuildKind> {
    let item = if item == "run" { "" } else { item };
    Some(match (control, item) {
        ("recommended-charts", _) | ("insert-column-or-bar-chart", "" | "clustered-column") => {
            BuildKind::Column
        }
        ("insert-column-or-bar-chart", "stacked-column") => BuildKind::ColumnStacked,
        ("insert-column-or-bar-chart", "100-stacked-column") => BuildKind::ColumnPercent,
        ("insert-column-or-bar-chart", "clustered-bar") => BuildKind::Bar,
        ("insert-column-or-bar-chart", "stacked-bar") => BuildKind::BarStacked,
        ("insert-column-or-bar-chart", "100-stacked-bar") => BuildKind::BarPercent,
        ("insert-line-or-area-chart", "" | "line") => BuildKind::Line,
        ("insert-line-or-area-chart", "line-with-markers") => BuildKind::LineMarker,
        ("insert-line-or-area-chart", "area") => BuildKind::Area,
        ("insert-line-or-area-chart", "stacked-area") => BuildKind::AreaStacked,
        ("insert-line-or-area-chart", "100-stacked-area") => BuildKind::AreaPercent,
        ("insert-pie-or-doughnut-chart", "" | "pie") => BuildKind::Pie,
        ("insert-pie-or-doughnut-chart", "doughnut") => BuildKind::Doughnut,
        ("insert-scatter-x-y-or-bubble-chart", "" | "scatter") => BuildKind::Scatter,
        ("insert-waterfall-funnel-stock-surface-or-radar-chart", "" | "radar") => BuildKind::Radar,
        ("insert-waterfall-funnel-stock-surface-or-radar-chart", "stock") => BuildKind::Stock,
        _ => return None,
    })
}

/// How the selected cells become chart series. Positions are in the oriented grid: `k` runs along
/// the categories (rows when series are in columns), a line is one series (a column then).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartPlan {
    /// Series are in rows (the selection is wider than tall).
    pub transposed: bool,
    /// The position of the header (series names) line, when there is one.
    pub header_k: Option<usize>,
    /// The first data position.
    pub data_k0: usize,
    /// The line holding the categories (the X values of a scatter chart).
    pub cat_line: Option<usize>,
    pub series_lines: Vec<usize>,
    /// Positions per line.
    pub count: usize,
}

/// Excel's reading of a block of cells: `kinds[row][col]`.
#[must_use]
pub fn plan(kinds: &[Vec<Kind>], scatter: bool) -> Option<ChartPlan> {
    let rows = kinds.len();
    let cols = kinds.first().map_or(0, Vec::len);
    if rows == 0 || cols == 0 {
        return None;
    }
    let transposed = cols > rows;
    let (n_k, n_l) = if transposed {
        (cols, rows)
    } else {
        (rows, cols)
    };
    let at = |k: usize, l: usize| if transposed { kinds[l][k] } else { kinds[k][l] };
    let top_left_empty = n_k > 1 && n_l > 1 && at(0, 0) == Kind::Empty;
    let first_line_has_numbers = (0..n_l).any(|l| at(0, l) == Kind::Number);
    let rest_has_numbers = (1..n_k).any(|k| (0..n_l).any(|l| at(k, l) == Kind::Number));
    let header = n_k > 1 && (top_left_empty || (!first_line_has_numbers && rest_has_numbers));
    let data_k0 = usize::from(header);
    let first_text = (data_k0..n_k).any(|k| at(k, 0) == Kind::Text);
    let first_number = (data_k0..n_k).any(|k| at(k, 0) == Kind::Number);
    let cat = n_l > 1
        && if scatter {
            first_number && !first_text
        } else {
            top_left_empty || (first_text && !first_number)
        };
    let series_lines: Vec<usize> = (usize::from(cat)..n_l)
        .filter(|l| (data_k0..n_k).any(|k| at(k, *l) == Kind::Number))
        .collect();
    if series_lines.is_empty() {
        return None;
    }
    Some(ChartPlan {
        transposed,
        header_k: header.then_some(0),
        data_k0,
        cat_line: cat.then_some(0),
        series_lines,
        count: n_k - data_k0,
    })
}
