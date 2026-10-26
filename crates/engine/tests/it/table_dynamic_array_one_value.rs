//! A dynamic-array formula in a table column that returns one value shows it (#2335).
//!
//! Excel shows a one-value result in a table cell like any other value (a FILTER matching one row
//! in a calculated column, a SEQUENCE(1)) and refuses only a result of more than one cell. Since
//! the fix for #1491 a one-value result is a 1 x 1 spill, and every spill anchored in a table was
//! refused, so those cells showed #SPILL! (#2335). Fixture and scenario: the issue's, checked
//! against Excel for Microsoft 365 on the issue's fixture file.

use gridcraft_engine::Session;
use serde_json::json;

/// The issue's workbook: `T_Main` (A1:C4) has a calculated `Owner` column of one-row FILTERs over
/// `T_Data` and a plain COUNTIF column; `Main!F2` holds the same FILTER outside any table.
fn fixture() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["Key", "Owner", "Count"], ["k1", "", ""], ["k2", "", ""], ["k3", "", ""]]}))
        .unwrap();
    s.execute("range.setValues", json!({"range": "D1", "values": [["Key", "Owner"], ["k1", "South"], ["k2", "East"], ["k3", "North"]]})).unwrap();
    s.execute("insert.table", json!({"range": "A1:C4", "name": "T_Main"})).unwrap();
    s.execute("insert.table", json!({"range": "D1:E4", "name": "T_Data"})).unwrap();
    for row in 2..=4 {
        s.execute("cell.set", json!({"cell": format!("B{row}"), "input": "=FILTER(T_Data[Owner],T_Data[Key]=[@Key])"})).unwrap();
        s.execute("cell.set", json!({"cell": format!("C{row}"), "input": "=COUNTIF(T_Data[Key],[@Key])"})).unwrap();
    }
    s.execute("cell.set", json!({"cell": "F2", "input": "=FILTER(T_Data[Owner],T_Data[Key]=\"k1\")"})).unwrap();
    s
}

fn value(s: &mut Session, cell: &str) -> serde_json::Value {
    s.execute("cell.get", json!({"cell": cell})).unwrap()["value"].clone()
}

#[test]
fn a_one_value_filter_in_a_table_column_shows_the_value() {
    let mut s = fixture();
    // Excel on the issue's fixture: Main!B2:B4 = South, East, North (HasSpill = false).
    assert_eq!(value(&mut s, "B2"), json!("South"));
    assert_eq!(value(&mut s, "B3"), json!("East"));
    assert_eq!(value(&mut s, "B4"), json!("North"));
    // The controls were right before and stay right.
    assert_eq!(value(&mut s, "C2").as_f64(), Some(1.0), "the COUNTIF column is unchanged");
    assert_eq!(value(&mut s, "F2"), json!("South"), "the FILTER outside the table is unchanged");
}

#[test]
fn a_one_value_cell_in_a_table_still_anchors_its_range() {
    let mut s = fixture();
    // On the same fixture Excel answers ROWS(B2#) = 1, COUNTA(B2#) = 1 and ISREF(B2#) = TRUE.
    s.execute("cell.set", json!({"cell": "H2", "input": "=ROWS(B2#)"})).unwrap();
    s.execute("cell.set", json!({"cell": "H3", "input": "=COUNTA(B2#)"})).unwrap();
    s.execute("cell.set", json!({"cell": "H4", "input": "=ISREF(B2#)"})).unwrap();
    assert_eq!(value(&mut s, "H2").as_f64(), Some(1.0), "the one-value cell is a 1 x 1 range");
    assert_eq!(value(&mut s, "H3").as_f64(), Some(1.0), "its one value is there");
    assert_eq!(value(&mut s, "H4"), json!(true), "and it is a reference");
}

#[test]
fn a_two_value_result_in_a_table_column_is_still_spill() {
    let mut s = Session::new();
    s.new_workbook();
    // The table's data rows below B2 are empty, so the table rule alone refuses the spill.
    s.execute("range.setValues", json!({"range": "A1", "values": [["H1", "H2"], ["x", ""], ["z", ""], ["v", ""]]})).unwrap();
    s.execute("insert.table", json!({"range": "A1:B4", "name": "Table1"})).unwrap();
    // Excel refuses a spilled array of more than one cell in a table.
    s.execute("cell.set", json!({"cell": "B2", "input": "=SEQUENCE(2)"})).unwrap();
    assert_eq!(value(&mut s, "B2"), json!({"error": "#SPILL!"}));
    assert!(value(&mut s, "B3").is_null(), "B3 keeps nothing");
}
