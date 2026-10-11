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
                (1 + ((x - start) / step).floor() as usize).min(inner_last)
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
                "days" => {
                    let n = g.interval.filter(|i| *i >= 1.0).unwrap_or(1.0);
                    1 + ((x.floor() - start.floor()) / n).floor() as usize
                }
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
