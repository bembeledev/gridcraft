use super::*;

/// SUMIF and AVERAGEIF with a sum range: Excel sizes the sum range like the criteria range.
pub(super) fn resizes_sum_range(name: &str, args: usize) -> bool {
    args == 3 && matches!(name, "SUMIF" | "AVERAGEIF")
}

/// Height and width of a single-area reference or an array argument.
pub(super) fn ev_shape(ev: &Ev) -> Option<(u32, u32)> {
    match ev {
        Ev::R(areas) => match areas.as_slice() {
            [a] => Some((a.range.height(), a.range.width())),
            _ => None,
        },
        Ev::V(Value::Array(a)) => Some((u32::try_from(a.rows).unwrap_or(u32::MAX), u32::try_from(a.cols).unwrap_or(u32::MAX))),
        Ev::V(_) => Some((1, 1)),
        Ev::L(_) => None,
    }
}

/// The range SUMIF/AVERAGEIF add up: from the sum range's top-left cell, as many rows and columns
/// as the criteria range (`height` × `width`), cut off at the sheet's edge.
pub fn resized_sum_range(sum: RangeRef, height: u32, width: u32) -> RangeRef {
    let row = sum.start.row.saturating_add(height.saturating_sub(1)).min(MAX_ROWS - 1);
    let col = sum.start.col.saturating_add(width.saturating_sub(1)).min(MAX_COLS - 1);
    RangeRef::new(sum.start, CellRef::new(row, col))
}

/// Collects the static references of an expression with their sheets resolved; `dynamic` is
/// set when the formula uses INDIRECT/OFFSET/tables (or a name that can't be resolved now) whose
/// targets can change between recalcs.
///
/// Defined names are resolved here, not treated as dynamic: a name holding a range contributes
/// that range, a name holding a LAMBDA contributes the references inside its body (its
/// parameters, and LET variables, are bound names, not lookups), names referring to names follow
/// the chain to a bounded depth. A column of cells calling one LAMBDA name is then an ordinary
/// static dependency, re-evaluated when its inputs change and only then. Changing a name's
/// definition is a structural edit (full rebuild), and the engine re-resolves the readers of the
/// hidden names its commands rewrite without one, so the resolution can't go stale. Names with
/// relative references (evaluated relative to the using cell), unknown names, unparsable names,
/// sheet-scoped names used from another sheet, cycles between names and chains deeper than the
/// bound stay dynamic. A LAMBDA name calling itself is not a cycle here: the call reads what its
/// body reads.
///
/// Resolves names through a fresh [`NameCache`]; a graph build resolving many formulas shares one
/// through [`precedents_with`].
pub fn precedents(wb: &Workbook, sheet: usize, e: &Expr) -> (Vec<Area>, bool) {
    precedents_with(wb, sheet, e, &mut NameCache::default())
}

/// [`precedents`] with the defined names resolved through `names`, shared across the formulas of
/// one graph build.
pub fn precedents_with(wb: &Workbook, sheet: usize, e: &Expr, names: &mut NameCache) -> (Vec<Area>, bool) {
    let mut out = Vec::new();
    let mut dynamic = false;
    collect_precedents(wb, sheet, e, &mut Vec::new(), &mut out, &mut dynamic, 0, 0, stack_position(), names);
    // Each area once: a formula mentioning a name or a range k times depends on it once.
    let mut distinct = HashSet::new();
    out.retain(|a| distinct.insert(*a));
    (out, dynamic)
}

/// Names referring to names are followed this many levels before the formula is called dynamic.
const NAME_DEPTH: usize = 8;

/// Nesting the precedent walk follows in total, through names included: the parser's own bound
/// for one formula, counted the way the parser counts (a left operand, a postfix `%` or `#`, an
/// invoked result and an array constant's elements sit at their expression's level; a right
/// operand, an argument and a parenthesis one deeper; the walk's count is one less than the
/// parser's depth, the formula's own level), so
/// no single formula is cut, while a chain of names that together nest past it is. The native
/// stack the walk may use is bounded on top of this, by `WALK_STACK_BUDGET` and `WALK_PARSE_BUDGET`.
const MAX_NESTING: usize = 256;

