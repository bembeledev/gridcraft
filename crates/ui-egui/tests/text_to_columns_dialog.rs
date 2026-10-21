//! Convert Text to Columns: the dialog previews the split and runs it, delimited or fixed width.

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    harness_with(json!({}))
}

fn harness_with(options: serde_json::Value) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [["ab,cd;ef"], ["\"x,y\",z"]]})).unwrap();
    s.execute("selection.set", json!({"range": "A1:A2"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.state_mut().open_dialog("textToColumns", options);
    h.run_steps(4);
    h
}

fn value(h: &mut Harness, cell: &str) -> serde_json::Value {
    h.state_mut().session.execute("cell.get", json!({"cell": cell})).unwrap()["value"].clone()
}

#[test]
fn the_preview_follows_the_options_and_ok_splits() {
    let mut h = harness();
    // Comma is on by default: the preview shows the pieces, the quoted comma kept.
    assert!(h.query_by_label("ab").is_some() && h.query_by_label("cd;ef").is_some());
    assert!(h.query_by_label("x,y").is_some());
    h.get_by_label("Semicolon").click();
    h.run_steps(3);
    assert!(h.query_by_label("ef").is_some(), "ticking Semicolon splits the preview there too");
    h.get_by_label_contains("OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    for (cell, v) in [("A1", "ab"), ("B1", "cd"), ("C1", "ef"), ("A2", "x,y"), ("B2", "z")] {
        assert_eq!(value(&mut h, cell), json!(v), "{cell}");
    }
}

#[test]
fn fixed_width_splits_at_the_typed_breaks() {
    let mut h = harness_with(json!({"widths": "5, 2, 2"}));
    h.get_by_label("Fixed width").click();
    h.run_steps(3);
    assert!(h.query_by_label(",cd").is_some(), "the preview cuts at 2 and 5");
    h.get_by_label_contains("OK").click();
    h.run_steps(3);
    for (cell, v) in [("A1", "ab"), ("B1", ",cd"), ("C1", ";ef")] {
        assert_eq!(value(&mut h, cell), json!(v), "{cell}");
    }
}
