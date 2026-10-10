//! Dependency graph for incremental recalculation.
//!
//! Formula cells register what they read: single cells and rectangular areas. Areas are
//! deduplicated and indexed by row buckets (very tall areas such as `A:A` live in a short
//! side list), so a whole-column reference is one index entry, never a million. Given a changed
//! cell the graph finds its transitive dependents and orders them topologically; cells that
//! are part of a reference cycle are reported separately.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ast::{AreaKind, Ast, MAX_COL, MAX_ROW};
use crate::functions::is_volatile;

/// Rows per index bucket.
const BUCKET_ROWS: u32 = 256;
/// Areas taller than this many rows go to the linear "tall" list instead of buckets.
const TALL_ROWS: u32 = 4096;
/// Maximum depth when following defined names that refer to other names.
const MAX_NAME_DEPTH: u32 = 16;

/// A cell address with an interned sheet index (rows and columns are 1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellId {
    /// Sheet index.
    pub sheet: u32,
    /// Row.
    pub row: u32,
    /// Column.
    pub col: u32,
}

/// A normalised rectangular area on one sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AreaKey {
    /// Sheet index.
    pub sheet: u32,
    /// First row.
    pub row1: u32,
    /// First column.
    pub col1: u32,
    /// Last row.
    pub row2: u32,
    /// Last column.
    pub col2: u32,
}

impl AreaKey {
    fn contains(&self, cell: CellId) -> bool {
        self.sheet == cell.sheet
            && (self.row1..=self.row2).contains(&cell.row)
            && (self.col1..=self.col2).contains(&cell.col)
    }
}

/// One thing a formula reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dep {
    /// A single cell.
    Cell(CellId),
    /// A multi-cell area.
    Area(AreaKey),
}

impl Dep {
    /// Builds a dependency from any two corners; a 1x1 area becomes a cell dependency.
    pub fn from_corners(sheet: u32, row_a: u32, col_a: u32, row_b: u32, col_b: u32) -> Dep {
        let (row1, row2) = (row_a.min(row_b), row_a.max(row_b));
        let (col1, col2) = (col_a.min(col_b), col_a.max(col_b));
        if row1 == row2 && col1 == col2 {
            Dep::Cell(CellId {
                sheet,
                row: row1,
                col: col1,
            })
        } else {
            Dep::Area(AreaKey {
                sheet,
                row1,
                col1,
                row2,
                col2,
            })
        }
    }
}

/// Result of [`Graph::plan`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecalcPlan {
    /// Cells in evaluation order: every cell appears after the cells it reads.
    pub order: Vec<CellId>,
    /// Cells that belong to a reference cycle (including self references). They are also
    /// present in `order`; cells merely downstream of a cycle are not listed here.
    pub cyclic: HashSet<CellId>,
}

/// The dependency graph. See the module documentation.
#[derive(Debug, Default)]
pub struct Graph {
    deps: HashMap<CellId, Vec<Dep>>,
    cell_dependents: HashMap<CellId, HashSet<CellId>>,
    areas: HashMap<AreaKey, HashSet<CellId>>,
    buckets: HashMap<(u32, u32), Vec<AreaKey>>,
    tall: HashSet<AreaKey>,
    volatile: HashSet<CellId>,
}

impl Graph {
    /// An empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers (or replaces) the dependencies of a formula cell.
    pub fn set_formula(
        &mut self,
        cell: CellId,
        deps: impl IntoIterator<Item = Dep>,
        volatile: bool,
    ) {
        self.remove(cell);
        let unique: HashSet<Dep> = deps.into_iter().collect();
        for dep in &unique {
            match *dep {
                Dep::Cell(c) => {
                    self.cell_dependents.entry(c).or_default().insert(cell);
                }
                Dep::Area(key) => {
                    let first = {
                        let entry = self.areas.entry(key).or_default();
                        let was_empty = entry.is_empty();
                        entry.insert(cell);
                        was_empty
                    };
                    if first {
                        self.index_area(key);
                    }
                }
            }
        }
        self.deps.insert(cell, unique.into_iter().collect());
        if volatile {
            self.volatile.insert(cell);
        }
    }

