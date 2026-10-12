//! Field grouping: maps source values to the group items stored in `fieldGroup/groupItems`
//! (numeric ranges, and date parts: seconds, minutes, hours, days, months, quarters, years).

use crate::model::{CacheField, FieldGroup};
use crate::value::{Value, serial_to_ymd};

/// Index into `group.items` of the group a source value falls into. Item 0 is the
/// "below the start (or blank)" bucket and the last item the "above the end" bucket.
fn bucket(g: &FieldGroup, v: &Value, date1904: bool) -> usize {
    let last = g.items.len().saturating_sub(1);
    let inner_last = last.saturating_sub(1).max(1);
    let Some(x) = v.number() else {
        return 0;
    };
    match g.group_by.as_deref() {
        None => {
            let (Some(start), Some(end)) = (g.start_num, g.end_num) else {
                return 0;
            };
            let step = g.interval.filter(|i| *i > 0.0).unwrap_or(1.0);
            if x < start {
                0
            } else if x > end {
                last
            } else {
                (1 + (((x - start) / step) + 1e-9).floor() as usize).min(inner_last)
            }
        }
        Some(by) => {
            let (Some(start), Some(end)) = (g.start_date, g.end_date) else {
                return 0;
            };
            if x < start {
                return 0;
            }
            if x > end {
                return last;
            }
            let (y, m, _) = serial_to_ymd(x, date1904);
            let frac = x - x.floor();
            let secs = (frac * 86_400.0).round() as i64;
            let idx = match by {
                "months" => m as usize,
                "quarters" => 1 + ((m - 1) / 3) as usize,
                "years" => {
                    let (sy, _, _) = serial_to_ymd(start, date1904);
                    1 + (y - sy).max(0) as usize
                }
                "days" => match g.interval.filter(|i| *i > 1.0) {
                    Some(n) => 1 + ((x.floor() - start.floor()) / n).floor() as usize,
                    // One day: the day of a leap year (`01-Jan` ... `31-Dec`).
                    None => {
                        let (_, m, d) = serial_to_ymd(x, date1904);
                        1 + LEAP_DAYS[..(m as usize - 1)].iter().sum::<usize>() + d as usize - 1
                    }
                },
                "hours" => 1 + (secs / 3600) as usize % 24,
                "minutes" => 1 + ((secs / 60) % 60) as usize,
                "seconds" => 1 + (secs % 60) as usize,
                _ => return 0,
            };
            idx.min(inner_last)
        }
    }
}

/// The group item each source value belongs to, or `None` when the field has no group items.
pub(crate) fn grouped_column(
    field: &CacheField,
    source: &[Value],
    date1904: bool,
) -> Option<Vec<Value>> {
    let g = field.group.as_ref()?;
    if g.items.is_empty() {
        return None;
    }
    Some(
        source
            .iter()
            .map(|v| g.items[bucket(g, v, date1904).min(g.items.len() - 1)].clone())
            .collect(),
    )
}

fn fmt_date(x: f64) -> String {
    let (y, m, d) = serial_to_ymd(x, false);
    format!("{d:02}/{m:02}/{y}")
}

/// A number as Excel prints it in a group label: 15 significant digits, no float noise.
fn fmt_num(n: f64) -> String {
    let clean = format!("{n:.14e}").parse::<f64>().unwrap_or(n);
    format!("{clean}")
}

/// What a refresh does to the group of one cache field.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Regrouped {
    /// The group with bounds and items for the new data.
    Group(FieldGroup),
    /// Excel drops the group: the data holds something the group cannot place.
    Ungroup,
}

