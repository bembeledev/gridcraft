//! The title bar's Share button and the people in a live session: Share opens the Share dialog
//! (which, with no transport add-in, points at Automate › Add-ins); in a session the button shows
//! its state, the people appear as circles (yourself last, at most four, then "+N"), clicking one
//! goes to their selection on their sheet, and the popover lists everyone and leaves. The other
//! people are engine sessions connected in memory (a fake transport).

use egui::vec2;
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::collab::IN_MEMORY;
use gridcraft_engine::plugin::push::{Incoming, Pushed};
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};

type H = egui_kittest::Harness<'static, SheetApp>;

fn harness(session: Session, width: f32) -> H {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 700.0)).build_ui_state(
        |ui, app: &mut SheetApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        SheetApp::new(session, Default::default()),
    );
    h.run_steps(3);
    h
}

fn deliver(s: &mut Session, msgs: Vec<Json>) {
    let pushed = msgs.into_iter().map(|m| Pushed { plugin: IN_MEMORY.into(), incoming: Incoming::Message(m) }).collect();
    s.collab_poll_pushed(gridcraft_ui_egui::now_ms() as u64, pushed);
}

/// Relays every side's messages to every other side until all are quiet.
fn exchange(h: &mut H, peers: &mut [Session]) {
    for _ in 0..12 {
        let mut all = vec![(usize::MAX, h.state_mut().session.collab_take_outgoing())];
        for (i, p) in peers.iter_mut().enumerate() {
            all.push((i, p.collab_take_outgoing()));
        }
        if all.iter().all(|(_, m)| m.is_empty()) {
            break;
        }
        for (from, msgs) in all {
            if from != usize::MAX {
                deliver(&mut h.state_mut().session, msgs.clone());
            }
            for (i, p) in peers.iter_mut().enumerate() {
                if i != from {
                    deliver(p, msgs.clone());
                }
            }
        }
        h.run_steps(1);
    }
}

const PEOPLE: [(&str, &str); 5] =
    [("Bob Rossi", "#D8513B"), ("Chiara", "#F2C94C"), ("Dan Okafor", "#2E9E5B"), ("Eve Park", "#9B4FD0"), ("Femi", "#1F9BB0")];

/// Ann shares a two-sheet workbook; `n` others join.
fn session_with(n: usize, width: f32) -> (H, Vec<Session>) {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": "Budget"})).unwrap();
    s.execute("home.insertSheet", json!({})).unwrap();
    // The second sheet has content (a peer's selection is placed by row and column ids, which an
    // empty sheet doesn't have yet).
    s.execute("cell.set", json!({"cell": "E42", "input": "Notes"})).unwrap();
    let second = s.active().unwrap().wb.active_sheet;
    s.execute("sheet.activate", json!({"sheet": 1 - second})).unwrap();
    s.collab_join_in_memory(json!({"room": "q4", "mode": "share", "user": {"id": "u-ann", "name": "Ann Lee", "color": "#3B7DD8"}})).unwrap();
    let mut h = harness(s, width);
    assert!(h.query_by_label("Share, connecting").is_some(), "before the transport says online, the button says connecting");
    let mut peers: Vec<Session> = PEOPLE
        .iter()
        .take(n)
        .enumerate()
        .map(|(i, (name, color))| {
            let mut p = Session::new();
            p.collab_join_in_memory(json!({"room": "q4", "mode": "join", "user": {"id": format!("u-{i}"), "name": name, "color": color}})).unwrap();
            p
        })
        .collect();
    deliver(&mut h.state_mut().session, vec![json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "q4", "state": "online"}})]);
    exchange(&mut h, &mut peers);
    h.run_steps(2);
    (h, peers)
}

#[test]
fn share_without_a_transport_points_at_add_ins() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = harness(s, 1200.0);
    // Not in a session: plain "Share", no circles.
    assert!(h.query_by_label("Share, live").is_none());
    h.get_by_label("Share").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name().to_string()).as_deref(), Some("collabShare"));
    let _ = h.get_by_label("A live session needs an add-in");
    let _ = h.get_by_label("sirmmo/craft-collab");
    h.get_by_label("  Add-ins…  ").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name().to_string()).as_deref(), Some("addIns"));
}

