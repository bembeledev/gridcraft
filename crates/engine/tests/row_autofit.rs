//! Whole-row and whole-column formatting must re-fit row heights (issue #833).

use serde_json::json;

fn sheet_mut(s: &mut gridcraft_engine::Session) -> &mut gridcraft_engine::model::Sheet {
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().wb).sheet_mut(0).unwrap()
}

#[test]
fn whole_row_font_format_grows_row_height_and_undoes() {
    let mut s = gridcraft_engine::Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A5", "values": [["plain big bold"]]})).unwrap();
    let before = s.doc().unwrap().wb.active().unwrap().row_height(4);
    s.execute("home.fontSize", json!({"range": "5:5", "size": 20})).unwrap();
    let after = s.doc().unwrap().wb.active().unwrap().row_height(4);
    assert!(after > before + 10.0, "20pt whole-row format should grow the row: before={before}, after={after}");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.doc().unwrap().wb.active().unwrap().row_height(4), before, "undo must restore the height");
}

#[test]
fn whole_column_font_format_grows_only_rows_with_cells() {
    let mut s = gridcraft_engine::Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["a"]]})).unwrap();
    s.execute("range.setValues", json!({"range": "A50", "values": [["b"]]})).unwrap();
    s.execute("home.fontSize", json!({"range": "A:A", "size": 20})).unwrap();
    let sh = s.doc().unwrap().wb.active().unwrap();
    assert!(sh.row_height(0) > 30.0 && sh.row_height(49) > 30.0, "rows with cells grow");
    assert!(sh.rows.len() <= 3, "full-column format must fit only rows that have cells, rows map has {} entries", sh.rows.len());
}

#[test]
fn whole_row_format_does_not_resize_custom_height_rows() {
    let mut s = gridcraft_engine::Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A5", "values": [["plain big bold"]]})).unwrap();
    {
        let sh = sheet_mut(&mut s);
        let e = sh.rows.entry(4).or_default();
        e.custom = true;
        e.size = Some(25.0);
    }
    s.execute("home.fontSize", json!({"range": "5:5", "size": 20})).unwrap();
    assert_eq!(s.doc().unwrap().wb.active().unwrap().row_height(4), 25.0, "custom-height rows keep their height");
}

#[test]
fn whole_row_format_keeps_hidden_rows_hidden() {
    let mut s = gridcraft_engine::Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A5", "values": [["plain big bold"]]})).unwrap();
    let before = s.doc().unwrap().wb.active().unwrap().row_height(4);
    sheet_mut(&mut s).rows.entry(4).or_default().hidden = true;
    s.execute("home.fontSize", json!({"range": "5:5", "size": 20})).unwrap();
    let sh = s.doc().unwrap().wb.active().unwrap();
    let info = sh.rows.get(&4).unwrap();
    assert!(info.size.unwrap_or(before) > before + 10.0, "hidden row still grows to its font: before={before}, size={:?}", info.size);
    assert!(info.hidden, "formatting a hidden row must not unhide it");
}

/// Wrapped text moves whole words to the next line, leaving room at the end of each line. A row
/// sized from the text's total width over the column's (as if words split anywhere) cut off the
/// last lines of a narrow column.
#[test]
fn wrapped_text_row_fits_every_word_wrapped_line() {
    let height = |text: &str| {
        let mut s = gridcraft_engine::Session::new();
        s.new_workbook();
        s.execute("home.columnWidth", json!({"cols": "A:A", "chars": 12})).unwrap();
        s.execute("home.wrapText", json!({"range": "A1", "on": true})).unwrap();
        s.execute("range.setValues", json!({"range": "A1", "values": [[text]]})).unwrap();
        s.doc().unwrap().wb.active().unwrap().row_height(0)
    };
    // Each word fits the column alone, never two together: one line per word.
    let wrapped = height("aaaaaaaa aaaaaaaa aaaaaaaa aaaaaaaa aaaaaaaa");
    let broken = height("aaaaaaaa\naaaaaaaa\naaaaaaaa\naaaaaaaa\naaaaaaaa");
    assert_eq!(wrapped, broken, "five words that each fill a line take five lines");
}

