//! Format Cells in the desktop layout: the tabs draft a style, OK applies it to the selection and
//! Cancel leaves the sheet alone.

use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type H = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> H {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "B2", "input": "0.25"})).unwrap();
    s.execute("selection.set", json!({"range": "B2"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
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

fn open(h: &mut H, tab: &str) {
    h.state_mut().open_dialog("formatCells", json!({"tab": tab}));
    h.run_steps(4);
}

fn style(h: &H) -> gridcraft_engine::model::Style {
    let d = h.state().session.active().unwrap();
    let sh = d.wb.active().unwrap();
    d.wb.styles.get(sh.style_id(CellRef::parse("B2").unwrap())).clone()
}

fn click(h: &mut H, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

#[test]
fn number_fill_and_protection_tabs_apply_on_ok() {
    let mut h = harness();
    open(&mut h, "Number");
    click(&mut h, "Percentage");
    click(&mut h, "Fill");
    click(&mut h, "Yellow");
    click(&mut h, "Protection");
    click(&mut h, "Locked");
    click(&mut h, "OK");
    assert!(h.state().dialog.is_none());
    let st = style(&h);
    assert!(st.num_fmt.as_str().contains('%'), "{}", st.num_fmt.as_str());
    assert!(!st.protection.locked);
    assert_ne!(st.fill, Default::default());
}

#[test]
fn cancel_leaves_the_cell_alone() {
    let mut h = harness();
    let before = style(&h);
    open(&mut h, "Alignment");
    click(&mut h, "Wrap text");
    click(&mut h, "Cancel");
    assert!(h.state().dialog.is_none());
    assert_eq!(style(&h), before);
}

#[test]
fn the_size_box_takes_a_fractional_size() {
    let mut h = harness();
    open(&mut h, "Font");
    // The Size box sits right under its "Size:" label.
    let label = h.get_by_label("Size:").rect();
    let field = egui::pos2(label.left() + 30.0, label.bottom() + 12.0);
    h.input_mut().events.extend([
        egui::Event::PointerMoved(field),
        egui::Event::PointerButton { pos: field, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos: field, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    h.run_steps(2);
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.run_steps(1);
    // Typed one character a frame, as a person types: "10." must survive until the "5".
    for ch in ["1", "0", ".", "5"] {
        h.input_mut().events.push(egui::Event::Text(ch.into()));
        h.run_steps(1);
    }
    click(&mut h, "OK");
    assert!((style(&h).font.size - 10.5).abs() < 1e-6, "{}", style(&h).font.size);
}
