//! Read-only repeatable frame benchmark: cargo run -p gridcraft-ui-egui --example perf_workbook -- book.xlsx [frames]
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)] // Desktop-only benchmark; never executed on WASM.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let path = args.first().ok_or("workbook path required")?;
    let frames = args.get(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(40).clamp(4, 1000);
    let start = std::time::Instant::now();
    let mut session = Session::new();
    session.execute("file.open", json!({"path": path}))?;
    let opened = start.elapsed().as_secs_f64();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1920.0, 1000.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    harness.run_steps(5);
    let mut times = Vec::new();
    let mut grid = Vec::new();
    for i in 0..frames {
        if let Some(view) = harness.state_mut().view_mut() {
            view.scroll.y = (i * 90) as f32;
        }
        let start = std::time::Instant::now();
        harness.run_steps(1);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        grid.push(harness.state().perf.grid_ms);
    }
    times.sort_by(f64::total_cmp);
    grid.sort_by(f64::total_cmp);
    println!(
        "{}",
        json!({"openSeconds":opened,"frames":frames,"frameMedianMs":times.get(frames/2),"frameP95Ms":times.get(frames*95/100),"gridMedianMs":grid.get(frames/2),"gridP95Ms":grid.get(frames*95/100)})
    );
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
