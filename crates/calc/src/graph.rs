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
use crate::tables::PlacedTable;

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
    /// Tables named by structured references (as written).
    pub tables: Vec<String>,
    /// The formula calls a volatile function such as `NOW`.
    pub volatile: bool,
    /// The formula calls `OFFSET` or `INDIRECT`: it reads cells that no reference in the formula names, so
    /// the engine re-evaluates it until those cells have settled.
    pub dynamic: bool,
    /// The formula calls `SUBTOTAL` or `AGGREGATE`, so other subtotals skip its cell.
    pub subtotal: bool,
    /// Cells the formula only asks about (`ISFORMULA(A1)`): the answer changes when the cell gains or loses a
    /// formula, but never depends on its value, so these are not dependencies (`=ISFORMULA(A1)` in A1 is no cycle).
    pub watched: Vec<SheetRect>,
}

/// Collects the references, defined names and volatility of a formula. `resolve` expands
/// defined names so the formula also depends on what they point to.
///
/// `cell` is the formula's own cell (1-based row, column): the relative references of a defined name resolve
/// against it, as the name's text is stored relative to `A1`.
pub fn extract_deps(
    ast: &Ast,
    sheet: &str,
    cell: (u32, u32),
    resolve: &dyn Fn(&str) -> Option<Ast>,
    table: &dyn Fn(&str) -> Option<PlacedTable>,
) -> Deps {
    let mut deps = Deps::default();
    let ctx = WalkCtx {
        sheet,
        cell,
        resolve,
        table,
    };
    walk(ast, &ctx, &mut deps, 0, None);
    deps
}

struct WalkCtx<'a> {
    sheet: &'a str,
    cell: (u32, u32),
    resolve: &'a dyn Fn(&str) -> Option<Ast>,
    table: &'a dyn Fn(&str) -> Option<PlacedTable>,
}

#[inline(never)]
fn walk_leaf(ast: &Ast, ctx: &WalkCtx<'_>, out: &mut Deps, shift: Option<(i64, i64)>) {
    let sheet = ctx.sheet;
    match ast {
        Ast::Ref(r) => {
            let (row, col) = match shift {
                Some((dr, dc)) => (
                    crate::ast::shift_coord(r.row, r.row_abs, dr, MAX_ROW),
                    crate::ast::shift_coord(r.col, r.col_abs, dc, MAX_COL),
                ),
                None => (r.row, r.col),
            };
            out.refs.push(SheetRect {
                sheet: r.sheet.clone().unwrap_or_else(|| sheet.to_owned()),
                row1: row,
                col1: col,
                row2: row,
                col2: col,
            });
        }
        Ast::Area(a) => {
            let a = &match shift {
                Some((dr, dc)) => crate::ast::shift_area(a, dr, dc),
                None => a.clone(),
            };
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
        Ast::Structured { table, spec } => {
            if !out.tables.iter().any(|t| t.eq_ignore_ascii_case(table)) {
                out.tables.push(table.clone());
            }
            if let Some(placed) = (ctx.table)(table)
                && let Ok((row1, col1, row2, col2)) =
                    crate::tables::resolve(&placed.def, spec, Some(ctx.cell.0))
            {
                out.refs.push(SheetRect {
                    sheet: placed.sheet,
                    row1,
                    col1,
                    row2,
                    col2,
                });
            }
        }
        _ => {}
    }
}

fn walk(ast: &Ast, ctx: &WalkCtx<'_>, out: &mut Deps, depth: u32, shift: Option<(i64, i64)>) {
    let resolve = ctx.resolve;
    match ast {
        Ast::Ref(_) | Ast::Area(_) | Ast::Structured { .. } => walk_leaf(ast, ctx, out, shift),
        Ast::Name(name) => {
            if !out.names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                out.names.push(name.clone());
            }
            if depth < MAX_NAME_DEPTH
                && let Some(target) = resolve(name)
            {
                let shift = shift.or(Some((i64::from(ctx.cell.0) - 1, i64::from(ctx.cell.1) - 1)));
                walk(&target, ctx, out, depth + 1, shift);
            }
        }
        Ast::Func { name, args, .. } => {
            if is_volatile(name) {
                out.volatile = true;
            }
            if name.eq_ignore_ascii_case("OFFSET") || name.eq_ignore_ascii_case("INDIRECT") {
                out.dynamic = true;
            }
            if name.eq_ignore_ascii_case("SUBTOTAL") || name.eq_ignore_ascii_case("AGGREGATE") {
                out.subtotal = true;
            }
            // `INDEX(area,r,3)` with a literal column (or row) reads that column (row) of the area only, so
            // `=INDEX(T[],ROW(),1)` inside column 2 of table `T` is no cycle.
            if name.eq_ignore_ascii_case("INDEX")
                && args.len() >= 3
                && walk_index(args, ctx, out, depth, shift)
            {
                return;
            }
            // `ROW(A1)`, `COLUMNS(A:Z)` and the base of `OFFSET(A1,..)` only use the position of a
            // reference, never its value: they do not depend on the cells (`=ROW(A1)` in A1 is no cycle).
            let position_only = ["ROW", "COLUMN", "ROWS", "COLUMNS"]
                .iter()
                .any(|n| n.eq_ignore_ascii_case(name));
            let offset = name.eq_ignore_ascii_case("OFFSET");
            let is_formula =
                name.eq_ignore_ascii_case("ISFORMULA") || name.eq_ignore_ascii_case("FORMULATEXT");
            for (i, a) in args.iter().enumerate() {
                let literal = matches!(a, Ast::Ref(_) | Ast::Area(_));
                if literal && is_formula {
                    let mut seen = Deps::default();
                    walk(a, ctx, &mut seen, depth, shift);
                    out.watched.extend(seen.refs);
                    continue;
                }
                if literal && (position_only || offset && i == 0) {
                    continue;
                }
                walk(a, ctx, out, depth, shift);
            }
        }
        Ast::Invoke { callee, args } => {
            walk(callee, ctx, out, depth, shift);
            for a in args {
                walk(a, ctx, out, depth, shift);
            }
        }
        Ast::Unary { operand, .. } => walk(operand, ctx, out, depth, shift),
        Ast::Percent(inner) => walk(inner, ctx, out, depth, shift),
        Ast::Binary { lhs, rhs, .. } => {
            walk(lhs, ctx, out, depth, shift);
            walk(rhs, ctx, out, depth, shift);
        }
        Ast::Array(rows) => {
            for item in rows.iter().flatten() {
                walk(item, ctx, out, depth, shift);
            }
        }
        Ast::Number(_) | Ast::Text(_) | Ast::Bool(_) | Ast::Error(_) | Ast::Missing => {}
    }
}