/// Native stack, in bytes, the precedent walk may use below `precedents_with`, however the parser
/// counted the nesting: the nesting count follows the parser shape by shape, but a shape the parser
/// counts differently (a trim reference costs it nothing and the walk one or two frames a link)
/// must still fit a 1 MiB thread (the Windows main thread), with room for the frames above the
/// walk. Past it the formula is dynamic: of what one formula can hold, only a trim-reference chain
/// of over a thousand links gets there. Measured like [`STACK_BUDGET`], with [`stack_position`].
const WALK_STACK_BUDGET: usize = 768 * 1024;
/// Stack the walk may have used when it meets a defined name, whose definition it parses on top of
/// its deepest frame: a definition nested to the parser's limit needs about 565 KiB in the test
/// profile and 500 KiB in release, so this and that together stay within the budget above. Met
/// deeper down, the name stays dynamic for that formula.
const WALK_PARSE_BUDGET: usize = 192 * 1024;

/// What each defined name contributes to the formulas using it, resolved once per graph build.
///
/// The workbook's names don't change while a graph is built, so a name read from thousands of
/// cells, or mentioned many times inside another name's definition, is resolved the first time
/// and looked up after that. The work of a build is then linear in the size of the definitions,
/// however often they are referenced; followed afresh at every mention, a chain of names each
/// mentioning the previous one k times would cost k^depth per use. A name met again from its own
/// definition (a recursive LAMBDA) adds nothing; met again from deeper down the chain it is a
/// cycle, and the names on it are dynamic. A chain cut by [`NAME_DEPTH`] is
/// dynamic from the cut up to the formula, and the cache keeps that answer for those names even
/// where a shorter chain reaches them later: dynamic is always a correct answer, only a slower one.
#[derive(Default)]
pub struct NameCache {
    /// Keyed by the name, its scope and the sheet its unqualified references resolve against.
    entries: HashMap<(String, Option<usize>, usize), NameEntry>,
}

enum NameEntry {
    /// Being resolved further up the current chain, entered at `depth`: met again from its own
    /// definition it is a direct self-call, from deeper down a cycle.
    Resolving {
        depth: usize,
    },
    Resolved {
        areas: Vec<Area>,
        dynamic: bool,
    },
}

/// Adds `range` to `out` on the sheet(s) `r` names, or on `sheet` for an unqualified reference;
/// a sheet that doesn't exist contributes nothing (evaluation reports #REF!).
fn push_area(wb: &Workbook, sheet: usize, out: &mut Vec<Area>, r: &Reference, range: RangeRef) {
    match &r.sheet {
        SheetSel::Current => out.push(Area { sheet, range }),
        SheetSel::Named(n) => {
            if let Some(i) = wb.sheet_index(n) {
                out.push(Area { sheet: i, range });
            }
        }
        SheetSel::Span(a, b) => {
            if let (Some(i), Some(j)) = (wb.sheet_index(a), wb.sheet_index(b)) {
                for s in i.min(j)..=i.max(j) {
                    out.push(Area { sheet: s, range });
                }
            }
        }
    }
}

/// A LET/LAMBDA parameter as written: `[x]` for an optional one is bound as `x`.
fn bound_name(n: &str) -> String {
    n.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase()
}

