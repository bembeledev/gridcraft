//! The middle mouse button pans the grid, AutoCAD style: the sheet follows the pointer from the
//! press on, nothing selects, the cursor is a grabbing hand and the pan stops dead on release.

use egui::{CursorIcon, Event, Modifiers, PointerButton, Pos2, vec2};
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness() -> Harness {
    let mut session = Session::new();
    session.new_workbook();
    crate::support::stepped_harness(session)
}

fn frame(h: &mut Harness, event: Event) {
    h.input_mut().events.push(event);
    h.run_steps(1);
}

fn move_to(h: &mut Harness, pos: Pos2) {
    frame(h, Event::PointerMoved(pos));
}

fn button(h: &mut Harness, pos: Pos2, button: PointerButton, pressed: bool) {
    frame(h, Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
}

/// A middle-button drag the way the window sends it: press, interpolated moves, release.
fn middle_drag(h: &mut Harness, from: Pos2, by: egui::Vec2) {
    move_to(h, from);
    button(h, from, PointerButton::Middle, true);
    for i in 1..=5 {
        move_to(h, from + by * (i as f32 / 5.0));
    }
    button(h, from + by, PointerButton::Middle, false);
}

/// The zoom factor screen points per sheet point, as the grid is drawn.
fn zoom(h: &Harness) -> f32 {
    h.state().grid.geo.as_ref().map_or(1.0, |geo| geo.z)
}

fn assert_a1_still_selected(h: &Harness) {
    let sel = &h.state().session.active().unwrap().selection;
    assert_eq!(sel.active, CellRef::new(0, 0), "panning must not move the active cell: {sel:?}");
    assert_eq!(sel.ranges.len(), 1, "{sel:?}");
    assert_eq!(sel.ranges[0].start, CellRef::new(0, 0), "{sel:?}");
    assert_eq!(sel.ranges[0].end, CellRef::new(0, 0), "panning must not select cells: {sel:?}");
}

#[test]
fn middle_drag_pans_the_grid_instead_of_selecting() {
    let mut h = harness();
    let cells = h.state().grid.cells_rect.unwrap();
    let z = zoom(&h);
    let from = cells.center() + vec2(0.0, 100.0);
    middle_drag(&mut h, from, vec2(0.0, -200.0));
    let scroll = h.state().view().scroll;
    assert!((scroll.y - 200.0 / z).abs() < 1.0, "the sheet follows the pointer exactly: scroll = {scroll:?}, z = {z}");
    assert_eq!(scroll.x, 0.0, "{scroll:?}");
    assert_a1_still_selected(&h);
    assert!(!h.state().grid.middle_pan, "the pan ends on release");

    let from = cells.center() + vec2(100.0, 0.0);
    middle_drag(&mut h, from, vec2(-200.0, 0.0));
    let scroll = h.state().view().scroll;
    assert!((scroll.x - 200.0 / z).abs() < 1.0, "dragging left scrolls right: scroll = {scroll:?}");
    assert_a1_still_selected(&h);
    assert!(!h.state().grid.middle_pan);
}

#[test]
fn a_middle_click_without_moving_leaves_the_view_alone() {
    let mut h = harness();
    let cells = h.state().grid.cells_rect.unwrap();
    let p = cells.center();
    move_to(&mut h, p);
    button(&mut h, p, PointerButton::Middle, true);
    button(&mut h, p, PointerButton::Middle, false);
    assert_eq!(h.state().view().scroll, egui::Vec2::ZERO);
    assert_a1_still_selected(&h);
    assert!(!h.state().grid.middle_pan);
}

#[test]
fn the_cursor_is_a_grabbing_hand_while_the_pan_lasts() {
    let mut h = harness();
    let cells = h.state().grid.cells_rect.unwrap();
    let from = cells.center();
    move_to(&mut h, from);
    button(&mut h, from, PointerButton::Middle, true);
    move_to(&mut h, from - vec2(0.0, 40.0));
    assert!(h.state().grid.middle_pan);
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Grabbing, "middle-button pan shows a grabbing hand");
    button(&mut h, from - vec2(0.0, 40.0), PointerButton::Middle, false);
    assert!(!h.state().grid.middle_pan);
}

/// A middle-button drag over a chart pans the sheet and leaves the chart where it is: the pan
/// swallows the drag before any object or cell drag can start.
#[test]
fn a_middle_drag_over_a_chart_does_not_move_it() {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    session.execute("cell.set", json!({"cell": "A2", "input": "2"})).unwrap();
    session.execute("selection.set", json!({"range": "A1:A2"})).unwrap();
    let chart = session.execute("insert.chart", json!({"type": "column", "at": "D3", "width": 300, "height": 200})).unwrap();
    let id = chart["chart"].as_u64().unwrap() as u32;
    let mut h = crate::support::stepped_harness(session);
    let geo = h.state().grid.geo.clone().unwrap();
    let sh = h.state().session.doc().unwrap().wb.active().unwrap();
    let anchor = sh.charts.iter().find(|c| c.id == id).unwrap().anchor;
    // The chart's top-left as it is drawn (see `chartview`), a little inside its frame.
    let start = geo.cell_rect(sh, anchor.cell).min + vec2(anchor.dx, anchor.dy) * geo.z + vec2(60.0, 60.0);
    middle_drag(&mut h, start, vec2(0.0, -150.0));
    assert!(h.state().view().scroll.y > 0.0, "the sheet pans under the chart");
    let sh = h.state().session.doc().unwrap().wb.active().unwrap();
    let moved = sh.charts.iter().find(|c| c.id == id).unwrap().anchor;
    assert_eq!(
        (moved.dx, moved.dy, moved.width, moved.height),
        (anchor.dx, anchor.dy, anchor.width, anchor.height),
        "a middle drag must not move a chart"
    );
    assert_ne!(h.state().selected_chart, Some(id), "a middle drag must not select the chart");
}
