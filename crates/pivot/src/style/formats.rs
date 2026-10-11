//! User formats: `pivotTableDefinition/formats`, the formatting a user applied to areas of the
//! pivot table (a `pivotArea` selecting cells and a `dxf` of the style sheet to lay over them).
//! They are layered over the table style, in document order (a later format wins), and they
//! survive a refresh because they select by field and item position, not by cell address.

use open365_xlsx::{Theme, XmlElement};

use super::{CellStyle, Edge, EdgeDef, ElementDef, StyledGrid, custom::dxf_element};
use crate::compute::{AxisLine, LineKind, PivotGrid};
use crate::model::{PivotTable, VALUES_FIELD};
use crate::xmlutil::attr_bool;

/// `pivotArea/@type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AreaType {
    None,
    Normal,
    Data,
    All,
    Origin,
    Button,
    TopRight,
}

/// One `references/reference`: a field and the item positions it selects.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AreaRef {
    field: u32,
    items: Vec<u32>,
    /// `defaultSubtotal`: the subtotal line of the field.
    default_subtotal: bool,
}

/// A `pivotArea` of a format.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PivotArea {
    kind: AreaType,
    data_only: bool,
    label_only: bool,
    grand_row: bool,
    grand_col: bool,
    /// `outline` (default true).
    outline: bool,
    /// `offset` is set (a range shifted from the item).
    offset: bool,
    field: Option<u32>,
    axis: Option<String>,
    field_position: u32,
    refs: Vec<AreaRef>,
}

/// One `format`: the area and the `dxfId` laid over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFormat {
    dxf: usize,
    area: PivotArea,
}

/// A differential format of the style sheet, as far as pivot cells show it.
#[derive(Debug, Clone, Default)]
struct Dxf {
    def: ElementDef,
    num_fmt_id: Option<u32>,
}

/// The `dxfs` of a style sheet.
#[derive(Debug, Clone, Default)]
pub struct Dxfs {
    list: Vec<Dxf>,
    /// `numFmtId` to format code of the dxf number formats.
    codes: std::collections::HashMap<u32, String>,
}

