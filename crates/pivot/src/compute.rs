//! Pivot computation: source data, item order, axis lines and the output grid.

use std::cmp::Ordering;
use std::collections::HashMap;

use open365_locale::{CollationStrength, Locale};
use open365_opc::{PartName, Relationships, rels_part_name, resolve_target};
use open365_xlsx::{Workbook, parse_cell_ref, parse_range};

use crate::calcitems::{CalcItems, Comb};
use crate::error::PivotError;
use crate::grouping::grouped_column;
use crate::location::Location;
use crate::model::{
    Axis, CacheDefinition, CacheRecords, CacheSource, Func, PivotTable, Record, VALUES_FIELD,
};
use crate::showas::{Calc, Cons, Mode};
use crate::value::{ItemKey, Value, compare_values};

/// Captions Excel does not store in the file; they follow the UI language of the
/// application that last refreshed the pivot. Defaults are English (en-GB measured).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotStrings {
    pub row_labels: String,
    pub column_labels: String,
    pub grand_total: String,
    pub blank_item: String,
    pub all_items: String,
    pub true_text: String,
    pub false_text: String,
    /// `{0}` is the item text.
    pub total_of: String,
    /// Prefix of a grand total column when several data fields exist.
    pub grand_total_prefix: String,
    pub subtotal_names: [String; 11],
}

impl Default for PivotStrings {
    fn default() -> Self {
        let n = |s: &str| s.to_owned();
        Self {
            row_labels: n("Row Labels"),
            column_labels: n("Column Labels"),
            grand_total: n("Grand Total"),
            blank_item: n("(blank)"),
            all_items: n("(All)"),
            true_text: n("TRUE"),
            false_text: n("FALSE"),
            total_of: n("{0} Total"),
            grand_total_prefix: n("Total"),
            subtotal_names: [
                n("Sum"),
                n("Count"),
                n("Average"),
                n("Max."),
                n("Min."),
                n("Product"),
                n("Count"),
                n("StdDev"),
                n("StdDevp"),
                n("Var"),
                n("Varp"),
            ],
        }
    }
}

/// Source columns, one `Vec<Value>` per cache field (empty for derived group fields).
#[derive(Debug, Clone, Default)]
pub struct Dataset {
    pub columns: Vec<Vec<Value>>,
    pub rows: usize,
}

impl Dataset {
    /// From the cache as stored in the file (no source access).
    pub fn from_cache(def: &CacheDefinition, records: &CacheRecords) -> Self {
        let db: Vec<usize> = def
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.database_field)
            .map(|(i, _)| i)
            .collect();
        let mut columns = vec![Vec::new(); def.fields.len()];
        for row in &records.rows {
            for (k, rec) in row.iter().enumerate() {
                let Some(&fi) = db.get(k) else { continue };
                let v = match rec {
                    Record::Shared(i) => def.fields[fi]
                        .shared
                        .get(*i as usize)
                        .cloned()
                        .unwrap_or(Value::Blank),
                    Record::Value(v) => v.clone(),
                };
                columns[fi].push(v);
            }
        }
        Self {
            columns,
            rows: records.rows.len(),
        }
    }

    /// Re-reads the source range of a worksheet cache from the workbook's cells.
    pub fn from_workbook(wb: &Workbook, def: &CacheDefinition) -> Result<Self, PivotError> {
        let CacheSource::Worksheet {
            sheet,
            reference,
            name,
        } = &def.source
        else {
            return Err(PivotError::UnsupportedSource(
                "not a worksheet source".into(),
            ));
        };
        let (Some(sheet), Some(reference)) = (sheet, reference) else {
            return Err(PivotError::UnsupportedSource(format!(
                "named source {name:?}"
            )));
        };
        let si = wb
            .sheet_index(sheet)
            .ok_or_else(|| PivotError::MissingSource(sheet.clone()))?;
        let plain: String = reference.chars().filter(|c| *c != '$').collect();
        let (r1, c1, r2, c2) =
            parse_range(&plain).ok_or_else(|| PivotError::MissingSource(reference.clone()))?;
        let mut columns = vec![Vec::new(); def.fields.len()];
        for (k, col) in (c1..=c2).enumerate() {
            let Some(slot) = columns.get_mut(k) else {
                break;
            };
            for row in (r1 + 1)..=r2 {
                slot.push(
                    wb.cell(si, row, col)
                        .map_or(Value::Blank, |c| Value::from_cell(&c.value)),
                );
            }
        }
        Ok(Self {
            columns,
            rows: (r2 - r1) as usize,
        })
    }
}

/// How item order is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderMode {
    /// The order stored in the pivot field `items` (a file as Excel saved it).
    Stored,
    /// Re-sorted from the data per the field's sort type (a refresh).
    Refresh,
}

/// A computed cell value.
#[derive(Debug, Clone, PartialEq)]
pub enum GridValue {
    Text(String),
    Number(f64),
    Error(String),
}

/// One output cell. Rows and columns are 1-based sheet coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct GridCell {
    pub row: u32,
    pub col: u32,
    pub value: GridValue,
    /// `numFmtId` to apply (0 is General).
    pub num_fmt_id: u32,
}

/// The refreshed pivot output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PivotGrid {
    /// Non-empty cells in row-major order, page fields included.
    pub cells: Vec<GridCell>,
    /// Bounding box of the table body (`location/@ref`): first row, col, last row, col.
    pub body: (u32, u32, u32, u32),
    /// Bounding box including the page-field area.
    pub range: (u32, u32, u32, u32),
    /// The `location` element the refreshed table gets.
    pub location: Location,
    /// Header rows of the table body (column labels, data field captions).
    pub header_rows: usize,
    /// Label columns at the left of the body.
    pub label_cols: usize,
    /// Row lines below the header rows, in order (what the style engine needs).
    pub row_lines: Vec<LineInfo>,
    /// Column lines right of the label columns, in order.
    pub col_lines: Vec<LineInfo>,
    /// Identity of every value cell, for GETPIVOTDATA.
    pub data_cells: Vec<DataCell>,
    /// The lines of the row axis as `rowItems` lists them (item, subtotal, blank and grand lines).
    pub row_items: Vec<AxisLine>,
    /// The lines of the column axis as `colItems` lists them.
    pub col_items: Vec<AxisLine>,
    /// Row axis levels: the pivot field, `None` for the Values field.
    pub row_levels: Vec<Option<usize>>,
    /// Column axis levels: the pivot field, `None` for the Values field.
    pub col_levels: Vec<Option<usize>>,
    /// Item order per pivot field (axis fields only), the item positions the lines refer to.
    pub item_order: Vec<Vec<Value>>,
}

