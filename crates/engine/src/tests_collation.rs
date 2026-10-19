//! Engine commands group, deduplicate, match and filter text the way `gridcraft_core::text_eq`
//! does: a letter typed precomposed (NFC) equals the same letter typed as base + marks (NFD),
//! case is ignored, and the user's text itself is never rewritten.

use gridcraft_core::{CellRef, Value, canonical};
use serde_json::json;

use crate::Session;

const NFC: &str = "Nguyễn Thị Ngọc Ánh";

fn nfd(s: &str) -> String {
    canonical(s)
}

fn marks(s: &str) -> usize {
    s.chars().filter(|c| ('\u{300}'..='\u{36f}').contains(c)).count()
}

fn v(s: &Session, a: &str) -> Value {
    s.doc().unwrap().wb.active().unwrap().value(CellRef::parse(a).unwrap())
}

fn hidden_rows(s: &Session) -> Vec<u32> {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    (0..20).filter(|r| sh.is_row_hidden(*r)).collect()
}

/// A1:C6: header, then column A = [NFC, NFD, "NGUYỄN THỊ NGỌC ÁNH", "Đà Nẵng", "ĐÀ NẴNG"].
fn people(extra: &[(&str, i64, &str)]) -> Session {
    let d = nfd(NFC);
    assert_ne!(d, NFC);
    assert_eq!(marks(NFC), 0, "the NFC literal must be precomposed");
    assert!(marks(&d) > 0);
    let mut rows = vec![
        json!(["Tên", "Qty", "Kho"]),
        json!([NFC, 1, "K1"]),
        json!([d, 2, "K2"]),
        json!(["NGUYỄN THỊ NGỌC ÁNH", 3, "K1"]),
        json!(["Đà Nẵng", 4, "K1"]),
        json!(["ĐÀ NẴNG", 5, "K2"]),
    ];
    rows.extend(extra.iter().map(|(a, b, c)| json!([a, b, c])));
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s
}

fn last_row(extra: usize) -> u32 {
    6 + extra as u32
}

#[test]
fn autofilter_list_merges_canonical_and_case_variants() {
    let mut s = people(&[]);
    s.execute("data.filter", json!({"range": "A1:C6"})).unwrap();
    let list = crate::cmd::data::filter_values(&s.doc().unwrap().wb, 0, 0);
    // One entry per text, showing the first spelling met, in the Root order (Đ sorts as D).
    assert_eq!(list, vec![("Đà Nẵng".to_string(), true), (NFC.to_string(), true)]);
    // Selecting "Đà Nẵng" keeps "ĐÀ NẴNG" too.
    let r = s.execute("data.filterBy", json!({"column": "A", "values": ["Đà Nẵng"]})).unwrap();
    assert_eq!(r["hiddenRows"], 3);
    assert_eq!(hidden_rows(&s), vec![1, 2, 3]);
    let list = crate::cmd::data::filter_values(&s.doc().unwrap().wb, 0, 0);
    assert_eq!(list, vec![("Đà Nẵng".to_string(), true), (NFC.to_string(), false)]);
    // A value typed in NFD and lowercase selects every spelling of the name.
    let r = s.execute("data.filterBy", json!({"column": "A", "values": [nfd("nguyễn thị ngọc ánh")]})).unwrap();
    assert_eq!(r["hiddenRows"], 2);
    assert_eq!(hidden_rows(&s), vec![4, 5]);
}

#[test]
fn autofilter_list_keeps_numbers_and_blanks_apart_from_text() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["H"], [2], ["b"], ["ấm"], [""], [1], ["Ấm"], ["B"]]})).unwrap();
    s.execute("data.filter", json!({"range": "A1:A8"})).unwrap();
    let list: Vec<String> = crate::cmd::data::filter_values(&s.doc().unwrap().wb, 0, 0).into_iter().map(|(t, _)| t).collect();
    assert_eq!(list, ["1", "2", "ấm", "b", "(Blanks)"]);
}

