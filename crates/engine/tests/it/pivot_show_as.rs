//! Show Values As variants: each one is checked against the numbers it must produce, not just
//! against "the report still exists".

use gridcraft_core::CellRef;
use gridcraft_engine::Session;
use serde_json::json;

fn sales() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let rows = json!([
        ["Region", "Product", "Date", "Units", "Sales"],
        ["East", "Apples", "1/15/2026", 10, 100],
        ["West", "Apples", "2/10/2026", 5, 50],
        ["East", "Pears", "3/3/2026", 8, 120],
        ["North", "Pears", "4/20/2026", 2, 30],
        ["West", "Plums", "5/5/2026", 7, 70],
        ["East", "Plums", "6/30/2026", 4, 60],
        ["North", "Apples", "7/4/2026", 9, 90],
        ["West", "Pears", "8/8/2026", 3, 45],
        ["East", "Apples", "9/9/2026", 6, 60],
    ]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s
}

/// East 340, North 120, West 165; grand total 625.
fn report(show_as: &str) -> Session {
    let mut s = sales();
    s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"], "destination": "Sheet1!G3"})).unwrap();
    if show_as != "normal" {
        s.execute("pivot.valueSettings", json!({"index": 0, "showAs": show_as})).unwrap();
    }
    s
}

/// The value cells, top item first (East, North, West, Grand Total).
fn values(s: &Session) -> Vec<f64> {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots[0].last_range.unwrap();
    (r.start.row + 1..=r.end.row).map(|row| sh.value(CellRef::new(row, r.start.col + 1)).as_f64().unwrap_or(f64::NAN)).collect()
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
}

#[test]
fn normal_is_the_plain_aggregate() {
    assert!(close(&values(&report("normal")), &[340.0, 120.0, 165.0, 625.0]));
}

#[test]
fn percent_of_grand_total_divides_by_the_total() {
    assert!(close(&values(&report("percentOfGrandTotal")), &[340.0 / 625.0, 120.0 / 625.0, 165.0 / 625.0, 1.0]));
}

#[test]
fn running_total_accumulates_down_the_rows() {
    assert!(close(&values(&report("runningTotal")), &[340.0, 460.0, 625.0, 625.0]));
}

#[test]
fn percent_running_total_accumulates_as_a_share() {
    let v = values(&report("percentRunningTotalIn"));
    assert!(close(&v, &[340.0 / 625.0, 460.0 / 625.0, 1.0, 1.0]), "got {v:?}");
}

#[test]
fn rank_is_largest_to_smallest_and_ascending_is_the_reverse() {
    assert!(close(&values(&report("rank"))[..3], &[1.0, 3.0, 2.0]), "East 340 > West 165 > North 120");
    assert!(close(&values(&report("rankAscending"))[..3], &[3.0, 1.0, 2.0]));
}

#[test]
fn rank_leaves_the_grand_total_empty() {
    let v = values(&report("rank"));
    assert!(v[3].is_nan(), "the grand total has no rank, got {:?}", v[3]);
}