/// One line of an axis with the identity `rowItems`/`colItems` store.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisLine {
    pub kind: LineKind,
    /// Custom subtotal function of a subtotal line (`None` is the automatic subtotal).
    pub func: Option<Func>,
    /// Axis level the line belongs to.
    pub level: usize,
    /// Item position per level up to the line's own (the data field index at a Values level).
    pub path: Vec<usize>,
    /// Data field the line stands for.
    pub data: Option<usize>,
}

fn axis_items(lines: &[Line]) -> Vec<AxisLine> {
    lines
        .iter()
        .map(|l| AxisLine {
            kind: match l.kind {
                Kind::Item => LineKind::Item,
                Kind::Subtotal => LineKind::Subtotal,
                Kind::Grand => LineKind::Grand,
                Kind::Blank => LineKind::Blank,
                Kind::DataLeaf => LineKind::DataLeaf,
            },
            func: l.func,
            level: l.level,
            path: l.path.clone(),
            data: l.data,
        })
        .collect()
}

fn level_fields(levels: &[Lvl]) -> Vec<Option<usize>> {
    levels
        .iter()
        .map(|l| match l {
            Lvl::Field(f) => Some(*f),
            Lvl::Values => None,
        })
        .collect()
}

/// What a value cell of the table stands for: a data field at an intersection of items.
#[derive(Debug, Clone, PartialEq)]
pub struct DataCell {
    pub row: u32,
    pub col: u32,
    /// Index into the table's data fields.
    pub data: usize,
    /// (pivot field index, item value) constraints; empty for the grand total.
    pub items: Vec<(usize, Value)>,
}

/// What a row or column line of the body is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// An item label line (with its values when `values` is set).
    Item,
    Subtotal,
    Grand,
    /// The empty line after an item group.
    Blank,
    /// A data field line or column.
    DataLeaf,
}

/// Structure of one row or column line of the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineInfo {
    pub kind: LineKind,
    /// Depth of the axis level the line belongs to (0 outermost).
    pub level: usize,
    /// The line is of the innermost level.
    pub leaf: bool,
    /// The line carries values.
    pub values: bool,
    /// Leading item constraints shared with the next line (how many enclosing groups go on).
    pub shared_next: usize,
}

impl LineInfo {
    fn of(l: &Line) -> Self {
        Self {
            kind: match l.kind {
                Kind::Item => LineKind::Item,
                Kind::Subtotal => LineKind::Subtotal,
                Kind::Grand => LineKind::Grand,
                Kind::Blank => LineKind::Blank,
                Kind::DataLeaf => LineKind::DataLeaf,
            },
            level: l.level,
            leaf: l.leaf,
            values: l.values,
            shared_next: 0,
        }
    }
}

/// Line infos of an axis, with the number of constraints each line shares with the next.
fn line_infos(lines: &[Line]) -> Vec<LineInfo> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut info = LineInfo::of(l);
            info.shared_next = lines.get(i + 1).map_or(0, |n| {
                l.cons
                    .iter()
                    .zip(&n.cons)
                    .take_while(|(a, b)| a == b)
                    .count()
            });
            info
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Item,
    Subtotal,
    Grand,
    Blank,
    DataLeaf,
}

#[derive(Debug, Clone)]
enum Label {
    Item(usize, Value),
    Text(String),
}

#[derive(Debug, Clone)]
struct Line {
    kind: Kind,
    /// Depth of the axis level the line belongs to.
    level: usize,
    /// An item line of the innermost level, or a data line.
    leaf: bool,
    recs: Vec<u32>,
    func: Option<Func>,
    data: Option<usize>,
    /// Item constraints of the line (field, item position), outermost first.
    cons: Cons,
    /// Position of every axis level up to the line's own (data index at a Values level).
    path: Vec<usize>,
    /// Whether data cells of this line carry values.
    values: bool,
    /// (column for row lines, header row for column lines, label)
    labels: Vec<(usize, Label)>,
}

struct Ctx<'a> {
    data: &'a Dataset,
    pt: &'a PivotTable,
    strings: &'a PivotStrings,
    order: Vec<Vec<Value>>,
    /// Position of every record's item per pivot field (only axis fields are filled).
    rpos: Vec<Vec<usize>>,
    /// (pivot field, item position) of the calculated items.
    calc: std::collections::HashSet<(usize, usize)>,
}

fn sub_funcs(ctx: &Ctx, field: usize) -> Vec<Option<Func>> {
    let f = &ctx.pt.fields[field];
    let mut out = Vec::new();
    if f.default_subtotal && f.subtotals.is_empty() {
        out.push(None);
    }
    out.extend(f.subtotals.iter().map(|x| Some(*x)));
    out
}

fn func_index(f: Func) -> usize {
    match f {
        Func::Sum => 0,
        Func::Count => 1,
        Func::Average => 2,
        Func::Max => 3,
        Func::Min => 4,
        Func::Product => 5,
        Func::CountNums => 6,
        Func::StdDev => 7,
        Func::StdDevP => 8,
        Func::Var => 9,
        Func::VarP => 10,
    }
}

fn total_label(ctx: &Ctx, item: &str, func: Option<Func>) -> String {
    match func {
        None => ctx.strings.total_of.replace("{0}", item),
        Some(f) => format!("{item} {}", ctx.strings.subtotal_names[func_index(f)]),
    }
}

fn item_text(ctx: &Ctx, v: &Value) -> String {
    match v {
        Value::Blank => ctx.strings.blank_item.clone(),
        Value::Number(n) => {
            let mut s = String::new();
            open365_xlsx::format_number(*n, &mut s);
            s
        }
        Value::Bool(true) => ctx.strings.true_text.clone(),
        Value::Bool(false) => ctx.strings.false_text.clone(),
        Value::Error(e) | Value::Text(e) => e.clone(),
    }
}

/// State for tabular label suppression.
#[derive(Default)]
struct Walk {
    prev: Vec<usize>,
}

/// One level of an axis: a pivot field or the Values field (the data fields as items).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lvl {
    Field(usize),
    Values,
}

