//! The document chat in the interface: the title bar's chat button (in a session, or when the
//! file has a chat) with the unread count; the Chat pane opening from it, sending with Enter
//! (Shift+Enter: a new line), others' messages arriving, a mention of you flashing the button,
//! and Clear chat… asking first. The other person is an engine session connected in memory.

use egui::{Key, Modifiers, vec2};
use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use gridcraft_engine::collab::IN_MEMORY;
use gridcraft_engine::plugin::push::{Incoming, Pushed};
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};

type H = egui_kittest::Harness<'static, SheetApp>;

fn harness(session: Session) -> H {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1300.0, 760.0)).build_ui_state(
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

fn exchange(h: &mut H, bob: &mut Session) {
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

fn texts(s: &Session) -> Vec<String> {
    s.active().unwrap().wb.chat.messages.iter().map(|m| m.text.clone().unwrap_or_default()).collect()
}

/// Ann (the interface) shares a workbook; Bob joins.
fn session() -> (H, Session) {
    let mut s = Session::new();
    s.new_workbook();
    s.collab_join_in_memory(json!({"room": "chat-ui", "mode": "share", "user": {"id": "u-ann", "name": "Ann Lee", "color": "#3B7DD8"}})).unwrap();
    let mut h = harness(s);
    let mut bob = Session::new();
    bob.collab_join_in_memory(json!({"room": "chat-ui", "mode": "join", "user": {"id": "u-bob", "name": "Bob", "color": "#D8513B"}})).unwrap();
    deliver(&mut h.state_mut().session, vec![json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "chat-ui", "state": "online"}})]);
    exchange(&mut h, &mut bob);
    (h, bob)
}

#[test]
fn unread_count_pane_and_sending_with_enter() {
    let (mut h, mut bob) = session();
    assert!(h.query_by_label("Chat").is_some(), "in a session the title bar has the chat button");
    bob.execute("chat.send", json!({"text": "Hi Ann, totals look off"})).unwrap();
    bob.execute("chat.send", json!({"text": "Row 14 especially"})).unwrap();
    exchange(&mut h, &mut bob);
    h.run_steps(2);
    assert!(h.query_by_label("Chat, 2 unread").is_some(), "two unread messages from Bob");

    h.get_by_label("Chat, 2 unread").click();
    h.run_steps(3);
    assert!(gridcraft_ui_egui::chat_ui::open(h.state()), "the pane opened");
    assert!(h.query_by_label_contains("unread").is_none(), "opening it reads them");
    assert!(h.query_by_label_contains("Hi Ann, totals look off").is_some());

    // Typing and Enter sends; Shift+Enter makes a new line.
    h.get_by_label("Chat message").click();
    h.run_steps(1);
    h.get_by_label("Chat message").type_text("Checking now");
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::SHIFT, Key::Enter);
    h.run_steps(1);
    h.get_by_label("Chat message").type_text("one sec");
    h.run_steps(1);
    assert_eq!(h.state().chat.draft, "Checking now\none sec");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(h.state().chat.draft.is_empty(), "sent");
    exchange(&mut h, &mut bob);
    assert_eq!(texts(&bob).last().map(String::as_str), Some("Checking now\none sec"), "Bob got it");
    assert!(h.state().session.active().unwrap().undo.is_empty(), "no undo step");

    // Closed again: a mention of Ann flashes the button and counts as unread.
    h.state_mut().run("view.chat", json!({"on": false})).unwrap();
    h.run_steps(1);
    bob.execute("chat.send", json!({"text": "@Ann Lee thanks!"})).unwrap();
    exchange(&mut h, &mut bob);
    h.run_steps(2);
    assert!(h.query_by_label("Chat, 1 unread").is_some());
    assert!(h.state().chat.flash_at.is_some(), "a mention flashes the button");
}

#[test]
fn clear_chat_asks_first() {
    let (mut h, mut bob) = session();
    h.state_mut().run("chat.send", json!({"text": "one"})).unwrap();
    h.state_mut().run("view.chat", json!({})).unwrap();
    h.run_steps(2);
    h.get_by_label("Chat options").click();
    h.run_steps(2);
    assert!(h.query_by_label("Kept in the file; Office drops it when it saves").is_some(), "the menu says Office drops the chat");
    h.get_by_label("Clear chat…").click();
    h.run_steps(2);
    assert!(h.query_by_label("Remove the whole chat from the document for everyone?").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(texts(&h.state().session), ["one"], "Cancel keeps it");
    h.state_mut().chat.confirm_clear = true;
    h.run_steps(2);
    h.get_by_label("Clear chat").click();
    h.run_steps(2);
    assert!(texts(&h.state().session).is_empty());
    exchange(&mut h, &mut bob);
    assert!(texts(&bob).is_empty(), "cleared for everyone");
}

#[test]
fn a_file_with_a_chat_shows_the_button_outside_a_session() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = harness(s);
    assert!(h.query_by_label("Chat").is_none(), "no session, no chat: no button");
    h.state_mut().run("chat.send", json!({"text": "A note for whoever opens this"})).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label("Chat").is_some(), "the workbook has a chat");
    h.get_by_label("Chat").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("A note for whoever opens this").is_some());
}