/// Adds the precedents a defined name contributes when used from `sheet` to `out`, resolving it
/// through `names` the first time and looking it up after that.
#[allow(clippy::too_many_arguments)]
fn name_precedents(
    wb: &Workbook,
    sheet: usize,
    n: &str,
    out: &mut Vec<Area>,
    dynamic: &mut bool,
    depth: usize,
    nest: usize,
    top: usize,
    names: &mut NameCache,
) {
    let (scope_sheet, bare) = match n.split_once('!') {
        Some((s, b)) => match wb.sheet_index(s.trim_matches('\'')) {
            Some(i) => (i, b),
            None => {
                *dynamic = true;
                return;
            }
        },
        None => (sheet, n),
    };
    let Some(def) = wb.name(bare, scope_sheet) else {
        // Not a defined name: a table name (data body known only at evaluation) or a typo
        // (#NAME?). Either way, evaluate every time.
        *dynamic = true;
        return;
    };
    // A sheet-scoped name used from another sheet (`Sheet1!F`): a LAMBDA it yields runs its body
    // against the calling cell's sheet, not the scope sheet its references are resolved against
    // here, so the cells it reads aren't known statically.
    if def.scope.is_some_and(|s| s != sheet) {
        *dynamic = true;
    }
    let name_sheet = def.scope.unwrap_or(sheet);
    let key = (def.name.to_ascii_lowercase(), def.scope, name_sheet);
    // Looked up before the depth bound: a hit doesn't recurse.
    match names.entries.get(&key) {
        // A direct self-call (a recursive LAMBDA): it reads what the body being collected reads,
        // nothing more.
        Some(NameEntry::Resolving { depth: entered }) if entered.saturating_add(1) == depth => return,
        Some(NameEntry::Resolving { .. }) => {
            // A longer cycle (A→B→A): B's entry lacks A's areas, so the names on it are dynamic.
            *dynamic = true;
            return;
        }
        Some(NameEntry::Resolved { areas, dynamic: d }) => {
            out.extend_from_slice(areas);
            *dynamic |= *d;
            return;
        }
        None => {}
    }
    if depth >= NAME_DEPTH || stack_position().abs_diff(top) > WALK_PARSE_BUDGET {
        *dynamic = true;
        return;
    }
    let mut areas = Vec::new();
    let mut d = false;
    match gridcraft_formula::parse_name(&def.formula) {
        Ok(expr) if !gridcraft_formula::adjust::has_relative_refs(&expr) => {
            names.entries.insert(key.clone(), NameEntry::Resolving { depth });
            collect_precedents(wb, name_sheet, &expr, &mut Vec::new(), &mut areas, &mut d, depth + 1, nest + 1, top, names);
            // Stored once each: a definition mentioning a range k times contributes it once.
            let mut distinct = HashSet::new();
            areas.retain(|a| distinct.insert(*a));
        }
        _ => d = true,
    }
    out.extend_from_slice(&areas);
    *dynamic |= d;
    names.entries.insert(key, NameEntry::Resolved { areas, dynamic: d });
}