/// Shape of the axis being built.
#[derive(Clone, Copy)]
struct AxisShape {
    is_row: bool,
    tabular: bool,
    compact: bool,
}

fn level_label(ctx: &Ctx, lvl: Lvl, pos: usize) -> Label {
    match lvl {
        Lvl::Field(f) => Label::Item(f, ctx.order[f][pos].clone()),
        Lvl::Values => Label::Text(ctx.pt.data_fields[pos].name.clone()),
    }
}

fn level_fill(ctx: &Ctx, lvl: Lvl) -> bool {
    match lvl {
        Lvl::Field(f) => ctx.pt.fields[f].fill_down,
        Lvl::Values => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_axis(
    ctx: &Ctx,
    levels: &[Lvl],
    level: usize,
    recs: &[u32],
    path: &mut Vec<usize>,
    out: &mut Vec<Line>,
    walk: &mut Walk,
    shape: AxisShape,
    cur: Option<usize>,
) {
    let AxisShape {
        is_row,
        tabular,
        compact,
    } = shape;
    let last = level + 1 == levels.len();
    let stacked = is_row && !tabular;
    // label slot: column for rows, header row for columns
    let slot = |lvl: usize| -> usize {
        if is_row {
            if compact { 0 } else { lvl }
        } else {
            lvl + 1
        }
    };
    // Labels of a leaf line: every level whose label changed since the previous line.
    let leaf_labels = |path: &[usize], walk: &Walk| -> Vec<(usize, Label)> {
        let mut labels = Vec::new();
        let fill = (0..level).any(|l| level_fill(ctx, levels[l]));
        if stacked && !fill {
            labels.push((slot(level), level_label(ctx, levels[level], path[level])));
        } else {
            for l in 0..=level {
                let changed = !stacked && (walk.prev.len() <= l || walk.prev[..=l] != path[..=l])
                    || level_fill(ctx, levels[l])
                    || (stacked && l == level)
                    || (fill && l < level);
                if changed {
                    labels.push((slot(l), level_label(ctx, levels[l], path[l])));
                }
            }
        }
        labels
    };
    // Subtotal lines repeat the outer labels when "repeat all labels" is on (tabular and the like).
    let sub_labels = |path: &[usize], text: String| -> Vec<(usize, Label)> {
        let mut labels = Vec::new();
        if !stacked {
            for l in 0..level {
                if level_fill(ctx, levels[l]) {
                    labels.push((slot(l), level_label(ctx, levels[l], path[l])));
                }
            }
        }
        labels.push((slot(level), Label::Text(text)));
        labels
    };
    let cons_at = |path: &[usize]| -> Cons {
        path.iter()
            .enumerate()
            .filter_map(|(l, p)| match levels[l] {
                Lvl::Field(f) => Some((f, *p)),
                Lvl::Values => None,
            })
            .collect()
    };
    let field = match levels[level] {
        Lvl::Values => {
            for j in 0..ctx.pt.data_fields.len() {
                path.push(j);
                if last {
                    let labels = leaf_labels(path, walk);
                    walk.prev = path.clone();
                    out.push(Line {
                        kind: Kind::DataLeaf,
                        level,
                        leaf: true,
                        recs: recs.to_vec(),
                        func: None,
                        data: Some(j),
                        cons: cons_at(path),
                        path: path.clone(),
                        values: true,
                        labels,
                    });
                } else {
                    if stacked {
                        out.push(Line {
                            kind: Kind::Item,
                            level,
                            leaf: false,
                            recs: recs.to_vec(),
                            func: None,
                            data: Some(j),
                            cons: cons_at(path),
                            path: path.clone(),
                            values: false,
                            labels: vec![(slot(level), level_label(ctx, Lvl::Values, j))],
                        });
                    }
                    build_axis(
                        ctx,
                        levels,
                        level + 1,
                        recs,
                        path,
                        out,
                        walk,
                        shape,
                        Some(j),
                    );
                }
                path.pop();
            }
            return;
        }
        Lvl::Field(f) => f,
    };
    let n_items = ctx.order[field].len();
    let values_below = cur.is_none() && levels[level + 1..].contains(&Lvl::Values);
    // Only Values follows: the next level holds the data lines, so this field has no subtotals.
    let leaf_parent = levels[level + 1..].iter().all(|l| *l == Lvl::Values);
    let pf = &ctx.pt.fields[field];
    let mut positions: Vec<usize> = (0..n_items).collect();
    if let Some(j) = pf.sort_by
        && let Some(df) = ctx.pt.data_fields.get(j)
    {
        let col = ctx
            .data
            .columns
            .get(df.fld as usize)
            .map_or(&[][..], Vec::as_slice);
        let key = |pos: usize| -> Option<f64> {
            let vals: Vec<&Value> = recs
                .iter()
                .filter(|r| ctx.rpos[field][**r as usize] == pos)
                .filter_map(|r| col.get(*r as usize))
                .collect();
            match aggregate(df.func, &vals) {
                Some(GridValue::Number(n)) => Some(n),
                _ => None,
            }
        };
        let keys: Vec<Option<f64>> = (0..n_items).map(key).collect();
        let desc = pf.sort_type == "descending";
        positions.sort_by(|a, b| match (keys[*a], keys[*b]) {
            (Some(x), Some(y)) => {
                let o = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
                if desc { o.reverse() } else { o }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        });
    }
    for pos in positions {
        // A calculated item has no records of its own: its lines see all records of the parent.
        let sub: Vec<u32> = if ctx.calc.contains(&(field, pos)) {
            recs.to_vec()
        } else {
            recs.iter()
                .copied()
                .filter(|r| ctx.rpos[field][*r as usize] == pos)
                .collect()
        };
        if sub.is_empty() {
            continue;
        }
        path.push(pos);
        let item_label = level_label(ctx, levels[level], pos);
        let funcs = sub_funcs(ctx, field);
        if last {
            let labels = leaf_labels(path, walk);
            walk.prev = path.clone();
            out.push(Line {
                kind: Kind::Item,
                level,
                leaf: true,
                recs: sub,
                func: None,
                data: cur,
                cons: cons_at(path),
                path: path.clone(),
                values: true,
                labels,
            });
        } else {
            let top_auto = stacked
                && pf.subtotal_top
                && pf.default_subtotal
                && pf.subtotals.is_empty()
                && !values_below;
            if stacked {
                out.push(Line {
                    kind: Kind::Item,
                    level,
                    leaf: false,
                    recs: sub.clone(),
                    func: None,
                    data: cur,
                    cons: cons_at(path),
                    path: path.clone(),
                    values: top_auto,
                    labels: vec![(slot(level), item_label)],
                });
            }
            build_axis(ctx, levels, level + 1, &sub, path, out, walk, shape, cur);
            let name = item_text(ctx, &ctx.order[field][pos]);
            if !top_auto && !leaf_parent {
                if values_below {
                    if !funcs.is_empty() {
                        for (j, df) in ctx.pt.data_fields.iter().enumerate() {
                            out.push(Line {
                                kind: Kind::Subtotal,
                                level,
                                leaf: false,
                                recs: sub.clone(),
                                func: None,
                                data: Some(j),
                                cons: cons_at(path),
                                path: path.clone(),
                                values: true,
                                labels: sub_labels(path, format!("{name} {}", df.name)),
                            });
                        }
                    }
                } else {
                    for f in &funcs {
                        out.push(Line {
                            kind: Kind::Subtotal,
                            level,
                            leaf: false,
                            recs: sub.clone(),
                            func: *f,
                            data: cur,
                            cons: cons_at(path),
                            path: path.clone(),
                            values: true,
                            labels: sub_labels(path, total_label(ctx, &name, *f)),
                        });
                    }
                }
            }
            if is_row && pf.insert_blank_row {
                out.push(Line {
                    kind: Kind::Blank,
                    level,
                    leaf: false,
                    recs: Vec::new(),
                    func: None,
                    data: None,
                    cons: cons_at(path),
                    path: path.clone(),
                    values: false,
                    labels: Vec::new(),
                });
            }
        }
        path.pop();
    }
}

/// Sorts items the way a sorted pivot field orders them (ascending, or descending reversed).
pub(crate) fn sort_items<T>(items: &mut [(Value, T)], descending: bool) {
    let Ok(collator) = Locale::en_us().collator_with(CollationStrength::Secondary) else {
        return;
    };
    let text_cmp = |a: &str, b: &str| collator.compare(a, b);
    items.sort_by(|a, b| compare_values(&a.0, &b.0, &text_cmp));
    if descending {
        items.reverse();
    }
}

/// Computes the pivot output grid.
#[allow(clippy::needless_range_loop)]
pub fn compute(
    pt: &PivotTable,
    cache: &CacheDefinition,
    data: &Dataset,
    strings: &PivotStrings,
    mode: OrderMode,
) -> Result<PivotGrid, PivotError> {
    if !cache.calculated_items.is_empty()
        && pt
            .data_fields
            .iter()
            .any(|d| matches!(d.func, Func::Average | Func::StdDev | Func::Var))
    {
        // Excel refuses these functions while the table has calculated items.
        return Err(PivotError::Unsupported(
            "Average, StdDev and Var with calculated items",
        ));
    }
    let collator = Locale::en_us()
        .collator_with(CollationStrength::Secondary)
        .map_err(|_| PivotError::Unsupported("collation data"))?;
    let text_cmp = |a: &str, b: &str| collator.compare(a, b);
    // Grouped axis fields read their group item instead of the source value.
    let mut owned: Option<Dataset> = None;
    for (fi, cf) in cache.fields.iter().enumerate() {
        let on_axis = pt
            .fields
            .get(fi)
            .is_some_and(|p| matches!(p.axis, Some(Axis::Row | Axis::Col | Axis::Page)));
        let Some(g) = cf.group.as_ref().filter(|_| on_axis) else {
            continue;
        };
        let base = if cf.database_field {
            fi
        } else {
            g.base.map_or(fi, |b| b as usize)
        };
        let src = data.columns.get(base).map_or(&[][..], Vec::as_slice);
        if let Some(col) = grouped_column(cf, src, false) {
            owned.get_or_insert_with(|| data.clone()).columns[fi] = col;
        }
    }
    let data: &Dataset = owned.as_ref().unwrap_or(data);
    let nf = pt.fields.len();
    // Item order per pivot field.
    let mut order: Vec<Vec<Value>> = vec![Vec::new(); nf];
    for (fi, pf) in pt.fields.iter().enumerate() {
        let Some(cf) = cache.fields.get(fi) else {
            continue;
        };
        let stored: Vec<Value> = pf
            .items
            .iter()
            .filter_map(|i| i.x)
            .map(|x| {
                cf.item_values()
                    .get(x as usize)
                    .cloned()
                    .unwrap_or(Value::Blank)
            })
            .collect();
        let on_axis = matches!(pf.axis, Some(Axis::Row | Axis::Col | Axis::Page));
        if !on_axis {
            continue;
        }
        if mode == OrderMode::Stored && !stored.is_empty() {
            order[fi] = stored;
            continue;
        }
        if cf.group.as_ref().is_some_and(|g| !g.items.is_empty()) {
            let mut items = cf.item_values().to_vec();
            if pf.sort_type == "descending" {
                items.reverse();
            }
            order[fi] = items;
            continue;
        }
        let mut seen: HashMap<ItemKey, ()> = HashMap::new();
        let mut uniq: Vec<Value> = Vec::new();
        let calc_values = cache
            .calculated_items
            .iter()
            .filter(|c| c.field == fi)
            .filter_map(|c| cf.item_values().get(c.item as usize));
        for v in data
            .columns
            .get(fi)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .chain(calc_values)
        {
            if seen.insert(ItemKey::of(v), ()).is_none() {
                uniq.push(v.clone());
            }
        }
        if pf.sort_type == "manual" && !stored.is_empty() {
            // The refreshed cache spells an item as its first row does.
            let spelling: HashMap<ItemKey, &Value> =
                uniq.iter().map(|v| (ItemKey::of(v), v)).collect();
            let mut list: Vec<Value> = stored
                .into_iter()
                .map(|v| {
                    spelling
                        .get(&ItemKey::of(&v))
                        .map_or(v.clone(), |s| (*s).clone())
                })
                .collect();
            let known: HashMap<ItemKey, ()> = list.iter().map(|v| (ItemKey::of(v), ())).collect();
            // New items follow the stored ones in order of first appearance in the source.
            let extra = uniq
                .into_iter()
                .filter(|v| !known.contains_key(&ItemKey::of(v)));
            list.extend(extra);
            order[fi] = list;
        } else {
            uniq.sort_by(|a, b| compare_values(a, b, &text_cmp));
            if pf.sort_type == "descending" {
                uniq.reverse();
            }
            order[fi] = uniq;
        }
    }
    // Which records are alive: page filters and hidden items.
    let n = data.rows;
    let mut alive: Vec<bool> = vec![true; n];
    let mut rpos: Vec<Vec<usize>> = vec![Vec::new(); nf];
    for fi in 0..nf {
        if order[fi].is_empty()
            && !matches!(pt.fields[fi].axis, Some(Axis::Row | Axis::Col | Axis::Page))
        {
            continue;
        }
        let map: HashMap<ItemKey, usize> = order[fi]
            .iter()
            .enumerate()
            .map(|(i, v)| (ItemKey::of(v), i))
            .collect();
        let hidden: Vec<bool> = {
            let mut h = vec![false; order[fi].len()];
            let mut k = 0;
            for it in &pt.fields[fi].items {
                if it.x.is_some() {
                    if it.hidden && k < h.len() {
                        h[k] = true;
                    }
                    k += 1;
                }
            }
            h
        };
        let col = data.columns.get(fi).map_or(&[][..], Vec::as_slice);
        let mut pos = vec![usize::MAX; n];
        for r in 0..n {
            let p = col.get(r).and_then(|v| map.get(&ItemKey::of(v)).copied());
            match p {
                Some(p) => {
                    pos[r] = p;
                    if hidden[p] {
                        alive[r] = false;
                    }
                }
                None => alive[r] = false,
            }
        }
        rpos[fi] = pos;
    }
    for pg in &pt.page_fields {
        let fi = pg.fld as usize;
        if let (Some(sel), Some(pf)) = (pg.item, pt.fields.get(fi)) {
            let want = pf
                .items
                .get(sel as usize)
                .and_then(|i| i.x)
                .and_then(|x| {
                    cache
                        .fields
                        .get(fi)
                        .and_then(|c| c.item_values().get(x as usize))
                })
                .map(ItemKey::of);
            if let Some(want) = want {
                let col = data.columns.get(fi).map_or(&[][..], Vec::as_slice);
                for r in 0..n {
                    if col.get(r).map(ItemKey::of).as_ref() != Some(&want) {
                        alive[r] = false;
                    }
                }
            }
        }
    }
    let alive_recs: Vec<u32> = (0..n as u32).filter(|r| alive[*r as usize]).collect();
    // Calculated items: formula per (field, display position).
    let mut calc_formulas: HashMap<(usize, usize), crate::calc::Expr> = HashMap::new();
    for ci in &cache.calculated_items {
        let Some(want) = cache
            .fields
            .get(ci.field)
            .and_then(|f| f.item_values().get(ci.item as usize))
            .map(ItemKey::of)
        else {
            continue;
        };
        let on_axis = pt
            .fields
            .get(ci.field)
            .is_some_and(|p| matches!(p.axis, Some(Axis::Row | Axis::Col | Axis::Page)));
        let Some(pos) = order
            .get(ci.field)
            .filter(|_| on_axis)
            .and_then(|o| o.iter().position(|v| ItemKey::of(v) == want))
        else {
            continue;
        };
        if let Some(expr) = crate::calc::parse(&ci.formula) {
            calc_formulas.insert((ci.field, pos), expr);
        }
    }
    let ctx = Ctx {
        data,
        pt,
        strings,
        order,
        rpos,
        calc: calc_formulas.keys().copied().collect(),
    };
    let item_calc = (!calc_formulas.is_empty()).then(|| {
        let mut fields: Vec<usize> = calc_formulas.keys().map(|(f, _)| *f).collect();
        fields.sort_unstable();
        fields.dedup();
        CalcItems {
            rpos: &ctx.rpos,
            names: cache.fields.iter().map(|f| f.name.to_lowercase()).collect(),
            items: ctx
                .order
                .iter()
                .map(|o| {
                    o.iter()
                        .map(|v| item_text(&ctx, v).to_lowercase())
                        .collect()
                })
                .collect(),
            formulas: calc_formulas,
            fields,
        }
    });

    let to_levels = |list: &[u32]| -> Vec<Lvl> {
        list.iter()
            .map(|f| {
                if *f == VALUES_FIELD {
                    Lvl::Values
                } else {
                    Lvl::Field(*f as usize)
                }
            })
            .collect()
    };
    let mut row_levels = to_levels(&pt.row_fields);
    let mut col_levels = to_levels(&pt.col_fields);
    let nd = pt.data_fields.len();
    if nd <= 1 {
        row_levels.retain(|l| *l != Lvl::Values);
        col_levels.retain(|l| *l != Lvl::Values);
    } else if !row_levels.contains(&Lvl::Values) && !col_levels.contains(&Lvl::Values) {
        if pt.data_on_rows {
            row_levels.push(Lvl::Values);
        } else {
            col_levels.push(Lvl::Values);
        }
    }
    let values_on_rows = row_levels.contains(&Lvl::Values);
    let values_on_cols = col_levels.contains(&Lvl::Values);
    let col_has_fields = col_levels.iter().any(|l| matches!(l, Lvl::Field(_)));
    let tabular = !pt.compact && !pt.outline;
    let compact = pt.compact;
    let nlabel = if row_levels.is_empty() {
        usize::from(col_has_fields && nd == 1)
    } else if compact {
        1
    } else {
        row_levels.len()
    };
    let gt = pt
        .grand_total_caption
        .clone()
        .unwrap_or_else(|| strings.grand_total.clone());
    let grand_lines = |slot: usize, with_values: bool| -> Vec<Line> {
        let mk = |data: Option<usize>, text: String| Line {
            kind: Kind::Grand,
            level: 0,
            leaf: false,
            recs: alive_recs.clone(),
            func: None,
            data,
            cons: Vec::new(),
            path: Vec::new(),
            values: true,
            labels: vec![(slot, Label::Text(text))],
        };
        if with_values {
            pt.data_fields
                .iter()
                .enumerate()
                .map(|(j, df)| {
                    mk(
                        Some(j),
                        format!("{} {}", strings.grand_total_prefix, df.name),
                    )
                })
                .collect()
        } else {
            vec![mk(None, gt.clone())]
        }
    };

    // Row lines.
    let mut rows: Vec<Line> = Vec::new();
    if row_levels.is_empty() {
        rows.push(Line {
            kind: Kind::Grand,
            level: 0,
            leaf: false,
            recs: alive_recs.clone(),
            func: None,
            data: None,
            cons: Vec::new(),
            path: Vec::new(),
            values: true,
            labels: if nlabel == 1 {
                vec![(0, Label::Text(pt.data_fields[0].name.clone()))]
            } else {
                Vec::new()
            },
        });
    } else {
        build_axis(
            &ctx,
            &row_levels,
            0,
            &alive_recs,
            &mut Vec::new(),
            &mut rows,
            &mut Walk::default(),
            AxisShape {
                is_row: true,
                tabular,
                compact,
            },
            None,
        );
        // A group that ends together with its parent gets one blank row, not one per level.
        let mut k = 0;
        while k + 1 < rows.len() {
            if rows[k].kind == Kind::Blank && rows[k + 1].kind == Kind::Blank {
                rows.remove(k);
            } else {
                k += 1;
            }
        }
        if pt.row_grand_totals && row_levels.iter().any(|l| matches!(l, Lvl::Field(_))) {
            rows.extend(grand_lines(0, values_on_rows));
        }
    }

    // Column lines.
    let mut cols: Vec<Line> = Vec::new();
    if !col_has_fields {
        if values_on_rows {
            cols.push(Line {
                kind: Kind::DataLeaf,
                level: 0,
                leaf: true,
                recs: alive_recs.clone(),
                func: None,
                data: None,
                cons: Vec::new(),
                path: Vec::new(),
                values: true,
                labels: Vec::new(),
            });
        } else {
            for (j, df) in pt.data_fields.iter().enumerate() {
                cols.push(Line {
                    kind: Kind::DataLeaf,
                    level: 0,
                    leaf: true,
                    recs: alive_recs.clone(),
                    func: None,
                    data: Some(j),
                    cons: Vec::new(),
                    path: if nd > 1 { vec![j] } else { Vec::new() },
                    values: true,
                    labels: vec![(0, Label::Text(df.name.clone()))],
                });
            }
        }
    } else {
        build_axis(
            &ctx,
            &col_levels,
            0,
            &alive_recs,
            &mut Vec::new(),
            &mut cols,
            &mut Walk::default(),
            AxisShape {
                is_row: false,
                tabular: true,
                compact: false,
            },
            None,
        );
        if pt.col_grand_totals {
            cols.extend(grand_lines(1, values_on_cols));
        }
    }

    let hdr_rows = if col_has_fields {
        col_levels.len() + 1
    } else {
        1
    };
    let (or, oc) = {
        let plain: String = pt.reference.chars().filter(|c| *c != '$').collect();
        let first = plain.split(':').next().unwrap_or("A1");
        let (r, c) = parse_cell_ref(first).unwrap_or((1, 1));
        (r, c)
    };
    let mut cells: Vec<GridCell> = Vec::new();
    let mut put = |r: usize, c: usize, value: GridValue, fmt: u32| {
        cells.push(GridCell {
            row: or + r as u32,
            col: oc + c as u32,
            value,
            num_fmt_id: fmt,
        });
    };
    let label_cell = |ctx: &Ctx, lb: &Label| -> (GridValue, u32) {
        match lb {
            Label::Text(t) => (GridValue::Text(t.clone()), 0),
            Label::Item(f, v) => match v {
                Value::Number(x) => (GridValue::Number(*x), ctx.pt.fields[*f].num_fmt_id),
                other => (GridValue::Text(item_text(ctx, other)), 0),
            },
        }
    };
    // Header area.
    let level_caption = |l: &Lvl| -> String {
        match l {
            Lvl::Field(f) => cache
                .fields
                .get(*f)
                .map_or("", |c| c.name.as_str())
                .to_owned(),
            Lvl::Values => pt.data_caption.clone(),
        }
    };
    let row_hdr = |put: &mut dyn FnMut(usize, usize, GridValue, u32), r: usize| {
        if row_levels.is_empty() {
            return;
        }
        if compact {
            let t = pt
                .row_header_caption
                .clone()
                .unwrap_or_else(|| strings.row_labels.clone());
            let only_values = row_levels.iter().all(|l| *l == Lvl::Values);
            let t = if only_values {
                pt.data_caption.clone()
            } else {
                t
            };
            put(r, 0, GridValue::Text(t), 0);
        } else {
            for (k, l) in row_levels.iter().enumerate() {
                put(r, k, GridValue::Text(level_caption(l)), 0);
            }
        }
    };
    if !col_has_fields {
        row_hdr(&mut put, 0);
        for (j, line) in cols.iter().enumerate() {
            for (_, lb) in &line.labels {
                let (v, f) = label_cell(&ctx, lb);
                put(0, nlabel + j, v, f);
            }
        }
    } else {
        if nd == 1 && !row_levels.is_empty() {
            put(0, 0, GridValue::Text(pt.data_fields[0].name.clone()), 0);
        }
        if compact {
            let t = pt
                .col_header_caption
                .clone()
                .unwrap_or_else(|| strings.column_labels.clone());
            put(0, nlabel, GridValue::Text(t), 0);
        } else {
            for (k, l) in col_levels.iter().enumerate() {
                put(0, nlabel + k, GridValue::Text(level_caption(l)), 0);
            }
        }
        row_hdr(&mut put, hdr_rows - 1);
        for (j, line) in cols.iter().enumerate() {
            for (hr, lb) in &line.labels {
                let (v, f) = label_cell(&ctx, lb);
                put(*hr, nlabel + j, v, f);
            }
        }
    }
    // Rows and data.
    let calc = if pt.data_fields.iter().any(|d| Mode::of(d) != Mode::Normal) {
        let row_cons: Vec<&Cons> = rows.iter().map(|l| &l.cons).collect();
        let col_cons: Vec<&Cons> = cols.iter().map(|l| &l.cons).collect();
        Some(Calc::new(
            pt,
            cache,
            &data.columns,
            &ctx.rpos,
            &ctx.order,
            &alive_recs,
            &row_cons,
            &col_cons,
        ))
    } else {
        None
    };
    let calc_exprs: Vec<Option<crate::calc::Expr>> = cache
        .fields
        .iter()
        .map(|f| {
            f.formula
                .as_deref()
                .filter(|_| !f.database_field)
                .and_then(crate::calc::parse)
        })
        .collect();
    let field_names: Vec<String> = cache.fields.iter().map(|f| f.name.to_lowercase()).collect();
    let mut data_cells: Vec<DataCell> = Vec::new();
    let mut mark = vec![false; n];
    // Row and column axis fields (the page fields are filters, not totals to split).
    let axis_fields: Vec<usize> = row_levels
        .iter()
        .chain(col_levels.iter())
        .filter_map(|l| match l {
            Lvl::Field(f) => Some(*f),
            Lvl::Values => None,
        })
        .collect();
    for (i, rl) in rows.iter().enumerate() {
        for (slot, lb) in &rl.labels {
            let (v, f) = label_cell(&ctx, lb);
            put(hdr_rows + i, *slot, v, f);
        }
        if rl.kind == Kind::Blank || !rl.values {
            continue;
        }
        for r in &rl.recs {
            mark[*r as usize] = true;
        }
        for (j, cl) in cols.iter().enumerate() {
            let di = rl.data.or(cl.data).unwrap_or(0);
            let Some(df) = pt.data_fields.get(di) else {
                continue;
            };
            let func = rl.func.or(cl.func).unwrap_or(df.func);
            let recs: Vec<u32> = cl
                .recs
                .iter()
                .copied()
                .filter(|r| mark[*r as usize])
                .collect();
            let cell_calcs = item_calc
                .as_ref()
                .map(|ic| ic.calcs_of(&[rl.cons.as_slice(), cl.cons.as_slice()].concat()));
            let cell_free = item_calc
                .as_ref()
                .zip(cell_calcs.as_ref())
                .map(|(ic, c)| ic.free(c, &[rl.cons.as_slice(), cl.cons.as_slice()]));
            let cell_split: Vec<usize> = item_calc.as_ref().map_or_else(Vec::new, |ic| {
                axis_fields
                    .iter()
                    .copied()
                    .filter(|f| {
                        !ic.fields.contains(f)
                            && !rl.cons.iter().chain(cl.cons.iter()).any(|(cf, _)| cf == f)
                    })
                    .collect()
            });
            let item_ctx = match (item_calc.as_ref(), cell_calcs.as_ref(), cell_free.as_ref()) {
                (Some(items), Some(calcs), Some(free)) => Some(ItemCtx {
                    items,
                    calcs,
                    free,
                    split: &cell_split,
                }),
                _ => None,
            };
            let col = data
                .columns
                .get(df.fld as usize)
                .map_or(&[][..], Vec::as_slice);
            let v = if let Some(Some(expr)) = calc_exprs.get(df.fld as usize) {
                FieldEval {
                    exprs: &calc_exprs,
                    names: &field_names,
                    columns: &data.columns,
                    items: item_ctx,
                }
                .cell(expr, &recs)
            } else {
                let plain = |rs: &[u32]| -> Option<GridValue> {
                    let vals: Vec<&Value> =
                        rs.iter().filter_map(|r| col.get(*r as usize)).collect();
                    aggregate(func, &vals)
                };
                match item_ctx {
                    Some(ic) => ic.items.value(
                        &recs,
                        ic.calcs,
                        ic.free,
                        ic.split,
                        Comb::of(func),
                        &plain,
                        0,
                    ),
                    None => plain(&recs),
                }
            };
            let v = match &calc {
                Some(c) => c.apply(di, &rl.cons, &cl.cons, v),
                None => v,
            };
            if let Some(v) = v {
                data_cells.push(DataCell {
                    row: or + (hdr_rows + i) as u32,
                    col: oc + (nlabel + j) as u32,
                    data: di,
                    items: rl
                        .cons
                        .iter()
                        .chain(cl.cons.iter())
                        .filter_map(|(f, p)| Some((*f, ctx.order[*f].get(*p)?.clone())))
                        .collect(),
                });
                put(hdr_rows + i, nlabel + j, v, df.num_fmt_id);
            }
        }
        for r in &rl.recs {
            mark[*r as usize] = false;
        }
    }
    // Page fields.
    let np = pt.page_fields.len();
    for (k, pg) in pt.page_fields.iter().enumerate() {
        let row = or as i64 - 1 - np as i64 + k as i64;
        let name = cache
            .fields
            .get(pg.fld as usize)
            .map_or("", |c| c.name.as_str());
        let sel = pg.item.and_then(|s| {
            let pf = pt.fields.get(pg.fld as usize)?;
            let x = pf.items.get(s as usize)?.x?;
            cache
                .fields
                .get(pg.fld as usize)?
                .item_values()
                .get(x as usize)
                .cloned()
        });
        let text = sel.map_or_else(|| strings.all_items.clone(), |v| item_text(&ctx, &v));
        if row >= 1 {
            cells.push(GridCell {
                row: row as u32,
                col: oc,
                value: GridValue::Text(name.to_owned()),
                num_fmt_id: 0,
            });
            cells.push(GridCell {
                row: row as u32,
                col: oc + 1,
                value: GridValue::Text(text),
                num_fmt_id: 0,
            });
        }
    }
    cells.sort_by_key(|c| (c.row, c.col));
    let last_row = or + (hdr_rows + rows.len()).saturating_sub(1) as u32;
    let last_col = oc + (nlabel + cols.len()).saturating_sub(1) as u32;
    let first_row = if np > 0 { or - 1 - np as u32 } else { or };
    let first_header_row = if col_has_fields {
        1
    } else {
        u32::from(nd <= 1 || values_on_rows)
    };
    let location = Location {
        reference: Location::range_string((or, oc, last_row, last_col)),
        first_header_row,
        first_data_row: hdr_rows as u32,
        first_data_col: nlabel as u32,
        row_page_count: np as u32,
        col_page_count: u32::from(np > 0),
    };
    Ok(PivotGrid {
        cells,
        body: (or, oc, last_row, last_col),
        range: (first_row, oc, last_row, last_col),
        location,
        header_rows: hdr_rows,
        label_cols: nlabel,
        row_lines: line_infos(&rows),
        col_lines: line_infos(&cols),
        data_cells,
        row_items: axis_items(&rows),
        col_items: axis_items(&cols),
        row_levels: level_fields(&row_levels),
        col_levels: level_fields(&col_levels),
        item_order: ctx.order.clone(),
    })
}

/// Calculated items in force for one output cell: the table's items, the ones the cell's lines
/// stand on and the fields whose calculated items its total includes.
#[derive(Clone, Copy)]
struct ItemCtx<'a> {
    items: &'a CalcItems<'a>,
    calcs: &'a [(usize, usize)],
    free: &'a [usize],
    /// Axis fields the cell does not constrain.
    split: &'a [usize],
}

/// Evaluates calculated fields over the records of one output cell.
struct FieldEval<'a> {
    exprs: &'a [Option<crate::calc::Expr>],
    names: &'a [String],
    columns: &'a [Vec<Value>],
    items: Option<ItemCtx<'a>>,
}

impl FieldEval<'_> {
    /// Sum of field `fi` over `recs`; a calculated field is its formula over such sums.
    fn field_value(&self, fi: usize, recs: &[u32], depth: usize) -> crate::calc::Val {
        use crate::calc::{Val, eval};
        if let Some(Some(e)) = self.exprs.get(fi) {
            if depth > 8 {
                return Val::Err("#REF!");
            }
            return eval(e, &|n| self.lookup(n, recs, depth + 1));
        }
        let col = self.columns.get(fi).map_or(&[][..], Vec::as_slice);
        let sum_leaf = |rs: &[u32]| -> Option<GridValue> {
            let mut sum = 0.0;
            for r in rs {
                match col.get(*r as usize) {
                    Some(Value::Number(x)) => sum += x,
                    Some(Value::Error(e)) => return Some(GridValue::Error(e.clone())),
                    _ => {}
                }
            }
            Some(GridValue::Number(sum))
        };
        let v = match self.items {
            Some(ic) => ic
                .items
                .value(recs, ic.calcs, ic.free, &[], Comb::Add, &sum_leaf, 0),
            None => sum_leaf(recs),
        };
        match v {
            Some(GridValue::Error(e)) => Val::Err(match e.as_str() {
                "#DIV/0!" => "#DIV/0!",
                "#N/A" => "#N/A",
                "#NUM!" => "#NUM!",
                _ => "#VALUE!",
            }),
            Some(GridValue::Number(n)) => Val::Num(n),
            _ => Val::Num(0.0),
        }
    }

    fn lookup(&self, name: &str, recs: &[u32], depth: usize) -> crate::calc::Val {
        match self.names.iter().position(|n| *n == name.to_lowercase()) {
            Some(fi) => self.field_value(fi, recs, depth),
            None => crate::calc::Val::Err("#NAME?"),
        }
    }

    /// Value of a calculated field's formula for one output cell. A cell without records stays
    /// empty.
    fn cell(&self, expr: &crate::calc::Expr, recs: &[u32]) -> Option<GridValue> {
        use crate::calc::{Val, eval};
        if recs.is_empty() {
            return None;
        }
        Some(match eval(expr, &|n| self.lookup(n, recs, 1)) {
            Val::Num(n) => GridValue::Number(n),
            Val::Text(t) => GridValue::Text(t),
            Val::Bool(b) => GridValue::Number(f64::from(u8::from(b))),
            Val::Err(e) => GridValue::Error(e.to_owned()),
        })
    }
}

