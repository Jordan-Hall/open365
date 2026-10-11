//! Calculated items: a formula over the items of one field (`Product[Apple]+Product[Banana]`).
//!
//! Measured rules (see `docs/architecture/pivot-tables.md`):
//!
//! * A cell on a calculated item line is the formula evaluated over the *aggregates of the named
//!   items*, each taken under the cell's other constraints (the data field's function is applied
//!   per named item, then the formula combines the results).
//! * A line that does not constrain a field with calculated items (a grand total, or the subtotal
//!   of an outer field) includes them: the value is the aggregate over the records combined with
//!   the calculated items' values (sum and count add, max and min compare).
//! * A calculated field on the same table is evaluated over the item-level sums of the fields it
//!   names, so the item formulas apply to each of those sums.
//! * Excel refuses Average, StdDev and Var data fields while a table has calculated items.

use std::collections::HashMap;

use crate::calc::{Expr, Val, eval};
use crate::compute::GridValue;
use crate::model::Func;

/// Calculated items of a table, by (pivot field, item position in the display order).
pub(crate) struct CalcItems<'a> {
    pub rpos: &'a [Vec<usize>],
    /// Lower-case cache field names.
    pub names: Vec<String>,
    /// Lower-case item text per field, in display order.
    pub items: Vec<Vec<String>>,
    pub formulas: HashMap<(usize, usize), Expr>,
    /// Fields with calculated items that are on an axis, in field order.
    pub fields: Vec<usize>,
}

/// How the values of items combine into a total over several items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Comb {
    Add,
    Max,
    Min,
    Mul,
}

impl Comb {
    pub(crate) fn of(f: Func) -> Self {
        match f {
            Func::Max => Self::Max,
            Func::Min => Self::Min,
            Func::Product => Self::Mul,
            _ => Self::Add,
        }
    }
}

/// An item formula needs the value of a leaf for each constraint set.
pub(crate) type Leaf<'f> = &'f dyn Fn(&[u32]) -> Option<GridValue>;

fn static_err(e: &str) -> &'static str {
    match e {
        "#DIV/0!" => "#DIV/0!",
        "#N/A" => "#N/A",
        "#NUM!" => "#NUM!",
        "#REF!" => "#REF!",
        "#NAME?" => "#NAME?",
        "#NULL!" => "#NULL!",
        _ => "#VALUE!",
    }
}

fn combine(comb: Comb, a: Option<GridValue>, b: Option<GridValue>) -> Option<GridValue> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(GridValue::Error(e)), _) | (_, Some(GridValue::Error(e))) => {
            Some(GridValue::Error(e))
        }
        (Some(GridValue::Number(x)), Some(GridValue::Number(y))) => {
            Some(GridValue::Number(match comb {
                Comb::Add => x + y,
                Comb::Max => x.max(y),
                Comb::Min => x.min(y),
                Comb::Mul => x * y,
            }))
        }
        (x, _) => x,
    }
}

impl CalcItems<'_> {
    /// Fields with calculated items that `calcs` does not already constrain.
    pub(crate) fn free(&self, calcs: &[(usize, usize)], cons: &[&[(usize, usize)]]) -> Vec<usize> {
        self.fields
            .iter()
            .copied()
            .filter(|f| {
                !calcs.iter().any(|(cf, _)| cf == f)
                    && !cons.iter().any(|c| c.iter().any(|(cf, _)| cf == f))
            })
            .collect()
    }

    /// The calculated items among the constraints of a line.
    pub(crate) fn calcs_of(&self, cons: &[(usize, usize)]) -> Vec<(usize, usize)> {
        cons.iter()
            .copied()
            .filter(|k| self.formulas.contains_key(k))
            .collect()
    }

    /// Value of a cell over `recs` with the calculated items `calcs` substituted and the
    /// calculated items of the `free` fields added to the total.
    ///
    /// `split` lists the axis fields the cell does not constrain: a calculated item's formula is
    /// evaluated per item of those fields and the results are combined (measured: the grand total
    /// column of a calculated item line is the sum, or the maximum, of the line's cells, not the
    /// formula over the totals).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn value(
        &self,
        recs: &[u32],
        calcs: &[(usize, usize)],
        free: &[usize],
        split: &[usize],
        comb: Comb,
        leaf: Leaf,
        depth: usize,
    ) -> Option<GridValue> {
        if recs.is_empty() || depth > 8 {
            return None;
        }
        if let Some((&(f, p), rest)) = calcs.split_first() {
            if let Some((&sf, more)) = split.split_first() {
                let mut acc = None;
                for q in 0..self.items[sf].len() {
                    let sub: Vec<u32> = recs
                        .iter()
                        .copied()
                        .filter(|r| self.rpos[sf].get(*r as usize) == Some(&q))
                        .collect();
                    let v = self.value(&sub, calcs, free, more, comb, leaf, depth);
                    acc = combine(comb, acc, v);
                }
                return acc;
            }
            let expr = self.formulas.get(&(f, p))?;
            let lookup = |name: &str| -> Val {
                let Some((fname, rest_name)) = name.split_once('[') else {
                    return Val::Err("#NAME?");
                };
                let iname = rest_name.trim_end_matches(']').to_lowercase();
                let Some(fi) = self.names.iter().position(|n| *n == fname.to_lowercase()) else {
                    return Val::Err("#REF!");
                };
                let Some(ip) = self.items[fi].iter().position(|t| *t == iname) else {
                    return Val::Err("#REF!");
                };
                let v = if self.formulas.contains_key(&(fi, ip)) {
                    let mut next = vec![(fi, ip)];
                    next.extend_from_slice(rest);
                    self.value(recs, &next, free, &[], comb, leaf, depth + 1)
                } else {
                    let sub: Vec<u32> = recs
                        .iter()
                        .copied()
                        .filter(|r| self.rpos[fi].get(*r as usize) == Some(&ip))
                        .collect();
                    self.value(&sub, rest, free, &[], comb, leaf, depth + 1)
                };
                match v {
                    None => Val::Num(0.0),
                    Some(GridValue::Number(n)) => Val::Num(n),
                    Some(GridValue::Text(t)) => Val::Text(t),
                    Some(GridValue::Error(e)) => Val::Err(static_err(&e)),
                }
            };
            return Some(match eval(expr, &lookup) {
                Val::Num(n) => GridValue::Number(n),
                Val::Text(t) => GridValue::Text(t),
                Val::Bool(b) => GridValue::Number(f64::from(u8::from(b))),
                Val::Err(e) => GridValue::Error(e.to_owned()),
            });
        }
        let Some((&f, more)) = free.split_first() else {
            return leaf(recs);
        };
        let mut total = self.value(recs, &[], more, split, comb, leaf, depth);
        let mut mine: Vec<usize> = self
            .formulas
            .keys()
            .filter(|(cf, _)| *cf == f)
            .map(|(_, p)| *p)
            .collect();
        mine.sort_unstable();
        for p in mine {
            let v = self.value(recs, &[(f, p)], more, split, comb, leaf, depth);
            total = combine(comb, total, v);
        }
        total
    }
}
