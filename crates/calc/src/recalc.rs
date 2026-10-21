//! Dependency graph and recalculation.
//!
//! The graph maps every formula cell to its static precedents (resolved areas). A reverse index
//! answers "which formulas read this cell?": single-cell precedents in a hash map, ranges as row
//! intervals per column (wide ranges per sheet), each an interval index so a lookup costs
//! O(log n + matches) however many ranges share the column. After an edit, the dirty set is the
//! transitive closure of dependents; it is evaluated in topological order, with on-demand
//! evaluation as a safety net for dynamic references. Cycles produce `#CIRC!` unless iterative
//! calculation is on: then a formula read while it is being evaluated gives its previous value,
//! and the formulas that read one are evaluated again until the loop settles
//! (`PassHost::iterate_loops`).
//!
//! On native builds, formulas whose static precedents are not themselves dirty (and that are not
//! dynamic) are evaluated together, one wave at a time, on several threads. Dynamic formulas
//! (`INDIRECT`, `OFFSET`, `RAND`, names, …), iterative calculation, cycles and WASM stay on the
//! calling thread: they share the random stream and discover dependencies while evaluating.
//! A panic inside a worker is caught and the dirty set is evaluated on the caller instead.

use std::collections::{BTreeSet, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};

/// A fast, non-cryptographic hasher for cell keys (FxHash-style multiply-rotate).
#[derive(Default, Clone, Copy)]
pub struct FxHasher(u64);

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(*b as u64);
        }
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64);
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
}

type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FxHasher>>;
type HashSet<K> = std::collections::HashSet<K, BuildHasherDefault<FxHasher>>;

use gridcraft_core::{Array, CellError, CellRef, RangeRef, Value};
use gridcraft_formula::Expr;
#[cfg(not(target_arch = "wasm32"))]
use gridcraft_formula::{BinOp, SheetSel, UnOp};
use gridcraft_model::{CalcMode, Workbook};

#[cfg(not(target_arch = "wasm32"))]
use crate::eval::binary_values;
use crate::eval::{Area, Evaluator, Host, precedents};

pub type Key = (usize, CellRef);

#[derive(Clone, Debug)]
struct Node {
    areas: Vec<Area>,
    dynamic: bool,
    /// Parsed formula, kept so a recalc does not look the expression up in the cell store again.
    expr: Option<std::sync::Arc<Expr>>,
    /// Legacy array-formula range. `None` for an ordinary formula, so a flat wave does not
    /// look the cell up again just to learn that.
    // Wasm does not read this field; native code does.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    legacy: Option<RangeRef>,
}

/// Ranges wider than this many columns go to the per-sheet wide index instead of per-column ones.
const WIDE: u32 = 64;

/// Insertions an [`Intervals`] keeps aside (scanned by every query) before merging them in.
const PENDING: usize = 32;

/// Formula evaluations iterative calculation may spend on one recalculation (passes × formulas
/// evaluated again), however high Maximum Iterations is.
const MAX_ITERATION_WORK: usize = 10_000_000;

/// Row intervals `[r0, r1]`, each with a payload, answering "which intervals overlap rows
/// `a..=b`?" in O(log n + k).
///
/// The intervals are sorted by start with an implicit segment tree of their largest end, so a query
/// only descends into subtrees that hold a match. Removals mark entries dead (re-inserting one
/// revives it); insertions wait in a small sorted set. [`Intervals::settle`] merges them and drops
/// dead entries once either grows, so mutations cost O(log n) amortised.
#[derive(Clone, Debug)]
struct Intervals<T> {
    /// Sorted, without duplicates.
    items: Vec<(u32, u32, T)>,
    dead: Vec<bool>,
    dead_count: usize,
    /// Segment tree over `items`: node `i` has children `2i` and `2i + 1`, leaves start at
    /// `max_end.len() / 2`. Each node holds the largest end below it (dead entries included).
    max_end: Vec<u32>,
    /// Inserted since the last merge.
    pending: BTreeSet<(u32, u32, T)>,
}

impl<T> Default for Intervals<T> {
    fn default() -> Self {
        Intervals { items: Vec::new(), dead: Vec::new(), dead_count: 0, max_end: Vec::new(), pending: BTreeSet::new() }
    }
}

impl<T: Copy + Ord> Intervals<T> {
    fn is_empty(&self) -> bool {
        self.items.len() == self.dead_count && self.pending.is_empty()
    }

    fn insert(&mut self, r0: u32, r1: u32, t: T) {
        let it = (r0, r1, t);
        if let Ok(i) = self.items.binary_search(&it) {
            if let Some(d) = self.dead.get_mut(i)
                && *d
            {
                *d = false;
                self.dead_count -= 1;
            }
            return;
        }
        self.pending.insert(it);
    }

    fn remove(&mut self, r0: u32, r1: u32, t: T) {
        let it = (r0, r1, t);
        if let Ok(i) = self.items.binary_search(&it) {
            if let Some(d) = self.dead.get_mut(i)
                && !*d
            {
                *d = true;
                self.dead_count += 1;
            }
            return;
        }
        self.pending.remove(&it);
    }

    /// Merges pending insertions and drops dead entries when there are enough of them.
    fn settle(&mut self) {
        if self.pending.len() <= PENDING && self.dead_count <= (self.items.len() / 2).max(PENDING) {
            return;
        }
        let mut all: Vec<(u32, u32, T)> = Vec::with_capacity(self.items.len() - self.dead_count + self.pending.len());
        all.extend(self.items.iter().zip(&self.dead).filter(|(_, d)| !**d).map(|(it, _)| *it));
        all.extend(std::mem::take(&mut self.pending));
        // Two sorted runs: the stable sort merges them in linear time.
        all.sort();
        all.dedup();
        let size = all.len().next_power_of_two();
        let mut max_end = vec![0; 2 * size];
        if let Some(leaves) = max_end.get_mut(size..) {
            for (slot, it) in leaves.iter_mut().zip(&all) {
                *slot = it.1;
            }
        }
        for i in (1..size).rev() {
            let m = max_end.get(2 * i).copied().unwrap_or(0).max(max_end.get(2 * i + 1).copied().unwrap_or(0));
            if let Some(slot) = max_end.get_mut(i) {
                *slot = m;
            }
        }
        self.dead = vec![false; all.len()];
        self.dead_count = 0;
        self.items = all;
        self.max_end = max_end;
    }

    /// Calls `f` for every interval overlapping rows `a..=b` (`r0 <= b && r1 >= a`). Returns the
    /// number of tree nodes visited.
    fn query(&self, a: u32, b: u32, f: &mut dyn FnMut(&T)) -> usize {
        for it in &self.pending {
            if it.0 <= b && it.1 >= a {
                f(&it.2);
            }
        }
        // Entries before `p` start at or before row `b`.
        let p = self.items.partition_point(|it| it.0 <= b);
        if p == 0 {
            return 0;
        }
        self.walk(1, 0, self.max_end.len() / 2, p, a, f)
    }

    fn walk(&self, node: usize, lo: usize, hi: usize, p: usize, a: u32, f: &mut dyn FnMut(&T)) -> usize {
        if lo >= p || self.max_end.get(node).is_none_or(|m| *m < a) {
            return 1;
        }
        if hi - lo <= 1 {
            if let (Some(it), Some(false)) = (self.items.get(lo), self.dead.get(lo))
                && it.1 >= a
            {
                f(&it.2);
            }
            return 1;
        }
        let mid = lo + (hi - lo) / 2;
        1 + self.walk(2 * node, lo, mid, p, a, f) + self.walk(2 * node + 1, mid, hi, p, a, f)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    nodes: HashMap<Key, Node>,
    /// single cell → dependents
    cell_deps: HashMap<Key, HashSet<Key>>,
    /// (sheet, col) → row intervals of the ranges (up to `WIDE` columns) that cover the column
    col_deps: HashMap<(usize, u32), Intervals<Key>>,
    /// sheet → row intervals of wider ranges, with their columns: (first, last, dependent)
    wide_deps: HashMap<usize, Intervals<(u32, u32, Key)>>,
    dynamic: HashSet<Key>,
    /// Range indexes changed since the last [`Graph::settle`].
    unsettled_cols: HashSet<(usize, u32)>,
    unsettled_wide: HashSet<usize>,
}

impl Graph {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Index entries of a formula's precedents, without duplicates: single cells, column
    /// intervals and wide ranges.
    fn entries(areas: &[Area]) -> (Vec<Key>, Vec<(usize, u32, u32, u32)>, Vec<(usize, RangeRef)>) {
        let (mut cells, mut cols, mut wide) = (Vec::new(), Vec::new(), Vec::new());
        for a in areas {
            if a.range.is_single() {
                cells.push((a.sheet, a.range.start));
            } else if a.range.width() > WIDE {
                wide.push((a.sheet, a.range));
            } else {
                for c in a.range.start.col..=a.range.end.col {
                    cols.push((a.sheet, c, a.range.start.row, a.range.end.row));
                }
            }
        }
        cells.sort_unstable();
        cells.dedup();
        cols.sort_unstable();
        cols.dedup();
        wide.sort_unstable_by_key(|(s, r)| (*s, r.start, r.end));
        wide.dedup();
        (cells, cols, wide)
    }

    fn insert(&mut self, k: Key, node: Node) {
        self.remove(k);
        let (cells, cols, wide) = Self::entries(&node.areas);
        for c in cells {
            self.cell_deps.entry(c).or_default().insert(k);
        }
        for (s, c, r0, r1) in cols {
            self.col_deps.entry((s, c)).or_default().insert(r0, r1, k);
            self.unsettled_cols.insert((s, c));
        }
        for (s, r) in wide {
            self.wide_deps.entry(s).or_default().insert(r.start.row, r.end.row, (r.start.col, r.end.col, k));
            self.unsettled_wide.insert(s);
        }
        if node.dynamic {
            self.dynamic.insert(k);
        }
        self.nodes.insert(k, node);
    }

    fn remove(&mut self, k: Key) {
        let Some(old) = self.nodes.remove(&k) else { return };
        let (cells, cols, wide) = Self::entries(&old.areas);
        for c in cells {
            if let Some(v) = self.cell_deps.get_mut(&c) {
                v.remove(&k);
                if v.is_empty() {
                    self.cell_deps.remove(&c);
                }
            }
        }
        for (s, c, r0, r1) in cols {
            if let Some(v) = self.col_deps.get_mut(&(s, c)) {
                v.remove(r0, r1, k);
                self.unsettled_cols.insert((s, c));
            }
        }
        for (s, r) in wide {
            if let Some(v) = self.wide_deps.get_mut(&s) {
                v.remove(r.start.row, r.end.row, (r.start.col, r.end.col, k));
                self.unsettled_wide.insert(s);
            }
        }
        self.dynamic.remove(&k);
    }

    /// Tidies the range indexes changed by `insert`/`remove` (queries stay correct without it,
    /// just slower).
    fn settle(&mut self) {
        for key in std::mem::take(&mut self.unsettled_cols) {
            if let Some(v) = self.col_deps.get_mut(&key) {
                v.settle();
                if v.is_empty() {
                    self.col_deps.remove(&key);
                }
            }
        }
        for key in std::mem::take(&mut self.unsettled_wide) {
            if let Some(v) = self.wide_deps.get_mut(&key) {
                v.settle();
                if v.is_empty() {
                    self.wide_deps.remove(&key);
                }
            }
        }
    }

    /// Formulas that read cell `c` of `sheet`.
    pub fn dependents(&self, sheet: usize, c: CellRef, out: &mut Vec<Key>) {
        if let Some(v) = self.cell_deps.get(&(sheet, c)) {
            out.extend(v.iter().copied());
        }
        if let Some(v) = self.col_deps.get(&(sheet, c.col)) {
            v.query(c.row, c.row, &mut |k| out.push(*k));
        }
        if let Some(v) = self.wide_deps.get(&sheet) {
            v.query(c.row, c.row, &mut |(c0, c1, k)| {
                if *c0 <= c.col && c.col <= *c1 {
                    out.push(*k);
                }
            });
        }
    }

    /// Formulas that read any cell of `range` (a formula can be listed more than once).
    pub fn dependents_of_range(&self, sheet: usize, range: RangeRef, out: &mut Vec<Key>) {
        if range.count() <= 4096 {
            for c in range.iter() {
                if let Some(v) = self.cell_deps.get(&(sheet, c)) {
                    out.extend(v.iter().copied());
                }
            }
        } else {
            for ((s, c), v) in &self.cell_deps {
                if *s == sheet && range.contains(*c) {
                    out.extend(v.iter().copied());
                }
            }
        }
        let (r0, r1) = (range.start.row, range.end.row);
        if (range.width() as usize) <= self.col_deps.len() {
            for col in range.start.col..=range.end.col {
                if let Some(v) = self.col_deps.get(&(sheet, col)) {
                    v.query(r0, r1, &mut |k| out.push(*k));
                }
            }
        } else {
            for ((s, col), v) in &self.col_deps {
                if *s == sheet && range.start.col <= *col && *col <= range.end.col {
                    v.query(r0, r1, &mut |k| out.push(*k));
                }
            }
        }
        if let Some(v) = self.wide_deps.get(&sheet) {
            v.query(r0, r1, &mut |(c0, c1, k)| {
                if *c0 <= range.end.col && range.start.col <= *c1 {
                    out.push(*k);
                }
            });
        }
    }

