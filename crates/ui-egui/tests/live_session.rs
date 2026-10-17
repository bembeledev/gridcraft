//! A live session in the interface: the title bar shows it, others' edits arrive and are
//! highlighted, their selection is drawn (no panic), and Start Live Session (`collab.share`)
//! opens the Share dialog. The other person is a second engine session connected in memory.

use egui::vec2;
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::collab::IN_MEMORY;
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::plugin::push::{Incoming, Pushed};
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};

fn deliver(s: &mut Session, msgs: Vec<Json>) {
    let pushed = msgs.into_iter().map(|m| Pushed { plugin: IN_MEMORY.into(), incoming: Incoming::Message(m) }).collect();
    // The interface's clock, which its own pump uses too.
    s.collab_poll_pushed(gridcraft_ui_egui::now_ms() as u64, pushed);
}

/// Moves messages both ways until both sides are quiet.
fn exchange(h: &mut egui_kittest::Harness<'static, SheetApp>, bob: &mut Session) {
    for _ in 0..10 {
        let from_app = h.state_mut().session.collab_take_outgoing();
        let from_bob = bob.collab_take_outgoing();
        if from_app.is_empty() && from_bob.is_empty() {
            break;
        }
        deliver(bob, from_app);
        deliver(&mut h.state_mut().session, from_bob);
        h.run_steps(2);
    }
}

#[test]
fn a_live_session_shows_and_merges() {
    let mut session = Session::new();
    session.new_workbook();
    session.execute("cell.set", json!({"cell": "A1", "input": "Budget"})).unwrap();
    session.collab_join_in_memory(json!({"room": "ui", "mode": "share", "user": {"id": "u-ann", "name": "Ann", "color": "#3B7DD8"}})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(4);
    let mut bob = Session::new();
    bob.collab_join_in_memory(json!({"room": "ui", "mode": "join", "user": {"id": "u-bob", "name": "Bob", "color": "#D8513B"}})).unwrap();
    deliver(&mut h.state_mut().session, vec![json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "ui", "state": "online"}})]);
    exchange(&mut h, &mut bob);
    assert_eq!(bob.active().unwrap().wb.active().unwrap().value(CellRef::new(0, 0)), Value::Text("Budget".into()));
    h.run_steps(2);
    let _ = h.get_by_label("Share, live");

    // Bob types and selects: Ann sees the value (highlighted) and Bob in the room.
    bob.execute("cell.set", json!({"cell": "B2", "input": "42"})).unwrap();
    bob.execute("selection.set", json!({"range": "B2:C3"})).unwrap();
    bob.collab_poll(gridcraft_ui_egui::now_ms() as u64 + 1_000);
    exchange(&mut h, &mut bob);
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.session.active().unwrap().wb.active().unwrap().value(CellRef::new(1, 1)), Value::Number(42.0));
    assert!(app.collab_highlights.iter().any(|(_, _, c, _)| *c == CellRef::new(1, 1)), "the remote edit is highlighted");
    let room = app.session.active_room().unwrap();
    assert!(room.peers().iter().any(|p| p.user.name == "Bob" && p.selection.is_some()), "Bob and his selection are known");

    // Start Live Session opens the Share dialog (programmatic calls never do).
    h.state_mut().run("collab.share", json!({})).unwrap();
    h.state_mut().after_engine();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name().to_string()).as_deref(), Some("collabShare"));
}