#[test]
fn remove_duplicates_treats_nfd_as_duplicate() {
    let mut s = people(&[]);
    let r = s.execute("data.removeDuplicates", json!({"range": "A1:A6", "header": true})).unwrap();
    assert_eq!(r["removed"], 3);
    assert_eq!(r["remaining"], 2);
    // The first spelling of each text is the one kept, unchanged.
    assert_eq!(v(&s, "A2"), Value::from(NFC));
    assert_eq!(v(&s, "A3"), Value::from("Đà Nẵng"));
    assert_eq!(v(&s, "A4"), Value::Empty);
}

#[test]
fn validation_list_accepts_other_case_and_nfd() {
    let mut s = people(&[]);
    s.execute("data.validation", json!({"range": "E1:E3", "type": "list", "formula1": "\"Đà Nẵng,Hà Nội\""})).unwrap();
    let ok = |s: &mut Session, input: &str| s.execute("data.validate", json!({"cell": "E1", "input": input})).unwrap()["ok"] == json!(true);
    assert!(ok(&mut s, "Đà Nẵng"));
    assert!(ok(&mut s, "đà nẵng"));
    assert!(ok(&mut s, &nfd("Đà Nẵng")));
    assert!(ok(&mut s, &nfd("HÀ NỘI")));
    assert!(!ok(&mut s, "Da Nang"));
    assert!(!ok(&mut s, "Hà Nôi"));
    // A list read from cells: the column holds every spelling, each one matches.
    s.execute("data.validation", json!({"range": "F1", "type": "list", "formula1": "=$A$2:$A$6"})).unwrap();
    let ok_f = |s: &mut Session, input: &str| s.execute("data.validate", json!({"cell": "F1", "input": input})).unwrap()["ok"] == json!(true);
    assert!(ok_f(&mut s, "nguyễn thị ngọc ánh"));
    assert!(!ok_f(&mut s, "Nguyen Thi Ngoc Anh"));
}

fn pivot_height(s: &Session) -> u32 {
    s.doc().unwrap().wb.active().unwrap().pivots.last().unwrap().last_range.unwrap().height()
}

fn pivot_column(s: &Session) -> Vec<String> {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots.last().unwrap().last_range.unwrap();
    (r.start.row..=r.end.row).map(|row| sh.value(CellRef::new(row, r.start.col)).display()).collect()
}

#[test]
fn pivot_merges_nfc_nfd_items() {
    let mut s = people(&[]);
    s.execute("insert.pivotTable", json!({"rows": ["Tên"], "values": ["Qty"]})).unwrap();
    // Header, two items, grand total; the first spelling labels the item, Root order.
    assert_eq!(pivot_height(&s), 4);
    let col = pivot_column(&s);
    assert_eq!(col.get(1).map(String::as_str), Some("Đà Nẵng"));
    assert_eq!(col.get(2).map(String::as_str), Some(NFC));
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots.last().unwrap().last_range.unwrap();
    assert_eq!(sh.value(CellRef::new(r.start.row + 1, r.start.col + 1)), Value::Number(9.0));
    assert_eq!(sh.value(CellRef::new(r.start.row + 2, r.start.col + 1)), Value::Number(6.0));
    // The field list shows the same two labels.
    let f = s.execute("pivot.fields", json!({})).unwrap();
    let samples = f["fields"].as_array().unwrap().iter().find(|x| x["name"] == "Tên").unwrap()["samples"].clone();
    assert_eq!(samples, json!(["Đà Nẵng", NFC]));
}