impl Dxfs {
    /// Reads the `dxfs` of `xl/styles.xml`.
    pub fn parse(styles_xml: &[u8]) -> Self {
        let Ok(xml) = open365_xlsx::parse_xml(styles_xml) else {
            return Self::default();
        };
        let mut codes = std::collections::HashMap::new();
        let list = xml
            .root
            .child_main("dxfs")
            .map(|d| {
                d.elements()
                    .filter(|e| e.head.is_main("dxf"))
                    .map(|dxf| {
                        let nf = dxf.child_main("numFmt");
                        let id = nf
                            .and_then(|n| n.head.attr("numFmtId"))
                            .and_then(|v| v.trim().parse().ok());
                        if let (Some(id), Some(code)) =
                            (id, nf.and_then(|n| n.head.attr("formatCode")))
                        {
                            codes.insert(id, code.to_owned());
                        }
                        Dxf {
                            def: dxf_element(dxf, 1),
                            num_fmt_id: id,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { list, codes }
    }

    /// Format code of a `numFmtId` a user format sets (a dxf's own code, else a built-in one).
    pub fn number_format_code(&self, id: u32) -> Option<String> {
        self.codes
            .get(&id)
            .cloned()
            .or_else(|| open365_xlsx::builtin_number_format(id).map(str::to_owned))
    }

    /// The `dxfs` of an opened workbook.
    pub fn from_workbook(wb: &open365_xlsx::Workbook) -> Self {
        wb.package()
            .part("xl/styles.xml")
            .map(|p| Self::parse(p.data()))
            .unwrap_or_default()
    }
}

fn area_ref(e: &XmlElement) -> Option<AreaRef> {
    Some(AreaRef {
        field: e.head.attr("field")?.trim().parse().ok()?,
        items: e
            .elements()
            .filter(|x| x.head.is_main("x"))
            .filter_map(|x| x.head.attr("v")?.trim().parse().ok())
            .collect(),
        default_subtotal: attr_bool(e, "defaultSubtotal", false),
    })
}

fn pivot_area(e: &XmlElement) -> PivotArea {
    let kind = match e.head.attr("type").map(str::trim) {
        Some("none") => AreaType::None,
        Some("data") => AreaType::Data,
        Some("all") => AreaType::All,
        Some("origin") => AreaType::Origin,
        Some("button") => AreaType::Button,
        Some("topRight") => AreaType::TopRight,
        _ => AreaType::Normal,
    };
    PivotArea {
        kind,
        data_only: attr_bool(e, "dataOnly", true),
        label_only: attr_bool(e, "labelOnly", false),
        grand_row: attr_bool(e, "grandRow", false),
        grand_col: attr_bool(e, "grandCol", false),
        outline: attr_bool(e, "outline", true),
        offset: e.head.attr("offset").is_some(),
        field: e.head.attr("field").and_then(|v| v.trim().parse().ok()),
        axis: e.head.attr("axis").map(|a| a.trim().to_owned()),
        field_position: e
            .head
            .attr("fieldPosition")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0),
        refs: e
            .child_main("references")
            .map(|r| {
                r.elements()
                    .filter(|x| x.head.is_main("reference"))
                    .filter_map(area_ref)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

impl PivotTable {
    /// The user formats (`formats/format`) in document order, which is the order they were
    /// applied in.
    pub fn user_formats(&self) -> Vec<UserFormat> {
        let Some(formats) = self.xml().root.child_main("formats") else {
            return Vec::new();
        };
        formats
            .elements()
            .filter(|e| e.head.is_main("format"))
            .filter_map(|f| {
                Some(UserFormat {
                    dxf: f.head.attr("dxfId")?.trim().parse().ok()?,
                    area: pivot_area(f.child_main("pivotArea")?),
                })
            })
            .collect()
    }
}

/// What a cell of the table is, for matching a `pivotArea`.
struct Cell<'a> {
    /// Row of the body (header rows first).
    r: usize,
    c: usize,
    header_rows: usize,
    label_cols: usize,
    row: Option<&'a AxisLine>,
    col: Option<&'a AxisLine>,
    /// The cell holds the label of its row line (or column line, in a header row).
    row_label: bool,
    col_label: bool,
}

struct Matcher<'a> {
    grid: &'a PivotGrid,
    compact: bool,
}

impl Matcher<'_> {
    /// Position of `field` on the row axis, else on the column axis: (is row axis, level).
    fn level_of(&self, field: u32) -> Option<(bool, usize)> {
        let f = Some(field as usize);
        if let Some(l) = self.grid.row_levels.iter().position(|x| *x == f) {
            return Some((true, l));
        }
        self.grid
            .col_levels
            .iter()
            .position(|x| *x == f)
            .map(|l| (false, l))
    }

    /// Item position of `field` that the cell is in, if its line goes down to the field.
    fn item_of(&self, cell: &Cell, field: u32) -> Option<(usize, bool)> {
        let (row_axis, level) = self.level_of(field)?;
        let line = if row_axis { cell.row } else { cell.col }?;
        if line.kind == LineKind::Grand || line.kind == LineKind::Blank || line.level < level {
            return None;
        }
        let pos = *line.path.get(level)?;
        Some((pos, line.kind == LineKind::Subtotal && line.level == level))
    }

    fn matches(&self, area: &PivotArea, cell: &Cell) -> bool {
        let in_data = cell.r >= cell.header_rows && cell.c >= cell.label_cols;
        let labels = !area.data_only || area.label_only;
        let data = !area.label_only;
        let is_label = cell.row_label || cell.col_label;
        let corner = cell.r < cell.header_rows && cell.c < cell.label_cols;
        match area.kind {
            AreaType::None => return false,
            AreaType::All => return true,
            AreaType::Origin => return corner,
            AreaType::TopRight if !area.outline => {
                return cell.r == 0 && cell.c >= cell.label_cols;
            }
            AreaType::TopRight => {
                return cell.r < cell.header_rows
                    && cell.c >= cell.label_cols
                    && cell.col_label
                    && cell.col.is_some_and(|l| l.level == 0);
            }
            AreaType::Button => return self.button(area, cell),
            AreaType::Normal | AreaType::Data => {}
        }
        if corner {
            return false;
        }
        if std::env::var("PIVOT_OLD").is_err()
            && !area.outline
            && !area.refs.is_empty()
            && matches!(area.kind, AreaType::Normal)
            && area.label_only
            && !self.compact
        {
            return self.field_range(area, cell);
        }
        if !area.outline
            && area.label_only
            && area.grand_row
            && area.refs.is_empty()
            && matches!(area.kind, AreaType::Normal)
            && cell.c < cell.label_cols
            && cell.row.is_some_and(|l| l.kind == LineKind::Grand)
        {
            return true;
        }
        if is_label && !labels || in_data && !data || !is_label && !in_data {
            return false;
        }
        if area.grand_row || area.grand_col {
            let row_ok = !area.grand_row || cell.row.is_some_and(|l| l.kind == LineKind::Grand);
            let col_ok = !area.grand_col || cell.col.is_some_and(|l| l.kind == LineKind::Grand);
            if (area.grand_col && cell.col.is_none()) || (area.grand_row && cell.row.is_none()) {
                return false;
            }
            if !(row_ok && col_ok) {
                return false;
            }
            if area.refs.is_empty() {
                return true;
            }
        }
        if area.refs.is_empty() {
            return in_data;
        }
        self.refs_match(area, cell)
    }

    /// The refs above `level` hold for a line shallower than the whole-field ref at `level`.
    fn parent_matches(&self, area: &PivotArea, cell: &Cell, level: usize) -> bool {
        area.refs.iter().all(|r| {
            let Some((true, lv)) = self.level_of(r.field) else {
                return true;
            };
            if lv >= level || r.items.is_empty() {
                return true;
            }
            self.item_of(cell, r.field)
                .is_some_and(|(pos, _)| r.items.iter().any(|i| *i as usize == pos))
        })
    }

    /// `outline="0"` label areas in outline/tabular layouts: the field's data range, which is the
    /// label columns right of the deepest named field over every line of the named items.
    fn field_range(&self, area: &PivotArea, cell: &Cell) -> bool {
        let mut d: Option<usize> = None;
        for r in &area.refs {
            if r.field == VALUES_FIELD {
                continue;
            }
            let Some((true, level)) = self.level_of(r.field) else {
                return false;
            };
            d = Some(d.map_or(level, |x| x.max(level)));
        }
        let Some(d) = d else { return false };
        let subtotal_area = area.refs.iter().any(|r| r.default_subtotal);
        let lc = cell.label_cols;
        let line_level = cell.row.map_or(0, |l| l.level);
        if subtotal_area {
            if cell.c + 1 != lc {
                return false;
            }
        } else if area.offset {
            if cell.c + 1 != lc || line_level != d {
                return false;
            }
        } else if cell.c != d.min(lc.saturating_sub(1)) {
            return false;
        }
        for r in &area.refs {
            if r.field == VALUES_FIELD {
                continue;
            }
            let Some((_, level)) = self.level_of(r.field) else {
                return false;
            };
            if level > line_level && r.items.is_empty() && level == d && !subtotal_area {
                // The whole field range spans the parent's own line.
                if !self.parent_matches(area, cell, level) {
                    return false;
                }
                continue;
            }
            let Some((pos, subtotal)) = self.item_of(cell, r.field) else {
                return false;
            };
            if r.default_subtotal && !subtotal {
                return false;
            }
            if !r.items.is_empty() && !r.items.iter().any(|i| *i as usize == pos) {
                return false;
            }
        }
        cell.row
            .is_some_and(|l| !matches!(l.kind, LineKind::Grand | LineKind::Blank))
    }

    fn refs_match(&self, area: &PivotArea, cell: &Cell) -> bool {
        // The deepest level per axis the references name: the cell's line must end there.
        let mut deepest: [Option<usize>; 2] = [None, None];
        for r in &area.refs {
            if r.field == VALUES_FIELD || (r.items.is_empty() && r.default_subtotal) {
                continue;
            }
            let Some((row_axis, level)) = self.level_of(r.field) else {
                return false;
            };
            let d = &mut deepest[usize::from(!row_axis)];
            *d = Some(d.map_or(level, |x| x.max(level)));
        }
        for (axis_i, d) in deepest.iter().enumerate() {
            let Some(d) = d else { continue };
            let line = if axis_i == 0 { cell.row } else { cell.col };
            let Some(line) = line else { return false };
            if line.level != *d || matches!(line.kind, LineKind::Grand | LineKind::Blank) {
                return false;
            }
        }
        for r in &area.refs {
            if r.field == VALUES_FIELD {
                if !r.items.iter().all(|i| {
                    [cell.row, cell.col]
                        .iter()
                        .flatten()
                        .any(|l| l.data == Some(*i as usize))
                }) {
                    return false;
                }
                continue;
            }
            let Some((pos, subtotal)) = self.item_of(cell, r.field) else {
                return false;
            };
            if r.default_subtotal && !r.items.is_empty() && !subtotal {
                return false;
            }
            if !r.default_subtotal && subtotal && r.items.is_empty() {
                return false;
            }
            if !r.items.is_empty() && !r.items.iter().any(|i| *i as usize == pos) {
                return false;
            }
        }
        true
    }

    fn button(&self, area: &PivotArea, cell: &Cell) -> bool {
        let Some(field) = area.field else {
            return false;
        };
        let Some((row_axis, level)) = self.level_of(field) else {
            return false;
        };
        if row_axis {
            let col = if self.compact { 0 } else { level };
            cell.r + 1 == cell.header_rows && cell.c == col
        } else {
            cell.c == cell.label_cols && cell.r == level
        }
    }
}

fn to_edge(e: &EdgeDef, theme: &Theme) -> Edge {
    Edge {
        line: e.line,
        weight: e.weight,
        color: e.color.resolve(theme),
    }
}

fn paint(style: &mut CellStyle, dxf: &Dxf, theme: &Theme) {
    let d = &dxf.def;
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
    if let Some(n) = dxf.num_fmt_id {
        style.num_fmt_id = Some(n);
    }
    // left, top, bottom, right
    for side in 0..4 {
        if let Some(b) = &d.borders[side] {
            style.borders[side] = Some(to_edge(b, theme));
        }
    }
}

/// Lays the user formats over `styled` (the table style's formatting, or an empty grid).
pub fn apply_user_formats(
    styled: &mut StyledGrid,
    table: &PivotTable,
    grid: &PivotGrid,
    formats: &[UserFormat],
    dxfs: &Dxfs,
    theme: &Theme,
) {
    if formats.is_empty() || grid.row_items.len() != grid.row_lines.len() {
        return;
    }
    let m = Matcher {
        grid,
        compact: table.compact,
    };
    let page_rows = (grid.body.0 - grid.range.0) as usize;
    let col_items: &[AxisLine] = &grid.col_items;
    let rows = grid.header_rows + grid.row_lines.len();
    let width = styled.cells.first().map_or(0, Vec::len);
    let lc = grid.label_cols;
    for f in formats {
        let Some(dxf) = dxfs.list.get(f.dxf) else {
            continue;
        };
        let mut hit: Vec<(usize, usize)> = Vec::new();
        for r in 0..rows {
            for c in 0..width {
                let row = r
                    .checked_sub(grid.header_rows)
                    .and_then(|i| grid.row_items.get(i));
                let col = c.checked_sub(lc).and_then(|i| col_items.get(i));
                let row_label = row.is_some_and(|l| {
                    c < lc
                        && matches!(
                            l.kind,
                            LineKind::Item | LineKind::Subtotal | LineKind::Grand
                        )
                        && c == if m.compact { 0 } else { l.level.min(lc - 1) }
                });
                let col_label = col.is_some_and(|l| {
                    r < grid.header_rows
                        && r >= 1
                        && r - 1 == l.level
                        && matches!(
                            l.kind,
                            LineKind::Item | LineKind::Subtotal | LineKind::Grand
                        )
                });
                let cell = Cell {
                    r,
                    c,
                    header_rows: grid.header_rows,
                    label_cols: lc,
                    row,
                    col,
                    row_label,
                    col_label,
                };
                if m.matches(&f.area, &cell) {
                    hit.push((r, c));
                }
            }
        }
        for (r, c) in hit {
            if let Some(cell) = styled
                .cells
                .get_mut(page_rows + r)
                .and_then(|row| row.get_mut(c))
            {
                paint(cell, dxf, theme);
            }
        }
    }
}
