//! PivotTable table styles: the built-in `PivotStyleLight/Medium/Dark` definitions (measured
//! from Excel's object model into `builtin_styles.txt` by `tools/office/make-pivot-styledefs.ps1`)
//! and the rules that decide which style element formats which cell of the table.

mod custom;
mod formats;

pub use custom::CustomStyles;
pub use formats::{Dxfs, UserFormat, apply_user_formats};

use std::collections::HashMap;
use std::sync::OnceLock;

use open365_xlsx::{Rgb, Theme, apply_tint};

use crate::compute::{LineInfo, LineKind, PivotGrid};
use crate::model::PivotTable;

/// `xl/pivotTables` `pivotTableStyleInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleInfo {
    pub name: String,
    pub show_row_headers: bool,
    pub show_col_headers: bool,
    pub show_row_stripes: bool,
    pub show_col_stripes: bool,
    pub show_last_column: bool,
}

/// Table style element numbers (Excel's `XlTableStyleElementType`).
pub mod element {
    pub const WHOLE_TABLE: usize = 0;
    pub const HEADER_ROW: usize = 1;
    pub const TOTAL_ROW: usize = 2;
    pub const FIRST_COLUMN: usize = 3;
    pub const LAST_COLUMN: usize = 4;
    pub const ROW_STRIPE_1: usize = 5;
    pub const ROW_STRIPE_2: usize = 6;
    pub const COL_STRIPE_1: usize = 7;
    pub const COL_STRIPE_2: usize = 8;
    pub const FIRST_HEADER_CELL: usize = 9;
    pub const LAST_HEADER_CELL: usize = 10;
    pub const FIRST_TOTAL_CELL: usize = 11;
    pub const LAST_TOTAL_CELL: usize = 12;
    pub const SUBTOTAL_COLUMN_1: usize = 13;
    pub const SUBTOTAL_ROW_1: usize = 16;
    pub const ROW_SUBHEADING_1: usize = 23;
    pub const COLUMN_SUBHEADING_1: usize = 20;
    pub const BLANK_ROW: usize = 19;
    pub const PAGE_FIELD_LABELS: usize = 26;
    pub const PAGE_FIELD_VALUES: usize = 27;
    pub const COUNT: usize = 28;
}

/// A colour of a style element: theme index (OOXML order) with tint, or RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
enum StyleColor {
    Theme(u32, f64),
    Rgb(Rgb),
}

impl StyleColor {
    fn parse(s: &str) -> Option<Self> {
        if let Some(t) = s.strip_prefix('t') {
            let (i, tint) = t.split_once(':')?;
            return Some(Self::Theme(i.parse().ok()?, tint.parse().ok()?));
        }
        let v = u32::from_str_radix(s.strip_prefix('r')?, 16).ok()?;
        Some(Self::Rgb(Rgb::new(
            (v >> 16) as u8,
            (v >> 8) as u8,
            v as u8,
        )))
    }

    fn resolve(self, theme: &Theme) -> Rgb {
        match self {
            Self::Rgb(c) => c,
            Self::Theme(i, tint) => apply_tint(theme.color(i).unwrap_or(Rgb::BLACK), tint),
        }
    }
}

/// One border of a style element (`LineStyle`, `Weight`, colour).
#[derive(Debug, Clone, Copy, PartialEq)]
struct EdgeDef {
    line: i32,
    weight: i32,
    color: StyleColor,
}

/// One element of a built-in style. Unset properties stay `None`.
#[derive(Debug, Clone, Default, PartialEq)]
struct ElementDef {
    bold: Option<bool>,
    italic: Option<bool>,
    font: Option<StyleColor>,
    fill: Option<StyleColor>,
    /// left, top, bottom, right, inside vertical, inside horizontal.
    borders: [Option<EdgeDef>; 6],
    stripe: usize,
}

type StyleDef = Vec<Option<ElementDef>>;

fn parse_edge(s: &str) -> Option<EdgeDef> {
    let mut it = s.splitn(3, ':');
    let line = it.next()?.parse().ok()?;
    let weight = it.next()?.parse().ok()?;
    Some(EdgeDef {
        line,
        weight,
        color: StyleColor::parse(it.next()?)?,
    })
}

