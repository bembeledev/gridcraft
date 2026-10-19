//! Space on a checkbox cell toggles it instead of starting a text edit (#1951).

use egui::{Event, Key, Modifiers};
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(session: Session) -> Harness {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    h
}

fn checkbox_book() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("insert.checkbox", json!({"range": "A1"})).unwrap();
    s.execute("selection.set", json!({"cell": "A1"})).unwrap();
    s
}

fn value(h: &Harness, a1: &str) -> gridcraft_engine::core::Value {
    let d = h.state().session.active().unwrap();
    let sh = d.wb.active().unwrap();
    sh.value(CellRef::parse(a1).unwrap())
}

/// Press Space the way winit reports it: a Key press followed by the Text(" ") it types.
fn press_space(h: &mut Harness) {
    h.input_mut().events.push(Event::Key { key: Key::Space, physical_key: Some(Key::Space), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::Text(" ".to_string()));
    h.input_mut().events.push(Event::Key { key: Key::Space, physical_key: Some(Key::Space), pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn space_toggles_checkbox_without_editing() {
    let mut h = harness(checkbox_book());
    assert_eq!(value(&h, "A1"), gridcraft_engine::core::Value::Bool(false), "starts unchecked");
    press_space(&mut h);
    assert!(h.state().editor.is_none(), "Space must not start a text edit on a checkbox cell");
    assert_eq!(value(&h, "A1"), gridcraft_engine::core::Value::Bool(true), "Space toggles the checkbox");
    press_space(&mut h);
    assert!(h.state().editor.is_none());
    assert_eq!(value(&h, "A1"), gridcraft_engine::core::Value::Bool(false), "Space toggles back");
}

#[test]
fn space_still_starts_edit_on_a_plain_cell() {
    // Control: the Text(" ") edit path must stay intact for non-checkbox cells.
    let mut s = Session::new();
    s.new_workbook();
    s.execute("selection.set", json!({"cell": "A1"})).unwrap();
    let mut h = harness(s);
    press_space(&mut h);
    assert!(h.state().editor.is_some(), "Space on a plain cell types a space, as before");
}