    /// Forgets a formula cell (it became a constant or was cleared).
    pub fn remove(&mut self, cell: CellId) {
        self.volatile.remove(&cell);
        let Some(deps) = self.deps.remove(&cell) else {
            return;
        };
        for dep in deps {
            match dep {
                Dep::Cell(c) => {
                    if let Some(set) = self.cell_dependents.get_mut(&c) {
                        set.remove(&cell);
                        if set.is_empty() {
                            self.cell_dependents.remove(&c);
                        }
                    }
                }
                Dep::Area(key) => {
                    let now_empty = self.areas.get_mut(&key).is_some_and(|set| {
                        set.remove(&cell);
                        set.is_empty()
                    });
                    if now_empty {
                        self.areas.remove(&key);
                        self.unindex_area(key);
                    }
                }
            }
        }
    }

    /// Whether the cell has a registered formula.
    pub fn contains(&self, cell: CellId) -> bool {
        self.deps.contains_key(&cell)
    }

    /// Number of formula cells.
    pub fn formula_count(&self) -> usize {
        self.deps.len()
    }

    /// Number of distinct areas currently tracked.
    pub fn area_count(&self) -> usize {
        self.areas.len()
    }

    /// Total entries in the spatial index; independent of how many cells the areas cover.
    pub fn index_entry_count(&self) -> usize {
        self.buckets.values().map(Vec::len).sum::<usize>() + self.tall.len()
    }