fn builtin_table() -> &'static HashMap<String, StyleDef> {
    static TABLE: OnceLock<HashMap<String, StyleDef>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut map: HashMap<String, StyleDef> = HashMap::new();
        for line in include_str!("builtin_styles.txt").lines() {
            let f: Vec<&str> = line.split('|').collect();
            if f.len() < 14 {
                continue;
            }
            let Ok(el) = f[1].parse::<usize>() else {
                continue;
            };
            if el >= element::COUNT {
                continue;
            }
            let flag = |s: &str| match s {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            };
            let mut def = ElementDef {
                bold: flag(f[3]),
                italic: flag(f[4]),
                font: StyleColor::parse(f[5]),
                fill: if f[6].is_empty() {
                    None
                } else {
                    StyleColor::parse(f[7])
                },
                stripe: f[2].parse().unwrap_or(1),
                ..ElementDef::default()
            };
            for (k, b) in f[8..14].iter().enumerate() {
                def.borders[k] = parse_edge(b);
            }
            map.entry(f[0].to_owned())
                .or_insert_with(|| vec![None; element::COUNT])[el] = Some(def);
        }
        map
    })
}

/// Names of the built-in pivot styles the engine knows.
pub fn builtin_style_names() -> Vec<&'static str> {
    let mut v: Vec<&str> = builtin_table().keys().map(String::as_str).collect();
    v.sort_unstable();
    v
}

/// A border line as Excel reports it (`XlLineStyle`, `XlBorderWeight`, colour).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub line: i32,
    pub weight: i32,
    pub color: Rgb,
}

/// The formatting a table style gives one cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellStyle {
    pub fill: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    /// `None` is the automatic (black) colour.
    pub font_color: Option<Rgb>,
    /// left, top, bottom, right.
    pub borders: [Option<Edge>; 4],
    /// `numFmtId` a user format sets (the dxf's number format); `None` leaves the cell's own.
    pub num_fmt_id: Option<u32>,
}

/// Styles for every cell of the pivot range (page fields included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledGrid {
    /// Sheet row and column (1-based) of the first cell.
    pub top: u32,
    pub left: u32,
    pub cells: Vec<Vec<CellStyle>>,
}

impl PivotTable {
    /// Table style, as stored.
    pub fn style_info(&self) -> Option<StyleInfo> {
        crate::model::read_style_info(self.xml())
    }
}

/// Ascending priority of the elements: a later element overrides the properties it sets.
const PRIORITY: [usize; 28] = [
    element::WHOLE_TABLE,
    element::ROW_STRIPE_1,
    element::ROW_STRIPE_2,
    element::COL_STRIPE_1,
    element::COL_STRIPE_2,
    element::FIRST_COLUMN,
    element::LAST_COLUMN,
    element::HEADER_ROW,
    element::SUBTOTAL_COLUMN_1,
    element::SUBTOTAL_COLUMN_1 + 1,
    element::SUBTOTAL_COLUMN_1 + 2,
    element::TOTAL_ROW,
    element::COLUMN_SUBHEADING_1 + 2,
    element::COLUMN_SUBHEADING_1 + 1,
    element::COLUMN_SUBHEADING_1,
    element::SUBTOTAL_ROW_1 + 2,
    element::SUBTOTAL_ROW_1 + 1,
    element::SUBTOTAL_ROW_1,
    element::ROW_SUBHEADING_1 + 2,
    element::ROW_SUBHEADING_1 + 1,
    element::ROW_SUBHEADING_1,
    element::BLANK_ROW,
    element::FIRST_HEADER_CELL,
    element::LAST_HEADER_CELL,
    element::FIRST_TOTAL_CELL,
    element::LAST_TOTAL_CELL,
    element::PAGE_FIELD_LABELS,
    element::PAGE_FIELD_VALUES,
];

/// Marks a cell of the page-field area: it takes the whole-table font and fill, but not the
/// whole-table outer borders (the page area is outside the table body).
const PAGE_AREA: u32 = 1 << 31;

/// Marks a label cell whose subheading element ends at its own top and bottom edges: it does
/// not join the cells above and below into one block.
const OWN_BLOCK: u32 = 1 << 30;

/// Like [`OWN_BLOCK`] for the left and right edges: a data field caption cell of the column
/// axis is a block of its own.
const OWN_BLOCK_H: u32 = 1 << 29;

/// Position of a cell relative to the table body.
struct Place<'a> {
    info: &'a StyleInfo,
    /// The row axis is in tabular form (one label column per field).
    tabular: bool,
    grid: &'a PivotGrid,
    cols: usize,
    /// Stripe sizes (first, second) of rows and of columns.
    row_stripes: (usize, usize),
    col_stripes: (usize, usize),
}

