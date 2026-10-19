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
fn tone_marks_typed_as_their_singletons_merge_with_the_usual_marks() {
    // U+0340 and U+0341 (the grave and acute tone marks) canonically decompose to U+0300 and
    // U+0301, so `Ma` + U+0341 is `Má` and `MA` + U+0340 is `Mà`.
    let rows = json!([["Tên", "Qty"], ["Má", 1], ["Ma\u{341}", 2], ["Mà", 4], ["MA\u{340}", 8]]);
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Tên"], "values": ["Qty"]})).unwrap();
    // Header, two items (Root order: grave U+0300 before acute U+0301), grand total.
    assert_eq!(pivot_height(&s), 4);
    let col = pivot_column(&s);
    assert_eq!(col.get(1).map(String::as_str), Some("Mà"));
    assert_eq!(col.get(2).map(String::as_str), Some("Má"));
    let d = s.doc().unwrap();
    let sh = d.wb.active().unwrap();
    let r = sh.pivots.last().unwrap().last_range.unwrap();
    assert_eq!(sh.value(CellRef::new(r.start.row + 1, r.start.col + 1)), Value::Number(12.0));
    assert_eq!(sh.value(CellRef::new(r.start.row + 2, r.start.col + 1)), Value::Number(3.0));
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    let r = s.execute("data.removeDuplicates", json!({"range": "A1:B5", "columns": ["A"], "header": true})).unwrap();
    assert_eq!(r["removed"], 2);
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
fn pivot_collapse_without_items_is_set_based_and_spelling_blind() {
    let mut s = people(&[]);
    s.execute("insert.pivotTable", json!({"rows": ["Tên", "Kho"], "values": ["Qty"]})).unwrap();
    let full = pivot_height(&s);
    // No `items`: every label of the field, once per item whatever the spelling.
    let all = s.execute("pivot.collapse", json!({"field": "Tên"})).unwrap();
    assert_eq!(all["collapsed"].as_array().unwrap().len(), 2, "{all}");
    let collapsed_all = pivot_height(&s);
    assert!(collapsed_all < full);
    // Collapsing again, or naming other spellings, adds nothing.
    let again = s.execute("pivot.collapse", json!({"field": "Tên"})).unwrap();
    assert_eq!(again["collapsed"], all["collapsed"]);
    let spelled = s.execute("pivot.collapse", json!({"field": "Tên", "items": [nfd("đà nẵng"), "NGUYỄN THỊ NGỌC ÁNH"]})).unwrap();
    assert_eq!(spelled["collapsed"], all["collapsed"]);
    assert_eq!(pivot_height(&s), collapsed_all);
    // Expanding by name in another spelling removes exactly that item.
    let left = s.execute("pivot.collapse", json!({"field": "Tên", "collapse": false, "items": [nfd("ĐÀ NẴNG"), "không có"]})).unwrap();
    let left = left["collapsed"].as_array().unwrap().clone();
    assert_eq!(left.len(), 1);
    assert!(gridcraft_core::text_eq(left[0].as_str().unwrap(), NFC), "{left:?}");
    // Expanding everything with no `items` clears it.
    s.execute("pivot.collapse", json!({"field": "Tên", "collapse": false})).unwrap();
    assert_eq!(pivot_height(&s), full);
}

#[test]
fn pivot_collapse_keeps_the_spelling_it_met_first_and_adds_in_order() {
    let mut s = people(&[]);
    s.execute("insert.pivotTable", json!({"rows": ["Tên", "Kho"], "values": ["Qty"]})).unwrap();
    s.execute("pivot.collapse", json!({"field": "Tên", "items": ["đà nẵng"]})).unwrap();
    let r = s.execute("pivot.collapse", json!({"field": "Tên", "items": [nfd("ĐÀ NẴNG"), "Hà Nội", "HÀ NỘI", nfd("hà nội"), "Huế"]})).unwrap();
    assert_eq!(r["collapsed"], json!(["đà nẵng", "Hà Nội", "Huế"]));
}

#[test]
fn pivot_column_collapse_matches_any_spelling() {
    let mut s = people(&[]);
    s.execute("insert.pivotTable", json!({"rows": ["Kho"], "columns": ["Tên", "Qty"], "values": ["Qty"]})).unwrap();
    let width = |s: &Session| s.doc().unwrap().wb.active().unwrap().pivots.last().unwrap().last_range.unwrap().width();
    let full = width(&s);
    s.execute("pivot.collapse", json!({"field": "Tên", "items": [nfd("đà nẵng")]})).unwrap();
    let nfd_width = width(&s);
    assert!(nfd_width < full, "{nfd_width} < {full}");
    s.execute("pivot.collapse", json!({"field": "Tên", "collapse": false})).unwrap();
    assert_eq!(width(&s), full);
    s.execute("pivot.collapse", json!({"field": "Tên", "items": ["ĐÀ NẴNG"]})).unwrap();
    assert_eq!(width(&s), nfd_width, "the uppercase spelling collapses the same item");
}

#[test]
fn pivot_collapse_of_many_distinct_non_ascii_labels_completes() {
    // 20 000 distinct Vietnamese labels: collapsing all of them used to compare every label with
    // every collapsed item (and fold both each time).
    const N: usize = 20_000;
    let rows: Vec<serde_json::Value> = std::iter::once(json!(["Tên", "Qty", "Kho"]))
        .chain((0..N).map(|i| json!([format!("Đà Nẵng số {i}"), 1, if i % 2 == 0 { "K1" } else { "K2" }])))
        .collect();
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("selection.set", json!({"range": "A1"})).unwrap();
    s.execute("insert.pivotTable", json!({"rows": ["Tên", "Kho"], "values": ["Qty"]})).unwrap();
    let all = s.execute("pivot.collapse", json!({"field": "Tên"})).unwrap();
    assert_eq!(all["collapsed"].as_array().unwrap().len(), N);
    // Header, one line per label, grand total.
    assert_eq!(pivot_height(&s) as usize, N + 2);
    // Every item again in another spelling adds none; expanding them all removes them all.
    let names: Vec<String> = (0..N).map(|i| nfd(&format!("ĐÀ NẴNG SỐ {i}"))).collect();
    let again = s.execute("pivot.collapse", json!({"field": "Tên", "items": names})).unwrap();
    assert_eq!(again["collapsed"].as_array().unwrap().len(), N);
    let names: Vec<String> = (0..N).map(|i| nfd(&format!("Đà Nẵng số {i}"))).collect();
    let none = s.execute("pivot.collapse", json!({"field": "Tên", "collapse": false, "items": names})).unwrap();
    assert!(none["collapsed"].as_array().unwrap().is_empty());
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

#[test]
fn xlsx_pivot_over_the_nfd_twin_of_a_header_round_trips_on_its_column() {
    // Two headers equal but for encoding: the engine keeps them apart (source headers match by
    // ASCII case), so the saved cache must too, and the field must resolve to its own column.
    let tn = "Tên";
    for (header, other) in [(nfd(tn), "x"), (tn.to_string(), "a")] {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[tn, nfd(tn), "Qty"], ["a", "x", 1], ["b", "y", 2]]})).unwrap();
        s.execute("selection.set", json!({"range": "A1"})).unwrap();
        s.execute("insert.pivotTable", json!({"rows": [header], "values": ["Qty"]})).unwrap();
        let shown = pivot_column(&s);
        assert_eq!(shown.get(1).map(String::as_str), Some(other), "{shown:?}");
        let (_, back, si, pt) = save_and_reopen(&s);
        assert_eq!(pt.rows.iter().map(|f| f.source_col.clone()).collect::<Vec<_>>(), vec![header.clone()]);
        let labels = computed_labels(&back, si, &pt);
        assert_eq!(labels, shown, "the reopened PivotTable reads the same column");
    }
}