    /// Precedent areas of a formula cell (for Trace Precedents).
    pub fn precedents_of(&self, k: Key) -> Vec<Area> {
        self.nodes.get(&k).map(|n| n.areas.clone()).unwrap_or_default()
    }
}

/// Calculation state kept beside a workbook.
#[derive(Clone, Debug)]
pub struct Calc {
    pub graph: Graph,
    rng: u64,
    /// Fixed "now" for deterministic tests; `None` = system clock.
    pub fixed_now: Option<f64>,
    /// Cells whose formulas have errors from cycles.
    pub circular: Vec<Key>,
    pub last_recalc_cells: usize,
}

impl Default for Calc {
    fn default() -> Self {
        Calc { graph: Graph::default(), rng: 0x9E37_79B9_7F4A_7C15, fixed_now: None, circular: vec![], last_recalc_cells: 0 }
    }
}

struct PassHost<'a> {
    wb: &'a Workbook,
    exprs: &'a HashMap<Key, std::sync::Arc<Expr>>,
    results: HashMap<Key, Value>,
    pending: HashSet<Key>,
    in_progress: HashSet<Key>,
    /// New spills found during the pass: anchor → array.
    spills: HashMap<Key, std::sync::Arc<Array>>,
    /// Used range per sheet: the workbook doesn't change during a pass, and working it out walks
    /// every row, so once per sheet rather than once per range read.
    used: HashMap<usize, Option<RangeRef>>,
    rng: &'a mut u64,
    now: f64,
    cycle: bool,
    cycles: Vec<Key>,
    depth: usize,
    /// Iterative calculation: a circular reference reads the cell's previous value instead of
    /// giving `#CIRC!`.
    iterate: bool,
    /// Values from the previous iteration; cells not in it read their stored value.
    prev: HashMap<Key, Value>,
    /// Formulas being evaluated, innermost last (kept only when iterating).
    stack: Vec<Key>,
    /// Formulas found on a circular reference (when iterating).
    members: HashSet<Key>,
    /// Results that read a previous value, directly or through other formulas.
    tainted: HashSet<Key>,
    /// Whether the formula being evaluated has read a previous value.
    taint: bool,
}

impl<'a> PassHost<'a> {
    fn new(wb: &'a Workbook, exprs: &'a HashMap<Key, std::sync::Arc<Expr>>, rng: &'a mut u64, now: f64) -> Self {
        PassHost {
            wb,
            exprs,
            results: HashMap::with_capacity_and_hasher(exprs.len(), Default::default()),
            pending: exprs.keys().copied().collect(),
            in_progress: HashSet::default(),
            spills: HashMap::default(),
            used: HashMap::default(),
            rng,
            now,
            cycle: false,
            cycles: vec![],
            depth: 0,
            iterate: false,
            prev: HashMap::default(),
            stack: Vec::new(),
            members: HashSet::default(),
            tainted: HashSet::default(),
            taint: false,
        }
    }
}

/// Whether an array result at `k` can't spill: it is in a table, or its area runs off the sheet,
/// overlaps merged cells, holds other cells, or another formula's spill (unless that formula is
/// recalculated in this pass, and so may spill elsewhere now).
fn spill_blocked(wb: &Workbook, exprs: &HashMap<Key, std::sync::Arc<Expr>>, (sheet, anchor): Key, a: &Array) -> bool {
    let Some(sh) = wb.sheet(sheet) else { return false };
    let end =
        CellRef::new(anchor.row.saturating_add((a.rows as u32).saturating_sub(1)), anchor.col.saturating_add((a.cols as u32).saturating_sub(1)));
    let range = RangeRef::new(anchor, end);
    let own = sh.spill_ranges.get(&anchor);
    let spilled_over = |c: CellRef| {
        !own.is_some_and(|o| o.contains(c)) && sh.spill_ranges.iter().any(|(o, r)| *o != anchor && r.contains(c) && !exprs.contains_key(&(sheet, *o)))
    };
    end.row >= gridcraft_core::MAX_ROWS
        || end.col >= gridcraft_core::MAX_COLS
        || sh.merges.iter().any(|m| m.intersects(&range))
        || sh.tables.iter().any(|t| t.range.contains(anchor))
        || sh.cells.iter_range(range).any(|(c, cell)| c != anchor && (!cell.value.is_empty() || cell.formula.is_some()))
        || sh.spill.range(range.start..=range.end).any(|(c, _)| range.contains(*c) && spilled_over(*c))
}

/// Spill cells inside `range`, excluding each array's anchor (that cell is stored and visited
/// with the formula). Sheet spills are filled first; a spill computed in this pass overwrites
/// them. `None` when an array is too large to enumerate: the caller then takes the dense path.
fn spill_cells_in_range(
    sheet: usize,
    range: RangeRef,
    stored: &std::collections::BTreeMap<CellRef, Value>,
    spills: &HashMap<Key, std::sync::Arc<Array>>,
) -> Option<Vec<(CellRef, Value)>> {
    if stored.is_empty() && spills.is_empty() {
        return Some(Vec::new());
    }
    let mut out: std::collections::BTreeMap<CellRef, Value> = std::collections::BTreeMap::new();
    for (c, v) in stored.range(range.start..=range.end) {
        if range.contains(*c) {
            out.insert(*c, v.clone());
        }
    }
    for (anchor, arr) in spills {
        if anchor.0 != sheet {
            continue;
        }
        let n = (arr.rows as u64).checked_mul(arr.cols as u64)?;
        if n > crate::eval::MAX_CELLS {
            return None;
        }
        for r in 0..arr.rows {
            for cc in 0..arr.cols {
                let c = CellRef::new(anchor.1.row.saturating_add(r as u32), anchor.1.col.saturating_add(cc as u32));
                if c == anchor.1 || !range.contains(c) {
                    continue;
                }
                out.insert(c, arr.get(r, cc).cloned().unwrap_or_default());
            }
        }
        if out.len() as u64 > crate::eval::MAX_CELLS {
            return None;
        }
    }
    if out.len() as u64 > crate::eval::MAX_CELLS {
        return None;
    }
    Some(out.into_iter().collect())
}

impl PassHost<'_> {
    fn compute(&mut self, k: Key) -> Value {
        if let Some(v) = self.results.get(&k) {
            let v = v.clone();
            if self.iterate && self.tainted.contains(&k) {
                self.taint = true;
            }
            return v;
        }
        let Some(expr) = self.exprs.get(&k) else {
            return self.wb.sheet(k.0).map(|s| s.value(k.1)).unwrap_or_default();
        };
        if self.in_progress.contains(&k) || self.depth > 2000 {
            self.cycle = true;
            if self.iterate {
                return self.circular_read(k);
            }
            self.cycles.push(k);
            return Value::Error(CellError::Circ);
        }
        self.in_progress.insert(k);
        self.depth += 1;
        let outer_taint = std::mem::replace(&mut self.taint, false);
        if self.iterate {
            self.stack.push(k);
        }
        let expr = std::sync::Arc::clone(expr);
        let v = {
            let mut ev = Evaluator::new(self, k.0, k.1);
            ev.value(&expr)
        };
        self.depth -= 1;
        self.in_progress.remove(&k);
        self.pending.remove(&k);
        if self.iterate {
            self.stack.pop();
            if self.taint {
                self.tainted.insert(k);
            } else {
                self.tainted.remove(&k);
            }
        }
        self.taint |= outer_taint;
        let (v, spill) = finish_formula(self.wb, self.exprs, k, &expr, v, stored_legacy(self.wb, k), true);
        if let Some(a) = spill {
            self.spills.insert(k, a);
        }
        self.results.insert(k, v.clone());
        v
    }

    /// A formula's previous value for iterative calculation: from the last iteration, else the
    /// value it shows. Blank, and the `#CIRC!` of a loop calculated with iteration off, are 0.
    fn previous(&self, k: Key) -> Value {
        let v = match self.prev.get(&k) {
            Some(v) => v.clone(),
            None => self.wb.sheet(k.0).map(|s| s.value(k.1)).unwrap_or_default(),
        };
        if matches!(v, Value::Empty | Value::Error(CellError::Circ)) { Value::Number(0.0) } else { v }
    }

    /// Reading `k` while it is being evaluated, with iterative calculation on: its previous value.
    /// The formulas around the loop are the ones whose changes decide when iteration stops.
    fn circular_read(&mut self, k: Key) -> Value {
        self.taint = true;
        match self.stack.iter().rposition(|s| *s == k) {
            Some(i) => self.members.extend(self.stack.get(i..).unwrap_or_default().iter().copied()),
            // Too deep to follow: the formula itself is watched.
            None => {
                self.members.insert(k);
            }
        }
        self.previous(k)
    }

    /// Iterative calculation, after a first pass over `order` that met circular references. The
    /// formulas that read a previous value (the loops and what depends on them) are evaluated
    /// again in the same order, each pass starting from the last one's values and reading what it
    /// has already produced (Gauss-Seidel), until no formula on a loop changes by `max_change` or
    /// more, or after `max_iterations` passes in all. Returns the number of passes.
    fn iterate_loops(&mut self, order: &[Key], max_iterations: u32, max_change: f64) -> u32 {
        let again: Vec<Key> = order.iter().copied().filter(|k| self.tainted.contains(k)).collect();
        let set: HashSet<Key> = again.iter().copied().collect();
        // Bounds the work whatever Maximum Iterations says.
        let max_passes = u32::try_from(MAX_ITERATION_WORK / again.len().max(1)).unwrap_or(u32::MAX).max(1);
        let mut passes = 1u32;
        while passes < max_iterations.min(max_passes) && !self.converged(max_change) {
            self.prev = again.iter().filter_map(|k| self.results.remove(k).map(|v| (*k, v))).collect();
            self.pending = set.clone();
            self.spills.retain(|a, _| !set.contains(a));
            for k in &again {
                self.compute(*k);
            }
            passes += 1;
        }
        passes
    }

    /// Whether every formula on a loop changed by less than `max_change` in the last pass (text,
    /// logical and error values: not at all).
    fn converged(&self, max_change: f64) -> bool {
        self.members.iter().all(|k| {
            let Some(new) = self.results.get(k) else { return true };
            match (new, &self.previous(*k)) {
                (Value::Number(a), Value::Number(b)) => {
                    let d = (a - b).abs();
                    d < max_change || d == 0.0
                }
                (new, old) => new == old,
            }
        })
    }
}

/// The value a formula shows, and the array it spills (when it does). Shared by the single-thread
/// pass and the parallel waves so both lay out blanks, legacy arrays and `#SPILL!` the same way.
fn stored_legacy(wb: &Workbook, k: Key) -> Option<RangeRef> {
    wb.sheet(k.0).and_then(|s| s.cell(k.1)).and_then(|c| c.formula.as_ref()).and_then(|f| f.array)
}

fn finish_formula(
    wb: &Workbook,
    exprs: &HashMap<Key, std::sync::Arc<Expr>>,
    k: Key,
    expr: &Expr,
    v: Value,
    legacy: Option<RangeRef>,
    block_spills: bool,
) -> (Value, Option<std::sync::Arc<Array>>) {
    let v = match legacy {
        Some(range) => fit_to_range(v, range),
        None => v,
    };
    // A blank cell shows as 0 in a formula's result, inside an array as well (`=A1:A3`,
    // FILTER or SORT of a range with blanks). GROUPBY and PIVOTBY lay out blank cells
    // of their own, which stay blank.
    let keep_blanks = matches!(expr, Expr::Call(n, _) if matches!(n.as_str(), "GROUPBY" | "PIVOTBY"));
    let (v, spill) = match v {
        Value::Array(a) => {
            let a = if keep_blanks || !a.data.iter().any(|x| matches!(x, Value::Empty)) {
                a
            } else {
                let data = a.data.iter().map(|x| if matches!(x, Value::Empty) { Value::Number(0.0) } else { x.clone() }).collect();
                Array::new(a.rows, a.cols, data).map(std::sync::Arc::new).unwrap_or(a)
            };
            let spill = (a.rows > 1 || a.cols > 1).then(|| a.clone());
            (a.data.first().cloned().unwrap_or_default(), spill)
        }
        Value::Empty => (Value::Number(0.0), None),
        v => (v, None),
    };
    // The flat wave lays spills out afterwards, so it must not turn one into `#SPILL!` yet.
    if !block_spills {
        return (v, spill);
    }
    // An array that can't spill is #SPILL! for the formulas evaluated after it in this pass too.
    match spill {
        Some(a) if spill_blocked(wb, exprs, k, &a) => (Value::Error(CellError::Spill), None),
        other => (v, other),
    }
}

/// A legacy (Ctrl+Shift+Enter) array formula's result fitted to the range it was entered in, as
/// Excel does: a single row or column repeats across the range, a single value fills it, cells
/// past the result show `#N/A`, and a result larger than the range is cut off.
fn fit_to_range(v: Value, range: RangeRef) -> Value {
    if range.count() > crate::eval::MAX_CELLS {
        return Value::Error(CellError::Num);
    }
    let (h, w) = (range.height() as usize, range.width() as usize);
    let Value::Array(a) = v else {
        return if h * w == 1 { v } else { Array::new(h, w, vec![v; h * w]).map(Value::from).unwrap_or(Value::Error(CellError::Value)) };
    };
    if h * w == 1 {
        return a.data.first().cloned().unwrap_or_default();
    }
    let mut data = Vec::with_capacity(h * w);
    for r in 0..h {
        for c in 0..w {
            let (r, c) = (if a.rows == 1 { 0 } else { r }, if a.cols == 1 { 0 } else { c });
            data.push(a.get(r, c).cloned().unwrap_or(Value::Error(CellError::NA)));
        }
    }
    Array::new(h, w, data).map(Value::from).unwrap_or(Value::Error(CellError::Value))
}