/// Sizes of the two stripes of a pair of elements (`size` of the style element, default 1).
fn stripe_sizes(def: &StyleDef, first: usize) -> (usize, usize) {
    let size = |e: usize| {
        def.get(e)
            .and_then(Option::as_ref)
            .map_or(1, |d| d.stripe.max(1))
    };
    (size(first), size(first + 1))
}

impl Place<'_> {
    /// Which stripe element (0 first, 1 second) the stripe index `i` falls in.
    fn stripe(sizes: (usize, usize), i: usize) -> usize {
        usize::from(i % (sizes.0 + sizes.1) >= sizes.0)
    }

    fn row_line(&self, r: usize) -> Option<&LineInfo> {
        r.checked_sub(self.grid.header_rows)
            .and_then(|i| self.grid.row_lines.get(i))
    }

    fn col_line(&self, c: usize) -> Option<&LineInfo> {
        c.checked_sub(self.grid.label_cols)
            .and_then(|i| self.grid.col_lines.get(i))
    }

    /// Elements that apply to the cell at body row `r`, column `c`, as a bit set.
    fn elements(&self, r: usize, c: usize) -> u32 {
        let mut set = 1u32 << element::WHOLE_TABLE;
        let mut add = |e: usize| set |= 1 << e;
        let header = r < self.grid.header_rows;
        let rl = self.row_line(r);
        let cl = self.col_line(c);
        if self.info.show_row_stripes && !header {
            add(element::ROW_STRIPE_1 + Self::stripe(self.row_stripes, r - self.grid.header_rows));
        }
        if self.info.show_col_stripes && c >= self.grid.label_cols && !header {
            add(element::COL_STRIPE_1 + Self::stripe(self.col_stripes, c - self.grid.label_cols));
        }
        if self.info.show_row_headers && c < self.grid.label_cols {
            add(element::FIRST_COLUMN);
        }
        if self.info.show_last_column && c + 1 == self.cols && r > 0 {
            add(element::LAST_COLUMN);
        }
        if self.info.show_col_headers && header {
            add(element::HEADER_ROW);
            // The corner above the row-field captions: every header row but the last, over
            // the label columns.
            if r + 1 < self.grid.header_rows && c < self.grid.label_cols {
                add(element::FIRST_HEADER_CELL);
            }
        }
        if let Some(l) = rl
            && self.tabular
            && self.info.show_row_headers
            && c < self.grid.label_cols
        {
            // Tabular layouts have no subheading rows: the subheading element of level k formats
            // the label column k of every line of the group.
            // A blank row after the last group of its parent is outside the parent's group.
            let continues = l.kind != LineKind::Blank || l.shared_next > c;
            if matches!(
                l.kind,
                LineKind::Item | LineKind::DataLeaf | LineKind::Subtotal | LineKind::Blank
            ) && c < l.level
                && continues
            {
                add(element::ROW_SUBHEADING_1 + c.min(2));
                if c == 0
                    && l.kind == LineKind::DataLeaf
                    && self.grid.row_levels.last() == Some(&None)
                    && self.grid.row_levels.len() > 2
                {
                    add(30);
                }
            }
        }
        if let Some(l) = rl {
            match l.kind {
                LineKind::Grand => add(element::TOTAL_ROW),
                // In tabular form the label columns left of the subtotal belong to the groups.
                LineKind::Subtotal if !(self.grid.label_cols > 1 && c < l.level) => {
                    add(element::SUBTOTAL_ROW_1 + l.level.min(2));
                    // One subtotal line per data field: each is a block of its own.
                    if self.grid.row_levels.last() == Some(&None) {
                        add(30);
                    }
                }
                LineKind::Item
                    if !l.leaf
                        && self.info.show_row_headers
                        && !(self.grid.label_cols > 1 && c < l.level) =>
                {
                    add(element::ROW_SUBHEADING_1 + l.level.min(2));
                }
                LineKind::Blank => add(element::BLANK_ROW),
                _ => {}
            }
        }
        if let Some(l) = cl
            && self.info.show_col_headers
            && header
            && r >= 1
            && r - 1 == l.level
            && (l.kind == LineKind::Item || (l.kind == LineKind::DataLeaf && l.level >= 1))
        {
            // The labels of column field level k sit in header row k + 1.
            add(element::COLUMN_SUBHEADING_1 + l.level.min(2));
            if l.kind == LineKind::DataLeaf {
                add(29);
            }
        }
        if let Some(l) = cl {
            match l.kind {
                LineKind::Subtotal if r >= 1 => add(element::SUBTOTAL_COLUMN_1 + l.level.min(2)),
                _ => {}
            }
        }
        set
    }
}

