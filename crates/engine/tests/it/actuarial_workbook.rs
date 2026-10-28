//! The shipped command-script template is exercised through recalculation, XLSX and MC.
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use serde_json::json;

fn template() -> Session {
    replay(include_str!("../../../../docs/examples/actuarial.jsonl"))
}

fn replay(script: &str) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    for line in script.lines() {
        let command: serde_json::Value = serde_json::from_str(line).unwrap();
        s.execute(command["command"].as_str().unwrap(), command["params"].clone()).unwrap();
    }
    s
}
fn numeric(s: &Session, sheet: &str, cell: &str) -> f64 {
    let sh = s.doc().unwrap().wb.sheets.iter().find(|s| s.name == sheet).unwrap();
    match sh.value(CellRef::parse(cell).unwrap()) {
        Value::Number(x) => x,
        other => panic!("{sheet}!{cell}: expected numeric, got {other:?}"),
    }
}
fn close(s: &Session, sheet: &str, cell: &str, expected: f64) {
    let actual = numeric(s, sheet, cell);
    assert!((actual - expected).abs() < 1e-9 * (1.0 + expected.abs()), "{sheet}!{cell}: {actual} != {expected}");
}
fn check(s: &Session) {
    close(s, "Loss", "B6", 10_000.0);
    close(s, "Loss", "B7", 20_000_000.0);
    close(s, "Loss", "E6", 0.0);
    close(s, "Loss", "E7", 100.0);
    close(s, "Loss", "E15", 500.0);
    close(s, "Life", "B11", 0.0);
    close(s, "Reserves", "G8", 148.0);
    close(s, "Reserves", "G9", 148.0);
    close(s, "Markov", "G8", 1.0);
    close(s, "Markov", "G9", 1.0);
    close(s, "Markov", "F14", (-0.2_f64).exp());
    close(s, "Pricing", "F3", 2.0_f64.ln());
    close(s, "Pricing", "F4", 2.0_f64.ln());
    close(s, "Pricing", "H3", 2.0);
    close(s, "Pricing", "H6", 4.0);
    close(s, "Pricing", "B9", 0.0);
    // No worksheet error is permitted anywhere in the teaching template, including spills.
    for sh in &s.doc().unwrap().wb.sheets {
        for (cell, data) in sh.cells.iter() {
            assert!(!matches!(data.value, Value::Error(_)), "{}!{cell:?}: {:?}", sh.name, data.value);
        }
    }
}
#[test]
fn actuarial_template_calculates_and_roundtrips_xlsx() {
    let mut s = template();
    check(&s);
    let saved = s.execute("file.saveBytes", json!({"format":"xlsx"})).unwrap();
    let mut reopened = Session::new();
    reopened.execute("file.open", json!({"name":"actuarial.xlsx","base64":saved["base64"]})).unwrap();
    check(&reopened);
    reopened.execute("cell.set", json!({"sheet":"Loss","cell":"B2","input":"12"})).unwrap();
    close(&reopened, "Loss", "B6", 12_000.0);
    reopened.execute("edit.undo", json!({})).unwrap();
    close(&reopened, "Loss", "B6", 10_000.0);
}
#[test]
fn actuarial_formula_models_use_existing_seeded_monte_carlo_without_changing_sources() {
    let mut s = template();
    let report = s
        .execute(
            "data.monteCarlo",
            json!({
                "inputs":[{"cell":"Loss!B2","distribution":"uniform","min":8,"max":12}],
                "outputs":["Loss!B6","Loss!B10"],"iterations":32,"seed":42,
                "method":"latinHypercube","replicates":4,"samples":true,"newSheet":"Actuarial uncertainty"
            }),
        )
        .unwrap();
    assert!(!report.is_null());
    close(&s, "Loss", "B2", 10.0);
    close(&s, "Loss", "B6", 10_000.0);
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 6);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.sheets.len(), 5);
    check(&s);
}

#[test]
fn scientific_template_has_calibrated_models_and_validated_predictions_after_xlsx_reopen() {
    fn check_science(s: &Session) {
        close(s, "Inference", "F16", 1.2);
        close(s, "Inference", "G16", 0.24);
        close(s, "Inference", "K16", 1.44);
        close(s, "Inference", "I3", 0.75);
        close(s, "Inference", "I4", 0.5);
        close(s, "Inference", "J12", 0.75);
        close(s, "Inference", "K12", 0.25);
        close(s, "Inference", "E26", 0.0);
        close(s, "Inference", "G26", 2.0 / 3.0);
        close(s, "Extremes", "E14", 100.0 + 20.0 * 10.0_f64.ln());
        close(s, "Extremes", "E15", 120.0 + 20.0 * 10.0_f64.ln());
        close(s, "Decision", "D19", 5.0);
        close(s, "Decision", "D20", 2.5);
        close(s, "Decision", "E28", 200.0);
        close(s, "Decision", "H28", 19.0);
        close(s, "Proxy", "M3", 4.75);
        close(s, "Proxy", "M6", 10.75);
        close(s, "Duration", "D12", 0.4);
        close(s, "Duration", "E12", 0.6);
        close(s, "Bootstrap", "H3", 940.0);
        close(s, "Bootstrap", "M3", 110.0 / 7.0);
        for sh in &s.doc().unwrap().wb.sheets {
            for (cell, data) in sh.cells.iter() {
                assert!(!matches!(data.value, Value::Error(_)), "{}!{cell:?}: {:?}", sh.name, data.value);
            }
        }
    }
    let mut s = replay(include_str!("../../../../docs/examples/actuarial-scientific.jsonl"));
    check_science(&s);
    let saved = s.execute("file.saveBytes", json!({"format":"xlsx"})).unwrap();
    let mut reopened = Session::new();
    reopened.execute("file.open", json!({"name":"scientific-actuarial.xlsx","base64":saved["base64"]})).unwrap();
    check_science(&reopened);
}

#[test]
fn independent_scientific_reference_oracles_are_preserved_at_engine_boundary() {
    // These expectations come from independent closed-form/Decimal oracles, not from
    // recorded outputs of this engine. Tiny nonzero tails must never pass as false zero.
    fn compare(actual: &serde_json::Value, expected: &serde_json::Value, relative: f64, label: &str) {
        if let Some(expected_number) = expected.as_f64() {
            let actual_number = actual.as_f64().unwrap_or_else(|| panic!("{label}: {actual:?}"));
            let tolerance = if expected_number == 0.0 { 1e-12 } else { relative * expected_number.abs() };
            assert!((actual_number - expected_number).abs() <= tolerance, "{label}: {actual_number} != {expected_number}, tol {tolerance}");
        } else if let Some(expected_array) = expected.as_array() {
            let actual_array = actual.as_array().unwrap_or_else(|| panic!("{label}: {actual:?}"));
            assert_eq!(actual_array.len(), expected_array.len(), "{label}");
            for (actual, expected) in actual_array.iter().zip(expected_array) {
                compare(actual, expected, relative, label);
            }
        } else {
            assert_eq!(actual, expected, "{label}");
        }
    }
    let data: serde_json::Value = serde_json::from_str(include_str!("../fixtures/actuarial-reference.json")).unwrap();
    let mut s = Session::new();
    s.new_workbook();
    for case in data["cases"].as_array().unwrap() {
        let actual = s.execute("formulas.evaluate", json!({"formula":case["formula"]})).unwrap();
        compare(&actual, &case["expected"], case["relative_tolerance"].as_f64().unwrap(), case["label"].as_str().unwrap());
    }
}
