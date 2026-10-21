//! The colour palette: theme, tint and standard swatches, the no-colour row, and the Colors
//! window opened by More Colors…, whose answer reaches the caller through `take_result`.

use egui_kittest::kittest::Queryable;
use gridcraft_ui_egui::{color_picker, theme};

#[derive(Default)]
struct State {
    fonts: bool,
    picked: Option<String>,
    more: Option<String>,
}

const COLORS: [u32; 12] = [0xFFFFFF, 0x000000, 0xE7E6E6, 0x44546A, 0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5, 0x70AD47, 0x0563C1, 0x954F72];

fn harness(none_label: &'static str) -> egui_kittest::Harness<'static, State> {
    egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(
        move |ui, s: &mut State| {
            let ctx = ui.ctx().clone();
            if !s.fonts {
                // The interface fonts take effect from the next frame.
                ctx.set_fonts(theme::font_definitions());
                s.fonts = true;
                return;
            }
            if let Some(c) = color_picker::palette(ui, &COLORS, none_label, "test") {
                s.picked = Some(c);
            }
            color_picker::window(&ctx);
            if let Some(c) = color_picker::take_result(&ctx, "test") {
                s.more = Some(c);
            }
        },
        State::default(),
    )
}

#[test]
fn swatches_answer_their_colour() {
    let mut h = harness("No Fill");
    h.run_steps(2);
    h.get_by_label("Accent 1").click();
    h.run_steps(2);
    assert_eq!(h.state().picked.as_deref(), Some("#4472C4"));
    h.get_by_label("Red").click();
    h.run_steps(2);
    assert_eq!(h.state().picked.as_deref(), Some("#FF0000"));
    // Tints are lighter versions of the theme colour.
    h.get_by_label("Accent 1, Lighter 80%").click();
    h.run_steps(2);
    let tint = h.state().picked.clone().unwrap_or_default();
    assert!(tint.starts_with('#') && tint != "#4472C4", "{tint}");
    h.get_by_label("No Fill").click();
    h.run_steps(2);
    assert_eq!(h.state().picked.as_deref(), Some("none"));
}

#[test]
fn more_colors_answers_through_take_result_and_cancel_answers_nothing() {
    let mut h = harness("Automatic");
    h.run_steps(2);
    h.get_by_label("Automatic").click();
    h.run_steps(2);
    assert_eq!(h.state().picked.as_deref(), Some("none"));
    for ok in [false, true] {
        h.get_by_label("More Colors…").click();
        h.run_steps(3);
        h.get_by_label(if ok { "OK" } else { "Cancel" }).click();
        h.run_steps(3);
        assert_eq!(h.state().more.is_some(), ok, "OK answers a colour, Cancel nothing");
    }
    assert!(h.state().more.as_deref().is_some_and(|c| c.starts_with('#') && c.len() == 7));
}

fn app_harness() -> egui_kittest::Harness<'static, gridcraft_ui_egui::SheetApp> {
    let mut s = gridcraft_engine::Session::new();
    s.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut gridcraft_ui_egui::SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        gridcraft_ui_egui::SheetApp::new(s, Default::default()),
    );
    h.run_steps(4);
    h
}

fn fill(h: &egui_kittest::Harness<'static, gridcraft_ui_egui::SheetApp>) -> gridcraft_engine::model::Fill {
    let d = h.state().session.active().unwrap();
    let sh = d.wb.active().unwrap();
    d.wb.styles.get(sh.style_id(d.selection.active)).fill
}

#[test]
fn more_colors_in_format_cells_keeps_the_dialog_and_fills_the_cell() {
    let mut h = app_harness();
    h.state_mut().open_dialog("formatCells", serde_json::json!({"tab": "Fill"}));
    h.run_steps(3);
    // Escape cancels the Colors window alone: Format Cells stays open.
    h.get_by_label("More Colors…").click();
    h.run_steps(3);
    assert!(color_picker::is_open(&h.ctx), "the Colors window opens");
    assert!(h.state().dialog.is_some(), "More Colors… leaves Format Cells open");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!color_picker::is_open(&h.ctx));
    assert!(h.state().dialog.is_some(), "Escape closes the Colors window, not Format Cells");
    // OK hands the colour to the dialog's draft; the dialog's OK applies it to the cell.
    h.get_by_label("More Colors…").click();
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_some());
    assert_eq!(fill(&h), Default::default(), "nothing changes before the dialog's OK");
    // The dialog's own OK button.
    h.get_by_label("   OK   ").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_ne!(fill(&h), Default::default(), "the More Colors colour fills the cell");
}

#[test]
fn a_more_colors_answer_nobody_collects_expires() {
    let mut h = harness("No Fill");
    h.run_steps(2);
    h.get_by_label("More Colors…").click();
    h.run_steps(3);
    h.get_by_label("OK").click();
    // Taken in the next frames by the harness: one answer, once.
    h.run_steps(3);
    assert!(h.state().more.is_some());
    assert!(color_picker::take_result(&h.ctx, "test").is_none(), "an answer is handed out once");
}

#[test]
fn enter_in_the_colors_window_is_its_ok_and_not_the_dialogs() {
    let mut h = app_harness();
    h.state_mut().open_dialog("formatCells", serde_json::json!({"tab": "Fill"}));
    h.run_steps(3);
    h.get_all_by_label("More Colors…").next().expect("More Colors…").click();
    h.run_steps(3);
    assert!(color_picker::is_open(&h.ctx));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(!color_picker::is_open(&h.ctx), "Enter is the Colors window's OK");
    assert!(h.state().dialog.is_some(), "and Format Cells stays open with the colour in its draft");
    assert_eq!(fill(&h), Default::default(), "the cell changes only with the dialog's OK");
}
