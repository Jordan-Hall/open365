//! A date group Excel drops on refresh.
//!
//! When a source date is blank (or text), Excel cannot keep a date group: the derived group
//! fields (`databaseField="0"`) leave the cache and the pivot fields, the axis, page and format
//! entries that named them go, the later field indices move up, and the base field is a plain
//! field again (no `fieldGroup`, records inline unless it sits on an axis). This module rewrites
//! the pivot and the cache that way before the refresh runs, so the regeneration in `regen.rs`
//! sees an ordinary pivot. Measured on `pivot-refresh-grouped-blank-date` (months; years and
//! months): table, cache and records parts equal Excel's.

use std::collections::BTreeSet;

use open365_xlsx::{RawXml, XmlElement};

use crate::SheetPivot;
use crate::compute::Dataset;
use crate::grouping::{Regrouped, regroup};
use crate::model::{CacheDefinition, PivotTable};

fn is_el(n: &RawXml, local: &str) -> bool {
    n.as_element().is_some_and(|e| e.head.is_main(local))
}

fn child_mut<'a>(e: &'a mut XmlElement, local: &str) -> Option<&'a mut XmlElement> {
    e.children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
        .find(|c| c.head.is_main(local))
}

fn remove_child(e: &mut XmlElement, local: &str) {
    e.children.retain(|c| !is_el(c, local));
}

/// New index of an old field index (`None` when the field is gone).
fn moved(map: &[Option<usize>], old: i64) -> Option<i64> {
    if old < 0 {
        return Some(old);
    }
    map.get(old as usize).copied().flatten().map(|n| n as i64)
}

/// Keeps the entries `list/entry@attr` whose field survives and renumbers them; the list goes
/// when it is empty.
fn renumber_list(root: &mut XmlElement, list: &str, attr: &str, map: &[Option<usize>]) {
    let Some(el) = child_mut(root, list) else {
        return;
    };
    el.children.retain_mut(|n| {
        let Some(e) = n.as_element_mut() else {
            return true;
        };
        let Some(old) = e.head.attr(attr).and_then(|v| v.trim().parse::<i64>().ok()) else {
            return true;
        };
        match moved(map, old) {
            Some(new) => {
                e.head.set_attr(attr, &new.to_string());
                true
            }
            None => false,
        }
    });
    let left = el.elements().count();
    if left == 0 {
        remove_child(root, list);
    } else {
        el.head.set_attr("count", &left.to_string());
    }
}

/// Formats whose areas name a dropped field go; the others are renumbered.
fn renumber_formats(root: &mut XmlElement, map: &[Option<usize>]) {
    let Some(formats) = child_mut(root, "formats") else {
        return;
    };
    formats.children.retain_mut(|n| {
        let Some(format) = n.as_element_mut() else {
            return true;
        };
        let Some(area) = child_mut(format, "pivotArea") else {
            return true;
        };
        let Some(refs) = child_mut(area, "references") else {
            return true;
        };
        for r in refs
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .filter(|e| e.head.is_main("reference"))
        {
            let Some(old) = r
                .head
                .attr("field")
                .and_then(|v| v.trim().parse::<i64>().ok())
            else {
                continue;
            };
            match moved(map, old) {
                Some(new) => r.head.set_attr("field", &new.to_string()),
                None => return false,
            }
        }
        true
    });
    let left = formats.elements().count();
    if left == 0 {
        remove_child(root, "formats");
    } else if formats.head.attr("count").is_some() {
        formats.head.set_attr("count", &left.to_string());
    }
}

impl SheetPivot {
    /// The pivot and data as they are once the date groups this refresh drops are gone, or
    /// `None` when no date group is dropped.
    pub(crate) fn without_dropped_date_groups(
        &self,
        data: &Dataset,
        date1904: bool,
    ) -> Option<(SheetPivot, Dataset)> {
        let fields = &self.cache.fields;
        let dropped: BTreeSet<usize> = regroup(&self.cache, &data.columns)
            .into_iter()
            .filter(|(fi, r)| *r == Regrouped::Ungroup && !fields[*fi].database_field)
            .map(|(fi, _)| fi)
            .collect();
        if dropped.is_empty() || data.columns.len() != fields.len() {
            return None;
        }
        // Base fields that keep no derived field are plain fields again.
        let bases_of = |keep: bool| -> BTreeSet<usize> {
            fields
                .iter()
                .enumerate()
                .filter(|(fi, f)| !f.database_field && dropped.contains(fi) != keep)
                .filter_map(|(_, f)| f.group.as_ref().and_then(|g| g.base))
                .map(|b| b as usize)
                .collect()
        };
        let still_derived = bases_of(true);
        let plain: BTreeSet<usize> = bases_of(false)
            .into_iter()
            .filter(|b| !still_derived.contains(b))
            .collect();
        let mut next = 0;
        let map: Vec<Option<usize>> = (0..fields.len())
            .map(|fi| {
                if dropped.contains(&fi) {
                    None
                } else {
                    next += 1;
                    Some(next - 1)
                }
            })
            .collect();

        // Cache definition.
        let mut cache_xml = self.cache.xml().clone();
        let cache_fields = child_mut(&mut cache_xml.root, "cacheFields")?;
        let mut idx = 0;
        cache_fields.children.retain_mut(|n| {
            let Some(cf) = n.as_element_mut().filter(|e| e.head.is_main("cacheField")) else {
                return true;
            };
            let i = idx;
            idx += 1;
            if dropped.contains(&i) {
                return false;
            }
            if plain.contains(&i) {
                remove_child(cf, "fieldGroup");
                if let Some(si) = child_mut(cf, "sharedItems") {
                    si.children.clear();
                    si.head.remove_attr("count");
                }
            }
            true
        });
        let kept = cache_fields.elements().count();
        cache_fields.head.set_attr("count", &kept.to_string());

        // Pivot table.
        let mut table_xml = self.table.xml().clone();
        let root = &mut table_xml.root;
        let pivot_fields = child_mut(root, "pivotFields")?;
        let mut idx = 0;
        pivot_fields.children.retain_mut(|n| {
            let Some(pf) = n.as_element_mut().filter(|e| e.head.is_main("pivotField")) else {
                return true;
            };
            let i = idx;
            idx += 1;
            if dropped.contains(&i) {
                return false;
            }
            if plain.contains(&i) && pf.head.attr("axis").is_none() {
                remove_child(pf, "items");
            }
            true
        });
        let kept = pivot_fields.elements().count();
        pivot_fields.head.set_attr("count", &kept.to_string());
        renumber_list(root, "rowFields", "x", &map);
        renumber_list(root, "colFields", "x", &map);
        renumber_list(root, "pageFields", "fld", &map);
        renumber_list(root, "dataFields", "fld", &map);
        renumber_formats(root, &map);

        let table = PivotTable::parse(&table_xml.to_bytes()).ok()?;
        let cache = CacheDefinition::parse(&cache_xml.to_bytes(), date1904).ok()?;
        let columns = data
            .columns
            .iter()
            .enumerate()
            .filter(|(fi, _)| !dropped.contains(fi))
            .map(|(_, c)| c.clone())
            .collect();
        Some((
            SheetPivot {
                table,
                cache,
                ..self.clone()
            },
            Dataset {
                columns,
                rows: data.rows,
            },
        ))
    }
}
