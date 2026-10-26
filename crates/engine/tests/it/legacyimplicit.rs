//! A plain formula from a file that predates dynamic arrays is calculated with implicit
//! intersection; edits that only move or adjust it keep that, so it never starts spilling.

use std::sync::Arc;

use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::{Cell, Formula, Workbook};
use gridcraft_engine::{DocState, Session, io};
use serde_json::json;

fn at(s: &str) -> CellRef {
    CellRef::parse(s).unwrap()
}

/// A workbook whose D2 formula stands in for one read from a legacy file (the save below takes
/// the plain-formula path the writer uses for those).
fn opened() -> Session {
    let mut wb = Workbook::new();
    let sh = wb.sheet_mut(0).unwrap();
    for (r, (a, b)) in [(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)].into_iter().enumerate() {
        sh.set_value(CellRef::new(r as u32, 0), Value::Number(a));
        sh.set_value(CellRef::new(r as u32, 1), Value::Number(b));
    }
    let mut f = Formula::new("Sheet1!A1:A3*Sheet1!B1:B3");
    f.implicit = true;
    sh.cells.set(at("D2"), Cell::formula(f));
    let bytes = io::save_bytes(&wb, "legacy.xlsx").unwrap();
    let (back, _) = io::open_bytes("legacy.xlsx", &bytes).unwrap();
    let mut s = Session::new();
    s.add_document(DocState::new(back, None, "legacy".into(), gridcraft_engine::Trust::FromFile));
    s
}

fn formula(s: &Session, cell: &str) -> Arc<Formula> {
    s.doc().unwrap().wb.sheet(0).unwrap().cell(at(cell)).unwrap().formula.clone().unwrap()
}

fn value(s: &Session, cell: &str) -> Value {
    s.doc().unwrap().wb.sheet(0).unwrap().value(at(cell))
}

/// The formula as a fresh open of the saved file reads it: the file's metadata decides whether
/// it is a legacy formula or a dynamic array.
fn saved(s: &Session, cell: &str) -> Arc<Formula> {
    let bytes = io::save_bytes(&s.doc().unwrap().wb, "legacy.xlsx").unwrap();
    let (back, _) = io::open_bytes("legacy.xlsx", &bytes).unwrap();
    back.sheet(0).unwrap().cell(at(cell)).unwrap().formula.clone().unwrap()
}

#[test]
fn opened_legacy_formula_intersects_and_stays_plain() {
    let s = opened();
    let f = formula(&s, "D2");
    assert!(f.implicit && !f.dynamic && f.array.is_none(), "the file's plain formula is legacy");
    // Row 2: A2 * B2, and nothing spills into the empty cells below.
    assert_eq!(value(&s, "D2"), Value::Number(40.0));
    assert_eq!(value(&s, "D3"), Value::Empty);
    let back = saved(&s, "D2");
    assert!(back.implicit && !back.dynamic, "saving must not turn it into a dynamic array");
}

#[test]
fn inserting_rows_and_moving_keep_implicit_intersection() {
    let mut s = opened();
    s.execute("home.insertRows", json!({"rows": "1:1"})).unwrap();
    let f = formula(&s, "D3");
    assert!(f.implicit, "a structural edit only moves the formula");
    assert_eq!(f.text, "Sheet1!A2:A4*Sheet1!B2:B4");
    assert_eq!(value(&s, "D3"), Value::Number(40.0));
    assert!(saved(&s, "D3").implicit);

    s.execute("selection.set", json!({"range": "D3"})).unwrap();
    s.execute("edit.cut", json!({})).unwrap();
    s.execute("selection.set", json!({"range": "F3"})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    assert!(formula(&s, "F3").implicit, "a cut keeps the formula");
    assert_eq!(value(&s, "F3"), Value::Number(40.0));
    assert!(saved(&s, "F3").implicit);
}

#[test]
fn renaming_the_sheet_keeps_implicit_intersection() {
    let mut s = opened();
    s.execute("sheet.rename", json!({"sheet": 0, "name": "Data"})).unwrap();
    let f = formula(&s, "D2");
    assert!(f.implicit);
    assert_eq!(f.text, "Data!A1:A3*Data!B1:B3");
    assert_eq!(value(&s, "D2"), Value::Number(40.0));
    assert!(saved(&s, "D2").implicit);
}

#[test]
fn a_retyped_formula_is_a_dynamic_array_again() {
    let mut s = opened();
    s.execute("cell.set", json!({"cell": "D2", "input": "=Sheet1!A1:A3*Sheet1!B1:B3"})).unwrap();
    assert!(!formula(&s, "D2").implicit, "typing a formula makes it dynamic");
    assert_eq!((value(&s, "D2"), value(&s, "D3"), value(&s, "D4")), (Value::Number(10.0), Value::Number(40.0), Value::Number(90.0)));
    let back = saved(&s, "D2");
    assert!(back.dynamic && !back.implicit);
}
