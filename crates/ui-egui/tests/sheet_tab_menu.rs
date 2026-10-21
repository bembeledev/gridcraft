//! The sheet tab strip in the desktop layout: the right-click menu, the Move or Copy and
//! Insert/Delete dialogs, and dragging a tab (with Ctrl to copy it).

use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type H = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> H {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("home.insertSheet", json!({})).unwrap();
    s.execute("sheet.activate", json!({"sheet": 0})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
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

fn names(h: &H) -> Vec<String> {
    h.state().session.active().unwrap().wb.sheets.iter().map(|s| s.name.clone()).collect()
}

/// The tab of `name` in the tab strip at the bottom of the window.
fn tab(h: &H, name: &str) -> Pos2 {
    h.get_all_by_label(name).find(|n| n.rect().bottom() > 700.0).expect("tab").rect().center()
}

fn press(h: &mut H, pos: Pos2, button: PointerButton, modifiers: Modifiers) {
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.extend([
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button, pressed: true, modifiers },
        Event::PointerButton { pos, button, pressed: false, modifiers },
    ]);
    h.run_steps(3);
}

/// Right-clicks the tab and clicks `item` in its menu (which opens upwards, above the strip).
fn menu(h: &mut H, sheet: &str, item: &str) {
    let at = tab(h, sheet);
    press(h, at, PointerButton::Secondary, Modifiers::NONE);
    let node = h.get_all_by_label(item).find(|n| n.rect().top() > 250.0 && n.rect().bottom() < 760.0).expect("menu item");
    node.click();
    h.run_steps(3);
}

#[test]
fn the_menu_deletes_renames_colours_and_hides_sheets() {
    let mut h = harness();
    let n = names(&h);
    menu(&mut h, &n[1], "Delete");
    assert_eq!(names(&h), vec![n[0].clone(), n[2].clone()]);
    menu(&mut h, &n[2], "Rename");
    assert_eq!(h.state().grid.renaming_tab, Some(1));
    h.state_mut().grid.renaming_tab = None;
    h.run_steps(2);
    menu(&mut h, &n[0], "Hide");
    assert_ne!(h.state().session.active().unwrap().wb.sheets[0].visibility, gridcraft_engine::model::Visibility::Visible);
}

#[test]
fn tab_color_opens_the_palette_and_colours_the_tab() {
    let mut h = harness();
    let n = names(&h);
    menu(&mut h, &n[2], "Tab Color");
    h.get_by_label("Red").click();
    h.run_steps(3);
    assert!(h.state().session.active().unwrap().wb.sheets[2].tab_color.is_some());
}

#[test]
fn move_or_copy_copies_a_sheet_to_the_end() {
    let mut h = harness();
    let n = names(&h);
    h.state_mut().session.execute("cell.set", json!({"cell": "B2", "input": "kept"})).unwrap();
    menu(&mut h, &n[0], "Move or Copy…");
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name.as_str()), Some("moveSheet"));
    h.get_by_label("(move to end)").click();
    h.run_steps(2);
    h.get_by_label("Create a copy").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    let wb = &h.state().session.active().unwrap().wb;
    assert_eq!(wb.sheets.len(), 4);
    assert_eq!(wb.sheets[3].value(CellRef::parse("B2").unwrap()), Value::from("kept"), "the copy is the last sheet");
}

#[test]
fn delete_dialog_removes_an_entire_row() {
    let mut h = harness();
    h.state_mut().session.execute("range.setValues", json!({"range": "A1", "values": [["one"], ["two"]]})).unwrap();
    h.state_mut().session.execute("selection.set", json!({"range": "A1"})).unwrap();
    h.state_mut().open_dialog("deleteCells", json!({}));
    h.run_steps(3);
    h.get_by_label("Entire row").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    let sh = h.state().session.active().unwrap().wb.active().unwrap().clone();
    assert_eq!(sh.value(CellRef::parse("A1").unwrap()), Value::from("two"));
}