#[test]
fn pivot_filter_and_collapse_match_any_spelling() {
    let mut s = people(&[]);
    s.execute("insert.pivotTable", json!({"rows": ["Tên", "Kho"], "values": ["Qty"]})).unwrap();
    let full = pivot_height(&s);
    // Collapsing "đà nẵng" typed in NFD hides the two Kho lines under "Đà Nẵng".
    s.execute("pivot.collapse", json!({"field": "Tên", "items": [nfd("đà nẵng")]})).unwrap();
    assert_eq!(pivot_height(&s), full - 2);
    // A second spelling of an already collapsed item is not added twice.
    let r = s.execute("pivot.collapse", json!({"field": "Tên", "items": ["ĐÀ NẴNG"]})).unwrap();
    assert_eq!(r["collapsed"].as_array().unwrap().len(), 1);
    s.execute("pivot.collapse", json!({"field": "Tên", "collapse": false, "items": ["Đà Nẵng"]})).unwrap();
    assert_eq!(pivot_height(&s), full);
    // Filtering the field to "đà nẵng" (NFD) keeps both "Đà Nẵng" rows: 4 + 5.
    s.execute("pivot.filter", json!({"field": "Tên", "selected": [nfd("đà nẵng")]})).unwrap();
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots.last().unwrap().last_range.unwrap();
    assert_eq!(sh.value(CellRef::new(r.end.row, r.start.col + 1)), Value::Number(9.0));
}

#[test]
fn slicer_merges_nfc_nfd() {
    let mut s = people(&[]);
    s.execute("insert.table", json!({"range": "A1:C6", "name": "People"})).unwrap();
    s.execute("selection.set", json!({"cell": "A2"})).unwrap();
    let id = s.execute("insert.slicer", json!({})).unwrap()["slicer"].as_u64().unwrap() as u32;
    let items = {
        let d = s.doc().unwrap();
        let sh = d.wb.active().unwrap();
        crate::cmd::slicer::slicer_items(&d.wb, 0, &sh.slicers[0])
    };
    assert_eq!(items, vec![(NFC.to_string(), true), ("Đà Nẵng".to_string(), true)]);
    // A tile value in another case and encoding is the same tile and keeps both spellings.
    s.execute("slicer.toggle", json!({"slicer": id, "value": nfd("đà nẵng")})).unwrap();
    assert_eq!(hidden_rows(&s), vec![1, 2, 3]);
    // Selecting the other tile too selects everything: the filter clears.
    s.execute("slicer.toggle", json!({"slicer": id, "value": "NGUYỄN THỊ NGỌC ÁNH", "multi": true})).unwrap();
    assert!(hidden_rows(&s).is_empty());
    assert!(s.doc().unwrap().wb.active().unwrap().slicers[0].selected.is_none());
    // Clicking the sole selected tile in another spelling clears the filter.
    s.execute("slicer.toggle", json!({"slicer": id, "value": "Đà Nẵng"})).unwrap();
    assert_eq!(hidden_rows(&s).len(), 3);
    s.execute("slicer.toggle", json!({"slicer": id, "value": "đà nẵng"})).unwrap();
    assert!(hidden_rows(&s).is_empty());
}

fn cf_hit(s: &Session, a: &str) -> bool {
    let wb = &s.doc().unwrap().wb;
    crate::cf::CfCache::new().look(wb, 0, CellRef::parse(a).unwrap()).is_some()
}

#[test]
fn cf_duplicate_marks_both_nfc_and_nfd() {
    let mut s = people(&[("Hà Nội", 6, "K1")]);
    s.execute("home.conditionalFormat", json!({"range": "A2:A7", "rule": {"type": "duplicate"}})).unwrap();
    for a in ["A2", "A3", "A4", "A5", "A6"] {
        assert!(cf_hit(&s, a), "{a} is a duplicate");
    }
    assert!(!cf_hit(&s, "A7"));
    // "Unique" is the complement: only "Hà Nội" has no other spelling in the range.
    let mut s = people(&[("Hà Nội", 6, "K1")]);
    s.execute("home.conditionalFormat", json!({"range": "A2:A7", "rule": {"type": "unique"}})).unwrap();
    assert_eq!(["A2", "A3", "A4", "A5", "A6", "A7"].map(|a| cf_hit(&s, a)), [false, false, false, false, false, true]);
}

