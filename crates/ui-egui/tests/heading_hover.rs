//! The row or column heading under the pointer is tinted, as in the desktop app, on left-to-right
//! and right-to-left sheets alike.

use egui::{Event, Pos2, Shape, pos2};
use gridcraft_engine::Session;
use gridcraft_ui_egui::{SheetApp, theme::Tokens};
use serde_json::json;

/// The tinted rectangle drawn this frame, if any.
fn tint(h: &egui_kittest::Harness<'static, SheetApp>, color: egui::Color32) -> Option<egui::Rect> {
    h.output().shapes.iter().find_map(|s| match &s.shape {
        Shape::Rect(r) if r.fill == color => Some(r.rect),
        _ => None,
    })
}

#[test]
fn the_heading_under_the_pointer_is_tinted() {
    let hover = Tokens::light().header_hover;
    assert_ne!(hover, Tokens::light().header_bg);
    assert_ne!(Tokens::dark().header_hover, Tokens::dark().header_bg);
    for rtl in [false, true] {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("view.rightToLeft", json!({"on": rtl})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
            |ui, app: &mut SheetApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            SheetApp::new(s, Default::default()),
        );
        h.run_steps(4);
        assert!(tint(&h, hover).is_none(), "nothing is tinted before the pointer arrives (rtl {rtl})");
        let cells = h.state().grid.cells_rect.expect("grid laid out");
        // A point on the column headings, a few columns in from the sheet's start.
        let x = if rtl { cells.right() - 64.0 * 2.5 } else { cells.left() + 64.0 * 2.5 };
        let at: Pos2 = pos2(x, cells.top() - 8.0);
        h.input_mut().events.push(Event::PointerMoved(at));
        h.run_steps(2);
        let tinted = tint(&h, hover).unwrap_or_else(|| panic!("a heading is tinted (rtl {rtl})"));
        assert!(tinted.contains(at), "the tinted heading {tinted:?} is the one under the pointer {at:?} (rtl {rtl})");
    }
}
