//! Legacy implicit intersection.
//!
//! A formula read from a file that predates dynamic arrays (no `cm` cell metadata, not a legacy
//! CSE array) is calculated by Excel with *implicit intersection*: where the formula expects one
//! value, a multi-cell reference is reduced to the cell on the formula's row or column, and an
//! array value to its top-left element. Excel 365 applies the same rule, and shows the
//! intersection as `@`.
//!
//! [`scalarize`] rewrites the parsed formula the same way: every reference in a scalar position
//! is wrapped in `@`, which the evaluator already understands. The calculation engine uses it
//! for formulas marked [`Formula::implicit`](gridcraft_model::Formula::implicit); the formula
//! text, its references and the saved file stay unchanged.
//!
//! Rules, from Microsoft's *Implicit intersection operator: @*:
//! - a reference in a scalar position is intersected: `=A1:A10` is `=@A1:A10`, and so is
//!   `=INDEX(A1:A10,B1)`, a call that may return a range;
//! - a reference that is the whole argument of a function taking ranges or arrays is not
//!   intersected: `SUM(A1:A10)` is unchanged, and so is `SUM(INDEX(A1:A10,B1))`;
//! - an expression inside such an argument is a scalar position: `SUM(A1:A3*B1:B3)` intersects
//!   both references, as `@A1:A3*@B1:B3` (older Excel could not mix the two modes);
//! - `SUMPRODUCT`, `MMULT`, `TRANSPOSE` and the other functions that always took arrays keep
//!   their arguments in array mode, and `IF`/`IFERROR`/`CHOOSE`/`SWITCH` pass the position on,
//!   so a legacy formula can still contain an array computation there;
//! - an explicit `@` (read from `_xlfn.SINGLE`) is left as it is.

use gridcraft_formula::{BinOp, Expr, UnOp};

/// Functions that always evaluate their arguments as arrays in a legacy formula (they never
/// needed Ctrl+Shift+Enter).
const ARRAY_FUNCTIONS: &[&str] = &["SUMPRODUCT", "MMULT", "MINVERSE", "MDETERM", "TRANSPOSE", "FREQUENCY", "LINEST", "LOGEST", "TREND", "GROWTH"];

/// Functions that return one of their arguments: they keep the position they are used in.
const PASS_THROUGH: &[&str] = &["IF", "IFS", "IFERROR", "IFNA", "CHOOSE", "SWITCH"];

/// Functions that may return a range or an array, so a scalar position intersects their result.
/// Any other function returns a single value (a scalar function, or one that aggregates).
const RANGE_FUNCTIONS: &[&str] = &["INDEX", "OFFSET", "INDIRECT", "CHOOSE"];

/// Deepest nesting the rewrite follows; a formula may nest about 4,000 levels (the 8,192
/// character limit), so this only stops hostile input, which is then left as it is.
const MAX_DEPTH: usize = 512;

