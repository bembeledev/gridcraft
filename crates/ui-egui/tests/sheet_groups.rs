//! Sheet groups from the tab strip: Shift+click groups a run of sheets, Ctrl+click adds one, and
//! edits on the active sheet reach every grouped sheet.

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

fn click(h: &mut H, pos: Pos2, modifiers: Modifiers) {
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.extend([
        Event::PointerMoved(pos),
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers },
        Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers },
    ]);
    h.run_steps(3);
    h.input_mut().events.push(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

#[test]
fn shift_click_groups_sheets_and_edits_reach_every_one() {
    let mut h = harness();
    let n = names(&h);
    let last = tab(&h, &n[2]);
    click(&mut h, last, Modifiers::SHIFT);
    assert_eq!(h.state().session.active().unwrap().grouped, vec![0, 1, 2]);
    h.state_mut().run("cell.set", json!({"cell": "C3", "input": "42"})).unwrap();
    for sheet in &h.state().session.active().unwrap().wb.sheets {
        assert_eq!(sheet.value(CellRef::parse("C3").unwrap()), Value::number(42.0), "{}", sheet.name);
    }
    // A plain click when every sheet is grouped ungroups, as in Excel.
    let second = tab(&h, &n[1]);
    click(&mut h, second, Modifiers::NONE);
    assert!(h.state().session.active().unwrap().grouped.is_empty());
    assert_eq!(h.state().session.active().unwrap().wb.active_sheet, 1);
}

#[test]
fn ctrl_click_adds_one_sheet_to_the_group() {
    let mut h = harness();
    let n = names(&h);
    let third = tab(&h, &n[2]);
    click(&mut h, third, Modifiers::COMMAND);
    assert_eq!(h.state().session.active().unwrap().grouped, vec![0, 2]);
    h.state_mut().run("cell.set", json!({"cell": "A1", "input": "both"})).unwrap();
    let sheets = &h.state().session.active().unwrap().wb.sheets;
    assert_eq!(sheets[2].value(CellRef::parse("A1").unwrap()), Value::from("both"));
    assert!(sheets[1].value(CellRef::parse("A1").unwrap()).is_empty(), "a sheet outside the group is untouched");
}
