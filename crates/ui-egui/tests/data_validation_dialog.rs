//! The Data Validation dialog: Settings, Input Message and Error Alert tabs, and Clear All.

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(s: Session) -> Harness {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.state_mut().open_dialog("dataValidation", json!({}));
    h.run_steps(4);
    h
}

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("selection.set", json!({"range": "B2:B4"})).unwrap();
    s
}

fn rules(h: &Harness) -> Vec<gridcraft_engine::model::Validation> {
    h.state().session.active().unwrap().wb.active().unwrap().validations.clone()
}

#[test]
fn the_tabs_set_the_messages_and_the_alert() {
    let mut h = harness(session());
    for (k, v) in [("formula1", json!("\"Sim,Não\"")), ("inputTitle", json!("Escolha")), ("errorTitle", json!("Ops"))] {
        h.state_mut().dialog.as_mut().unwrap().values.insert(k.into(), v);
    }
    assert!(h.query_by_label("Validation criteria").is_some(), "Settings is the first tab");
    h.get_by_label("Error Alert").click();
    h.run_steps(3);
    h.get_by_label("Show error alert after invalid data is entered").click();
    h.run_steps(3);
    h.get_by_label("Input Message").click();
    h.run_steps(3);
    assert!(h.query_by_label("Show input message when cell is selected").is_some());
    h.get_by_label_contains("OK").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    let dv = &rules(&h)[0];
    assert_eq!(dv.f1, "\"Sim,Não\"");
    assert_eq!((dv.input_title.as_str(), dv.error_title.as_str()), ("Escolha", "Ops"));
    assert!(dv.show_input && !dv.show_error, "unticking the alert switches it off");
}

#[test]
fn clear_all_removes_the_rule() {
    let mut s = session();
    s.execute("data.validation", json!({"type": "whole", "operator": "greater", "formula1": "5"})).unwrap();
    let mut h = harness(s);
    assert_eq!(h.state().dialog.as_ref().unwrap().values["type"], "whole", "opens on the selection's rule");
    h.get_by_label("Clear All").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert!(rules(&h).is_empty());
}
