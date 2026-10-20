//! Read-only project indexing benchmark with an interactive frame on every poll.
use gridcraft_engine::Session;
use gridcraft_ui_egui::{Services, SheetApp};
use serde_json::json;
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)] // Desktop-only benchmark; never executed on WASM.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let root = args.first().ok_or("folder required")?;
    let mut s = Session::new();
    s.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1920.0, 1000.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        SheetApp::new(s, Services::default()),
    );
    h.state_mut().run("workspace.open", json!({"path": root}))?;
    let start = std::time::Instant::now();
    let mut times = Vec::new();
    let mut last = std::time::Instant::now();
    let mut requested = args.get(1).is_none();
    let mut open_ms = None;
    loop {
        let frame = std::time::Instant::now();
        h.run_steps(1);
        times.push(frame.elapsed().as_secs_f64() * 1000.0);
        if !requested && h.state().workspace.root().is_some() && !h.state().workspace.busy() {
            if let Some(path) = args.get(1) {
                let before = std::time::Instant::now();
                h.state_mut().run("workspace.openFile", json!({"path":path,"background":true}))?;
                open_ms = Some(before.elapsed().as_secs_f64() * 1000.0);
            }
            requested = true;
        }
        if !h.state().workspace.busy() && !h.state().patch_data.busy() {
            break;
        }
        if last.elapsed().as_secs() >= 5 {
            let info = h.state_mut().run("patch.inspect", json!({}))?;
            eprintln!("{}", info["progress"]);
            last = std::time::Instant::now();
        }
        if start.elapsed().as_secs() > 1800 {
            return Err("index did not complete within 30 minutes".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    times.sort_by(f64::total_cmp);
    let info = h.state_mut().run("patch.inspect", json!({}))?;
    println!(
        "{}",
        json!({"openRequestMs":open_ms,"seconds":start.elapsed().as_secs_f64(),"frames":times.len(),"frameMedianMs":times.get(times.len()/2),"frameP95Ms":times.get(times.len()*95/100),"frameMaxMs":times.last(),"index":info})
    );
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