/// Walks `e`, adding every static reference to `out` and setting `dynamic` for what can't be
/// resolved now. `bound` holds the LAMBDA parameters and LET variables in scope, which are not
/// name lookups; `depth` counts the defined names entered on the way here.
#[allow(clippy::too_many_arguments)]
fn collect_precedents(
    wb: &Workbook,
    sheet: usize,
    e: &Expr,
    bound: &mut Vec<String>,
    out: &mut Vec<Area>,
    dynamic: &mut bool,
    depth: usize,
    nest: usize,
    top: usize,
    names: &mut NameCache,
) {
    if nest >= MAX_NESTING || stack_position().abs_diff(top) > WALK_STACK_BUDGET {
        // Deeper than the parser lets one formula nest, or than the stack is given, so only a
        // chain of names gets here: what it reads is known at evaluation instead.
        *dynamic = true;
        return;
    }
    match e {
        Expr::Ref(r) => push_area(wb, sheet, out, r, r.range()),
        Expr::Name(n) => {
            if !bound.contains(&bound_name(n)) {
                name_precedents(wb, sheet, n, out, dynamic, depth, nest, top, names);
            }
        }
        Expr::Struct(_) => *dynamic = true,
        // An array constant's elements are read by the parser without a level of their own.
        Expr::Array(rows) => {
            for x in rows.iter().flatten() {
                collect_precedents(wb, sheet, x, bound, out, dynamic, depth, nest, top, names);
            }
        }
        Expr::Unary(UnOp::Percent, _) => {
            // Postfix: the parser wraps the operand without a level of its own, as often as the
            // nesting left allows; unwound here the same way.
            let mut node = e;
            while let Expr::Unary(UnOp::Percent, inner) = node {
                node = inner;
            }
            collect_precedents(wb, sheet, node, bound, out, dynamic, depth, nest, top, names);
        }
        // A trim reference is the range operator with a modifier: its operands sit where a
        // range's do. The spill operator (`A1#`) is a postfix on a reference, at its level.
        Expr::Unary(UnOp::Trim(_) | UnOp::Spill, a) => collect_precedents(wb, sheet, a, bound, out, dynamic, depth, nest, top, names),
        Expr::Unary(_, a) | Expr::Paren(a) => collect_precedents(wb, sheet, a, bound, out, dynamic, depth, nest + 1, top, names),
        // A range ending in a function's result (`A1:INDEX(B1:B9,n)`) covers the cells between
        // the references inside it, not only those references. A left spine of such ranges
        // (`X:(B1):(C1)`) is unwound like an operator chain: the parser has each left operand at
        // its expression's level and each right operand one deeper.
        Expr::Binary(BinOp::Range, a, b) if !matches!((&**a, &**b), (Expr::Ref(_), Expr::Ref(_))) => {
            let mut inner = Vec::new();
            let mut rights = vec![&**b];
            let mut node = &**a;
            while let Expr::Binary(BinOp::Range, x, y) = node
                && !matches!((&**x, &**y), (Expr::Ref(_), Expr::Ref(_)))
            {
                rights.push(&**y);
                node = x;
            }
            collect_precedents(wb, sheet, node, bound, &mut inner, dynamic, depth, nest, top, names);
            for r in rights.into_iter().rev() {
                collect_precedents(wb, sheet, r, bound, &mut inner, dynamic, depth, nest + 1, top, names);
            }
            let mut boxes: Vec<Area> = Vec::new();
            for area in inner {
                match boxes.iter_mut().find(|x| x.sheet == area.sheet) {
                    Some(x) => x.range = x.range.union(&area.range),
                    None => boxes.push(area),
                }
            }
            out.extend(boxes);
        }
        Expr::Binary(..) => {
            // Unwind the left spine of a long operator chain (`=A1+A2+…`, 4,096 operands within
            // Excel's formula limit) into a list, so the walk recurses once per right operand
            // and not once per operator. A range operator with a computed end stays a node of
            // its own (the arm above).
            let mut rights = Vec::new();
            let mut node = e;
            while let Expr::Binary(op, a, b) = node
                && !(*op == BinOp::Range && !matches!((&**a, &**b), (Expr::Ref(_), Expr::Ref(_))))
            {
                rights.push(&**b);
                node = a;
            }
            // The spine's bottom sits where the parser parsed it, at this level: the nesting
            // counted here then matches the parser's on every path.
            collect_precedents(wb, sheet, node, bound, out, dynamic, depth, nest, top, names);
            for b in rights.into_iter().rev() {
                collect_precedents(wb, sheet, b, bound, out, dynamic, depth, nest + 1, top, names);
            }
        }
        Expr::Invoke(..) => {
            // Invoking a result, perhaps again (`f(x)(y)`): the parser wraps the callee without a
            // level of its own and parses each argument list one deeper.
            let mut lists = Vec::new();
            let mut node = e;
            while let Expr::Invoke(f, args) = node {
                lists.push(args);
                node = f;
            }
            collect_precedents(wb, sheet, node, bound, out, dynamic, depth, nest, top, names);
            for args in lists.into_iter().rev() {
                for a in args {
                    collect_precedents(wb, sheet, a, bound, out, dynamic, depth, nest + 1, top, names);
                }
            }
        }
        Expr::Call(n, args) => {
            // The cells SUMIF/AVERAGEIF really read from the sum range (see `resized_sum_range`);
            // when the criteria range's size isn't known statically, recalculate every time.
            if resizes_sum_range(n, args.len()) {
                match (args.first(), args.get(2)) {
                    (Some(Expr::Ref(c)), Some(Expr::Ref(s))) => {
                        let (crit, sum) = (c.range(), s.range());
                        push_area(wb, sheet, out, s, resized_sum_range(sum, crit.height(), crit.width()));
                    }
                    (_, Some(Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) | Expr::Error(_) | Expr::Array(_) | Expr::Missing)) => {}
                    _ => *dynamic = true,
                }
            }
            if matches!(n.as_str(), "INDIRECT" | "OFFSET" | "NOW" | "TODAY" | "RAND" | "RANDBETWEEN" | "RANDARRAY" | "CELL" | "INFO") {
                *dynamic = true;
            }
            match n.as_str() {
                // Parameters are bound inside the body only; the body is the last argument.
                "LAMBDA" if !args.is_empty() => {
                    let mut added = 0;
                    for p in &args[..args.len() - 1] {
                        if let Expr::Name(pn) = p {
                            bound.push(bound_name(pn));
                            added += 1;
                        }
                    }
                    if let Some(body) = args.last() {
                        collect_precedents(wb, sheet, body, bound, out, dynamic, depth, nest + 1, top, names);
                    }
                    bound.truncate(bound.len().saturating_sub(added));
                }
                // Each LET variable is bound for the values after it and for the result.
                "LET" if args.len() >= 3 => {
                    let mut added = 0;
                    for pair in args[..args.len() - 1].chunks(2) {
                        if let [Expr::Name(vn), value] = pair {
                            collect_precedents(wb, sheet, value, bound, out, dynamic, depth, nest + 1, top, names);
                            bound.push(bound_name(vn));
                            added += 1;
                        } else if let [other] = pair {
                            collect_precedents(wb, sheet, other, bound, out, dynamic, depth, nest + 1, top, names);
                        }
                    }
                    if let Some(result) = args.last() {
                        collect_precedents(wb, sheet, result, bound, out, dynamic, depth, nest + 1, top, names);
                    }
                    bound.truncate(bound.len().saturating_sub(added));
                }
                _ => {
                    for a in args {
                        collect_precedents(wb, sheet, a, bound, out, dynamic, depth, nest + 1, top, names);
                    }
                    // Not a built-in: a defined name holding a LAMBDA (its body's references are
                    // this cell's precedents too), or unknown (#NAME?, evaluated every time).
                    if gridcraft_functions::lookup(n).is_none() && !is_special(n) && !bound.contains(&bound_name(n)) {
                        name_precedents(wb, sheet, n, out, dynamic, depth, nest, top, names);
                    }
                }
            }
        }
        Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) | Expr::Error(_) | Expr::Missing => {}
    }
}