#[test]
fn ctrl_drag_copies_a_tab_and_plain_drag_moves_it() {
    let mut h = harness();
    let n = names(&h);
    let drag = |h: &mut H, from: Pos2, to: Pos2, modifiers: Modifiers| {
        h.input_mut().events.push(Event::ModifiersChanged(modifiers));
        h.input_mut()
            .events
            .extend([Event::PointerMoved(from), Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers }]);
        h.run_steps(2);
        for k in 1..=6 {
            let p = from + (to - from) * (k as f32 / 6.0);
            h.input_mut().events.push(Event::PointerMoved(p));
            h.run_steps(1);
        }
        h.input_mut().events.push(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers });
        h.run_steps(3);
        h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(1);
    };
    // Past the right half of the last tab: the end of the strip.
    let end = tab(&h, &n[2]) + egui::vec2(40.0, 0.0);
    let from = tab(&h, &n[0]);
    drag(&mut h, from, end, Modifiers::COMMAND);
    assert_eq!(names(&h).len(), 4, "Ctrl+drag copies");
    assert_eq!(names(&h)[..3], n[..]);
    let end = tab(&h, &names(&h)[3]) + egui::vec2(40.0, 0.0);
    let from = tab(&h, &n[0]);
    drag(&mut h, from, end, Modifiers::NONE);
    let after = names(&h);
    assert_eq!(after.len(), 4, "a plain drag moves");
    assert_eq!(after.last(), Some(&n[0]));
}

#[test]
fn an_access_key_runs_its_entry_without_typing_into_the_cell() {
    let mut h = harness();
    let n = names(&h);
    let at = tab(&h, &n[2]);
    press(&mut h, at, PointerButton::Secondary, Modifiers::NONE);
    // H is Hide's access key.
    h.input_mut().events.push(Event::Text("h".into()));
    h.run_steps(3);
    let d = h.state().session.active().unwrap();
    assert_ne!(d.wb.sheets[2].visibility, gridcraft_engine::model::Visibility::Visible, "H hid the sheet");
    assert!(h.state().editor.is_none(), "and the grid didn't start editing a cell with it");
}

#[test]
fn link_to_this_sheet_quotes_names_formulas_quote() {
    let mut h = harness();
    h.state_mut().session.execute("sheet.rename", json!({"sheet": 1, "name": "2024"})).unwrap();
    h.run_steps(3);
    let at = tab(&h, "2024");
    press(&mut h, at, PointerButton::Secondary, Modifiers::NONE);
    h.get_all_by_label("Link to this Sheet").find(|n| n.rect().top() > 250.0).expect("menu item").click();
    // The copy is in the output of the frame that handled the click.
    let mut copied = None;
    for _ in 0..3 {
        h.step();
        copied = copied.or_else(|| {
            h.output().platform_output.commands.iter().find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
        });
    }
    assert_eq!(copied.as_deref(), Some("#'2024'!A1"));
}

#[test]
fn access_key_labels_are_translated() {
    let mut h = harness();
    h.state_mut().ui.language = gridcraft_ui_egui::i18n::Language::PtBr;
    h.state_mut().open_dialog("deleteCells", json!({}));
    h.run_steps(3);
    assert!(h.query_by_label("Linha inteira").is_some(), "Entire &row is shown translated");
}

#[test]
fn more_colors_closes_the_menu_and_colours_the_tab() {
    let mut h = harness();
    let n = names(&h);
    menu(&mut h, &n[1], "Tab Color");
    h.get_by_label("More Colors…").click();
    h.run_steps(3);
    assert!(gridcraft_ui_egui::color_picker::is_open(&h.ctx));
    assert!(gridcraft_ui_egui::sheet_tab_menu::is_open(&h.ctx).is_none(), "the menu gave way to the Colors window");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().session.active().unwrap().wb.sheets[1].tab_color.is_some(), "the colour reached the sheet");
}

#[test]
fn every_option_of_the_right_to_left_delete_dialog_has_its_own_key() {
    let mut h = harness();
    h.state_mut().session.execute("view.rightToLeft", json!({"on": true})).unwrap();
    h.state_mut().open_dialog("deleteCells", json!({}));
    h.run_steps(3);
    h.input_mut().events.push(Event::Text("i".into()));
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().and_then(|d| d.values.get("shift")).and_then(|v| v.as_str()), Some("left"));
    h.input_mut().events.push(Event::Text("r".into()));
    h.run_steps(2);
    assert_eq!(h.state().dialog.as_ref().and_then(|d| d.values.get("shift")).and_then(|v| v.as_str()), Some("row"));
}
