//! A press on a cell selects it when the button goes down, as in Excel (#900), and a double-click
//! on the fill handle fills down to the data next to it (#877).

use egui::{Event, Modifiers, PointerButton, Pos2};
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, RangeRef, Value};
use gridcraft_ui_egui::{SheetApp, grid::Geo};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(session: Session) -> Harness {
    // 60 frames a second, so two clicks a few frames apart are a double-click to egui.
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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

fn blank() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn geo(h: &Harness) -> Geo {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll)
}

fn rect(h: &Harness, a1: &str) -> egui::Rect {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    geo(h).cell_rect(sheet, CellRef::parse(a1).unwrap())
}

fn frame(h: &mut Harness, event: Event) {
    h.input_mut().events.push(event);
    h.run_steps(1);
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

fn down(h: &mut Harness, at: Pos2, modifiers: Modifiers) {
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    frame(h, Event::PointerMoved(at));
    frame(h, button(at, true, modifiers));
}

fn up(h: &mut Harness, at: Pos2, modifiers: Modifiers) {
    frame(h, button(at, false, modifiers));
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

/// Waits out the double-click window (the grid times it on the wall clock).
fn pause(h: &mut Harness) {
    std::thread::sleep(std::time::Duration::from_millis(500));
    h.run_steps(2);
}

fn selection(h: &Harness) -> (String, String, Vec<String>) {
    let s = &h.state().session.active().unwrap().selection;
    (s.active.a1(), s.anchor.a1(), s.ranges.iter().map(RangeRef::a1).collect())
}

fn value(h: &Harness, a1: &str) -> Value {
    h.state().session.active().unwrap().wb.active().unwrap().value(CellRef::parse(a1).unwrap())
}

#[test]
fn the_press_selects_before_the_release() {
    let mut h = harness(blank());
    let c3 = rect(&h, "C3").center();
    down(&mut h, c3, Modifiers::NONE);
    assert_eq!(selection(&h).0, "C3", "the cell is selected while the button is still down");
    up(&mut h, c3, Modifiers::NONE);
    assert_eq!(selection(&h), ("C3".into(), "C3".into(), vec!["C3".into()]));
}

#[test]
fn a_press_that_wobbles_inside_its_cell_still_selects_it() {
    let mut h = harness(blank());
    let r = rect(&h, "C3");
    let at = r.left_top() + egui::vec2(4.0, 4.0);
    down(&mut h, at, Modifiers::NONE);
    // Further than egui's click distance, but still inside C3: a drag that never leaves the cell.
    for step in 1..=6 {
        frame(&mut h, Event::PointerMoved(at + egui::vec2(step as f32 * 5.0, step as f32 * 1.5)));
    }
    up(&mut h, at + egui::vec2(30.0, 9.0), Modifiers::NONE);
    assert_eq!(selection(&h), ("C3".into(), "C3".into(), vec!["C3".into()]));
}

#[test]
fn a_long_press_without_moving_selects_the_cell() {
    let mut h = harness(blank());
    let c3 = rect(&h, "C3").center();
    down(&mut h, c3, Modifiers::NONE);
    // Longer than egui's longest click: the release is no longer a click.
    h.run_steps(60);
    up(&mut h, c3, Modifiers::NONE);
    assert_eq!(selection(&h).2, vec!["C3".to_string()]);
}

#[test]
fn dragging_from_the_press_extends_live_then_stays() {
    let mut h = harness(blank());
    let c3 = rect(&h, "C3").center();
    down(&mut h, c3, Modifiers::NONE);
    let e5 = rect(&h, "E5").center();
    frame(&mut h, Event::PointerMoved(c3.lerp(e5, 0.5)));
    frame(&mut h, Event::PointerMoved(e5));
    assert_eq!(selection(&h), ("C3".into(), "C3".into(), vec!["C3:E5".into()]), "the range follows the pointer while held");
    up(&mut h, e5, Modifiers::NONE);
    assert_eq!(selection(&h), ("C3".into(), "C3".into(), vec!["C3:E5".into()]));
}

#[test]
fn shift_and_ctrl_presses_act_on_the_press() {
    let mut h = harness(blank());
    let b2 = rect(&h, "B2").center();
    down(&mut h, b2, Modifiers::NONE);
    up(&mut h, b2, Modifiers::NONE);
    pause(&mut h);
    let d4 = rect(&h, "D4").center();
    down(&mut h, d4, Modifiers::SHIFT);
    assert_eq!(selection(&h).2, vec!["B2:D4".to_string()], "Shift+press extends at once");
    up(&mut h, d4, Modifiers::SHIFT);
    assert_eq!(selection(&h).2, vec!["B2:D4".to_string()]);
    pause(&mut h);
    let f6 = rect(&h, "F6").center();
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    down(&mut h, f6, ctrl);
    assert_eq!(selection(&h).2, vec!["B2:D4".to_string(), "F6".to_string()], "Ctrl+press adds an area at once");
    up(&mut h, f6, ctrl);
    assert_eq!(selection(&h).2, vec!["B2:D4".to_string(), "F6".to_string()], "and the release doesn't add it twice");
}

#[test]
fn a_checkbox_toggles_on_the_click_not_the_press() {
    let mut s = blank();
    s.execute("insert.checkbox", json!({"range": "B2"})).unwrap();
    let mut h = harness(s);
    let b2 = rect(&h, "B2").center();
    down(&mut h, b2, Modifiers::NONE);
    assert_eq!(selection(&h).0, "B2");
    assert_eq!(value(&h, "B2"), Value::Bool(false), "pressing doesn't toggle yet");
    up(&mut h, b2, Modifiers::NONE);
    assert_eq!(value(&h, "B2"), Value::Bool(true), "the click toggles once");
    pause(&mut h);
    // A press that is dragged off the cell is not a click.
    down(&mut h, b2, Modifiers::NONE);
    let d2 = rect(&h, "D2").center();
    frame(&mut h, Event::PointerMoved(d2));
    up(&mut h, d2, Modifiers::NONE);
    assert_eq!(value(&h, "B2"), Value::Bool(true));
    assert_eq!(selection(&h).2, vec!["B2:D2".to_string()]);
}

#[test]
fn double_click_still_edits_the_cell() {
    let mut s = blank();
    s.execute("cell.set", json!({"cell": "C3", "input": "hello"})).unwrap();
    let mut h = harness(s);
    let c3 = rect(&h, "C3").center();
    for _ in 0..2 {
        down(&mut h, c3, Modifiers::NONE);
        frame(&mut h, button(c3, false, Modifiers::NONE));
    }
    h.run_steps(2);
    let editor = h.state().editor.as_ref().expect("a double-click edits");
    assert_eq!((editor.cell.a1(), editor.text.as_str()), ("C3".to_string(), "hello"));
}

#[test]
fn a_click_on_the_selection_border_selects_the_cell_and_a_drag_moves_it() {
    let mut s = blank();
    s.execute("cell.set", json!({"cell": "B2", "input": "7"})).unwrap();
    s.execute("selection.set", json!({"range": "B2"})).unwrap();
    let mut h = harness(s);
    // On B2's right border, which belongs to the move handle: nothing happens on the press.
    let r = rect(&h, "B2");
    let border = egui::pos2(r.right(), r.center().y);
    down(&mut h, border, Modifiers::NONE);
    assert_eq!(selection(&h).0, "B2");
    let d4 = rect(&h, "D4").center();
    frame(&mut h, Event::PointerMoved(border + egui::vec2(20.0, 0.0)));
    frame(&mut h, Event::PointerMoved(d4));
    up(&mut h, d4, Modifiers::NONE);
    assert_eq!(value(&h, "B2"), Value::Empty, "dragging the border moved the cell");
    assert_eq!(value(&h, "D4"), Value::Number(7.0));
}

#[test]
fn double_clicking_the_fill_handle_fills_to_the_data_beside_it() {
    // The sheet from #877: 1..10 in A, 2, 4, 6, 8 in B1:B4.
    let mut s = blank();
    let a: Vec<Vec<f64>> = (1..=10).map(|i| vec![i as f64]).collect();
    s.execute("range.setValues", json!({"range": "A1", "values": a})).unwrap();
    s.execute("range.setValues", json!({"range": "B1", "values": [[2], [4], [6], [8]]})).unwrap();
    s.execute("selection.set", json!({"range": "B1:B4"})).unwrap();
    let mut h = harness(s);
    let handle = rect(&h, "B4").right_bottom() - egui::vec2(1.0, 1.0);
    frame(&mut h, Event::PointerMoved(handle));
    frame(&mut h, button(handle, true, Modifiers::NONE));
    frame(&mut h, button(handle, false, Modifiers::NONE));
    // A few frames between the clicks, as a real double-click has (still well inside egui's window).
    h.run_steps(4);
    frame(&mut h, button(handle, true, Modifiers::NONE));
    frame(&mut h, button(handle, false, Modifiers::NONE));
    h.run_steps(3);
    assert_eq!(value(&h, "B10"), Value::Number(20.0));
    assert_eq!(value(&h, "B11"), Value::Empty);
    assert_eq!(selection(&h).2, vec!["B1:B10".to_string()]);
    // One undo step.
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(value(&h, "B5"), Value::Empty);
    assert_eq!(value(&h, "B4"), Value::Number(8.0));
}

fn header_point(h: &Harness, col: Option<&str>, row: Option<u32>) -> Pos2 {
    let g = geo(h);
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    match (col, row) {
        (Some(c), _) => {
            let x = g.cell_rect(sheet, CellRef::parse(&format!("{c}1")).unwrap()).center().x;
            egui::pos2(x, g.cells.top() - g.header_h / 2.0)
        }
        (_, Some(r)) => {
            let y = g.cell_rect(sheet, CellRef::new(r - 1, 0)).center().y;
            egui::pos2(g.cells.left() - g.header_w / 2.0, y)
        }
        _ => unreachable!(),
    }
}

fn start_typing(h: &mut Harness, cell: &str, text: &str) {
    h.state_mut().session.execute("selection.set", json!({"cell": cell})).unwrap();
    h.run_steps(2);
    h.input_mut().events.push(Event::Text(text.into()));
    h.run_steps(3);
}

fn editor_text(h: &Harness) -> String {
    h.state().editor.as_ref().map(|e| e.text.clone()).unwrap_or_default()
}

fn enter(h: &mut Harness) {
    h.input_mut().events.push(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn formula(h: &Harness, a1: &str) -> Option<String> {
    let sheet = h.state().session.active().unwrap().wb.active().unwrap();
    sheet.cell(CellRef::parse(a1).unwrap()).and_then(|c| c.formula.as_ref().map(|f| f.text.clone()))
}

/// While typing a formula, the headings point like cells do, as observed in Excel: a column
/// heading puts `B:B` in, a drag across headings `B:D`, a row heading `3:3` (#872).
#[test]
fn headings_put_whole_columns_and_rows_into_a_formula() {
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "B1", "values": [[1, 10, 100], [2, 20, 200], [3, 30, 300]]})).unwrap();
    let mut h = harness(s);

    start_typing(&mut h, "F1", "=SUM(");
    let b = header_point(&h, Some("B"), None);
    down(&mut h, b, Modifiers::NONE);
    assert_eq!(editor_text(&h), "=SUM(B:B", "the press points");
    up(&mut h, b, Modifiers::NONE);
    assert_eq!(editor_text(&h), "=SUM(B:B");
    assert_eq!(h.state().session.active().unwrap().selection.active.a1(), "F1", "the selection stays on the edited cell");
    h.input_mut().events.push(Event::Text(")".into()));
    h.run_steps(2);
    enter(&mut h);
    assert_eq!(formula(&h, "F1").as_deref(), Some("SUM(B:B)"));
    assert_eq!(value(&h, "F1"), Value::Number(6.0));

    // A drag across the headings widens it, and back again.
    pause(&mut h);
    start_typing(&mut h, "F2", "=SUM(");
    let (b, c, d) = (header_point(&h, Some("B"), None), header_point(&h, Some("C"), None), header_point(&h, Some("D"), None));
    down(&mut h, b, Modifiers::NONE);
    frame(&mut h, Event::PointerMoved(c));
    frame(&mut h, Event::PointerMoved(d));
    assert_eq!(editor_text(&h), "=SUM(B:D");
    frame(&mut h, Event::PointerMoved(c));
    assert_eq!(editor_text(&h), "=SUM(B:C");
    up(&mut h, c, Modifiers::NONE);
    h.input_mut().events.push(Event::Text(")".into()));
    h.run_steps(2);
    enter(&mut h);
    assert_eq!(value(&h, "F2"), Value::Number(66.0));

    // A row heading.
    pause(&mut h);
    start_typing(&mut h, "F4", "=SUM(");
    let r3 = header_point(&h, None, Some(3));
    down(&mut h, r3, Modifiers::NONE);
    up(&mut h, r3, Modifiers::NONE);
    assert_eq!(editor_text(&h), "=SUM(3:3");
    h.input_mut().events.push(Event::Text(")".into()));
    h.run_steps(2);
    enter(&mut h);
    assert_eq!(value(&h, "F4"), Value::Number(333.0));

    // Not in point mode (after a reference was typed), a heading click ends the edit and selects.
    pause(&mut h);
    start_typing(&mut h, "F5", "=A1");
    let c = header_point(&h, Some("C"), None);
    down(&mut h, c, Modifiers::NONE);
    up(&mut h, c, Modifiers::NONE);
    assert_ne!(editor_text(&h), "=A1C:C");
}

/// Pointing at cells while typing a formula still works with presses: the press puts the
/// reference in, a drag widens it, and the keyboard stays with the formula.
#[test]
fn cells_point_into_a_formula_on_the_press() {
    let mut s = blank();
    s.execute("range.setValues", json!({"range": "B1", "values": [[1], [2], [3]]})).unwrap();
    let mut h = harness(s);
    start_typing(&mut h, "D1", "=SUM(");
    let (b1, b3) = (rect(&h, "B1").center(), rect(&h, "B3").center());
    down(&mut h, b1, Modifiers::NONE);
    assert_eq!(editor_text(&h), "=SUM(B1");
    frame(&mut h, Event::PointerMoved(b1.lerp(b3, 0.5)));
    frame(&mut h, Event::PointerMoved(b3));
    up(&mut h, b3, Modifiers::NONE);
    assert_eq!(editor_text(&h), "=SUM(B1:B3");
    h.input_mut().events.push(Event::Text(")".into()));
    h.run_steps(2);
    enter(&mut h);
    assert_eq!(formula(&h, "D1").as_deref(), Some("SUM(B1:B3)"));
    assert_eq!(value(&h, "D1"), Value::Number(6.0));
}
