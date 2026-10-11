//! Show Values As: the calculations Excel applies on top of the aggregated data cells.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::compute::{GridValue, aggregate};
use crate::model::{CacheDefinition, DataField, PivotTable};
use crate::value::{ItemKey, Value};

/// Item constraints of an axis line: (pivot field, item position) per level, outermost first.
pub(crate) type Cons = Vec<(usize, usize)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Normal,
    Diff,
    Pct,
    PctDiff,
    Running,
    PctRow,
    PctCol,
    PctTotal,
    Index,
    ParentRow,
    ParentCol,
    Parent,
    PctRunning,
    RankAsc,
    RankDesc,
}

impl Mode {
    pub(crate) fn of(df: &DataField) -> Mode {
        let name = df.pivot_show_as.as_deref().unwrap_or(&df.show_data_as);
        match name {
            "difference" => Mode::Diff,
            "percent" | "percentOf" => Mode::Pct,
            "percentDiff" | "percentDifferenceFrom" => Mode::PctDiff,
            "runTotal" | "runningTotal" => Mode::Running,
            "percentOfRow" => Mode::PctRow,
            "percentOfCol" => Mode::PctCol,
            "percentOfTotal" => Mode::PctTotal,
            "index" => Mode::Index,
            "percentOfParentRow" => Mode::ParentRow,
            "percentOfParentCol" => Mode::ParentCol,
            "percentOfParent" => Mode::Parent,
            "percentOfRunningTotal" => Mode::PctRunning,
            "rankAscending" => Mode::RankAsc,
            "rankDescending" => Mode::RankDesc,
            _ => Mode::Normal,
        }
    }
}

const PREVIOUS: u32 = 1_048_828;
const NEXT: u32 = 1_048_829;

pub(crate) struct Calc<'a> {
    pub pt: &'a PivotTable,
    pub cache: &'a CacheDefinition,
    pub columns: &'a [Vec<Value>],
    pub rpos: &'a [Vec<usize>],
    pub order: &'a [Vec<Value>],
    pub alive: &'a [u32],
    memo: RefCell<HashMap<(Cons, Cons, usize), Option<f64>>>,
    /// Display order of the items of a field below a given parent path.
    sib: HashMap<(usize, Cons), Vec<usize>>,
    /// Constraint lists of the rows and columns that exist as lines of the output.
    exists: [std::collections::HashSet<Cons>; 2],
}

fn div(num: f64, den: Option<f64>) -> Option<GridValue> {
    let den = den?;
    if den == 0.0 {
        return Some(GridValue::Error("#DIV/0!".into()));
    }
    Some(GridValue::Number(num / den))
}

/// Like [`div`], but an empty denominator is `#DIV/0!` too (percent of total, row, column).
fn div_err(num: f64, den: Option<f64>) -> Option<GridValue> {
    div(num, Some(den.unwrap_or(0.0)))
}