/// Table style formatting of every cell of `grid`, or `None` when the table has no style or
/// the style is not a built-in one.
pub fn style_cells(table: &PivotTable, grid: &PivotGrid, theme: &Theme) -> Option<StyledGrid> {
    style_cells_with(table, grid, theme, &CustomStyles::default())
}

/// Like [`style_cells`], with the workbook's custom table styles (`tableStyles` in the style
/// sheet) looked up before the built-in ones.
pub fn style_cells_with(
    table: &PivotTable,
    grid: &PivotGrid,
    theme: &Theme,
    custom: &CustomStyles,
) -> Option<StyledGrid> {
    let info = table.style_info()?;
    let def = custom
        .get(&info.name)
        .or_else(|| builtin_table().get(&info.name))?;
    let page_rows = (grid.body.0 - grid.range.0) as usize;
    let np = table.page_fields.len().min(page_rows);
    let rows = grid.header_rows + grid.row_lines.len();
    let cols = grid.label_cols + grid.col_lines.len();
    let place = Place {
        info: &info,
        tabular: !table.compact && !table.outline,
        grid,
        cols,
        row_stripes: stripe_sizes(def, element::ROW_STRIPE_1),
        col_stripes: stripe_sizes(def, element::COL_STRIPE_1),
    };
    let width = ((grid.range.3 - grid.range.1 + 1) as usize).max(cols);
    let total = page_rows + rows;
    let mut out = vec![vec![CellStyle::default(); width]; total];
    // Element sets of every cell: the page-field area (labels in the first column, values in the
    // second, then a spacer row) above the body.
    let sets: Vec<Vec<u32>> = (0..total)
        .map(|r| {
            (0..width)
                .map(|c| match r.checked_sub(page_rows) {
                    Some(br) if c < cols => place.elements(br, c),
                    Some(_) => 0,
                    None if r < np && c == 0 => PAGE_AREA | 1 | 1 << element::PAGE_FIELD_LABELS,
                    None if r < np && c == 1 => PAGE_AREA | 1 | 1 << element::PAGE_FIELD_VALUES,
                    None => 0,
                })
                .collect()
        })
        .collect();
    let mut cand: Vec<Vec<Candidates>> = vec![vec![Candidates::default(); width]; total];
    for (r, line) in out.iter_mut().enumerate() {
        for (c, cell) in line.iter_mut().enumerate() {
            if sets[r][c] == 0 {
                continue;
            }
            let (st, edges) = compose(def, &sets, r, c, theme);
            *cell = st;
            cand[r][c] = edges;
        }
    }
    // A border shared by two cells shows on both: the one from the higher-priority element.
    for r in 0..total {
        for c in 0..width {
            let mut final_edges: [Option<Edge>; 4] = [None; 4];
            for (side, slot) in final_edges.iter_mut().enumerate() {
                let (dr, dc, opposite) = match side {
                    0 => (0isize, -1isize, 3),
                    1 => (-1, 0, 2),
                    2 => (1, 0, 1),
                    _ => (0, 1, 0),
                };
                let own = cand[r][c].0[side].as_slice();
                let other = r
                    .checked_add_signed(dr)
                    .zip(c.checked_add_signed(dc))
                    .and_then(|(nr, nc)| cand.get(nr)?.get(nc))
                    .map_or(&[][..], |n| n.0[opposite].as_slice());
                *slot = if side <= 1 {
                    resolve_edge(other, own)
                } else {
                    resolve_edge(own, other)
                };
            }
            out[r][c].borders = final_edges;
        }
    }
    Some(StyledGrid {
        top: grid.range.0,
        left: grid.range.1,
        cells: out,
    })
}

/// Like [`style_cells_with`], then the user formats (`formats` of the table definition, the
/// `dxfs` of the style sheet) laid over it. A table without a style still gets its user formats.
pub fn style_cells_full(
    table: &PivotTable,
    grid: &PivotGrid,
    theme: &Theme,
    custom: &CustomStyles,
    dxfs: &Dxfs,
) -> Option<StyledGrid> {
    let user = table.user_formats();
    let styled = style_cells_with(table, grid, theme, custom);
    if user.is_empty() {
        return styled;
    }
    let mut styled = styled.unwrap_or_else(|| {
        let page_rows = (grid.body.0 - grid.range.0) as usize;
        let rows = grid.header_rows + grid.row_lines.len();
        let cols = grid.label_cols + grid.col_lines.len();
        let width = ((grid.range.3 - grid.range.1 + 1) as usize).max(cols);
        StyledGrid {
            top: grid.range.0,
            left: grid.range.1,
            cells: vec![vec![CellStyle::default(); width]; page_rows + rows],
        }
    });
    apply_user_formats(&mut styled, table, grid, &user, dxfs, theme);
    Some(styled)
}

