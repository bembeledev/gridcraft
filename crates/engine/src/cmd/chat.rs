//! Document chat (craft-collab `docs/chat.md`, protocol 2.1): the people working on a workbook
//! talk beside it, in a live session or as notes left in the file.
//!
//! The chat is part of the workbook (`Workbook::chat`): saved with it, shared in a session (the
//! engine's commit turns the change into the room's `chat` entries), but never an undo step:
//! these commands are `noundo` and Undo/Redo keep the current chat. They mark the workbook
//! modified, so the chat is kept when it's saved.

use std::sync::Arc;

use gridcraft_model::chat::{self, Chat, ChatAuthor, ChatMessage};
use serde_json::{Value as Json, json};

use super::*;
use crate::collab;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "chat.list",
            "Chat Messages",
            [],
            None,
            "{since?: message id or time (ms: the messages sent after it), limit?} → {messages: [{id, author: {id, name, color}, at, text?, replyTo?, edited?, deleted?}], clearedAt?, count}: the messages after `since`, newest last",
            has_doc,
            list
        ),
        cmd!(
            noundo "chat.send",
            "Send Chat Message",
            [],
            None,
            "{text: 1–4,000 characters, replyTo?: message id} → {id}. Shared with the live session; kept in the file when it's saved",
            has_doc,
            send
        ),
        cmd!(noundo "chat.edit", "Edit Chat Message", [], None, "{id, text} → {id}. Only your own messages", has_doc, edit),
        cmd!(noundo "chat.delete", "Delete Chat Message", [], None, "{id} → {id}. Only your own; it stays as \"Message deleted\"", has_doc, delete),
        cmd!(
            noundo "chat.clear",
            "Clear Chat",
            [],
            None,
            "{} → {removed}. Removes the whole chat from the workbook, for everyone in the session (the interface asks first; programmatic calls don't)",
            has_doc,
            clear
        ),
        cmd!(query "chat.export", "Export Chat", [], None, "{format?: text|json} → {format, text}: the chat for copying elsewhere", has_doc, export),
        cmd!(
            query "chat.typing",
            "Typing in Chat",
            [],
            None,
            "{typing?: bool (alias `on`), default true} → {on}. Tells the live session you're typing a message (awareness `typing`); nothing outside a session",
            has_doc,
            typing
        ),
    ]
}

/// Who's writing: the live session's user, or this person's own outside one.
fn author(s: &mut Session) -> Result<ChatAuthor> {
    let uid = s.doc()?.uid;
    let user = match s.collab.room_of(uid) {
        Some(r) => r.user.clone(),
        None => collab::user_from_prefs(s),
    };
    Ok(ChatAuthor { id: user.id, name: user.name, color: user.color })
}

/// The chat can't change while the room's document hasn't arrived, or in another app's room.
fn writable(s: &Session, id: &str) -> Result<()> {
    let Some(r) = s.active().and_then(|d| s.collab.room_of(d.uid)) else { return Ok(()) };
    if let Some(why) = &r.refused {
        return Err(EngineError::Disabled(id.into(), why.clone()));
    }
    if r.joining {
        return Err(EngineError::Disabled(id.into(), "Joining…".into()));
    }
    Ok(())
}

fn now_ms() -> i64 {
    gridcraft_core::date::unix_now_ms().map(|m| m as i64).unwrap_or(0).max(0)
}

/// A new message id: 8 random characters of `0-9a-z`, not in `chat`.
fn new_id(chat: &Chat) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    loop {
        let mut b = [0u8; 8];
        let mut v = if gridcraft_collab::random_bytes(&mut b).is_ok() { u64::from_le_bytes(b) } else { gridcraft_collab::new_client_id() };
        let id: String = (0..8)
            .map(|_| {
                let c = DIGITS.get((v % 36) as usize).copied().unwrap_or(b'0') as char;
                v /= 36;
                c
            })
            .collect();
        if chat.get(&id).is_none() {
            return id;
        }
    }
}

/// Changes the active workbook's chat: a new workbook version (modified), no undo step.
fn change<T>(s: &mut Session, f: impl FnOnce(&mut Chat) -> std::result::Result<T, String>, id: &str) -> Result<T> {
    let d = s.doc_mut()?;
    let mut chat = (*d.wb.chat).clone();
    let out = f(&mut chat).map_err(|e| bad(id, e))?;
    Arc::make_mut(&mut d.wb).chat = Arc::new(chat);
    d.revision += 1;
    Ok(out)
}

fn text_param(p: &Json, id: &str) -> Result<String> {
    let t = str_param(p, "text").ok_or_else(|| bad(id, "missing `text`"))?;
    chat::valid_text(t).map_err(|e| bad(id, e))?;
    Ok(t.to_string())
}

