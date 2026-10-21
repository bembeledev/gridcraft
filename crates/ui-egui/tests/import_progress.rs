//! Import progress keeps the normal workspace background and identifies the current sheet.
use egui_kittest::kittest::Queryable;
use gridcraft_engine::{
    Session,
    core::work::{WorkProgress, WorkStage},
};
use gridcraft_ui_egui::{ImportTask, Services, SheetApp};
use std::{cell::RefCell, rc::Rc};

#[test]
fn loading_retains_workspace_and_sheet_label_in_both_themes() {
    for (dark, open_at_start) in [(false, false), (true, false), (false, true), (true, true)] {
        let senders = Rc::new(RefCell::new(Vec::new()));
        let keep = senders.clone();
        let services = Services {
            start_import: Some(Box::new(move |_, _, _| {
                let (task, sender) = ImportTask::channel();
                *task.progress.lock().unwrap() =
                    WorkProgress { stage: WorkStage::Worksheet, completed: 25, total: 100, part: 2, parts: 3, sheet_name: Some("Premium".into()) };
                keep.borrow_mut().push(sender);
                task
            })),
            ..Default::default()
        };
        let mut session = Session::new();
        session.new_workbook();
        session.run("cell.set", serde_json::json!({"cell":"A1","input":"Keep this workspace"})).unwrap();
        let mut app = SheetApp::new(session, services);
        app.ui.dark = dark;
        if open_at_start {
            app.open_path("synthetic.xlsx");
        }
        let capture = std::env::var_os("GRIDCRAFT_IMPORT_SCREENSHOTS");
        let builder = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_theme(if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let builder = if capture.is_some() { builder.wgpu() } else { builder };
        let mut h = builder.build_ui_state(
            move |ui, app: &mut SheetApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        let workspace_text = |shapes: &[egui::epaint::ClippedShape]| {
            shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Keep this workspace"))
        };
        assert!(workspace_text(&h.output().shapes), "fixture cell must be visible before opening");
        let revision = h.state().session.active().unwrap().revision;
        if !open_at_start {
            h.state_mut().open_path("synthetic.xlsx");
        }
        h.run_steps(4);
        assert!(h.state().is_importing());
        h.get_by_label("Sheet 2 / 3: Premium");
        assert!(workspace_text(&h.output().shapes), "the existing grid must remain visible behind progress");
        // A replayed workspace is paint only, so existing cells cannot be edited during import.
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        if let Some(dir) = capture.filter(|_| !open_at_start) {
            let image = h.render().unwrap();
            image.save(std::path::PathBuf::from(dir).join(if dark { "import-dark.png" } else { "import-light.png" })).unwrap();
        }
    }
}
