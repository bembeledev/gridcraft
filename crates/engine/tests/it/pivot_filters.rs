//! Pivot value filters: Top/Bottom N, above/below, between and by-field.

use gridcraft_core::CellRef;
use gridcraft_engine::Session;
use serde_json::json;

fn sales() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let rows = json!([["Region", "Sales"], ["East", 340], ["West", 165], ["North", 120],]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"], "destination": "Sheet1!D3"})).unwrap();
    s
}

/// The row-item labels the report shows (grand total excluded), top to bottom.
fn items(s: &Session) -> Vec<String> {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let Some(r) = sh.pivots[0].last_range else { return vec![] };
    (r.start.row + 1..r.end.row)
        .map(|row| sh.value(CellRef::new(row, r.start.col)).display())
        .filter(|t| !t.is_empty() && t != "Grand Total")
        .collect()
}

/// The value cell of the item `label`.
fn value(s: &Session, label: &str) -> String {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots[0].last_range.unwrap();
    (r.start.row + 1..r.end.row)
        .find(|row| sh.value(CellRef::new(*row, r.start.col)).display() == label)
        .map(|row| sh.value(CellRef::new(row, r.start.col + 1)).display())
        .unwrap_or_default()
}

fn clear(s: &mut Session) {
    s.execute("pivot.filter", json!({"field": "Region", "selected": null})).unwrap();
}

#[test]
fn top_n_keeps_the_highest_values() {
    let mut s = sales();
    assert_eq!(items(&s), vec!["East", "North", "West"]);
    s.execute("pivot.filter", json!({"field": "Region", "topN": 2})).unwrap();
    assert_eq!(items(&s), vec!["East", "West"], "East 340 and West 165 are the top two");
}

#[test]
fn bottom_n_keeps_the_lowest_values() {
    let mut s = sales();
    s.execute("pivot.filter", json!({"field": "Region", "bottomN": 1})).unwrap();
    assert_eq!(items(&s), vec!["North"], "North 120 is the lowest");
}

#[test]
fn above_and_below_thresholds() {
    let mut s = sales();
    s.execute("pivot.filter", json!({"field": "Region", "above": 150.0})).unwrap();
    assert_eq!(items(&s), vec!["East", "West"]);
    clear(&mut s);
    s.execute("pivot.filter", json!({"field": "Region", "below": 150.0})).unwrap();
    assert_eq!(items(&s), vec!["North"]);
}

#[test]
fn between_keeps_the_inclusive_band() {
    let mut s = sales();
    s.execute("pivot.filter", json!({"field": "Region", "betweenLo": 100.0, "betweenHi": 200.0})).unwrap();
    assert_eq!(items(&s), vec!["North", "West"], "120 and 165 are inside 100..200");
}

#[test]
fn top_percent_rounds_up() {
    let mut s = sales();
    // 3 items, top 50% → ceil(3 * 0.5) = 2.
    s.execute("pivot.filter", json!({"field": "Region", "topPercent": 50})).unwrap();
    assert_eq!(items(&s), vec!["East", "West"]);
}

#[test]
fn a_repeated_filter_replaces_the_previous_one() {
    let mut s = sales();
    s.execute("pivot.filter", json!({"field": "Region", "topN": 2})).unwrap();
    s.execute("pivot.filter", json!({"field": "Region", "bottomN": 1})).unwrap();
    assert_eq!(items(&s), vec!["North"], "the second filter wins, it does not stack");
    clear(&mut s);
    assert_eq!(items(&s), vec!["East", "North", "West"], "clearing brings every item back");
    assert_eq!(value(&s, "East"), "340");
}

#[test]
fn a_label_filter_and_a_value_filter_stay_independent() {
    let mut s = sales();
    s.execute("pivot.filter", json!({"field": "Region", "selected": ["East", "West"]})).unwrap();
    assert_eq!(items(&s), vec!["East", "West"]);
    // Bottom N over the selection is West (165), not North (120): the untouched selection is what
    // makes the result differ, so this fails if the value filter clears it.
    s.execute("pivot.filter", json!({"field": "Region", "bottomN": 1})).unwrap();
    assert_eq!(items(&s), vec!["West"], "the value filter applies within the selected labels");
}

/// A report counting a text column, so `Count` and a raw sum differ sharply.
fn counting() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    // East 3 records, West 2, North 1.
    let rows = json!([["Region", "Rep"], ["East", "a"], ["East", "b"], ["East", "c"], ["West", "d"], ["West", "e"], ["North", "f"],]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": [{"field": "Rep", "func": "count"}], "destination": "Sheet1!D3"})).unwrap();
    s
}

#[test]
fn a_value_filter_ranks_by_the_summary_function() {
    // The filter used to sum the raw source value, and a text column sums to 0: every region tied at
    // zero and Top 1 kept them all. With Count it must keep the region with the most records.
    let mut s = counting();
    assert_eq!(items(&s), vec!["East", "North", "West"]);
    s.execute("pivot.filter", json!({"field": "Region", "topN": 1})).unwrap();
    assert_eq!(items(&s), vec!["East"], "East has the most records");
    clear(&mut s);
    s.execute("pivot.filter", json!({"field": "Region", "bottomN": 1})).unwrap();
    assert_eq!(items(&s), vec!["North"], "North has the fewest");
    clear(&mut s);
    s.execute("pivot.filter", json!({"field": "Region", "above": 2.0})).unwrap();
    assert_eq!(items(&s), vec!["East"], "only East has more than two records");
}
