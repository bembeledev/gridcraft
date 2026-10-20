//! Read-only drag benchmark: cargo run -p gridcraft-ui-egui --example perf_selection -- book.xlsx
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use egui::{Event, Modifiers, PointerButton, pos2};
    use gridcraft_engine::{Session, core::CellRef};
    use gridcraft_ui_egui::SheetApp;
    use serde_json::json;
    let path = std::env::args().nth(1).ok_or("workbook path required")?;
    let mut session = Session::new();
    session.execute("file.open", json!({"path": path}))?;
    session.execute("selection.set", json!({"cell":"A1"}))?;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1920.0, 1000.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.state_mut().view_mut().ok_or("no sheet view")?.scroll = egui::Vec2::ZERO;
    h.run_steps(5);
    h.ctx.all_styles_mut(|s| s.animation_time = 0.1);
    let app = h.state();
    let doc = app.session.active().ok_or("no document")?;
    let sh = doc.wb.active().ok_or("no sheet")?;
    let geo = app.grid.geo.as_ref().ok_or("no grid")?;
    let start = geo.cell_rect(sh, CellRef::new(1, 1)).center();
    if !geo.cells.contains(start) {
        return Err("drag start is outside the visible grid".into());
    }
    let end = pos2(geo.cells.right() - 60.0, geo.cells.bottom() - 80.0);
    h.input_mut().events.extend([
        Event::PointerMoved(start),
        Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
    ]);
    h.run_steps(1);
    let mut times = Vec::new();
    let mut stalled = 0;
    let mut changes = 0;
    for i in 1..=60 {
        let before = h.state().grid.glide.drawn();
        let old_range = h.state().session.active().ok_or("no document")?.selection.current();
        h.input_mut().events.push(Event::PointerMoved(start.lerp(end, i as f32 / 60.0)));
        let start_time = std::time::Instant::now();
        h.run_steps(1);
        times.push(start_time.elapsed().as_secs_f64() * 1000.0);
        if old_range != h.state().session.active().ok_or("no document")?.selection.current() {
            changes += 1;
            if before == h.state().grid.glide.drawn() {
                stalled += 1;
            }
        }
    }
    if changes == 0 {
        return Err("drag did not change the selection".into());
    }
    times.sort_by(f64::total_cmp);
    println!(
        "{}",
        json!({"frames":times.len(),"frameMedianMs":times.get(30),"frameP95Ms":times.get(57),"rangeChanges":changes,"stalledOutlineFrames":stalled})
    );
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