fn num_err(code: &str) -> GridValue {
    GridValue::Error(code.to_owned())
}

/// Aggregates source values; `None` is an empty cell (no non-blank source value).
pub(crate) fn aggregate(func: Func, vals: &[&Value]) -> Option<GridValue> {
    let non_blank = vals.iter().filter(|v| !v.is_blank()).count();
    if non_blank == 0 {
        return None;
    }
    if func == Func::Count {
        return Some(GridValue::Number(non_blank as f64));
    }
    let nums: Vec<f64> = vals.iter().filter_map(|v| v.number()).collect();
    if func == Func::CountNums {
        return Some(GridValue::Number(nums.len() as f64));
    }
    if let Some(Value::Error(e)) = vals.iter().find(|v| matches!(v, Value::Error(_))) {
        return Some(GridValue::Error(e.clone()));
    }
    let n = nums.len();
    let sum: f64 = nums.iter().sum();
    let var = |denom: f64| -> f64 {
        let mean = sum / n as f64;
        nums.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / denom
    };
    Some(match func {
        Func::Sum => GridValue::Number(sum),
        Func::Average if n == 0 => num_err("#DIV/0!"),
        Func::Average => GridValue::Number(sum / n as f64),
        Func::Max => GridValue::Number(
            nums.iter()
                .copied()
                .fold(None, |a: Option<f64>, x| Some(a.map_or(x, |m| m.max(x))))
                .unwrap_or(0.0),
        ),
        Func::Min => GridValue::Number(
            nums.iter()
                .copied()
                .fold(None, |a: Option<f64>, x| Some(a.map_or(x, |m| m.min(x))))
                .unwrap_or(0.0),
        ),
        Func::Product if n == 0 => GridValue::Number(0.0),
        Func::Product => GridValue::Number(nums.iter().product()),
        Func::StdDev if n < 2 => num_err("#DIV/0!"),
        Func::StdDev => GridValue::Number(var(n as f64 - 1.0).sqrt()),
        Func::Var if n < 2 => num_err("#DIV/0!"),
        Func::Var => GridValue::Number(var(n as f64 - 1.0)),
        Func::StdDevP if n < 1 => num_err("#DIV/0!"),
        Func::StdDevP => GridValue::Number(var(n as f64).sqrt()),
        Func::VarP if n < 1 => num_err("#DIV/0!"),
        Func::VarP => GridValue::Number(var(n as f64)),
        Func::Count | Func::CountNums => return None,
    })
}

/// Part names of the pivot tables of a worksheet, read from its relationships.
pub(crate) fn sheet_pivot_parts(wb: &Workbook, sheet: usize) -> Vec<PartName> {
    let Some(part) = wb.sheet_part(sheet) else {
        return Vec::new();
    };
    let Some(rels) = wb
        .package()
        .part(rels_part_name(Some(part)).as_str())
        .and_then(|p| Relationships::parse(p.data()).ok())
    else {
        return Vec::new();
    };
    rels.iter()
        .filter(|r| r.rel_type.ends_with("/pivotTable"))
        .filter_map(|r| resolve_target(Some(part), &r.target).ok())
        .collect()
}