#[test]
fn a_session_shows_its_state_and_people_with_overflow() {
    let (mut h, mut peers) = session_with(5, 1200.0);
    assert!(h.query_by_label("Share, live").is_some(), "online");
    // Six people: three others, "+2", then you.
    for name in ["Bob Rossi", "Chiara", "Dan Okafor", "Ann Lee (you)"] {
        assert!(h.query_by_label(name).is_some(), "{name} has a circle");
    }
    assert!(h.query_by_label("+2").is_some(), "the overflow");
    assert!(h.query_by_label("Eve Park").is_none() && h.query_by_label("Femi").is_none(), "behind +2");

    // Bob selects on the second sheet; clicking his circle goes there and flashes it.
    let second = (0..2)
        .find(|&i| {
            h.state().session.active().unwrap().wb.sheets[i].value(gridcraft_engine::core::CellRef::new(41, 4))
                != gridcraft_engine::core::Value::Empty
        })
        .unwrap();
    assert_ne!(h.state().session.active().unwrap().wb.active_sheet, second);
    peers[0].execute("sheet.activate", json!({"sheet": second})).unwrap();
    peers[0].execute("selection.set", json!({"range": "D40:E42"})).unwrap();
    peers[0].collab_poll(gridcraft_ui_egui::now_ms() as u64 + 1_000);
    exchange(&mut h, &mut peers);
    h.get_by_label("Bob Rossi").click();
    h.run_steps(2);
    let app = h.state();
    assert_eq!(app.session.active().unwrap().wb.active_sheet, second, "switched to Bob's sheet");
    let flash = app.collab_flash.as_ref().expect("Bob's selection flashes");
    assert_eq!((flash.sheet, flash.ranges[0].a1()), (second, "D40:E42".to_string()));
    assert!(app.grid.reveal.is_none(), "the grid scrolled to it");
    assert_eq!(app.session.active().unwrap().selection.active.a1(), "A1", "your own selection stays");

    // The popover: everyone, and Leave.
    h.get_by_label("Share, live").click();
    h.run_steps(2);
    let _ = h.get_by_label("Femi");
    h.get_by_label("Leave").click();
    h.run_steps(3);
    assert!(h.state().session.active_room().is_none(), "left");
    assert!(h.query_by_label("Share").is_some() && h.query_by_label("Ann Lee (you)").is_none(), "back to plain Share, no circles");
}

#[test]
fn a_narrow_window_keeps_share_and_fewer_circles() {
    let (h, _peers) = session_with(5, 640.0);
    assert!(h.query_by_label("Share, live").is_some(), "Share always shows");
    let share = h.get_by_label("Share, live").rect();
    assert!(share.right() <= 640.0, "inside the window");
    // Whatever circles fit stay inside the window, left of Share, and don't overlap the left controls.
    // The left controls (AutoSave switch to More commands) end before x = 220 at this width.
    let left_end = 220.0;
    let names: Vec<String> = PEOPLE.iter().map(|(p, _)| p.to_string()).chain(["Ann Lee (you)".to_string()]).collect();
    let mut circles = 0;
    for name in &names {
        if let Some(n) = h.query_by_label(name) {
            let r = n.rect();
            assert!(r.left() > left_end && r.right() <= share.left(), "{name}'s circle at {r:?}");
            circles += 1;
        }
    }
    assert!(circles <= 4, "at most four circles");
}

#[test]
fn a_transport_invitation_is_offered_in_the_popover() {
    let (mut h, _peers) = session_with(1, 1200.0);
    h.state_mut().collab_invites.insert("q4".into(), "craft+mqtt://broker:1883/q4#k=abc".into());
    h.get_by_label("Share, live").click();
    h.run_steps(2);
    h.get_by_label("Copy invitation").click();
    h.run_steps(2);
    assert_eq!(h.state().status_message.as_ref().map(|m| m.0.as_str()), Some("Invitation copied"));
}
