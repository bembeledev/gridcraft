use crate::{SheetApp, chart_tools};
use gridcraft_engine::Session;
use serde_json::json;

#[test]
fn saved_template_is_applied_to_future_charts_of_the_same_kind_and_persisted() {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("range.setValues", json!({"range":"A1", "values":[["X", "Y"], [1, 2], [2, 4]]})).unwrap();
    let mut app = SheetApp::new(session, Default::default());

    let first = app.run("insert.chart", json!({"type":"column"})).unwrap()["chart"].as_u64().unwrap() as u32;
    app.run("chart.set", json!({"chart":first, "format":{"border_width":2.5, "vertical_gridlines":true}})).unwrap();
    chart_tools::save_template(&mut app, first, "Thesis columns").unwrap();

    let second = app.run("insert.chart", json!({"type":"column"})).unwrap()["chart"].as_u64().unwrap() as u32;
    let chart = app.session.active().unwrap().wb.active().unwrap().charts.iter().find(|chart| chart.id == second).unwrap();
    assert_eq!(chart.format.border_width, Some(2.5));
    assert!(chart.format.vertical_gridlines);

    let encoded = serde_json::to_vec(&app.ui).unwrap();
    let restored: crate::UiState = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(restored.chart_templates.len(), 1);
    assert_eq!(restored.chart_templates[0].name, "Thesis columns");
    assert!(restored.chart_templates[0].use_for_new);
}