/// The dependencies of `INDEX(area,r,c)` with a literal row or column; false when `args[0]` is not a literal area.
#[inline(never)]
fn walk_index(
    args: &[Ast],
    ctx: &WalkCtx<'_>,
    out: &mut Deps,
    depth: u32,
    shift: Option<(i64, i64)>,
) -> bool {
    let Some((mut row1, mut col1, mut row2, mut col2)) = literal_rect(&args[0], ctx, shift) else {
        return false;
    };
    if let Some(k) = literal_index(&args[1])
        && k <= row2 - row1 + 1
    {
        (row1, row2) = (row1 + k - 1, row1 + k - 1);
    }
    if let Some(k) = literal_index(&args[2])
        && k <= col2 - col1 + 1
    {
        (col1, col2) = (col1 + k - 1, col1 + k - 1);
    }
    out.refs.push(SheetRect {
        sheet: literal_sheet(&args[0], ctx),
        row1,
        col1,
        row2,
        col2,
    });
    for a in &args[1..] {
        walk(a, ctx, out, depth, shift);
    }
    true
}

/// A positive whole number written as a literal.
fn literal_index(ast: &Ast) -> Option<u32> {
    match ast {
        Ast::Number(n) if *n >= 1.0 && n.fract() == 0.0 && *n <= f64::from(MAX_ROW) => {
            Some(*n as u32)
        }
        _ => None,
    }
}

/// The cell rectangle a literal area or structured reference names: `(row1, col1, row2, col2)`.
fn literal_rect(
    ast: &Ast,
    ctx: &WalkCtx<'_>,
    shift: Option<(i64, i64)>,
) -> Option<(u32, u32, u32, u32)> {
    match ast {
        Ast::Area(a) if a.kind == AreaKind::Cells => {
            let a = match shift {
                Some((dr, dc)) => crate::ast::shift_area(a, dr, dc),
                None => a.clone(),
            };
            Some((
                a.row1.min(a.row2),
                a.col1.min(a.col2),
                a.row1.max(a.row2),
                a.col1.max(a.col2),
            ))
        }
        Ast::Structured { table, spec } => {
            let placed = (ctx.table)(table)?;
            crate::tables::resolve(&placed.def, spec, Some(ctx.cell.0)).ok()
        }
        _ => None,
    }
}

/// The sheet of an area or structured reference accepted by [`literal_rect`].
fn literal_sheet(ast: &Ast, ctx: &WalkCtx<'_>) -> String {
    match ast {
        Ast::Area(a) => a.sheet.clone().unwrap_or_else(|| ctx.sheet.to_owned()),
        Ast::Structured { table, .. } => (ctx.table)(table).map_or_else(String::new, |p| p.sheet),
        _ => ctx.sheet.to_owned(),
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
        extract_deps(
            &ast,
            "S1",
            (1, 1),
            &|name| (name == "Rate").then(|| parse_formula("Other!$A$1").unwrap()),
            &|_| None,
        )
    }

    #[test]
    fn relative_name_refs_move_with_the_formula_cell() {
        let ast = parse_formula("Next+1").unwrap();
        let name = |n: &str| (n == "Next").then(|| parse_formula("S1!$B1").unwrap());
        let d = extract_deps(&ast, "S1", (5, 3), &name, &|_| None);
        assert_eq!(d.refs, vec![rect("S1", 5, 2, 5, 2)]);
        // Wraps past the last row, as Excel does.
        let name = |n: &str| (n == "Next").then(|| parse_formula("S1!$B1048576").unwrap());
        let d = extract_deps(&ast, "S1", (5, 3), &name, &|_| None);
        assert_eq!(d.refs, vec![rect("S1", 4, 2, 4, 2)]);
    }

    #[test]
    fn structured_reference_depends_on_its_rectangle() {
        use crate::tables::TableDef;
        let ast = parse_formula("SUM(T[Qty])+T[[#This Row],[Price]]").unwrap();
        let table = |n: &str| {
            n.eq_ignore_ascii_case("t").then(|| PlacedTable {
                sheet: "S1".into(),
                def: TableDef {
                    name: "T".into(),
                    rect: (2, 1, 6, 2),
                    header_rows: 1,
                    totals_rows: 0,
                    columns: vec!["Qty".into(), "Price".into()],
                },
            })
        };
        let d = extract_deps(&ast, "S1", (4, 5), &|_| None, &table);
        assert_eq!(d.refs, vec![rect("S1", 3, 1, 6, 1), rect("S1", 4, 2, 4, 2)]);
        assert_eq!(d.tables, vec!["T".to_owned()]);
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