/// The first `n` (at most 2 560) of a list of distinct Vietnamese full names.
fn many_names(n: usize) -> Vec<String> {
    let fam = ["Nguyễn", "Trần", "Lê", "Phạm", "Hoàng", "Huỳnh", "Phan", "Vũ", "Võ", "Đặng", "Bùi", "Đỗ", "Hồ", "Ngô", "Dương", "Lý"];
    let mid = ["Văn", "Thị", "Ngọc", "Minh", "Hữu", "Đức", "Thanh", "Quốc"];
    let giv = [
        "Anh", "Ánh", "Bình", "Cường", "Dũng", "Hà", "Hương", "Lan", "Long", "Mai", "Nam", "Phương", "Quân", "Sơn", "Thảo", "Trung", "Tuấn", "Uyên",
        "Ưng", "Yến",
    ];
    let mut out = Vec::new();
    for f in fam {
        for m in mid {
            for g in giv {
                out.push(format!("{f} {m} {g}"));
            }
        }
    }
    out.truncate(n);
    out
}

#[test]
fn value_filter_over_2000_names_folds_each_cell_once() {
    // Review C1: every row used to be compared with every chosen value (two folds per pair, 3 s
    // per apply and per menu frame at this size). The chosen values now go into a `TextSet`
    // once (one fold each, pinned by core's `text_set_folds_each_text_once`) and each cell is
    // folded once to look it up; the answers are the same relation as before.
    let names = many_names(2_000);
    let mut rows = vec![json!(["Tên"])];
    rows.extend(names.iter().map(|n| json!([n])));
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s.execute("data.filter", json!({"range": "A1:A2001"})).unwrap();
    // Every name but the first, chosen in NFD or upper case.
    let chosen: Vec<String> = names.iter().skip(1).enumerate().map(|(i, n)| if i % 2 == 0 { nfd(n) } else { n.to_uppercase() }).collect();
    let r = s.execute("data.filterBy", json!({"column": "A", "values": chosen})).unwrap();
    assert_eq!(r["hiddenRows"], 1);
    assert_eq!(hidden_rows(&s), vec![1]);
    let list = crate::cmd::data::filter_values(&s.doc().unwrap().wb, 0, 0);
    assert_eq!(list.len(), 2_000);
    let off: Vec<&String> = list.iter().filter(|(_, on)| !on).map(|(t, _)| t).collect();
    assert_eq!(off, vec![&names[0]]);
}