#[test]
fn cf_text_rules_match_whole_letters_in_any_spelling() {
    let mut s = people(&[("Teo", 6, "K1")]);
    s.execute("home.conditionalFormat", json!({"range": "A2:A7", "rule": {"type": "containsText", "text": "e"}})).unwrap();
    assert_eq!(["A2", "A3", "A4", "A5", "A6", "A7"].map(|a| cf_hit(&s, a)), [false, false, false, false, false, true]);
    let mut s = people(&[("Teo", 6, "K1")]);
    s.execute("home.conditionalFormat", json!({"range": "A2:A7", "rule": {"type": "containsText", "text": "ngọc"}})).unwrap();
    assert_eq!(["A2", "A3", "A4", "A5", "A6", "A7"].map(|a| cf_hit(&s, a)), [true, true, true, false, false, false]);
    let mut s = people(&[("Teo", 6, "K1")]);
    s.execute("home.conditionalFormat", json!({"range": "A2:A7", "rule": {"type": "beginsWith", "text": nfd("đà")}})).unwrap();
    assert_eq!(["A2", "A3", "A4", "A5", "A6", "A7"].map(|a| cf_hit(&s, a)), [false, false, false, true, true, false]);
    let mut s = people(&[("Teo", 6, "K1")]);
    s.execute("home.conditionalFormat", json!({"range": "A2:A7", "rule": {"type": "endsWith", "text": "ÁNH"}})).unwrap();
    assert_eq!(["A2", "A3", "A4", "A5", "A6", "A7"].map(|a| cf_hit(&s, a)), [true, true, true, false, false, false]);
    let mut s = people(&[("Teo", 6, "K1")]);
    s.execute("home.conditionalFormat", json!({"range": "A2:A7", "rule": {"type": "notContainsText", "text": "nẵng"}})).unwrap();
    assert_eq!(["A2", "A3", "A4", "A5", "A6", "A7"].map(|a| cf_hit(&s, a)), [true, true, true, false, false, true]);
}

#[test]
fn consolidate_merges_labels() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("sheet.rename", json!({"name": "East"})).unwrap();
    s.execute("range.setValues", json!({"range": "A1", "values": [["", "Số lượng"], [NFC, 10], ["Đà Nẵng", 5]]})).unwrap();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("sheet.rename", json!({"name": "West"})).unwrap();
    s.execute("range.setValues", json!({"range": "A1", "values": [["", nfd("SỐ LƯỢNG")], [nfd(NFC), 1], ["ĐÀ NẴNG", 2]]})).unwrap();
    s.execute("sheet.activate", json!({"sheet": "Sheet1"})).unwrap();
    let r = s
        .execute("data.consolidate", json!({"sources": ["East!A1:B3", "West!A1:B3"], "destination": "A1", "topRow": true, "leftColumn": true}))
        .unwrap();
    assert_eq!(r["range"], "A1:B3");
    assert_eq!(v(&s, "B1"), Value::from("Số lượng"));
    assert_eq!(v(&s, "A2"), Value::from(NFC));
    assert_eq!(v(&s, "B2"), Value::Number(11.0));
    assert_eq!(v(&s, "A3"), Value::from("Đà Nẵng"));
    assert_eq!(v(&s, "B3"), Value::Number(7.0));
}

