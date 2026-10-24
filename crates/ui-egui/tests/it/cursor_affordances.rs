//! The worksheet pointer indicates cell selection and whole-row/column selection targets.

use egui::{CursorIcon, Event, Pos2, pos2, vec2};
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::grid::Geo;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    h
}

fn move_to(h: &mut Harness, pos: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(pos));
    h.run_steps(2);
}

#[test]
fn cell_and_header_hover_use_consistent_custom_cursors_in_both_directions() {
    for rtl in [false, true] {
        let mut h = harness();
        h.state_mut().session.execute("view.rightToLeft", serde_json::json!({"on": rtl})).unwrap();
        h.run_steps(2);
        let geo: Geo = h.state().grid.geo.clone().unwrap();
        let cell = {
            let sheet = h.state().session.active().unwrap().wb.active().unwrap();
            geo.screen_cell_rect(sheet, CellRef::parse("C3").unwrap()).center()
        };
        let column_header = pos2(cell.x, geo.head_top() + geo.header_h * 0.5);
        let row_header = pos2(geo.head_left() + geo.header_w * 0.5, cell.y);

        move_to(&mut h, geo.mirror_pos(cell));
        assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::None, "cell uses GridCraft's painted cross (rtl={rtl})");

        move_to(&mut h, geo.mirror_pos(column_header));
        assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::None, "column heading uses a directional arrow (rtl={rtl})");

        move_to(&mut h, geo.mirror_pos(row_header));
        assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::None, "row heading uses a directional arrow (rtl={rtl})");
    }
}