impl Host for PassHost<'_> {
    fn workbook(&self) -> &Workbook {
        self.wb
    }
    fn spill_range(&mut self, sheet: usize, anchor: CellRef) -> Option<RangeRef> {
        let k = (sheet, anchor);
        if !(self.pending.contains(&k) || self.results.contains_key(&k)) {
            return self.wb.sheet(sheet)?.spill_ranges.get(&anchor).copied();
        }
        // Recalculated in this pass: its new array, unless something blocks it (#SPILL!).
        self.compute(k);
        let arr = self.spills.get(&k)?;
        let end = CellRef::new(
            anchor.row.saturating_add((arr.rows as u32).saturating_sub(1)),
            anchor.col.saturating_add((arr.cols as u32).saturating_sub(1)),
        );
        let range = RangeRef::new(anchor, end);
        let sh = self.wb.sheet(sheet)?;
        let blocked = end.row >= gridcraft_core::MAX_ROWS
            || end.col >= gridcraft_core::MAX_COLS
            || sh.cells.iter_range(range).any(|(c, cell)| c != anchor && (cell.formula.is_some() || !cell.value.is_empty()))
            || sh.merges.iter().any(|m| m.intersects(&range));
        (!blocked).then_some(range)
    }
    fn cell_value(&mut self, sheet: usize, c: CellRef) -> Value {
        let k = (sheet, c);
        if self.pending.contains(&k) || self.results.contains_key(&k) {
            return self.compute(k);
        }
        // A cell covered by a spill computed in this pass.
        for (anchor, arr) in &self.spills {
            if anchor.0 == sheet && c.row >= anchor.1.row && c.col >= anchor.1.col {
                let (dr, dc) = ((c.row - anchor.1.row) as usize, (c.col - anchor.1.col) as usize);
                if dr < arr.rows && dc < arr.cols {
                    if self.iterate && self.tainted.contains(anchor) {
                        self.taint = true;
                    }
                    return arr.get(dr, dc).cloned().unwrap_or_default();
                }
            }
        }
        self.wb.sheet(sheet).map(|s| s.value(c)).unwrap_or_default()
    }
    fn range_values(&mut self, sheet: usize, range: RangeRef) -> Vec<Value> {
        let (h, w) = (range.height() as usize, range.width() as usize);
        let mut out = vec![Value::Empty; h * w];
        let Some(sh) = self.wb.sheet(sheet) else { return out };
        // Sparse walk: only stored cells; formula cells that are (or were) dirty go through
        // `compute`, constants are copied straight from the store.
        let mut formulas: Vec<(usize, CellRef)> = Vec::new();
        for (c, cell) in sh.cells.iter_range(range) {
            let i = (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize;
            if cell.formula.is_some() {
                formulas.push((i, c));
            } else if let Some(slot) = out.get_mut(i) {
                *slot = cell.value.clone();
            }
        }
        for (c, v) in sh.spill.range(range.start..=range.end) {
            if range.contains(*c) {
                let i = (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize;
                if let Some(slot) = out.get_mut(i)
                    && slot.is_empty()
                {
                    *slot = v.clone();
                }
            }
        }
        for (i, c) in formulas {
            let v = self.cell_value(sheet, c);
            if let Some(slot) = out.get_mut(i) {
                *slot = v;
            }
        }
        if !self.spills.is_empty() {
            let new: Vec<(usize, Value)> = self
                .spills
                .iter()
                .filter(|(a, _)| a.0 == sheet)
                .flat_map(|(a, arr)| {
                    let mut v = Vec::new();
                    for r in 0..arr.rows {
                        for cc in 0..arr.cols {
                            let c = CellRef::new(a.1.row.saturating_add(r as u32), a.1.col.saturating_add(cc as u32));
                            if c != a.1 && range.contains(c) {
                                v.push((
                                    (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize,
                                    arr.get(r, cc).cloned().unwrap_or_default(),
                                ));
                            }
                        }
                    }
                    v
                })
                .collect();
            // Iterating: an array spilled by a formula that read a previous value.
            if self.iterate
                && self.spills.iter().any(|(a, arr)| {
                    let end = CellRef::new(
                        a.1.row.saturating_add((arr.rows as u32).saturating_sub(1)),
                        a.1.col.saturating_add((arr.cols as u32).saturating_sub(1)),
                    );
                    a.0 == sheet && self.tainted.contains(a) && RangeRef::new(a.1, end).intersects(&range)
                })
            {
                self.taint = true;
            }
            for (i, v) in new {
                if let Some(slot) = out.get_mut(i) {
                    *slot = v;
                }
            }
        }
        out
    }
    fn visit_range(&mut self, sheet: usize, range: RangeRef, f: &mut dyn FnMut(&Value) -> bool) -> bool {
        let wb = self.wb;
        let Some(sh) = wb.sheet(sheet) else { return false };
        // Formula cells first, in row-major order like `range_values`: reading one calculates it
        // and the pass remembers the value and any spill, so the walk below sees both.
        for (c, cell) in sh.cells.iter_range(range) {
            if cell.formula.is_some() {
                let _ = self.cell_value(sheet, c);
            }
        }
        let Some(extras) = spill_cells_in_range(sheet, range, &sh.spill, &self.spills) else {
            return false;
        };
        // Stored cells and spilled numbers, row-major. The anchor is the stored formula, so it is
        // not also taken from the array. A spill that misses the block adds nothing here.
        let mut i = 0;
        for (c, cell) in sh.cells.iter_range(range) {
            while i < extras.len() && extras[i].0 < c {
                if !f(&extras[i].1) {
                    return true;
                }
                i += 1;
            }
            let go = if cell.formula.is_some() {
                if i < extras.len() && extras[i].0 == c {
                    i += 1;
                }
                let v = self.cell_value(sheet, c);
                f(&v)
            } else if i < extras.len() && extras[i].0 == c {
                let v = extras[i].1.clone();
                i += 1;
                f(&v)
            } else {
                f(&cell.value)
            };
            if !go {
                return true;
            }
        }
        while i < extras.len() {
            if !f(&extras[i].1) {
                return true;
            }
            i += 1;
        }
        true
    }
    fn used_range(&mut self, sheet: usize) -> Option<RangeRef> {
        let wb = self.wb;
        *self.used.entry(sheet).or_insert_with(|| wb.sheet(sheet).and_then(|s| s.used_range()))
    }
    fn now_serial(&self) -> f64 {
        self.now
    }
    fn random(&mut self) -> f64 {
        // xorshift64*
        let mut x = *self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

impl Calc {
    pub fn new() -> Self {
        Self::default()
    }

    fn now(&self) -> f64 {
        if let Some(n) = self.fixed_now {
            return n;
        }
        now_serial()
    }

    /// Rebuilds the graph from scratch.
    pub fn rebuild(&mut self, wb: &Workbook) {
        self.graph = Graph::default();
        for (si, sheet) in wb.sheets.iter().enumerate() {
            for (c, cell) in sheet.cells.iter() {
                if let Some(f) = &cell.formula
                    && let Some(e) = f.expr_arc()
                {
                    let (areas, dynamic) = precedents(wb, si, &e);
                    self.graph.insert((si, c), Node { areas, dynamic, expr: Some(e), legacy: f.array });
                }
            }
        }
        self.graph.settle();
    }

    /// Recalculates every formula (F9 / Ctrl+Alt+F9, and after loading).
    pub fn recalc_all(&mut self, wb: &mut Workbook) {
        self.rebuild(wb);
        // Spills are laid out again from the formulas where they are now (a structural edit moves
        // or deletes the formulas, not the values they spilled).
        for i in 0..wb.sheets.len() {
            if wb.sheet(i).is_some_and(|s| !s.spill_ranges.is_empty() || !s.spill.is_empty())
                && let Some(sh) = wb.sheet_mut(i)
            {
                sh.spill.clear();
                sh.spill_ranges.clear();
            }
        }
        let all: Vec<Key> = self.graph.nodes.keys().copied().collect();
        self.run(wb, all, true);
    }

    /// Updates the graph for cells whose content changed and recalculates what depends on them.
    pub fn cells_changed(&mut self, wb: &mut Workbook, changed: &[Key]) {
        let mut cleared: Vec<(usize, RangeRef)> = Vec::new();
        for &k in changed {
            let formula = wb.sheet(k.0).and_then(|s| s.cell(k.1)).and_then(|c| c.formula.clone());
            match formula.as_ref().and_then(|f| f.expr_arc()) {
                Some(e) => {
                    let (areas, dynamic) = precedents(wb, k.0, &e);
                    let legacy = formula.as_ref().and_then(|f| f.array);
                    self.graph.insert(k, Node { areas, dynamic, expr: Some(e), legacy });
                }
                None => {
                    self.graph.remove(k);
                    // A formula replaced by a constant (or cleared) takes its spilled values with it.
                    if let Some(sh) = wb.sheet_mut(k.0)
                        && let Some(r) = sh.spill_ranges.remove(&k.1)
                    {
                        for c in r.iter() {
                            sh.spill.remove(&c);
                        }
                        cleared.push((k.0, r));
                    }
                    // A formula that doesn't parse evaluates to #NAME?.
                    if let Some(cell) = wb.sheet_mut(k.0).and_then(|s| s.cells.get_mut(k.1))
                        && cell.formula.is_some()
                    {
                        cell.value = Value::Error(CellError::Name);
                    }
                }
            }
        }
        self.graph.settle();
        if wb.calc.mode == CalcMode::Manual {
            // Only the edited formulas themselves are evaluated.
            let own: Vec<Key> = changed.iter().copied().filter(|k| self.graph.nodes.contains_key(k)).collect();
            self.run(wb, own, false);
            return;
        }
        let mut seeds: Vec<Key> = changed.to_vec();
        let mut emptied: Vec<Key> = Vec::new();
        // Typing into (or clearing) a cell a dynamic array spills over (or would spill over)
        // re-evaluates the anchor, which may now be blocked or unblocked.
        for &k in changed {
            if let Some(sh) = wb.sheet(k.0) {
                for (anchor, r) in &sh.spill_ranges {
                    if *anchor != k.1 && r.contains(k.1) {
                        seeds.push((k.0, *anchor));
                    }
                }
                for (c, cell) in sh.cells.iter_range(RangeRef::new(CellRef::new(k.1.row.saturating_sub(64), k.1.col.saturating_sub(64)), k.1)) {
                    if c != k.1 && cell.value == Value::Error(CellError::Spill) {
                        seeds.push((k.0, c));
                    }
                }
                if !sh.cells.has(k.1) {
                    match emptied.iter_mut().find(|x| x.0 == k.0) {
                        Some(x) => x.1 = CellRef::new(x.1.row.max(k.1.row), x.1.col.max(k.1.col)),
                        None => emptied.push(k),
                    }
                }
            }
        }
        // An emptied cell can unblock an anchor any distance above or to the left of it.
        for (si, end) in emptied {
            for &(s, c) in self.graph.nodes.keys() {
                if s == si && c.row <= end.row && c.col <= end.col && wb.sheet(s).is_some_and(|sh| sh.value(c) == Value::Error(CellError::Spill)) {
                    seeds.push((s, c));
                }
            }
        }
        // Spill areas of changed anchors also change.
        for &k in changed {
            if let Some(r) = wb.sheet(k.0).and_then(|s| s.spill_ranges.get(&k.1)) {
                seeds.extend(r.iter().map(|c| (k.0, c)));
            }
        }
        for (si, r) in &cleared {
            seeds.extend(r.iter().map(|c| (*si, c)));
        }
        let t0 = prof_now();
        let dirty = self.dirty_closure(wb, &seeds);
        let t1 = prof_now();
        let n = dirty.len();
        self.run(wb, dirty, false);
        if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
            eprintln!("recalc: {} seeds, closure {} cells {:.1} ms, run {:.1} ms", seeds.len(), n, t1 - t0, prof_now() - t1);
        }
    }

    fn dirty_closure(&self, _wb: &Workbook, seeds: &[Key]) -> Vec<Key> {
        let mut seen: HashSet<Key> = HashSet::default();
        let mut queue: VecDeque<Key> = VecDeque::new();
        for k in seeds {
            if self.graph.nodes.contains_key(k) && seen.insert(*k) {
                queue.push_back(*k);
            }
            let mut deps = Vec::new();
            self.graph.dependents(k.0, k.1, &mut deps);
            for d in deps {
                if seen.insert(d) {
                    queue.push_back(d);
                }
            }
        }
        for k in &self.graph.dynamic {
            if seen.insert(*k) {
                queue.push_back(*k);
            }
        }
        let mut deps = Vec::new();
        while let Some(k) = queue.pop_front() {
            deps.clear();
            self.graph.dependents(k.0, k.1, &mut deps);
            for d in deps.drain(..) {
                if seen.insert(d) {
                    queue.push_back(d);
                }
            }
        }
        seen.into_iter().collect()
    }

    /// Evaluates the dirty set and writes results (and spills) back.
    fn run(&mut self, wb: &mut Workbook, mut dirty: Vec<Key>, full: bool) {
        let mut dynamic_again = !self.graph.dynamic.is_empty();
        for _round in 0..8 {
            if dirty.is_empty() {
                break;
            }
            // Row-major order is a good topological guess; on-demand evaluation fixes the rest.
            dirty.sort_by_key(|(s, c)| (*s, c.row, c.col));
            let now = self.now();
            // Independent formulas (no dirty precedent, not dynamic) are one wave. Writing that
            // wave in row order skips the expression map and the on-demand host.
            let flat = if wb.calc.iterative { None } else { eval_flat_wave(wb, &self.graph, &dirty, now, wb.calc.max_workers) };
            if let Some(wave) = flat.as_ref()
                && wave.spills.is_empty()
                && wave.pairs.len() == dirty.len()
                && !sheets_have_spills(wb)
            {
                let n = wave.pairs.len();
                let tw = prof_now();
                write_flat_values(wb, &wave.pairs);
                if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
                    eprintln!("  flat-write {:.1} ms ({} cells)", prof_now() - tw, n);
                }
                self.last_recalc_cells = n;
                self.circular.clear();
                break;
            }
            let mut exprs: HashMap<Key, std::sync::Arc<Expr>> = HashMap::with_capacity_and_hasher(dirty.len(), Default::default());
            for k in &dirty {
                if let Some(e) = self.graph.nodes.get(k).and_then(|n| n.expr.clone()) {
                    exprs.insert(*k, e);
                } else if let Some(e) = wb.sheet(k.0).and_then(|s| s.cell(k.1)).and_then(|c| c.formula.as_ref()).and_then(|f| f.expr_arc()) {
                    exprs.insert(*k, e);
                }
            }
            let mut rng = self.rng;
            let tp = prof_now();
            let (results, spills, cycles) = {
                let mut host = PassHost::new(wb, &exprs, &mut rng, now);
                host.iterate = wb.calc.iterative;
                if let Some(wave) = flat {
                    for (k, v) in wave.pairs {
                        host.pending.remove(&k);
                        host.results.insert(k, v);
                    }
                    host.spills = wave.spills;
                    for k in &dirty {
                        if !host.results.contains_key(k) {
                            host.compute(*k);
                        }
                    }
                } else {
                    let scheduled = if host.iterate { None } else { eval_independent(wb, &self.graph, &exprs, &dirty, now, wb.calc.max_workers) };
                    if let Some(batch) = scheduled {
                        for (k, v) in &batch.values {
                            host.results.insert(*k, v.clone());
                            host.pending.remove(k);
                        }
                        host.spills = batch.spills;
                        for k in &dirty {
                            if !batch.done.contains(k) {
                                host.compute(*k);
                            }
                        }
                    } else {
                        for k in &dirty {
                            host.compute(*k);
                        }
                    }
                }
                if host.iterate && !host.members.is_empty() {
                    let passes = host.iterate_loops(&dirty, wb.calc.max_iterations, wb.calc.max_change);
                    if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
                        eprintln!("  iterative calculation: {} passes over {} formulas", passes, host.tainted.len());
                    }
                }
                (host.results, host.spills, host.cycles)
            };
            self.rng = rng;
            if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
                eprintln!("  eval {} formulas {:.1} ms", results.len(), prof_now() - tp);
            }
            let tw = prof_now();
            self.last_recalc_cells = results.len();
            self.circular = cycles;
            // Write back. Row order keeps the copy-on-write bands in cache.
            let mut spill_changes: Vec<Key> = Vec::new();
            let mut freed: Vec<(usize, RangeRef)> = Vec::new();
            let mut ordered: Vec<(Key, Value)> = results.into_iter().collect();
            ordered.sort_unstable_by_key(|((s, c), _)| (*s, c.row, c.col));
            write_flat_values(wb, &ordered);
            if !spills.is_empty() || sheets_have_spills(wb) {
                for k in &dirty {
                    let new = spills.get(k);
                    let Some(sheet) = wb.sheet_mut(k.0) else { continue };
                    let old = sheet.spill_ranges.get(&k.1).copied();
                    if old.is_none() && new.is_none() {
                        continue;
                    }
                    if let Some(r) = old {
                        for c in r.iter() {
                            sheet.spill.remove(&c);
                        }
                        sheet.spill_ranges.remove(&k.1);
                    }
                    let mut changed_range = old;
                    if let Some(arr) = new {
                        let end = CellRef::new(k.1.row.saturating_add(arr.rows as u32 - 1), k.1.col.saturating_add(arr.cols as u32 - 1));
                        let range = RangeRef::new(k.1, end);
                        let blocked = end.row >= gridcraft_core::MAX_ROWS
                            || end.col >= gridcraft_core::MAX_COLS
                            || range.iter().any(|c| c != k.1 && (sheet.cells.has(c) || sheet.spill.contains_key(&c)))
                            || sheet.merges.iter().any(|m| m.intersects(&range))
                            || sheet.tables.iter().any(|t| t.range.contains(k.1));
                        if blocked {
                            if let Some(cell) = sheet.cells.get_mut(k.1) {
                                cell.value = Value::Error(CellError::Spill);
                            }
                        } else {
                            for (i, c) in range.iter().enumerate() {
                                if c != k.1 {
                                    sheet.spill.insert(c, arr.data.get(i).cloned().unwrap_or_default());
                                }
                            }
                            sheet.spill_ranges.insert(k.1, range);
                            changed_range = Some(changed_range.map_or(range, |o| o.union(&range)));
                        }
                    }
                    if let Some(r) = changed_range {
                        let mut deps = Vec::new();
                        self.graph.dependents_of_range(k.0, r, &mut deps);
                        spill_changes.extend(deps.into_iter().filter(|d| d != k));
                    }
                    if let Some(o) = old
                        && !sheet.spill_ranges.get(&k.1).is_some_and(|n| n.contains(o.start) && n.contains(o.end))
                    {
                        freed.push((k.0, o));
                    }
                }
            }
            // Formulas with references only known while evaluating (INDIRECT, OFFSET) may have read
            // a spill area before it was laid out: they are evaluated once more.
            if dynamic_again && !spills.is_empty() {
                dynamic_again = false;
                spill_changes.extend(self.graph.dynamic.iter().copied());
            }
            // An area a formula no longer spills over may unblock another formula's array.
            for (si, o) in freed {
                for &(s, c) in self.graph.nodes.keys() {
                    if s == si
                        && c.row <= o.end.row
                        && c.col <= o.end.col
                        && wb.sheet(s).is_some_and(|sh| sh.value(c) == Value::Error(CellError::Spill))
                    {
                        spill_changes.push((s, c));
                    }
                }
            }
            if full {
                // Everything was already evaluated once; spill dependents need another pass.
            }
            if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
                eprintln!("  write-back {:.1} ms", prof_now() - tw);
            }
            spill_changes.sort_by_key(|(s, c)| (*s, c.row, c.col));
            spill_changes.dedup();
            if spill_changes.is_empty() {
                break;
            }
            dirty = self.dirty_closure(wb, &spill_changes);
        }
    }
}