impl<'a> Calc<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        pt: &'a PivotTable,
        cache: &'a CacheDefinition,
        columns: &'a [Vec<Value>],
        rpos: &'a [Vec<usize>],
        order: &'a [Vec<Value>],
        alive: &'a [u32],
        row_lines: &[&Cons],
        col_lines: &[&Cons],
    ) -> Self {
        let mut sib: HashMap<(usize, Cons), Vec<usize>> = HashMap::new();
        for cons in row_lines.iter().chain(col_lines.iter()) {
            for i in 0..cons.len() {
                let list = sib.entry((cons[i].0, cons[..i].to_vec())).or_default();
                if !list.contains(&cons[i].1) {
                    list.push(cons[i].1);
                }
            }
        }
        Self {
            pt,
            cache,
            columns,
            rpos,
            order,
            alive,
            memo: RefCell::new(HashMap::new()),
            sib,
            exists: [
                row_lines.iter().map(|c| (*c).clone()).collect(),
                col_lines.iter().map(|c| (*c).clone()).collect(),
            ],
        }
    }

    /// Aggregate of data field `d` over the records matching both constraint lists.
    fn val(&self, rc: &Cons, cc: &Cons, d: usize) -> Option<f64> {
        let key = (rc.clone(), cc.clone(), d);
        if let Some(v) = self.memo.borrow().get(&key) {
            return *v;
        }
        let df = &self.pt.data_fields[d];
        let col = self
            .columns
            .get(df.fld as usize)
            .map_or(&[][..], Vec::as_slice);
        let vals: Vec<&Value> = self
            .alive
            .iter()
            .filter(|r| {
                rc.iter()
                    .chain(cc.iter())
                    .all(|(f, p)| self.rpos[*f][**r as usize] == *p)
            })
            .filter_map(|r| col.get(*r as usize))
            .collect();
        let out = match aggregate(df.func, &vals) {
            Some(GridValue::Number(n)) => Some(n),
            _ => None,
        };
        self.memo.borrow_mut().insert(key, out);
        out
    }

    /// Position of the named base item in the field's display item list.
    fn named_pos(&self, field: usize, item: u32) -> Option<usize> {
        let pf = self.pt.fields.get(field)?;
        let x = pf.items.get(item as usize)?.x?;
        let v = self
            .cache
            .fields
            .get(field)?
            .item_values()
            .get(x as usize)?;
        let key = ItemKey::of(v);
        self.order[field].iter().position(|o| ItemKey::of(o) == key)
    }

    /// Applies the data field's Show Values As to the cell at (`rc`, `cc`).
    pub(crate) fn apply(
        &self,
        d: usize,
        rc: &Cons,
        cc: &Cons,
        x: Option<GridValue>,
    ) -> Option<GridValue> {
        let df = &self.pt.data_fields[d];
        let mode = Mode::of(df);
        if mode == Mode::Normal {
            return x;
        }
        let x0 = match &x {
            Some(GridValue::Number(n)) => *n,
            Some(other) => return Some(other.clone()),
            None => 0.0,
        };
        let empty: Cons = Vec::new();
        let num = |v: Option<f64>| v.map(GridValue::Number);
        match mode {
            Mode::Normal => x,
            Mode::PctTotal => div_err(x0, self.val(&empty, &empty, d)),
            Mode::PctRow => div_err(x0, self.val(rc, &empty, d)),
            Mode::PctCol => div_err(x0, self.val(&empty, cc, d)),
            Mode::Index => {
                let (t, r, c) = (
                    self.val(&empty, &empty, d).unwrap_or(0.0),
                    self.val(rc, &empty, d).unwrap_or(0.0),
                    self.val(&empty, cc, d).unwrap_or(0.0),
                );
                div(x0 * t, Some(r * c))
            }
            Mode::ParentRow => {
                let parent: Cons = rc[..rc.len().saturating_sub(1)].to_vec();
                if rc.is_empty() {
                    return div(x0, self.val(rc, cc, d));
                }
                div(x0, self.val(&parent, cc, d))
            }
            Mode::ParentCol => {
                let parent: Cons = cc[..cc.len().saturating_sub(1)].to_vec();
                if cc.is_empty() {
                    return div(x0, self.val(rc, cc, d));
                }
                div(x0, self.val(rc, &parent, d))
            }
            _ => {
                let base = usize::try_from(df.base_field).ok()?;
                let (on_rows, k) = if let Some(k) = rc.iter().position(|(f, _)| *f == base) {
                    (true, k)
                } else {
                    (false, cc.iter().position(|(f, _)| *f == base)?)
                };
                let cons = if on_rows { rc } else { cc };
                let cur = cons[k].1;
                let cell = |p: usize| -> Option<f64> {
                    let mut c = cons.clone();
                    c[k].1 = p;
                    if on_rows {
                        self.val(&c, cc, d)
                    } else {
                        self.val(rc, &c, d)
                    }
                };
                if mode == Mode::Parent {
                    let c: Cons = cons[..=k].to_vec();
                    return div(
                        x0,
                        if on_rows {
                            self.val(&c, cc, d)
                        } else {
                            self.val(rc, &c, d)
                        },
                    );
                }
                let items = self.sib.get(&(base, cons[..k].to_vec()))?;
                let at = items.iter().position(|p| *p == cur)?;
                match mode {
                    Mode::Running | Mode::PctRunning => {
                        let run: f64 = items[..=at].iter().filter_map(|p| cell(*p)).sum();
                        if mode == Mode::Running {
                            Some(GridValue::Number(run))
                        } else {
                            let all: f64 = items.iter().filter_map(|p| cell(*p)).sum();
                            if all == 0.0 && x.is_none() {
                                None
                            } else {
                                div(run, Some(all))
                            }
                        }
                    }
                    Mode::RankAsc | Mode::RankDesc => {
                        let mine = num(self.cell_or_none(&x))?;
                        let GridValue::Number(mine) = mine else {
                            return None;
                        };
                        let better = items
                            .iter()
                            .filter_map(|p| cell(*p))
                            .filter(|v| {
                                if mode == Mode::RankAsc {
                                    *v < mine
                                } else {
                                    *v > mine
                                }
                            })
                            .count();
                        Some(GridValue::Number((better + 1) as f64))
                    }
                    _ => {
                        let present = |p: &usize| {
                            let mut c = cons.clone();
                            c[k].1 = *p;
                            self.exists[usize::from(!on_rows)].contains(&c)
                        };
                        let bp = match df.base_item {
                            PREVIOUS => *items[..at].iter().rev().find(|p| present(p))?,
                            NEXT => *items[at + 1..].iter().find(|p| present(p))?,
                            n => self.named_pos(base, n)?,
                        };
                        if bp == cur && (mode != Mode::Pct || x.is_none()) {
                            return None;
                        }
                        let b = cell(bp);
                        let x_empty = x.is_none();
                        match mode {
                            Mode::Diff => Some(GridValue::Number(x0 - b.unwrap_or(0.0))),
                            // An empty cell is `#NULL!`; a value over an empty base stays blank.
                            Mode::Pct | Mode::PctDiff if x_empty => {
                                Some(GridValue::Error("#NULL!".into()))
                            }
                            Mode::Pct => div(x0, b),
                            Mode::PctDiff => div(x0 - b.unwrap_or(0.0), b),
                            _ => None,
                        }
                    }
                }
            }
        }
    }

    fn cell_or_none(&self, x: &Option<GridValue>) -> Option<f64> {
        match x {
            Some(GridValue::Number(n)) => Some(*n),
            _ => None,
        }
    }
}
