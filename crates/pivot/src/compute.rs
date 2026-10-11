//! Pivot computation: source data, item order, axis lines and the output grid.

use std::cmp::Ordering;
use std::collections::HashMap;

use open365_locale::{CollationStrength, Locale};
use open365_opc::{PartName, Relationships, rels_part_name, resolve_target};
use open365_xlsx::{Workbook, parse_cell_ref, parse_range};

use crate::error::PivotError;
use crate::model::{
    Axis, CacheDefinition, CacheRecords, CacheSource, Func, PivotTable, Record, VALUES_FIELD,
};
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
    recs: Vec<u32>,
    func: Option<Func>,
    data: Option<usize>,
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

#[allow(clippy::too_many_arguments)]
fn build_axis(
    ctx: &Ctx,
    fields: &[usize],
    level: usize,
    recs: &[u32],
    path: &mut Vec<usize>,
    out: &mut Vec<Line>,
    walk: &mut Walk,
    is_row: bool,
    tabular: bool,
    compact: bool,
) {
    let field = fields[level];
    let last = level + 1 == fields.len();
    let n_items = ctx.order[field].len();
    // label slot: column for rows, header row for columns
    let slot = |lvl: usize| -> usize {
        if is_row {
            if compact { 0 } else { lvl }
        } else {
            lvl + 1
        }
    };
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
        let sub: Vec<u32> = recs
            .iter()
            .copied()
            .filter(|r| ctx.rpos[field][*r as usize] == pos)
            .collect();
        if sub.is_empty() {
            continue;
        }
        path.push(pos);
        let item_label = Label::Item(field, ctx.order[field][pos].clone());
        let funcs = sub_funcs(ctx, field);
        // Row lines in the stacked (compact/outline) forms carry the item on their own line.
        let stacked = is_row && !tabular;
        if last {
            let mut labels = Vec::new();
            let fill = (0..level).any(|l| ctx.pt.fields[fields[l]].fill_down);
            if stacked && !fill {
                labels.push((slot(level), item_label));
            } else {
                for l in 0..=level {
                    let changed = !stacked
                        && (walk.prev.len() <= l || walk.prev[..=l] != path[..=l])
                        || ctx.pt.fields[fields[l]].fill_down
                        || (stacked && l == level)
                        || (fill && l < level);
                    if changed {
                        let fl = fields[l];
                        labels.push((slot(l), Label::Item(fl, ctx.order[fl][path[l]].clone())));
                    }
                }
            }
            walk.prev = path.clone();
            out.push(Line {
                kind: Kind::Item,
                recs: sub,
                func: None,
                data: None,
                values: true,
                labels,
            });
        } else {
            let top_auto =
                stacked && pf.subtotal_top && pf.default_subtotal && pf.subtotals.is_empty();
            if stacked {
                out.push(Line {
                    kind: Kind::Item,
                    recs: sub.clone(),
                    func: None,
                    data: None,
                    values: top_auto,
                    labels: vec![(slot(level), item_label)],
                });
            }
            build_axis(
                ctx,
                fields,
                level + 1,
                &sub,
                path,
                out,
                walk,
                is_row,
                tabular,
                compact,
            );
            let name = item_text(ctx, &ctx.order[field][pos]);
            if !top_auto {
                for f in &funcs {
                    out.push(Line {
                        kind: Kind::Subtotal,
                        recs: sub.clone(),
                        func: *f,
                        data: None,
                        values: true,
                        labels: vec![(slot(level), Label::Text(total_label(ctx, &name, *f)))],
                    });
                }
            }
            if is_row && pf.insert_blank_row {
                out.push(Line {
                    kind: Kind::Blank,
                    recs: Vec::new(),
                    func: None,
                    data: None,
                    values: false,
                    labels: Vec::new(),
                });
            }
        }
        path.pop();
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
    if pt.calculated_items {
        return Err(PivotError::Unsupported("calculated items"));
    }
    let collator = Locale::en_us()
        .collator_with(CollationStrength::Secondary)
        .map_err(|_| PivotError::Unsupported("collation data"))?;
    let text_cmp = |a: &str, b: &str| collator.compare(a, b);
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
            .map(|x| cf.shared.get(x as usize).cloned().unwrap_or(Value::Blank))
            .collect();
        let on_axis = matches!(pf.axis, Some(Axis::Row | Axis::Col | Axis::Page));
        if !on_axis {
            continue;
        }
        if mode == OrderMode::Stored && !stored.is_empty() {
            order[fi] = stored;
            continue;
        }
        let mut seen: HashMap<ItemKey, ()> = HashMap::new();
        let mut uniq: Vec<Value> = Vec::new();
        for v in data.columns.get(fi).map_or(&[][..], Vec::as_slice) {
            if seen.insert(ItemKey::of(v), ()).is_none() {
                uniq.push(v.clone());
            }
        }
        if pf.sort_type == "manual" && !stored.is_empty() {
            let mut list = stored;
            let known: HashMap<ItemKey, ()> = list.iter().map(|v| (ItemKey::of(v), ())).collect();
            let mut extra: Vec<Value> = uniq
                .into_iter()
                .filter(|v| !known.contains_key(&ItemKey::of(v)))
                .collect();
            extra.sort_by(|a, b| compare_values(a, b, &text_cmp));
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
                .and_then(|x| cache.fields.get(fi).and_then(|c| c.shared.get(x as usize)))
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
    let ctx = Ctx {
        data,
        pt,
        strings,
        order,
        rpos,
    };

    let row_fields: Vec<usize> = pt
        .row_fields
        .iter()
        .filter(|f| **f != VALUES_FIELD)
        .map(|f| *f as usize)
        .collect();
    let col_fields: Vec<usize> = pt
        .col_fields
        .iter()
        .filter(|f| **f != VALUES_FIELD)
        .map(|f| *f as usize)
        .collect();
    if pt.data_on_rows && pt.data_fields.len() > 1 {
        return Err(PivotError::Unsupported("data fields on rows"));
    }
    let nd = pt.data_fields.len();
    let tabular = !pt.compact && !pt.outline;
    let compact = pt.compact;
    let nlabel = if row_fields.is_empty() {
        usize::from(!col_fields.is_empty() && pt.data_fields.len() == 1)
    } else if compact {
        1
    } else {
        row_fields.len()
    };
    let gt = pt
        .grand_total_caption
        .clone()
        .unwrap_or_else(|| strings.grand_total.clone());

    // Row lines.
    let mut rows: Vec<Line> = Vec::new();
    if row_fields.is_empty() {
        rows.push(Line {
            kind: Kind::Grand,
            recs: alive_recs.clone(),
            func: None,
            data: None,
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
            &row_fields,
            0,
            &alive_recs,
            &mut Vec::new(),
            &mut rows,
            &mut Walk::default(),
            true,
            tabular,
            compact,
        );
        if pt.row_grand_totals {
            rows.push(Line {
                kind: Kind::Grand,
                recs: alive_recs.clone(),
                func: None,
                data: None,
                values: true,
                labels: vec![(0, Label::Text(gt.clone()))],
            });
        }
    }

    // Column lines.
    let mut cols: Vec<Line> = Vec::new();
    let nc = col_fields.len();
    if nc == 0 {
        for (j, df) in pt.data_fields.iter().enumerate() {
            cols.push(Line {
                kind: Kind::DataLeaf,
                recs: alive_recs.clone(),
                func: None,
                data: Some(j),
                values: true,
                labels: vec![(0, Label::Text(df.name.clone()))],
            });
        }
    } else {
        let mut base: Vec<Line> = Vec::new();
        build_axis(
            &ctx,
            &col_fields,
            0,
            &alive_recs,
            &mut Vec::new(),
            &mut base,
            &mut Walk::default(),
            false,
            true,
            false,
        );
        if pt.col_grand_totals {
            base.push(Line {
                kind: Kind::Grand,
                recs: alive_recs.clone(),
                func: None,
                data: None,
                values: true,
                labels: vec![(1, Label::Text(gt.clone()))],
            });
        }
        if nd > 1 {
            for line in base {
                for (j, df) in pt.data_fields.iter().enumerate() {
                    let mut l = line.clone();
                    if line.kind == Kind::Grand {
                        l.labels = vec![(
                            1,
                            Label::Text(format!("{} {}", strings.grand_total_prefix, df.name)),
                        )];
                    } else if line.kind == Kind::Subtotal {
                        let suffix = strings.total_of.replace("{0}", "");
                        for (_, lb) in &mut l.labels {
                            if let Label::Text(t) = lb {
                                let item = t.strip_suffix(suffix.as_str()).unwrap_or(t.as_str());
                                *t = format!("{item} {}", df.name);
                            }
                        }
                    } else if j > 0 {
                        l.labels.clear();
                    }
                    if line.kind != Kind::Grand {
                        l.labels.push((nc + 1, Label::Text(df.name.clone())));
                    }
                    l.data = Some(j);
                    cols.push(l);
                }
            }
        } else {
            cols = base;
        }
    }

    let hdr_rows = if nc == 0 {
        1
    } else {
        nc + usize::from(nd > 1) + 1
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
    let row_hdr = |put: &mut dyn FnMut(usize, usize, GridValue, u32), r: usize| {
        if row_fields.is_empty() {
            return;
        }
        if compact {
            let t = pt
                .row_header_caption
                .clone()
                .unwrap_or_else(|| strings.row_labels.clone());
            put(r, 0, GridValue::Text(t), 0);
        } else {
            for (k, f) in row_fields.iter().enumerate() {
                let name = cache.fields.get(*f).map_or("", |c| c.name.as_str());
                put(r, k, GridValue::Text(name.to_owned()), 0);
            }
        }
    };
    if nc == 0 {
        row_hdr(&mut put, 0);
        for (j, line) in cols.iter().enumerate() {
            for (_, lb) in &line.labels {
                let (v, f) = label_cell(&ctx, lb);
                put(0, nlabel + j, v, f);
            }
        }
    } else {
        if nd == 1 && !row_fields.is_empty() {
            put(0, 0, GridValue::Text(pt.data_fields[0].name.clone()), 0);
        }
        if compact {
            let t = pt
                .col_header_caption
                .clone()
                .unwrap_or_else(|| strings.column_labels.clone());
            put(0, nlabel, GridValue::Text(t), 0);
        } else {
            for (k, f) in col_fields.iter().enumerate() {
                let name = cache.fields.get(*f).map_or("", |c| c.name.as_str());
                put(0, nlabel + k, GridValue::Text(name.to_owned()), 0);
            }
            if nd > 1 {
                put(0, nlabel + nc, GridValue::Text(pt.data_caption.clone()), 0);
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
    let mut mark = vec![false; n];
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
            let di = cl.data.unwrap_or(0);
            let Some(df) = pt.data_fields.get(di) else {
                continue;
            };
            let func = rl.func.or(cl.func).unwrap_or(df.func);
            let col = data
                .columns
                .get(df.fld as usize)
                .map_or(&[][..], Vec::as_slice);
            let vals: Vec<&Value> = cl
                .recs
                .iter()
                .filter(|r| mark[**r as usize])
                .filter_map(|r| col.get(*r as usize))
                .collect();
            if let Some(v) = aggregate(func, &vals) {
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
                .shared
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
    Ok(PivotGrid {
        cells,
        body: (or, oc, last_row, last_col),
        range: (first_row, oc, last_row, last_col),
    })
}

fn num_err(code: &str) -> GridValue {
    GridValue::Error(code.to_owned())
}

/// Aggregates source values; `None` is an empty cell (no non-blank source value).
fn aggregate(func: Func, vals: &[&Value]) -> Option<GridValue> {
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
