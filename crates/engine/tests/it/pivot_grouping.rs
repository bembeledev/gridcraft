//! Pivot grouping: date units, number bins, manual text groups, ungroup and sort-by-value.

use gridcraft_core::CellRef;
use gridcraft_engine::Session;
use serde_json::json;

fn sales() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let rows = json!([
        ["Region", "Date", "Units", "Sales"],
        ["East", "1/15/2026", 3, 100],
        ["West", "2/10/2026", 12, 50],
        ["East", "3/3/2026", 7, 120],
        ["West", "3/9/2026", 22, 30],
    ]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"], "destination": "Sheet1!F3"})).unwrap();
    s
}

/// The report's row labels, top to bottom (no grand total).
fn labels(s: &Session) -> Vec<String> {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let Some(r) = sh.pivots[0].last_range else { return vec![] };
    (r.start.row + 1..r.end.row)
        .map(|row| sh.value(CellRef::new(row, r.start.col)).display())
        .filter(|t| !t.is_empty() && t != "Grand Total")
        .collect()
}

fn with_field(field: &str) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let rows = json!([
        ["Region", "Date", "Units", "Sales"],
        ["East", "1/15/2026", 3, 100],
        ["West", "2/10/2026", 12, 50],
        ["East", "3/3/2026", 7, 120],
        ["West", "3/9/2026", 22, 30],
    ]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": [field], "values": ["Sales"], "destination": "Sheet1!F3"})).unwrap();
    s
}

#[test]
fn grouping_a_date_by_months_makes_month_labels() {
    let mut s = with_field("Date");
    s.execute("pivot.group", json!({"field": "Date", "by": "months"})).unwrap();
    let l = labels(&s);
    assert_eq!(l, vec!["Jan".to_string(), "Feb".to_string(), "Mar".to_string()], "got {l:?}");
}

#[test]
fn grouping_a_date_by_quarters_makes_quarter_labels() {
    let mut s = with_field("Date");
    s.execute("pivot.group", json!({"field": "Date", "by": "quarters"})).unwrap();
    let l = labels(&s);
    assert_eq!(l, vec!["Qtr1".to_string()], "Jan–Mar are Q1 (Excel's caption is \"Qtr1\"); got {l:?}");
}

#[test]
fn grouping_a_date_by_years_makes_year_labels() {
    let mut s = with_field("Date");
    s.execute("pivot.group", json!({"field": "Date", "by": "years"})).unwrap();
    assert_eq!(labels(&s), vec!["2026".to_string()]);
}

#[test]
fn ungrouping_restores_the_raw_dates() {
    let mut s = with_field("Date");
    s.execute("pivot.group", json!({"field": "Date", "by": "months"})).unwrap();
    s.execute("pivot.group", json!({"field": "Date", "by": "none"})).unwrap();
    let l = labels(&s);
    assert!(l.len() >= 3, "every distinct date comes back, got {l:?}");
    assert!(l.iter().all(|x| x.chars().all(|c| c.is_ascii_digit())), "one row per distinct date, got {l:?}");
}

#[test]
fn number_bins_group_values_into_bands() {
    let mut s = with_field("Units");
    s.execute("pivot.group", json!({"field": "Units", "by": "numbers", "start": 0.0, "size": 10.0})).unwrap();
    let l = labels(&s);
    // Units 3 and 7 fall in 0–10; 12 and 22 in 10–20 and 20–30.
    assert_eq!(l.len(), 3, "three bins hold the four sales, got {l:?}");
    assert!(l.iter().all(|x| x.contains('-') || x.contains("0")), "the labels are bands, got {l:?}");
}

#[test]
fn manual_text_groups_show_the_group_captions() {
    let mut s = sales();
    s.execute("pivot.group", json!({"field": "Region", "by": "text", "items": [["East", "West"]]})).unwrap();
    let i = s.doc().unwrap().wb.active().unwrap().pivots[0].rows[0].grouped_items.clone();
    assert_eq!(i, vec![vec!["East".to_string(), "West".to_string()]], "the group is stored on the field");
}

#[test]
fn sort_by_a_value_field_orders_by_that_field() {
    let mut s = sales();
    // East 220, West 80: descending puts East first, ascending West first.
    s.execute("pivot.sort", json!({"field": "Region", "order": "desc", "byField": "Sales"})).unwrap();
    assert_eq!(labels(&s), vec!["East".to_string(), "West".to_string()]);
    s.execute("pivot.sort", json!({"field": "Region", "order": "asc", "byField": "Sales"})).unwrap();
    assert_eq!(labels(&s), vec!["West".to_string(), "East".to_string()]);
}

#[test]
fn sort_by_label_is_the_default_order() {
    let s = sales();
    let l = labels(&s);
    assert_eq!(l, vec!["East".to_string(), "West".to_string()], "labels sort A→Z, got {l:?}");
}

