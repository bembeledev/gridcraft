//! Stacked vertical text (`textRotation="255"`) end to end: the grid draws it as upright glyphs
//! reading down the cell, and Format Cells › Alignment keeps the mode and the sentinel itself.

use egui::Shape;
use egui_kittest::kittest::{NodeT, Queryable};
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::Alignment;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::grid::Geo;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const VERTICAL_TEXT: &str = "VERTICAL MODE TEST";

/// B2 stacked (255), B3 90°, B4 45° — the three cells of the reproduction fixture. Row 2 is tall
/// enough for the whole stack; the default one-line row would clip most of it.
fn harness() -> Harness {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "B2", "input": VERTICAL_TEXT})).unwrap();
    s.execute("home.orientation", json!({"range": "B2", "angle": "vertical"})).unwrap();
    s.execute("cell.set", json!({"cell": "B3", "input": "UP NINETY"})).unwrap();
    s.execute("home.orientation", json!({"range": "B3", "angle": 90})).unwrap();
    s.execute("cell.set", json!({"cell": "B4", "input": "FORTY FIVE"})).unwrap();
    s.execute("home.orientation", json!({"range": "B4", "angle": 45})).unwrap();
    s.execute("home.rowHeight", json!({"rows": "2:2", "height": 300.0})).unwrap();
    s.execute("selection.set", json!({"range": "H20"})).unwrap();
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

fn rotation(h: &Harness, cell: &str) -> i16 {
    let doc = h.state().session.active().unwrap();
    let sheet = doc.wb.active().unwrap();
    doc.wb.styles.get(sheet.style_id(CellRef::parse(cell).unwrap())).align.rotation
}

/// The on-screen rectangle of `cell`.
fn cell_rect(h: &Harness, cell: CellRef) -> egui::Rect {
    let app = h.state();
    let sheet = app.session.active().unwrap().wb.active().unwrap();
    Geo::new(sheet, app.grid.rect.unwrap(), app.view().scroll).cell_rect(sheet, cell)
}

/// The single-glyph shapes drawn inside `cell`, top to bottom: (x, y, angle in degrees). Stacked
/// vertical text is painted one character per galley; an ordinary cell paints its whole string in
/// one galley, so this finds nothing for it. The cell rect filters out the single letters of the
/// surrounding interface chrome.
fn stack(h: &Harness, cell: CellRef, text: &str) -> Vec<(f32, f32, f32)> {
    let r = cell_rect(h, cell);
    let mut out: Vec<(f32, f32, f32)> = h
        .output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.job.text.chars().count() == 1 && text.contains(t.galley.job.text.as_str()) && r.contains(t.pos) => {
                Some((t.pos.x, t.pos.y, t.angle.to_degrees()))
            }
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.total_cmp(&b.0)));
    out
}

/// The one shape inside `cell` holding the whole of `text`: (x, y, angle in degrees).
fn whole(h: &Harness, cell: CellRef, text: &str) -> Option<(f32, f32, f32)> {
    let r = cell_rect(h, cell);
    h.output().shapes.iter().find_map(|s| match &s.shape {
        Shape::Text(t) if t.galley.job.text == text && r.contains(t.pos) => Some((t.pos.x, t.pos.y, t.angle.to_degrees())),
        _ => None,
    })
}

/// Selects `cell` and opens Format Cells on the Alignment tab — what a user does to edit that cell.
fn open_alignment(h: &mut Harness, cell: &str) {
    h.state_mut().run("selection.set", json!({ "range": cell })).unwrap();
    h.state_mut().open_dialog("formatCells", json!({ "tab": "Alignment" }));
    h.run_steps(6);
}

fn confirm(h: &mut Harness) {
    h.get_by_label("   OK   ").click();
    h.run_steps(6);
}