#[test]
fn contains_filter_matches_whole_letters() {
    let mut s = people(&[("Teo", 6, "K1")]);
    let end = last_row(1);
    s.execute("data.filter", json!({"range": format!("A1:C{end}")})).unwrap();
    // "contains e" does not see the e inside ễ: only "Teo" stays.
    let r = s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "contains", "value": "e"}})).unwrap();
    assert_eq!(r["hiddenRows"], 5);
    assert_eq!(hidden_rows(&s), vec![1, 2, 3, 4, 5]);
    // "contains nguyễn" keeps the NFC, NFD and uppercase spellings.
    let r = s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "contains", "value": "nguyễn"}})).unwrap();
    assert_eq!(r["hiddenRows"], 3);
    assert_eq!(hidden_rows(&s), vec![4, 5, 6]);
    let r = s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "notContains", "value": nfd("ngọc")}})).unwrap();
    assert_eq!(hidden_rows(&s), vec![1, 2, 3]);
    assert_eq!(r["hiddenRows"], 3);
    s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "beginsWith", "value": "đà"}})).unwrap();
    assert_eq!(hidden_rows(&s), vec![1, 2, 3, 6]);
    // A prefix that stops before a letter's marks is not a prefix.
    s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "beginsWith", "value": "nguy"}})).unwrap();
    assert_eq!(hidden_rows(&s), vec![4, 5, 6]);
    s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "beginsWith", "value": "nguye"}})).unwrap();
    assert_eq!(hidden_rows(&s), vec![1, 2, 3, 4, 5, 6]);
    s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "endsWith", "value": nfd("ÁNH")}})).unwrap();
    assert_eq!(hidden_rows(&s), vec![4, 5, 6]);
    // "=" and "<>" without wildcards compare whole texts in any spelling.
    s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "=", "value": nfd("đà nẵng")}})).unwrap();
    assert_eq!(hidden_rows(&s), vec![1, 2, 3, 6]);
    s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "<>", "value": "ĐÀ NẴNG"}})).unwrap();
    assert_eq!(hidden_rows(&s), vec![4, 5]);
}

#[test]
fn data_sort_root_puts_accented_letters_by_base() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [[3], ["Zebra"], [""], ["ấm"], [true], ["bàn"], ["#N/A"]]})).unwrap();
    assert!(matches!(v(&s, "A7"), Value::Error(_)), "A7 holds #N/A");
    assert_eq!(v(&s, "A3"), Value::Empty);
    s.execute("data.sort", json!({"range": "A1:A7", "header": false, "keys": [{"column": "A"}]})).unwrap();
    let col: Vec<Value> = (1..=7).map(|r| v(&s, &format!("A{r}"))).collect();
    assert_eq!(&col[..6], &[Value::Number(3.0), Value::from("ấm"), Value::from("bàn"), Value::from("Zebra"), Value::Bool(true), v(&s, "A6")]);
    assert!(matches!(col[5], Value::Error(_)));
    assert_eq!(col[6], Value::Empty);
    // Descending reverses the classes and the text order; blanks stay last.
    s.execute("data.sort", json!({"range": "A1:A7", "header": false, "keys": [{"column": "A", "order": "desc"}]})).unwrap();
    let col: Vec<Value> = (1..=7).map(|r| v(&s, &format!("A{r}"))).collect();
    assert!(matches!(col[0], Value::Error(_)));
    assert_eq!(&col[1..], &[Value::Bool(true), Value::from("Zebra"), Value::from("bàn"), Value::from("ấm"), Value::Number(3.0), Value::Empty]);
}

#[test]
fn data_sort_custom_list_matches_any_spelling() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["Huế"], [nfd("hà nội")], ["Zebra"], ["ĐÀ NẴNG"]]})).unwrap();
    s.execute("data.sort", json!({"range": "A1:A4", "header": false, "keys": [{"column": "A", "customList": ["Hà Nội", "Đà Nẵng", "Huế"]}]}))
        .unwrap();
    let col: Vec<Value> = (1..=4).map(|r| v(&s, &format!("A{r}"))).collect();
    assert_eq!(col, vec![Value::from(nfd("hà nội").as_str()), Value::from("ĐÀ NẴNG"), Value::from("Huế"), Value::from("Zebra")]);
}