/// The formula rewritten for legacy implicit intersection: the same expression with `@` around
/// every reference used where Excel expects one value.
pub fn scalarize(e: &Expr) -> Expr {
    scalarize_in(e, Position::Scalar, 0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    /// One value is expected: references are intersected.
    Scalar,
    /// An array or range is expected: references keep their shape.
    Array,
}

fn scalarize_in(e: &Expr, at: Position, depth: usize) -> Expr {
    if depth >= MAX_DEPTH {
        return e.clone();
    }
    let d = depth + 1;
    match e {
        Expr::Ref(_) | Expr::Name(_) | Expr::Struct(_) => match at {
            Position::Array => e.clone(),
            Position::Scalar => Expr::Unary(UnOp::At, Box::new(e.clone())),
        },
        Expr::Paren(x) => Expr::Paren(Box::new(scalarize_in(x, at, d))),
        // An array constant's items are values, not references in the formula's position.
        Expr::Array(rows) => Expr::Array(rows.iter().map(|r| r.iter().map(|x| scalarize_in(x, Position::Array, d)).collect()).collect()),
        // An explicit `@` already does the intersection: its operand keeps its shape.
        Expr::Unary(UnOp::At, x) => Expr::Unary(UnOp::At, Box::new(scalarize_in(x, Position::Array, d))),
        Expr::Unary(op, x) => {
            // `A1#` and trim references are reference operators: their operand stays a range.
            let inner = if matches!(op, UnOp::Spill | UnOp::Trim(_)) { Position::Array } else { at };
            Expr::Unary(*op, Box::new(scalarize_in(x, inner, d)))
        }
        Expr::Binary(op, a, b) => binary(*op, a, b, at, d),
        Expr::Call(name, args) => call(name, args, at, d),
        Expr::Invoke(callee, args) => Expr::Invoke(Box::new(scalarize_in(callee, at, d)), args.iter().map(|a| scalarize_in(a, at, d)).collect()),
        other => other.clone(),
    }
}

fn binary(op: BinOp, a: &Expr, b: &Expr, at: Position, depth: usize) -> Expr {
    if matches!(op, BinOp::Range | BinOp::Union | BinOp::Intersect) {
        // A reference: both ends stay references, and the result is intersected if one value
        // is expected (`=A1:B3`, `=A1 A2:B3`).
        let node = Expr::Binary(op, Box::new(scalarize_in(a, Position::Array, depth)), Box::new(scalarize_in(b, Position::Array, depth)));
        return match at {
            Position::Array => node,
            Position::Scalar => Expr::Unary(UnOp::At, Box::new(node)),
        };
    }
    // A flat chain nests on its left, so follow that spine in a loop rather than one call per
    // operator (a formula as long as Excel allows is about 4,000 levels deep).
    let mut spine = vec![(op, b)];
    let mut left = a;
    while let Expr::Binary(op, l, r) = left {
        if matches!(op, BinOp::Range | BinOp::Union | BinOp::Intersect) {
            break;
        }
        spine.push((*op, r.as_ref()));
        left = l.as_ref();
    }
    let mut acc = scalarize_in(left, at, depth);
    while let Some((op, right)) = spine.pop() {
        acc = Expr::Binary(op, Box::new(acc), Box::new(scalarize_in(right, at, depth)));
    }
    acc
}

fn call(name: &str, args: &[Expr], at: Position, depth: usize) -> Expr {
    let rewritten: Vec<Expr> = args.iter().enumerate().map(|(i, a)| scalarize_in(a, argument_position(name, i, a, at), depth)).collect();
    let node = Expr::Call(name.to_string(), rewritten);
    // Where one value is expected, the result is intersected: a call that returns a range
    // (`INDEX`, `OFFSET`, `INDIRECT`, a user function) gives the cell on the formula's row or
    // column, and one that returns an array (`ROW(A1:A3)`) its top-left value. A call that
    // returns one value is unaffected by `@`.
    match at {
        Position::Scalar => Expr::Unary(UnOp::At, Box::new(node)),
        Position::Array => node,
    }
}

/// The position argument `i` of `name` is in: an array where the function takes ranges or
/// arrays as values, one value everywhere else.
fn argument_position(name: &str, i: usize, arg: &Expr, at: Position) -> Position {
    if ARRAY_FUNCTIONS.iter().any(|f| f.eq_ignore_ascii_case(name)) {
        return Position::Array;
    }
    if PASS_THROUGH.iter().any(|f| f.eq_ignore_ascii_case(name)) {
        return at;
    }
    match gridcraft_functions::lookup(name) {
        // A value parameter rejects a multi-cell reference with #VALUE! instead of intersecting
        // it, in legacy formulas too: its references keep their shape so the function can.
        Some(spec) if spec.is_value_arg(i) => Position::Array,
        // A whole reference (or array constant) is the function's argument: `SUM(A1:A3)` keeps
        // its three cells. Any other expression is calculated as one value: `SUM(A1:A3*B1:B3)`.
        Some(spec) if !spec.is_scalar_arg(i) => {
            if whole_argument(arg) {
                Position::Array
            } else {
                Position::Scalar
            }
        }
        // A single-value parameter is one value, which an old formula intersects.
        Some(_) => Position::Scalar,
        // A function GridCraft does not know: its arguments may be anything, so leave them.
        None => Position::Array,
    }
}

/// Whether `e` is a reference or an array constant in itself, rather than a calculation over
/// one: a range argument used whole by a function that takes ranges.
fn whole_argument(e: &Expr) -> bool {
    match e {
        Expr::Ref(_) | Expr::Name(_) | Expr::Struct(_) | Expr::Array(_) => true,
        Expr::Paren(x) => whole_argument(x),
        Expr::Unary(UnOp::Spill, _) => true,
        Expr::Binary(op, a, b) => matches!(op, BinOp::Range | BinOp::Union | BinOp::Intersect) && whole_argument(a) && whole_argument(b),
        Expr::Call(name, _) => RANGE_FUNCTIONS.iter().any(|f| f.eq_ignore_ascii_case(name)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_formula::print;

    use super::scalarize;

    /// The formula after the rewrite, as a person would type it.
    fn rewritten(text: &str) -> String {
        let e = gridcraft_formula::parse(text).expect("test formula parses");
        print(&scalarize(&e))
    }

    #[test]
    fn references_used_as_one_value_are_intersected() {
        assert_eq!(rewritten("A1:A10"), "@A1:A10");
        assert_eq!(rewritten("A1:A10*2"), "@A1:A10*2");
        assert_eq!(rewritten("A1:A10&B1"), "@A1:A10&@B1");
        assert_eq!(rewritten("IF(Pay_Num<>\"\",Beg_Bal*Rate/12,\"\")"), "@IF(@Pay_Num<>\"\",@Beg_Bal*@Rate/12,\"\")");
        assert_eq!(rewritten("SQRT(A1:A3)"), "@SQRT(@A1:A3)");
        assert_eq!(rewritten("Sheet2!A1:B3"), "@Sheet2!A1:B3");
        assert_eq!(rewritten("Data2[[#This Row],[Input]]*2"), "@Data2[@Input]*2");
    }

    #[test]
    fn a_reference_used_whole_is_not_intersected() {
        assert_eq!(rewritten("SUM(A1:A10)"), "@SUM(A1:A10)");
        assert_eq!(rewritten("AVERAGE((A1:A10))"), "@AVERAGE((A1:A10))");
        assert_eq!(rewritten("SUM(Sheet2!A1:B3)"), "@SUM(Sheet2!A1:B3)");
        assert_eq!(rewritten("SUM(Pay_Num)"), "@SUM(Pay_Num)");
        assert_eq!(rewritten("SUM({1,2,3})"), "@SUM({1,2,3})");
        assert_eq!(rewritten("SUM(INDEX(A1:B3,0,1))"), "@SUM(INDEX(A1:B3,0,1))");
        assert_eq!(rewritten("COUNTIF(A1:A10,\">0\")"), "@COUNTIF(A1:A10,\">0\")");
    }

    #[test]
    fn an_expression_in_a_range_argument_is_one_value() {
        assert_eq!(rewritten("SUM(A1:A3*B1:B3)"), "@SUM(@A1:A3*@B1:B3)");
        assert_eq!(rewritten("SUM(A1:A3+0)"), "@SUM(@A1:A3+0)");
    }

    #[test]
    fn array_functions_and_pass_through_keep_array_positions() {
        assert_eq!(rewritten("SUMPRODUCT(A1:A3*B1:B3)"), "@SUMPRODUCT(A1:A3*B1:B3)");
        assert_eq!(rewritten("SUMPRODUCT(--(A1:A3=\"x\"))"), "@SUMPRODUCT(--(A1:A3=\"x\"))");
        assert_eq!(rewritten("MMULT(A1:B3,D1:E2)"), "@MMULT(A1:B3,D1:E2)");
        assert_eq!(rewritten("IF(A1:A3>0,B1:B3,0)"), "@IF(@A1:A3>0,@B1:B3,0)");
        assert_eq!(rewritten("SUMPRODUCT(IF(A1:A3>0,B1:B3,0))"), "@SUMPRODUCT(IF(A1:A3>0,B1:B3,0))");
        assert_eq!(rewritten("IFERROR(ROW(A1:A3),0)"), "@IFERROR(@ROW(A1:A3),0)");
    }

    #[test]
    fn calls_that_return_a_range_are_intersected() {
        assert_eq!(rewritten("INDEX(A1:A10,B1)"), "@INDEX(A1:A10,@B1)");
        assert_eq!(rewritten("INDEX(A1:A10,B1)*2"), "@INDEX(A1:A10,@B1)*2");
        assert_eq!(rewritten("OFFSET(A1,0,0,3,1)"), "@OFFSET(A1,0,0,3,1)");
        assert_eq!(rewritten("MYUDF(A1)"), "@MYUDF(A1)");
    }

    #[test]
    fn value_parameters_keep_their_reference_so_it_is_rejected() {
        // `EOMONTH` and its peers give #VALUE! for a multi-cell reference in a legacy formula
        // too, instead of intersecting it (`functions::lift`), so the reference stays whole.
        assert_eq!(rewritten("EOMONTH(A1:A3,0)"), "@EOMONTH(A1:A3,0)");
        assert_eq!(rewritten("EOMONTH(INDEX(A1:A3,0,1),0)"), "@EOMONTH(INDEX(A1:A3,0,1),0)");
        assert_eq!(rewritten("ISEVEN(B1:B3)"), "@ISEVEN(B1:B3)");
        assert_eq!(rewritten("SUM(EOMONTH(A1:A3,0))"), "@SUM(@EOMONTH(A1:A3,0))");
    }

    #[test]
    fn scalar_functions_are_not_intersected() {
        assert_eq!(rewritten("SUM(A1:A3)+1"), "@SUM(A1:A3)+1");
        assert_eq!(rewritten("ROUND(A1*2,0)"), "@ROUND(@A1*2,0)");
    }

    #[test]
    fn an_explicit_intersection_is_kept() {
        assert_eq!(rewritten("@A1:A3*2"), "@A1:A3*2");
        assert_eq!(rewritten("SUM(@A1:A3)"), "@SUM(@A1:A3)");
        assert_eq!(rewritten("A1#"), "A1#");
    }

    #[test]
    fn a_long_chain_does_not_recurse_per_operator() {
        let text: String = (1..=1_000).map(|i| format!("+A{i}")).collect();
        let e = gridcraft_formula::parse(&format!("1{text}")).expect("parses");
        let out = scalarize(&e);
        // Every reference is intersected, and the chain is still one flat formula.
        assert_eq!(gridcraft_formula::print(&out), format!("1{}", (1..=1_000).map(|i| format!("+@A{i}")).collect::<String>()));
    }
}
