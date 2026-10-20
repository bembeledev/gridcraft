//! #1967: entering a bare number into a percentage-formatted cell divides by 100,
//! the way Excel does. A typed `%` is already self-describing and stays as-is.

use gridcraft_engine::Session;
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn value(s: &mut Session, cell: &str) -> f64 {
    s.execute("cell.get", json!({"cell": cell})).unwrap()["value"].as_f64().unwrap_or(f64::NAN)
}

#[test]
fn bare_numbers_divide_by_100_in_percent_cells() {
    let mut s = session();
    s.execute("home.numberFormat", json!({"range": "A1:A6", "code": "0%"})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "3"})).unwrap();
    s.execute("cell.set", json!({"cell": "A2", "input": "0.03"})).unwrap();
    s.execute("cell.set", json!({"cell": "A3", "input": "-4"})).unwrap();
    s.execute("cell.set", json!({"cell": "A4", "input": "12.5"})).unwrap();
    assert_eq!(value(&mut s, "A1"), 0.03, "3 -> 3%");
    assert_eq!(value(&mut s, "A2"), 0.0003, "0.03 -> 0.03%");
    assert_eq!(value(&mut s, "A3"), -0.04, "-4 -> -4%");
    assert_eq!(value(&mut s, "A4"), 0.125, "12.5 -> 12.5%");
}

#[test]
fn typed_percent_and_non_numbers_are_untouched() {
    let mut s = session();
    s.execute("home.numberFormat", json!({"range": "B1:B5", "code": "0%"})).unwrap();
    // explicit % already carries the scale
    s.execute("cell.set", json!({"cell": "B1", "input": "3%"})).unwrap();
    assert_eq!(value(&mut s, "B1"), 0.03, "3% stays 3%");
    // text and formulas are not numbers
    s.execute("cell.set", json!({"cell": "B2", "input": "hello"})).unwrap();
    s.execute("cell.set", json!({"cell": "B3", "input": "=2"})).unwrap();
    assert_eq!(value(&mut s, "B3"), 2.0, "formula result is not rescaled");
    // General-formatted cells never divide
    s.execute("cell.set", json!({"cell": "C1", "input": "3"})).unwrap();
    assert_eq!(value(&mut s, "C1"), 3.0, "General format is untouched");
    // a percent format entered in the same commit (typing "3%" into General) is not double-divided
    s.execute("cell.set", json!({"cell": "C2", "input": "5%"})).unwrap();
    assert_eq!(value(&mut s, "C2"), 0.05, "typing 5% into a General cell gives 5% once");
}

#[test]
fn scenario_replay_does_not_rescale_numbers() {
    // #1967 review: scenario values are stored numbers; showing a scenario must write them
    // as-is, not re-run them through the typed-entry rule (which would divide by 100 again).
    let mut s = session();
    s.execute("home.numberFormat", json!({"range": "A1", "code": "0%"})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "5"})).unwrap();
    assert_eq!(value(&mut s, "A1"), 0.05, "typed 5 -> 5%");
    s.execute("data.scenarioAdd", json!({"name": "Base", "changing": "A1"})).unwrap();
    s.execute("cell.set", json!({"cell": "A1", "input": "9"})).unwrap();
    s.execute("data.scenarioShow", json!({"name": "Base"})).unwrap();
    assert_eq!(value(&mut s, "A1"), 0.05, "scenario restores the stored 0.05, not 0.0005");
}