/// Border candidates of one cell: per side (left, top, bottom, right), the edge each applying
/// element defines, in ascending element priority.
#[derive(Debug, Clone, Default)]
struct Candidates([Vec<(usize, Edge)>; 4]);

/// Ascending priority of the elements for borders (own and shared). It differs from the fill
/// order: measured, subtotal columns lose to the total row, which loses to subtotal rows, which
/// lose to row subheadings, and the header row beats them all.
const EDGE_PRIORITY: [usize; 28] = [
    element::WHOLE_TABLE,
    element::ROW_STRIPE_1,
    element::ROW_STRIPE_2,
    element::COL_STRIPE_1,
    element::COL_STRIPE_2,
    element::FIRST_COLUMN,
    element::LAST_COLUMN,
    element::SUBTOTAL_COLUMN_1,
    element::SUBTOTAL_COLUMN_1 + 1,
    element::SUBTOTAL_COLUMN_1 + 2,
    element::COLUMN_SUBHEADING_1 + 2,
    element::COLUMN_SUBHEADING_1 + 1,
    element::COLUMN_SUBHEADING_1,
    element::TOTAL_ROW,
    element::SUBTOTAL_ROW_1 + 2,
    element::SUBTOTAL_ROW_1 + 1,
    element::SUBTOTAL_ROW_1,
    element::ROW_SUBHEADING_1 + 2,
    element::ROW_SUBHEADING_1 + 1,
    element::ROW_SUBHEADING_1,
    element::BLANK_ROW,
    element::HEADER_ROW,
    element::FIRST_HEADER_CELL,
    element::LAST_HEADER_CELL,
    element::FIRST_TOTAL_CELL,
    element::LAST_TOTAL_CELL,
    element::PAGE_FIELD_LABELS,
    element::PAGE_FIELD_VALUES,
];

/// Pseudo element of an inside border of the whole table (between two cells of the table):
/// it ranks just above the plain whole-table border and beats a stripe border only when it is
/// heavier (measured on custom styles).
const WHOLE_INSIDE: usize = element::COUNT;

/// Heaviness of an Excel border weight (hair, thin, medium, thick).
fn weight_order(w: i32) -> i32 {
    match w {
        1 => 0,
        2 => 1,
        -4138 => 2,
        _ => 3,
    }
}

fn edge_rank(element: usize) -> usize {
    let rank = |e: usize| 2 * EDGE_PRIORITY.iter().position(|x| *x == e).unwrap_or(0);
    // Subtotal rows and row subheadings tie (the lower cell wins a shared border).
    match element {
        WHOLE_INSIDE => rank(element::WHOLE_TABLE) + 1,
        e if (element::SUBTOTAL_ROW_1..element::SUBTOTAL_ROW_1 + 3).contains(&e) => {
            rank(element::SUBTOTAL_ROW_1)
        }
        e if (element::ROW_SUBHEADING_1..element::ROW_SUBHEADING_1 + 3).contains(&e) => {
            rank(element::SUBTOTAL_ROW_1)
        }
        e => rank(e),
    }
}

/// The border a shared edge shows: the best-ranked candidate of the two cells; on a tie the
/// lower (or right) cell's. `first` is the upper (or left) cell's candidates.
fn resolve_edge(first: &[(usize, Edge)], second: &[(usize, Edge)]) -> Option<Edge> {
    let best = |c: &[(usize, Edge)]| {
        let top = c.iter().max_by_key(|(e, _)| edge_rank(*e)).copied()?;
        let stripe = (element::ROW_STRIPE_1..=element::COL_STRIPE_2).contains(&top.0);
        if stripe
            && let Some(w) = c
                .iter()
                .filter(|(e, x)| {
                    *e == WHOLE_INSIDE && weight_order(x.weight) > weight_order(top.1.weight)
                })
                .max_by_key(|(_, x)| weight_order(x.weight))
        {
            return Some(*w);
        }
        Some(top)
    };
    match (best(first), best(second)) {
        (Some((a, x)), Some((b, y))) => Some(if edge_rank(a) > edge_rank(b) { x } else { y }),
        (Some((_, x)), None) | (None, Some((_, x))) => Some(x),
        (None, None) => None,
    }
}