fn is_special(n: &str) -> bool {
    matches!(
        n,
        "IF" | "IFS"
            | "IFERROR"
            | "IFNA"
            | "CHOOSE"
            | "SWITCH"
            | "ROW"
            | "COLUMN"
            | "ROWS"
            | "COLUMNS"
            | "AREAS"
            | "ISREF"
            | "ISFORMULA"
            | "FORMULATEXT"
            | "SHEET"
            | "SHEETS"
            | "OFFSET"
            | "INDIRECT"
            | "INDEX"
            | "CELL"
            | "TRIMRANGE"
            | "SUBTOTAL"
            | "AGGREGATE"
            | "TEXT"
            | "LET"
            | "LAMBDA"
            | "ISOMITTED"
            | "MAP"
            | "REDUCE"
            | "SCAN"
            | "BYROW"
            | "BYCOL"
            | "MAKEARRAY"
    )
}

/// Names of functions the evaluator implements itself (merged into the function list for UI).
pub const SPECIAL_FUNCTIONS: &[&str] = &[
    "IF",
    "IFS",
    "IFERROR",
    "IFNA",
    "CHOOSE",
    "SWITCH",
    "ROW",
    "COLUMN",
    "ROWS",
    "COLUMNS",
    "AREAS",
    "ISREF",
    "ISFORMULA",
    "FORMULATEXT",
    "SHEET",
    "SHEETS",
    "OFFSET",
    "INDIRECT",
    "INDEX",
    "CELL",
    "TRIMRANGE",
    "SUBTOTAL",
    "AGGREGATE",
    "TEXT",
    "LET",
    "LAMBDA",
    "ISOMITTED",
    "MAP",
    "REDUCE",
    "SCAN",
    "BYROW",
    "BYCOL",
    "MAKEARRAY",
];

pub fn is_known_function(n: &str) -> bool {
    is_special(&n.to_ascii_uppercase()) || gridcraft_functions::lookup(n).is_some()
}

#[allow(dead_code)]
fn unused(_: RefKind) {}
