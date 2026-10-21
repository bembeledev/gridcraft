//! Space on a checkbox cell toggles it instead of starting a text edit (#1951).

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::{SheetApp, grid::Geo};
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

fn point(h: &Harness, a1: &str) -> Pos2 {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll).cell_rect(sheet, CellRef::parse(a1).unwrap()).center()
}

/// Press Space the way winit reports it: a Key press followed by the Text(" ") it types.
fn press_space(h: &mut Harness) {
    h.input_mut().events.push(Event::Key {
        key: Key::Space,
        physical_key: Some(Key::Space),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.input_mut().events.push(Event::Text(" ".to_string()));
    h.input_mut().events.push(Event::Key {
        key: Key::Space,
        physical_key: Some(Key::Space),
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(2);
}

/// All events land in one frame, before the next `run_steps`.
fn push(h: &mut Harness, ev: Event) {
    h.input_mut().events.push(ev);
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

#[test]
fn arrow_off_the_checkbox_then_space_edits_the_new_cell() {
    // Arrow off the checkbox, then Space: it must apply to the plain cell the arrow landed
    // on, not the checkbox the selection started on (#1951 review).
    let mut h = harness(checkbox_book());
    // Click a plain cell so the grid has focus (clicking A1 would toggle the checkbox),
    // then move the selection onto the checkbox via command.
    let at = point(&h, "C1");
    push(&mut h, Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    push(&mut h, Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.state_mut().session.execute("selection.set", json!({"cell": "A1"})).unwrap();
    push(&mut h, Event::Key { key: Key::ArrowRight, physical_key: Some(Key::ArrowRight), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    press_space(&mut h);
    assert_eq!(value(&h, "A1"), gridcraft_engine::core::Value::Bool(false), "the checkbox was not toggled");
    assert!(h.state().editor.is_some(), "Space on the plain cell B1 starts an edit");
}

#[test]
fn checkbox_cell_holding_text_still_edits() {
    // A checkbox-formatted cell with text can't be toggled, so Space must not be swallowed (#1951 review).
    let mut s = checkbox_book();
    s.execute("cell.set", json!({"cell": "A1", "input": "hello"})).unwrap();
    let mut h = harness(s);
    press_space(&mut h);
    assert_eq!(value(&h, "A1").display(), "hello", "text is untouched");
    assert!(h.state().editor.is_some(), "Space starts an edit when the toggle cannot apply");
}

#[test]
fn space_on_a_checkbox_swallows_only_its_own_space() {
    // One frame: Space (+release) on a checkbox types " ", then the user's next keystroke
    // arrives. Only the checkbox's own " " may be swallowed — the following character must
    // still start an edit (#1951 review).
    let mut h = harness(checkbox_book());
    push(&mut h, Event::Key { key: Key::Space, physical_key: Some(Key::Space), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    push(&mut h, Event::Text(" ".to_string()));
    push(&mut h, Event::Key { key: Key::Space, physical_key: Some(Key::Space), pressed: false, repeat: false, modifiers: Modifiers::NONE });
    push(&mut h, Event::Text("x".to_string()));
    h.run_steps(2);
    assert_eq!(value(&h, "A1"), gridcraft_engine::core::Value::Bool(true), "the checkbox toggled");
    let ed = h.state().editor.clone().expect("the following letter starts an edit");
    assert!(ed.text.starts_with('x'), "the first character after the checkbox's space survives: {:?}", ed.text);
}