/// Per side (left, top, bottom, right) the elements that define the border, with their edge.
pub type EdgeSources = [Vec<(usize, Edge)>; 4];

/// Test support: the elements that define each border of each body cell
/// (`[left, top, bottom, right]`, ascending priority).
#[doc(hidden)]
pub fn edge_sources(
    table: &PivotTable,
    grid: &PivotGrid,
    theme: &Theme,
) -> Option<Vec<Vec<EdgeSources>>> {
    let info = table.style_info()?;
    let def = builtin_table().get(&info.name)?;
    let rows = grid.header_rows + grid.row_lines.len();
    let cols = grid.label_cols + grid.col_lines.len();
    let place = Place {
        info: &info,
        tabular: !table.compact && !table.outline,
        grid,
        cols,
        row_stripes: stripe_sizes(def, element::ROW_STRIPE_1),
        col_stripes: stripe_sizes(def, element::COL_STRIPE_1),
    };
    let sets: Vec<Vec<u32>> = (0..rows)
        .map(|r| (0..cols).map(|c| place.elements(r, c)).collect())
        .collect();
    Some(
        (0..rows)
            .map(|r| {
                (0..cols)
                    .map(|c| compose(def, &sets, r, c, theme).1.0)
                    .collect()
            })
            .collect(),
    )
}

/// Composes the formatting of body cell (`r`, `c`) from the elements that apply to it.
fn compose(
    def: &StyleDef,
    sets: &[Vec<u32>],
    r: usize,
    c: usize,
    theme: &Theme,
) -> (CellStyle, Candidates) {
    let mut style = CellStyle::default();
    let here = sets[r][c];
    let neighbour = |dr: isize, dc: isize| -> Option<u32> {
        let (nr, nc) = (r.checked_add_signed(dr)?, c.checked_add_signed(dc)?);
        sets.get(nr)?.get(nc).copied()
    };
    let mut cand = Candidates::default();
    for &e in &PRIORITY {
        if here & (1 << e) == 0 {
            continue;
        }
        let Some(d) = def.get(e).and_then(Option::as_ref) else {
            continue;
        };
        if let Some(b) = d.bold {
            style.bold = b;
        }
        if let Some(i) = d.italic {
            style.italic = i;
        }
        if let Some(f) = d.font {
            style.font_color = Some(f.resolve(theme));
        }
        if let Some(f) = d.fill {
            style.fill = Some(f.resolve(theme));
        }
        let own = here & OWN_BLOCK != 0
            && ((element::ROW_SUBHEADING_1..element::ROW_SUBHEADING_1 + 3).contains(&e)
                || (element::SUBTOTAL_ROW_1..element::SUBTOTAL_ROW_1 + 3).contains(&e));
        let own_h = here & OWN_BLOCK_H != 0
            && (element::COLUMN_SUBHEADING_1..element::COLUMN_SUBHEADING_1 + 3).contains(&e);
        let inside = |dr: isize, dc: isize| {
            !(own && dr != 0 || own_h && dc != 0)
                && neighbour(dr, dc).is_some_and(|s| s & (1 << e) != 0)
        };
        let page_whole = e == element::WHOLE_TABLE && here & PAGE_AREA != 0;
        // side: 0 left, 1 top, 2 bottom, 3 right; borders: left, top, bottom, right, iv, ih
        let picks = [
            (0, if inside(0, -1) { 4 } else { 0 }),
            (1, if inside(-1, 0) { 5 } else { 1 }),
            (2, if inside(1, 0) { 5 } else { 2 }),
            (3, if inside(0, 1) { 4 } else { 3 }),
        ];
        for (side, idx) in picks {
            // The page-field area takes the whole-table borders between its own cells only.
            if page_whole && idx < 4 && !(side == 1 || side == 2) {
                continue;
            }
            if let Some(b) = d.borders[idx] {
                let tag = if e == element::WHOLE_TABLE && idx >= 4 {
                    WHOLE_INSIDE
                } else {
                    e
                };
                cand.0[side].push((
                    tag,
                    Edge {
                        line: b.line,
                        weight: b.weight,
                        color: b.color.resolve(theme),
                    },
                ));
            }
        }
    }
    (style, cand)
}
