use gridcraft_engine::Session;
use serde_json::json;

/// Writes a small workbook to a unique temp path and returns it.
fn temp_book(tag: &str) -> String {
    let mut s = Session::new();
    s.new_workbook();
    let bytes = gridcraft_engine::io::save_bytes(&s.doc().unwrap().wb, "x.xlsx").unwrap();
    let path = std::env::temp_dir().join(format!("gridcraft-hidden-{tag}-{}.xlsx", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn open(s: &mut Session, path: &str) -> serde_json::Value {
    s.execute("file.open", json!({"path": path})).unwrap()
}

#[test]
fn reopening_the_only_hidden_workbook_unhides_it() {
    let path = temp_book("single");
    let mut s = Session::new();
    open(&mut s, &path);
    s.execute("view.hideWindow", json!({})).unwrap();
    assert_eq!(s.hidden_windows, vec![0]);
    let r = open(&mut s, &path);
    assert_eq!(r["alreadyOpen"], true);
    assert_eq!(s.active_index(), 0);
    assert!(s.hidden_windows.is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn reopening_a_hidden_workbook_among_visible_ones_unhides_and_activates_it() {
    let path = temp_book("multi");
    let mut s = Session::new();
    s.new_workbook();
    s.execute("file.new", json!({})).unwrap();
    open(&mut s, &path);
    let i = s.active_index();
    s.execute("view.hideWindow", json!({})).unwrap();
    assert_ne!(s.active_index(), i);
    assert!(s.hidden_windows.contains(&i));
    let r = open(&mut s, &path);
    assert_eq!(r["alreadyOpen"], true);
    assert_eq!(s.active_index(), i);
    assert!(!s.hidden_windows.contains(&s.active_index()));
    let _ = std::fs::remove_file(path);
}

#[test]
fn switching_to_a_hidden_window_unhides_it() {
    let mut s = Session::new();
    s.new_workbook();
    s.new_workbook();
    s.execute("view.hideWindow", json!({})).unwrap();
    assert_eq!(s.hidden_windows, vec![1]);
    s.execute("window.activate", json!({"index": 1})).unwrap();
    assert_eq!(s.active_index(), 1);
    assert!(s.hidden_windows.is_empty());
}

#[test]
fn closing_the_active_window_does_not_activate_a_hidden_one() {
    let mut s = Session::new();
    for _ in 0..3 {
        s.new_workbook();
    }
    s.set_active(1);
    s.execute("view.hideWindow", json!({})).unwrap(); // hides 1, activates 0
    s.set_active(2);
    s.execute("file.close", json!({})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.hidden_windows, vec![1]);
    assert_eq!(s.active_index(), 0);
}

#[test]
fn closing_a_window_keeps_hidden_indices_on_their_documents() {
    let mut s = Session::new();
    for _ in 0..3 {
        s.new_workbook();
    }
    let hidden_title = s.documents()[2].display_title();
    s.execute("view.hideWindow", json!({})).unwrap(); // hides 2, activates 0
    assert_eq!(s.active_index(), 0);
    s.execute("file.close", json!({})).unwrap(); // closes 0
    assert_eq!(s.hidden_windows, vec![1]);
    assert_eq!(s.documents()[1].display_title(), hidden_title);
    // A new workbook takes a fresh index and must not inherit a stale hidden mark.
    let n = s.new_workbook();
    assert!(!s.hidden_windows.contains(&n));
}

#[test]
fn hiding_both_of_two_workbooks_keeps_both_hidden() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("file.new", json!({})).unwrap();
    s.execute("view.hideWindow", json!({})).unwrap();
    s.execute("view.hideWindow", json!({})).unwrap();
    let mut hidden = s.hidden_windows.clone();
    hidden.sort_unstable();
    hidden.dedup();
    assert_eq!(hidden, vec![0, 1]);
}

#[test]
fn hiding_the_only_workbook_keeps_it_hidden() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("view.hideWindow", json!({})).unwrap();
    assert_eq!(s.hidden_windows, vec![0]);
    assert_eq!(s.active_index(), 0);
}
