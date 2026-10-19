//! A rejected conditional-format formula stays available for correction (#1999).

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

#[test]
fn invalid_formula_keeps_the_dialog_open_until_corrected() {
    let mut session = Session::new();
    session.new_workbook();
    let mut app = SheetApp::new(session, Default::default());
    app.open_dialog("cfQuick", json!({"type": "expression"}));
    app.dialog.as_mut().unwrap().values.insert("formula".into(), json!("=A1>"));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("   OK   ").click();
    h.run_steps(3);
    assert!(h.state().session.doc().unwrap().wb.active().unwrap().cond_formats.is_empty());
    assert_eq!(h.state().dialog.as_ref().unwrap().values.get("formula"), Some(&json!("=A1>")));
    assert!(h.state().message.as_ref().unwrap().1.contains("formula"));

    h.state_mut().message = None;
    h.state_mut().dialog.as_mut().unwrap().values.insert("formula".into(), json!("=A1>5"));
    h.run_steps(3);
    h.get_by_label("   OK   ").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert!(h.state().message.is_none());
    assert_eq!(h.state().session.doc().unwrap().wb.active().unwrap().cond_formats.len(), 1);
}