#[test]
fn advanced_filter_matches_labels_and_texts_in_any_spelling() {
    let mut s = people(&[]);
    // The criteria header is the list header typed in NFD; "<>" excludes every Đà Nẵng spelling.
    s.execute("range.setValues", json!({"range": "E1", "values": [[nfd("TÊN")], [format!("<>{}", nfd("đà nẵng"))]]})).unwrap();
    s.execute("data.advancedFilter", json!({"range": "A1:C6", "criteria": "E1:E2"})).unwrap();
    assert_eq!(hidden_rows(&s), vec![4, 5]);
    // Unique records: the NFC, NFD and uppercase spellings of one row count once.
    s.execute("range.setValues", json!({"range": "G1", "values": [["Tên"], [NFC], [nfd(NFC)], ["NGUYỄN THỊ NGỌC ÁNH"], ["x"]]})).unwrap();
    s.execute("range.setValues", json!({"range": "I1", "values": [["Tên"]]})).unwrap();
    let r = s.execute("data.advancedFilter", json!({"range": "G1:G5", "criteria": "I1:I1", "unique": true, "copyTo": "K1"})).unwrap();
    assert_eq!(v(&s, "K2"), Value::from(NFC));
    assert_eq!(v(&s, "K3"), Value::from("x"));
    assert_eq!(v(&s, "K4"), Value::Empty, "{r}");
}

#[test]
fn autofill_continues_a_custom_list_typed_in_nfd() {
    let mut s = Session::new();
    s.new_workbook();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().wb).custom_lists = vec![vec!["Thứ Hai".into(), "Thứ Ba".into(), "Thứ Tư".into()]];
    s.execute("range.setValues", json!({"range": "A1", "values": [[nfd("thứ hai")]]})).unwrap();
    s.execute("edit.autoFill", json!({"source": "A1", "target": "A1:A3"})).unwrap();
    // The list's own spelling continues the series (a capitalised entry keeps its capital).
    assert_eq!(v(&s, "A2"), Value::from("Thứ Ba"));
    assert_eq!(v(&s, "A3"), Value::from("Thứ Tư"));
}

fn sheet_marks(s: &Session) -> usize {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    sh.cells.iter().filter_map(|(_, c)| c.value.as_text().map(marks)).sum()
}

fn sheet_texts(s: &Session) -> Vec<String> {
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let mut t: Vec<String> = sh.cells.iter().filter_map(|(_, c)| c.value.as_text().map(str::to_string)).collect();
    t.sort();
    t
}

#[test]
fn user_text_is_never_normalized() {
    let mut s = Session::new();
    s.new_workbook();
    // The NFD spellings come first, so Remove Duplicates keeps them and drops the NFC ones.
    let rows = json!([["Tên"], [nfd(NFC)], ["Zebra"], [nfd("Đà Nẵng")], [NFC], ["bàn"], ["Đà Nẵng"]]);
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    let before = sheet_marks(&s);
    assert!(before > 0);
    let texts = sheet_texts(&s);
    s.execute("data.sort", json!({"range": "A1:A7", "header": true, "keys": [{"column": "A"}]})).unwrap();
    assert_eq!(sheet_marks(&s), before);
    assert_eq!(sheet_texts(&s), texts, "Sort only moves cells");
    // A stable sort keeps each NFD spelling ahead of its NFC twin.
    assert_eq!(v(&s, "A2"), Value::from("bàn"));
    assert_eq!(v(&s, "A3"), Value::from(nfd("Đà Nẵng").as_str()));
    assert_eq!(v(&s, "A5"), Value::from(nfd(NFC).as_str()));
    let r = s.execute("data.removeDuplicates", json!({"range": "A1:A7", "header": true})).unwrap();
    assert_eq!(r["removed"], 2);
    assert_eq!(sheet_marks(&s), before);
    s.execute("data.filter", json!({"range": "A1:A5"})).unwrap();
    s.execute("data.filterBy", json!({"column": "A", "values": ["ĐÀ NẴNG"]})).unwrap();
    s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "contains", "value": nfd("nẵng")}})).unwrap();
    assert_eq!(sheet_marks(&s), before);
    let kept: Vec<Value> = (2..=5).map(|r| v(&s, &format!("A{r}"))).collect();
    assert_eq!(kept, vec![Value::from("bàn"), Value::from(nfd("Đà Nẵng").as_str()), Value::from(nfd(NFC).as_str()), Value::from("Zebra")]);
}

