//! Document chat (craft-collab `docs/chat.md`): the `chat.*` commands, their limits, the chat
//! outside undo but marking the workbook modified, saved and opened with XLSX and the native
//! format, and in live sessions (connected in memory): two and three people chatting at once
//! converge to one order, clearing, a newcomer receiving the chat, an encrypted room, a viewer's
//! messages and typing.

use std::sync::Arc;

use gridcraft_engine::collab::IN_MEMORY;
use gridcraft_engine::io::{FileKind, open_bytes, save_bytes_as};
use gridcraft_engine::model::chat::{CHAT_FULL, MAX_MESSAGES};
use gridcraft_engine::model::{ChatAuthor, ChatMessage};
use gridcraft_engine::plugin::push::{Incoming, Pushed};
use gridcraft_engine::{EngineError, Session};
use serde_json::{Value as Json, json};

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.prefs.user_name = "Ann Lee".into();
    s
}

fn texts(s: &Session) -> Vec<String> {
    s.doc().unwrap().wb.chat.messages.iter().map(|m| m.text.clone().unwrap_or_else(|| "(deleted)".into())).collect()
}

#[test]
fn send_list_edit_delete_clear_export() {
    let mut s = session();
    assert_eq!(s.execute("chat.list", json!({})).unwrap(), json!({"messages": [], "count": 0}));
    let a = s.execute("chat.send", json!({"text": "Totals first?"})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(gridcraft_engine::model::chat::valid_id(&a));
    let b = s.execute("chat.send", json!({"text": "Yes.\nThen the notes.", "replyTo": a})).unwrap()["id"].as_str().unwrap().to_string();
    let l = s.execute("chat.list", json!({})).unwrap();
    assert_eq!(l["count"], 2);
    assert_eq!(l["messages"][0]["author"]["name"], "Ann Lee", "outside a session, the app's own user");
    assert!(l["messages"][0]["at"].as_i64().unwrap() > 1_700_000_000_000);
    assert_eq!(l["messages"][1]["replyTo"], json!(a));
    assert_eq!(s.execute("chat.list", json!({"since": a})).unwrap()["messages"].as_array().unwrap().len(), 1);
    assert_eq!(s.execute("chat.list", json!({"limit": 1})).unwrap()["messages"][0]["id"], json!(b));
    let at_a = l["messages"][0]["at"].as_i64().unwrap();
    assert_eq!(s.execute("chat.list", json!({"since": at_a - 1})).unwrap()["messages"].as_array().unwrap().len(), 2, "since a time");
    assert_eq!(s.execute("chat.list", json!({"since": 4_000_000_000_000i64})).unwrap()["messages"].as_array().unwrap().len(), 0);

    s.execute("chat.edit", json!({"id": a, "text": "Totals first? (edited)"})).unwrap();
    assert!(s.doc().unwrap().wb.chat.get(&a).unwrap().edited.is_some());
    s.execute("chat.delete", json!({"id": b})).unwrap();
    assert_eq!(texts(&s), ["Totals first? (edited)", "(deleted)"]);
    assert!(s.execute("chat.edit", json!({"id": b, "text": "back"})).is_err(), "a deleted message stays deleted");

    let text = s.execute("chat.export", json!({})).unwrap();
    assert!(text["text"].as_str().unwrap().contains("Ann Lee: Totals first? (edited) (edited)"), "{text}");
    assert!(text["text"].as_str().unwrap().contains("Message deleted"));
    let json = s.execute("chat.export", json!({"format": "json"})).unwrap();
    assert_eq!(json["chat"]["messages"].as_array().unwrap().len(), 2);
    assert!(s.execute("chat.export", json!({"format": "pdf"})).is_err());

    assert_eq!(s.execute("chat.clear", json!({})).unwrap(), json!({"removed": 2}));
    let l = s.execute("chat.list", json!({})).unwrap();
    assert_eq!(l["count"], 0);
    assert!(l["clearedAt"].is_i64());
}

#[test]
fn bad_params_are_refused() {
    let mut s = session();
    for (cmd, p) in [
        ("chat.send", json!({})),
        ("chat.send", json!({"text": ""})),
        ("chat.send", json!({"text": "x".repeat(4_001)})),
        ("chat.send", json!({"text": "hi", "replyTo": "zzzzzzzz"})),
        ("chat.edit", json!({"id": "zzzzzzzz", "text": "x"})),
        ("chat.edit", json!({"text": "x"})),
        ("chat.delete", json!({"id": "zzzzzzzz"})),
        ("chat.delete", json!({})),
    ] {
        assert!(matches!(s.execute(cmd, p.clone()), Err(EngineError::BadParams { .. })), "{cmd} {p}");
    }
    assert!(s.execute("chat.send", json!({"text": "é".repeat(4_000)})).is_ok(), "4,000 UTF-16 units fit");
    // Someone else's message: only its author changes it.
    let d = s.doc_mut().unwrap();
    let mut chat = (*d.wb.chat).clone();
    let bob = ChatAuthor { id: "u-bob".into(), name: "Bob".into(), color: "#D8513B".into() };
    chat.send(ChatMessage { id: "bbbbbbbb".into(), author: bob, at: 5, text: Some("mine".into()), ..Default::default() }).unwrap();
    Arc::make_mut(&mut d.wb).chat = Arc::new(chat);
    assert!(s.execute("chat.edit", json!({"id": "bbbbbbbb", "text": "not yours"})).is_err());
    assert!(s.execute("chat.delete", json!({"id": "bbbbbbbb"})).is_err());
}

#[test]
fn the_limit_is_ten_thousand() {
    let mut s = session();
    let d = s.doc_mut().unwrap();
    let mut chat = (*d.wb.chat).clone();
    let a = ChatAuthor { id: "u-x".into(), name: "X".into(), color: "#000000".into() };
    chat.messages = (0..MAX_MESSAGES)
        .map(|i| ChatMessage { id: format!("{i:08}"), author: a.clone(), at: 1, text: Some("m".into()), ..Default::default() })
        .collect();
    Arc::make_mut(&mut d.wb).chat = Arc::new(chat);
    let e = s.execute("chat.send", json!({"text": "one more"})).unwrap_err().to_string();
    assert!(e.contains(CHAT_FULL), "{e}");
    s.execute("chat.clear", json!({})).unwrap();
    s.execute("chat.send", json!({"text": "one more"})).unwrap();
    assert_eq!(texts(&s), ["one more"]);
}

#[test]
fn chat_is_not_undone_but_marks_the_workbook_modified() {
    let mut s = session();
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("chat.send", json!({"text": "a note"})).unwrap();
    assert!(s.doc().unwrap().is_dirty(), "the chat is kept only when the file is saved");
    assert!(s.doc().unwrap().undo.is_empty(), "sending isn't an undo step");
    s.execute("cell.set", json!({"cell": "A1", "input": "7"})).unwrap();
    s.execute("chat.send", json!({"text": "after the edit"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.doc().unwrap().wb.sheets[0].cells.get(gridcraft_engine::core::CellRef::new(0, 0)).is_none(), "the cell edit is undone");
    assert_eq!(texts(&s), ["a note", "after the edit"], "Undo never removes a message");
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(texts(&s), ["a note", "after the edit"]);
    s.execute("chat.clear", json!({})).unwrap();
    s.execute("cell.set", json!({"cell": "B1", "input": "8"})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(texts(&s).is_empty(), "nor brings back a cleared chat");
}

#[test]
fn saved_and_opened_with_xlsx_and_json() {
    let mut s = session();
    let a = s.execute("chat.send", json!({"text": "Kept in the file"})).unwrap()["id"].as_str().unwrap().to_string();
    s.execute("chat.send", json!({"text": "A reply", "replyTo": a})).unwrap();
    let wb = s.doc().unwrap().wb.clone();
    for (name, kind) in [("book.xlsx", FileKind::Xlsx), ("book.xlsm", FileKind::Xlsx), ("book.json", FileKind::Json)] {
        let bytes = save_bytes_as(&wb, name, kind).unwrap();
        let (back, warnings) = open_bytes(name, &bytes).unwrap();
        assert_eq!(*back.chat, *wb.chat, "{name}");
        assert!(!warnings.iter().any(|w| w.contains("chat")), "{name}: {warnings:?}");
    }
    // A workbook without a chat writes none.
    let plain = save_bytes_as(&gridcraft_engine::model::Workbook::new(), "plain.json", FileKind::Json).unwrap();
    assert!(!String::from_utf8_lossy(&plain).contains("\"chat\""));
}

// ---------------------------------------------------------------- live sessions

fn user(id: &str, name: &str, color: &str) -> Json {
    json!({"id": id, "name": name, "color": color})
}

struct Room {
    people: Vec<Session>,
    now: u64,
}

impl Room {
    /// The first person shares their workbook (`key`: an encrypted room); the others join.
    fn new(first: Session, names: &[(&str, &str)], key: Option<&str>, viewer: Option<usize>) -> Room {
        let mut people = Vec::new();
        let mut first = Some(first);
        for (i, (name, color)) in names.iter().enumerate() {
            let mut p = first.take().unwrap_or_default();
            let mut params =
                json!({"room": "chat-room", "mode": if i == 0 { "share" } else { "join" }, "user": user(&format!("u-{i}"), name, color)});
            if let Some(k) = key {
                params["key"] = json!(k);
            }
            if viewer == Some(i) {
                params["role"] = json!("view");
            }
            p.collab_join_in_memory(params).unwrap();
            people.push(p);
        }
        let mut r = Room { people, now: 1_000 };
        for i in 0..r.people.len() {
            r.deliver(i, vec![json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "chat-room", "state": "online"}})]);
        }
        r.exchange();
        r
    }

    fn deliver(&mut self, to: usize, msgs: Vec<Json>) {
        self.now += 10;
        let pushed = msgs.into_iter().map(|m| Pushed { plugin: IN_MEMORY.into(), incoming: Incoming::Message(m) }).collect();
        self.people[to].collab_poll_pushed(self.now, pushed);
    }

    /// Relays everything to everyone until all are quiet.
    fn exchange(&mut self) {
        for _ in 0..20 {
            let out: Vec<Vec<Json>> = self.people.iter_mut().map(|p| p.collab_take_outgoing()).collect();
            if out.iter().all(Vec::is_empty) {
                return;
            }
            for (from, msgs) in out.into_iter().enumerate() {
                let msgs: Vec<Json> =
                    msgs.into_iter().filter(|m| m.get("method").is_some_and(|x| x != "collab/joined" && x != "collab/leave")).collect();
                for to in 0..self.people.len() {
                    if to != from && !msgs.is_empty() {
                        self.deliver(to, msgs.clone());
                    }
                }
            }
        }
    }

    fn texts(&self, i: usize) -> Vec<String> {
        texts(&self.people[i])
    }
}

#[test]
fn two_and_three_people_chatting_at_once_converge() {
    let mut first = session();
    first.execute("chat.send", json!({"text": "A note from before"})).unwrap();
    let mut r = Room::new(first, &[("Ann", "#3B7DD8"), ("Bob", "#D8513B"), ("Cy", "#2E9E5B")], None, None);
    assert_eq!(r.texts(1), ["A note from before"], "sharing seeds the room's chat from the workbook; joining brings it in");
    assert_eq!(r.texts(2), ["A note from before"]);
    // Everyone sends at once, twice, before anything is delivered.
    for round in 0..2 {
        for (i, p) in r.people.iter_mut().enumerate() {
            p.execute("chat.send", json!({"text": format!("{i}:{round}")})).unwrap();
        }
    }
    r.exchange();
    let order = r.texts(0);
    assert_eq!(order.len(), 7);
    assert_eq!(r.texts(1), order, "one order for everyone");
    assert_eq!(r.texts(2), order);
    let msgs = &r.people[1].doc().unwrap().wb.chat.messages;
    assert_eq!(msgs.iter().find(|m| m.text.as_deref() == Some("1:0")).unwrap().author.name, "Bob");
    // Bob edits and deletes his own; Ann's copy follows; Bob can't touch Ann's.
    let bobs = msgs.iter().find(|m| m.text.as_deref() == Some("1:1")).unwrap().id.clone();
    let anns = msgs.iter().find(|m| m.text.as_deref() == Some("0:0")).unwrap().id.clone();
    r.people[1].execute("chat.edit", json!({"id": bobs, "text": "1:1 (better)"})).unwrap();
    assert!(r.people[1].execute("chat.delete", json!({"id": anns})).is_err());
    r.exchange();
    assert!(r.texts(0).contains(&"1:1 (better)".to_string()));
    assert!(r.people[0].doc().unwrap().is_dirty(), "others' messages mark the workbook modified");
    // A newcomer gets the room's chat.
    let mut d = Session::new();
    d.collab_join_in_memory(json!({"room": "chat-room", "mode": "join", "user": user("u-9", "Dee", "#9B4FD0")})).unwrap();
    r.people.push(d);
    r.deliver(3, vec![json!({"jsonrpc": "2.0", "method": "collab/status", "params": {"room": "chat-room", "state": "online"}})]);
    r.exchange();
    assert_eq!(r.texts(3), r.texts(0));
    // Undo in a session is per person and leaves the chat alone.
    r.people[0].execute("cell.set", json!({"cell": "A1", "input": "x"})).unwrap();
    r.people[0].execute("chat.send", json!({"text": "after x"})).unwrap();
    r.people[0].execute("edit.undo", json!({})).unwrap();
    r.exchange();
    assert_eq!(r.texts(0).last().map(String::as_str), Some("after x"));
    assert_eq!(r.texts(2), r.texts(0));
}

#[test]
fn clearing_removes_the_chat_for_everyone_and_keeps_what_was_sent_meanwhile() {
    let mut r = Room::new(session(), &[("Ann", "#3B7DD8"), ("Bob", "#D8513B")], None, None);
    r.people[0].execute("chat.send", json!({"text": "one"})).unwrap();
    r.people[1].execute("chat.send", json!({"text": "two"})).unwrap();
    r.exchange();
    assert_eq!(r.texts(0).len(), 2);
    // Ann clears while Bob sends: Bob's new message survives on both sides.
    r.people[0].execute("chat.clear", json!({})).unwrap();
    r.people[1].execute("chat.send", json!({"text": "meanwhile"})).unwrap();
    r.exchange();
    assert_eq!(r.texts(0), ["meanwhile"]);
    assert_eq!(r.texts(1), ["meanwhile"]);
    assert!(r.people[1].doc().unwrap().wb.chat.cleared_at.is_some());
}

#[test]
fn an_encrypted_room_chats_in_ciphertext() {
    let key = gridcraft_engine::collab_core::b64_encode(&[7u8; 32]);
    let mut r = Room::new(session(), &[("Ann", "#3B7DD8"), ("Bob", "#D8513B")], Some(&key), None);
    r.people[1].execute("chat.send", json!({"text": "secret plans"})).unwrap();
    let out = r.people[1].collab_take_outgoing();
    assert!(!out.is_empty());
    for m in &out {
        assert!(!m.to_string().contains("secret plans"), "the transport sees only ciphertext");
    }
    r.deliver(0, out);
    r.exchange();
    assert_eq!(r.texts(0), ["secret plans"]);
}

#[test]
fn a_viewer_chats_but_doesnt_edit() {
    let mut r = Room::new(session(), &[("Ann", "#3B7DD8"), ("Vi", "#D8513B")], None, Some(1));
    assert!(r.people[1].execute("cell.set", json!({"cell": "A1", "input": "no"})).is_err());
    r.people[1].execute("chat.send", json!({"text": "Looks good"})).unwrap();
    r.exchange();
    assert_eq!(r.texts(0), ["Looks good"]);
    assert_eq!(r.texts(1), ["Looks good"]);
}

#[test]
fn typing_shows_in_the_others_awareness() {
    let mut r = Room::new(session(), &[("Ann", "#3B7DD8"), ("Bob", "#D8513B")], None, None);
    assert_eq!(r.people[1].execute("chat.typing", json!({"typing": true})).unwrap(), json!({"on": true}));
    r.exchange();
    assert_eq!(r.people[0].active_room().unwrap().typing_names(), ["Bob"]);
    // Sending ends it.
    r.people[1].execute("chat.send", json!({"text": "done"})).unwrap();
    r.exchange();
    assert!(r.people[0].active_room().unwrap().typing_names().is_empty());
    // Outside a session it does nothing.
    assert_eq!(session().execute("chat.typing", json!({"on": true})).unwrap(), json!({"on": false}));
}

#[test]
fn chatting_waits_while_joining() {
    let mut s = Session::new();
    s.collab_join_in_memory(json!({"room": "r", "mode": "join", "user": user("u-1", "Bob", "#D8513B")})).unwrap();
    assert!(matches!(s.execute("chat.send", json!({"text": "too early"})), Err(EngineError::Disabled(..))));
}