    /// Formula cells that must be recalculated on every pass.
    pub fn volatile_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.volatile.iter().copied()
    }

    /// All formula cells.
    pub fn formula_cells(&self) -> impl Iterator<Item = CellId> + '_ {
        self.deps.keys().copied()
    }

    fn index_area(&mut self, key: AreaKey) {
        if key.row2 - key.row1 + 1 > TALL_ROWS {
            self.tall.insert(key);
            return;
        }
        for bucket in key.row1 / BUCKET_ROWS..=key.row2 / BUCKET_ROWS {
            self.buckets
                .entry((key.sheet, bucket))
                .or_default()
                .push(key);
        }
    }

    fn unindex_area(&mut self, key: AreaKey) {
        if self.tall.remove(&key) {
            return;
        }
        for bucket in key.row1 / BUCKET_ROWS..=key.row2 / BUCKET_ROWS {
            let id = (key.sheet, bucket);
            if let Some(list) = self.buckets.get_mut(&id) {
                list.retain(|k| *k != key);
                if list.is_empty() {
                    self.buckets.remove(&id);
                }
            }
        }
    }

    /// Formula cells that read `cell` directly.
    pub fn dependents_of(&self, cell: CellId) -> Vec<CellId> {
        let mut out: HashSet<CellId> = HashSet::new();
        if let Some(set) = self.cell_dependents.get(&cell) {
            out.extend(set);
        }
        let candidates = self
            .buckets
            .get(&(cell.sheet, cell.row / BUCKET_ROWS))
            .into_iter()
            .flatten()
            .chain(self.tall.iter());
        for key in candidates {
            if key.contains(cell)
                && let Some(set) = self.areas.get(key)
            {
                out.extend(set);
            }
        }
        out.into_iter().collect()
    }

    /// Transitive dependents of the given cells. The roots themselves are included only if
    /// they depend on one another (or on themselves).
    pub fn dirty_from(&self, roots: &[CellId]) -> HashSet<CellId> {
        let mut seen: HashSet<CellId> = HashSet::new();
        let mut queue: VecDeque<CellId> = roots.iter().copied().collect();
        while let Some(cell) = queue.pop_front() {
            for dep in self.dependents_of(cell) {
                if seen.insert(dep) {
                    queue.push_back(dep);
                }
            }
        }
        seen
    }

    /// Orders `dirty` so each cell comes after the dirty cells it reads, and finds cycles.
    /// Uses an iterative Tarjan strongly-connected-components pass, so long dependency chains
    /// cannot overflow the stack.
    pub fn plan(&self, dirty: &HashSet<CellId>) -> RecalcPlan {
        let mut nodes: Vec<CellId> = dirty.iter().copied().collect();
        nodes.sort_unstable();
        let index_of: HashMap<CellId, usize> =
            nodes.iter().enumerate().map(|(i, c)| (*c, i)).collect();
        // Edge precedent -> dependent, restricted to dirty cells.
        let adjacency: Vec<Vec<usize>> = nodes
            .iter()
            .map(|c| {
                let mut next: Vec<usize> = self
                    .dependents_of(*c)
                    .iter()
                    .filter_map(|d| index_of.get(d).copied())
                    .collect();
                next.sort_unstable();
                next
            })
            .collect();

        let n = nodes.len();
        let unvisited = usize::MAX;
        let mut index = vec![unvisited; n];
        let mut low = vec![0usize; n];
        let mut on_stack = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut call: Vec<(usize, usize)> = Vec::new();
        let mut counter = 0usize;
        let mut components: Vec<Vec<usize>> = Vec::new();

        for start in 0..n {
            if index[start] != unvisited {
                continue;
            }
            index[start] = counter;
            low[start] = counter;
            counter += 1;
            stack.push(start);
            on_stack[start] = true;
            call.push((start, 0));
            while let Some(&(v, next_child)) = call.last() {
                if let Some(&w) = adjacency[v].get(next_child) {
                    if let Some(top) = call.last_mut() {
                        top.1 += 1;
                    }
                    if index[w] == unvisited {
                        index[w] = counter;
                        low[w] = counter;
                        counter += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        call.push((w, 0));
                    } else if on_stack[w] {
                        low[v] = low[v].min(index[w]);
                    }
                } else {
                    call.pop();
                    if let Some(&(parent, _)) = call.last() {
                        low[parent] = low[parent].min(low[v]);
                    }
                    if low[v] == index[v] {
                        let mut component = Vec::new();
                        while let Some(w) = stack.pop() {
                            on_stack[w] = false;
                            component.push(w);
                            if w == v {
                                break;
                            }
                        }
                        components.push(component);
                    }
                }
            }
        }

        let mut plan = RecalcPlan::default();
        // Tarjan emits components sinks-first; reversed they run precedents-first.
        for component in components.iter().rev() {
            let cyclic = component.len() > 1
                || component
                    .first()
                    .is_some_and(|&v| adjacency[v].contains(&v));
            for &v in component {
                plan.order.push(nodes[v]);
                if cyclic {
                    plan.cyclic.insert(nodes[v]);
                }
            }
        }
        plan
    }
}

/// A rectangle on a named sheet that a formula reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetRect {
    /// Sheet name as written (or the formula's own sheet).
    pub sheet: String,
    /// First row.
    pub row1: u32,
    /// First column.
    pub col1: u32,
    /// Last row.
    pub row2: u32,
    /// Last column.
    pub col2: u32,
}

/// What a formula reads, extracted from its syntax tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deps {
    /// Cells and areas read.
    pub refs: Vec<SheetRect>,
    /// Defined names used (as written).
    pub names: Vec<String>,
    /// The formula calls a volatile function such as `NOW`.
    pub volatile: bool,
}

/// Collects the references, defined names and volatility of a formula. `resolve` expands
/// defined names so the formula also depends on what they point to.
pub fn extract_deps(ast: &Ast, sheet: &str, resolve: &dyn Fn(&str) -> Option<Ast>) -> Deps {
    let mut deps = Deps::default();
    walk(ast, sheet, resolve, &mut deps, 0);
    deps
}