/// The shared items written for the `name` cache field of a saved workbook.
fn shared_items(bytes: &[u8], name: &str) -> usize {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    let mut xml = String::new();
    zip.by_name("xl/pivotCache/pivotCacheDefinition1.xml").unwrap().read_to_string(&mut xml).unwrap();
    let field = xml.split("<cacheField ").find(|f| f.contains(&format!("name=\"{name}\""))).unwrap();
    let shared = field.split("</sharedItems>").next().unwrap();
    shared.matches("<s ").count()
}

/// Saves the workbook to XLSX and reopens it: the bytes, the reopened workbook, the pivot's
/// sheet and the PivotTable.
fn save_and_reopen(s: &Session) -> (Vec<u8>, gridcraft_model::Workbook, usize, gridcraft_model::PivotTable) {
    let bytes = gridcraft_xlsx::write_xlsx(&s.doc().unwrap().wb).unwrap();
    let (back, _) = gridcraft_xlsx::read_xlsx(&bytes).unwrap();
    let si = back.sheets.iter().position(|sh| !sh.pivots.is_empty()).unwrap();
    let pt = back.sheet(si).unwrap().pivots[0].clone();
    (bytes, back, si, pt)
}

/// The first column of a recomputed PivotTable (its row labels).
fn computed_labels(wb: &gridcraft_model::Workbook, si: usize, pt: &gridcraft_model::PivotTable) -> Vec<String> {
    let out = crate::pivot::compute(wb, pt, si).unwrap();
    out.grid.iter().skip(out.body_offset as usize).filter_map(|r| r.first()).map(|c| c.value.display()).collect()
}

#[test]
fn xlsx_pivot_filter_named_in_nfd_round_trips_on_one_item() {
    let mut s = people(&[]);
    s.execute("insert.pivotTable", json!({"rows": ["Tên"], "values": ["Qty"]})).unwrap();
    s.execute("pivot.filter", json!({"field": "Tên", "selected": [nfd("đà nẵng")]})).unwrap();
    let (bytes, back, si, pt) = save_and_reopen(&s);
    // One shared item per text: NFC, NFD and uppercase spellings are one item in the cache.
    assert_eq!(shared_items(&bytes, "Tên"), 2);
    // The NFC name item is hidden, "Đà Nẵng" is not: the filter survives the save.
    let f = pt.filters.iter().find(|f| f.source_col == "Tên").expect("the filter is saved");
    assert_eq!(f.selected, Some(vec!["Đà Nẵng".to_string()]));
    let labels = computed_labels(&back, si, &pt);
    assert_eq!(labels.iter().filter(|l| l.as_str() == "Đà Nẵng").count(), 1, "{labels:?}");
    assert!(!labels.iter().any(|l| gridcraft_core::text_eq(l, NFC)), "{labels:?}");
}

#[test]
fn xlsx_pivot_collapsed_item_named_in_nfd_round_trips() {
    let mut s = people(&[]);
    s.execute("insert.pivotTable", json!({"rows": ["Tên", "Kho"], "values": ["Qty"]})).unwrap();
    s.execute("pivot.collapse", json!({"field": "Tên", "items": [nfd("NGUYỄN THỊ NGỌC ÁNH")]})).unwrap();
    let collapsed_height = pivot_height(&s);
    let (bytes, back, si, pt) = save_and_reopen(&s);
    assert_eq!(shared_items(&bytes, "Tên"), 2);
    let f = pt.rows.iter().find(|f| f.source_col == "Tên").unwrap();
    // The saved flag sits on the one item (labelled by the first spelling).
    assert_eq!(f.collapsed_items, vec![NFC.to_string()]);
    let labels = computed_labels(&back, si, &pt);
    assert_eq!(labels.len() as u32, collapsed_height, "{labels:?}");
}
