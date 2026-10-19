//! Malformed conditional-format formulas must be refused before they reach a saved workbook (#1999).

use gridcraft_engine::Session;
use serde_json::{Value, json};

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("home.conditionalFormat", json!({"range": "A1:A3", "rule": {"type": "expression", "formula": "A1>5"}})).unwrap();
    // A rejected rule must preserve both existing rules and a pending redo.
    s.execute("cell.set", json!({"cell": "B1", "input": "7"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    s
}

fn rejects_without_changes(rule: Value) {
    let mut s = session();
    let before = serde_json::to_value(&s.doc().unwrap().wb).unwrap();
    let history = (s.doc().unwrap().undo.len(), s.doc().unwrap().redo.len());
    let error = s.execute("home.conditionalFormat", json!({"range": "A1:A3", "rule": rule})).expect_err("malformed formula was accepted");
    assert!(error.to_string().contains("formula"), "{error}");
    assert_eq!(serde_json::to_value(&s.doc().unwrap().wb).unwrap(), before);
    assert_eq!((s.doc().unwrap().undo.len(), s.doc().unwrap().redo.len()), history);
}

#[test]
fn invalid_expression_is_refused_without_changing_rules_or_history() {
    for formula in ["=A1>", "=1+", "", "   "] {
        rejects_without_changes(json!({"type": "expression", "formula": formula}));
    }
}

#[test]
fn invalid_cell_comparison_bounds_are_refused() {
    rejects_without_changes(json!({"type": "cellIs", "operator": "greaterThan", "value": "=1+"}));
    rejects_without_changes(json!({"type": "cellIs", "operator": "between", "value": 1, "value2": "=A1>"}));
}

#[test]
fn invalid_data_bar_formula_bounds_are_refused() {
    for bound in ["min", "max"] {
        let mut rule = json!({"type": "dataBar"});
        rule[bound] = json!({"type": "formula", "value": "1+"});
        rejects_without_changes(rule);
    }
}

#[test]
fn valid_formulas_and_literal_comparisons_still_save_and_reopen() {
    let mut s = Session::new();
    s.new_workbook();
    for rule in [
        json!({"type": "expression", "formula": "A1>5"}),
        json!({"type": "expression", "formula": "=AND(A1>0,A1<10)"}),
        json!({"type": "expression", "formula": "=FutureFunction(A1)"}),
        json!({"type": "expression", "formula": "=#REF!"}),
        json!({"type": "cellIs", "operator": "between", "value": "=$Z$1", "value2": "=MAX($Z$2,10)"}),
        json!({"type": "cellIs", "operator": "equal", "value": "1+"}),
        json!({"type": "cellIs", "operator": "equal", "value": 5}),
        json!({"type": "dataBar", "min": {"type": "formula", "value": "$Z$1"}, "max": {"type": "formula", "value": "MAX($Z$2,10)"}}),
    ] {
        s.execute("home.conditionalFormat", json!({"range": "A1:A3", "rule": rule})).unwrap();
    }
    let formulas = |s: &Session| {
        use gridcraft_engine::model::{CfRule, CfValueKind};
        s.doc()
            .unwrap()
            .wb
            .active()
            .unwrap()
            .cond_formats
            .iter()
            .map(|cf| {
                let values = match &cf.rule {
                    CfRule::Expression { formula, .. } => vec![formula.as_str()],
                    CfRule::CellIs { a, b, .. } => std::iter::once(a.as_str()).chain(b.as_deref()).collect(),
                    CfRule::DataBar { min: CfValueKind::Formula(a), max: CfValueKind::Formula(b), .. } => vec![a.as_str(), b.as_str()],
                    _ => panic!("unexpected rule"),
                };
                values.into_iter().map(|v| gridcraft_formula::print(&gridcraft_formula::parse(v).unwrap())).collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let expected = formulas(&s);
    let saved = s.execute("file.saveBytes", json!({"format": "xlsx"})).unwrap();
    let mut reopened = Session::new();
    reopened.execute("file.open", json!({"name": "rules.xlsx", "base64": saved["base64"]})).unwrap();
    assert_eq!(formulas(&reopened), expected);
}