fn walk(ast: &Ast, sheet: &str, resolve: &dyn Fn(&str) -> Option<Ast>, out: &mut Deps, depth: u32) {
    match ast {
        Ast::Ref(r) => out.refs.push(SheetRect {
            sheet: r.sheet.clone().unwrap_or_else(|| sheet.to_owned()),
            row1: r.row,
            col1: r.col,
            row2: r.row,
            col2: r.col,
        }),
        Ast::Area(a) => {
            let (row1, row2, col1, col2) = match a.kind {
                AreaKind::Cells => (a.row1, a.row2, a.col1, a.col2),
                AreaKind::Cols => (1, MAX_ROW, a.col1, a.col2),
                AreaKind::Rows => (a.row1, a.row2, 1, MAX_COL),
            };
            out.refs.push(SheetRect {
                sheet: a.sheet.clone().unwrap_or_else(|| sheet.to_owned()),
                row1: row1.min(row2),
                col1: col1.min(col2),
                row2: row1.max(row2),
                col2: col1.max(col2),
            });
        }
        Ast::Name(name) => {
            if !out.names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                out.names.push(name.clone());
            }
            if depth < MAX_NAME_DEPTH
                && let Some(target) = resolve(name)
            {
                walk(&target, sheet, resolve, out, depth + 1);
            }
        }
        Ast::Func { name, args, .. } => {
            if is_volatile(name) {
                out.volatile = true;
            }
            for a in args {
                walk(a, sheet, resolve, out, depth);
            }
        }
        Ast::Unary { operand, .. } => walk(operand, sheet, resolve, out, depth),
        Ast::Percent(inner) => walk(inner, sheet, resolve, out, depth),
        Ast::Binary { lhs, rhs, .. } => {
            walk(lhs, sheet, resolve, out, depth);
            walk(rhs, sheet, resolve, out, depth);
        }
        Ast::Array(rows) => {
            for item in rows.iter().flatten() {
                walk(item, sheet, resolve, out, depth);
            }
        }
        Ast::Number(_) | Ast::Text(_) | Ast::Bool(_) | Ast::Error(_) | Ast::Missing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_formula;

    fn id(row: u32, col: u32) -> CellId {
        CellId { sheet: 0, row, col }
    }

    fn cell(row: u32, col: u32) -> Dep {
        Dep::Cell(id(row, col))
    }

    fn area(r1: u32, c1: u32, r2: u32, c2: u32) -> Dep {
        Dep::from_corners(0, r1, c1, r2, c2)
    }

    fn position(plan: &RecalcPlan, c: CellId) -> usize {
        plan.order.iter().position(|x| *x == c).unwrap()
    }

    #[test]
    fn transitive_dirty_set() {
        let mut g = Graph::new();
        // A1 = B1 + 1; B1 = C1 + 1; D1 = E1 (unrelated)
        g.set_formula(id(1, 1), [cell(1, 2)], false);
        g.set_formula(id(1, 2), [cell(1, 3)], false);
        g.set_formula(id(1, 4), [cell(1, 5)], false);
        let dirty = g.dirty_from(&[id(1, 3)]);
        assert_eq!(dirty, HashSet::from([id(1, 1), id(1, 2)]));
        assert!(g.dirty_from(&[id(9, 9)]).is_empty());
    }

    #[test]
    fn topological_order_diamond() {
        let mut g = Graph::new();
        // top = left + right; left = base; right = base
        let (top, left, right, base) = (id(1, 1), id(2, 1), id(3, 1), id(4, 1));
        g.set_formula(top, [Dep::Cell(left), Dep::Cell(right)], false);
        g.set_formula(left, [Dep::Cell(base)], false);
        g.set_formula(right, [Dep::Cell(base)], false);
        let dirty = g.dirty_from(&[base]);
        let plan = g.plan(&dirty);
        assert_eq!(plan.order.len(), 3);
        assert!(plan.cyclic.is_empty());
        assert!(position(&plan, left) < position(&plan, top));
        assert!(position(&plan, right) < position(&plan, top));
    }

    #[test]
    fn range_dependencies() {
        let mut g = Graph::new();
        g.set_formula(id(1, 2), [area(1, 1, 100, 1)], false); // B1 = SUM(A1:A100)
        assert_eq!(g.dependents_of(id(50, 1)), vec![id(1, 2)]);
        assert_eq!(g.dependents_of(id(100, 1)), vec![id(1, 2)]);
        assert!(g.dependents_of(id(101, 1)).is_empty());
        assert!(g.dependents_of(id(50, 2)).is_empty());
        let other_sheet = CellId {
            sheet: 1,
            row: 50,
            col: 1,
        };
        assert!(g.dependents_of(other_sheet).is_empty());
    }

    #[test]
    fn range_spanning_buckets() {
        let mut g = Graph::new();
        g.set_formula(id(1, 3), [area(10, 1, 3000, 2)], false);
        for row in [10, 255, 256, 257, 1000, 2999, 3000] {
            assert_eq!(g.dependents_of(id(row, 2)), vec![id(1, 3)], "row {row}");
        }
        assert!(g.dependents_of(id(9, 2)).is_empty());
        assert!(g.dependents_of(id(3001, 2)).is_empty());
    }

    #[test]
    fn whole_column_is_one_index_entry() {
        let mut g = Graph::new();
        // 2000 formulas all reading A:A (and one reading 1:1).
        for i in 0..2000 {
            g.set_formula(id(i + 1, 2), [area(1, 1, MAX_ROW, 1)], false);
        }
        g.set_formula(id(1, 5), [area(1, 1, 1, MAX_COL)], false);
        assert_eq!(g.area_count(), 2);
        assert!(
            g.index_entry_count() <= 3,
            "index entries: {}",
            g.index_entry_count()
        );
        assert_eq!(g.dependents_of(id(900_000, 1)).len(), 2000);
        assert_eq!(g.dependents_of(id(1, 9000)), vec![id(1, 5)]);
        assert!(g.dependents_of(id(900_000, 2)).is_empty());
    }

    #[test]
    fn whole_column_recalc_of_100k_cells_is_incremental() {
        let mut g = Graph::new();
        g.set_formula(id(1, 2), [area(1, 1, MAX_ROW, 1)], false); // B1 = SUM(A:A)
        // Changing any of 100k cells in A marks the single dependent.
        for row in (1..=100_000).step_by(997) {
            assert_eq!(g.dirty_from(&[id(row, 1)]), HashSet::from([id(1, 2)]));
        }
    }

    #[test]
    fn replacing_and_removing_formulas_cleans_the_index() {
        let mut g = Graph::new();
        g.set_formula(id(1, 1), [cell(2, 1), area(1, 5, 100, 5)], false);
        assert_eq!(g.dependents_of(id(2, 1)), vec![id(1, 1)]);
        g.set_formula(id(1, 1), [cell(3, 1)], false);
        assert!(g.dependents_of(id(2, 1)).is_empty());
        assert!(g.dependents_of(id(50, 5)).is_empty());
        assert_eq!(g.dependents_of(id(3, 1)), vec![id(1, 1)]);
        assert_eq!(g.area_count(), 0);
        assert_eq!(g.index_entry_count(), 0);
        g.remove(id(1, 1));
        assert!(g.dependents_of(id(3, 1)).is_empty());
        assert_eq!(g.formula_count(), 0);
        assert!(!g.contains(id(1, 1)));
    }

    #[test]
    fn two_cell_cycle_is_detected() {
        let mut g = Graph::new();
        g.set_formula(id(1, 1), [cell(2, 1)], false); // A1 = A2
        g.set_formula(id(2, 1), [cell(1, 1)], false); // A2 = A1
        g.set_formula(id(3, 1), [cell(1, 1)], false); // A3 = A1 (downstream)
        let dirty: HashSet<CellId> = [id(1, 1), id(2, 1), id(3, 1)].into_iter().collect();
        let plan = g.plan(&dirty);
        assert_eq!(plan.cyclic, HashSet::from([id(1, 1), id(2, 1)]));
        assert_eq!(plan.order.len(), 3);
        assert!(position(&plan, id(3, 1)) > position(&plan, id(1, 1)));
        assert!(position(&plan, id(3, 1)) > position(&plan, id(2, 1)));
    }

    #[test]
    fn self_reference_is_a_cycle() {
        let mut g = Graph::new();
        g.set_formula(id(1, 1), [cell(1, 1)], false);
        let plan = g.plan(&HashSet::from([id(1, 1)]));
        assert_eq!(plan.cyclic, HashSet::from([id(1, 1)]));
    }

    #[test]
    fn cycle_through_a_range() {
        let mut g = Graph::new();
        g.set_formula(id(5, 1), [area(1, 1, 10, 1)], false); // A5 = SUM(A1:A10) includes itself
        let plan = g.plan(&HashSet::from([id(5, 1)]));
        assert_eq!(plan.cyclic, HashSet::from([id(5, 1)]));
    }

    #[test]
    fn long_chain_does_not_overflow_the_stack() {
        let mut g = Graph::new();
        let n = 50_000;
        for i in 1..n {
            g.set_formula(id(i, 1), [cell(i + 1, 1)], false);
        }
        let dirty = g.dirty_from(&[id(n, 1)]);
        assert_eq!(dirty.len(), (n - 1) as usize);
        let plan = g.plan(&dirty);
        assert_eq!(plan.order.len(), dirty.len());
        assert!(plan.cyclic.is_empty());
        assert_eq!(plan.order.first(), Some(&id(n - 1, 1)));
        assert_eq!(plan.order.last(), Some(&id(1, 1)));
    }

    #[test]
    fn volatile_cells_are_tracked() {
        let mut g = Graph::new();
        g.set_formula(id(1, 1), [], true);
        g.set_formula(id(2, 1), [], false);
        assert_eq!(g.volatile_cells().collect::<Vec<_>>(), vec![id(1, 1)]);
        g.remove(id(1, 1));
        assert_eq!(g.volatile_cells().count(), 0);
    }

    fn deps_of(formula: &str) -> Deps {
        let ast = parse_formula(formula).unwrap();
        extract_deps(&ast, "S1", &|name| {
            (name == "Rate").then(|| parse_formula("Other!$A$1").unwrap())
        })
    }

    fn rect(sheet: &str, r1: u32, c1: u32, r2: u32, c2: u32) -> SheetRect {
        SheetRect {
            sheet: sheet.to_owned(),
            row1: r1,
            col1: c1,
            row2: r2,
            col2: c2,
        }
    }

    #[test]
    fn extracts_references() {
        let d = deps_of("SUM(A1:B2,Other!C3)+D4");
        assert_eq!(
            d.refs,
            vec![
                rect("S1", 1, 1, 2, 2),
                rect("Other", 3, 3, 3, 3),
                rect("S1", 4, 4, 4, 4)
            ]
        );
        assert!(!d.volatile);
    }

    #[test]
    fn extracts_whole_column_and_row_areas() {
        let d = deps_of("SUM(B:B)+SUM(2:3)");
        assert_eq!(
            d.refs,
            vec![rect("S1", 1, 2, MAX_ROW, 2), rect("S1", 2, 1, 3, MAX_COL)]
        );
    }

    #[test]
    fn extracts_names_and_expands_them() {
        let d = deps_of("Rate*2+Unknown");
        assert_eq!(d.names, vec!["Rate".to_owned(), "Unknown".to_owned()]);
        assert_eq!(d.refs, vec![rect("Other", 1, 1, 1, 1)]);
    }

    #[test]
    fn detects_volatile_functions() {
        assert!(deps_of("TODAY()+1").volatile);
        assert!(deps_of("IF(A1,NOW(),0)").volatile);
        assert!(!deps_of("SUM(A1)").volatile);
    }
}