#[test]
fn index_with_one_value_field_is_one_everywhere() {
    // Index = (value / row total) / (column total / grand total); with a single value field and
    // one row axis, every ratio cancels to 1. Excel shows 1 for the same layout.
    assert!(close(&values(&report("index")), &[1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn percent_of_parent_row_total_with_one_row_axis_is_the_grand_total() {
    // The parent of a top-level item is the whole report, so every item is its share of 625.
    let v = values(&report("percentOfParentRowTotal"));
    assert!(close(&v, &[340.0 / 625.0, 120.0 / 625.0, 165.0 / 625.0, 1.0]), "got {v:?}");
}

#[test]
fn percent_of_parent_row_total_uses_the_subtotal_of_a_nested_group() {
    let mut s = sales();
    s.execute("insert.pivotTable", json!({"rows": ["Region", "Product"], "values": ["Sales"], "destination": "Sheet1!G3"})).unwrap();
    s.execute("pivot.valueSettings", json!({"index": 0, "showAs": "percentOfParentRowTotal"})).unwrap();
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots[0].last_range.unwrap();
    // East = 100 + 120 + 60 + 60 = 340, of which Apples is 100 + 60 = 160.
    let apples = (r.start.row..=r.end.row)
        .find(|x| sh.value(CellRef::new(*x, r.start.col)).display() == "Apples")
        .map(|x| sh.value(CellRef::new(x, r.start.col + 1)).as_f64().unwrap_or(f64::NAN))
        .unwrap_or(f64::NAN);
    assert!((apples - 160.0 / 340.0).abs() < 1e-9, "East's Apples is its share of East's 340, got {apples}");
}

/// The report cell holding `label` in the first column, and the value beside it.
fn cell_of(s: &Session, label: &str) -> Option<f64> {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots[0].last_range?;
    (r.start.row..=r.end.row)
        .find(|x| sh.value(CellRef::new(*x, r.start.col)).display() == label)
        .map(|x| sh.value(CellRef::new(x, r.start.col + 1)))
        .and_then(|v| v.as_f64())
}

#[test]
fn difference_from_subtracts_the_previous_item() {
    let s = report("differenceFrom");
    // East 340 is first: nothing to compare to, so Excel leaves it empty.
    assert!(cell_of(&s, "East").is_none(), "the first item has no previous sibling, got {:?}", cell_of(&s, "East"));
    assert!((cell_of(&s, "North").unwrap_or(f64::NAN) - (120.0 - 340.0)).abs() < 1e-9, "North minus East");
    assert!((cell_of(&s, "West").unwrap_or(f64::NAN) - (165.0 - 120.0)).abs() < 1e-9, "West minus North");
}

#[test]
fn difference_from_uses_the_chosen_base_item() {
    let mut s = report("normal");
    s.execute("pivot.valueSettings", json!({"index": 0, "showAs": "differenceFrom", "baseItem": "East"})).unwrap();
    // The base item itself shows nothing; every other item is measured against East's 340.
    assert!(cell_of(&s, "East").is_none(), "the base item has no difference from itself");
    assert!((cell_of(&s, "North").unwrap_or(f64::NAN) - (120.0 - 340.0)).abs() < 1e-9);
    assert!((cell_of(&s, "West").unwrap_or(f64::NAN) - (165.0 - 340.0)).abs() < 1e-9);
}

#[test]
fn percent_difference_from_divides_by_the_base() {
    let s = report("percentDifferenceFrom");
    assert!(cell_of(&s, "East").is_none(), "the first item has no previous sibling");
    assert!((cell_of(&s, "North").unwrap_or(f64::NAN) - (120.0 - 340.0) / 340.0).abs() < 1e-9);
    assert!((cell_of(&s, "West").unwrap_or(f64::NAN) - (165.0 - 120.0) / 120.0).abs() < 1e-9);
}

#[test]
fn percent_difference_from_reads_the_base_item_of_its_own_nesting_level() {
    // The base item is a Product label and the report's Product level is the second one: looking
    // it up in the Region list (the outermost level) found nothing.
    let mut s = sales();
    s.execute("insert.pivotTable", json!({"rows": ["Region", "Product"], "values": ["Sales"], "destination": "Sheet1!G3"})).unwrap();
    s.execute("pivot.valueSettings", json!({"index": 0, "showAs": "differenceFrom", "baseItem": "Pears"})).unwrap();
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots[0].last_range.unwrap();
    // Rows are East{Apples, Pears, Plums}, North{Apples, Pears, Plums}, West{Apples, Pears, Plums},
    // Grand Total. East's Pears is 120, so its Plums (60) is 60 - 120 = -60.
    let rows: Vec<(String, f64)> = (r.start.row..=r.end.row)
        .map(|x| (sh.value(CellRef::new(x, r.start.col)).display(), sh.value(CellRef::new(x, r.start.col + 1)).as_f64().unwrap_or(f64::NAN)))
        .collect();
    let plums_at = rows.iter().position(|(l, _)| l == "Plums").map(|i| rows[i].1).unwrap_or(f64::NAN);
    assert!((plums_at - (60.0 - 120.0)).abs() < 1e-9, "Plums measured against Pears in its own group, got {plums_at}");
}

#[test]
fn difference_from_can_name_another_row_field_as_the_base() {
    // The dialog offers the report's fields, as Excel does. The engine used to look the name up
    // among the *value* fields, so "Region" (a row field) was ignored and the innermost row field
    // was used instead — every difference came out as "this product minus the previous product".
    let mut s = sales();
    s.execute("insert.pivotTable", json!({"rows": ["Region", "Product"], "values": ["Sales"], "destination": "Sheet1!G3"})).unwrap();
    s.execute("pivot.valueSettings", json!({"index": 0, "showAs": "differenceFrom", "baseField": "Region", "baseItem": "West"})).unwrap();
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots[0].last_range.unwrap();
    let cells: Vec<(String, f64)> = (r.start.row..=r.end.row)
        .map(|x| (sh.value(CellRef::new(x, r.start.col)).display(), sh.value(CellRef::new(x, r.start.col + 1)).as_f64().unwrap_or(f64::NAN)))
        .collect();
    let at = |name: &str, nth: usize| -> f64 {
        cells.iter().enumerate().filter(|(_, (l, _))| l == name).nth(nth).map(|(_, (_, v))| *v).unwrap_or(f64::NAN)
    };
    // Same product, measured against West's: East Apples 160 - West Apples 50 = 110.
    assert!((at("Apples", 0) - (160.0 - 50.0)).abs() < 1e-9, "East Apples against West Apples, got {}", at("Apples", 0));
    // Pears: East 120 - West 45 = 75; Plums: East 60 - West 70 = -10.
    assert!((at("Pears", 0) - (120.0 - 45.0)).abs() < 1e-9, "got {}", at("Pears", 0));
    assert!((at("Plums", 0) - (60.0 - 70.0)).abs() < 1e-9, "got {}", at("Plums", 0));
    // Region itself: East 340 - West 165 = 175, and West's own group has nothing to compare to.
    assert!((at("East", 0) - (340.0 - 165.0)).abs() < 1e-9, "got {}", at("East", 0));
    assert!(at("West", 0).is_nan(), "the base group has no difference from itself, got {}", at("West", 0));
}

#[test]
fn percent_of_field_divides_by_that_fields_subtotal() {
    let mut s = sales();
    s.execute("insert.pivotTable", json!({"rows": ["Region", "Product"], "values": ["Sales"], "destination": "Sheet1!G3"})).unwrap();
    s.execute("pivot.valueSettings", json!({"index": 0, "showAs": "percentOf", "baseField": "Region"})).unwrap();
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots[0].last_range.unwrap();
    // Rows are the header, then East, East's products, ... The base field is Region, whose outermost
    // level has no parent, so the share is of the grand total: 160 / 625 for East's Apples.
    let east_apples = sh.value(CellRef::new(r.start.row + 2, r.start.col + 1)).as_f64().unwrap_or(f64::NAN);
    assert!((east_apples - 160.0 / 625.0).abs() < 1e-9, "the share of Region's top level, got {east_apples}");
}