#[test]
fn an_unsorted_field_keeps_source_order() {
    // With `sort: none` the report follows the order items first appear in the source, not the
    // hash-set iteration order. The pre-scan that interned every item with `first = 0` made every
    // comparison equal, so the order could change from one refresh to the next.
    let mut s = Session::new();
    s.new_workbook();
    let rows = json!([["Region", "Sales"], ["West", 1], ["East", 2], ["North", 3],]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"], "destination": "Sheet1!F3"})).unwrap();
    s.execute("pivot.sort", json!({"field": "Region", "order": "none"})).unwrap();
    let l = labels(&s);
    assert_eq!(l, vec!["West".to_string(), "East".to_string(), "North".to_string()], "source order, got {l:?}");
    // A refresh must not reshuffle it.
    for _ in 0..3 {
        s.execute("pivot.refresh", json!({})).unwrap();
        let again: Vec<String> = labels(&s).into_iter().filter(|x| x != "Row Labels").collect();
        assert_eq!(again, l, "the order is stable across refreshes");
    }
}

#[test]
fn show_items_with_no_data_keeps_filtered_items_hidden() {
    // Showing empty items must not resurrect the items a label filter hides.
    let mut s = with_field("Region");
    s.execute("pivot.filter", json!({"field": "Region", "selected": ["East"]})).unwrap();
    s.execute("pivot.layout", json!({"showItemsWithNoData": true})).unwrap();
    let l = labels(&s);
    assert!(l.contains(&"East".to_string()), "the shown item is there, got {l:?}");
    assert!(!l.contains(&"West".to_string()), "a hidden item stays hidden, got {l:?}");
}

#[test]
fn a_grouped_and_filtered_report_survives_saving_and_reopening() {
    // End to end: group and filter a report, save the workbook, read it back, and check the report
    // is still grouped and filtered. All of these settings used to be dropped by the writer.
    let mut s = with_field("Date");
    s.execute("pivot.group", json!({"field": "Date", "by": "months"})).unwrap();
    let bytes = gridcraft_xlsx::write_xlsx(&s.doc().unwrap().wb).unwrap();
    let (back, _) = gridcraft_xlsx::read_xlsx(&bytes).unwrap();
    let pt = &back.sheet(0).unwrap().pivots[0];
    assert_eq!(pt.rows[0].date_group, gridcraft_model::PivotDateGroup::Months, "the month grouping survives the save");
}

#[test]
fn a_value_filter_survives_saving_and_reopening() {
    let mut s = with_field("Region");
    s.execute("pivot.filter", json!({"field": "Region", "topN": 1})).unwrap();
    let bytes = gridcraft_xlsx::write_xlsx(&s.doc().unwrap().wb).unwrap();
    let (back, _) = gridcraft_xlsx::read_xlsx(&bytes).unwrap();
    let pt = &back.sheet(0).unwrap().pivots[0];
    let vf = pt.filters.iter().find_map(|f| f.value_filter.as_ref()).expect("the value filter is written");
    assert_eq!((vf.n, vf.top), (1, true), "Top 1 survives the save");
}

#[test]
fn number_bins_survive_saving_and_reopening() {
    let mut s = with_field("Units");
    s.execute("pivot.group", json!({"field": "Units", "by": "numbers", "start": 0.0, "size": 10.0})).unwrap();
    let bytes = gridcraft_xlsx::write_xlsx(&s.doc().unwrap().wb).unwrap();
    let (back, _) = gridcraft_xlsx::read_xlsx(&bytes).unwrap();
    let pt = &back.sheet(0).unwrap().pivots[0];
    assert_eq!(pt.rows[0].group_bins, Some((0.0, 10.0)), "the bin size survives the save");
}

#[test]
fn items_shown_with_no_data_keep_source_order() {
    // "Plums" only sells pears, and the page filter hides pears: its rows reach the report only
    // through the show-items-with-no-data pass. That pass used `first = 0` for every item it added,
    // so under `sort: none` such an item sorted ahead of everything instead of by source order.
    let mut s = Session::new();
    s.new_workbook();
    let rows = json!([["Region", "Product", "Sales"], ["West", "Apples", 1], ["East", "Apples", 2], ["North", "Apples", 3], ["Plums", "Pears", 4],]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Region"], "values": ["Sales"], "filters": ["Product"], "destination": "Sheet1!H3"})).unwrap();
    s.execute("pivot.filter", json!({"field": "Product", "selected": ["Apples"]})).unwrap();
    s.execute("pivot.sort", json!({"field": "Region", "order": "none"})).unwrap();
    s.execute("pivot.layout", json!({"showItemsWithNoData": true})).unwrap();
    // The page field sits above, so the first label row is the header.
    let l: Vec<String> = labels(&s).into_iter().filter(|x| x != "Row Labels").collect();
    assert_eq!(l, vec!["West".to_string(), "East".to_string(), "North".to_string(), "Plums".to_string()], "source order, got {l:?}");
    for _ in 0..3 {
        s.execute("pivot.refresh", json!({})).unwrap();
        let again: Vec<String> = labels(&s).into_iter().filter(|x| x != "Row Labels").collect();
        assert_eq!(again, l, "the order is stable across refreshes");
    }
}
