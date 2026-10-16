//! A live session the hosting page runs (craft-live in Nextcloud, the web build): the title bar
//! gets a Close button (Nextcloud shows none) and File › Close closes the document through the
//! host; the session's popover names the host instead of a transport, shows its lines and offers
//! Close instead of Leave.

use std::cell::Cell;
use std::rc::Rc;

use egui::vec2;
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_ui_egui::{HostSession, Services, SheetApp};
use serde_json::json;

type H = egui_kittest::Harness<'static, SheetApp>;

fn harness(closed: Rc<Cell<u32>>) -> H {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": "Budget"})).unwrap();
    s.collab_serve_in_memory("craft-live");
    s.collab_join_in_memory(json!({"plugin": "craft-live", "room": "nc:cloud.test:288", "mode": "share", "user": {"id": "ada", "name": "Ada Lovelace", "color": "#3B7DD8"}}))
        .unwrap();
    let services = Services { close: Some(Box::new(move || closed.set(closed.get() + 1))), ..Default::default() };
    let mut app = SheetApp::new(s, services);
    app.host_session =
        Some(HostSession { label: "Nextcloud".into(), lines: vec!["Budget.xlsx in Nextcloud".into(), "All changes saved in Nextcloud".into()] });
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1300.0, 700.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

#[test]
fn the_host_closes_the_document() {
    let closed = Rc::new(Cell::new(0));
    let mut h = harness(closed.clone());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Close").click();
    h.run_steps(2);
    assert_eq!(closed.get(), 1, "the title bar's Close asks the host");
    // File › Close (and Cmd+W) go to the host too; the workbook stays until the host is done.
    assert!(h.state_mut().run("file.close", json!({})).is_ok());
    assert_eq!(closed.get(), 2);
    assert_eq!(h.state().session.documents().len(), 1);
}

#[test]
fn the_popover_shows_the_host_session() {
    let closed = Rc::new(Cell::new(0));
    let mut h = harness(closed.clone());
    h.get_by_label("Share, connecting").click();
    h.run_steps(2);
    assert!(h.query_by_label("Room nc:cloud.test:288 · Nextcloud").is_some(), "the host's name, not a transport's");
    assert!(h.query_by_label("Budget.xlsx in Nextcloud").is_some());
    assert!(h.query_by_label("All changes saved in Nextcloud").is_some());
    assert!(h.query_by_label("Leave").is_none(), "the host owns the session");
    let closes = h.query_all_by_label("Close").count();
    assert_eq!(closes, 2, "the title bar's and the popover's");
}

#[test]
fn without_a_host_there_is_no_close_button() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1300.0, 700.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        SheetApp::new(s, Default::default()),
    );
    h.run_steps(3);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Close").is_none());
}