/// Current date-time as a serial (1900 system), from the system clock (browser clock on wasm).
/// UTC; the local offset isn't available without platform calls. Falls back to a fixed date only
/// when no clock is available.
pub fn now_serial() -> f64 {
    gridcraft_core::date::unix_now_ms().map_or(46302.5, gridcraft_core::date::serial_from_unix_ms)
}

/// Below this many dirty formulas the scheduling cost is not worth a thread pool.
#[cfg(not(target_arch = "wasm32"))]
const PARALLEL_MIN: usize = 64;
/// A formula with more dirty precedents than this stays on the calling thread (the edge list
/// would dwarf the formula).
#[cfg(not(target_arch = "wasm32"))]
const MAX_LIGHT_PREDS: usize = 4_096;

struct ParallelBatch {
    values: HashMap<Key, Value>,
    spills: HashMap<Key, std::sync::Arc<Array>>,
    done: HashSet<Key>,
}

#[cfg(not(target_arch = "wasm32"))]
struct DirtyIndex {
    set: HashSet<Key>,
    /// (sheet, col) → sorted rows of dirty formula cells.
    cols: HashMap<(usize, u32), Vec<u32>>,
}

/// Reads cells from the workbook and from waves that have already finished. It does not evaluate
/// other dirty formulas: the scheduler only hands it formulas whose dirty precedents are done.
#[cfg(not(target_arch = "wasm32"))]
struct FrozenHost<'a> {
    wb: &'a Workbook,
    prior: &'a HashMap<Key, Value>,
    spills: &'a HashMap<Key, std::sync::Arc<Array>>,
    now: f64,
    at: Key,
    used: HashMap<usize, Option<RangeRef>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Host for FrozenHost<'_> {
    fn workbook(&self) -> &Workbook {
        self.wb
    }
    fn cell_value(&mut self, sheet: usize, c: CellRef) -> Value {
        let k = (sheet, c);
        if let Some(v) = self.prior.get(&k) {
            return v.clone();
        }
        for (anchor, arr) in self.spills {
            if anchor.0 == sheet && c.row >= anchor.1.row && c.col >= anchor.1.col {
                let (dr, dc) = ((c.row - anchor.1.row) as usize, (c.col - anchor.1.col) as usize);
                if dr < arr.rows && dc < arr.cols {
                    return arr.get(dr, dc).cloned().unwrap_or_default();
                }
            }
        }
        self.wb.sheet(sheet).map(|s| s.value(c)).unwrap_or_default()
    }
    fn range_values(&mut self, sheet: usize, range: RangeRef) -> Vec<Value> {
        let (h, w) = (range.height() as usize, range.width() as usize);
        let Some(len) = h.checked_mul(w) else { return Vec::new() };
        if len as u64 > crate::eval::MAX_CELLS {
            return Vec::new();
        }
        let mut out = vec![Value::Empty; len];
        let Some(sh) = self.wb.sheet(sheet) else { return out };
        for (c, cell) in sh.cells.iter_range(range) {
            let i = (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize;
            let value = if cell.formula.is_some() { self.cell_value(sheet, c) } else { cell.value.clone() };
            if let Some(slot) = out.get_mut(i) {
                *slot = value;
            }
        }
        for (c, v) in sh.spill.range(range.start..=range.end) {
            if range.contains(*c) {
                let i = (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize;
                if let Some(slot) = out.get_mut(i)
                    && slot.is_empty()
                {
                    *slot = v.clone();
                }
            }
        }
        for (anchor, arr) in self.spills {
            if anchor.0 != sheet {
                continue;
            }
            for r in 0..arr.rows {
                for cc in 0..arr.cols {
                    let c = CellRef::new(anchor.1.row.saturating_add(r as u32), anchor.1.col.saturating_add(cc as u32));
                    if c != anchor.1 && range.contains(c) {
                        let i = (c.row - range.start.row) as usize * w + (c.col - range.start.col) as usize;
                        if let Some(slot) = out.get_mut(i) {
                            *slot = arr.get(r, cc).cloned().unwrap_or_default();
                        }
                    }
                }
            }
        }
        out
    }
    fn visit_range(&mut self, sheet: usize, range: RangeRef, f: &mut dyn FnMut(&Value) -> bool) -> bool {
        let wb = self.wb;
        let Some(sh) = wb.sheet(sheet) else { return false };
        // A short single row with no spills is the shape of `SUM(E2:J2)`. Copy the stored
        // values out, then fold them, so the walk does not build a spill list. A formula in the
        // row keeps the general path (it may need a value from an earlier wave).
        if self.spills.is_empty() && sh.spill.is_empty() && range.height() == 1 && range.width() <= 32 {
            let mut vals: [Value; 32] = std::array::from_fn(|_| Value::Empty);
            let mut n = 0usize;
            let mut plain = true;
            for (_, cell) in sh.cells.row(range.start.row, range.start.col, range.end.col) {
                if cell.formula.is_some() || n >= vals.len() {
                    plain = false;
                    break;
                }
                vals[n] = cell.value.clone();
                n += 1;
            }
            if plain {
                for v in vals.iter().take(n) {
                    if !f(v) {
                        return true;
                    }
                }
                return true;
            }
        }
        // A value from an earlier wave, including a spill that meets this block, stays on the
        // sparse walk. Declining would make `SUM` build the trimmed rectangle and return `#NUM!`
        // once that rectangle passes `MAX_CELLS`. Decline only when a spill is too large to list.
        let Some(extras) = spill_cells_in_range(sheet, range, &sh.spill, self.spills) else {
            return false;
        };
        let mut i = 0;
        for (c, cell) in sh.cells.iter_range(range) {
            while i < extras.len() && extras[i].0 < c {
                if !f(&extras[i].1) {
                    return true;
                }
                i += 1;
            }
            let go = if cell.formula.is_some() {
                if i < extras.len() && extras[i].0 == c {
                    i += 1;
                }
                // `cell_value` prefers the value this pass already stored for the cell. The sheet
                // still holds the value from before the pass.
                let v = self.cell_value(sheet, c);
                f(&v)
            } else if i < extras.len() && extras[i].0 == c {
                let v = extras[i].1.clone();
                i += 1;
                f(&v)
            } else {
                f(&cell.value)
            };
            if !go {
                return true;
            }
        }
        while i < extras.len() {
            if !f(&extras[i].1) {
                return true;
            }
            i += 1;
        }
        true
    }
    fn used_range(&mut self, sheet: usize) -> Option<RangeRef> {
        let wb = self.wb;
        *self.used.entry(sheet).or_insert_with(|| wb.sheet(sheet).and_then(|s| s.used_range()))
    }
    fn now_serial(&self) -> f64 {
        self.now
    }
    fn random(&mut self) -> f64 {
        // Dynamic formulas are not scheduled here. A deterministic stream keyed by the cell keeps
        // a stray `RAND` from racing on the shared generator.
        let mut x = 0x9E37_79B9_7F4A_7C15u64
            ^ ((self.at.0 as u64) << 32)
            ^ (self.at.1.row as u64).wrapping_mul(0x1000_0001)
            ^ (self.at.1.col as u64).wrapping_mul(0x9E37);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn dirty_index(keys: &[Key]) -> DirtyIndex {
    let mut set = HashSet::with_capacity_and_hasher(keys.len(), Default::default());
    let mut cols: HashMap<(usize, u32), Vec<u32>> = HashMap::default();
    for k in keys {
        if set.insert(*k) {
            cols.entry((k.0, k.1.col)).or_default().push(k.1.row);
        }
    }
    for rows in cols.values_mut() {
        rows.sort_unstable();
        rows.dedup();
    }
    DirtyIndex { set, cols }
}

/// Dirty formula cells inside `area`, including the formula itself when it reads its own cell.
/// `Err` when there are too many to schedule as light edges.
#[cfg(not(target_arch = "wasm32"))]
fn precedent_keys(index: &DirtyIndex, areas: &[Area]) -> Result<Vec<Key>, ()> {
    let mut out = Vec::new();
    let mut seen: HashSet<Key> = HashSet::default();
    for area in areas {
        if area.range.is_single() {
            let p = (area.sheet, area.range.start);
            if index.set.contains(&p) && seen.insert(p) {
                out.push(p);
            }
        } else {
            let (c0, c1) = (area.range.start.col, area.range.end.col);
            let (r0, r1) = (area.range.start.row, area.range.end.row);
            for ((sheet, col), rows) in &index.cols {
                if *sheet != area.sheet || *col < c0 || *col > c1 {
                    continue;
                }
                let i = rows.partition_point(|r| *r < r0);
                for row in rows.get(i..).unwrap_or(&[]).iter().take_while(|r| **r <= r1) {
                    let p = (*sheet, CellRef::new(*row, *col));
                    if seen.insert(p) {
                        out.push(p);
                        if out.len() > MAX_LIGHT_PREDS {
                            return Err(());
                        }
                    }
                }
            }
        }
        if out.len() > MAX_LIGHT_PREDS {
            return Err(());
        }
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn eval_frozen(
    wb: &Workbook,
    exprs: &HashMap<Key, std::sync::Arc<Expr>>,
    prior: &HashMap<Key, Value>,
    spills: &HashMap<Key, std::sync::Arc<Array>>,
    now: f64,
    k: Key,
) -> (Value, Option<std::sync::Arc<Array>>) {
    let Some(expr) = exprs.get(&k) else {
        return (wb.sheet(k.0).map(|s| s.value(k.1)).unwrap_or_default(), None);
    };
    let mut host = FrozenHost { wb, prior, spills, now, at: k, used: HashMap::default() };
    let v = {
        let mut ev = Evaluator::new(&mut host, k.0, k.1);
        ev.value(expr)
    };
    finish_formula(wb, exprs, k, expr, v, stored_legacy(wb, k), true)
}

/// Threads for one wave. `cap` is the workbook setting: `None` follows the machine. Tests with no
/// cap use two once there is enough work, so the parallel path is exercised on a one-core runner.
/// An explicit cap of 1 stays on the caller. Production clamps to 1..=8.
#[cfg(not(target_arch = "wasm32"))]
fn worker_count(jobs: usize, cap: Option<u8>) -> usize {
    let machine = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1).clamp(1, 8);
    let n = match cap {
        Some(c) => usize::from(c).clamp(1, 8),
        None => machine,
    };
    let n = n.min(jobs.max(1));
    if n >= 2 {
        n
    } else if cap.is_none() && cfg!(test) && jobs >= PARALLEL_MIN {
        2
    } else {
        1
    }
}

/// `(watch token, thread id)` for workers that ran a parallel wave. Test-only, so a unit test can
/// see that the shipped scheduler actually left the caller. Tokens keep parallel tests apart.
#[cfg(test)]
static WAVE_THREADS: std::sync::Mutex<Vec<(u64, std::thread::ThreadId)>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
std::thread_local! {
    static WAVE_WATCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
static NEXT_WAVE_WATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Records worker threads spawned while this value is alive on the calling thread.
#[cfg(test)]
struct WaveWatch(u64);

#[cfg(test)]
fn watch_wave_threads() -> WaveWatch {
    let token = NEXT_WAVE_WATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    WAVE_WATCH.with(|slot| slot.set(token));
    WaveWatch(token)
}

#[cfg(test)]
impl Drop for WaveWatch {
    fn drop(&mut self) {
        WAVE_WATCH.with(|slot| {
            if slot.get() == self.0 {
                slot.set(0);
            }
        });
    }
}

#[cfg(test)]
fn current_wave_watch() -> u64 {
    WAVE_WATCH.with(|slot| slot.get())
}

/// When set, each worker panics after recording its id. The scheduler must catch that and let
/// the caller evaluate the same formulas.
#[cfg(test)]
static WAVE_PANIC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
fn note_wave_thread(token: u64) {
    if token == 0 {
        return;
    }
    let id = std::thread::current().id();
    if let Ok(mut guard) = WAVE_THREADS.lock()
        && !guard.iter().any(|(t, seen)| *t == token && *seen == id)
    {
        guard.push((token, id));
    }
}

#[cfg(test)]
fn maybe_panic_worker() {
    if WAVE_PANIC.load(std::sync::atomic::Ordering::Relaxed) {
        #[allow(clippy::panic)]
        {
            panic!("injected worker failure");
        }
    }
}

#[cfg(test)]
fn take_wave_threads(token: u64) -> Vec<std::thread::ThreadId> {
    WAVE_THREADS
        .lock()
        .map(|mut guard| {
            let mut mine = Vec::new();
            guard.retain(|(t, id)| {
                if *t == token {
                    mine.push(*id);
                    false
                } else {
                    true
                }
            });
            mine
        })
        .unwrap_or_default()
}

struct FlatWave {
    pairs: Vec<(Key, Value)>,
    spills: HashMap<Key, std::sync::Arc<Array>>,
}

fn sheets_have_spills(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|s| !s.spill.is_empty() || !s.spill_ranges.is_empty())
}

/// Writes formula results in sheet order. `flat` is grouped by sheet (the dirty set is sorted
/// that way). Each sheet's cells are applied in row order.
fn write_flat_values(wb: &mut Workbook, flat: &[(Key, Value)]) {
    let mut i = 0;
    while i < flat.len() {
        let sheet = flat[i].0.0;
        let end = flat[i..].iter().position(|(k, _)| k.0 != sheet).map_or(flat.len(), |p| i + p);
        if let Some(sh) = wb.sheet_mut(sheet) {
            let mut updates = Vec::with_capacity(end - i);
            for ((_, c), v) in &flat[i..end] {
                updates.push((*c, v.clone()));
            }
            sh.cells.assign_existing(&mut updates);
        }
        i = end;
    }
}

/// One wave of formulas that do not read other dirty formulas and are not dynamic. `None` when
/// any formula needs the dependency scheduler, the set is small, or a worker panicked.
fn eval_flat_wave(wb: &Workbook, graph: &Graph, dirty: &[Key], now: f64, workers: Option<u8>) -> Option<FlatWave> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (wb, graph, dirty, now, workers);
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        eval_flat_wave_native(wb, graph, dirty, now, workers)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn has_dirty_precedent(index: &DirtyIndex, areas: &[Area]) -> bool {
    for area in areas {
        if area.range.is_single() {
            if index.set.contains(&(area.sheet, area.range.start)) {
                return true;
            }
        } else {
            let (c0, c1) = (area.range.start.col, area.range.end.col);
            let (r0, r1) = (area.range.start.row, area.range.end.row);
            for ((sheet, col), rows) in &index.cols {
                if *sheet != area.sheet || *col < c0 || *col > c1 {
                    continue;
                }
                let i = rows.partition_point(|r| *r < r0);
                if rows.get(i..).is_some_and(|tail| tail.iter().any(|r| *r <= r1)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Every dirty formula sits in one column, and none of them reads that column. The column
/// index is unnecessary: a precedent would have to name this column to see a dirty cell.
#[cfg(not(target_arch = "wasm32"))]
fn column_wave_is_clear(graph: &Graph, dirty: &[Key]) -> bool {
    let Some(first) = dirty.first() else { return false };
    let (sheet, col) = (first.0, first.1.col);
    for k in dirty {
        if k.0 != sheet || k.1.col != col {
            return false;
        }
        let Some(node) = graph.nodes.get(k) else { return false };
        if node.dynamic || node.expr.is_none() {
            return false;
        }
        if node.areas.iter().any(|a| a.sheet == sheet && a.range.start.col <= col && col <= a.range.end.col) {
            return false;
        }
    }
    true
}

#[cfg(not(target_arch = "wasm32"))]
fn eval_flat_wave_native(wb: &Workbook, graph: &Graph, dirty: &[Key], now: f64, workers: Option<u8>) -> Option<FlatWave> {
    if dirty.len() < PARALLEL_MIN || worker_count(dirty.len(), workers) < 2 {
        return None;
    }
    let t_ready = prof_now();
    // One column of formulas that do not read themselves needs no dirty index and no
    // expression clone. Anything else still goes through the index, which can say no.
    if !column_wave_is_clear(graph, dirty) {
        let index = dirty_index(dirty);
        for k in dirty {
            let node = graph.nodes.get(k)?;
            if node.dynamic || node.expr.is_none() || has_dirty_precedent(&index, &node.areas) {
                return None;
            }
        }
    }
    let n_workers = worker_count(dirty.len(), workers);
    if n_workers < 2 {
        return None;
    }
    let t_workers = prof_now();
    let chunk = dirty.len().div_ceil(n_workers);
    let prior: HashMap<Key, Value> = HashMap::default();
    let spills_in: HashMap<Key, std::sync::Arc<Array>> = HashMap::default();
    let mut parts: Vec<Vec<(Key, Value, Option<std::sync::Arc<Array>>)>> = Vec::new();
    let mut failed = false;
    #[cfg(test)]
    let watch = current_wave_watch();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for group in dirty.chunks(chunk) {
            let prior = &prior;
            let spills_in = &spills_in;
            handles.push(scope.spawn(move || {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    #[cfg(test)]
                    {
                        note_wave_thread(watch);
                        maybe_panic_worker();
                    }
                    let mut scratch: Option<HashMap<Key, std::sync::Arc<Expr>>> = None;
                    let mut local = Vec::with_capacity(group.len());
                    for k in group {
                        let Some(node) = graph.nodes.get(k) else { continue };
                        let Some(expr) = node.expr.as_deref() else { continue };
                        let legacy = node.legacy;
                        // Numbers, references and SUM/COUNT/AVERAGE of stored cells have no
                        // side effects. They match the evaluator, and they stay off the
                        // virtual host so a column of the same shape does not allocate per row.
                        let fast = eval_flat_scalar(wb, k.0, expr);
                        if let Some(v) = fast.as_ref()
                            && legacy.is_none()
                            && !matches!(v, Value::Array(_) | Value::Empty)
                        {
                            local.push((*k, v.clone(), None));
                            continue;
                        }
                        let v = match fast {
                            Some(v) => v,
                            None => {
                                let mut host = FrozenHost { wb, prior, spills: spills_in, now, at: *k, used: HashMap::default() };
                                let mut ev = Evaluator::new(&mut host, k.0, k.1);
                                ev.value(expr)
                            }
                        };
                        let exprs = scratch.get_or_insert_with(HashMap::default);
                        let (v, sp) = finish_formula(wb, exprs, *k, expr, v, legacy, false);
                        local.push((*k, v, sp));
                    }
                    local
                }))
            }));
        }
        for handle in handles {
            match handle.join() {
                Ok(Ok(local)) => parts.push(local),
                Ok(Err(_)) | Err(_) => failed = true,
            }
        }
    });
    if failed {
        return None;
    }
    let t_merge = prof_now();
    let mut pairs = Vec::with_capacity(dirty.len());
    let mut spills = HashMap::default();
    for local in parts {
        for (k, v, sp) in local {
            if let Some(a) = sp {
                spills.insert(k, a);
            }
            pairs.push((k, v));
        }
    }
    if std::env::var_os("GRIDCRAFT_PROFILE").is_some() {
        let t_done = prof_now();
        eprintln!("  flat classify {:.1} ms, workers {:.1} ms, merge {:.1} ms", t_workers - t_ready, t_merge - t_workers, t_done - t_merge);
    }
    Some(FlatWave { pairs, spills })
}

/// Scalar formulas whose result is a function of stored cells: literals, arithmetic, and
/// `SUM` / `COUNT` / `AVERAGE` of references. `None` means the ordinary evaluator must run
/// (names, dynamic refs, spills, arrays). A range is folded from the cells that are stored;
/// blanks are not visited, which is what those three functions do. A sheet that already has a
/// spill falls back, so a spill inside the range is not missed.
#[cfg(not(target_arch = "wasm32"))]
fn eval_flat_scalar(wb: &Workbook, sheet: usize, expr: &Expr) -> Option<Value> {
    eval_flat_scalar_at(wb, sheet, expr, 0)
}

#[cfg(not(target_arch = "wasm32"))]
fn eval_flat_scalar_at(wb: &Workbook, sheet: usize, expr: &Expr, depth: u32) -> Option<Value> {
    if depth > 64 {
        return None;
    }
    let next = depth + 1;
    match expr {
        Expr::Paren(e) => eval_flat_scalar_at(wb, sheet, e, next),
        Expr::Number(n) => Some(Value::number(*n)),
        Expr::Bool(b) => Some(Value::Bool(*b)),
        Expr::Text(t) => Some(Value::Text(std::sync::Arc::clone(t))),
        Expr::Error(e) => Some(Value::Error(*e)),
        Expr::Unary(op, e) => {
            let Value::Number(n) = eval_flat_scalar_at(wb, sheet, e, next)? else {
                return None;
            };
            match op {
                UnOp::Neg => Some(Value::number(-n)),
                UnOp::Plus => Some(Value::number(n)),
                UnOp::Percent => Some(Value::number(n / 100.0)),
                UnOp::At | UnOp::Spill | UnOp::Trim(_) => None,
            }
        }
        Expr::Ref(r) => {
            if !matches!(r.sheet, SheetSel::Current) {
                return None;
            }
            let range = r.range();
            if !range.is_single() {
                return None;
            }
            Some(wb.sheet(sheet)?.value(range.start))
        }
        Expr::Binary(op, a, b) if !matches!(op, BinOp::Range | BinOp::Union | BinOp::Intersect) => {
            let x = eval_flat_scalar_at(wb, sheet, a, next)?;
            let y = eval_flat_scalar_at(wb, sheet, b, next)?;
            Some(binary_values(*op, &x, &y, wb.date_system))
        }
        Expr::Call(name, args) if matches!(name.as_str(), "SUM" | "COUNT" | "AVERAGE") => eval_flat_agg(wb, sheet, name, args, next),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn eval_flat_agg(wb: &Workbook, sheet: usize, name: &str, args: &[Expr], depth: u32) -> Option<Value> {
    if depth > 64 {
        return None;
    }
    if args.is_empty() {
        return Some(Value::Error(CellError::Value));
    }
    let mut sum = 0.0;
    let mut count = 0u64;
    let mut error: Option<CellError> = None;
    for a in args {
        match a {
            Expr::Paren(e) => match eval_flat_scalar_at(wb, sheet, e, depth + 1)? {
                Value::Number(n) => {
                    sum += n;
                    count = count.saturating_add(1);
                }
                Value::Error(e) => {
                    if error.is_none() {
                        error = Some(e);
                    }
                }
                Value::Bool(_) | Value::Text(_) | Value::Empty | Value::Array(_) => return None,
            },
            Expr::Number(n) => match Value::number(*n) {
                Value::Number(n) => {
                    sum += n;
                    count = count.saturating_add(1);
                }
                Value::Error(e) => {
                    if error.is_none() {
                        error = Some(e);
                    }
                }
                _ => return None,
            },
            Expr::Ref(r) if matches!(r.sheet, SheetSel::Current) => {
                if !fold_stored_ref(wb, sheet, r.range(), &mut sum, &mut count, &mut error) {
                    return None;
                }
            }
            _ => return None,
        }
    }
    let value = if name != "COUNT"
        && let Some(e) = error
    {
        Value::Error(e)
    } else if name == "COUNT" {
        Value::number(count as f64)
    } else if name == "AVERAGE" {
        if count == 0 { Value::Error(CellError::Div0) } else { Value::number(sum / count as f64) }
    } else {
        Value::number(sum)
    };
    Some(value)
}

/// Adds one reference's stored cells. `false` when a spill (or an array value) means the
/// ordinary aggregate must run instead.
#[cfg(not(target_arch = "wasm32"))]
fn fold_stored_ref(wb: &Workbook, sheet: usize, range: RangeRef, sum: &mut f64, count: &mut u64, error: &mut Option<CellError>) -> bool {
    let Some(sh) = wb.sheet(sheet) else { return false };
    if !sh.spill.is_empty() || !sh.spill_ranges.is_empty() {
        return false;
    }
    let mut accept = |cell: &gridcraft_model::Cell| -> bool {
        match &cell.value {
            Value::Number(n) => {
                *sum += *n;
                *count = count.saturating_add(1);
                true
            }
            Value::Error(e) => {
                if error.is_none() {
                    *error = Some(*e);
                }
                true
            }
            Value::Array(_) => false,
            _ => true,
        }
    };
    if range.height() == 1 {
        for (_, cell) in sh.cells.row(range.start.row, range.start.col, range.end.col) {
            if !accept(cell) {
                return false;
            }
        }
    } else {
        for (_, cell) in sh.cells.iter_range(range) {
            if !accept(cell) {
                return false;
            }
        }
    }
    true
}

/// Evaluates waves of formulas that do not read other dirty formulas. `None` when the set is too
/// small, single-threaded, or a worker panicked (the caller then evaluates everything itself).
fn eval_independent(
    wb: &Workbook,
    graph: &Graph,
    exprs: &HashMap<Key, std::sync::Arc<Expr>>,
    dirty: &[Key],
    now: f64,
    workers: Option<u8>,
) -> Option<ParallelBatch> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (wb, graph, exprs, dirty, now, workers);
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        eval_independent_native(wb, graph, exprs, dirty, now, workers)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn eval_independent_native(
    wb: &Workbook,
    graph: &Graph,
    exprs: &HashMap<Key, std::sync::Arc<Expr>>,
    dirty: &[Key],
    now: f64,
    workers: Option<u8>,
) -> Option<ParallelBatch> {
    if dirty.len() < PARALLEL_MIN || worker_count(dirty.len(), workers) < 2 {
        return None;
    }
    let index = dirty_index(dirty);
    let mut left: HashMap<Key, u32> = HashMap::default();
    let mut succs: HashMap<Key, Vec<Key>> = HashMap::default();
    let mut heavy: HashSet<Key> = HashSet::default();
    for k in dirty {
        if !exprs.contains_key(k) {
            continue;
        }
        let Some(node) = graph.nodes.get(k) else {
            heavy.insert(*k);
            continue;
        };
        if node.dynamic {
            heavy.insert(*k);
            continue;
        }
        match precedent_keys(&index, &node.areas) {
            Err(()) => {
                heavy.insert(*k);
            }
            Ok(preds) => {
                left.insert(*k, preds.len() as u32);
                for p in preds {
                    if p != *k {
                        succs.entry(p).or_default().push(*k);
                    }
                }
            }
        }
    }
    let mut values: HashMap<Key, Value> = HashMap::default();
    let mut spills: HashMap<Key, std::sync::Arc<Array>> = HashMap::default();
    let mut done: HashSet<Key> = HashSet::default();
    loop {
        let ready: Vec<Key> = dirty.iter().copied().filter(|k| !done.contains(k) && !heavy.contains(k) && left.get(k).copied() == Some(0)).collect();
        if ready.len() < PARALLEL_MIN || worker_count(ready.len(), workers) < 2 {
            break;
        }
        let n_workers = worker_count(ready.len(), workers);
        let chunk = ready.len().div_ceil(n_workers);
        let mut parts: Vec<Vec<(Key, Value, Option<std::sync::Arc<Array>>)>> = Vec::new();
        let mut failed = false;
        #[cfg(test)]
        let watch = current_wave_watch();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for group in ready.chunks(chunk) {
                let values = &values;
                let spills = &spills;
                handles.push(scope.spawn(move || {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        #[cfg(test)]
                        {
                            note_wave_thread(watch);
                            maybe_panic_worker();
                        }
                        let mut local = Vec::with_capacity(group.len());
                        for k in group {
                            let (v, sp) = eval_frozen(wb, exprs, values, spills, now, *k);
                            local.push((*k, v, sp));
                        }
                        local
                    }))
                }));
            }
            for handle in handles {
                match handle.join() {
                    Ok(Ok(local)) => parts.push(local),
                    Ok(Err(_)) | Err(_) => failed = true,
                }
            }
        });
        if failed {
            return None;
        }
        for local in parts {
            for (k, v, sp) in local {
                if let Some(children) = succs.get(&k) {
                    for c in children {
                        if let Some(n) = left.get_mut(c) {
                            *n = n.saturating_sub(1);
                        }
                    }
                }
                if let Some(a) = sp {
                    spills.insert(k, a);
                }
                values.insert(k, v);
                done.insert(k);
            }
        }
    }
    if done.is_empty() { None } else { Some(ParallelBatch { values, spills, done }) }
}

/// Evaluates a formula in the context of a cell without storing it (conditional formats, data
/// validation, Evaluate Formula, Name Manager previews, the CLI `eval`).
pub fn evaluate(wb: &Workbook, sheet: usize, at: CellRef, formula: &str) -> Value {
    let body = formula.strip_prefix('=').unwrap_or(formula);
    let Ok(expr) = gridcraft_formula::parse(body) else { return Value::Error(CellError::Name) };
    evaluate_expr(wb, sheet, at, &expr)
}

pub fn evaluate_expr(wb: &Workbook, sheet: usize, at: CellRef, expr: &Expr) -> Value {
    let exprs = HashMap::default();
    let mut rng = 0x1234_5678_9ABC_DEF0u64;
    let mut host = PassHost::new(wb, &exprs, &mut rng, now_serial());
    let mut ev = Evaluator::new(&mut host, sheet, at);
    ev.value(expr)
}

#[allow(clippy::disallowed_methods)] // the clock is read only off wasm
fn prof_now() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

#[cfg(test)]
struct ResetWavePanic;

#[cfg(test)]
impl Drop for ResetWavePanic {
    fn drop(&mut self) {
        WAVE_PANIC.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::{Duration, Instant};

    use gridcraft_core::{CellError, CellRef, RangeRef, Value};
    use gridcraft_model::{Cell, Formula, Workbook};

    use super::{Calc, HashMap, Host, Intervals, Key, PassHost, evaluate};

    /// xorshift64 for repeatable pseudo-random cases.
    fn rng(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    fn found(iv: &Intervals<u32>, a: u32, b: u32) -> Vec<u32> {
        let mut v = Vec::new();
        iv.query(a, b, &mut |t| v.push(*t));
        v.sort_unstable();
        v
    }

    #[test]
    fn intervals_match_a_linear_scan() {
        let mut iv: Intervals<u32> = Intervals::default();
        let mut model: Vec<(u32, u32, u32)> = Vec::new();
        let mut s = 0x9E37_79B9_7F4A_7C15;
        for step in 0..20_000u32 {
            let roll = rng(&mut s) % 10;
            if roll < 6 || model.is_empty() {
                let r0 = (rng(&mut s) % 500) as u32;
                let r1 = r0 + (rng(&mut s) % 40) as u32;
                if !model.contains(&(r0, r1, step)) {
                    iv.insert(r0, r1, step);
                    model.push((r0, r1, step));
                }
            } else {
                let i = (rng(&mut s) % model.len() as u64) as usize;
                let (r0, r1, t) = model.swap_remove(i);
                iv.remove(r0, r1, t);
                if roll == 9 {
                    // Removed and put back before the next merge.
                    iv.insert(r0, r1, t);
                    model.push((r0, r1, t));
                }
            }
            if rng(&mut s).is_multiple_of(50) {
                iv.settle();
            }
            if step.is_multiple_of(97) {
                let a = (rng(&mut s) % 560) as u32;
                let b = a + (rng(&mut s) % 30) as u32;
                let mut want: Vec<u32> = model.iter().filter(|(r0, r1, _)| *r0 <= b && *r1 >= a).map(|(_, _, t)| *t).collect();
                want.sort_unstable();
                assert_eq!(found(&iv, a, b), want, "rows {a}..={b} at step {step}");
            }
        }
        iv.settle();
        for row in 0..560 {
            let mut want: Vec<u32> = model.iter().filter(|(r0, r1, _)| *r0 <= row && row <= *r1).map(|(_, _, t)| *t).collect();
            want.sort_unstable();
            assert_eq!(found(&iv, row, row), want, "row {row}");
        }
        for (r0, r1, t) in std::mem::take(&mut model) {
            iv.remove(r0, r1, t);
        }
        iv.settle();
        assert!(iv.is_empty());
    }

    #[test]
    fn interval_queries_cost_log_n_plus_matches() {
        // 40k one-row ranges (like =SUM(E1:J1) down a column) and 1k ranges over everything:
        // a point query visits O(log n) tree nodes per match, not every range in the column.
        let n = 40_000u32;
        let mut iv: Intervals<u32> = Intervals::default();
        for r in 0..n {
            iv.insert(r, r, r);
        }
        for t in 0..1_000 {
            iv.insert(0, n, n + t);
        }
        iv.settle();
        let mut worst = 0;
        for row in (0..n).step_by(7) {
            let mut k = 0;
            let visited = iv.query(row, row, &mut |_| k += 1);
            assert_eq!(k, 1_001);
            worst = worst.max(visited);
        }
        // 1,001 matches in a tree of depth 16: well under 4 nodes per match plus the path.
        assert!(worst < 4 * 1_001 + 64, "visited {worst} nodes");
        let mut k = 0;
        let visited = iv.query(n + 1, n + 5, &mut |_| k += 1);
        assert_eq!(k, 0);
        assert!(visited < 64, "visited {visited} nodes for no match");
    }

    /// `rows` rows of six numbers (E:J) and `=SUM(E{r}:J{r})` in K.
    fn sum_rows(rows: u32) -> Workbook {
        let mut wb = Workbook::new();
        if let Some(sh) = wb.sheet_mut(0) {
            for r in 0..rows {
                for c in 4..10 {
                    sh.cells.set(CellRef::new(r, c), Cell::value(Value::Number(f64::from(r % 97 + c))));
                }
                sh.cells.set(CellRef::new(r, 10), Cell::formula(Formula::new(&format!("=SUM(E{0}:J{0})", r + 1))));
            }
        }
        wb
    }

    /// Opening (full recalc), changing every value of a column, and re-entering every formula.
    #[allow(clippy::disallowed_methods)] // a native-only timing test: the clock is read off wasm
    fn open_and_bulk_edit(rows: u32) -> Duration {
        let mut wb = sum_rows(rows);
        let mut calc = Calc::new();
        let t = Instant::now();
        calc.recalc_all(&mut wb);
        let values: Vec<Key> = (0..rows).map(|r| (0, CellRef::new(r, 4))).collect();
        if let Some(sh) = wb.sheet_mut(0) {
            for (_, c) in &values {
                sh.cells.set(*c, Cell::value(Value::Number(1.0)));
            }
        }
        calc.cells_changed(&mut wb, &values);
        let formulas: Vec<Key> = (0..rows).map(|r| (0, CellRef::new(r, 10))).collect();
        calc.cells_changed(&mut wb, &formulas);
        let elapsed = t.elapsed();
        let last = wb.sheet(0).map(|s| s.value(CellRef::new(rows - 1, 10)));
        let r = rows - 1;
        assert_eq!(last, Some(Value::Number(f64::from(1 + (5..10).map(|c| r % 97 + c).sum::<u32>()))));
        elapsed
    }

    #[test]
    fn range_formulas_recalc_and_bulk_edit_correctly() {
        // The values behind the timing test below, checked on every run.
        open_and_bulk_edit(500);
    }

    #[test]
    #[ignore = "timing-sensitive; run with --ignored in release"]
    #[allow(clippy::disallowed_methods)]
    fn recalc_with_range_formulas_scales_linearly() {
        // Each row's SUM reads a range in the same columns. Recalculating on open used to walk
        // every row per range read (the used range), and finding a cell's dependents scanned
        // every range in its column: 4x the rows took 16x the time. Linear is 4x; allow 8x.
        let best = |rows| (0..3).map(|_| open_and_bulk_edit(rows)).min().unwrap_or_default();
        let small = best(5_000);
        let large = best(20_000);
        assert!(large < small * 8 + Duration::from_millis(100), "5k rows {small:?}, 20k rows {large:?}");
    }

    #[test]
    fn range_dependents_after_edits() {
        // Formulas that read ranges, wide ranges and whole columns, found again after edits.
        let mut wb = Workbook::new();
        let mut calc = Calc::new();
        let set = |wb: &mut Workbook, calc: &mut Calc, at: &str, input: &str| {
            let c = CellRef::parse(at).unwrap();
            let cell = if input.starts_with('=') { Cell::formula(Formula::new(input)) } else { Cell::value(Value::Number(input.parse().unwrap())) };
            wb.sheet_mut(0).unwrap().cells.set(c, cell);
            calc.cells_changed(wb, &[(0, c)]);
        };
        let num = |wb: &Workbook, at: &str| {
            let v = wb.sheet(0).unwrap().value(CellRef::parse(at).unwrap());
            v.as_f64().unwrap_or_else(|| panic!("{at} = {v:?}"))
        };
        set(&mut wb, &mut calc, "A1", "1");
        set(&mut wb, &mut calc, "B2", "2");
        set(&mut wb, &mut calc, "CZ3", "3");
        set(&mut wb, &mut calc, "DA5", "=SUM(A1:B2)");
        set(&mut wb, &mut calc, "DA6", "=SUM(A1:CZ3)");
        set(&mut wb, &mut calc, "DA7", "=SUM(B:B)");
        set(&mut wb, &mut calc, "DA8", "=SUM(2:2)");
        assert_eq!((num(&wb, "DA5"), num(&wb, "DA6"), num(&wb, "DA7"), num(&wb, "DA8")), (3.0, 6.0, 2.0, 2.0));
        set(&mut wb, &mut calc, "B2", "20");
        assert_eq!((num(&wb, "DA5"), num(&wb, "DA6"), num(&wb, "DA7"), num(&wb, "DA8")), (21.0, 24.0, 20.0, 20.0));
        // Re-entering a formula with another range drops the old one.
        set(&mut wb, &mut calc, "DA5", "=SUM(A1:A2)");
        set(&mut wb, &mut calc, "B2", "5");
        assert_eq!(num(&wb, "DA5"), 1.0);
        let mut deps = Vec::new();
        calc.graph.dependents(0, CellRef::parse("B2").unwrap(), &mut deps);
        deps.sort_unstable();
        let at = |s: &str| (0, CellRef::parse(s).unwrap());
        assert_eq!(deps, vec![at("DA6"), at("DA7"), at("DA8")]);
        deps.clear();
        calc.graph.dependents_of_range(0, RangeRef::new(CellRef::parse("A1").unwrap(), CellRef::parse("A9").unwrap()), &mut deps);
        deps.sort_unstable();
        deps.dedup();
        assert_eq!(deps, vec![at("DA5"), at("DA6"), at("DA8")]);
        set(&mut wb, &mut calc, "CZ3", "30");
        assert_eq!(num(&wb, "DA6"), 36.0);
        // Ranges on another sheet.
        wb.sheets.push(std::sync::Arc::new(gridcraft_model::Sheet::new("Data")));
        set(&mut wb, &mut calc, "DA9", "=SUM(Data!A1:A3)+SUM(Data!A:CZ)");
        assert_eq!(num(&wb, "DA9"), 0.0);
        wb.sheet_mut(1).unwrap().cells.set(CellRef::parse("A2").unwrap(), Cell::value(Value::Number(4.0)));
        calc.cells_changed(&mut wb, &[(1, CellRef::parse("A2").unwrap())]);
        assert_eq!(num(&wb, "DA9"), 8.0);
        deps.clear();
        calc.graph.dependents(1, CellRef::parse("A2").unwrap(), &mut deps);
        assert_eq!(deps, vec![at("DA9"), at("DA9")]);
    }

    #[test]
    fn independent_columns_recalculate_on_several_threads() {
        // Two hundred rows: B reads a shared input, C reads B. The first column is one parallel
        // wave; the second waits for it. A self-reference stays on the calling thread (`#CIRC!`).
        let n = 200u32;
        let mut wb = Workbook::new();
        {
            let sh = wb.sheet_mut(0).unwrap();
            sh.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(3.0)));
            for r in 0..n {
                sh.cells.set(CellRef::new(r, 1), Cell::formula(Formula::new(&format!("=$A$1+{r}"))));
                sh.cells.set(CellRef::new(r, 2), Cell::formula(Formula::new(&format!("=B{}+1", r + 1))));
            }
            sh.cells.set(CellRef::new(0, 25), Cell::formula(Formula::new("=Z1+1")));
        }
        let mut calc = Calc::new();
        calc.recalc_all(&mut wb);
        let sh = wb.sheet(0).unwrap();
        for r in 0..n {
            assert_eq!(sh.value(CellRef::new(r, 1)), Value::Number(3.0 + f64::from(r)));
            assert_eq!(sh.value(CellRef::new(r, 2)), Value::Number(4.0 + f64::from(r)));
        }
        assert_eq!(sh.value(CellRef::new(0, 25)), Value::Error(gridcraft_core::CellError::Circ));
        wb.sheet_mut(0).unwrap().cells.set(CellRef::new(0, 0), Cell::value(Value::Number(10.0)));
        calc.cells_changed(&mut wb, &[(0, CellRef::new(0, 0))]);
        let sh = wb.sheet(0).unwrap();
        assert_eq!(sh.value(CellRef::new(0, 1)), Value::Number(10.0));
        assert_eq!(sh.value(CellRef::new(n - 1, 2)), Value::Number(10.0 + f64::from(n - 1) + 1.0));
        assert_eq!(sh.value(CellRef::new(0, 25)), Value::Error(gridcraft_core::CellError::Circ));
    }

    #[test]
    fn parallel_wave_uses_two_threads_and_matches_serial_dynamic_results() {
        // Same dynamic formulas on a book too small to schedule, and on one large enough for
        // two waves. `INDIRECT` / `OFFSET` / `RAND` and the cycle must match the serial book.
        fn fill(wb: &mut Workbook, rows: u32) {
            let sh = wb.sheet_mut(0).unwrap();
            sh.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(7.0)));
            for r in 0..rows {
                sh.cells.set(CellRef::new(r, 1), Cell::formula(Formula::new(&format!("=$A$1+{r}"))));
                sh.cells.set(CellRef::new(r, 2), Cell::formula(Formula::new(&format!("=B{}+1", r + 1))));
            }
            sh.cells.set(CellRef::new(0, 3), Cell::formula(Formula::new("=INDIRECT(\"B1\")")));
            sh.cells.set(CellRef::new(0, 4), Cell::formula(Formula::new("=OFFSET(A1,0,1)")));
            sh.cells.set(CellRef::new(0, 5), Cell::formula(Formula::new("=RAND()")));
            sh.cells.set(CellRef::new(0, 25), Cell::formula(Formula::new("=Z1+1")));
        }
        let value = |wb: &Workbook, col: u32| wb.sheet(0).unwrap().value(CellRef::new(0, col));
        let mut small = Workbook::new();
        fill(&mut small, 8);
        Calc::new().recalc_all(&mut small);
        let mut large = Workbook::new();
        fill(&mut large, 200);
        let watch = super::watch_wave_threads();
        Calc::new().recalc_all(&mut large);
        let threads = super::take_wave_threads(watch.0);
        assert!(threads.len() >= 2, "parallel wave ran on {threads:?}");
        assert_eq!(value(&large, 1), Value::Number(7.0));
        assert_eq!(value(&large, 2), Value::Number(8.0));
        assert_eq!(value(&large, 3), value(&small, 3));
        assert_eq!(value(&large, 3), Value::Number(7.0));
        assert_eq!(value(&large, 4), value(&small, 4));
        assert_eq!(value(&large, 4), Value::Number(7.0));
        let rand = value(&large, 5).as_f64().unwrap();
        assert_eq!(value(&large, 5), value(&small, 5));
        assert!((0.0..1.0).contains(&rand), "{rand}");
        assert_eq!(value(&large, 25), Value::Error(gridcraft_core::CellError::Circ));
        assert_eq!(value(&large, 25), value(&small, 25));
    }

    #[test]
    fn second_wave_column_sum_reads_prior_wave_values() {
        // A1:A70 are `=1` (one wave). P1048576 is a constant, so the used range of A:P is the
        // whole column span and the trimmed block is past `MAX_CELLS`. AA1:AA80 are `=SUM(A:P)`
        // and wait for that first wave. The sparse walk must add the prior ones (70); building
        // the rectangle would store `#NUM!`.
        let mut wb = Workbook::new();
        {
            let sh = wb.sheet_mut(0).unwrap();
            for r in 0..70 {
                sh.cells.set(CellRef::new(r, 0), Cell::formula(Formula::new("=1")));
            }
            let corner = CellRef::parse("P1048576").unwrap();
            sh.cells.set(corner, Cell::value(Value::Number(0.0)));
            for r in 0..80 {
                sh.cells.set(CellRef::new(r, 26), Cell::formula(Formula::new("=SUM(A:P)")));
            }
        }
        Calc::new().recalc_all(&mut wb);
        let sh = wb.sheet(0).unwrap();
        assert_eq!(sh.value(CellRef::new(0, 0)), Value::Number(1.0));
        assert_eq!(sh.value(CellRef::parse("P1048576").unwrap()), Value::Number(0.0));
        for r in 0..80 {
            assert_eq!(sh.value(CellRef::new(r, 26)), Value::Number(70.0), "AA{}", r + 1);
        }
    }

    #[test]
    fn second_wave_sum_ignores_a_spill_outside_the_range() {
        // `=SEQUENCE(3)` spills outside A:P. That spill must not make `SUM(A:P)` build the
        // trimmed rectangle: `P1048576` pushes it past `MAX_CELLS`, which would store `#NUM!`.
        fn book(sum_rows: u32, ones_are_formulas: bool, spill_at: &str) -> Workbook {
            let mut wb = Workbook::new();
            let sh = wb.sheet_mut(0).unwrap();
            for r in 0..70 {
                let cell = if ones_are_formulas { Cell::formula(Formula::new("=1")) } else { Cell::value(Value::Number(1.0)) };
                sh.cells.set(CellRef::new(r, 0), cell);
            }
            sh.cells.set(CellRef::parse("P1048576").unwrap(), Cell::value(Value::Number(0.0)));
            sh.cells.set(CellRef::parse(spill_at).unwrap(), Cell::formula(Formula::new("=SEQUENCE(3)")));
            for r in 0..sum_rows {
                sh.cells.set(CellRef::new(r, 26), Cell::formula(Formula::new("=SUM(A:P)")));
            }
            wb
        }
        let mut large = book(80, true, "Z100");
        Calc::new().recalc_all(&mut large);
        let sh = large.sheet(0).unwrap();
        assert_eq!(sh.value(CellRef::parse("Z100").unwrap()), Value::Number(1.0));
        assert_eq!(sh.value(CellRef::parse("Z101").unwrap()), Value::Number(2.0));
        for r in 0..80 {
            assert_eq!(sh.value(CellRef::new(r, 26)), Value::Number(70.0), "AA{}", r + 1);
        }

        // Under `PARALLEL_MIN` the caller evaluates in row order, so Z1 spills before AA1 is read.
        let mut small = book(8, false, "Z1");
        Calc::new().recalc_all(&mut small);
        let sh = small.sheet(0).unwrap();
        assert_eq!(sh.value(CellRef::parse("Z1").unwrap()), Value::Number(1.0));
        assert_eq!(sh.value(CellRef::parse("Z2").unwrap()), Value::Number(2.0));
        assert_eq!(sh.value(CellRef::parse("Z3").unwrap()), Value::Number(3.0));
        for r in 0..8 {
            assert_eq!(sh.value(CellRef::new(r, 26)), Value::Number(70.0), "AA{}", r + 1);
        }
    }

    #[test]
    fn second_wave_sum_adds_a_spill_inside_the_range() {
        // A1:A70 are 1, P1048576 is 0, and B1 = SEQUENCE(3) spills B1:B3 inside A:P.
        // AA = SUM(A:P) is 70 plus 1+2+3. The trimmed rectangle is past MAX_CELLS, so building
        // it would store #NUM!. A book under PARALLEL_MIN takes the same sparse total.
        fn book(sum_rows: u32, ones_are_formulas: bool) -> Workbook {
            let mut wb = Workbook::new();
            let sh = wb.sheet_mut(0).unwrap();
            for r in 0..70 {
                let cell = if ones_are_formulas { Cell::formula(Formula::new("=1")) } else { Cell::value(Value::Number(1.0)) };
                sh.cells.set(CellRef::new(r, 0), cell);
            }
            sh.cells.set(CellRef::parse("P1048576").unwrap(), Cell::value(Value::Number(0.0)));
            sh.cells.set(CellRef::parse("B1").unwrap(), Cell::formula(Formula::new("=SEQUENCE(3)")));
            for r in 0..sum_rows {
                sh.cells.set(CellRef::new(r, 26), Cell::formula(Formula::new("=SUM(A:P)")));
            }
            wb
        }
        let mut large = book(80, true);
        Calc::new().recalc_all(&mut large);
        let sh = large.sheet(0).unwrap();
        assert_eq!(sh.value(CellRef::parse("B1").unwrap()), Value::Number(1.0));
        assert_eq!(sh.value(CellRef::parse("B2").unwrap()), Value::Number(2.0));
        assert_eq!(sh.value(CellRef::parse("B3").unwrap()), Value::Number(3.0));
        for r in 0..80 {
            assert_eq!(sh.value(CellRef::new(r, 26)), Value::Number(76.0), "AA{}", r + 1);
        }

        let mut small = book(8, false);
        Calc::new().recalc_all(&mut small);
        let sh = small.sheet(0).unwrap();
        assert_eq!(sh.value(CellRef::parse("B1").unwrap()), Value::Number(1.0));
        assert_eq!(sh.value(CellRef::parse("B2").unwrap()), Value::Number(2.0));
        assert_eq!(sh.value(CellRef::parse("B3").unwrap()), Value::Number(3.0));
        for r in 0..8 {
            assert_eq!(sh.value(CellRef::new(r, 26)), Value::Number(76.0), "AA{}", r + 1);
        }
    }

    #[test]
    fn same_row_sum_after_an_edit_matches_the_evaluator() {
        // The #66 shape: `=SUM(E{r}:J{r})+$A$1`. Eighty rows take the flat wave; twenty stay
        // under the parallel threshold. Text, bools and errors in the row follow SUM's rules.
        fn book(rows: u32) -> Workbook {
            let mut wb = Workbook::new();
            let sh = wb.sheet_mut(0).unwrap();
            sh.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(1.0)));
            for r in 0..rows {
                for c in 4..10 {
                    sh.cells.set(CellRef::new(r, c), Cell::value(Value::Number(1.0)));
                }
                let row = r + 1;
                sh.cells.set(CellRef::new(r, 10), Cell::formula(Formula::new(&format!("=SUM(E{row}:J{row})+$A$1"))));
            }
            sh.cells.set(CellRef::new(10, 4), Cell::value(Value::from("x")));
            sh.cells.set(CellRef::new(11, 4), Cell::value(Value::Bool(true)));
            sh.cells.set(CellRef::new(12, 4), Cell::value(Value::Error(CellError::NA)));
            sh.cells.set(CellRef::new(13, 4), Cell::formula(Formula::new("=2")));
            wb
        }
        for rows in [20u32, 80] {
            let mut wb = book(rows);
            let mut calc = Calc::new();
            calc.recalc_all(&mut wb);
            if let Some(sh) = wb.sheet_mut(0) {
                sh.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(4.0)));
            }
            calc.cells_changed(&mut wb, &[(0, CellRef::new(0, 0))]);
            let sh = wb.sheet(0).unwrap();
            let samples = [
                (0u32, Value::Number(10.0)),
                (10, Value::Number(9.0)),
                (11, Value::Number(9.0)),
                (12, Value::Error(CellError::NA)),
                (13, Value::Number(11.0)),
            ];
            for (r, want) in samples {
                let at = CellRef::new(r, 10);
                let formula = sh.cell(at).and_then(|c| c.formula.as_ref()).map(|f| f.text.clone()).unwrap();
                assert_eq!(sh.value(at), want, "stored row {} of {rows}", r + 1);
                assert_eq!(evaluate(&wb, 0, at, &formula), want, "eval row {} of {rows}", r + 1);
            }
        }
    }

    #[test]
    fn flat_wave_sum_keeps_a_spill_that_intersects_the_range() {
        // Once the spill is on the sheet, editing a constant dirties only the sums. Those sums
        // are independent of each other, so the flat wave runs. `P1048576` makes the dense
        // rectangle too big: the total has to stay 70 plus the spilled 1+2+3.
        let mut wb = Workbook::new();
        {
            let sh = wb.sheet_mut(0).unwrap();
            for r in 0..70 {
                sh.cells.set(CellRef::new(r, 0), Cell::value(Value::Number(1.0)));
            }
            sh.cells.set(CellRef::parse("P1048576").unwrap(), Cell::value(Value::Number(0.0)));
            sh.cells.set(CellRef::parse("B1").unwrap(), Cell::formula(Formula::new("=SEQUENCE(3)")));
            for r in 0..80 {
                sh.cells.set(CellRef::new(r, 26), Cell::formula(Formula::new("=SUM(A:P)")));
            }
        }
        let mut calc = Calc::new();
        calc.recalc_all(&mut wb);
        assert_eq!(wb.sheet(0).unwrap().value(CellRef::new(0, 26)), Value::Number(76.0));
        if let Some(sh) = wb.sheet_mut(0) {
            sh.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(2.0)));
        }
        calc.cells_changed(&mut wb, &[(0, CellRef::new(0, 0))]);
        let sh = wb.sheet(0).unwrap();
        assert_eq!(sh.value(CellRef::parse("B1").unwrap()), Value::Number(1.0));
        assert_eq!(sh.value(CellRef::parse("B2").unwrap()), Value::Number(2.0));
        assert_eq!(sh.value(CellRef::parse("B3").unwrap()), Value::Number(3.0));
        for r in 0..80 {
            assert_eq!(sh.value(CellRef::new(r, 26)), Value::Number(77.0), "AA{}", r + 1);
        }
    }

    #[test]
    fn worker_cap_of_one_matches_serial_rand_iteration_and_cycles() {
        fn fill(wb: &mut Workbook, rows: u32, iterative: bool) {
            wb.calc.iterative = iterative;
            wb.calc.set_max_iterations(5);
            let sh = wb.sheet_mut(0).unwrap();
            sh.cells.set(CellRef::new(0, 0), Cell::formula(Formula::new("=A1+1")));
            for r in 0..rows {
                sh.cells.set(CellRef::new(r, 1), Cell::formula(Formula::new(&format!("={r}+3"))));
            }
            sh.cells.set(CellRef::new(0, 3), Cell::formula(Formula::new("=INDIRECT(\"B1\")")));
            sh.cells.set(CellRef::new(0, 4), Cell::formula(Formula::new("=OFFSET(A1,0,1)")));
            sh.cells.set(CellRef::new(0, 5), Cell::formula(Formula::new("=RAND()")));
            sh.cells.set(CellRef::new(0, 25), Cell::formula(Formula::new("=Z1+1")));
        }
        let value = |wb: &Workbook, col: u32| wb.sheet(0).unwrap().value(CellRef::new(0, col));
        for iterative in [false, true] {
            let mut small = Workbook::new();
            fill(&mut small, 8, iterative);
            Calc::new().recalc_all(&mut small);
            let mut capped = Workbook::new();
            fill(&mut capped, 200, iterative);
            capped.calc.set_max_workers(1);
            let watch = super::watch_wave_threads();
            Calc::new().recalc_all(&mut capped);
            let threads = super::take_wave_threads(watch.0);
            assert!(threads.is_empty(), "cap 1 spawned workers (iterative={iterative}): {threads:?}");
            assert_eq!(value(&capped, 1), Value::Number(3.0));
            assert_eq!(value(&capped, 1), value(&small, 1));
            assert_eq!(value(&capped, 3), value(&small, 3));
            assert_eq!(value(&capped, 4), value(&small, 4));
            assert_eq!(value(&capped, 5), value(&small, 5), "RAND iterative={iterative}");
            assert_eq!(value(&capped, 25), value(&small, 25), "cycle iterative={iterative}");
            if iterative {
                assert_eq!(value(&capped, 0), Value::Number(5.0));
            } else {
                assert_eq!(value(&capped, 0), Value::Error(gridcraft_core::CellError::Circ));
                assert_eq!(value(&capped, 25), Value::Error(gridcraft_core::CellError::Circ));
            }
            let last = capped.sheet(0).unwrap().value(CellRef::new(199, 1));
            assert_eq!(last, Value::Number(199.0 + 3.0));
        }
    }

    #[test]
    fn worker_panic_falls_back_to_the_caller() {
        super::WAVE_PANIC.store(true, std::sync::atomic::Ordering::Relaxed);
        let _reset = super::ResetWavePanic;
        let n = 80u32;
        let mut wb = Workbook::new();
        {
            let sh = wb.sheet_mut(0).unwrap();
            for r in 0..n {
                sh.cells.set(CellRef::new(r, 0), Cell::formula(Formula::new(&format!("={r}+1"))));
            }
        }
        Calc::new().recalc_all(&mut wb);
        let sh = wb.sheet(0).unwrap();
        for r in 0..n {
            assert_eq!(sh.value(CellRef::new(r, 0)), Value::Number(f64::from(r) + 1.0));
        }
    }

    #[test]
    fn iterative_calculation_stays_correct_beside_independent_formulas() {
        let n = 80u32;
        let mut wb = Workbook::new();
        wb.calc.iterative = true;
        wb.calc.set_max_iterations(5);
        {
            let sh = wb.sheet_mut(0).unwrap();
            sh.cells.set(CellRef::new(0, 0), Cell::formula(Formula::new("=A1+1")));
            for r in 0..n {
                sh.cells.set(CellRef::new(r, 1), Cell::formula(Formula::new(&format!("={r}+3"))));
            }
        }
        Calc::new().recalc_all(&mut wb);
        let sh = wb.sheet(0).unwrap();
        assert_eq!(sh.value(CellRef::new(0, 0)), Value::Number(5.0));
        for r in 0..n {
            assert_eq!(sh.value(CellRef::new(r, 1)), Value::Number(f64::from(r) + 3.0));
        }
    }

    /// For tests of `PassHost::visit_range`, which can't be reached from outside this module: the
    /// values it hands over for `range` while the formulas in `pending` (cell, formula) have not been
    /// calculated yet, or `None` when it declines the block.
    pub(crate) fn probe_visit_range(wb: &Workbook, pending: &[(&str, &str)], range: RangeRef) -> Option<Vec<Value>> {
        let mut exprs = HashMap::default();
        for (at, formula) in pending {
            let expr = gridcraft_formula::parse(formula.strip_prefix('=').unwrap_or(formula)).ok()?;
            exprs.insert((0, CellRef::parse(at)?), std::sync::Arc::new(expr));
        }

        // A cell in `pending` is calculated the first time something reads it, which is the state
        // of a real pass partway through; `visit_range` has to cope with the formulas it reads
        // being calculated under it.
        let mut rng = 1u64;
        let mut host = PassHost::new(wb, &exprs, &mut rng, 0.0);

        let mut seen = Vec::new();
        host.visit_range(0, range, &mut |v| {
            seen.push(v.clone());
            true
        })
        .then_some(seen)
    }
}
