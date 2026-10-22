//! General numbers fit their column as Excel counts it: an n-character column holds an n-digit
//! number although the drawn digits are a little wider than Excel's hinted 7 px digit, and a
//! number with more digits than that narrows (`1.23E+09`) instead.

use egui::Shape;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::grid::{Density, Geo};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(mac: bool) -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    // A and C keep the default 8.43-character width; B is 9 characters.
    s.execute("range.setValues", json!({"range": "A1:C1", "values": [[24959397, 100105891, 1234567890]]})).unwrap();
    s.execute("home.columnWidth", json!({"cols": "B:B", "chars": 9})).unwrap();
    s.execute("selection.set", json!({"range": "H20"})).unwrap();
    let mut app = SheetApp::new(s, Default::default());
    app.grid.mac_density = mac;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// Painted text shapes: (text, left x, width).
fn texts(h: &Harness) -> Vec<(String, f32, f32)> {
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Text(t) => Some((t.galley.job.text.clone(), t.pos.x, t.galley.size().x)),
            _ => None,
        })
        .collect()
}

fn check(mac: bool) {
    let h = harness(mac);
    let app = h.state();
    let wb = &app.session.active().unwrap().wb;
    let sheet = wb.active().unwrap();
    let geo = Geo::with_density(sheet, app.grid.rect.unwrap(), app.view().scroll, Density::of(wb, mac));
    let drawn = texts(&h);
    let inside = |cell: &str, text: &str| {
        let r = geo.cell_rect(sheet, CellRef::parse(cell).unwrap());
        drawn.iter().any(|(t, x, w)| t == text && *x >= r.left() - 0.01 && x + w <= r.right() + 0.01)
    };
    assert!(inside("A1", "24959397"), "8 digits fit a default column (mac: {mac}): {drawn:?}");
    assert!(inside("B1", "100105891"), "9 digits fit a 9-character column (mac: {mac}): {drawn:?}");
    assert!(inside("C1", "1.23E+09"), "10 digits narrow in a default column (mac: {mac}): {drawn:?}");
    assert!(!drawn.iter().any(|(t, _, _)| t.starts_with('#')), "no ####: {drawn:?}");
}

#[test]
fn general_numbers_fit_the_column_in_excel_digits() {
    check(false);
}

#[test]
fn general_numbers_fit_the_column_at_mac_density() {
    check(true);
}
