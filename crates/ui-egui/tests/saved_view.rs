//! Opening a workbook shows its saved view as Excel does: from the saved first cell, scrolled just
//! far enough to show the active cell, with the tab strip starting at the saved first tab.

use std::sync::Arc;

use egui::Shape;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::{Sheet, Workbook};
use gridcraft_ui_egui::{SheetApp, grid::Geo};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn open(wb: &Workbook, name: &str) -> Harness {
    let path = std::env::temp_dir().join(format!("gridcraft-saved-view-{}-{name}.xlsx", std::process::id()));
    std::fs::write(&path, gridcraft_xlsx::write_xlsx(wb).unwrap()).unwrap();
    let mut s = Session::new();
    s.execute("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&path);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h
}

fn book(top_left: &str, active: &str, freeze: Option<(u32, u32)>) -> Workbook {
    let mut wb = Workbook::new();
    let mut sh = Sheet::new("Data");
    sh.view_top_left = CellRef::parse(top_left).unwrap();
    sh.view_active = CellRef::parse(active).unwrap();
    sh.freeze = freeze;
    wb.sheets = vec![Arc::new(sh)];
    wb
}

/// The first cell shown in the scrolling pane, and whether the active cell is on screen.
fn shown(h: &Harness) -> (CellRef, bool) {
    let app = h.state();
    let sh = app.session.active().unwrap().wb.active().unwrap();
    let geo = Geo::new(sh, app.grid.rect.unwrap(), app.view().scroll);
    let first = geo.cell_at(sh, egui::pos2(geo.cells.left() + geo.frozen_w + 1.0, geo.cells.top() + geo.frozen_h + 1.0));
    let active = geo.cell_rect(sh, app.session.active().unwrap().selection.active);
    (first, geo.cells.contains(active.center()))
}

#[test]
fn active_cell_above_the_saved_first_cell_scrolls_up_to_it() {
    // Saved scrolled to row 71 with the active cell in row 10: Excel shows row 10 at the top.
    let h = open(&book("A71", "D10", None), "up");
    let (first, visible) = shown(&h);
    assert_eq!(first.row, 9, "{first:?}");
    assert!(visible);
}

#[test]
fn active_cell_below_the_screen_scrolls_down_just_enough() {
    // Frozen row 1 and columns A:B, scrolled to M2, active AH200: the row comes into view at the bottom.
    let h = open(&book("M2", "AH200", Some((1, 2))), "down");
    let (first, visible) = shown(&h);
    assert!(visible);
    assert!(first.row > 1 && first.row < 199, "scrolled just enough, not to the top: {first:?}");
}

#[test]
fn visible_active_cell_keeps_the_saved_first_cell() {
    let h = open(&book("C20", "D25", None), "keep");
    let (first, visible) = shown(&h);
    assert_eq!(first, CellRef::parse("C20").unwrap());
    assert!(visible);
}

fn painted(h: &Harness, text: &str) -> bool {
    h.output().shapes.iter().any(|s| matches!(&s.shape, Shape::Text(t) if t.galley.job.text == text))
}

#[test]
fn tab_strip_starts_at_the_saved_first_tab() {
    let mut wb = Workbook::new();
    wb.sheets = ["Alpha", "Beta", "Gamma", "Delta"].iter().map(|n| Arc::new(Sheet::new(*n))).collect();
    wb.first_tab = 2;
    wb.active_sheet = 3;
    let h = open(&wb, "tabs");
    assert!(!painted(&h, "Alpha") && !painted(&h, "Beta"), "tabs before the first tab are scrolled away");
    assert!(painted(&h, "Gamma") && painted(&h, "Delta"));
}