/// Whether the "Vertical Text" checkbox is drawn as checked, read through accesskit.
fn vertical_checkbox(h: &Harness) -> bool {
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Vertical Text").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// B5–B9: the sentinel draws one upright glyph per line, top to bottom, all at one x, and it never
/// becomes a rotation: every angle is 0 and the stack grows downwards.
#[test]
fn a_vertical_cell_is_stacked_upright_from_top_to_bottom() {
    let h = harness();
    let lines = gridcraft_ui_egui::grid::vertical_text_lines(VERTICAL_TEXT);
    assert_eq!(lines.len(), VERTICAL_TEXT.chars().count(), "one line per character, spaces included");
    let b2 = CellRef::new(1, 1);
    let cell = cell_rect(&h, b2);
    let painted = h
        .output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            Shape::Text(t) if t.galley.job.text.chars().count() == 1 && cell.contains(t.pos) => {
                Some((t.galley.job.text.as_str(), (t.pos.x, t.pos.y, t.angle.to_degrees())))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        painted.iter().map(|p| p.0).collect::<Vec<_>>(),
        lines.iter().map(String::as_str).collect::<Vec<_>>(),
        "one galley per character, in reading order, inside {cell:?}"
    );

    let drawn: Vec<(f32, f32, f32)> = painted.iter().map(|p| p.1).collect();
    let ys: Vec<f32> = drawn.iter().map(|g| g.1).collect();
    let xs: Vec<f32> = drawn.iter().map(|g| g.0).collect();
    assert!(ys.windows(2).all(|w| w[1] > w[0]), "reading order runs downwards: {ys:?}");
    assert!(xs.windows(2).all(|w| (w[1] - w[0]).abs() < 0.5), "the stack keeps one x: {xs:?}");
    assert!(drawn.iter().all(|g| g.2.abs() < 1e-3), "no glyph is turned: {:?}", drawn.iter().map(|g| g.2).collect::<Vec<_>>());
    assert!(whole(&h, b2, VERTICAL_TEXT).is_none(), "the string is never painted as one sideways run");
    assert!(drawn.first().is_some_and(|g| g.1 >= cell.top()), "the stack starts inside the cell: {ys:?} vs {cell:?}");
    assert!(drawn.last().is_some_and(|g| g.1 < cell.bottom()), "and runs down inside the cell");
    assert!(ys.last().unwrap() - ys.first().unwrap() > cell.height() * 0.7, "the stack fills the tall row");
}

/// Numeric angles keep their own path: 90° and 45° are painted as one rotated galley.
#[test]
fn numeric_angles_still_paint_rotated_and_whole() {
    let h = harness();
    for (cell, text, degrees) in [(CellRef::new(2, 1), "UP NINETY", 90.0), (CellRef::new(3, 1), "FORTY FIVE", 45.0)] {
        assert!(stack(&h, cell, text).is_empty(), "{text} is not stacked into separate glyphs");
        let (_, _, angle) = whole(&h, cell, text).unwrap_or_else(|| panic!("{text} is drawn"));
        assert!((angle + degrees).abs() < 1e-3, "{text} is turned {degrees}° down (angle {angle})");
    }
    assert_eq!(rotation(&h, "B2"), Alignment::VERTICAL);
    assert_eq!(rotation(&h, "B3"), 90);
    assert_eq!(rotation(&h, "B4"), 45);
}

/// A short row clips the stack instead of turning it sideways.
#[test]
fn a_short_row_clips_the_stack_rather_than_rotating_it() {
    let mut h = harness();
    h.state_mut().run("home.rowHeight", json!({"rows": "2:2", "height": 30.0})).unwrap();
    h.run_steps(4);
    let b2 = CellRef::new(1, 1);
    let drawn = stack(&h, b2, VERTICAL_TEXT);
    assert!(drawn.len() < VERTICAL_TEXT.chars().count(), "the rest is clipped, not laid sideways: {}", drawn.len());
    assert!(!drawn.is_empty(), "the first glyphs are still shown");
    assert!(drawn.iter().all(|g| g.2.abs() < 1e-3), "still no rotation");
    assert!(drawn.windows(2).all(|w| w[1].1 > w[0].1), "still a downwards stack");
    assert!(whole(&h, b2, VERTICAL_TEXT).is_none(), "and never a horizontal fallback");
}

/// C10: opening the dialog on a stacked cell shows the mode as switched on.
#[test]
fn the_alignment_tab_recognises_the_vertical_mode() {
    let mut h = harness();
    open_alignment(&mut h, "B2");
    assert!(vertical_checkbox(&h), "a textRotation=255 cell shows Vertical Text as on");
}

/// C11: confirming the dialog untouched must not turn 255 into 0.
#[test]
fn confirming_without_changes_keeps_the_vertical_mode() {
    let mut h = harness();
    open_alignment(&mut h, "B2");
    confirm(&mut h);
    assert!(h.state().dialog.is_none());
    assert_eq!(rotation(&h, "B2"), Alignment::VERTICAL, "an untouched OK keeps textRotation=255");
}

/// C12: visiting another tab and coming back keeps it too.
#[test]
fn switching_tabs_and_back_keeps_the_vertical_mode() {
    let mut h = harness();
    open_alignment(&mut h, "B2");
    h.get_by_label("Font").click();
    h.run_steps(4);
    h.get_by_label("Alignment").click();
    h.run_steps(4);
    assert!(vertical_checkbox(&h), "coming back still shows the mode");
    confirm(&mut h);
    assert_eq!(rotation(&h, "B2"), Alignment::VERTICAL, "the sentinel survives the tab round trip");
}

/// C13: changing another alignment property keeps the orientation.
#[test]
fn editing_another_alignment_property_keeps_the_vertical_mode() {
    let mut h = harness();
    open_alignment(&mut h, "B2");
    h.get_by_label("Wrap text").click();
    h.run_steps(4);
    confirm(&mut h);
    let doc = h.state().session.active().unwrap();
    let sheet = doc.wb.active().unwrap();
    let align = doc.wb.styles.get(sheet.style_id(CellRef::new(1, 1))).align;
    assert!(align.wrap, "wrap was applied");
    assert_eq!(align.rotation, Alignment::VERTICAL, "and the orientation is untouched");
}

/// C14: the checkbox switches the sentinel on and off; off returns to ordinary 0°.
#[test]
fn the_vertical_checkbox_switches_the_sentinel_on_and_off() {
    let mut h = harness();
    // B4 is at 45°; turning the checkbox on replaces that angle with the sentinel.
    open_alignment(&mut h, "B4");
    assert!(!vertical_checkbox(&h), "an ordinary 45° cell shows Vertical Text as off");
    h.get_by_label("Vertical Text").click();
    h.run_steps(4);
    assert!(vertical_checkbox(&h), "the checkbox is now on");
    confirm(&mut h);
    assert_eq!(rotation(&h, "B4"), Alignment::VERTICAL, "on means textRotation=255");

    open_alignment(&mut h, "B4");
    assert!(vertical_checkbox(&h), "reopening shows the mode again");
    h.get_by_label("Vertical Text").click();
    h.run_steps(4);
    confirm(&mut h);
    assert_eq!(rotation(&h, "B4"), 0, "off returns to 0°, the documented default");
}

/// C15: dragging the degrees slider is a deliberate angle and leaves stacked mode.
#[test]
fn dragging_the_angle_leaves_the_vertical_mode() {
    let mut h = harness();
    open_alignment(&mut h, "B2");
    let from = h.get_by_role_and_label(egui::accesskit::Role::Slider, "Orientation (degrees)").rect().center();
    let to = egui::pos2(from.x + 90.0, from.y);
    let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    h.input_mut().events.extend([egui::Event::PointerMoved(from), press(from, true)]);
    h.step();
    for i in 1..=6 {
        h.input_mut().events.push(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        h.step();
    }
    h.input_mut().events.push(press(to, false));
    h.run_steps(4);
    assert!(!vertical_checkbox(&h), "an ordinary angle is not stacked mode");
    confirm(&mut h);
    let angle = rotation(&h, "B2");
    assert!((1..=90).contains(&angle), "the dragged angle replaced the sentinel: {angle}");
}

/// C16: Cancel applies nothing.
#[test]
fn cancel_does_not_apply_the_orientation_change() {
    let mut h = harness();
    open_alignment(&mut h, "B2");
    h.get_by_label("Vertical Text").click();
    h.run_steps(4);
    h.get_by_label(" Cancel ").click();
    h.run_steps(6);
    assert!(h.state().dialog.is_none());
    assert_eq!(rotation(&h, "B2"), Alignment::VERTICAL, "the cell is untouched");
}

/// C17: an ordinary angle confirmed without changes is left alone too.
#[test]
fn confirming_an_ordinary_angle_changes_nothing() {
    let mut h = harness();
    open_alignment(&mut h, "B4");
    assert!(!vertical_checkbox(&h));
    confirm(&mut h);
    assert_eq!(rotation(&h, "B4"), 45, "45° is not silently turned into 0° or 255");
}