fn list(s: &mut Session, p: &Json) -> Result<Json> {
    let d = s.doc()?;
    let limit = p.get("limit").and_then(Json::as_u64).map(|l| l.min(chat::MAX_MESSAGES as u64) as usize);
    // `since`: a message id (the ones after it) or a time in milliseconds (the ones sent after it).
    let messages: Vec<Json> = match p.get("since").and_then(Json::as_f64) {
        Some(ms) => {
            let after: Vec<&ChatMessage> = d.wb.chat.messages.iter().filter(|m| m.at as f64 > ms).collect();
            let from = after.len().saturating_sub(limit.unwrap_or(usize::MAX));
            after.get(from..).unwrap_or_default().iter().map(|m| m.to_json()).collect()
        }
        None => d.wb.chat.list(str_param(p, "since"), limit).iter().map(ChatMessage::to_json).collect(),
    };
    let mut out = json!({"messages": messages, "count": d.wb.chat.messages.len()});
    if let Some(c) = d.wb.chat.cleared_at {
        out["clearedAt"] = json!(c);
    }
    Ok(out)
}

fn send(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "chat.send";
    writable(s, ID)?;
    let text = text_param(p, ID)?;
    let author = author(s)?;
    let reply_to = str_param(p, "replyTo").filter(|r| !r.is_empty()).map(str::to_string);
    let at = now_ms();
    let id = change(
        s,
        |c| {
            let id = new_id(c);
            c.send(ChatMessage { id: id.clone(), author, at, text: Some(text), reply_to, ..ChatMessage::default() })?;
            Ok(id)
        },
        ID,
    )?;
    if let Some(r) = s.active().and_then(|d| s.collab.room_of(d.uid)).map(|r| r.room.clone()) {
        set_typing(s, &r, false);
    }
    Ok(json!({"id": id}))
}

fn edit(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "chat.edit";
    writable(s, ID)?;
    let id = str_param(p, "id").ok_or_else(|| bad(ID, "missing `id`"))?.to_string();
    let text = text_param(p, ID)?;
    let author = author(s)?;
    let at = now_ms();
    change(s, |c| c.edit(&id, &author.id, &text, at), ID)?;
    Ok(json!({"id": id}))
}

fn delete(s: &mut Session, p: &Json) -> Result<Json> {
    const ID: &str = "chat.delete";
    writable(s, ID)?;
    let id = str_param(p, "id").ok_or_else(|| bad(ID, "missing `id`"))?.to_string();
    let author = author(s)?;
    change(s, |c| c.delete(&id, &author.id), ID)?;
    Ok(json!({"id": id}))
}

fn clear(s: &mut Session, _p: &Json) -> Result<Json> {
    const ID: &str = "chat.clear";
    writable(s, ID)?;
    let at = now_ms();
    let removed = change(s, |c| Ok(c.clear(at)), ID)?;
    Ok(json!({"removed": removed}))
}

fn export(s: &mut Session, p: &Json) -> Result<Json> {
    let d = s.doc()?;
    match str_param(p, "format").unwrap_or("text") {
        "text" => {
            let offset = p.get("utcOffsetMinutes").and_then(Json::as_i64).map(|m| m.clamp(-14 * 60, 14 * 60) as i32).unwrap_or(0);
            Ok(json!({"format": "text", "text": d.wb.chat.to_text(offset)}))
        }
        "json" => Ok(json!({"format": "json", "text": String::from_utf8_lossy(&d.wb.chat.to_part()), "chat": d.wb.chat.to_json()})),
        _ => Err(bad("chat.export", "`format` is `text` or `json`")),
    }
}

fn set_typing(s: &mut Session, room: &str, on: bool) {
    if let Some(r) = s.collab.rooms.iter_mut().find(|r| r.room == room)
        && r.typing != on
    {
        r.typing = on;
        collab::awareness_changed(s);
    }
}

fn typing(s: &mut Session, p: &Json) -> Result<Json> {
    let on = bool_param(p, "typing").or_else(|| bool_param(p, "on")).unwrap_or(true);
    let uid = s.doc()?.uid;
    let Some(room) = s.collab.room_of(uid).map(|r| r.room.clone()) else { return Ok(json!({"on": false})) };
    set_typing(s, &room, on);
    Ok(json!({"on": on}))
}

/// Undo and Redo bring back an earlier workbook: it keeps the current chat (chat changes are
/// never undone or redone).
pub(crate) fn keep_chat(from: &gridcraft_model::Workbook, to: &mut Arc<gridcraft_model::Workbook>) {
    if !Arc::ptr_eq(&from.chat, &to.chat) && *from.chat != *to.chat {
        Arc::make_mut(to).chat = from.chat.clone();
    }
}
