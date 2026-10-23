//! Cmd+Shift+{ and Cmd+Shift+} switch sheets on the Mac, as the Mac's tabbed apps switch tabs.
//! Excel for Mac leaves the chords unbound, so this is a GridCraft extra; elsewhere Ctrl+Shift+[
//! and ] stay Excel's "select all precedents / dependents".

use egui::os::OperatingSystem;
use egui::{Event, Key, Modifiers, vec2};
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::keymap::{Shortcut, lookup};
use serde_json::json;

const CMD_SHIFT: Modifiers = Modifiers { alt: false, ctrl: false, shift: true, mac_cmd: true, command: true };
/// Ctrl on Windows and Linux sets both `ctrl` and `command`.
const CTRL_SHIFT: Modifiers = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: true };

#[test]
fn the_brace_chords_switch_sheets_on_the_mac_only() {
    assert_eq!(lookup(Key::OpenBracket, Some(Key::OpenBracket), CMD_SHIFT, true), Some(Shortcut::Run("sheet.previous", json!({}))));
    assert_eq!(lookup(Key::CloseBracket, Some(Key::CloseBracket), CMD_SHIFT, true), Some(Shortcut::Run("sheet.next", json!({}))));
    // Elsewhere the same physical chord is Excel's select-all-precedents / dependents.
    assert_eq!(
        lookup(Key::OpenBracket, Some(Key::OpenBracket), CTRL_SHIFT, false),
        Some(Shortcut::Run("formulas.selectPrecedents", json!({"all": true})))
    );
    assert_eq!(
        lookup(Key::CloseBracket, Some(Key::CloseBracket), CTRL_SHIFT, false),
        Some(Shortcut::Run("formulas.selectDependents", json!({"all": true})))
    );
    // And the Control key on the Mac keeps Excel for Mac's meaning too.
    let mac_ctrl_shift = Modifiers { command: false, mac_cmd: false, ..CTRL_SHIFT };
    assert_eq!(
        lookup(Key::OpenBracket, Some(Key::OpenBracket), mac_ctrl_shift, true),
        Some(Shortcut::Run("formulas.selectPrecedents", json!({"all": true})))
    );
}

#[test]
fn pressing_the_chords_moves_between_the_sheets_and_wraps_nowhere() {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_os(OperatingSystem::Mac).with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    let active = |h: &egui_kittest::Harness<'_, SheetApp>| h.state().session.active().unwrap().wb.active_sheet;
    let press = |h: &mut egui_kittest::Harness<'_, SheetApp>, k: Key| {
        h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: CMD_SHIFT });
        h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: CMD_SHIFT });
        h.run_steps(2);
    };
    assert_eq!(active(&h), 0);
    press(&mut h, Key::CloseBracket);
    assert_eq!(active(&h), 1);
    press(&mut h, Key::CloseBracket);
    assert_eq!(active(&h), 2);
    press(&mut h, Key::CloseBracket);
    assert_eq!(active(&h), 2, "the last sheet stays, as the sheet tabs do");
    press(&mut h, Key::OpenBracket);
    press(&mut h, Key::OpenBracket);
    assert_eq!(active(&h), 0);
    press(&mut h, Key::OpenBracket);
    assert_eq!(active(&h), 0);
    assert!(h.state().message.is_none() && h.state().toast.is_none());
}