const LEAP_DAYS: [usize; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn text(s: String) -> Value {
    Value::Text(s)
}

/// The group a refresh gives a date or number group: automatic bounds follow the data (first
/// to last value; the end date is the day after the last), the items are rebuilt for the
/// bounds. `None` when the group is of a kind this does not rebuild (hours, minutes, seconds,
/// or items it cannot reproduce): it stays as stored.
fn regroup_one(g: &FieldGroup, src: &[Value]) -> Option<Regrouped> {
    let mut nums: Vec<f64> = Vec::new();
    let mut blanks = false;
    for v in src {
        match v {
            Value::Number(n) => nums.push(*n),
            Value::Blank => blanks = true,
            _ => return Some(Regrouped::Ungroup),
        }
    }
    let (min, max) = nums
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
            (lo.min(*x), hi.max(*x))
        });
    if nums.is_empty() {
        return None;
    }
    let mut out = g.clone();
    match g.group_by.as_deref() {
        None => {
            // A number group made without blanks cannot take one.
            let had_blank = g
                .items
                .first()
                .is_some_and(|v| matches!(v, Value::Text(t) if t.contains("(blank)")));
            if blanks && !had_blank {
                return Some(Regrouped::Ungroup);
            }
            let step = g.interval.filter(|i| *i > 0.0)?;
            let start = if g.auto_start { min } else { g.start_num? };
            let end = if g.auto_end { max } else { g.end_num? };
            out.start_num = Some(start);
            out.end_num = Some(end);
            // Whole numbers throughout: `lo-hi` with hi one below the next group (the last group
            // ends at the end bound when it falls on a step). Any fraction in the data, the step
            // or the bounds: the groups touch, `lo-(lo+step)`.
            let whole = |x: f64| x.fract() == 0.0;
            let contiguous =
                !(nums.iter().all(|n| whole(*n)) && [step, start, end].iter().all(|n| whole(*n)));
            let first = match (blanks, g.auto_start) {
                (true, true) => "(blank)".to_owned(),
                (true, false) => format!("<{} or (blank)", fmt_num(start)),
                (false, _) => format!("<{}", fmt_num(start)),
            };
            let mut items = vec![text(first)];
            let mut k = 0.0;
            let mut lower = start;
            loop {
                let next = start + (k + 1.0) * step;
                let upper = if contiguous || next == end {
                    next
                } else {
                    next - 1.0
                };
                items.push(text(format!("{}-{}", fmt_num(lower), fmt_num(upper))));
                lower = next;
                k += 1.0;
                if lower >= end {
                    break;
                }
            }
            items.push(text(format!(">{}", fmt_num(lower))));
            out.items = items;
        }
        Some(by) => {
            if blanks {
                return Some(Regrouped::Ungroup);
            }
            // The first and the last value as they are; a last value on midnight (a date without
            // a time) ends the range a day later.
            let start = if g.auto_start { min } else { g.start_date? };
            let end = if g.auto_end {
                if max.fract() == 0.0 { max + 1.0 } else { max }
            } else {
                g.end_date?
            };
            out.start_date = Some(start);
            out.end_date = Some(end);
            let body: Vec<Value> = match by {
                "months" => MONTHS.iter().map(|m| text((*m).to_owned())).collect(),
                "quarters" => (1..=4).map(|q| text(format!("Qtr{q}"))).collect(),
                "years" => {
                    let (sy, _, _) = serial_to_ymd(start, false);
                    let (ey, _, _) = serial_to_ymd(end, false);
                    (sy..=ey).map(|y| text(y.to_string())).collect()
                }
                "hours" => (0..24).map(|h| text(format!("{h:02}"))).collect(),
                "minutes" | "seconds" => (0..60).map(|m| text(format!(":{m:02}"))).collect(),
                "days" if g.interval.is_none_or(|i| i <= 1.0) => LEAP_DAYS
                    .iter()
                    .enumerate()
                    .flat_map(|(m, n)| (1..=*n).map(move |d| text(format!("{d:02}-{}", MONTHS[m]))))
                    .collect(),
                "days" => {
                    let n = g.interval.filter(|i| *i >= 1.0)?;
                    let mut v = Vec::new();
                    let mut lower = start.floor();
                    while lower <= end {
                        let upper = (lower + n - 1.0).min(end);
                        v.push(text(format!("{} - {}", fmt_date(lower), fmt_date(upper))));
                        lower += n;
                    }
                    v
                }
                _ => return None,
            };
            let mut items = vec![text(format!("<{}", fmt_date(start)))];
            items.extend(body);
            items.push(text(format!(">{}", fmt_date(end))));
            out.items = items;
        }
    }
    Some(Regrouped::Group(out))
}

/// The refreshed group of every grouped cache field, by field index.
pub(crate) fn regroup(
    cache: &crate::model::CacheDefinition,
    columns: &[Vec<Value>],
) -> Vec<(usize, Regrouped)> {
    let mut out = Vec::new();
    for (fi, cf) in cache.fields.iter().enumerate() {
        let Some(g) = cf.group.as_ref() else {
            continue;
        };
        if g.items.is_empty() {
            continue;
        }
        let base = if cf.database_field {
            fi
        } else {
            g.base.map_or(fi, |b| b as usize)
        };
        let Some(src) = columns.get(base) else {
            continue;
        };
        if let Some(r) = regroup_one(g, src) {
            out.push((fi, r));
        }
    }
    out
}

/// The order of a grouped field's items after a refresh: the stored order of the items still
/// there (same label), then the new ones in index order. Months stay in calendar order.
/// `old_x` is the stored item list (indices into `old_items`); the result lists indices into
/// `new_items`.
pub(crate) fn item_order(
    old_items: &[Value],
    old_x: &[usize],
    new_items: &[Value],
    months: bool,
) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::new();
    if !months {
        for x in old_x {
            if let Some(v) = old_items.get(*x)
                && let Some(n) = new_items
                    .iter()
                    .position(|w| crate::value::ItemKey::of(w) == crate::value::ItemKey::of(v))
                && !order.contains(&n)
            {
                order.push(n);
            }
        }
    }
    for n in 0..new_items.len() {
        if !order.contains(&n) {
            order.push(n);
        }
    }
    order
}
