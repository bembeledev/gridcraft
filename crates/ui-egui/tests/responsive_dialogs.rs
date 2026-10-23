//! Narrow windows must keep ribbon tabs and dialog actions reachable.
use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, TouchPhase, accesskit::Role, pos2, vec2};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

fn harness(width: f32, height: f32) -> egui_kittest::Harness<'static, SheetApp> {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(width, height)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(5);
    h
}

#[test]
fn narrow_ribbon_can_open_a_tab_hidden_on_the_right() {
    let mut h = harness(640.0, 500.0);
    h.get_by_role_and_label(Role::Button, "More tabs").click();
    h.run_steps(3);
    h.get_by_label("Automate").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.ribbon_tab, "Automate");
}

#[test]
fn analysis_actions_fit_a_low_resolution_screen() {
    let mut h = harness(900.0, 480.0);
    h.state_mut().open_dialog("analysisToolPak", json!({}));
    h.run_steps(6);
    for label in ["OK", "Cancel"] {
        let rect = h.get_by_role_and_label(Role::Button, label).rect();
        assert!(rect.top() >= 0.0 && rect.bottom() <= 480.0, "{label} outside screen: {rect:?}");
    }
}

#[test]
fn regression_range_picker_accepts_a_drag_on_the_worksheet() {
    let mut h = harness(1200.0, 800.0);
    h.state_mut().open_dialog("analysisToolPak", json!({"tool": "regression"}));
    h.run_steps(6);
    h.get_all_by_role_and_label(Role::Button, "Select…").next().unwrap().click();
    h.run_steps(3);
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    let geo = app.grid.geo.as_ref().unwrap();
    let start = geo.cell_rect(sheet, CellRef::new(0, 0)).center();
    let end = geo.cell_rect(sheet, CellRef::new(3, 1)).center();
    h.input_mut().events.extend([
        Event::PointerMoved(start),
        Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(2);
    h.input_mut().events.push(Event::PointerMoved(end));
    h.run_steps(3);
    h.input_mut().events.push(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let dialog = h.state().dialog.as_ref().unwrap();
    assert_eq!(dialog.values.get("inputY"), Some(&json!("A1:B4")));
    assert_eq!(h.state().session.active().unwrap().selection.current().a1(), "A1:B4");
}

#[test]
fn regression_actions_stay_put_while_the_form_scrolls() {
    let mut h = harness(900.0, 480.0);
    h.state_mut().open_dialog("analysisToolPak", json!({"tool":"regression"}));
    h.run_steps(6);
    let before = h.get_by_role_and_label(Role::Button, "OK").rect();
    let cancel_before = h.get_by_role_and_label(Role::Button, "Cancel").rect();
    h.input_mut().events.extend([
        Event::PointerMoved(pos2(450.0, 210.0)),
        Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, -700.0), phase: TouchPhase::Move, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(4);
    let after = h.get_by_role_and_label(Role::Button, "OK").rect();
    let cancel_after = h.get_by_role_and_label(Role::Button, "Cancel").rect();
    assert!((before.top() - after.top()).abs() < 1.0, "OK moved with the scroll area: {before:?} -> {after:?}");
    assert!((cancel_before.top() - cancel_after.top()).abs() < 1.0, "Cancel moved with the scroll area: {cancel_before:?} -> {cancel_after:?}");
    assert!(after.bottom() <= 480.0);
}
