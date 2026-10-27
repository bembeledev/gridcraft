//! A contextual ribbon tab (Table Design, Chart Design) only exists while its object is selected.
//! The tab remembered in the saved UI state, or left behind when the selection moves away, must
//! not leave the ribbon blank: it falls back to Home.

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

fn harness(session: Session) -> Harness {
    crate::support::harness(session, 4)
}

fn session_with_table() -> Session {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("range.setValues", json!({"range": "A1", "values": [["Name", "Qty"], ["a", 1], ["b", 2]]})).unwrap();
    session.execute("insert.table", json!({"range": "A1:B3", "header": true})).unwrap();
    session.execute("selection.set", json!({"range": "A2"})).unwrap();
    session
}

/// Sets a saved ribbon tab on a fresh app and builds the stepped harness, for unavailable-tab cases.
fn app_restored_on(tab: &str, session: Session) -> Harness {
    let mut app = crate::common::app(session);
    app.ui.ribbon_tab = tab.into();
    crate::support::app_harness(app, 4)
}

#[test]
fn a_saved_table_design_tab_without_a_table_opens_on_home() {
    let mut session = Session::new();
    session.new_workbook();
    let h = app_restored_on("Table Design", session);
    assert_eq!(h.state().ui.ribbon_tab, "Home");
}

#[test]
fn a_saved_chart_design_tab_without_a_chart_opens_on_home() {
    let mut session = Session::new();
    session.new_workbook();
    let h = app_restored_on("Chart Design", session);
    assert_eq!(h.state().ui.ribbon_tab, "Home");
}

#[test]
fn shape_format_is_available_only_while_a_shape_is_selected() {
    let mut session = Session::new();
    session.new_workbook();
    let result = session.execute("insert.shape", json!({"kind": "rectangle", "at": "B2"}));
    assert!(result.is_ok(), "test shape should be created");
    let id = result.ok().and_then(|value| value["shape"].as_u64()).and_then(|id| u32::try_from(id).ok());
    assert!(id.is_some(), "shape creation returns its object id");
    let Some(id) = id else { return };

    let mut h = harness(session);
    h.state_mut().selected_chart = Some(id);
    h.state_mut().ui.ribbon_tab = "Shape Format".into();
    h.run_steps(2);
    assert_eq!(h.state().ui.ribbon_tab, "Shape Format");
    assert!(h.query_by_label("Rotate selected shape 90 degrees").is_none());
    assert!(h.query_by_label("Bring object forward or to front").is_none());
    assert!(h.query_by_label("Send object backward or to back").is_none());
    h.get_by_label("Open Selection Pane").click();
    h.run_steps(2);
    assert!(h.query_by_label("Bring Forward").is_none());
    assert!(h.query_by_label("Send Backward").is_none());

    h.state_mut().selected_chart = None;
    h.run_steps(2);
    assert_eq!(h.state().ui.ribbon_tab, "Home");
}

#[test]
fn slicer_tab_is_available_only_while_a_slicer_is_selected() {
    let mut session = session_with_table();
    let result = session.execute("insert.slicer", json!({"table": "Table1", "column": "Name"}));
    assert!(result.is_ok(), "test slicer should be created");
    let id = result.ok().and_then(|value| value["slicer"].as_u64()).and_then(|id| u32::try_from(id).ok());
    assert!(id.is_some(), "slicer creation returns its object id");
    let Some(id) = id else { return };

    let mut h = harness(session);
    h.state_mut().selected_chart = Some(id);
    h.state_mut().ui.ribbon_tab = "Slicer".into();
    h.run_steps(2);
    assert_eq!(h.state().ui.ribbon_tab, "Slicer");

    h.state_mut().selected_chart = None;
    h.run_steps(2);
    assert_eq!(h.state().ui.ribbon_tab, "Home");
}

#[test]
fn picture_format_is_available_only_while_a_picture_is_selected() {
    let mut session = Session::new();
    session.new_workbook();
    let result = session.execute("insert.picture", json!({"base64": "AQID", "mime": "image/png", "at": "B2"}));
    assert!(result.is_ok(), "test picture should be created");
    let id = result.ok().and_then(|value| value["image"].as_u64()).and_then(|id| u32::try_from(id).ok());
    assert!(id.is_some(), "picture creation returns its object id");
    let Some(id) = id else { return };

    let mut h = harness(session);
    h.state_mut().selected_chart = Some(id);
    h.state_mut().ui.ribbon_tab = "Picture Format".into();
    h.run_steps(2);
    assert_eq!(h.state().ui.ribbon_tab, "Picture Format");

    h.state_mut().selected_chart = None;
    h.run_steps(2);
    assert_eq!(h.state().ui.ribbon_tab, "Home");
}

#[test]
fn leaving_the_table_while_on_table_design_returns_to_home() {
    let mut h = harness(session_with_table());
    h.state_mut().ui.ribbon_tab = "Table Design".into();
    h.run_steps(2);
    assert_eq!(h.state().ui.ribbon_tab, "Table Design", "the tab stays while the cell is in the table");
    h.state_mut().session.execute("selection.set", json!({"range": "D10"})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().ui.ribbon_tab, "Home");
}

#[test]
fn startup_opens_on_home_not_the_saved_tab() {
    let saved = gridcraft_ui_egui::UiState { ribbon_tab: "Table Design".into(), dark: true, ..Default::default() };
    let json = serde_json::to_string(&saved).unwrap();
    assert!(json.contains(r#""ribbonTab":"Table Design""#), "{json}");
    let restored = serde_json::from_str::<gridcraft_ui_egui::UiState>(&json).unwrap().at_startup();
    assert_eq!(restored.ribbon_tab, "Home", "the saved tab is not restored");
    assert!(restored.dark, "the other preferences are");
}

#[test]
fn an_ordinary_tab_stays_put_in_a_session() {
    let mut session = Session::new();
    session.new_workbook();
    let mut h = harness(session);
    h.state_mut().ui.ribbon_tab = "Data".into();
    h.run_steps(3);
    assert_eq!(h.state().ui.ribbon_tab, "Data");
}