#[test]
fn match_case_tie_break_reads_letters_in_canonical_form() {
    // Review M1: `ÁNH` and `Ánh` tie ignoring case; the first letter is `Á` in one encoding and
    // `A` + U+0301 in the other, so the case of `N`/`n` must decide: lowercase first.
    for (upper, lower) in [("ÁNH".to_string(), nfd("Ánh")), (nfd("ÁNH"), "Ánh".to_string())] {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [[upper.clone()], [lower.clone()]]})).unwrap();
        s.execute("data.sort", json!({"range": "A1:A2", "header": false, "matchCase": true, "keys": [{"column": "A"}]})).unwrap();
        assert_eq!((v(&s, "A1"), v(&s, "A2")), (Value::from(lower.as_str()), Value::from(upper.as_str())), "{upper:?} {lower:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// The session's sorting language (`data.sortLanguage`): Data › Sort, the AutoFilter list, Pivot,
// conditional formatting and formulas all order text by it, in every open workbook.

/// Vietnamese names in input order (the oracle's V1 input; same as `gridcraft_calc` tests).
const V1_INPUT: &[&str] = &[
    "Yến", "Ưng", "Uyên", "Út", "Phan", "Phạm", "Ơn", "Ông", "Oanh", "Nguyễn", "Ngô", "Huỳnh", "Hồ", "Hoàng", "Ê-đê", "Em", "Đỗ", "Đinh", "Đặng",
    "Đào", "Dương", "Duy", "Dũng", "Bùi", "Bình", "Âu", "Ấn", "Ân", "Ăn", "Ắc", "Ánh", "Ảnh", "Anh", "An",
];

/// V1 sorted in the CLDR `vi` order (ICU oracle).
const V1_EXPECTED: &[&str] = &[
    "An", "Anh", "Ảnh", "Ánh", "Ắc", "Ăn", "Ân", "Ấn", "Âu", "Bình", "Bùi", "Dũng", "Duy", "Dương", "Đào", "Đặng", "Đinh", "Đỗ", "Em", "Ê-đê",
    "Hoàng", "Hồ", "Huỳnh", "Ngô", "Nguyễn", "Oanh", "Ông", "Ơn", "Phạm", "Phan", "Út", "Uyên", "Ưng", "Yến",
];

fn lang(s: &mut Session, l: &str) -> serde_json::Value {
    s.execute("data.sortLanguage", json!({"lang": l})).unwrap()
}

fn coll(s: &Session) -> gridcraft_core::Collation {
    s.doc().unwrap().wb.collation
}

fn column(s: &Session, col: &str, n: usize) -> Vec<String> {
    (1..=n).map(|r| v(s, &format!("{col}{r}")).display()).collect()
}

fn names_session(words: &[&str]) -> Session {
    let mut s = Session::new();
    s.new_workbook();
    let rows: Vec<serde_json::Value> = words.iter().map(|w| json!([w])).collect();
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
    s
}

#[test]
fn sort_language_command_sets_and_reports() {
    use gridcraft_core::Collation;
    let mut s = Session::new();
    // No document needed: it's a session setting.
    assert_eq!(s.execute("data.sortLanguage", json!({})).unwrap(), json!({"lang": ""}));
    s.new_workbook();
    s.new_workbook();
    assert_eq!(s.execute("data.sortLanguage", json!({})).unwrap(), json!({"lang": ""}));
    assert_eq!(coll(&s), Collation::Root);
    let undo_before = s.doc().unwrap().undo.len();
    assert_eq!(lang(&mut s, "vi-VN"), json!({"lang": "vi"}));
    assert_eq!(s.prefs.sort_language, "vi");
    // Every open workbook follows it, and it's not an undo step.
    assert!(s.documents().iter().all(|d| d.wb.collation == Collation::Vi));
    assert_eq!(s.doc().unwrap().undo.len(), undo_before);
    assert_eq!(s.execute("data.sortLanguage", json!({})).unwrap(), json!({"lang": "vi"}));
    assert_eq!(lang(&mut s, "zz"), json!({"lang": ""}));
    assert_eq!(s.prefs.sort_language, "");
    assert!(s.documents().iter().all(|d| d.wb.collation == Collation::Root));
    // Junk parameters: an error or no change, never a panic.
    lang(&mut s, "vi");
    assert!(s.execute("data.sortLanguage", json!({"lang": 5})).is_err());
    assert_eq!(s.prefs.sort_language, "vi");
    let r = s.execute("data.sortLanguage", json!([1]));
    assert!(r.is_err() || r.unwrap() == json!({"lang": "vi"}));
    assert_eq!(s.prefs.sort_language, "vi");
    assert!(s.documents().iter().all(|d| d.wb.collation == Collation::Vi));
    // A new workbook starts in the session's language.
    s.new_workbook();
    assert_eq!(coll(&s), Collation::Vi);
    s.execute("file.new", json!({"sample": "sales"})).unwrap();
    assert_eq!(coll(&s), Collation::Vi);
    s.execute("view.newWindow", json!({})).unwrap();
    assert_eq!(coll(&s), Collation::Vi);
    // A document built outside the session (as the UI's tests do) gets it when added.
    s.add_document(crate::DocState::new(gridcraft_model::Workbook::new(), None, "Outside".into()));
    assert_eq!(coll(&s), Collation::Vi);
    // Preferences loaded after documents were opened (the desktop app's start-up order).
    let mut prefs = s.prefs.clone();
    prefs.sort_language = String::new();
    s.prefs = prefs;
    s.apply_sort_language();
    assert!(s.documents().iter().all(|d| d.wb.collation == Collation::Root));
}

#[test]
fn data_sort_follows_vi() {
    let n = V1_INPUT.len();
    let mut s = names_session(V1_INPUT);
    lang(&mut s, "vi");
    s.execute("data.sort", json!({"range": format!("A1:A{n}"), "header": false, "keys": [{"column": "A"}]})).unwrap();
    assert_eq!(column(&s, "A", n), V1_EXPECTED);
    // Root: accented letters by their base letter, `Đ` with `D`.
    lang(&mut s, "");
    s.execute("data.sort", json!({"range": format!("A1:A{n}"), "header": false, "keys": [{"column": "A"}]})).unwrap();
    let root = column(&s, "A", n);
    let at = |w: &str| root.iter().position(|x| x == w).unwrap();
    assert_eq!(&root[at("Đặng")..=at("Duy")], ["Đặng", "Đào", "Đinh", "Đỗ", "Dũng", "Dương", "Duy"]);
    assert_eq!(&root[..9], ["Ắc", "An", "Ân", "Ấn", "Ăn", "Anh", "Ánh", "Ảnh", "Âu"]);
    // Descending under vi is the exact reverse.
    lang(&mut s, "vi");
    s.execute("data.sort", json!({"range": format!("A1:A{n}"), "header": false, "keys": [{"column": "A", "order": "desc"}]})).unwrap();
    let mut rev = V1_EXPECTED.to_vec();
    rev.reverse();
    assert_eq!(column(&s, "A", n), rev);
}

#[test]
fn data_sort_vi_keeps_value_classes_and_case_order() {
    let mut s = Session::new();
    s.new_workbook();
    lang(&mut s, "vi");
    s.execute("range.setValues", json!({"range": "A1", "values": [[3], ["Đặng"], [""], ["Em"], [true], ["#N/A"], ["Dương"]]})).unwrap();
    s.execute("data.sort", json!({"range": "A1:A7", "header": false, "keys": [{"column": "A"}]})).unwrap();
    // Numbers < text < booleans < errors, blanks last; only the order inside text follows vi.
    assert_eq!(column(&s, "A", 7), ["3", "Dương", "Đặng", "Em", "TRUE", "#N/A", ""]);
    lang(&mut s, "");
    s.execute("data.sort", json!({"range": "A1:A7", "header": false, "keys": [{"column": "A"}]})).unwrap();
    assert_eq!(column(&s, "A", 7), ["3", "Đặng", "Dương", "Em", "TRUE", "#N/A", ""]);
    // Match case: equal ignoring case, lowercase first; the vi order still decides the rest.
    lang(&mut s, "vi");
    s.execute("range.setValues", json!({"range": "B1", "values": [["ĐÀ"], ["đà"], ["Da"], ["da"], ["Dz"]]})).unwrap();
    s.execute("data.sort", json!({"range": "B1:B5", "header": false, "matchCase": true, "keys": [{"column": "B"}]})).unwrap();
    assert_eq!(column(&s, "B", 5), ["da", "Da", "Dz", "đà", "ĐÀ"]);
}

#[test]
fn pivot_and_autofilter_follow_vi() {
    let provinces = ["Đắk Lắk", "Hà Nội", "Đà Nẵng", "Hà Nội", "Huế"];
    let build = |extra: &[&str], l: &str| {
        let mut s = Session::new();
        s.new_workbook();
        lang(&mut s, l);
        let mut rows = vec![json!(["Tỉnh", "Qty"])];
        rows.extend(provinces.iter().chain(extra).enumerate().map(|(i, p)| json!([p, i + 1])));
        let n = rows.len();
        s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
        s.execute("data.filter", json!({"range": format!("A1:B{n}")})).unwrap();
        let list: Vec<String> = crate::cmd::data::filter_values(&s.doc().unwrap().wb, 0, 0).into_iter().map(|(t, _)| t).collect();
        s.execute("selection.set", json!({"range": format!("A1:B{n}")})).unwrap();
        s.execute("insert.pivotTable", json!({"rows": ["Tỉnh"], "values": ["Qty"]})).unwrap();
        let col = pivot_column(&s);
        let items = col.get(1..col.len().saturating_sub(1)).unwrap_or_default().to_vec();
        let f = s.execute("pivot.fields", json!({})).unwrap();
        let samples = f["fields"].as_array().unwrap().iter().find(|x| x["name"] == "Tỉnh").unwrap()["samples"].clone();
        (list, items, samples)
    };
    let want = ["Đà Nẵng", "Đắk Lắk", "Hà Nội", "Huế"];
    let (list, items, samples) = build(&[], "vi");
    assert_eq!(items, want);
    assert_eq!(list, want);
    assert_eq!(samples, json!(want));
    // `Dĩ An` tells the two orders apart: vi puts `d` before `đ`, Root sorts `đ` as `d`.
    let (list, items, samples) = build(&["Dĩ An"], "vi");
    let want_vi = ["Dĩ An", "Đà Nẵng", "Đắk Lắk", "Hà Nội", "Huế"];
    assert_eq!((list, items), (want_vi.map(String::from).to_vec(), want_vi.map(String::from).to_vec()));
    assert_eq!(samples, json!(want_vi));
    let (list, items, _) = build(&["Dĩ An"], "");
    let want_root = ["Đà Nẵng", "Đắk Lắk", "Dĩ An", "Hà Nội", "Huế"];
    assert_eq!((list, items), (want_root.map(String::from).to_vec(), want_root.map(String::from).to_vec()));
}

#[test]
fn custom_filter_advanced_filter_and_cf_follow_vi() {
    let words = ["Tên", "Dương", "Đặng", "Em"];
    // A custom AutoFilter `<Đ`: Root sorts `Dương` after `Đ` (`đ` is `d` plus a stroke), vi
    // puts every `d` word before `đ`.
    let mut s = names_session(&words);
    s.execute("data.filter", json!({"range": "A1:A4"})).unwrap();
    s.execute("data.filterBy", json!({"column": "A", "custom": {"op": "<", "value": "Đ"}})).unwrap();
    assert_eq!(hidden_rows(&s), vec![1, 2, 3]);
    lang(&mut s, "vi");
    s.execute("data.reapply", json!({})).unwrap();
    assert_eq!(hidden_rows(&s), vec![2, 3]);
    // Advanced Filter with a `<Đ` criterion.
    let mut s = names_session(&words);
    s.execute("range.setValues", json!({"range": "C1", "values": [["Tên"], ["<Đ"]]})).unwrap();
    s.execute("data.advancedFilter", json!({"range": "A1:A4", "criteria": "C1:C2", "copyTo": "E1"})).unwrap();
    assert_eq!(column(&s, "E", 2), ["Tên", ""]);
    lang(&mut s, "vi");
    s.execute("data.advancedFilter", json!({"range": "A1:A4", "criteria": "C1:C2", "copyTo": "E1"})).unwrap();
    assert_eq!(column(&s, "E", 3), ["Tên", "Dương", ""]);
    // Conditional formatting "Cell Value less than Đ" is the `<` operator outside a formula.
    lang(&mut s, "");
    s.execute("home.conditionalFormat", json!({"range": "A2:A4", "rule": {"type": "cellIs", "operator": "less", "value": "Đ"}})).unwrap();
    assert!(!cf_hit(&s, "A2"));
    lang(&mut s, "vi");
    assert!(cf_hit(&s, "A2"));
    assert!(!cf_hit(&s, "A3"));
}

#[test]
fn changing_language_recalculates() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["Dương", "=A1<A2", "=COUNTIF(A1:A2,\"<Đ\")"], ["Đặng", "", ""]]})).unwrap();
    s.execute("cell.set", json!({"cell": "D1", "input": "x"})).unwrap();
    assert_eq!(v(&s, "B1"), Value::Bool(false));
    assert_eq!(v(&s, "C1"), Value::Number(0.0));
    lang(&mut s, "vi");
    // Right away, without editing a cell.
    assert_eq!(v(&s, "B1"), Value::Bool(true));
    assert_eq!(v(&s, "C1"), Value::Number(1.0));
    // Undo and redo of other commands don't bring the old order back, even to a state recorded
    // before the change.
    s.execute("cell.set", json!({"cell": "D2", "input": "y"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!((v(&s, "B1"), v(&s, "C1")), (Value::Bool(true), Value::Number(1.0)));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(v(&s, "D1"), Value::Empty);
    assert_eq!(coll(&s), gridcraft_core::Collation::Vi);
    assert_eq!((v(&s, "B1"), v(&s, "C1")), (Value::Bool(true), Value::Number(1.0)));
    s.execute("edit.redo", json!({"steps": 2})).unwrap();
    assert_eq!(v(&s, "D2"), Value::from("y"));
    assert_eq!((v(&s, "B1"), v(&s, "C1")), (Value::Bool(true), Value::Number(1.0)));
    lang(&mut s, "");
    assert_eq!((v(&s, "B1"), v(&s, "C1")), (Value::Bool(false), Value::Number(0.0)));
    s.execute("edit.undo", json!({"steps": 3})).unwrap();
    s.execute("edit.redo", json!({"steps": 3})).unwrap();
    assert_eq!((v(&s, "B1"), v(&s, "C1")), (Value::Bool(false), Value::Number(0.0)));
}

#[test]
fn undo_and_open_keep_the_session_sort_language() {
    use gridcraft_core::Collation;
    // A file saved from a Root session.
    let bytes = {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": [["Dương", "=A1<A2"], ["Đặng", ""]]})).unwrap();
        assert_eq!(v(&s, "B1"), Value::Bool(false));
        gridcraft_xlsx::write_xlsx(&s.doc().unwrap().wb).unwrap()
    };
    let mut s = Session::new();
    s.new_workbook();
    lang(&mut s, "vi");
    s.execute("cell.set", json!({"cell": "A1", "input": "Dương"})).unwrap();
    s.execute("cell.set", json!({"cell": "A2", "input": "Đặng"})).unwrap();
    s.execute("cell.set", json!({"cell": "B1", "input": "=A1<A2"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(coll(&s), Collation::Vi);
    assert_eq!(v(&s, "B1"), Value::Bool(true));
    // File › Open of an XLSX (no language in the file): the session's vi applies.
    s.execute("file.open", json!({"name": "root.xlsx", "base64": crate::io::base64_encode(&bytes)})).unwrap();
    assert_eq!(s.doc().unwrap().display_title(), "root.xlsx");
    assert_eq!(coll(&s), Collation::Vi);
    assert_eq!(v(&s, "B1"), Value::Bool(true));
    // Opening a file in vi doesn't make it dirty.
    assert!(!s.doc().unwrap().is_dirty());
    // CSV too.
    s.execute("file.open", json!({"name": "names.csv", "base64": crate::io::base64_encode("Dương\nĐặng\n".as_bytes())})).unwrap();
    assert_eq!(s.doc().unwrap().display_title(), "names.csv");
    assert_eq!(coll(&s), Collation::Vi);
    s.execute("cell.set", json!({"cell": "B1", "input": "=A1<A2"})).unwrap();
    assert_eq!(v(&s, "B1"), Value::Bool(true));
    // The language is not written into the file.
    let saved = gridcraft_xlsx::write_xlsx(&s.doc().unwrap().wb).unwrap();
    let (back, _) = gridcraft_xlsx::read_xlsx(&saved).unwrap();
    assert_eq!(back.collation, Collation::Root);
    // A clean document stays clean when the language changes.
    s.execute("file.open", json!({"name": "again.xlsx", "base64": crate::io::base64_encode(&bytes)})).unwrap();
    lang(&mut s, "");
    assert!(!s.doc().unwrap().is_dirty());
    assert_eq!(v(&s, "B1"), Value::Bool(false));
}

#[test]
fn hostile_sort_language_params_never_panic() {
    use gridcraft_core::Collation;
    let marks = format!("a{}", "\u{301}".repeat(10_000));
    let big = "Đ".repeat(500_000);
    let cells = [marks.as_str(), "\u{301}\u{300}x", "\u{10FFFF}", "İ", big.as_str(), "Đặng", "Dương", "", "-"];
    let mut s = names_session(&cells);
    s.execute("range.setValues", json!({"range": "B1", "values": [["=MATCH(\"Đ\",A1:A9,1)", "=COUNTIF(A1:A9,\"<Đ\")", "=A1<A5"]]})).unwrap();
    let long = "v".repeat(1 << 20);
    let params = [
        json!({"lang": ""}),
        json!({"lang": "-"}),
        json!({"lang": "VI_vn"}),
        json!({"lang": "zz"}),
        json!({"lang": "\u{10FFFF}"}),
        json!({"lang": "İ"}),
        json!({"lang": long}),
        json!({"lang": marks}),
        json!({"lang": 5}),
        json!({"lang": [1]}),
        json!({"lang": {"x": 1}}),
        json!({"lang": null}),
        json!({"lang": true}),
        json!([1]),
        json!(5),
        json!("vi"),
        json!(null),
    ];
    for p in params {
        let before = s.prefs.sort_language.clone();
        let vi_tag = p == json!({"lang": "VI_vn"});
        match s.execute("data.sortLanguage", p.clone()) {
            Ok(r) => {
                let tag = r["lang"].as_str().unwrap();
                assert!(tag.is_empty() || tag == "vi", "{p}: {r}");
                assert_eq!(s.prefs.sort_language, tag);
                assert_eq!(coll(&s), Collation::from_tag(tag));
                assert!(!vi_tag || tag == "vi", "VI_vn is Vietnamese");
            }
            Err(e) => {
                assert!(!matches!(e, crate::EngineError::Internal(..)), "{p}: {e}");
                assert_eq!(s.prefs.sort_language, before, "{p}: an error changes nothing");
            }
        }
        let r = s.execute("data.sort", json!({"range": "A1:A9", "header": false, "keys": [{"column": "A"}]}));
        assert!(!matches!(r, Err(crate::EngineError::Internal(..))), "{p}: {r:?}");
    }
    assert_eq!(s.prefs.sort_language, "", "the last tag set (a long non-vi string) is Root; junk after it changed nothing");
    // Sort only moves cells: every combining mark is still there.
    assert_eq!(sheet_marks(&s), 10_000 + 2);
}
