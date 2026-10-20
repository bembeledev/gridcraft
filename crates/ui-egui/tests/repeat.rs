//! F4, and Ctrl+Y with nothing to redo, repeat the last action on the selection, as in Excel.

use egui::{Event, Key, Modifiers};
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// Ctrl on Windows and Linux sets both `ctrl` and `command`.
const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };

fn harness() -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("selection.set", json!({"cell": "A1"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h
}

fn key(h: &mut Harness, k: Key, m: Modifiers) {
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
    h.run_steps(2);
}

fn bold(h: &mut Harness, cell: &str) -> bool {
    h.state_mut().session.execute("cell.get", json!({"cell": cell})).unwrap()["style"]["font"]["bold"] == true
}

fn select(h: &mut Harness, range: &str) {
    h.state_mut().session.execute("selection.set", json!({"range": range})).unwrap();
    h.run_steps(2);
}

#[test]
fn f4_and_ctrl_y_repeat_the_last_action() {
    let mut h = harness();
    key(&mut h, Key::F4, Modifiers::NONE);
    assert!(h.state().message.is_none(), "F4 with nothing to repeat stays quiet");
    key(&mut h, Key::B, CTRL);
    assert!(bold(&mut h, "A1"));
    select(&mut h, "C3");
    key(&mut h, Key::F4, Modifiers::NONE);
    assert!(bold(&mut h, "C3"), "F4 repeats Bold on the new selection");
    select(&mut h, "E5");
    key(&mut h, Key::Y, CTRL);
    assert!(bold(&mut h, "E5"), "Ctrl+Y with nothing to redo repeats too");
    // With something to redo Ctrl+Y redoes instead.
    key(&mut h, Key::Z, CTRL);
    assert!(!bold(&mut h, "E5"));
    select(&mut h, "G7");
    key(&mut h, Key::Y, CTRL);
    assert!(bold(&mut h, "E5") && !bold(&mut h, "G7"), "Ctrl+Y redid the undone step");
}