/// A word wider than the column is split across lines, as the grid draws it.
#[test]
fn a_word_wider_than_the_column_takes_several_lines() {
    let w = gridcraft_engine::cmd::format::approx_text_width("aaaaaaaa", 11.0, false);
    assert_eq!(gridcraft_engine::cmd::format::wrapped_line_count("aaaaaaaa aaaaaaaa", 11.0, false, w * 1.5), 2.0);
    assert_eq!(gridcraft_engine::cmd::format::wrapped_line_count(&"a".repeat(24), 11.0, false, w), 3.0);
    assert_eq!(gridcraft_engine::cmd::format::wrapped_line_count("", 11.0, false, w), 1.0);
    assert_eq!(gridcraft_engine::cmd::format::wrapped_line_count("aa", 11.0, false, 0.0), 2.0, "a zero width can't loop or panic");
    // A tab is four spaces wide and a break, as the grid draws it: two words that fit with a space
    // between them don't with a tab.
    let one_space = gridcraft_engine::cmd::format::approx_text_width("aaaaaaaa aaaaaaaa", 11.0, false) + 1.0;
    assert_eq!(gridcraft_engine::cmd::format::wrapped_line_count("aaaaaaaa aaaaaaaa", 11.0, false, one_space), 1.0);
    assert_eq!(gridcraft_engine::cmd::format::wrapped_line_count("aaaaaaaa\taaaaaaaa", 11.0, false, one_space), 2.0);
}

/// With a UI's measurer installed, a wrapped row takes the measured height (plus the grid's
/// margin), not the per-character estimate; one measured line keeps the font's line height.
#[test]
fn wrapped_text_row_uses_the_installed_measurer() {
    use gridcraft_engine::measure::{TextMeasure, set_text_measurer};
    use std::rc::Rc;
    let height = |text: &str| {
        let mut s = gridcraft_engine::Session::new();
        s.new_workbook();
        s.execute("home.wrapText", json!({"range": "A1", "on": true})).unwrap();
        s.execute("range.setValues", json!({"range": "A1", "values": [[text]]})).unwrap();
        s.doc().unwrap().wb.active().unwrap().row_height(0)
    };
    let default_one_line = height("one");
    set_text_measurer(Some(Rc::new(|q: &TextMeasure<'_>| {
        assert!(q.wrap_width.is_some_and(|w| w > 0.0), "wrapped text is measured at the column's width");
        Some(if q.text == "one" { (18.0, 1) } else { (200.0, 11) })
    })));
    let measured = height("many words that wrap");
    let one_line = height("one");
    set_text_measurer(None);
    assert_eq!(measured, 205.0, "the measured height and the grid's margin");
    assert_eq!(one_line, default_one_line, "a single measured line keeps the line height");
}

/// When the measurer changes (the UI bound the workbook's own fonts), rows of wrapped text are
/// refitted: they grow to the new measure, never shrink below a height already there, keep custom
/// heights, and the refit is no edit (no undo step, a saved workbook stays saved).
#[test]
fn wrapped_rows_refit_when_the_measurer_changes() {
    use gridcraft_engine::measure::{TextMeasure, set_text_measurer};
    use std::rc::Rc;
    let mut s = gridcraft_engine::Session::new();
    s.new_workbook();
    s.execute("home.wrapText", json!({"range": "A1:A2", "on": true})).unwrap();
    s.execute("range.setValues", json!({"range": "A1:A2", "values": [["many words that wrap"], ["more words that wrap"]]})).unwrap();
    s.execute("home.rowHeight", json!({"rows": "2:2", "height": 30})).unwrap();
    {
        let d = s.active_mut().unwrap();
        d.saved = d.wb.clone();
    }
    let undo = s.doc().unwrap().undo.len();
    let revision = s.doc().unwrap().revision;
    let row = |s: &gridcraft_engine::Session, r: u32| s.doc().unwrap().wb.active().unwrap().row_height(r);

    set_text_measurer(Some(Rc::new(|_: &TextMeasure<'_>| Some((300.0, 15)))));
    s.refit_wrapped_rows();
    assert_eq!(row(&s, 0), 305.0, "the row grows to the new measure");
    assert_eq!(row(&s, 1), 30.0, "a custom height is kept");
    let d = s.doc().unwrap();
    assert!(!d.is_dirty(), "a saved workbook stays saved");
    assert_eq!(d.undo.len(), undo, "the refit is no undo step");
    assert!(d.revision > revision, "views keyed on the revision redraw");

    set_text_measurer(Some(Rc::new(|_: &TextMeasure<'_>| Some((40.0, 2)))));
    s.refit_wrapped_rows();
    assert_eq!(row(&s, 0), 305.0, "a refit never shrinks a row");

    // Every open workbook is refitted, not only the active one: one opened earlier may become
    // active later with no font change to refit it then.
    s.new_workbook();
    set_text_measurer(Some(Rc::new(|_: &TextMeasure<'_>| Some((400.0, 20)))));
    s.refit_wrapped_rows();
    set_text_measurer(None);
    s.set_active(0);
    assert_eq!(row(&s, 0), 405.0, "a workbook that isn't active is refitted too");
}
