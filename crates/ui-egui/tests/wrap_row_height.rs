//! A row of wrapped text grows to the text as the grid draws it: the app lends the engine its
//! fonts, so a word measured wider than the engine's estimate still gets its line.
use egui::{Color32, Context, FontId, RawInput, Rect, pos2, vec2};
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

const TEXT: &str = "Since the 1960s, NASA has left hardware across the solar system–on the Moon, on Mars, and in deep space.";

/// Runs a few frames of the app in a 1200 × 800 window.
fn frames(ctx: &Context, app: &mut SheetApp) {
    for _ in 0..4 {
        let mut output =
            ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 800.0))), ..Default::default() }, |ui| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            });
        output.textures_delta.clear();
    }
}

#[test]
fn a_wrapped_row_fits_the_text_as_the_grid_draws_it() {
    let ctx = Context::default();
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    frames(&ctx, &mut app);
    for chars in [6, 9, 14] {
        app.run("home.columnWidth", json!({"cols": "A:A", "chars": chars})).unwrap();
        app.run("home.wrapText", json!({"range": "A1", "on": true})).unwrap();
        app.run("range.setValues", json!({"range": "A1", "values": [[TEXT]]})).unwrap();
        frames(&ctx, &mut app);
        let sheet = app.session.active().unwrap().wb.active().unwrap().clone();
        // The grid wraps at the column's width less 2.5 px a side, in the cell font, and insets
        // wrapped text 1.5 px top and bottom.
        let st = sheet.style_id(gridcraft_engine::cell(0, 0));
        let st = app.session.active().unwrap().wb.styles.get(st).clone();
        let font = FontId::new(st.font.size * 96.0 / 72.0, gridcraft_ui_egui::theme::cell_family(&st.font.name, st.font.bold, st.font.italic));
        let wrap = sheet.col_width(0) - 5.0;
        let drawn = ctx.fonts_mut(|f| f.layout_job(egui::text::LayoutJob::simple(TEXT.into(), font, Color32::PLACEHOLDER, wrap)).size().y);
        let row = sheet.row_height(0);
        assert!(row >= drawn + 3.0, "{chars} chars: the row ({row}) shows every drawn line ({drawn} px)");
        assert!(row <= drawn + 8.0, "{chars} chars: and isn't taller than they need ({row} vs {drawn})");
    }
}
